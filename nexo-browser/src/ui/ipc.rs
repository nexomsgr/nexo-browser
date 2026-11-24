//! Протокол между Rust и интерфейсом (панель вкладок и адресная строка в отдельном WebView2).
//!
//! Безопасность: сообщения от интерфейса принимаются только из источника `nexo-ui`;
//! внутренним страницам `nexo://` разрешено лишь подмножество команд (`allowed_from_page`).
//! Обычные сайты IPC не получают вообще.

use crate::browser::navigation::{SearchEngine, Security};
use crate::memory::policy::PerformanceMode;
use crate::storage::settings::ThemeChoice;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum UiMessage {
    Navigate {
        input: String,
    },
    Back,
    Forward,
    Reload,
    Stop,
    NewTab,
    CloseTab {
        id: u64,
    },
    ActivateTab {
        id: u64,
    },
    MoveTab {
        id: u64,
        index: usize,
    },
    ReopenClosedTab,
    /// Открыть внутреннюю страницу (имя без схемы, например `performance`) — только от интерфейса.
    OpenInternal {
        page: String,
    },
    /// Страница производительности просит свежие метрики.
    PerfSnapshot,
    /// Кнопка «Освободить память».
    FreeMemory,
    SetMode {
        mode: PerformanceMode,
    },
    SetSleeping {
        enabled: bool,
    },
    /// Нажатие клавиши в панели интерфейса (имя — из `KeyboardEvent.key`). Только от интерфейса.
    Key {
        ctrl: bool,
        shift: bool,
        alt: bool,
        key: String,
    },
    /// Звёздочка в адресной строке.
    ToggleBookmark,
    /// Страница просит данные для себя (какие — решает адрес её вкладки).
    PageData,
    BookmarkRemove {
        id: i64,
    },
    BookmarkMove {
        id: i64,
        folder: Option<i64>,
    },
    FolderAdd {
        name: String,
    },
    FolderRemove {
        id: i64,
    },
    HistorySearch {
        query: String,
    },
    HistoryDelete {
        id: i64,
    },
    HistoryClear,
    DownloadCancel {
        id: u64,
    },
    DownloadShow {
        id: u64,
    },
    DownloadsClearFinished,
    /// Изменение настроек со страницы настроек; отсутствующие поля не меняются.
    UpdateSettings {
        theme: Option<ThemeChoice>,
        search_engine: Option<SearchEngine>,
        restore_session: Option<bool>,
        homepage: Option<String>,
        default_zoom_pct: Option<u32>,
    },
}

impl UiMessage {
    pub fn parse(json: &str) -> Option<Self> {
        // Защита от чрезмерно больших сообщений.
        if json.len() > 8 * 1024 {
            return None;
        }
        serde_json::from_str(json).ok()
    }

    /// Команды, которые вправе отправлять внутренние страницы nexo://: навигация (строка поиска
    /// «Новой вкладки») и управление памятью со страницы производительности. Вкладками, историей
    /// и всем остальным управляет только собственный интерфейс.
    pub fn allowed_from_page(&self) -> bool {
        matches!(
            self,
            UiMessage::Navigate { .. }
                | UiMessage::PerfSnapshot
                | UiMessage::FreeMemory
                | UiMessage::SetMode { .. }
                | UiMessage::SetSleeping { .. }
                | UiMessage::PageData
                | UiMessage::BookmarkRemove { .. }
                | UiMessage::BookmarkMove { .. }
                | UiMessage::FolderAdd { .. }
                | UiMessage::FolderRemove { .. }
                | UiMessage::HistorySearch { .. }
                | UiMessage::HistoryDelete { .. }
                | UiMessage::HistoryClear
                | UiMessage::DownloadCancel { .. }
                | UiMessage::DownloadShow { .. }
                | UiMessage::DownloadsClearFinished
                | UiMessage::UpdateSettings { .. }
        )
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TabView {
    pub id: u64,
    pub title: String,
    pub active: bool,
    pub loading: bool,
    /// Вкладка приостановлена или выгружена из памяти (показывается приглушённой).
    pub sleeping: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UiState {
    pub tabs: Vec<TabView>,
    pub address: String,
    pub security: Security,
    pub can_back: bool,
    pub can_forward: bool,
    pub loading: bool,
    /// Текущая страница есть в закладках (звёздочка закрашена).
    pub bookmarked: bool,
    /// Можно ли добавить текущую страницу в закладки (обычные http/https).
    pub can_bookmark: bool,
}

impl UiState {
    /// JS, который передаёт состояние интерфейсу. Данные — только через JSON, без склейки строк.
    pub fn to_script(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_else(|_| "null".into());
        format!("window.nexo&&window.nexo.setState({json});")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_messages() {
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"navigate","input":"rust lang"}"#),
            Some(UiMessage::Navigate {
                input: "rust lang".into()
            })
        );
        assert_eq!(UiMessage::parse(r#"{"cmd":"back"}"#), Some(UiMessage::Back));
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"reload"}"#),
            Some(UiMessage::Reload)
        );
    }

    #[test]
    fn parses_tab_messages() {
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"new_tab"}"#),
            Some(UiMessage::NewTab)
        );
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"close_tab","id":7}"#),
            Some(UiMessage::CloseTab { id: 7 })
        );
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"activate_tab","id":3}"#),
            Some(UiMessage::ActivateTab { id: 3 })
        );
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"move_tab","id":3,"index":0}"#),
            Some(UiMessage::MoveTab { id: 3, index: 0 })
        );
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"reopen_closed_tab"}"#),
            Some(UiMessage::ReopenClosedTab)
        );
        // Отрицательный или нечисловой id/index не принимаются.
        assert_eq!(UiMessage::parse(r#"{"cmd":"close_tab","id":-1}"#), None);
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"move_tab","id":1,"index":"x"}"#),
            None
        );
    }

    #[test]
    fn parses_memory_messages() {
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"perf_snapshot"}"#),
            Some(UiMessage::PerfSnapshot)
        );
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"free_memory"}"#),
            Some(UiMessage::FreeMemory)
        );
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"set_mode","mode":"MemorySaver"}"#),
            Some(UiMessage::SetMode {
                mode: PerformanceMode::MemorySaver
            })
        );
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"set_sleeping","enabled":false}"#),
            Some(UiMessage::SetSleeping { enabled: false })
        );
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"open_internal","page":"performance"}"#),
            Some(UiMessage::OpenInternal {
                page: "performance".into()
            })
        );
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"set_mode","mode":"Turbo"}"#),
            None
        );
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"set_sleeping","enabled":"yes"}"#),
            None
        );
    }

    #[test]
    fn parses_new_step6_messages() {
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"key","ctrl":true,"shift":false,"alt":false,"key":"t"}"#),
            Some(UiMessage::Key {
                ctrl: true,
                shift: false,
                alt: false,
                key: "t".into()
            })
        );
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"toggle_bookmark"}"#),
            Some(UiMessage::ToggleBookmark)
        );
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"bookmark_move","id":3,"folder":null}"#),
            Some(UiMessage::BookmarkMove {
                id: 3,
                folder: None
            })
        );
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"update_settings","theme":"Light","default_zoom_pct":125}"#),
            Some(UiMessage::UpdateSettings {
                theme: Some(ThemeChoice::Light),
                search_engine: None,
                restore_session: None,
                homepage: None,
                default_zoom_pct: Some(125),
            })
        );
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"update_settings","theme":"Neon"}"#),
            None
        );
        assert_eq!(
            UiMessage::parse(r#"{"cmd":"download_cancel","id":-1}"#),
            None
        );
    }

    #[test]
    fn pages_cannot_inject_key_presses_or_bookmark_from_the_toolbar() {
        assert!(!UiMessage::Key {
            ctrl: true,
            shift: false,
            alt: false,
            key: "t".into()
        }
        .allowed_from_page());
        assert!(!UiMessage::ToggleBookmark.allowed_from_page());
        assert!(UiMessage::HistoryClear.allowed_from_page());
        assert!(UiMessage::UpdateSettings {
            theme: None,
            search_engine: None,
            restore_session: None,
            homepage: None,
            default_zoom_pct: None
        }
        .allowed_from_page());
    }

    #[test]
    fn pages_may_manage_memory_but_nothing_else() {
        for ok in [
            UiMessage::PerfSnapshot,
            UiMessage::FreeMemory,
            UiMessage::SetMode {
                mode: PerformanceMode::Balanced,
            },
            UiMessage::SetSleeping { enabled: true },
        ] {
            assert!(ok.allowed_from_page(), "{ok:?}");
        }
        assert!(!UiMessage::OpenInternal {
            page: "settings".into()
        }
        .allowed_from_page());
        assert!(!UiMessage::MoveTab { id: 1, index: 0 }.allowed_from_page());
        assert!(!UiMessage::ReopenClosedTab.allowed_from_page());
    }

    #[test]
    fn rejects_garbage_and_unknown() {
        assert_eq!(UiMessage::parse("not json"), None);
        assert_eq!(UiMessage::parse(r#"{"cmd":"format_disk"}"#), None);
        assert_eq!(UiMessage::parse(r#"{"cmd":"navigate"}"#), None);
        assert_eq!(
            UiMessage::parse(&format!(
                r#"{{"cmd":"navigate","input":"{}"}}"#,
                "a".repeat(9000)
            )),
            None
        );
    }

    #[test]
    fn pages_may_only_navigate() {
        assert!(UiMessage::Navigate { input: "x".into() }.allowed_from_page());
        assert!(!UiMessage::Back.allowed_from_page());
        assert!(!UiMessage::Reload.allowed_from_page());
        // Страницы не могут управлять вкладками.
        assert!(!UiMessage::NewTab.allowed_from_page());
        assert!(!UiMessage::CloseTab { id: 1 }.allowed_from_page());
        assert!(!UiMessage::ActivateTab { id: 1 }.allowed_from_page());
    }

    #[test]
    fn state_script_is_valid_json_call() {
        let s = UiState {
            tabs: vec![TabView {
                id: 1,
                title: "a\"b</script>".into(),
                active: true,
                loading: false,
                sleeping: false,
            }],
            address: "https://x.com/".into(),
            security: Security::Secure,
            can_back: true,
            can_forward: false,
            loading: true,
            bookmarked: true,
            can_bookmark: true,
        };
        let js = s.to_script();
        assert!(js.starts_with("window.nexo&&window.nexo.setState("));
        let json = js
            .trim_start_matches("window.nexo&&window.nexo.setState(")
            .trim_end_matches(");");
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["security"], "secure");
        assert_eq!(v["tabs"][0]["title"], "a\"b</script>"); // кавычки экранированы JSON-ом
        assert_eq!(v["can_back"], true);
        assert_eq!(v["bookmarked"], true);
    }
}
