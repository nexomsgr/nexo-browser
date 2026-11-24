//! Настройки браузера: key-value в SQLite + типизированная структура `Settings` (JSON).

use super::Db;
use crate::browser::navigation::SearchEngine;
use crate::memory::policy::PerformanceMode;
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeChoice {
    System,
    #[default]
    Dark,
    Light,
}

/// Все поля с `#[serde(default)]`: старые сохранённые настройки читаются и после добавления новых полей.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    pub search_engine: SearchEngine,
    pub performance_mode: PerformanceMode,
    pub sleeping_enabled: bool,
    pub restore_session: bool,
    /// Пустая строка — открывать nexo://newtab.
    pub homepage: String,
    pub default_zoom: f64,
    pub download_dir: Option<String>,
    pub ask_download_location: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::Dark,
            search_engine: SearchEngine::Google,
            performance_mode: PerformanceMode::Balanced,
            sleeping_enabled: true,
            restore_session: true,
            homepage: String::new(),
            default_zoom: 1.0,
            download_dir: None,
            ask_download_location: false,
        }
    }
}

impl Settings {
    /// Приводит значения к допустимым (на случай ручной правки базы).
    pub fn sanitized(mut self) -> Self {
        if !self.default_zoom.is_finite() {
            self.default_zoom = 1.0;
        }
        self.default_zoom = self.default_zoom.clamp(0.25, 5.0);
        self
    }
}

const SETTINGS_KEY: &str = "settings";

impl Db {
    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Повреждённый JSON не ломает запуск: возвращаются значения по умолчанию.
    pub fn load_settings(&self) -> Result<Settings> {
        Ok(match self.get_setting(SETTINGS_KEY)? {
            Some(json) => serde_json::from_str::<Settings>(&json)
                .unwrap_or_default()
                .sanitized(),
            None => Settings::default(),
        })
    }

    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        self.set_setting(
            SETTINGS_KEY,
            &serde_json::to_string(&s.clone().sanitized())?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_empty() {
        let db = Db::open_in_memory().unwrap();
        let s = db.load_settings().unwrap();
        assert_eq!(s.performance_mode, PerformanceMode::Balanced);
        assert_eq!(s.theme, ThemeChoice::Dark);
        assert!(s.restore_session);
    }

    #[test]
    fn roundtrip() {
        let db = Db::open_in_memory().unwrap();
        let s = Settings {
            theme: ThemeChoice::Light,
            search_engine: SearchEngine::DuckDuckGo,
            performance_mode: PerformanceMode::MemorySaver,
            default_zoom: 1.5,
            ..Settings::default()
        };
        db.save_settings(&s).unwrap();
        assert_eq!(db.load_settings().unwrap(), s);
    }

    #[test]
    fn older_json_without_new_fields_still_loads() {
        let db = Db::open_in_memory().unwrap();
        db.set_setting("settings", r#"{"theme":"Light"}"#).unwrap();
        let s = db.load_settings().unwrap();
        assert_eq!(s.theme, ThemeChoice::Light);
        assert_eq!(s.performance_mode, PerformanceMode::Balanced);
    }

    #[test]
    fn corrupt_json_falls_back_to_defaults() {
        let db = Db::open_in_memory().unwrap();
        db.set_setting("settings", "{{not json").unwrap();
        assert_eq!(db.load_settings().unwrap(), Settings::default());
    }

    #[test]
    fn zoom_is_clamped() {
        let db = Db::open_in_memory().unwrap();
        db.save_settings(&Settings {
            default_zoom: 99.0,
            ..Settings::default()
        })
        .unwrap();
        assert_eq!(db.load_settings().unwrap().default_zoom, 5.0);
    }
}
