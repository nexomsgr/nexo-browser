//! Данные для внутренних страниц (закладки, история, загрузки, настройки) в виде JSON.
//! Страницы получают их вызовом `window.nexoPage.update(kind, data)`; всё, что пришло из сети
//! (заголовки, адреса), страницы выводят только как текст.

use crate::downloads::{DownloadState, Downloads};
use crate::storage::settings::{Settings, ThemeChoice};
use crate::storage::Db;
use serde_json::{json, Value};

const MAX_FOLDER_DEPTH: usize = 8;
const HISTORY_LIMIT: usize = 500;

pub fn bookmarks_json(db: &Db) -> Value {
    let mut folders = Vec::new();
    let mut bookmarks = Vec::new();

    let add_bookmarks = |folder: Option<i64>, out: &mut Vec<Value>| {
        for b in db.bookmarks(folder).unwrap_or_default() {
            out.push(json!({"id": b.id, "folder": b.folder_id, "title": b.title, "url": b.url}));
        }
    };

    add_bookmarks(None, &mut bookmarks);
    let mut level: Vec<Option<i64>> = vec![None];
    for _ in 0..MAX_FOLDER_DEPTH {
        let mut next = Vec::new();
        for parent in &level {
            for f in db.folders(*parent).unwrap_or_default() {
                folders.push(json!({"id": f.id, "parent": f.parent_id, "name": f.name}));
                add_bookmarks(Some(f.id), &mut bookmarks);
                next.push(Some(f.id));
            }
        }
        if next.is_empty() {
            break;
        }
        level = next;
    }
    json!({"folders": folders, "bookmarks": bookmarks})
}

pub fn history_json(db: &Db, query: &str) -> Value {
    let q = query.trim();
    let rows = if q.is_empty() {
        db.recent_history(HISTORY_LIMIT)
    } else {
        db.search_history(q, HISTORY_LIMIT)
    };
    let entries: Vec<Value> = rows
        .unwrap_or_default()
        .into_iter()
        .map(|e| json!({"id": e.id, "url": e.url, "title": e.title, "visits": e.visit_count, "last_ms": e.last_visit_ms}))
        .collect();
    json!({"query": q, "entries": entries})
}

/// Плитки популярных сайтов для новой вкладки.
pub fn topsites_json(db: &Db) -> Value {
    let sites: Vec<Value> = db
        .top_history(8)
        .unwrap_or_default()
        .into_iter()
        .map(|e| json!({"url": e.url, "title": e.title}))
        .collect();
    json!({"sites": sites})
}

/// Состояние менеджера паролей для страницы. Пароли сюда НЕ попадают — только по запросу `vault_reveal`.
pub fn vault_json(available: bool, initialized: bool, vault: &crate::vault::Vault) -> Value {
    let items: Vec<Value> = vault
        .items()
        .map(|list| {
            list.iter()
                .map(|c| {
                    json!({"id": c.id, "name": c.name, "url": c.url, "username": c.username,
                           "note": c.note, "updated_ms": c.updated_ms})
                })
                .collect()
        })
        .unwrap_or_default();
    json!({"available": available, "initialized": initialized, "unlocked": vault.is_unlocked(), "items": items})
}

pub fn downloads_json(d: &Downloads) -> Value {
    let items: Vec<Value> = d
        .items()
        .iter()
        .rev()
        .map(|i| {
            let state = match i.state {
                DownloadState::InProgress => "in_progress",
                DownloadState::Completed => "completed",
                DownloadState::Cancelled => "cancelled",
                DownloadState::Failed => "failed",
            };
            json!({"id": i.id, "name": i.file_name(), "url": i.url, "path": i.path,
                   "received": i.received, "total": i.total, "progress": i.progress(), "state": state})
        })
        .collect();
    json!({"items": items})
}

pub fn settings_json(s: &Settings) -> Value {
    json!({
        "theme": match s.theme { ThemeChoice::System => "System", ThemeChoice::Dark => "Dark", ThemeChoice::Light => "Light" },
        "search_engine": serde_json::to_value(s.search_engine).unwrap_or(Value::Null),
        "restore_session": s.restore_session,
        "homepage": s.homepage,
        "default_zoom_pct": (s.default_zoom * 100.0).round() as u32,
    })
}

/// JS, который передаёт данные странице. `kind` — фиксированная строка из этого модуля.
pub fn to_script(kind: &str, data: &Value) -> String {
    let json = serde_json::to_string(data).unwrap_or_else(|_| "null".into());
    format!("window.nexoPage&&window.nexoPage.update({kind:?},{json});")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmarks_include_nested_folders_and_root_items() {
        let db = Db::open_in_memory().unwrap();
        let f = db.add_folder(None, "Работа").unwrap();
        let g = db.add_folder(Some(f), "Docs").unwrap();
        db.add_bookmark(None, "Root", "https://root.example/", 1)
            .unwrap();
        db.add_bookmark(Some(g), "Deep", "https://deep.example/", 2)
            .unwrap();
        let v = bookmarks_json(&db);
        assert_eq!(v["folders"].as_array().unwrap().len(), 2);
        assert_eq!(v["folders"][1]["parent"], f);
        let titles: Vec<_> = v["bookmarks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["title"].as_str().unwrap())
            .collect();
        assert_eq!(titles, ["Root", "Deep"]);
        assert_eq!(v["bookmarks"][1]["folder"], g);
    }

    #[test]
    fn history_lists_recent_and_filters_by_query() {
        let db = Db::open_in_memory().unwrap();
        db.record_visit("https://rust-lang.org/", "Rust", 10)
            .unwrap();
        db.record_visit("https://example.com/", "Example", 20)
            .unwrap();
        let all = history_json(&db, "");
        assert_eq!(all["entries"].as_array().unwrap().len(), 2);
        assert_eq!(all["entries"][0]["title"], "Example"); // последние — первыми
        let found = history_json(&db, "  rust ");
        assert_eq!(found["query"], "rust");
        assert_eq!(found["entries"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn top_sites_are_ordered_by_visits() {
        let db = Db::open_in_memory().unwrap();
        db.record_visit("https://a.example/", "A", 1).unwrap();
        for t in 2..5 {
            db.record_visit("https://b.example/", "B", t).unwrap();
        }
        let v = topsites_json(&db);
        assert_eq!(v["sites"][0]["url"], "https://b.example/");
        assert_eq!(v["sites"].as_array().unwrap().len(), 2);
        assert_eq!(topsites_json(&Db::open_in_memory().unwrap())["sites"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn downloads_report_real_progress_and_state() {
        let mut d = Downloads::new();
        let a = d.start(
            "https://a/x.zip".into(),
            "C:\\D\\x.zip".into(),
            Some(200),
            0,
        );
        d.update(a, 50, Some(200));
        let b = d.start("https://a/y.zip".into(), "y.zip".into(), None, 0);
        d.fail(b);
        let v = downloads_json(&d);
        assert_eq!(v["items"][0]["state"], "failed"); // новые — первыми
        assert_eq!(v["items"][1]["name"], "x.zip");
        assert_eq!(v["items"][1]["progress"], 0.25);
        assert_eq!(v["items"][1]["state"], "in_progress");
        assert!(v["items"][0]["total"].is_null());
    }

    #[test]
    fn settings_json_uses_percent_zoom() {
        let s = Settings {
            default_zoom: 1.25,
            ..Settings::default()
        };
        let v = settings_json(&s);
        assert_eq!(v["default_zoom_pct"], 125);
        assert_eq!(v["theme"], "Dark");
        assert_eq!(v["search_engine"], "Google");
    }

    #[test]
    fn script_escapes_payload_as_json() {
        let js = to_script("history", &json!({"t": "</script><img onerror=1>\"'"}));
        assert!(js.starts_with("window.nexoPage&&window.nexoPage.update(\"history\","));
        // JSON-строка остаётся внутри кавычек: выйти из неё из заголовка страницы нельзя.
        let body = js
            .trim_start_matches("window.nexoPage&&window.nexoPage.update(\"history\",")
            .trim_end_matches(");");
        let back: Value = serde_json::from_str(body).unwrap();
        assert_eq!(back["t"], "</script><img onerror=1>\"'");
    }
}
