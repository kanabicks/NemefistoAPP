//! 8.F: HTTP-клиент для Mihomo external-controller API.
//!
//! Mihomo при запуске поднимает HTTP API на адресе `external-controller`
//! (мы задаём `127.0.0.1:<random-port>` в `patch_full_yaml`) с авторизацией
//! по `secret` (наш UUID). Через этот API можно:
//!
//!  - получать список всех прокси и групп с current-selection и
//!    последним измеренным latency (`GET /proxies`);
//!  - выбирать конкретную ноду внутри select-группы без рестарта
//!    mihomo (`PUT /proxies/:group` body `{"name": "<node>"}`);
//!  - запускать тест задержки для одной ноды (`GET /proxies/:name/delay`)
//!    или для всей группы.
//!
//! Это даёт UI возможность показывать live-данные о профиле и
//! переключать ноды моментально (как FlClashX).
//!
//! ВАЖНО: API доступно только когда mihomo запущен. До connect — нет
//! adress'а, нет данных. Frontend должен вызывать команды только при
//! `vpnStatus === "running"` И `engine === "mihomo"`.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::Duration;

const HTTP_TIMEOUT: Duration = Duration::from_secs(8);

/// Параметры подключения к mihomo controller'у — заполняются при
/// connect и сохраняются в `MihomoApiState`. `secret` — наш UUID,
/// генерится в `commands::connect()` и одновременно идёт в YAML
/// (через `patch_full_yaml`) и сюда.
#[derive(Debug, Clone)]
pub struct ControllerEndpoint {
    pub host: String,
    pub port: u16,
    pub secret: String,
}

/// Глобальный state controller-endpoint'а. Tauri `manage`s его как
/// state, команды читают для каждого вызова.
pub struct MihomoApiState {
    pub endpoint: Mutex<Option<ControllerEndpoint>>,
}

impl MihomoApiState {
    pub fn new() -> Self {
        Self {
            endpoint: Mutex::new(None),
        }
    }

    pub fn set(&self, ep: ControllerEndpoint) {
        if let Ok(mut g) = self.endpoint.lock() {
            *g = Some(ep);
        }
    }

    pub fn clear(&self) {
        if let Ok(mut g) = self.endpoint.lock() {
            *g = None;
        }
    }

    pub fn get(&self) -> Option<ControllerEndpoint> {
        self.endpoint.lock().ok().and_then(|g| g.clone())
    }
}

/// Информация об одной прокси/группе из ответа `/proxies`.
///
/// Mihomo возвращает все прокси и группы под одной ручкой — type
/// различает (`Selector`, `URLTest`, `Fallback`, `LoadBalance`,
/// `Relay` для групп; конкретные `Vless`/`Vmess`/`Trojan`/... для нод).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyInfo {
    pub name: String,
    /// `Selector` / `URLTest` / `Fallback` / `LoadBalance` / `Relay` для
    /// групп; конкретный proto для нод (`Vless`, `Trojan`, `Direct`, ...).
    #[serde(rename = "type")]
    pub proxy_type: String,
    /// Только для групп — текущая выбранная нода (для select) или
    /// активная (для url-test/fallback).
    #[serde(default)]
    pub now: Option<String>,
    /// Только для групп — список членов (имена прокси/подгрупп).
    #[serde(default)]
    pub all: Vec<String>,
    /// История измерений latency. Последний элемент — самое свежее
    /// значение (мс). Пусто пока ни разу не измерялось.
    #[serde(default)]
    pub history: Vec<DelayHistory>,
    /// UDP-поддержка (для info, не используется в UI пока).
    #[serde(default)]
    pub udp: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelayHistory {
    /// ISO-8601 время измерения.
    pub time: String,
    /// Latency в миллисекундах. 0 если timeout/fail.
    pub delay: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProxiesSnapshot {
    /// Все прокси и группы под одной map'ой (имя → info).
    pub proxies: std::collections::HashMap<String, ProxyInfo>,
}

#[derive(Deserialize)]
struct ProxiesResponse {
    proxies: std::collections::HashMap<String, ProxyInfo>,
}

#[derive(Deserialize)]
struct DelayResponse {
    /// Mihomo при успехе возвращает `{"delay": N}`, при timeout —
    /// `{"message": "..."}` со статусом 408. Поэтому поле
    /// optional — десериализуем только при 2xx.
    delay: Option<u32>,
}

fn build_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        // controller на loopback — не используем системный proxy
        // (иначе зациклимся через сам mihomo's inbound).
        .no_proxy()
        .timeout(HTTP_TIMEOUT)
        .build()
        .context("reqwest::Client build")
}

fn url(ep: &ControllerEndpoint, path: &str) -> String {
    format!("http://{}:{}{}", ep.host, ep.port, path)
}

/// `GET /proxies` — снапшот всех нод и групп.
pub async fn fetch_proxies(ep: &ControllerEndpoint) -> Result<ProxiesSnapshot> {
    let client = build_client()?;
    let resp = client
        .get(url(ep, "/proxies"))
        .bearer_auth(&ep.secret)
        .send()
        .await
        .context("GET /proxies")?;
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {}", resp.status()));
    }
    let body: ProxiesResponse = resp.json().await.context("decode /proxies")?;
    Ok(ProxiesSnapshot {
        proxies: body.proxies,
    })
}

/// `PUT /proxies/:group` — переключить активную ноду в select-группе.
/// Для url-test/fallback групп возвращает 400 — клиент должен скрыть
/// «выбор вручную» для таких типов.
pub async fn select_proxy(
    ep: &ControllerEndpoint,
    group: &str,
    node_name: &str,
) -> Result<()> {
    let client = build_client()?;
    let resp = client
        .put(url(ep, &format!("/proxies/{}", urlencoding::encode(group))))
        .bearer_auth(&ep.secret)
        .json(&serde_json::json!({ "name": node_name }))
        .send()
        .await
        .context("PUT /proxies/:group")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("HTTP {status}: {body}"));
    }
    Ok(())
}

/// `DELETE /connections` — закрыть все активные TCP-соединения mihomo.
///
/// Зачем: `select_proxy` переключает только **новые** соединения. Браузер
/// держит keep-alive с прежней нодой через тот же сокет — пока он не
/// закрылся, трафик продолжает идти через старый outbound. Чтобы смена
/// прокси применилась сразу, после `select_proxy` зовём это API —
/// mihomo закрывает все TCP-сессии, браузер при следующем запросе
/// переподключается через свежий outbound.
pub async fn close_all_connections(ep: &ControllerEndpoint) -> Result<()> {
    let client = build_client()?;
    let resp = client
        .delete(url(ep, "/connections"))
        .bearer_auth(&ep.secret)
        .send()
        .await
        .context("DELETE /connections")?;
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {}", resp.status()));
    }
    Ok(())
}

/// `GET /proxies/:name/delay` — измерить latency для одной ноды через
/// заданный url. Возвращает мс или `None` при timeout/fail.
///
/// Для группы (url-test/fallback) — mihomo сам тестит все ноды
/// группы и возвращает значение для лучшей.
pub async fn delay_test(
    ep: &ControllerEndpoint,
    name: &str,
    test_url: &str,
    timeout_ms: u32,
) -> Result<Option<u32>> {
    let client = build_client()?;
    // Собираем query string руками — reqwest без feature `query` метод
    // не предоставляет, а тащить ради двух параметров его не хочется.
    let path = format!(
        "/proxies/{}/delay?url={}&timeout={}",
        urlencoding::encode(name),
        urlencoding::encode(test_url),
        timeout_ms,
    );
    let resp = client
        .get(url(ep, &path))
        .bearer_auth(&ep.secret)
        .send()
        .await
        .context("GET /proxies/:name/delay")?;
    if resp.status().as_u16() == 408 {
        // timeout — нормальный исход
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {}", resp.status()));
    }
    let body: DelayResponse = resp.json().await.context("decode delay response")?;
    Ok(body.delay)
}
