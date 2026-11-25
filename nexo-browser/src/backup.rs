//! Перенос данных между устройствами: экспорт и импорт закладок, истории и настроек одним JSON-файлом.
//! Пароли в этот файл НЕ попадают — для них отдельный экспорт CSV (требует мастер-пароль).
//!
//! Импорт — слияние: ничего не удаляется, повторы пропускаются. Файл считается недоверенным:
//! проверяются схема, размеры и допустимые адреса (только http/https).

use crate::storage::settings::Settings;
use crate::storage::Db;
use anyhow::{bail, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const FORMAT: &str = "nexo-backup";
pub const VERSION: u32 = 1;
pub const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
const MAX_ITEMS: usize = 200_000;
const MAX_DEPTH: usize = 16;
const HISTORY_EXPORT_LIMIT: usize = 100_000;

#[derive(Debug, Serialize, Deserialize)]
pub struct Backup {
    pub format: String,
    pub version: u32,
    pub exported_ms: u64,
    #[serde(default)]
    pub settings: Option<Settings>,
    #[serde(default)]
    pub folders: Vec<BFolder>,
    #[serde(default)]
    pub bookmarks: Vec<BBookmark>,
    #[serde(default)]
    pub history: Vec<BHistory>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BFolder {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BBookmark {
    pub folder_id: Option<i64>,
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub created_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BHistory {
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default = "one")]
    pub visit_count: u32,
    #[serde(default)]
    pub last_visit_ms: u64,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub folders: usize,
    pub bookmarks: usize,
    pub history: usize,
    pub settings: bool,
    pub skipped: usize,
}

fn web_url(u: &str) -> bool {
    let l = u.trim().to_ascii_lowercase();
    (l.starts_with("http://") || l.starts_with("https://")) && u.len() <= 4096
}

fn collect_folders(db: &Db, parent: Option<i64>, depth: usize, out: &mut Vec<BFolder>) -> Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    for f in db.folders(parent)? {
        out.push(BFolder { id: f.id, parent_id: f.parent_id, name: f.name.clone() });
        collect_folders(db, Some(f.id), depth + 1, out)?;
    }
    Ok(())
}

pub fn export(db: &Db, settings: &Settings, now_ms: u64) -> Result<String> {
    let mut folders = Vec::new();
    collect_folders(db, None, 0, &mut folders)?;
    let mut bookmarks = Vec::new();
    let mut scopes: Vec<Option<i64>> = vec![None];
    scopes.extend(folders.iter().map(|f| Some(f.id)));
    for scope in scopes {
        for b in db.bookmarks(scope)? {
            bookmarks.push(BBookmark { folder_id: b.folder_id, title: b.title, url: b.url, created_ms: b.created_ms });
        }
    }
    let history = db
        .recent_history(HISTORY_EXPORT_LIMIT)?
        .into_iter()
        .map(|h| BHistory { url: h.url, title: h.title, visit_count: h.visit_count, last_visit_ms: h.last_visit_ms })
        .collect();
    let mut s = settings.clone();
    s.download_dir = None; // путь относится к этому компьютеру
    let b = Backup {
        format: FORMAT.into(),
        version: VERSION,
        exported_ms: now_ms,
        settings: Some(s),
        folders,
        bookmarks,
        history,
    };
    Ok(serde_json::to_string_pretty(&b)?)
}

/// Результат разбора файла до записи в базу: ошибка формата не должна оставить половину данных.
pub fn parse(json: &str) -> Result<Backup> {
    if json.len() > MAX_FILE_BYTES {
        bail!("файл слишком большой");
    }
    let b: Backup = serde_json::from_str(json).map_err(|_| anyhow::anyhow!("это не файл резервной копии Nexo"))?;
    if b.format != FORMAT {
        bail!("это не файл резервной копии Nexo");
    }
    if b.version > VERSION {
        bail!("файл создан более новой версией Nexo (формат {}), обновите браузер", b.version);
    }
    if b.folders.len() + b.bookmarks.len() + b.history.len() > MAX_ITEMS {
        bail!("в файле слишком много записей");
    }
    Ok(b)
}

/// Применяет резервную копию к базе одной транзакцией. Возвращает отчёт и настройки (если они в файле),
/// которые вызывающий код применяет сам.
pub fn import(db: &Db, b: &Backup, apply_settings: bool) -> Result<(ImportReport, Option<Settings>)> {
    let tx = db.conn().unchecked_transaction()?;
    let mut rep = ImportReport::default();

    // Папки: в файле id произвольные. Сопоставляем со своими, повторно используя папку с тем же именем и родителем.
    let by_old: HashMap<i64, &BFolder> = b.folders.iter().map(|f| (f.id, f)).collect();
    let mut map: HashMap<i64, i64> = HashMap::new();
    for f in &b.folders {
        // цепочка родителей до корня; длина ограничена и защищает от циклов
        let mut chain = vec![f];
        let mut cur = f;
        let mut ok = true;
        while let Some(p) = cur.parent_id {
            match by_old.get(&p) {
                Some(pf) if chain.len() <= MAX_DEPTH && !chain.iter().any(|c| c.id == pf.id) => {
                    chain.push(pf);
                    cur = pf;
                }
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            rep.skipped += 1;
            continue;
        }
        let mut parent: Option<i64> = None;
        for node in chain.iter().rev() {
            let name: String = node.name.chars().take(200).collect();
            if let Some(&done) = map.get(&node.id) {
                parent = Some(done);
                continue;
            }
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT id FROM bookmark_folders WHERE name = ?1 AND parent_id IS ?2",
                    params![name, parent],
                    |r| r.get(0),
                )
                .ok();
            let id = match existing {
                Some(id) => id,
                None => {
                    let pos: i64 = tx.query_row(
                        "SELECT COALESCE(MAX(position) + 1, 0) FROM bookmark_folders WHERE parent_id IS ?1",
                        params![parent],
                        |r| r.get(0),
                    )?;
                    tx.execute(
                        "INSERT INTO bookmark_folders(parent_id, name, position) VALUES(?1, ?2, ?3)",
                        params![parent, name, pos],
                    )?;
                    rep.folders += 1;
                    tx.last_insert_rowid()
                }
            };
            map.insert(node.id, id);
            parent = Some(id);
        }
    }

    for bm in &b.bookmarks {
        if !web_url(&bm.url) {
            rep.skipped += 1;
            continue;
        }
        let folder = match bm.folder_id {
            None => None,
            Some(old) => match map.get(&old) {
                Some(&n) => Some(n),
                None => {
                    rep.skipped += 1;
                    continue;
                }
            },
        };
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM bookmarks WHERE url = ?1 AND folder_id IS ?2)",
            params![bm.url, folder],
            |r| r.get(0),
        )?;
        if exists {
            continue;
        }
        let pos: i64 = tx.query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM bookmarks WHERE folder_id IS ?1",
            params![folder],
            |r| r.get(0),
        )?;
        let title: String = bm.title.chars().take(500).collect();
        tx.execute(
            "INSERT INTO bookmarks(folder_id, title, url, created_ms, position) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![folder, title, bm.url, bm.created_ms as i64, pos],
        )?;
        rep.bookmarks += 1;
    }

    for h in &b.history {
        if !web_url(&h.url) {
            rep.skipped += 1;
            continue;
        }
        let title: String = h.title.chars().take(500).collect();
        let existed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM history WHERE url = ?1)", [&h.url], |r| r.get(0))?;
        tx.execute(
            "INSERT INTO history(url, title, visit_count, last_visit_ms) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(url) DO UPDATE SET
                 visit_count = MAX(visit_count, excluded.visit_count),
                 last_visit_ms = MAX(last_visit_ms, excluded.last_visit_ms),
                 title = CASE WHEN title = '' THEN excluded.title ELSE title END",
            params![h.url, title, h.visit_count.max(1) as i64, h.last_visit_ms as i64],
        )?;
        if !existed {
            rep.history += 1;
        }
    }
    tx.commit()?;

    let settings = if apply_settings { b.settings.clone() } else { None };
    if let Some(s) = &settings {
        let _ = s;
        rep.settings = true;
    }
    Ok((rep, settings.map(|mut s| {
        s.download_dir = None;
        s.sanitized()
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::settings::ThemeChoice;

    fn filled() -> Db {
        let db = Db::open_in_memory().unwrap();
        let work = db.add_folder(None, "Работа").unwrap();
        let sub = db.add_folder(Some(work), "Доки").unwrap();
        db.add_bookmark(None, "Корень", "https://root.example/", 10).unwrap();
        db.add_bookmark(Some(work), "Почта", "https://mail.example/", 11).unwrap();
        db.add_bookmark(Some(sub), "Wiki", "https://wiki.example/", 12).unwrap();
        db.record_visit("https://a.example/", "A", 100).unwrap();
        db.record_visit("https://a.example/", "A", 200).unwrap();
        db
    }

    #[test]
    fn export_import_moves_everything_to_another_device() {
        let src = filled();
        let s = Settings {
            theme: ThemeChoice::Light,
            download_dir: Some("C:\\Users\\me\\Dl".into()),
            ..Settings::default()
        };
        let json = export(&src, &s, 999).unwrap();
        assert!(!json.contains("C:\\\\Users"), "локальный путь не должен уезжать на другое устройство");

        let dst = Db::open_in_memory().unwrap();
        let (rep, st) = import(&dst, &parse(&json).unwrap(), true).unwrap();
        assert_eq!((rep.folders, rep.bookmarks, rep.history), (2, 3, 1));
        assert_eq!(st.unwrap().theme, ThemeChoice::Light);
        let folders = dst.folders(None).unwrap();
        assert_eq!(folders.len(), 1);
        let sub = dst.folders(Some(folders[0].id)).unwrap();
        assert_eq!(sub[0].name, "Доки");
        assert_eq!(dst.bookmarks(Some(sub[0].id)).unwrap()[0].url, "https://wiki.example/");
        let h = dst.recent_history(10).unwrap();
        assert_eq!((h[0].visit_count, h[0].last_visit_ms), (2, 200));
    }

    #[test]
    fn import_twice_is_idempotent_merge() {
        let src = filled();
        let json = export(&src, &Settings::default(), 1).unwrap();
        let dst = filled();
        let (rep, _) = import(&dst, &parse(&json).unwrap(), false).unwrap();
        assert_eq!((rep.folders, rep.bookmarks, rep.history), (0, 0, 0));
        assert!(!rep.settings);
        assert_eq!(dst.folders(None).unwrap().len(), 1);
    }

    #[test]
    fn rejects_foreign_files_and_newer_versions() {
        assert!(parse("{}").is_err());
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"format":"other","version":1,"exported_ms":0}"#).is_err());
        assert!(parse(r#"{"format":"nexo-backup","version":99,"exported_ms":0}"#).is_err());
        assert!(parse(r#"{"format":"nexo-backup","version":1,"exported_ms":0}"#).is_ok());
    }

    #[test]
    fn hostile_content_is_skipped_not_imported() {
        let json = r#"{"format":"nexo-backup","version":1,"exported_ms":0,
          "folders":[{"id":1,"parent_id":2,"name":"a"},{"id":2,"parent_id":1,"name":"b"}],
          "bookmarks":[{"folder_id":null,"title":"x","url":"javascript:alert(1)"},
                       {"folder_id":77,"title":"orphan","url":"https://ok.example/"},
                       {"folder_id":null,"title":"fine","url":"https://fine.example/"}],
          "history":[{"url":"data:text/html,hi","title":"","visit_count":1,"last_visit_ms":1}]}"#;
        let db = Db::open_in_memory().unwrap();
        let (rep, _) = import(&db, &parse(json).unwrap(), false).unwrap();
        assert_eq!(rep.bookmarks, 1);
        assert_eq!(rep.folders, 0, "папки с циклом пропускаются");
        assert_eq!(rep.history, 0);
        assert!(rep.skipped >= 4);
    }
}
