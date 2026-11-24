//! Данные для nexo://performance. Только реальные измерения: если замер недоступен
//! (не Windows, нет доступа к процессу), страница получает `supported = false` и текст ошибки,
//! а не придуманные числа.
//!
//! Память по каждой вкладке отдельно НЕ показывается: у WebView2 нет стабильного API, который
//! сопоставляет процесс рендеринга и вкладку (процессы могут делиться несколькими вкладками).
//! Вместо этого — честный список процессов дерева браузера.

use crate::browser::tab::{TabId, TabInfo, TabState};
use crate::memory::monitor::Snapshot;
use crate::memory::policy::PerformanceMode;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProcView {
    pub pid: u32,
    pub name: String,
    pub working_set: u64,
    pub private_bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TabRow {
    pub title: String,
    /// `active` — текущая; `background` — фоновая с живой страницей; `suspended`; `discarded`.
    pub state: &'static str,
    pub idle_secs: u64,
    /// Почему вкладку нельзя усыпить: `audio`, `downloading`, `input`, `loading`.
    pub protected: Option<&'static str>,
}

/// Параметры режима, как они реально применяются. Страница показывает именно эти числа.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModeInfo {
    pub id: PerformanceMode,
    pub label: &'static str,
    pub suspend_after_secs: u64,
    pub discard_after_secs: u64,
    /// 0 — без ограничения.
    pub max_live_webviews: usize,
    pub trims_hidden: bool,
    pub tick_secs: u64,
    /// Режим добавляет аргументы процессов WebView2 (действуют после перезапуска).
    pub extra_browser_args: bool,
}

pub fn mode_infos() -> Vec<ModeInfo> {
    PerformanceMode::ALL
        .iter()
        .map(|&m| {
            let p = m.policy();
            ModeInfo {
                id: m,
                label: m.label(),
                suspend_after_secs: p.suspend_after_secs,
                discard_after_secs: p.discard_after_secs,
                max_live_webviews: p.max_live_webviews,
                trims_hidden: m.trim_hidden_immediately(),
                tick_secs: m.tick_secs(),
                extra_browser_args: m.browser_args() != PerformanceMode::Balanced.browser_args(),
            }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PerfReport {
    pub supported: bool,
    pub error: Option<String>,
    /// Сумма Working Set по дереву процессов (включает общие страницы DLL — немного завышена).
    pub total_working_set: u64,
    /// Сумма Private Bytes по дереву процессов (зафиксированная приватная память).
    pub total_private_bytes: u64,
    pub process_count: usize,
    /// `None`, пока нет двух замеров для сравнения.
    pub cpu_percent: Option<f64>,
    pub tabs_total: usize,
    /// Вкладки с работающей страницей (включая текущую).
    pub tabs_awake: usize,
    /// Приостановленные и выгруженные вкладки.
    pub tabs_sleeping: usize,
    pub per_tab_memory_available: bool,
    pub mode: PerformanceMode,
    pub sleeping_enabled: bool,
    /// Аргументы процессов WebView2 текущего режима отличаются от тех, с которыми запущен браузер.
    pub restart_needed: bool,
    pub modes: Vec<ModeInfo>,
    pub processes: Vec<ProcView>,
    pub tabs: Vec<TabRow>,
    pub taken_at_ms: u64,
}

pub struct ReportInput<'a> {
    pub sample: Result<&'a Snapshot, String>,
    pub cpu_percent: Option<f64>,
    pub tabs: &'a [TabInfo],
    pub active: Option<TabId>,
    pub mode: PerformanceMode,
    pub sleeping_enabled: bool,
    pub restart_needed: bool,
    pub now_ms: u64,
}

fn tab_state_name(t: &TabInfo, active: Option<TabId>) -> &'static str {
    match t.state {
        TabState::Discarded => "discarded",
        TabState::Suspended => "suspended",
        TabState::Active if Some(t.id) == active => "active",
        TabState::Active => "background",
    }
}

pub fn build_report(input: ReportInput<'_>) -> PerfReport {
    let ReportInput {
        sample,
        cpu_percent,
        tabs,
        active,
        mode,
        sleeping_enabled,
        restart_needed,
        now_ms,
    } = input;

    let (supported, error, snap) = match sample {
        Ok(s) => (true, None, Some(s)),
        Err(e) => (false, Some(e), None),
    };

    let mut processes: Vec<ProcView> = snap
        .map(|s| {
            s.processes
                .iter()
                .map(|p| ProcView {
                    pid: p.pid,
                    name: p.name.clone(),
                    working_set: p.working_set_bytes,
                    private_bytes: p.private_bytes,
                })
                .collect()
        })
        .unwrap_or_default();
    processes.sort_by(|a, b| b.working_set.cmp(&a.working_set).then(a.pid.cmp(&b.pid)));

    let rows = tabs
        .iter()
        .map(|t| TabRow {
            title: if t.title.is_empty() {
                t.url.clone()
            } else {
                t.title.clone()
            },
            state: tab_state_name(t, active),
            idle_secs: if Some(t.id) == active {
                0
            } else {
                now_ms.saturating_sub(t.last_active_ms) / 1000
            },
            protected: t.protection_reason(),
        })
        .collect();

    let awake = tabs.iter().filter(|t| t.state == TabState::Active).count();

    PerfReport {
        supported,
        error,
        total_working_set: snap.map(|s| s.total_working_set()).unwrap_or(0),
        total_private_bytes: snap.map(|s| s.total_private_bytes()).unwrap_or(0),
        process_count: snap.map(|s| s.process_count()).unwrap_or(0),
        cpu_percent: if supported { cpu_percent } else { None },
        tabs_total: tabs.len(),
        tabs_awake: awake,
        tabs_sleeping: tabs.len() - awake,
        per_tab_memory_available: false,
        mode,
        sleeping_enabled,
        restart_needed,
        modes: mode_infos(),
        processes,
        tabs: rows,
        taken_at_ms: now_ms,
    }
}

impl PerfReport {
    /// JS для страницы производительности. Данные — только JSON.
    pub fn to_script(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_else(|_| "null".into());
        format!("window.nexoPerf&&window.nexoPerf.update({json});")
    }
}

/// Текст для уведомления после нажатия «Освободить память».
pub fn free_memory_notice(requested: usize, protected: usize) -> String {
    match (requested, protected) {
        (0, 0) => "Освобождать нечего: фоновых вкладок нет.".to_string(),
        (0, p) => format!("Все фоновые вкладки защищены от сна (звук, загрузка или ввод): {p}."),
        (r, 0) => format!("Усыпляю вкладок: {r}. Память освободится через несколько секунд."),
        (r, p) => {
            format!("Усыпляю вкладок: {r}. Пропущено защищённых (звук, загрузка, ввод): {p}.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::monitor::ProcessSample;

    fn proc(pid: u32, ws: u64, private: u64) -> ProcessSample {
        ProcessSample {
            pid,
            parent_pid: 0,
            name: format!("p{pid}.exe"),
            working_set_bytes: ws,
            private_bytes: private,
            cpu_time_100ns: 0,
        }
    }

    fn input<'a>(
        sample: Result<&'a Snapshot, String>,
        tabs: &'a [TabInfo],
        active: Option<TabId>,
    ) -> ReportInput<'a> {
        ReportInput {
            sample,
            cpu_percent: Some(3.5),
            tabs,
            active,
            mode: PerformanceMode::Balanced,
            sleeping_enabled: true,
            restart_needed: false,
            now_ms: 100_000,
        }
    }

    #[test]
    fn totals_and_sorting_come_from_the_real_snapshot() {
        let snap = Snapshot {
            processes: vec![proc(2, 100, 40), proc(1, 300, 120), proc(3, 200, 60)],
            taken_at_ms: 0,
        };
        let r = build_report(input(Ok(&snap), &[], None));
        assert!(r.supported && r.error.is_none());
        assert_eq!(r.total_working_set, 600);
        assert_eq!(r.total_private_bytes, 220);
        assert_eq!(r.process_count, 3);
        assert_eq!(
            r.processes.iter().map(|p| p.pid).collect::<Vec<_>>(),
            vec![1, 3, 2]
        );
        assert_eq!(r.cpu_percent, Some(3.5));
        assert!(!r.per_tab_memory_available);
    }

    #[test]
    fn unavailable_measurement_is_reported_not_faked() {
        let r = build_report(input(Err("только Windows".into()), &[], None));
        assert!(!r.supported);
        assert_eq!(r.error.as_deref(), Some("только Windows"));
        assert_eq!(
            (r.total_working_set, r.total_private_bytes, r.process_count),
            (0, 0, 0)
        );
        assert_eq!(r.cpu_percent, None);
        assert!(r.processes.is_empty());
    }

    #[test]
    fn tab_rows_report_state_idle_and_protection() {
        let mut active = TabInfo::new(TabId(1), "https://a.com/".into(), false, 90_000);
        active.title = "A".into();
        let mut bg = TabInfo::new(TabId(2), "https://b.com/".into(), false, 40_000);
        bg.playing_audio = true;
        let mut sus = TabInfo::new(TabId(3), "https://c.com/".into(), false, 10_000);
        sus.state = TabState::Suspended;
        let mut dis = TabInfo::new(TabId(4), "https://d.com/".into(), false, 0);
        dis.state = TabState::Discarded;

        let tabs = [active, bg, sus, dis];
        let r = build_report(input(Err("x".into()), &tabs, Some(TabId(1))));
        let states: Vec<_> = r.tabs.iter().map(|t| t.state).collect();
        assert_eq!(states, ["active", "background", "suspended", "discarded"]);
        assert_eq!(r.tabs[0].idle_secs, 0);
        assert_eq!(r.tabs[1].idle_secs, 60);
        assert_eq!(r.tabs[1].protected, Some("audio"));
        assert_eq!(r.tabs[2].protected, None);
        assert_eq!(r.tabs[1].title, "https://b.com/"); // нет заголовка — показываем адрес
        assert_eq!((r.tabs_total, r.tabs_awake, r.tabs_sleeping), (4, 2, 2));
    }

    #[test]
    fn script_is_valid_json_call() {
        let r = build_report(input(Err("e\"</script>".into()), &[], None));
        let js = r.to_script();
        let json = js
            .trim_start_matches("window.nexoPerf&&window.nexoPerf.update(")
            .trim_end_matches(");");
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["supported"], false);
        assert_eq!(v["mode"], "Balanced");
        assert_eq!(v["error"], "e\"</script>");
    }

    #[test]
    fn mode_infos_mirror_the_real_policy() {
        let infos = mode_infos();
        assert_eq!(infos.len(), 3);
        for i in &infos {
            let p = i.id.policy();
            assert_eq!(
                (
                    i.suspend_after_secs,
                    i.discard_after_secs,
                    i.max_live_webviews
                ),
                (
                    p.suspend_after_secs,
                    p.discard_after_secs,
                    p.max_live_webviews
                )
            );
            assert_eq!(i.label, i.id.label());
        }
        assert!(infos[0].trims_hidden && infos[0].extra_browser_args);
        assert!(!infos[1].extra_browser_args && !infos[2].extra_browser_args);
    }

    #[test]
    fn notices_are_truthful_about_what_happens() {
        assert!(free_memory_notice(0, 0).contains("нечего"));
        assert!(free_memory_notice(0, 2).contains("защищены"));
        assert!(free_memory_notice(3, 0).contains("3"));
        let both = free_memory_notice(3, 1);
        assert!(both.contains('3') && both.contains("Пропущено"));
    }
}
