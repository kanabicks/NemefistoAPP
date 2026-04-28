//! HWID — уникальный идентификатор устройства, передаётся в заголовке x-hwid.
//!
//! Приоритеты:
//! 1. Windows MachineGuid из `HKLM\SOFTWARE\Microsoft\Cryptography` —
//!    детерминированный для конкретной установки Windows, переустановка
//!    приложения его не меняет.
//! 2. Кешированный UUID v4 из %LOCALAPPDATA%\NemefistoVPN\hwid.txt.
//! 3. Свежесгенерированный UUID v4 (последний шанс).

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use uuid::Uuid;

/// Неизменяемый HWID, доступный через AppState.
pub struct HwidState(pub String);

fn hwid_path() -> Result<PathBuf> {
    let base = std::env::var("LOCALAPPDATA")
        .context("переменная LOCALAPPDATA не установлена")?;
    Ok(PathBuf::from(base).join("NemefistoVPN").join("hwid.txt"))
}

/// Читает Windows MachineGuid из реестра. Для одной и той же установки
/// Windows значение постоянно — это хорошая привязка устройства.
#[cfg(windows)]
fn read_machine_guid() -> Option<String> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hklm
        .open_subkey("SOFTWARE\\Microsoft\\Cryptography")
        .ok()?;
    let guid: String = key.get_value("MachineGuid").ok()?;
    let trimmed = guid.trim().to_lowercase();
    if trimmed.is_empty() { None } else { Some(trimmed) }
}

#[cfg(not(windows))]
fn read_machine_guid() -> Option<String> {
    None
}

/// Получить HWID устройства. Не модифицирует системный реестр.
pub fn load_or_create() -> Result<String> {
    if let Some(guid) = read_machine_guid() {
        return Ok(guid);
    }

    // Fallback: кешированный или свежий UUID v4
    let path = hwid_path()?;

    if let Ok(existing) = fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).context("не удалось создать директорию для HWID")?;
    }
    let hwid = Uuid::new_v4().to_string();
    fs::write(&path, &hwid).context("не удалось сохранить HWID")?;
    Ok(hwid)
}
