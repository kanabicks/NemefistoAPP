//! Загрузка и парсинг подписки.
//!
//! User-Agent: ClashforWindows/0.20.39 — remnawave panel отдаёт Clash YAML.
//! Заголовок x-hwid несёт уникальный идентификатор устройства.

use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::server::ProxyEntry;

/// Кешированный список серверов из последней успешной загрузки подписки.
pub struct SubscriptionState {
    pub servers: Mutex<Vec<ProxyEntry>>,
}

impl SubscriptionState {
    pub fn new() -> Self {
        Self {
            servers: Mutex::new(Vec::new()),
        }
    }
}

/// Скачать подписку по URL и вернуть список серверов.
pub async fn fetch_and_parse(url: &str, hwid: &str) -> Result<Vec<ProxyEntry>> {
    let client = reqwest::Client::builder()
        .user_agent("ClashforWindows/0.20.39")
        .build()
        .context("не удалось создать HTTP-клиент")?;

    let body = client
        .get(url)
        .header("x-hwid", hwid)
        .send()
        .await
        .context("ошибка HTTP-запроса")?
        .error_for_status()
        .context("сервер вернул ошибку")?
        .text()
        .await
        .context("не удалось прочитать тело ответа")?;

    parse_clash_yaml(&body)
}

// ─── внутренние хелперы ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ClashConfig {
    #[serde(default)]
    proxies: Vec<serde_yaml::Value>,
}

fn parse_clash_yaml(text: &str) -> Result<Vec<ProxyEntry>> {
    let config: ClashConfig =
        serde_yaml::from_str(text).context("не удалось распарсить Clash YAML")?;

    let entries = config
        .proxies
        .into_iter()
        .filter_map(|v| yaml_proxy_to_entry(v).ok())
        .collect();

    Ok(entries)
}

fn yaml_proxy_to_entry(v: serde_yaml::Value) -> Result<ProxyEntry> {
    let map = v.as_mapping().context("proxy-запись — не mapping")?;

    let name = map
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();

    let protocol = map
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let server = map
        .get("server")
        .and_then(|v| v.as_str())
        .context("поле server обязательно")?
        .to_string();

    let port = map
        .get("port")
        .and_then(|v| v.as_u64())
        .context("поле port обязательно")? as u16;

    // Конвертируем всю YAML-запись в JSON для хранения в ProxyEntry.raw.
    // Clash YAML использует только примитивные типы и строковые ключи,
    // поэтому serde_json::to_value корректно обрабатывает весь набор полей.
    let raw = serde_json::to_value(&v)
        .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()));

    Ok(ProxyEntry {
        name,
        protocol,
        server,
        port,
        raw,
    })
}
