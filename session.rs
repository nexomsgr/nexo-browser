//! Сохранение и восстановление вкладок между запусками.
//! Приватные вкладки сюда не попадают — фильтрует вызывающий код.

use super::Db;
use anyhow::Result;
use rusqlite::params;

#[derive(Debug, Clone, PartialEq)]
pub struct SessionTab {
    pub url: String,
    pub title: String,
    pub pinned: bool,
    pub zoom: f64,
    pub active: bool,
}

impl Db {
    /// Атомарно заменяет сохранённую сессию.
    pub fn save_session(&self, tabs: &[SessionTab]) -> Result<()> {
        let tx = self.conn().unchecked_transaction()?;
        tx.execute("DELETE FROM session_tabs", [])?;
        {
            let mut st = tx.prepare(
                "INSERT INTO session_tabs(position, url, title, pinned, zoom, active)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for (i, t) in tabs.iter().enumerate() {
                st.execute(params![
                    i as i64, t.url, t.title, t.pinned, t.zoom, t.active
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_session(&self) -> Result<Vec<SessionTab>> {
        let mut st = self.conn().prepare(
            "SELECT url, title, pinned, zoom, active FROM session_tabs ORDER BY position",
        )?;
        let rows = st.query_map([], |r| {
            Ok(SessionTab {
                url: r.get(0)?,
                title: r.get(1)?,
                pinned: r.get(2)?,
                zoom: r.get(3)?,
                active: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(url: &str, active: bool) -> SessionTab {
        SessionTab {
            url: url.into(),
            title: format!("T {url}"),
            pinned: false,
            zoom: 1.25,
            active,
        }
    }

    #[test]
    fn roundtrip_preserves_order_and_fields() {
        let db = Db::open_in_memory().unwrap();
        let tabs = vec![
            t("https://a.com/", false),
            t("https://b.com/", true),
            t("https://c.com/", false),
        ];
        db.save_session(&tabs).unwrap();
        assert_eq!(db.load_session().unwrap(), tabs);
    }

    #[test]
    fn save_replaces_previous_session() {
        let db = Db::open_in_memory().unwrap();
        db.save_session(&[t("https://a.com/", true), t("https://b.com/", false)])
            .unwrap();
        db.save_session(&[t("https://z.com/", true)]).unwrap();
        let s = db.load_session().unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].url, "https://z.com/");
    }

    #[test]
    fn empty_session_is_empty() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.load_session().unwrap().is_empty());
        db.save_session(&[]).unwrap();
        assert!(db.load_session().unwrap().is_empty());
    }
}
