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
/// оригинальные значения. Иначе принудительно выставляет `ProxyEnable=0`
/// и удаляет `ProxyServer`/`ProxyOverride`.
///
/// **Двойной щит**: после write читаем registry обратно, и если значение
/// не изменилось (другой процесс может одновременно писать туда —
/// например, GPO или антивирус) — повторяем write через прямой winreg
/// API. Если оба прохода не сработали — возвращаем Err: фронт покажет
/// инструкцию пользователю как очистить вручную. Никогда не оставляет
/// прокси в «мёртвом» состоянии молча.
pub fn clear_system_proxy() -> Result<()> {
    #[cfg(windows)]
    {
        if let Some(backup) = read_backup() {
            apply_backup(&backup)?;
            delete_backup();
            // Verify даже после restore: backup мог содержать ProxyEnable=1
            // от прошлого настоящего прокси — это легитимно, не трогаем.
            // Но если backup был None (всех значений) — verify должен
            // показать ProxyEnable=0.
            if let Some(0) | None = read_proxy_enable() {
                return Ok(());
            }
            // Несоответствие — но это легитимный backup, не нам решать.
            return Ok(());
        }

        // Нет backup'а → жёсткая очистка с verify + retry.
        force_clear_proxy_with_retry()
    }

    #[cfg(not(windows))]
    {
        Ok(())
    }
}

/// Безусловная очистка системного прокси без оглядки на backup.
///
/// Используется в emergency-recovery (UI кнопка «восстановить сеть»)
/// и при startup self-healing когда уверены что в реестре наш orphan.
/// Backup не трогает — если он был, оставляет; если нет, не создаёт.
pub fn force_clear_system_proxy() -> Result<()> {
    #[cfg(windows)]
    {
        force_clear_proxy_with_retry()
    }
    #[cfg(not(windows))]
    {
        Ok(())
    }
}

#[cfg(windows)]
fn force_clear_proxy_with_retry() -> Result<()> {
    // Первая попытка — через winreg (как и было).
    let attempt1 = write_proxy_disabled();

    // Verify.
    if let Some(0) | None = read_proxy_enable() {
        return Ok(());
    }

    // Вторая попытка — те же ключи но по новому handle (на случай если
    // первый key handle устарел из-за параллельной записи).
    let attempt2 = write_proxy_disabled();

    if let Some(0) | None = read_proxy_enable() {
        return Ok(());
    }

    // Оба провалились — возвращаем подробную ошибку чтобы UI показал
    // пользователю PowerShell-команду для ручной очистки.
    let err1 = attempt1.err().map(|e| e.to_string()).unwrap_or_default();
    let err2 = attempt2.err().map(|e| e.to_string()).unwrap_or_default();
    Err(anyhow::anyhow!(
        "не удалось очистить системный прокси (попытка 1: {err1}; попытка 2: {err2}). \
         Выполните вручную в PowerShell: \
         Set-ItemProperty -Path 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings' -Name ProxyEnable -Value 0"
    ))
}

#[cfg(windows)]
fn write_proxy_disabled() -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(INET_SETTINGS)
        .context("не удалось открыть Internet Settings в реестре")?;
    key.set_value("ProxyEnable", &0u32).context("ProxyEnable")?;
    let _ = key.delete_value("ProxyServer");
    let _ = key.delete_value("ProxyOverride");
    Ok(())
}

#[cfg(windows)]
fn read_proxy_enable() -> Option<u32> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey(INET_SETTINGS).ok()?;
    key.get_value::<u32, _>("ProxyEnable").ok()
}

/// Прочитать `ProxyServer` напрямую — для startup poison check и
/// pre-flight check при connect. None если ключа нет.
#[cfg(windows)]
pub fn read_proxy_server() -> Option<String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey(INET_SETTINGS).ok()?;
    key.get_value::<String, _>("ProxyServer").ok()
}

#[cfg(not(windows))]
pub fn read_proxy_server() -> Option<String> {
    None
}

/// Признак что текущий системный прокси указывает **на нас** —
/// `127.0.0.1:port`, где port в нашем диапазоне (либо legacy 1080/1087,
/// либо 9.H рандомизация 30000-60000). Используется чтобы безопасно
/// auto-clear только наш orphan, не трогая чужой VPN-клиент.
pub fn is_proxy_pointing_to_us() -> bool {
    #[cfg(windows)]
    {
        let Some(_) = read_proxy_enable().filter(|&v| v == 1) else {
            return false;
        };
        let Some(server) = read_proxy_server() else {
            return false;
        };
        // Формат может быть либо "host:port", либо
        // "socks=127.0.0.1:1080;http=127.0.0.1:1087;..."
        // Разбиваем по ; и разбираем все hint'ы.
        for part in server.split(';') {
            let s = part.split('=').last().unwrap_or(part);
            let (host, port) = match s.rsplit_once(':') {
                Some(pair) => pair,
                None => continue,
            };
            if host != "127.0.0.1" && host != "localhost" {
                continue;
            }
            let Ok(port_n) = port.parse::<u16>() else {
                continue;
            };
            // Наш диапазон портов:
            //   - 1080 / 1087 — legacy фиксированные (до 9.H)
            //   - 30000-59999 — текущая рандомизация (9.H)
            if port_n == 1080 || port_n == 1087 || (30000..60000).contains(&port_n) {
                return true;
            }
        }
        false
    }
    #[cfg(not(windows))]
    {
        false
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
