//! Управление системным прокси Windows через реестр.
//!
//! Устанавливает SOCKS5 + HTTP proxy в Internet Settings текущего пользователя.
//! Bypass включает localhost, 127.*, LAN-диапазоны и <local> (имена без точки).
//!
//! Backup/restore (9.D): перед перезаписью значений мы сохраняем оригиналы
//! `ProxyEnable` / `ProxyServer` / `ProxyOverride` в JSON-файл
//! `%LOCALAPPDATA%\NemefistoVPN\proxy_backup.json`. При `clear_system_proxy`
//! восстанавливаем их обратно. Если приложение крашнется в режиме proxy и
//! не успеет очистить — на старте next-run-а мы детектим backup-файл и
//! предлагаем пользователю восстановить.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[cfg(windows)]
use winreg::{enums::*, RegKey};

#[cfg(windows)]
const INET_SETTINGS: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

const BACKUP_DIR: &str = "NemefistoVPN";
const BACKUP_FILE: &str = "proxy_backup.json";

/// Снимок настроек системного прокси для backup/restore.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyBackup {
    /// Значение `ProxyEnable` (0/1). None — ключ отсутствовал.
    pub proxy_enable: Option<u32>,
    /// Значение `ProxyServer`. None — ключ отсутствовал.
    pub proxy_server: Option<String>,
    /// Значение `ProxyOverride`. None — ключ отсутствовал.
    pub proxy_override: Option<String>,
}

/// Путь к файлу backup'а в %LOCALAPPDATA%\NemefistoVPN\proxy_backup.json.
fn backup_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let local = std::env::var_os("LOCALAPPDATA")?;
        Some(PathBuf::from(local).join(BACKUP_DIR).join(BACKUP_FILE))
    }
    #[cfg(not(windows))]
    {
        // На *nix используем XDG_DATA_HOME или ~/.local/share
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
        Some(base.join(BACKUP_DIR).join(BACKUP_FILE))
    }
}

/// Прочитать текущие значения registry-ключей в ProxyBackup.
#[cfg(windows)]
fn read_current_proxy_state() -> Result<ProxyBackup> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey(INET_SETTINGS)
        .context("не удалось открыть Internet Settings в реестре")?;

    Ok(ProxyBackup {
        proxy_enable: key.get_value("ProxyEnable").ok(),
        proxy_server: key.get_value("ProxyServer").ok(),
        proxy_override: key.get_value("ProxyOverride").ok(),
    })
}

/// Сохранить backup в файл (создаёт папку если нужно). Тихо игнорирует
/// ошибки IO — backup это страховка, не критичный путь.
fn save_backup(backup: &ProxyBackup) {
    let Some(path) = backup_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(backup) {
        let _ = std::fs::write(&path, json);
    }
}

/// Удалить backup-файл (best-effort). Вызывается после успешного restore.
fn delete_backup() {
    if let Some(path) = backup_path() {
        let _ = std::fs::remove_file(path);
    }
}

/// Проверить, существует ли файл backup'а. Используется на старте app
/// для детекции прерванной прошлой сессии (краш или kill).
pub fn has_pending_backup() -> bool {
    backup_path().map(|p| p.is_file()).unwrap_or(false)
}

/// Прочитать backup из файла. Вернёт None если файла нет / битый JSON.
pub fn read_backup() -> Option<ProxyBackup> {
    let path = backup_path()?;
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

/// Включить системный прокси: SOCKS5 на socks_port, HTTP/HTTPS на http_port.
///
/// Перед перезаписью значений сохраняет оригиналы в backup-файл. Если в
/// момент `clear_system_proxy` (или `restore_from_backup` после краша)
/// мы находим backup — восстанавливаем точно эти значения.
pub fn set_system_proxy(socks_port: u16, http_port: u16) -> Result<()> {
    #[cfg(windows)]
    {
        // 9.D: сохраняем текущее состояние ДО изменений, чтобы пережить
        // краш приложения. Если backup уже есть (например, мы apply-нули
        // прокси, а пользователь вызвал set ещё раз) — не перезаписываем,
        // чтобы не потерять оригинал.
        if !has_pending_backup() {
            if let Ok(snapshot) = read_current_proxy_state() {
                save_backup(&snapshot);
            }
        }

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu
            .create_subkey(INET_SETTINGS)
            .context("не удалось открыть Internet Settings в реестре")?;

        // Формат строки: "protocol=host:port;..."
        let proxy_server = format!(
            "socks=127.0.0.1:{socks_port};http=127.0.0.1:{http_port};https=127.0.0.1:{http_port}"
        );
        key.set_value("ProxyServer", &proxy_server)
            .context("ProxyServer")?;
        key.set_value("ProxyEnable", &1u32)
            .context("ProxyEnable")?;
        // Bypass: локальные адреса и LAN не идут через прокси
        key.set_value(
            "ProxyOverride",
            &"localhost;127.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;\
              172.21.*;172.22.*;172.23.*;172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;\
              172.29.*;172.30.*;172.31.*;192.168.*;<local>",
        )
        .context("ProxyOverride")?;

        Ok(())
    }

    #[cfg(not(windows))]
    {
        let _ = (socks_port, http_port);
        Ok(()) // на macOS/Linux — заглушка, реализуем при портировании
    }
}

/// Выключить системный прокси.
///
/// Если есть backup от прошлого `set_system_proxy` — восстанавливает
/// оригинальные значения. Иначе просто выключает (`ProxyEnable=0`).
pub fn clear_system_proxy() -> Result<()> {
    #[cfg(windows)]
    {
        if let Some(backup) = read_backup() {
            apply_backup(&backup)?;
            delete_backup();
            return Ok(());
        }

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu
            .create_subkey(INET_SETTINGS)
            .context("не удалось открыть Internet Settings в реестре")?;
        key.set_value("ProxyEnable", &0u32)
            .context("ProxyEnable")?;
        Ok(())
    }

    #[cfg(not(windows))]
    {
        Ok(())
    }
}

/// Восстановить прокси-настройки из backup-файла (вызывается на старте
/// приложения если has_pending_backup() == true и пользователь подтвердил
/// recovery в диалоге). Удаляет backup-файл после успешного применения.
pub fn restore_from_backup() -> Result<()> {
    #[cfg(windows)]
    {
        let backup = read_backup().context("backup-файла нет или он битый")?;
        apply_backup(&backup)?;
        delete_backup();
        Ok(())
    }

    #[cfg(not(windows))]
    {
        Ok(())
    }
}

/// Удалить backup-файл без применения значений. Используется когда
/// пользователь в диалоге crash-recovery нажимает «не восстанавливать»
/// (значит наши значения он уже не считает актуальными — продолжаем
/// с текущим состоянием реестра).
pub fn discard_backup() {
    delete_backup();
}

/// Применить значения из backup'а к реестру. Если оригинальное значение
/// было None (ключа не существовало) — удаляем ключ из реестра.
#[cfg(windows)]
fn apply_backup(backup: &ProxyBackup) -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(INET_SETTINGS)
        .context("не удалось открыть Internet Settings в реестре")?;

    match backup.proxy_enable {
        Some(v) => {
            key.set_value("ProxyEnable", &v).context("ProxyEnable")?;
        }
        None => {
            let _ = key.delete_value("ProxyEnable");
        }
    }
    match &backup.proxy_server {
        Some(s) => {
            key.set_value("ProxyServer", s).context("ProxyServer")?;
        }
        None => {
            let _ = key.delete_value("ProxyServer");
        }
    }
    match &backup.proxy_override {
        Some(s) => {
            key.set_value("ProxyOverride", s).context("ProxyOverride")?;
        }
        None => {
            let _ = key.delete_value("ProxyOverride");
        }
    }
    Ok(())
}
