//! Разрешения сайтов (камера, микрофон, геолокация, уведомления и т.д.), хранятся по origin.
//!
//! Ограничение WebView2: диалог запроса разрешения показываем сами через событие
//! `PermissionRequested`; часть системных разрешений Windows (например, доступ к камере на уровне ОС)
//! этим слоем не управляется.

use crate::storage::Db;
use anyhow::Result;
use rusqlite::params;
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    Camera,
    Microphone,
    Geolocation,
    Notifications,
    ClipboardRead,
    Autoplay,
}

impl Permission {
    pub const ALL: [Permission; 6] = [
        Self::Camera,
        Self::Microphone,
        Self::Geolocation,
        Self::Notifications,
        Self::ClipboardRead,
        Self::Autoplay,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Camera => "camera",
            Self::Microphone => "microphone",
            Self::Geolocation => "geolocation",
            Self::Notifications => "notifications",
            Self::ClipboardRead => "clipboard_read",
            Self::Autoplay => "autoplay",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Ask,
    Allow,
    Deny,
}

impl Decision {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "allow" => Self::Allow,
            "deny" => Self::Deny,
            _ => Self::Ask,
        }
    }
}

/// `https://Example.com:443/a?b` → `https://example.com`. Для непрозрачных origin (file:, data:) — `None`.
pub fn origin_of(url: &str) -> Option<String> {
    let o = Url::parse(url).ok()?.origin();
    if o.is_tuple() {
        Some(o.ascii_serialization())
    } else {
        None
    }
}

impl Db {
    /// Решение по умолчанию — `Ask`.
    pub fn permission(&self, origin: &str, perm: Permission) -> Result<Decision> {
        let v: Option<String> = rusqlite::OptionalExtension::optional(self.conn().query_row(
            "SELECT decision FROM site_permissions WHERE origin = ?1 AND permission = ?2",
            params![origin, perm.as_str()],
            |r| r.get(0),
        ))?;
        Ok(v.map(|s| Decision::parse(&s)).unwrap_or(Decision::Ask))
    }

    /// `Decision::Ask` стирает запись (возврат к значению по умолчанию).
    pub fn set_permission(
        &self,
        origin: &str,
        perm: Permission,
        d: Decision,
        now_ms: u64,
    ) -> Result<()> {
        if d == Decision::Ask {
            self.conn().execute(
                "DELETE FROM site_permissions WHERE origin = ?1 AND permission = ?2",
                params![origin, perm.as_str()],
            )?;
            return Ok(());
        }
        self.conn().execute(
            "INSERT INTO site_permissions(origin, permission, decision, updated_ms)
             VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(origin, permission) DO UPDATE SET
                 decision = excluded.decision, updated_ms = excluded.updated_ms",
            params![origin, perm.as_str(), d.as_str(), now_ms as i64],
        )?;
        Ok(())
    }

    /// Все сохранённые решения (для страницы настроек «Разрешения сайтов»).
    pub fn list_permissions(&self) -> Result<Vec<(String, Permission, Decision)>> {
        let mut st = self.conn().prepare(
            "SELECT origin, permission, decision FROM site_permissions ORDER BY origin, permission",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (o, p, d) = row?;
            if let Some(p) = Permission::parse(&p) {
                out.push((o, p, Decision::parse(&d)));
            }
        }
        Ok(out)
    }

    pub fn reset_site_permissions(&self, origin: &str) -> Result<usize> {
        Ok(self
            .conn()
            .execute("DELETE FROM site_permissions WHERE origin = ?1", [origin])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_normalization() {
        assert_eq!(
            origin_of("https://Example.com:443/a?b#c").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            origin_of("http://localhost:3000/x").as_deref(),
            Some("http://localhost:3000")
        );
        assert_eq!(origin_of("file:///C:/a.html"), None);
        assert_eq!(origin_of("not a url"), None);
    }

    #[test]
    fn default_is_ask_then_set_and_reset() {
        let db = Db::open_in_memory().unwrap();
        let o = "https://meet.example";
        assert_eq!(db.permission(o, Permission::Camera).unwrap(), Decision::Ask);
        db.set_permission(o, Permission::Camera, Decision::Allow, 1)
            .unwrap();
        db.set_permission(o, Permission::Microphone, Decision::Deny, 1)
            .unwrap();
        assert_eq!(
            db.permission(o, Permission::Camera).unwrap(),
            Decision::Allow
        );
        assert_eq!(
            db.permission(o, Permission::Microphone).unwrap(),
            Decision::Deny
        );
        db.set_permission(o, Permission::Camera, Decision::Deny, 2)
            .unwrap();
        assert_eq!(
            db.permission(o, Permission::Camera).unwrap(),
            Decision::Deny
        );
        db.set_permission(o, Permission::Camera, Decision::Ask, 3)
            .unwrap();
        assert_eq!(db.permission(o, Permission::Camera).unwrap(), Decision::Ask);
        assert_eq!(db.list_permissions().unwrap().len(), 1);
        assert_eq!(db.reset_site_permissions(o).unwrap(), 1);
    }

    #[test]
    fn permissions_are_per_origin() {
        let db = Db::open_in_memory().unwrap();
        db.set_permission("https://a.com", Permission::Geolocation, Decision::Allow, 1)
            .unwrap();
        assert_eq!(
            db.permission("https://b.com", Permission::Geolocation)
                .unwrap(),
            Decision::Ask
        );
    }
}
