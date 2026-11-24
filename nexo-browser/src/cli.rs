//! Параметры командной строки.
//!
//! * `nexo.exe URL1 URL2 …` — открыть адреса в вкладках (первый становится активным);
//! * `--private` — приватное окно;
//! * `--data-dir=ПУТЬ` — другой каталог данных (отдельный профиль: нужен для воспроизводимых замеров);
//! * `--mode=memory|balanced|max` — режим производительности только на этот запуск (в настройки не пишется).

use crate::memory::policy::PerformanceMode;
use std::path::PathBuf;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Cli {
    pub private: bool,
    pub urls: Vec<String>,
    pub data_dir: Option<PathBuf>,
    pub mode: Option<PerformanceMode>,
}

pub const PRIVATE_FLAG: &str = "--private";

impl Cli {
    /// Разбирает аргументы (без имени программы). Неизвестные флаги `--…` игнорируются,
    /// чтобы системные параметры запуска не превращались в адреса.
    pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Cli {
        let mut cli = Cli::default();
        for a in args {
            if a == PRIVATE_FLAG {
                cli.private = true;
            } else if let Some(p) = a.strip_prefix("--data-dir=") {
                if !p.trim().is_empty() {
                    cli.data_dir = Some(PathBuf::from(p));
                }
            } else if let Some(m) = a.strip_prefix("--mode=") {
                cli.mode = match m.to_ascii_lowercase().as_str() {
                    "memory" | "memorysaver" => Some(PerformanceMode::MemorySaver),
                    "balanced" => Some(PerformanceMode::Balanced),
                    "max" | "maxperformance" => Some(PerformanceMode::MaxPerformance),
                    _ => None,
                };
            } else if !a.starts_with("--") && !a.trim().is_empty() {
                cli.urls.push(a);
            }
        }
        cli
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn empty_command_line() {
        assert_eq!(parse(&[]), Cli::default());
    }

    #[test]
    fn urls_keep_their_order_and_flags_are_not_urls() {
        let c = parse(&["https://a.com", "--private", "b.com", "--unknown", "  "]);
        assert_eq!(c.urls, ["https://a.com", "b.com"]);
        assert!(c.private);
    }

    #[test]
    fn data_dir_and_mode() {
        let c = parse(&["--data-dir=C:\\tmp\\x y", "--mode=memory", "https://a.com"]);
        assert_eq!(c.data_dir, Some(PathBuf::from("C:\\tmp\\x y")));
        assert_eq!(c.mode, Some(PerformanceMode::MemorySaver));
        assert_eq!(
            parse(&["--mode=MAX"]).mode,
            Some(PerformanceMode::MaxPerformance)
        );
        assert_eq!(
            parse(&["--mode=balanced"]).mode,
            Some(PerformanceMode::Balanced)
        );
    }

    #[test]
    fn bad_values_are_ignored() {
        let c = parse(&["--mode=turbo", "--data-dir="]);
        assert_eq!(c.mode, None);
        assert_eq!(c.data_dir, None);
    }
}
