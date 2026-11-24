//! Три режима производительности и их реальные параметры.
//! Числа — стартовые; окончательно их подберут замеры (шаг 8).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PerformanceMode {
    MemorySaver,
    #[default]
    Balanced,
    MaxPerformance,
}

#[derive(Debug, Clone, Copy)]
pub struct SleepPolicy {
    /// Через сколько секунд бездействия фоновая вкладка приостанавливается (TrySuspend).
    pub suspend_after_secs: u64,
    /// Через сколько секунд бездействия вкладка выгружается (WebView2 уничтожается).
    pub discard_after_secs: u64,
    /// Максимум одновременно живых WebView2 (0 = без ограничения).
    pub max_live_webviews: usize,
}

/// Базовые аргументы WebView2. Нужны, потому что передача своих аргументов отключает набор по умолчанию
/// из wry (убирает «мини-меню» и SmartScreen-интеграцию).
const BASE_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

impl PerformanceMode {
    pub const ALL: [PerformanceMode; 3] = [Self::MemorySaver, Self::Balanced, Self::MaxPerformance];

    pub fn policy(self) -> SleepPolicy {
        match self {
            Self::MemorySaver => SleepPolicy {
                suspend_after_secs: 30,
                discard_after_secs: 5 * 60,
                max_live_webviews: 4,
            },
            Self::Balanced => SleepPolicy {
                suspend_after_secs: 2 * 60,
                discard_after_secs: 15 * 60,
                max_live_webviews: 10,
            },
            Self::MaxPerformance => SleepPolicy {
                suspend_after_secs: 10 * 60,
                discard_after_secs: 60 * 60,
                max_live_webviews: 0,
            },
        }
    }

    /// Как часто просыпается проверка усыпления (секунды). Реже — меньше фоновой нагрузки на CPU.
    pub fn tick_secs(self) -> u64 {
        match self {
            Self::MemorySaver => 10,
            Self::Balanced => 15,
            Self::MaxPerformance => 30,
        }
    }

    /// Просить WebView2 сразу сжать память скрытой вкладки (MemoryUsageTargetLevel = Low),
    /// не дожидаясь приостановки. Экономит RAM ценой чуть более медленного возврата на вкладку.
    pub fn trim_hidden_immediately(self) -> bool {
        matches!(self, Self::MemorySaver)
    }

    /// Аргументы браузерного процесса WebView2. Применяются только при запуске: окружение WebView2
    /// создаётся один раз, поэтому смена режима «на лету» меняет всё остальное, но не эти аргументы.
    ///
    /// `--process-per-site` (только «Экономия памяти») — ЭКСПЕРИМЕНТАЛЬНО: вкладки одного сайта
    /// делят процесс рендеринга. Выигрыш и совместимость нужно подтвердить замерами (шаг 8).
    pub fn browser_args(self) -> String {
        match self {
            Self::MemorySaver => format!("{BASE_ARGS} --process-per-site"),
            Self::Balanced | Self::MaxPerformance => BASE_ARGS.to_string(),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::MemorySaver => "Экономия памяти",
            Self::Balanced => "Сбалансированный",
            Self::MaxPerformance => "Максимальная производительность",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_balanced() {
        assert_eq!(PerformanceMode::default(), PerformanceMode::Balanced);
    }

    #[test]
    fn modes_differ_in_every_real_parameter() {
        let (s, b, m) = (
            PerformanceMode::MemorySaver,
            PerformanceMode::Balanced,
            PerformanceMode::MaxPerformance,
        );
        assert!(s.tick_secs() < b.tick_secs() && b.tick_secs() < m.tick_secs());
        assert!(s.trim_hidden_immediately());
        assert!(!b.trim_hidden_immediately() && !m.trim_hidden_immediately());
        assert_ne!(s.browser_args(), b.browser_args());
        assert_eq!(b.browser_args(), m.browser_args());
    }

    #[test]
    fn browser_args_keep_base_flags_and_have_no_duplicates_of_disable_features() {
        for mode in PerformanceMode::ALL {
            let args = mode.browser_args();
            assert!(args.starts_with(BASE_ARGS), "{args}");
            // Несколько --disable-features перекрывают друг друга — должен быть ровно один.
            assert_eq!(args.matches("--disable-features").count(), 1, "{args}");
        }
        assert!(PerformanceMode::MemorySaver
            .browser_args()
            .contains("--process-per-site"));
        assert!(!PerformanceMode::Balanced
            .browser_args()
            .contains("--process-per-site"));
    }

    #[test]
    fn serde_names_are_stable() {
        // Эти строки используются страницей nexo://performance и сохранёнными настройками.
        assert_eq!(
            serde_json::to_string(&PerformanceMode::MemorySaver).unwrap(),
            "\"MemorySaver\""
        );
        assert_eq!(
            serde_json::from_str::<PerformanceMode>("\"MaxPerformance\"").unwrap(),
            PerformanceMode::MaxPerformance
        );
        assert!(serde_json::from_str::<PerformanceMode>("\"Turbo\"").is_err());
    }
}
