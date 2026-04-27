//! HWID — уникальный идентификатор установки, передаётся в заголовке x-hwid.
//!
//! UUID v4 генерируется при первом запуске и сохраняется в
//! %LOCALAPPDATA%\NemefistoVPN\hwid.txt. При последующих запусках читается оттуда.

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

/// Прочитать HWID из файла или сгенерировать новый и сохранить.
pub fn load_or_create() -> Result<String> {
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
