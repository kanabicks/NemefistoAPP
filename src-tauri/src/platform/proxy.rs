//! Управление системным прокси Windows через реестр.
//!
//! Устанавливает SOCKS5 + HTTP proxy в Internet Settings текущего пользователя.
//! Bypass включает localhost, 127.*, LAN-диапазоны и <local> (имена без точки).

use anyhow::{Context, Result};

#[cfg(windows)]
use winreg::{enums::*, RegKey};

#[cfg(windows)]
const INET_SETTINGS: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

/// Включить системный прокси: SOCKS5 на socks_port, HTTP/HTTPS на http_port.
pub fn set_system_proxy(socks_port: u16, http_port: u16) -> Result<()> {
    #[cfg(windows)]
    {
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

/// Выключить системный прокси (восстановить до «без прокси»).
pub fn clear_system_proxy() -> Result<()> {
    #[cfg(windows)]
    {
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
