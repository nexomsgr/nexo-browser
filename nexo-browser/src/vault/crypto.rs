//! Криптография хранилища паролей.
//!
//! * Ключ AES-256 получается из мастер-пароля через Argon2id (параметры хранятся рядом с данными,
//!   поэтому их можно усилить в будущем без потери старых хранилищ).
//! * Каждая запись шифруется AES-256-GCM со своим случайным nonce; формат блока: `nonce(12) || шифртекст+тег`.
//! * Ключ и расшифрованные байты стираются из памяти при освобождении (`Zeroizing`).

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, bail, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use zeroize::Zeroizing;

pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 12;
pub const KEY_LEN: usize = 32;
const AAD: &[u8] = b"nexo-vault-v1";

pub type Key = Zeroizing<[u8; KEY_LEN]>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

impl KdfParams {
    /// Рекомендация OWASP для Argon2id: 19 МиБ, 2 прохода, 1 поток.
    pub const DEFAULT: KdfParams = KdfParams {
        m_kib: 19 * 1024,
        t: 2,
        p: 1,
    };

    pub fn to_bytes(self) -> [u8; 12] {
        let mut b = [0u8; 12];
        b[..4].copy_from_slice(&self.m_kib.to_le_bytes());
        b[4..8].copy_from_slice(&self.t.to_le_bytes());
        b[8..].copy_from_slice(&self.p.to_le_bytes());
        b
    }

    /// Разбирает сохранённые параметры. Значения проверяются: испорченная или подменённая база не должна
    /// заставить браузер выделить гигабайты памяти.
    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        if b.len() != 12 {
            bail!("повреждены параметры ключа");
        }
        let rd = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let p = KdfParams {
            m_kib: rd(0),
            t: rd(4),
            p: rd(8),
        };
        if !(8..=256 * 1024).contains(&p.m_kib) || !(1..=10).contains(&p.t) || !(1..=4).contains(&p.p) {
            bail!("недопустимые параметры ключа");
        }
        Ok(p)
    }
}

pub fn random<const N: usize>() -> Result<[u8; N]> {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).map_err(|e| anyhow!("нет источника случайных чисел: {e}"))?;
    Ok(buf)
}

pub fn derive_key(master: &str, salt: &[u8], params: KdfParams) -> Result<Key> {
    let p = Params::new(params.m_kib, params.t, params.p, Some(KEY_LEN))
        .map_err(|e| anyhow!("параметры Argon2: {e}"))?;
    let mut key: Key = Zeroizing::new([0u8; KEY_LEN]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, p)
        .hash_password_into(master.as_bytes(), salt, &mut *key)
        .map_err(|e| anyhow!("Argon2: {e}"))?;
    Ok(key)
}

pub fn seal(key: &Key, plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(&key[..]).map_err(|e| anyhow!("{e}"))?;
    let nonce_bytes: [u8; NONCE_LEN] = random()?;
    let ct = cipher
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: plaintext,
                aad: AAD,
            },
        )
        .map_err(|_| anyhow!("ошибка шифрования"))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// `Err` — неверный ключ или повреждённые данные (AES-GCM не различает эти случаи, и это правильно).
pub fn open(key: &Key, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    if blob.len() < NONCE_LEN + 16 {
        bail!("запись слишком короткая");
    }
    let cipher = Aes256Gcm::new_from_slice(&key[..]).map_err(|e| anyhow!("{e}"))?;
    let (nonce, ct) = blob.split_at(NONCE_LEN);
    let pt = cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload { msg: ct, aad: AAD },
        )
        .map_err(|_| anyhow!("не удалось расшифровать"))?;
    Ok(Zeroizing::new(pt))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Быстрые параметры только для тестов.
    const FAST: KdfParams = KdfParams { m_kib: 8, t: 1, p: 1 };

    #[test]
    fn roundtrip_and_random_nonce() {
        let k = derive_key("пароль-123", &[7u8; SALT_LEN], FAST).unwrap();
        let a = seal(&k, b"secret").unwrap();
        let b = seal(&k, b"secret").unwrap();
        assert_ne!(a, b, "nonce должен быть разным");
        assert_eq!(&open(&k, &a).unwrap()[..], b"secret");
    }

    #[test]
    fn wrong_key_or_tampering_fails() {
        let k1 = derive_key("one-password", &[1u8; SALT_LEN], FAST).unwrap();
        let k2 = derive_key("two-password", &[1u8; SALT_LEN], FAST).unwrap();
        let mut blob = seal(&k1, b"data").unwrap();
        assert!(open(&k2, &blob).is_err());
        let last = blob.len() - 1;
        blob[last] ^= 1;
        assert!(open(&k1, &blob).is_err());
        assert!(open(&k1, &[0u8; 5]).is_err());
    }

    #[test]
    fn same_password_different_salt_gives_different_key() {
        let a = derive_key("same-password", &[1u8; SALT_LEN], FAST).unwrap();
        let b = derive_key("same-password", &[2u8; SALT_LEN], FAST).unwrap();
        assert_ne!(&a[..], &b[..]);
    }

    #[test]
    fn kdf_params_roundtrip_and_limits() {
        let p = KdfParams::DEFAULT;
        assert_eq!(KdfParams::from_bytes(&p.to_bytes()).unwrap(), p);
        let huge = KdfParams { m_kib: u32::MAX, t: 1, p: 1 };
        assert!(KdfParams::from_bytes(&huge.to_bytes()).is_err());
        assert!(KdfParams::from_bytes(&[0u8; 3]).is_err());
    }
}
