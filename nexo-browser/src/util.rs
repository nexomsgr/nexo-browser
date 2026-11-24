//! Мелкие общие утилиты.

use std::time::{SystemTime, UNIX_EPOCH};

/// Текущее время в миллисекундах с начала эпохи Unix.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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
}
