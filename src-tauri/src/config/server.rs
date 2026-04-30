//! Типы для представления VPN-серверов из подписки.

use serde::{Deserialize, Serialize};

/// Запись об одном сервере из подписки.
///
/// Поле `raw` хранит полный набор полей оригинальной Clash-записи — понадобится
/// при генерации Xray-конфига на Этапе 3.
///
/// Поле `engine_compat` помечает совместимость записи с VPN-ядром.
/// `["xray"]` — только Xray, `["mihomo"]` — только Mihomo, оба — общий
/// случай. UI и connect-логика на этапе 8.B используют это для блокировки
/// несовместимых выборов.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyEntry {
    pub name: String,
    /// Протокол в нижнем регистре: "vless", "vmess", "trojan", "ss",
    /// "hysteria2", "tuic", "wireguard", "socks", "xray-json".
    pub protocol: String,
    pub server: String,
    pub port: u16,
    pub raw: serde_json::Value,
    /// Список движков, которые могут поднять этот сервер. Если поле
    /// отсутствует в кеше старой версии — десериализатор подставляет
    /// fallback `["xray", "mihomo"]` (общий случай).
    #[serde(default = "default_engine_compat")]
    pub engine_compat: Vec<String>,
}

fn default_engine_compat() -> Vec<String> {
    vec!["xray".to_string(), "mihomo".to_string()]
}
