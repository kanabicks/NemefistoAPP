//! Типы для представления VPN-серверов из подписки.

use serde::{Deserialize, Serialize};

/// Запись об одном сервере из подписки.
///
/// Поле `raw` хранит полный набор полей оригинальной Clash-записи — понадобится
/// при генерации Xray-конфига на Этапе 3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyEntry {
    pub name: String,
    /// Протокол в нижнем регистре: "vless", "vmess", "trojan", "ss", …
    pub protocol: String,
    pub server: String,
    pub port: u16,
    pub raw: serde_json::Value,
}
