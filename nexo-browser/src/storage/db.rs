//! Открытие базы и миграции (через PRAGMA user_version).

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

const SCHEMA_V1: &str = "
CREATE TABLE bookmark_folders (
    id        INTEGER PRIMARY KEY,
    parent_id INTEGER REFERENCES bookmark_folders(id) ON DELETE CASCADE,
    name      TEXT NOT NULL,
    position  INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE bookmarks (
    id         INTEGER PRIMARY KEY,
    folder_id  INTEGER REFERENCES bookmark_folders(id) ON DELETE CASCADE,
    title      TEXT NOT NULL,
    url        TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    position   INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_bookmarks_url ON bookmarks(url);
CREATE TABLE history (
    id            INTEGER PRIMARY KEY,
    url           TEXT NOT NULL UNIQUE,
    title         TEXT NOT NULL DEFAULT '',
    visit_count   INTEGER NOT NULL DEFAULT 1,
    last_visit_ms INTEGER NOT NULL
);
CREATE INDEX idx_history_last ON history(last_visit_ms DESC);
CREATE TABLE session_tabs (
    position INTEGER PRIMARY KEY,
    url      TEXT NOT NULL,
    title    TEXT NOT NULL,
    pinned   INTEGER NOT NULL,
    zoom     REAL NOT NULL,
    active   INTEGER NOT NULL
);
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE site_permissions (
    origin     TEXT NOT NULL,
    permission TEXT NOT NULL,
    decision   TEXT NOT NULL,
    updated_ms INTEGER NOT NULL,
    PRIMARY KEY (origin, permission)
);
";

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("создание {}", dir.display()))?;
        }
        let conn =
            Connection::open(path).with_context(|| format!("открытие {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        // Небольшой кэш страниц: приоритет — низкое потребление RAM.
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -1024;",
        )?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(SCHEMA_V1)?;
            tx.execute_batch("PRAGMA user_version = 1;")?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_schema() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), 1);
        let n: i64 = db
            .conn()
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(n >= 6);
    }

    #[test]
    fn reopen_keeps_data_and_does_not_remigrate() {
        let dir = std::env::temp_dir().join(format!("nexo-test-{}", crate::util::now_ms()));
        let path = dir.join("browser.db");
        {
            let db = Db::open(&path).unwrap();
            db.set_setting("k", "v").unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(db.get_setting("k").unwrap().as_deref(), Some("v"));
        drop(db);
        let _ = std::fs::remove_dir_all(dir);
    }
}
