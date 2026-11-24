//! Закладки и папки закладок. `folder_id = None` означает корень.

use super::Db;
use anyhow::Result;
use rusqlite::{params, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub id: i64,
    pub folder_id: Option<i64>,
    pub title: String,
    pub url: String,
    pub created_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
}

impl Db {
    pub fn add_folder(&self, parent: Option<i64>, name: &str) -> Result<i64> {
        let pos = self.next_position("bookmark_folders", "parent_id", parent)?;
        self.conn().execute(
            "INSERT INTO bookmark_folders(parent_id, name, position) VALUES(?1, ?2, ?3)",
            params![parent, name, pos],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    /// Удаляет папку вместе с вложенными папками и закладками (ON DELETE CASCADE).
    pub fn remove_folder(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM bookmark_folders WHERE id = ?1", [id])?
            > 0)
    }

    pub fn folders(&self, parent: Option<i64>) -> Result<Vec<Folder>> {
        let mut st = self.conn().prepare(
            "SELECT id, parent_id, name FROM bookmark_folders
             WHERE parent_id IS ?1 ORDER BY position, id",
        )?;
        let rows = st.query_map([parent], |r| {
            Ok(Folder {
                id: r.get(0)?,
                parent_id: r.get(1)?,
                name: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn add_bookmark(
        &self,
        folder: Option<i64>,
        title: &str,
        url: &str,
        now_ms: u64,
    ) -> Result<i64> {
        let pos = self.next_position("bookmarks", "folder_id", folder)?;
        self.conn().execute(
            "INSERT INTO bookmarks(folder_id, title, url, created_ms, position)
             VALUES(?1, ?2, ?3, ?4, ?5)",
            params![folder, title, url, now_ms as i64, pos],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    pub fn remove_bookmark(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM bookmarks WHERE id = ?1", [id])?
            > 0)
    }

    pub fn bookmarks(&self, folder: Option<i64>) -> Result<Vec<Bookmark>> {
        let mut st = self.conn().prepare(
            "SELECT id, folder_id, title, url, created_ms FROM bookmarks
             WHERE folder_id IS ?1 ORDER BY position, id",
        )?;
        let rows = st.query_map([folder], |r| {
            Ok(Bookmark {
                id: r.get(0)?,
                folder_id: r.get(1)?,
                title: r.get(2)?,
                url: r.get(3)?,
                created_ms: r.get::<_, i64>(4)? as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn find_bookmark_by_url(&self, url: &str) -> Result<Option<Bookmark>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, folder_id, title, url, created_ms FROM bookmarks WHERE url = ?1 LIMIT 1",
                [url],
                |r| {
                    Ok(Bookmark {
                        id: r.get(0)?,
                        folder_id: r.get(1)?,
                        title: r.get(2)?,
                        url: r.get(3)?,
                        created_ms: r.get::<_, i64>(4)? as u64,
                    })
                },
            )
            .optional()?)
    }

    pub fn move_bookmark(&self, id: i64, folder: Option<i64>) -> Result<bool> {
        let pos = self.next_position("bookmarks", "folder_id", folder)?;
        Ok(self.conn().execute(
            "UPDATE bookmarks SET folder_id = ?1, position = ?2 WHERE id = ?3",
            params![folder, pos, id],
        )? > 0)
    }

    pub fn rename_bookmark(&self, id: i64, title: &str) -> Result<bool> {
        Ok(self.conn().execute(
            "UPDATE bookmarks SET title = ?1 WHERE id = ?2",
            params![title, id],
        )? > 0)
    }

    /// `table` и `col` — только внутренние константы, не пользовательский ввод.
    fn next_position(&self, table: &str, col: &str, parent: Option<i64>) -> Result<i64> {
        let sql = format!("SELECT COALESCE(MAX(position) + 1, 0) FROM {table} WHERE {col} IS ?1");
        Ok(self.conn().query_row(&sql, [parent], |r| r.get(0))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_list_remove() {
        let db = Db::open_in_memory().unwrap();
        let a = db.add_bookmark(None, "A", "https://a.com/", 1).unwrap();
        let b = db.add_bookmark(None, "B", "https://b.com/", 2).unwrap();
        let list = db.bookmarks(None).unwrap();
        assert_eq!(list.iter().map(|x| x.id).collect::<Vec<_>>(), vec![a, b]);
        assert!(db.find_bookmark_by_url("https://b.com/").unwrap().is_some());
        assert!(db.remove_bookmark(a).unwrap());
        assert_eq!(db.bookmarks(None).unwrap().len(), 1);
        assert!(!db.remove_bookmark(a).unwrap());
    }

    #[test]
    fn folders_nest_and_cascade() {
        let db = Db::open_in_memory().unwrap();
        let work = db.add_folder(None, "Работа").unwrap();
        let sub = db.add_folder(Some(work), "Docs").unwrap();
        db.add_bookmark(Some(sub), "Spec", "https://spec.dev/", 1)
            .unwrap();
        assert_eq!(db.folders(None).unwrap().len(), 1);
        assert_eq!(db.folders(Some(work)).unwrap()[0].name, "Docs");
        assert_eq!(db.bookmarks(Some(sub)).unwrap().len(), 1);
        assert!(db.remove_folder(work).unwrap());
        assert!(db
            .find_bookmark_by_url("https://spec.dev/")
            .unwrap()
            .is_none());
        assert!(db.folders(Some(work)).unwrap().is_empty());
    }

    #[test]
    fn move_and_rename() {
        let db = Db::open_in_memory().unwrap();
        let f = db.add_folder(None, "F").unwrap();
        let id = db.add_bookmark(None, "Old", "https://x.com/", 1).unwrap();
        assert!(db.move_bookmark(id, Some(f)).unwrap());
        assert!(db.bookmarks(None).unwrap().is_empty());
        assert!(db.rename_bookmark(id, "New").unwrap());
        assert_eq!(db.bookmarks(Some(f)).unwrap()[0].title, "New");
    }
}
