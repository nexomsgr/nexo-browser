//! Локальное хранилище на SQLite (rusqlite, bundled).
//! Приватные окна базу НЕ используют: история, сессия и разрешения там не пишутся
//! (это обеспечивает вызывающий код — приватной вкладке просто не передаётся `Db`).
pub mod bookmarks;
pub mod db;
pub mod history;
pub mod session;
pub mod settings;

pub use db::Db;
