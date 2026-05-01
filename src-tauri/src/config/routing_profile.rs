//! 11.A — Модель routing-профиля для split-routing конфигурации.
//!
//! Совместима с типовыми панелями (Marzban-style): GlobalProxy /
//! DomainStrategy / Direct/Proxy/BlockSites/Ip / Geoipurl/Geositeurl и др.
//! Парсится через `serde` из JSON; PascalCase поля переименовываются в
//! snake_case Rust-стороны через `#[serde(rename = "...")]`.
//!
//! Один профиль либо «статический» (вшит в base64 deep-link / в подписке),
//! либо «autorouting» (URL-источник с авто-обновлением). Различие хранится
//! в `RoutingStore` (см. routing_store.rs), здесь — только сам формат.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Стратегия резолва доменов для матчинга по IP-правилам.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum DomainStrategy {
    /// Не резолвить, матчить только домены.
    AsIs,
    /// Резолвить если домен не сматчился ни одному правилу — потом матчить IP.
    #[serde(rename = "IPIfNonMatch")]
    IpIfNonMatch,
    /// Всегда резолвить домен в IP перед матчингом.
    #[serde(rename = "IPOnDemand")]
    IpOnDemand,
}

impl Default for DomainStrategy {
    fn default() -> Self {
        Self::IpIfNonMatch
    }
}

/// Тип DNS-записи (DoH или обычный UDP).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DnsKind {
    #[default]
    DoH,
    Plain,
}

/// DNS-конфиг для проксированного либо direct трафика.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DnsConfig {
    /// Тип резолвера (`DoH` или `Plain`).
    #[serde(default)]
    pub kind: DnsKind,
    /// Для DoH — URL `https://...`, для Plain — IP-адрес.
    #[serde(default)]
    pub domain: String,
    /// Bootstrap IP для самого DoH-сервера (чтобы не резолвить через себя же).
    #[serde(default)]
    pub ip: String,
}

/// Routing-профиль — единая декларация split-routing правил.
///
/// Поля имеют PascalCase для совместимости с Marzban / 3x-ui / sing-box
/// дампами, которые пользователи обычно копируют как есть.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RoutingProfile {
    /// Человеко-читаемое имя профиля.
    #[serde(rename = "Name")]
    pub name: String,

    /// Если `true` — весь трафик через прокси кроме явных Direct-правил.
    /// Если `false` — только то что в ProxySites/ProxyIp.
    #[serde(rename = "GlobalProxy")]
    pub global_proxy: BoolString,

    /// Unix-timestamp последнего обновления (от автора профиля).
    #[serde(rename = "LastUpdated")]
    pub last_updated: String,

    /// Стратегия резолва доменов (см. enum).
    #[serde(rename = "DomainStrategy")]
    pub domain_strategy: DomainStrategy,

    // ── DNS ────────────────────────────────────────────────────────────────
    #[serde(rename = "RemoteDNSType")]
    pub remote_dns_type: String,
    #[serde(rename = "RemoteDNSDomain")]
    pub remote_dns_domain: String,
    #[serde(rename = "RemoteDNSIP")]
    pub remote_dns_ip: String,
    #[serde(rename = "DomesticDNSType")]
    pub domestic_dns_type: String,
    #[serde(rename = "DomesticDNSDomain")]
    pub domestic_dns_domain: String,
    #[serde(rename = "DomesticDNSIP")]
    pub domestic_dns_ip: String,

    /// Статические DNS-hosts (в обход всех резолверов).
    #[serde(rename = "DnsHosts")]
    pub dns_hosts: std::collections::BTreeMap<String, String>,

    /// FakeDNS — виртуальные IP для доменов (Mihomo only).
    #[serde(rename = "FakeDNS")]
    pub fake_dns: BoolString,

    // ── Правила маршрутизации ──────────────────────────────────────────────
    /// Сайты которые идут direct (например `geosite:ru`).
    #[serde(rename = "DirectSites")]
    pub direct_sites: Vec<String>,
    /// IP/CIDR direct (`geoip:ru`, `10.0.0.0/8`).
    #[serde(rename = "DirectIp")]
    pub direct_ip: Vec<String>,
    /// Сайты только через прокси.
    #[serde(rename = "ProxySites")]
    pub proxy_sites: Vec<String>,
    /// IP/CIDR только через прокси.
    #[serde(rename = "ProxyIp")]
    pub proxy_ip: Vec<String>,
    /// Сайты заблокировать.
    #[serde(rename = "BlockSites")]
    pub block_sites: Vec<String>,
    /// IP/CIDR заблокировать.
    #[serde(rename = "BlockIp")]
    pub block_ip: Vec<String>,

    // ── Geofiles ───────────────────────────────────────────────────────────
    #[serde(rename = "Geoipurl")]
    pub geoip_url: String,
    #[serde(rename = "Geositeurl")]
    pub geosite_url: String,

    /// Использовать chunked-файлы (только мобильные, на десктопе игнорим).
    #[serde(rename = "useChunkFiles")]
    pub use_chunk_files: bool,
}

/// Helper-обёртка над bool для совместимости с JSON где бы значение
/// могло быть строкой (`"true"`/`"false"`) ИЛИ натуральным bool. Marzban
/// и подобные часто пишут строкой.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BoolString(pub bool);

impl Serialize for BoolString {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(if self.0 { "true" } else { "false" })
    }
}

impl<'de> Deserialize<'de> for BoolString {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let v = serde_json::Value::deserialize(d)?;
        match v {
            serde_json::Value::Bool(b) => Ok(BoolString(b)),
            serde_json::Value::String(s) => match s.to_lowercase().as_str() {
                "true" | "1" | "yes" => Ok(BoolString(true)),
                "false" | "0" | "no" | "" => Ok(BoolString(false)),
                _ => Err(D::Error::custom(format!("ожидался bool, получено: {s:?}"))),
            },
            serde_json::Value::Number(n) => Ok(BoolString(n.as_i64().unwrap_or(0) != 0)),
            other => Err(D::Error::custom(format!("ожидался bool, получено: {other:?}"))),
        }
    }
}

impl RoutingProfile {
    /// Распарсить JSON-строку в RoutingProfile с базовой валидацией.
    pub fn parse_json(s: &str) -> Result<Self> {
        let p: Self = serde_json::from_str(s).context("невалидный routing JSON")?;
        p.validate()?;
        Ok(p)
    }

    /// Базовая валидация формата правил. Проверяем что URL'ы — это URL,
    /// IP/CIDR — корректные, geosite/geoip — известные префиксы.
    pub fn validate(&self) -> Result<()> {
        if !self.geoip_url.is_empty() && !is_http_url(&self.geoip_url) {
            bail!("Geoipurl не валидный URL: {}", self.geoip_url);
        }
        if !self.geosite_url.is_empty() && !is_http_url(&self.geosite_url) {
            bail!("Geositeurl не валидный URL: {}", self.geosite_url);
        }
        for (label, list) in [
            ("DirectIp", &self.direct_ip),
            ("ProxyIp", &self.proxy_ip),
            ("BlockIp", &self.block_ip),
        ] {
            for entry in list {
                if entry.starts_with("geoip:") {
                    continue;
                }
                if !is_ip_or_cidr(entry) {
                    bail!("{label}: невалидный IP/CIDR `{entry}`");
                }
            }
        }
        Ok(())
    }

    /// Удобный builder для встроенного «минимального RU» шаблона (13.Q).
    pub fn minimal_ru() -> Self {
        Self {
            name: "минимальный RU".to_string(),
            global_proxy: BoolString(true),
            domain_strategy: DomainStrategy::IpIfNonMatch,
            direct_sites: vec!["geosite:ru".to_string()],
            direct_ip: vec![
                "geoip:ru".to_string(),
                "10.0.0.0/8".to_string(),
                "172.16.0.0/12".to_string(),
                "192.168.0.0/16".to_string(),
            ],
            block_sites: vec!["geosite:category-ads-all".to_string()],
            geoip_url:
                "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/geoip.dat"
                    .to_string(),
            geosite_url:
                "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/geosite.dat"
                    .to_string(),
            ..Default::default()
        }
    }
}

fn is_http_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

fn is_ip_or_cidr(s: &str) -> bool {
    let core = s.split('/').next().unwrap_or(s);
    core.parse::<std::net::IpAddr>().is_ok()
        && s.split('/')
            .nth(1)
            .map(|p| p.parse::<u8>().map(|n| n <= 128).unwrap_or(false))
            .unwrap_or(true)
}

/// Источник профиля — критично для UX отображения и автообновления.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProfileSource {
    /// Статический — JSON прислан по deep-link (base64) или вручную.
    Static,
    /// Autorouting — скачан с URL и обновляется по интервалу часов.
    Autorouting { url: String, interval_hours: u32 },
}

/// Запись в `RoutingStore` — профиль + метаданные источника.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutingProfileEntry {
    /// Уникальный id (UUIDv4) — генерируется при добавлении.
    pub id: String,
    /// Сам профиль с правилами.
    pub profile: RoutingProfile,
    /// Откуда он взялся (для UI и scheduler'а).
    pub source: ProfileSource,
    /// Когда последний раз обновили (unix-ts). 0 если ещё ни разу.
    pub last_fetched_at: u64,
}

impl RoutingProfileEntry {
    /// Создать новую запись со свежим UUID.
    pub fn new(profile: RoutingProfile, source: ProfileSource) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            profile,
            source,
            last_fetched_at: 0,
        }
    }
}

/// Helper для парсинга: принимает либо JSON-строку, либо base64-encoded
/// JSON. Удобно для deep-link где JSON всегда base64.
pub fn parse_profile_input(input: &str) -> Result<RoutingProfile> {
    let trimmed = input.trim();
    // Если начинается с `{` — это уже JSON.
    if trimmed.starts_with('{') {
        return RoutingProfile::parse_json(trimmed);
    }
    // Иначе пробуем как base64.
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(trimmed)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(trimmed))
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(trimmed))
        .map_err(|e| anyhow!("не base64 и не JSON: {e}"))?;
    let s = String::from_utf8(decoded).context("base64 содержимое — не UTF-8")?;
    RoutingProfile::parse_json(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_marzban_style_json() {
        let json = r#"{
            "Name": "Test",
            "GlobalProxy": "true",
            "DomainStrategy": "IPIfNonMatch",
            "DirectSites": ["geosite:ru"],
            "DirectIp": ["10.0.0.0/8"],
            "Geoipurl": "https://example.com/geoip.dat",
            "Geositeurl": "https://example.com/geosite.dat"
        }"#;
        let p = RoutingProfile::parse_json(json).unwrap();
        assert_eq!(p.name, "Test");
        assert_eq!(p.global_proxy.0, true);
        assert_eq!(p.direct_sites.len(), 1);
        assert_eq!(p.direct_sites[0], "geosite:ru");
    }

    #[test]
    fn bool_string_accepts_native_bool() {
        let json = r#"{"Name":"X","GlobalProxy":true}"#;
        let p = RoutingProfile::parse_json(json).unwrap();
        assert_eq!(p.global_proxy.0, true);
    }

    #[test]
    fn rejects_invalid_cidr() {
        let json = r#"{
            "Name": "Bad",
            "DirectIp": ["999.0.0.0/8"]
        }"#;
        assert!(RoutingProfile::parse_json(json).is_err());
    }

    #[test]
    fn allows_geoip_prefix_in_ip_list() {
        let json = r#"{"Name":"X","DirectIp":["geoip:ru","10.0.0.0/8"]}"#;
        let p = RoutingProfile::parse_json(json).unwrap();
        assert_eq!(p.direct_ip.len(), 2);
    }

    #[test]
    fn minimal_ru_template_is_valid() {
        RoutingProfile::minimal_ru().validate().unwrap();
    }
}
