//! Защищённое хранилище секретов (этап 6.A).
//!
//! Использует Windows Credential Manager через `keyring-rs`. Каждое
//! значение хранится как отдельный credential под уникальным именем
//! `nemefisto.<key>`. На macOS — Keychain, на Linux — Secret Service
//! (kwallet/gnome-keyring); кросс-платформенно работает «out of the box».
//!
//! Хранится:
//! - `subscription_url` — URL подписки (содержит токен/HWID часто);
//! - `hwid_override` — кастомный HWID для разработки.
//!
//! НЕ хранится:
//! - Сгенерированный SOCKS5 password — он создаётся при connect и не
//!   переживает перезапуск.
//! - Кеш серверов, настройки UI — это не секреты, лежат в localStorage.

use anyhow::{Context, Result};
use keyring::Entry;

/// Префикс для всех keys в Credential Manager — чтобы наши значения
/// не путались с другими приложениями.
const SERVICE_PREFIX: &str = "nemefisto";
/// Username в credential — в Credential Manager у каждой записи есть
/// service+user пара. Нам user не нужен, ставим единый.
const USERNAME: &str = "default";

/// Создать `Entry` для ключа. На Windows credential будет виден в
/// «Учётные данные Windows» как «Универсальные учётные данные».
fn entry(key: &str) -> Result<Entry> {
    let service = format!("{SERVICE_PREFIX}.{key}");
    Entry::new(&service, USERNAME)
        .with_context(|| format!("не удалось создать keyring entry для {key}"))
}

/// Прочитать значение по ключу. Возвращает `None` если ключа нет
/// (не считаем за ошибку — это нормальный first-run сценарий).
pub fn get(key: &str) -> Result<Option<String>> {
    let e = entry(key)?;
    match e.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(anyhow::anyhow!("keyring get({key}): {err}")),
    }
}

/// Записать значение. Перезаписывает существующее.
pub fn set(key: &str, value: &str) -> Result<()> {
    let e = entry(key)?;
    e.set_password(value)
        .with_context(|| format!("keyring set({key})"))
}

/// Удалить значение. Если ключа уже нет — не считаем за ошибку.
pub fn delete(key: &str) -> Result<()> {
    let e = entry(key)?;
    match e.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(anyhow::anyhow!("keyring delete({key}): {err}")),
    }
}
