//! История посещений. Одна строка на URL со счётчиком визитов.

use super::Db;
use anyhow::Result;
use rusqlite::params;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub visit_count: u32,
    pub last_visit_ms: u64,
}

fn is_recordable(url: &str) -> bool {
    let l = url.to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: r.get(0)?,
        url: r.get(1)?,
        title: r.get(2)?,
        visit_count: r.get::<_, i64>(3)? as u32,
        last_visit_ms: r.get::<_, i64>(4)? as u64,
    })
}

impl Db {
    /// Записывает визит. Внутренние страницы и нестандартные схемы не сохраняются (`Ok(false)`).
    pub fn record_visit(&self, url: &str, title: &str, now_ms: u64) -> Result<bool> {
        if !is_recordable(url) {
            return Ok(false);
        }
        self.conn().execute(
            "INSERT INTO history(url, title, visit_count, last_visit_ms) VALUES(?1, ?2, 1, ?3)
             ON CONFLICT(url) DO UPDATE SET
                 visit_count = visit_count + 1,
                 last_visit_ms = ?3,
                 title = CASE WHEN ?2 <> '' THEN ?2 ELSE title END",
            params![url, title, now_ms as i64],
        )?;
        Ok(true)
    }

    /// Заголовок страницы приходит позже навигации — обновляем без увеличения счётчика.
    pub fn update_history_title(&self, url: &str, title: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE history SET title = ?1 WHERE url = ?2",
            params![title, url],
        )?;
        Ok(())
    }

    pub fn recent_history(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        let mut st = self.conn().prepare(
            "SELECT id, url, title, visit_count, last_visit_ms FROM history
             ORDER BY last_visit_ms DESC, id DESC LIMIT ?1",
        )?;
        let rows = st.query_map([limit as i64], row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Поиск по URL и заголовку без учёта регистра (для ASCII), спецсимволы LIKE экранируются.
    pub fn search_history(&self, query: &str, limit: usize) -> Result<Vec<HistoryEntry>> {
        let escaped = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pat = format!("%{escaped}%");
        let mut st = self.conn().prepare(
            "SELECT id, url, title, visit_count, last_visit_ms FROM history
             WHERE url LIKE ?1 ESCAPE '\\' OR title LIKE ?1 ESCAPE '\\'
             ORDER BY last_visit_ms DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![pat, limit as i64], row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn delete_history_entry(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM history WHERE id = ?1", [id])?
            > 0)
    }

    pub fn clear_history(&self) -> Result<usize> {
        Ok(self.conn().execute("DELETE FROM history", [])?)
    }

    pub fn clear_history_older_than(&self, cutoff_ms: u64) -> Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM history WHERE last_visit_ms < ?1",
            [cutoff_ms as i64],
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visits_upsert_and_count() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.record_visit("https://a.com/", "A", 1).unwrap());
        assert!(db.record_visit("https://a.com/", "", 2).unwrap());
        let h = db.recent_history(10).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].visit_count, 2);
        assert_eq!(h[0].title, "A"); // пустой заголовок не затирает старый
        assert_eq!(h[0].last_visit_ms, 2);
    }

    #[test]
    fn internal_pages_are_not_recorded() {
        let db = Db::open_in_memory().unwrap();
        assert!(!db.record_visit("nexo://performance", "P", 1).unwrap());
        assert!(!db.record_visit("about:blank", "", 1).unwrap());
        assert!(!db.record_visit("file:///C:/x.txt", "", 1).unwrap());
        assert!(db.recent_history(10).unwrap().is_empty());
    }

    #[test]
    fn recent_is_ordered_newest_first() {
        let db = Db::open_in_memory().unwrap();
        db.record_visit("https://a.com/", "A", 10).unwrap();
        db.record_visit("https://b.com/", "B", 30).unwrap();
        db.record_visit("https://c.com/", "C", 20).unwrap();
        let urls: Vec<_> = db
            .recent_history(2)
            .unwrap()
            .into_iter()
            .map(|h| h.url)
            .collect();
        assert_eq!(urls, vec!["https://b.com/", "https://c.com/"]);
    }

    #[test]
    fn search_escapes_like_wildcards() {
        let db = Db::open_in_memory().unwrap();
        db.record_visit("https://a.com/100%_done", "Done", 1)
            .unwrap();
        db.record_visit("https://b.com/other", "Other", 2).unwrap();
        assert_eq!(db.search_history("100%", 10).unwrap().len(), 1);
        assert_eq!(db.search_history("%", 10).unwrap().len(), 1); // % ищется буквально
        assert_eq!(db.search_history("other", 10).unwrap().len(), 1);
        assert_eq!(db.search_history("OTHER", 10).unwrap().len(), 1);
    }

    #[test]
    fn delete_and_clear() {
        let db = Db::open_in_memory().unwrap();
        db.record_visit("https://a.com/", "A", 100).unwrap();
        db.record_visit("https://b.com/", "B", 200).unwrap();
        db.record_visit("https://c.com/", "C", 300).unwrap();
        assert_eq!(db.clear_history_older_than(250).unwrap(), 2);
        let id = db.recent_history(1).unwrap()[0].id;
        assert!(db.delete_history_entry(id).unwrap());
        assert_eq!(db.clear_history().unwrap(), 0);
    }

    #[test]
    fn title_update_keeps_count() {
        let db = Db::open_in_memory().unwrap();
        db.record_visit("https://a.com/", "", 1).unwrap();
        db.update_history_title("https://a.com/", "Late title")
            .unwrap();
        let h = &db.recent_history(1).unwrap()[0];
        assert_eq!((h.title.as_str(), h.visit_count), ("Late title", 1));
    }
}
