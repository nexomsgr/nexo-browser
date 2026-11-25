//! Мелкие общие утилиты.

use std::time::{SystemTime, UNIX_EPOCH};

/// Текущее время в миллисекундах с начала эпохи Unix.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Метка времени UTC для имён файлов: `20261125-130400`.
pub fn timestamp_label(ms: u64) -> String {
    let secs = ms / 1000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Гражданская дата по дню от эпохи (алгоритм Говарда Хиннанта).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Ограничитель частоты: пропускает не чаще одного события за `min_gap_ms`.
/// Используется против шквала всплывающих окон.
pub struct Throttle {
    min_gap_ms: u64,
    last_ms: Option<u64>,
}

impl Throttle {
    pub fn new(min_gap_ms: u64) -> Self {
        Self {
            min_gap_ms,
            last_ms: None,
        }
    }

    pub fn allow(&mut self, now_ms: u64) -> bool {
        match self.last_ms {
            Some(last) if now_ms.saturating_sub(last) < self.min_gap_ms => false,
            _ => {
                self.last_ms = Some(now_ms);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttle_limits_bursts_then_recovers() {
        let mut t = Throttle::new(400);
        assert!(t.allow(1_000));
        assert!(!t.allow(1_100));
        assert!(!t.allow(1_399));
        assert!(t.allow(1_400));
        assert!(!t.allow(1_500));
    }

    #[test]
    fn throttle_ignores_clock_going_backwards() {
        let mut t = Throttle::new(100);
        assert!(t.allow(5_000));
        assert!(!t.allow(4_000));
    }

    #[test]
    fn timestamp_label_is_utc() {
        assert_eq!(timestamp_label(0), "19700101-000000");
        assert_eq!(timestamp_label(1_700_000_000_000), "20231114-221320");
        assert_eq!(timestamp_label(1_709_210_096_000), "20240229-123456");
    }
}
