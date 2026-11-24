//! Модель вкладки.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(pub u64);

/// Состояние вкладки с точки зрения управления памятью.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabState {
    /// WebView2 создан и страница работает.
    Active,
    /// Фоновая вкладка: WebView2 жив, но страница приостановлена (TrySuspend).
    Suspended,
    /// WebView2 уничтожен, хранится только URL, заголовок и состояние форм.
    Discarded,
}

#[derive(Debug, Clone)]
pub struct TabInfo {
    pub id: TabId,
    pub url: String,
    pub title: String,
    pub favicon_url: Option<String>,
    pub state: TabState,
    pub private: bool,
    pub pinned: bool,
    pub zoom: f64,
    /// Когда вкладка последний раз была активной (или создана), мс с эпохи Unix.
    pub last_active_ms: u64,
    pub playing_audio: bool,
    /// Страница сообщила о несохранённом вводе в формах (заполняется JS-скриптом, шаг 4).
    pub has_unsaved_input: bool,
    pub downloading: bool,
    /// Страница сейчас загружается (для индикатора на вкладке).
    pub loading: bool,
}

impl TabInfo {
    pub fn new(id: TabId, url: String, private: bool, now_ms: u64) -> Self {
        Self {
            id,
            url,
            title: String::new(),
            favicon_url: None,
            state: TabState::Active,
            private,
            pinned: false,
            zoom: 1.0,
            last_active_ms: now_ms,
            playing_audio: false,
            has_unsaved_input: false,
            downloading: false,
            loading: false,
        }
    }

    /// Почему вкладку нельзя усыплять (`None` — можно). Порядок важен: это первая названная причина.
    pub fn protection_reason(&self) -> Option<&'static str> {
        if self.playing_audio {
            Some("audio")
        } else if self.downloading {
            Some("downloading")
        } else if self.has_unsaved_input {
            Some("input")
        } else if self.loading {
            Some("loading")
        } else {
            None
        }
    }

    /// Вкладку нельзя усыплять: звук, загрузка файла, несохранённый ввод или идущая загрузка страницы.
    pub fn is_protected(&self) -> bool {
        self.protection_reason().is_some()
    }

    /// Для вкладки существует живой WebView2.
    pub fn is_live(&self) -> bool {
        self.state != TabState::Discarded
    }

    pub fn display_title(&self) -> &str {
        if self.title.is_empty() {
            &self.url
        } else {
            &self.title
        }
    }
}
