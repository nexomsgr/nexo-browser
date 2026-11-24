//! Реальные метрики для nexo://performance. Ничего не выдумывается:
//! если значение недоступно, возвращается ошибка или `None`.
//!
//! Метод: перечисляем процессы, берём дерево потомков нашего процесса (сам браузер +
//! процессы msedgewebview2.exe) и для каждого читаем Working Set, Private Bytes и CPU-время.
//! ВАЖНО: Working Set включает общие страницы (DLL), поэтому сумма по дереву слегка
//! завышена; Private Bytes — это committed-память процесса, а не «видимая» в диспетчере задач.
//! В отчётах всегда указываем, какая именно метрика приведена.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct ProcessSample {
    pub pid: u32,
    pub parent_pid: u32,
    pub name: String,
    pub working_set_bytes: u64,
    pub private_bytes: u64,
    /// Суммарное CPU-время (ядро + пользователь) в единицах по 100 нс.
    pub cpu_time_100ns: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub processes: Vec<ProcessSample>,
    pub taken_at_ms: u64,
}

impl Snapshot {
    pub fn process_count(&self) -> usize {
        self.processes.len()
    }

    pub fn total_working_set(&self) -> u64 {
        self.processes.iter().map(|p| p.working_set_bytes).sum()
    }

    pub fn total_private_bytes(&self) -> u64 {
        self.processes.iter().map(|p| p.private_bytes).sum()
    }
}

/// Все потомки `root` (включая сам `root`), по таблице (pid, parent_pid).
/// Устойчив к циклам, которые возникают при переиспользовании PID.
pub fn descendants(pairs: &[(u32, u32)], root: u32) -> Vec<u32> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for &(pid, parent) in pairs {
        if pid != parent {
            children.entry(parent).or_default().push(pid);
        }
    }
    let mut seen: HashSet<u32> = HashSet::new();
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(p) = stack.pop() {
        if !seen.insert(p) {
            continue;
        }
        out.push(p);
        if let Some(ch) = children.get(&p) {
            stack.extend(ch.iter().copied());
        }
    }
    out
}

/// Считает загрузку CPU между двумя снимками (в процентах от всех ядер).
pub struct CpuTracker {
    cores: u32,
    last: HashMap<u32, u64>,
    last_at_ms: Option<u64>,
}

impl CpuTracker {
    pub fn new(cores: u32) -> Self {
        Self {
            cores: cores.max(1),
            last: HashMap::new(),
            last_at_ms: None,
        }
    }

    /// Первый вызов возвращает `None` (не с чем сравнивать). В расчёт входят только процессы,
    /// присутствующие в обоих снимках, поэтому завершившиеся процессы не искажают результат.
    pub fn update(&mut self, snap: &Snapshot) -> Option<f64> {
        let result = match self.last_at_ms {
            Some(prev_ms) if snap.taken_at_ms > prev_ms => {
                let elapsed_100ns = (snap.taken_at_ms - prev_ms) as f64 * 10_000.0;
                let used: u64 = snap
                    .processes
                    .iter()
                    .filter_map(|p| {
                        self.last
                            .get(&p.pid)
                            .map(|old| p.cpu_time_100ns.saturating_sub(*old))
                    })
                    .sum();
                Some((used as f64 / (elapsed_100ns * self.cores as f64) * 100.0).clamp(0.0, 100.0))
            }
            _ => None,
        };
        self.last = snap
            .processes
            .iter()
            .map(|p| (p.pid, p.cpu_time_100ns))
            .collect();
        self.last_at_ms = Some(snap.taken_at_ms);
        result
    }
}

#[cfg(windows)]
pub fn sample_tree() -> anyhow::Result<Snapshot> {
    win::sample_tree()
}

#[cfg(not(windows))]
pub fn sample_tree() -> anyhow::Result<Snapshot> {
    anyhow::bail!("монитор процессов реализован только для Windows")
}

#[cfg(windows)]
mod win {
    use super::*;
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcessId, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    struct Entry {
        pid: u32,
        parent: u32,
        name: String,
    }

    fn list_processes() -> anyhow::Result<Vec<Entry>> {
        let mut out = Vec::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?;
            let mut e: PROCESSENTRY32W = std::mem::zeroed();
            e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut ok = Process32FirstW(snap, &mut e).is_ok();
            while ok {
                let len = e
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(e.szExeFile.len());
                out.push(Entry {
                    pid: e.th32ProcessID,
                    parent: e.th32ParentProcessID,
                    name: String::from_utf16_lossy(&e.szExeFile[..len]),
                });
                ok = Process32NextW(snap, &mut e).is_ok();
            }
            let _ = CloseHandle(snap);
        }
        Ok(out)
    }

    fn filetime_to_u64(ft: FILETIME) -> u64 {
        ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
    }

    /// Читает метрики одного процесса; `None`, если доступ запрещён или процесс уже завершился.
    fn read_process(entry: &Entry) -> Option<ProcessSample> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, entry.pid).ok()?;
            let mut mem: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
            mem.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
            let mem_ok = GetProcessMemoryInfo(
                h,
                &mut mem as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
                mem.cb,
            )
            .is_ok();

            let (mut c, mut x, mut k, mut u) = (
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
            );
            let times_ok = GetProcessTimes(h, &mut c, &mut x, &mut k, &mut u).is_ok();
            let _ = CloseHandle(h);

            if !mem_ok {
                return None;
            }
            Some(ProcessSample {
                pid: entry.pid,
                parent_pid: entry.parent,
                name: entry.name.clone(),
                working_set_bytes: mem.WorkingSetSize as u64,
                private_bytes: mem.PrivateUsage as u64,
                cpu_time_100ns: if times_ok {
                    filetime_to_u64(k) + filetime_to_u64(u)
                } else {
                    0
                },
            })
        }
    }

    pub fn sample_tree() -> anyhow::Result<Snapshot> {
        let entries = list_processes()?;
        let me = unsafe { GetCurrentProcessId() };
        let pairs: Vec<(u32, u32)> = entries.iter().map(|e| (e.pid, e.parent)).collect();
        let tree: HashSet<u32> = descendants(&pairs, me).into_iter().collect();
        let processes = entries
            .iter()
            .filter(|e| tree.contains(&e.pid))
            .filter_map(read_process)
            .collect();
        Ok(Snapshot {
            processes,
            taken_at_ms: crate::util::now_ms(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(pid: u32, cpu: u64) -> ProcessSample {
        ProcessSample {
            pid,
            parent_pid: 0,
            name: "x".into(),
            working_set_bytes: 1_000,
            private_bytes: 500,
            cpu_time_100ns: cpu,
        }
    }

    #[test]
    fn descendants_follow_parent_links() {
        let pairs = [(1, 0), (2, 1), (3, 2), (4, 1), (5, 99), (6, 5)];
        let mut d = descendants(&pairs, 1);
        d.sort();
        assert_eq!(d, vec![1, 2, 3, 4]);
    }

    #[test]
    fn descendants_survive_pid_cycles() {
        let pairs = [(1, 2), (2, 1), (3, 3)];
        let mut d = descendants(&pairs, 1);
        d.sort();
        assert_eq!(d, vec![1, 2]);
    }

    #[test]
    fn totals_add_up() {
        let s = Snapshot {
            processes: vec![sample(1, 0), sample(2, 0)],
            taken_at_ms: 0,
        };
        assert_eq!(s.total_working_set(), 2_000);
        assert_eq!(s.total_private_bytes(), 1_000);
        assert_eq!(s.process_count(), 2);
    }

    #[test]
    fn cpu_first_sample_is_none_then_measured() {
        let mut t = CpuTracker::new(4);
        let a = Snapshot {
            processes: vec![sample(1, 0)],
            taken_at_ms: 0,
        };
        assert_eq!(t.update(&a), None);
        // За 1000 мс процесс израсходовал 1 с CPU-времени = 25% от 4 ядер.
        let b = Snapshot {
            processes: vec![sample(1, 10_000_000)],
            taken_at_ms: 1_000,
        };
        let v = t.update(&b).unwrap();
        assert!((v - 25.0).abs() < 1e-9, "{v}");
    }

    #[test]
    fn cpu_ignores_new_and_exited_processes() {
        let mut t = CpuTracker::new(1);
        t.update(&Snapshot {
            processes: vec![sample(1, 0), sample(2, 0)],
            taken_at_ms: 0,
        });
        // Процесс 2 завершился, процесс 3 новый (его накопленное время не должно считаться).
        let v = t
            .update(&Snapshot {
                processes: vec![sample(1, 5_000_000), sample(3, 900_000_000)],
                taken_at_ms: 1_000,
            })
            .unwrap();
        assert!((v - 50.0).abs() < 1e-9, "{v}");
    }
}
