//! Таблицы хранилища паролей. Здесь только зашифрованные блоки: расшифровка — в `crate::vault`.

use super::Db;
use anyhow::Result;
use rusqlite::{params, OptionalExtension};

impl Db {
    pub fn vault_meta_get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM vault_meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn vault_meta_set(&self, key: &str, value: &[u8]) -> Result<()> {
        self.conn().execute(
            "INSERT INTO vault_meta(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn vault_items(&self) -> Result<Vec<(i64, Vec<u8>)>> {
        let mut st = self
            .conn()
            .prepare("SELECT id, data FROM vault_items ORDER BY id")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn vault_item_insert(&self, blob: &[u8], now_ms: u64) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO vault_items(data, updated_ms) VALUES(?1, ?2)",
            params![blob, now_ms as i64],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    pub fn vault_item_update(&self, id: i64, blob: &[u8], now_ms: u64) -> Result<bool> {
        Ok(self.conn().execute(
            "UPDATE vault_items SET data = ?2, updated_ms = ?3 WHERE id = ?1",
            params![id, blob, now_ms as i64],
        )? > 0)
    }

    pub fn vault_item_delete(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM vault_items WHERE id = ?1", [id])?
            > 0)
    }

    /// Полностью удаляет хранилище (забытый мастер-пароль восстановить невозможно).
    pub fn vault_wipe(&self) -> Result<()> {
        let tx = self.conn().unchecked_transaction()?;
        tx.execute("DELETE FROM vault_items", [])?;
        tx.execute("DELETE FROM vault_meta", [])?;
        tx.commit()?;
        Ok(())
    }

    /// Смена мастер-пароля: новые метаданные и перешифрованные записи записываются одной транзакцией —
    /// при сбое остаётся старое рабочее хранилище.
    pub fn vault_rekey(&self, meta: &[(&str, Vec<u8>)], items: &[(i64, Vec<u8>)], now_ms: u64) -> Result<()> {
        let tx = self.conn().unchecked_transaction()?;
        for (k, v) in meta {
            tx.execute(
                "INSERT INTO vault_meta(key, value) VALUES(?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![k, v],
            )?;
        }
        for (id, blob) in items {
            tx.execute(
                "UPDATE vault_items SET data = ?2, updated_ms = ?3 WHERE id = ?1",
                params![id, blob, now_ms as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
