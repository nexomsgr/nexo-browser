//! Менеджер паролей Nexo. Хранилище шифруется мастер-паролем (Argon2id → AES-256-GCM).
//!
//! * На диске — только шифртекст; в памяти расшифрованные данные живут, пока хранилище разблокировано.
//! * Автоблокировка через 10 минут без обращений; ключ и пароли стираются из памяти при блокировке.
//! * Забытый мастер-пароль восстановить невозможно (его нет нигде, кроме головы пользователя).

pub mod crypto;
pub mod csv;

use crate::storage::Db;
use anyhow::Result;
use crypto::{KdfParams, Key};
use serde::{Deserialize, Serialize};
use std::fmt;
use zeroize::Zeroize;

pub const AUTO_LOCK_MS: u64 = 10 * 60 * 1000;
pub const MIN_MASTER_CHARS: usize = 8;
const MAX_ITEMS: usize = 20_000;
const VERIFIER_PLAIN: &[u8] = b"nexo-vault-ok";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Credential {
    #[serde(skip)]
    pub id: i64,
    pub name: String,
    pub url: String,
    pub username: String,
    pub password: String,
    pub note: String,
    pub created_ms: u64,
    pub updated_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewCredential {
    pub name: String,
    pub url: String,
    pub username: String,
    pub password: String,
    pub note: String,
}

#[derive(Debug)]
pub enum VaultError {
    Locked,
    NotInitialized,
    AlreadyInitialized,
    WrongPassword,
    WeakPassword,
    Invalid(&'static str),
    NotFound,
    Storage(String),
}

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locked => write!(f, "Хранилище заблокировано"),
            Self::NotInitialized => write!(f, "Хранилище ещё не создано"),
            Self::AlreadyInitialized => write!(f, "Хранилище уже создано"),
            Self::WrongPassword => write!(f, "Неверный мастер-пароль"),
            Self::WeakPassword => write!(f, "Мастер-пароль должен быть не короче {MIN_MASTER_CHARS} символов"),
            Self::Invalid(m) => write!(f, "{m}"),
            Self::NotFound => write!(f, "Запись не найдена"),
            Self::Storage(m) => write!(f, "Ошибка хранилища: {m}"),
        }
    }
}

impl From<anyhow::Error> for VaultError {
    fn from(e: anyhow::Error) -> Self {
        Self::Storage(e.to_string())
    }
}

type R<T> = std::result::Result<T, VaultError>;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportStats {
    pub added: usize,
    pub duplicates: usize,
}

#[derive(Default)]
pub struct Vault {
    key: Option<Key>,
    items: Vec<Credential>,
    last_used_ms: u64,
}

fn validate(c: &NewCredential) -> R<()> {
    if c.password.is_empty() {
        return Err(VaultError::Invalid("Пароль не может быть пустым"));
    }
    if c.url.trim().is_empty() && c.name.trim().is_empty() {
        return Err(VaultError::Invalid("Укажите адрес сайта или название"));
    }
    let too_long = c.name.chars().count() > 200
        || c.url.chars().count() > 2048
        || c.username.chars().count() > 256
        || c.password.chars().count() > 1024
        || c.note.chars().count() > 4000;
    if too_long {
        return Err(VaultError::Invalid("Одно из полей слишком длинное"));
    }
    Ok(())
}

impl Vault {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_unlocked(&self) -> bool {
        self.key.is_some()
    }

    pub fn is_initialized(db: &Db) -> Result<bool> {
        Ok(db.vault_meta_get("verifier")?.is_some())
    }

    /// Вызывается перед каждой операцией: блокирует хранилище после простоя, иначе продлевает сеанс.
    pub fn refresh(&mut self, now_ms: u64) {
        if self.is_unlocked() {
            if now_ms.saturating_sub(self.last_used_ms) > AUTO_LOCK_MS {
                self.lock();
            } else {
                self.last_used_ms = now_ms;
            }
        }
    }

    /// Блокирует хранилище, если оно простаивает дольше лимита, не продлевая сеанс. `true` — только что заблокировано.
    pub fn expire(&mut self, now_ms: u64) -> bool {
        if self.is_unlocked() && now_ms.saturating_sub(self.last_used_ms) > AUTO_LOCK_MS {
            self.lock();
            return true;
        }
        false
    }

    pub fn lock(&mut self) {
        for c in &mut self.items {
            c.password.zeroize();
            c.note.zeroize();
            c.username.zeroize();
        }
        self.items.clear();
        self.key = None; // Zeroizing стирает ключ
    }

    pub fn create(&mut self, db: &Db, master: &str, now_ms: u64) -> R<()> {
        if Self::is_initialized(db)? {
            return Err(VaultError::AlreadyInitialized);
        }
        if master.chars().count() < MIN_MASTER_CHARS {
            return Err(VaultError::WeakPassword);
        }
        let salt: [u8; crypto::SALT_LEN] = crypto::random()?;
        let params = KdfParams::DEFAULT;
        let key = crypto::derive_key(master, &salt, params)?;
        let verifier = crypto::seal(&key, VERIFIER_PLAIN)?;
        db.vault_meta_set("salt", &salt)?;
        db.vault_meta_set("kdf", &params.to_bytes())?;
        db.vault_meta_set("verifier", &verifier)?;
        self.key = Some(key);
        self.items.clear();
        self.last_used_ms = now_ms;
        Ok(())
    }

    fn derive_checked(db: &Db, master: &str) -> R<Key> {
        let (Some(salt), Some(kdf), Some(verifier)) = (
            db.vault_meta_get("salt")?,
            db.vault_meta_get("kdf")?,
            db.vault_meta_get("verifier")?,
        ) else {
            return Err(VaultError::NotInitialized);
        };
        let key = crypto::derive_key(master, &salt, KdfParams::from_bytes(&kdf)?)?;
        match crypto::open(&key, &verifier) {
            Ok(p) if &p[..] == VERIFIER_PLAIN => Ok(key),
            _ => Err(VaultError::WrongPassword),
        }
    }

    pub fn unlock(&mut self, db: &Db, master: &str, now_ms: u64) -> R<()> {
        let key = Self::derive_checked(db, master)?;
        let mut items = Vec::new();
        for (id, blob) in db.vault_items()? {
            // Запись, которую не удалось расшифровать, пропускаем, а не роняем всё хранилище.
            let Ok(plain) = crypto::open(&key, &blob) else { continue };
            if let Ok(mut c) = serde_json::from_slice::<Credential>(&plain) {
                c.id = id;
                items.push(c);
            }
        }
        self.lock();
        self.items = items;
        self.key = Some(key);
        self.last_used_ms = now_ms;
        Ok(())
    }

    /// Повторная проверка мастер-пароля для опасных действий (экспорт открытым текстом, смена пароля).
    pub fn verify(&self, db: &Db, master: &str) -> R<()> {
        if !self.is_unlocked() {
            return Err(VaultError::Locked);
        }
        Self::derive_checked(db, master).map(|_| ())
    }

    pub fn items(&self) -> R<&[Credential]> {
        if self.is_unlocked() {
            Ok(&self.items)
        } else {
            Err(VaultError::Locked)
        }
    }

    pub fn password_of(&self, id: i64) -> R<&str> {
        self.items()?
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.password.as_str())
            .ok_or(VaultError::NotFound)
    }

    fn encrypt_item(key: &Key, c: &Credential) -> R<Vec<u8>> {
        let mut json = serde_json::to_vec(c).map_err(|e| VaultError::Storage(e.to_string()))?;
        let blob = crypto::seal(key, &json);
        json.zeroize();
        Ok(blob?)
    }

    /// Добавляет (`id = None`) или изменяет запись.
    pub fn save(&mut self, db: &Db, id: Option<i64>, input: NewCredential, now_ms: u64) -> R<i64> {
        let key = self.key.as_ref().ok_or(VaultError::Locked)?;
        validate(&input)?;
        match id {
            Some(id) => {
                let pos = self.items.iter().position(|c| c.id == id).ok_or(VaultError::NotFound)?;
                let mut c = self.items[pos].clone();
                c.name = input.name;
                c.url = input.url;
                c.username = input.username;
                c.password = input.password;
                c.note = input.note;
                c.updated_ms = now_ms;
                let blob = Self::encrypt_item(key, &c)?;
                if !db.vault_item_update(id, &blob, now_ms)? {
                    return Err(VaultError::NotFound);
                }
                self.items[pos] = c;
                Ok(id)
            }
            None => {
                if self.items.len() >= MAX_ITEMS {
                    return Err(VaultError::Invalid("Достигнут предел записей"));
                }
                let mut c = Credential {
                    id: 0,
                    name: input.name,
                    url: input.url,
                    username: input.username,
                    password: input.password,
                    note: input.note,
                    created_ms: now_ms,
                    updated_ms: now_ms,
                };
                let blob = Self::encrypt_item(key, &c)?;
                c.id = db.vault_item_insert(&blob, now_ms)?;
                let id = c.id;
                self.items.push(c);
                Ok(id)
            }
        }
    }

    pub fn delete(&mut self, db: &Db, id: i64) -> R<()> {
        if !self.is_unlocked() {
            return Err(VaultError::Locked);
        }
        db.vault_item_delete(id)?;
        if let Some(pos) = self.items.iter().position(|c| c.id == id) {
            let mut c = self.items.remove(pos);
            c.password.zeroize();
        }
        Ok(())
    }

    /// Импорт: записи, которые уже есть (тот же адрес, логин и пароль), пропускаются.
    pub fn import(&mut self, db: &Db, list: Vec<NewCredential>, now_ms: u64) -> R<ImportStats> {
        if !self.is_unlocked() {
            return Err(VaultError::Locked);
        }
        let mut stats = ImportStats::default();
        for n in list {
            if validate(&n).is_err() {
                continue;
            }
            let dup = self
                .items
                .iter()
                .any(|c| c.url == n.url && c.username == n.username && c.password == n.password);
            if dup {
                stats.duplicates += 1;
                continue;
            }
            self.save(db, None, n, now_ms)?;
            stats.added += 1;
        }
        Ok(stats)
    }

    pub fn export_csv(&self) -> R<String> {
        let items = self.items()?;
        let mut rows: Vec<Vec<String>> = vec![csv::EXPORT_HEADER.iter().map(|s| s.to_string()).collect()];
        for c in items {
            rows.push(vec![
                c.name.clone(),
                c.url.clone(),
                c.username.clone(),
                c.password.clone(),
                c.note.clone(),
            ]);
        }
        Ok(csv::write(&rows))
    }

    pub fn change_master(&mut self, db: &Db, old: &str, new: &str, now_ms: u64) -> R<()> {
        if !self.is_unlocked() {
            return Err(VaultError::Locked);
        }
        Self::derive_checked(db, old)?;
        if new.chars().count() < MIN_MASTER_CHARS {
            return Err(VaultError::WeakPassword);
        }
        let salt: [u8; crypto::SALT_LEN] = crypto::random()?;
        let params = KdfParams::DEFAULT;
        let key = crypto::derive_key(new, &salt, params)?;
        let verifier = crypto::seal(&key, VERIFIER_PLAIN)?;
        let mut blobs = Vec::with_capacity(self.items.len());
        for c in &self.items {
            blobs.push((c.id, Self::encrypt_item(&key, c)?));
        }
        db.vault_rekey(
            &[("salt", salt.to_vec()), ("kdf", params.to_bytes().to_vec()), ("verifier", verifier)],
            &blobs,
            now_ms,
        )?;
        self.key = Some(key);
        self.last_used_ms = now_ms;
        Ok(())
    }

    /// Забытый пароль: единственный выход — удалить хранилище целиком.
    pub fn reset(&mut self, db: &Db) -> R<()> {
        self.lock();
        db.vault_wipe()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nc(url: &str, user: &str, pw: &str) -> NewCredential {
        NewCredential {
            name: url.into(),
            url: url.into(),
            username: user.into(),
            password: pw.into(),
            note: String::new(),
        }
    }

    // Argon2 с реальными параметрами в тестах медленный, поэтому создаём хранилище вручную быстрым ключом
    // там, где важна не стойкость, а логика. Но один тест проходит полный путь с боевыми параметрами.
    #[test]
    fn full_lifecycle_with_real_parameters() {
        let db = Db::open_in_memory().unwrap();
        let mut v = Vault::new();
        assert!(!Vault::is_initialized(&db).unwrap());
        assert!(matches!(v.create(&db, "short", 0), Err(VaultError::WeakPassword)));
        v.create(&db, "correct horse battery", 1).unwrap();
        assert!(matches!(v.create(&db, "another long one", 1), Err(VaultError::AlreadyInitialized)));
        let id = v.save(&db, None, nc("https://a.com", "me", "pw1"), 2).unwrap();
        v.lock();
        assert!(matches!(v.items(), Err(VaultError::Locked)));
        assert!(matches!(v.unlock(&db, "wrong password!", 3), Err(VaultError::WrongPassword)));
        v.unlock(&db, "correct horse battery", 3).unwrap();
        assert_eq!(v.items().unwrap().len(), 1);
        assert_eq!(v.password_of(id).unwrap(), "pw1");
        v.save(&db, Some(id), nc("https://a.com", "me", "pw2"), 4).unwrap();
        assert_eq!(v.password_of(id).unwrap(), "pw2");
        assert_eq!(v.items().unwrap()[0].created_ms, 2);
        v.delete(&db, id).unwrap();
        assert!(v.items().unwrap().is_empty());
    }

    #[test]
    fn nothing_readable_is_stored_on_disk() {
        let db = Db::open_in_memory().unwrap();
        let mut v = Vault::new();
        v.create(&db, "master-password-1", 0).unwrap();
        v.save(&db, None, nc("https://secret-site.example", "alice", "SuperSecret!42"), 1).unwrap();
        let (_, blob) = db.vault_items().unwrap().remove(0);
        for needle in ["secret-site", "alice", "SuperSecret"] {
            assert!(!blob.windows(needle.len()).any(|w| w == needle.as_bytes()), "{needle} в открытом виде");
        }
    }

    #[test]
    fn auto_lock_after_idle() {
        let db = Db::open_in_memory().unwrap();
        let mut v = Vault::new();
        v.create(&db, "master-password-1", 0).unwrap();
        v.refresh(AUTO_LOCK_MS - 1);
        assert!(v.is_unlocked());
        v.refresh(AUTO_LOCK_MS - 1 + AUTO_LOCK_MS + 1);
        assert!(!v.is_unlocked());
    }

    #[test]
    fn import_skips_duplicates_and_invalid_and_export_roundtrips() {
        let db = Db::open_in_memory().unwrap();
        let mut v = Vault::new();
        v.create(&db, "master-password-1", 0).unwrap();
        v.save(&db, None, nc("https://a.com", "u", "p"), 1).unwrap();
        let st = v
            .import(&db, vec![nc("https://a.com", "u", "p"), nc("https://b.com", "u", "q"), nc("https://c.com", "u", "")], 2)
            .unwrap();
        assert_eq!(st, ImportStats { added: 1, duplicates: 1 });
        let csv = v.export_csv().unwrap();
        let parsed = csv::credentials_from_csv(&csv).unwrap();
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(parsed.items[1].password, "q");
    }

    #[test]
    fn change_master_reencrypts_everything() {
        let db = Db::open_in_memory().unwrap();
        let mut v = Vault::new();
        v.create(&db, "old-master-password", 0).unwrap();
        v.save(&db, None, nc("https://a.com", "u", "p"), 1).unwrap();
        assert!(matches!(v.change_master(&db, "not-the-old-one", "new-master-password", 2), Err(VaultError::WrongPassword)));
        v.change_master(&db, "old-master-password", "new-master-password", 2).unwrap();
        v.lock();
        assert!(matches!(v.unlock(&db, "old-master-password", 3), Err(VaultError::WrongPassword)));
        v.unlock(&db, "new-master-password", 3).unwrap();
        assert_eq!(v.items().unwrap()[0].password, "p");
    }

    #[test]
    fn reset_wipes_everything() {
        let db = Db::open_in_memory().unwrap();
        let mut v = Vault::new();
        v.create(&db, "master-password-1", 0).unwrap();
        v.save(&db, None, nc("https://a.com", "u", "p"), 1).unwrap();
        v.reset(&db).unwrap();
        assert!(!Vault::is_initialized(&db).unwrap());
        assert!(db.vault_items().unwrap().is_empty());
    }

    #[test]
    fn validation_rejects_bad_input() {
        let db = Db::open_in_memory().unwrap();
        let mut v = Vault::new();
        v.create(&db, "master-password-1", 0).unwrap();
        assert!(v.save(&db, None, nc("https://a.com", "u", ""), 1).is_err());
        assert!(v.save(&db, None, nc("", "u", "p"), 1).is_err());
        assert!(v.save(&db, Some(999), nc("https://a.com", "u", "p"), 1).is_err());
        let long = "x".repeat(2000);
        assert!(v.save(&db, None, nc("https://a.com", "u", &long), 1).is_err());
    }
}
