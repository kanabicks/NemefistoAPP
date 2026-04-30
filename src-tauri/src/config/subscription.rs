//! Загрузка и парсинг подписки.
//!
//! Основной формат — base64-список URI (vless://, ss://, vmess://, trojan://).
//! Fallback — Clash YAML (если сервер вернул его вместо base64).

use std::sync::Mutex;

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};

use super::server::ProxyEntry;

/// Метаданные подписки из заголовка `subscription-userinfo`
/// (де-факто стандарт у 3x-ui / Marzban / x-ui / sing-box).
///
/// Формат заголовка: `upload=X;download=Y;total=Z;expire=T`,
/// где X/Y/Z — байты (Z=0 → безлимит), T — unix-timestamp срока
/// истечения (T=0 → бессрочно).
#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionMeta {
    /// upload + download в байтах.
    pub used: u64,
    /// Общий лимит в байтах. 0 = безлимит.
    pub total: u64,
    /// Unix-timestamp истечения. None = бессрочно.
    pub expire_at: Option<i64>,
    /// Имя подписки из заголовка `profile-title`. Поддерживает префикс
    /// `base64:...` для не-ASCII значений. ≤25 символов по стандарту.
    pub title: Option<String>,
    /// URL «личного кабинета» из `profile-web-page-url`.
    pub web_page_url: Option<String>,
    /// URL поддержки из `support-url`.
    pub support_url: Option<String>,
    /// Желаемый интервал автообновления подписки в часах из
    /// `profile-update-interval`. Применяется только если пользователь
    /// не менял настройку вручную (override-логика).
    pub update_interval_hours: Option<u32>,
    /// Текстовое объявление от провайдера (`announce`, ≤200 символов).
    /// Поддерживает префикс `base64:...`.
    pub announce: Option<String>,
    /// URL-ссылка для объявления (`announce-url`). Если задана —
    /// объявление становится кликабельным.
    pub announce_url: Option<String>,
    /// URL страницы премиума (`premium-url`). UI показывает кнопку
    /// «премиум» в карточке подписки если задана.
    pub premium_url: Option<String>,
    /// Дефолтная тема UI (`X-Nemefisto-Theme`): dark/light/midnight/
    /// sunset/sand. Применяется если пользователь не менял.
    pub theme: Option<String>,
    /// 3D-фон (`X-Nemefisto-Background`): crystal/tunnel/globe/particles.
    pub background: Option<String>,
    /// Стиль кнопки питания (`X-Nemefisto-Button-Style`):
    /// glass/flat/neon/metallic.
    pub button_style: Option<String>,
    /// Готовая тема-пресет (`X-Nemefisto-Preset`): none/fluent/cupertino/
    /// vice/arcade/glacier.
    pub preset: Option<String>,
    /// Режим VPN по умолчанию (`X-Nemefisto-Mode`): proxy/tun.
    pub mode: Option<String>,
    /// Желаемое VPN-ядро (`X-Nemefisto-Engine`): xray/mihomo. Зарезер-
    /// вировано для этапа 8.B.
    pub engine: Option<String>,

    // ── Anti-DPI (этап 10) ──────────────────────────────────────────
    /// Включена ли TCP-фрагментация (`fragmentation-enable: 0|1`).
    pub fragmentation_enable: Option<bool>,
    /// Какие пакеты фрагментировать (`fragmentation-packets`):
    /// `tlshello` / `1-3` / `all`.
    pub fragmentation_packets: Option<String>,
    /// Длина фрагмента (`fragmentation-length`): `min-max`.
    pub fragmentation_length: Option<String>,
    /// Задержка между фрагментами (`fragmentation-interval`): `min-max` (мс).
    pub fragmentation_interval: Option<String>,
    /// Включены ли шумовые пакеты (`noises-enable: 0|1`).
    pub noises_enable: Option<bool>,
    /// Тип шума (`noises-type`): `rand` / `str` / `hex`.
    pub noises_type: Option<String>,
    /// Содержимое или размер шумового пакета (`noises-packet`).
    pub noises_packet: Option<String>,
    /// Задержка между шумовыми пакетами (`noises-delay`).
    pub noises_delay: Option<String>,
    /// Резолвить адрес сервера через DoH (`server-address-resolve-enable: 0|1`).
    pub server_resolve_enable: Option<bool>,
    /// DoH endpoint для резолва (`server-address-resolve-dns-domain`).
    pub server_resolve_doh: Option<String>,
    /// Bootstrap-IP для DoH (`server-address-resolve-dns-ip`).
    pub server_resolve_bootstrap: Option<String>,
}

/// Кешированный список серверов и метаданных из последней успешной
/// загрузки подписки. Живёт в памяти процесса, не персистится между
/// запусками (для этого есть localStorage на фронте).
pub struct SubscriptionState {
    pub servers: Mutex<Vec<ProxyEntry>>,
    pub meta: Mutex<Option<SubscriptionMeta>>,
}

impl SubscriptionState {
    pub fn new() -> Self {
        Self {
            servers: Mutex::new(Vec::new()),
            meta: Mutex::new(None),
        }
    }
}

/// Парсит заголовок `subscription-userinfo` вида
/// `upload=123;download=456;total=789;expire=1700000000`.
/// Неизвестные ключи игнорируются, отсутствующие → 0.
pub fn parse_subscription_userinfo(raw: &str) -> SubscriptionMeta {
    let mut upload: u64 = 0;
    let mut download: u64 = 0;
    let mut total: u64 = 0;
    let mut expire: i64 = 0;

    for pair in raw.split(';') {
        let pair = pair.trim();
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "upload" => upload = v.parse().unwrap_or(0),
            "download" => download = v.parse().unwrap_or(0),
            "total" => total = v.parse().unwrap_or(0),
            "expire" => expire = v.parse().unwrap_or(0),
            _ => {}
        }
    }

    SubscriptionMeta {
        used: upload.saturating_add(download),
        total,
        expire_at: if expire > 0 { Some(expire) } else { None },
        title: None,
        web_page_url: None,
        support_url: None,
        update_interval_hours: None,
        announce: None,
        announce_url: None,
        premium_url: None,
        theme: None,
        background: None,
        button_style: None,
        preset: None,
        mode: None,
        engine: None,
        fragmentation_enable: None,
        fragmentation_packets: None,
        fragmentation_length: None,
        fragmentation_interval: None,
        noises_enable: None,
        noises_type: None,
        noises_packet: None,
        noises_delay: None,
        server_resolve_enable: None,
        server_resolve_doh: None,
        server_resolve_bootstrap: None,
    }
}

/// Возвращает Some(s) если значение заголовка `s` входит в whitelist
/// `allowed`, иначе None. Регистронезависимое сравнение.
fn validate_enum(value: &str, allowed: &[&str]) -> Option<String> {
    let v = value.trim().to_lowercase();
    if v.is_empty() {
        return None;
    }
    if allowed.iter().any(|a| *a == v) {
        Some(v)
    } else {
        None
    }
}

/// Декодирует значение HTTP-заголовка с поддержкой опционального префикса
/// `base64:...` (стандарт у 3x-ui / Marzban для передачи не-ASCII значений
/// типа кириллических заголовков подписки).
fn decode_header_value(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(b64) = trimmed.strip_prefix("base64:") {
        let bytes = general_purpose::STANDARD
            .decode(b64.trim())
            .or_else(|_| general_purpose::STANDARD_NO_PAD.decode(b64.trim()))
            .or_else(|_| general_purpose::URL_SAFE.decode(b64.trim()))
            .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(b64.trim()))
            .ok()?;
        let s = String::from_utf8(bytes).ok()?;
        let s = s.trim().to_string();
        if s.is_empty() {
            return None;
        }
        return Some(s);
    }
    Some(trimmed.to_string())
}

/// Скачать подписку по URL и вернуть список серверов.
///
/// `user_agent` — UA для запроса. По умолчанию `Happ/2.7.0` (так провайдеры
/// на базе Marzban / RemnaWave отдают массив готовых Xray-конфигов).
/// `hwid` — идентификатор устройства, шлётся в заголовке `x-hwid`. Сервер
/// регистрирует новое устройство автоматически, если в подписке есть
/// свободный HWID-слот. Если `send_hwid=false`, заголовок не шлётся.
pub async fn fetch_and_parse(
    url: &str,
    hwid: &str,
    user_agent: &str,
    send_hwid: bool,
) -> Result<(Vec<ProxyEntry>, Option<SubscriptionMeta>)> {
    let ua = if user_agent.trim().is_empty() {
        "Happ/2.7.0"
    } else {
        user_agent
    };

    let client = reqwest::Client::builder()
        .user_agent(ua)
        .build()
        .context("не удалось создать HTTP-клиент")?;

    let mut req = client.get(url);
    if send_hwid && !hwid.is_empty() {
        req = req.header("x-hwid", hwid);
    }

    let response = req
        .send()
        .await
        .context("ошибка HTTP-запроса")?
        .error_for_status()
        .context("сервер вернул ошибку")?;

    // Извлекаем метаданные из заголовков ДО чтения body (после `text()`
    // response уже потреблён). Базовый заголовок — subscription-userinfo
    // с трафиком и сроком; остальные стандартные заголовки (имя, URL'ы,
    // интервал обновления) накладываются сверху если присутствуют.
    let headers = response.headers().clone();
    let meta = build_subscription_meta(&headers);

    let body = response
        .text()
        .await
        .context("не удалось прочитать тело ответа")?;

    let servers = parse_subscription_body(&body)?;
    Ok((servers, meta))
}

/// Собирает SubscriptionMeta из набора HTTP-заголовков ответа.
/// Возвращает None если ни один из распознаваемых заголовков не задан.
fn build_subscription_meta(headers: &reqwest::header::HeaderMap) -> Option<SubscriptionMeta> {
    let header_str = |name: &str| -> Option<String> {
        headers
            .get(name)
            .and_then(|h| h.to_str().ok())
            .and_then(decode_header_value)
    };

    // Базовая трафик/срок-часть. Если её нет — стартуем с zero-meta,
    // которая подхватит остальные поля.
    let mut meta = headers
        .get("subscription-userinfo")
        .and_then(|h| h.to_str().ok())
        .map(parse_subscription_userinfo)
        .unwrap_or(SubscriptionMeta {
            used: 0,
            total: 0,
            expire_at: None,
            title: None,
            web_page_url: None,
            support_url: None,
            update_interval_hours: None,
            announce: None,
            announce_url: None,
            premium_url: None,
            theme: None,
            background: None,
            button_style: None,
            preset: None,
            mode: None,
            engine: None,
            fragmentation_enable: None,
            fragmentation_packets: None,
            fragmentation_length: None,
            fragmentation_interval: None,
            noises_enable: None,
            noises_type: None,
            noises_packet: None,
            noises_delay: None,
            server_resolve_enable: None,
            server_resolve_doh: None,
            server_resolve_bootstrap: None,
        });

    // Стандартные заголовки (8.C, шаг 2)
    meta.title = header_str("profile-title");
    meta.web_page_url = header_str("profile-web-page-url");
    meta.support_url = header_str("support-url");
    meta.update_interval_hours = headers
        .get("profile-update-interval")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|n| *n > 0);

    // Стандартные заголовки (8.C, шаг 3 — объявления и премиум)
    meta.announce = header_str("announce");
    meta.announce_url = header_str("announce-url");
    meta.premium_url = header_str("premium-url");

    // Заголовки X-Nemefisto-* (наше расширение). Все enum-значения
    // валидируются по whitelist; неизвестные → None.
    let header_enum = |name: &str, allowed: &[&str]| -> Option<String> {
        header_str(name).and_then(|v| validate_enum(&v, allowed))
    };
    meta.theme = header_enum(
        "x-nemefisto-theme",
        &["dark", "light", "midnight", "sunset", "sand"],
    );
    meta.background = header_enum(
        "x-nemefisto-background",
        &["crystal", "tunnel", "globe", "particles"],
    );
    meta.button_style = header_enum(
        "x-nemefisto-button-style",
        &["glass", "flat", "neon", "metallic"],
    );
    meta.preset = header_enum(
        "x-nemefisto-preset",
        &["none", "fluent", "cupertino", "vice", "arcade", "glacier"],
    );
    meta.mode = header_enum("x-nemefisto-mode", &["proxy", "tun"]);
    meta.engine = header_enum("x-nemefisto-engine", &["xray", "mihomo"]);

    // Anti-DPI заголовки (этап 10)
    let header_bool = |name: &str| -> Option<bool> {
        header_str(name).map(|v| {
            let v = v.trim().to_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        })
    };
    meta.fragmentation_enable = header_bool("fragmentation-enable");
    meta.fragmentation_packets =
        header_enum("fragmentation-packets", &["tlshello", "1-3", "all"]);
    meta.fragmentation_length = header_str("fragmentation-length");
    meta.fragmentation_interval = header_str("fragmentation-interval");
    meta.noises_enable = header_bool("noises-enable");
    meta.noises_type = header_enum("noises-type", &["rand", "str", "hex"]);
    meta.noises_packet = header_str("noises-packet");
    meta.noises_delay = header_str("noises-delay");
    meta.server_resolve_enable = header_bool("server-address-resolve-enable");
    meta.server_resolve_doh = header_str("server-address-resolve-dns-domain");
    meta.server_resolve_bootstrap = header_str("server-address-resolve-dns-ip");

    // Если все поля пустые/нулевые — возвращаем None чтобы UI не рендерил
    // пустую плашку.
    let has_any = meta.used > 0
        || meta.total > 0
        || meta.expire_at.is_some()
        || meta.title.is_some()
        || meta.web_page_url.is_some()
        || meta.support_url.is_some()
        || meta.update_interval_hours.is_some()
        || meta.announce.is_some()
        || meta.announce_url.is_some()
        || meta.premium_url.is_some()
        || meta.theme.is_some()
        || meta.background.is_some()
        || meta.button_style.is_some()
        || meta.preset.is_some()
        || meta.mode.is_some()
        || meta.engine.is_some()
        || meta.fragmentation_enable.is_some()
        || meta.noises_enable.is_some()
        || meta.server_resolve_enable.is_some();

    if has_any {
        Some(meta)
    } else {
        None
    }
}

/// Парсит тело подписки, перебирая известные форматы по приоритету.
fn parse_subscription_body(body: &str) -> Result<Vec<ProxyEntry>> {
    // 1. Xray JSON конфиг — приоритетнее всего, чтобы случайно
    //    не распарсить JSON как base64. Может быть как одиночным объектом,
    //    так и массивом (Happ-формат подписки).
    let head = body.trim_start();
    if head.starts_with('{') || head.starts_with('[') {
        if let Ok(entries) = parse_xray_json(body) {
            if !entries.is_empty() {
                return Ok(entries);
            }
        }
    }

    // 2. base64-список URI
    if let Ok(entries) = parse_base64_uri_list(body) {
        if !entries.is_empty() {
            return Ok(entries);
        }
    }

    // 3. Plain text URI list (по одному URI на строку)
    if let Ok(entries) = parse_plain_uri_list(body) {
        if !entries.is_empty() {
            return Ok(entries);
        }
    }

    // 4. Fallback: Clash YAML
    parse_clash_yaml(body)
}

// ─── base64 URI-список ────────────────────────────────────────────────────────

fn parse_base64_uri_list(text: &str) -> Result<Vec<ProxyEntry>> {
    let trimmed = text.trim();
    let decoded = general_purpose::STANDARD
        .decode(trimmed)
        .or_else(|_| general_purpose::STANDARD_NO_PAD.decode(trimmed))
        .or_else(|_| general_purpose::URL_SAFE.decode(trimmed))
        .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(trimmed))
        .context("не base64")?;

    let text = String::from_utf8(decoded).context("декодированный текст — не UTF-8")?;

    let entries: Vec<ProxyEntry> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|l| parse_proxy_uri(l).ok())
        .collect();

    if entries.is_empty() {
        bail!("пустой список URI");
    }
    Ok(entries)
}

fn parse_proxy_uri(uri: &str) -> Result<ProxyEntry> {
    if uri.starts_with("vless://") {
        parse_vless(uri)
    } else if uri.starts_with("vmess://") {
        parse_vmess(uri)
    } else if uri.starts_with("trojan://") {
        parse_trojan(uri)
    } else if uri.starts_with("ss://") {
        parse_ss(uri)
    } else if uri.starts_with("hysteria2://") || uri.starts_with("hy2://") {
        parse_hysteria2(uri)
    } else if uri.starts_with("tuic://") {
        parse_tuic(uri)
    } else if uri.starts_with("wireguard://") || uri.starts_with("wg://") {
        parse_wireguard(uri)
    } else if uri.starts_with("socks5://") || uri.starts_with("socks://") {
        parse_socks(uri)
    } else {
        bail!("неизвестный протокол: {uri}")
    }
}

/// Стандартная пара движков для протоколов, поддерживаемых обоими ядрами.
fn engines_both() -> Vec<String> {
    vec!["xray".to_string(), "mihomo".to_string()]
}

/// Только Mihomo. Используется для протоколов, которые Xray-core нативно
/// НЕ поддерживает — это **TUIC** и **AnyTLS**. Hysteria2 и WireGuard
/// раньше тоже были тут, но Xray их добавил (1.8.16+ и 1.8.6+ соответственно),
/// поэтому теперь они в `engines_both()`.
fn engines_mihomo_only() -> Vec<String> {
    vec!["mihomo".to_string()]
}

// ─── парсеры URI ──────────────────────────────────────────────────────────────

fn parse_vless(uri: &str) -> Result<ProxyEntry> {
    let rest = uri.strip_prefix("vless://").unwrap();

    let (rest, name) = split_fragment(rest);
    let (authority, query) = split_query(rest);
    let (uuid, host, port) = split_userinfo_hostport(authority)
        .context("некорректный authority в VLESS URI")?;

    let mut raw = serde_json::Map::new();
    raw.insert("uuid".into(), uuid.to_string().into());
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        if let Some((k, v)) = pair.split_once('=') {
            raw.insert(k.to_string(), url_decode(v).into());
        }
    }

    Ok(ProxyEntry {
        name,
        protocol: "vless".to_string(),
        server: host.to_string(),
        port,
        raw: serde_json::Value::Object(raw),
        engine_compat: engines_both(),
    })
}

fn parse_vmess(uri: &str) -> Result<ProxyEntry> {
    let b64 = uri.strip_prefix("vmess://").unwrap().trim();
    let decoded = general_purpose::STANDARD
        .decode(b64)
        .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(b64))
        .context("не удалось декодировать VMess base64")?;

    let json: serde_json::Value =
        serde_json::from_slice(&decoded).context("VMess JSON невалиден")?;

    let name = json["ps"].as_str().unwrap_or("VMess").to_string();
    let server = json["add"]
        .as_str()
        .context("поле add обязательно")?
        .to_string();
    let port: u16 = json["port"]
        .as_u64()
        .or_else(|| json["port"].as_str().and_then(|s| s.parse().ok()))
        .context("поле port обязательно")? as u16;

    Ok(ProxyEntry {
        name,
        protocol: "vmess".to_string(),
        server,
        port,
        raw: json,
        engine_compat: engines_both(),
    })
}

fn parse_trojan(uri: &str) -> Result<ProxyEntry> {
    let rest = uri.strip_prefix("trojan://").unwrap();

    let (rest, name) = split_fragment(rest);
    let (authority, query) = split_query(rest);
    let (password, host, port) = split_userinfo_hostport(authority)
        .context("некорректный authority в Trojan URI")?;

    let mut raw = serde_json::Map::new();
    raw.insert("password".into(), password.to_string().into());
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        if let Some((k, v)) = pair.split_once('=') {
            raw.insert(k.to_string(), url_decode(v).into());
        }
    }

    Ok(ProxyEntry {
        name,
        protocol: "trojan".to_string(),
        server: host.to_string(),
        port,
        raw: serde_json::Value::Object(raw),
        engine_compat: engines_both(),
    })
}

fn parse_ss(uri: &str) -> Result<ProxyEntry> {
    let rest = uri.strip_prefix("ss://").unwrap();

    let (rest, name) = split_fragment(rest);
    let (rest, _query) = split_query(rest);

    let (userinfo_b64, host, port) =
        split_userinfo_hostport(rest).context("некорректный SS URI")?;

    let userinfo_bytes = general_purpose::STANDARD
        .decode(userinfo_b64)
        .or_else(|_| general_purpose::STANDARD_NO_PAD.decode(userinfo_b64))
        .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(userinfo_b64))
        .context("не удалось декодировать base64 userinfo в SS URI")?;

    let userinfo = String::from_utf8(userinfo_bytes)?;
    let (cipher, password) = userinfo
        .split_once(':')
        .context("некорректный userinfo в SS URI")?;

    let mut raw = serde_json::Map::new();
    raw.insert("cipher".into(), cipher.to_string().into());
    raw.insert("password".into(), password.to_string().into());

    Ok(ProxyEntry {
        name,
        protocol: "ss".to_string(),
        server: host.to_string(),
        port,
        raw: serde_json::Value::Object(raw),
        engine_compat: engines_both(),
    })
}

// ─── Hysteria2 ───────────────────────────────────────────────────────────────
//
// Формат: `hysteria2://password@server:port?sni=...&insecure=0&obfs=salamander
//          &obfs-password=...#name`
// Также допустимая короткая форма `hy2://...`.
//
// Особенность: пароль (`password`) — единственное «userinfo» в URL, без user.
// Параметры: sni, insecure (0/1), obfs (salamander), obfs-password,
// pinSHA256 (опционально), alpn (h3 по умолчанию).
//
// engine_compat: оба ядра. Xray-core поддерживает Hysteria2 outbound с
// версии 1.8.16 (сентябрь 2024); Mihomo — нативно с момента появления
// поддержки Hysteria2 в Clash Meta.

fn parse_hysteria2(uri: &str) -> Result<ProxyEntry> {
    let rest = uri
        .strip_prefix("hysteria2://")
        .or_else(|| uri.strip_prefix("hy2://"))
        .unwrap();

    let (rest, name) = split_fragment(rest);
    let (authority, query) = split_query(rest);
    let (password, host, port) = split_userinfo_hostport(authority)
        .context("некорректный authority в Hysteria2 URI")?;

    let mut raw = serde_json::Map::new();
    raw.insert("password".into(), url_decode(password).into());
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        if let Some((k, v)) = pair.split_once('=') {
            raw.insert(k.to_string(), url_decode(v).into());
        }
    }

    Ok(ProxyEntry {
        name,
        protocol: "hysteria2".to_string(),
        server: host.to_string(),
        port,
        raw: serde_json::Value::Object(raw),
        engine_compat: engines_both(),
    })
}

// ─── TUIC ────────────────────────────────────────────────────────────────────
//
// Формат: `tuic://uuid:password@server:port?sni=...&alpn=h3&congestion_control=bbr
//          &udp_relay_mode=quic&disable_sni=0#name`
//
// userinfo разделён двоеточием: до `:` — uuid, после — password.
//
// engine_compat: Mihomo only.

fn parse_tuic(uri: &str) -> Result<ProxyEntry> {
    let rest = uri.strip_prefix("tuic://").unwrap();

    let (rest, name) = split_fragment(rest);
    let (authority, query) = split_query(rest);
    let (userinfo, host, port) = split_userinfo_hostport(authority)
        .context("некорректный authority в TUIC URI")?;

    // userinfo: "uuid:password"
    let (uuid, password) = userinfo
        .split_once(':')
        .map(|(u, p)| (url_decode(u), url_decode(p)))
        .unwrap_or_else(|| (url_decode(userinfo), String::new()));

    let mut raw = serde_json::Map::new();
    raw.insert("uuid".into(), uuid.into());
    raw.insert("password".into(), password.into());
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        if let Some((k, v)) = pair.split_once('=') {
            raw.insert(k.to_string(), url_decode(v).into());
        }
    }

    Ok(ProxyEntry {
        name,
        protocol: "tuic".to_string(),
        server: host.to_string(),
        port,
        raw: serde_json::Value::Object(raw),
        engine_compat: engines_mihomo_only(),
    })
}

// ─── WireGuard ───────────────────────────────────────────────────────────────
//
// Формат: `wireguard://privateKey@server:port?publickey=...&address=10.0.0.2/32
//          &dns=1.1.1.1&mtu=1420&reserved=0,0,0&presharedkey=...#name`
//
// Также короткая форма `wg://...`. privateKey URL-encoded.
//
// engine_compat: оба ядра. Xray-core поддерживает WireGuard outbound
// с версии 1.8.6+ (через встроенный gVisor userspace stack); Mihomo —
// нативно.

fn parse_wireguard(uri: &str) -> Result<ProxyEntry> {
    let rest = uri
        .strip_prefix("wireguard://")
        .or_else(|| uri.strip_prefix("wg://"))
        .unwrap();

    let (rest, name) = split_fragment(rest);
    let (authority, query) = split_query(rest);
    let (private_key, host, port) = split_userinfo_hostport(authority)
        .context("некорректный authority в WireGuard URI")?;

    let mut raw = serde_json::Map::new();
    raw.insert("private-key".into(), url_decode(private_key).into());
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        if let Some((k, v)) = pair.split_once('=') {
            raw.insert(k.to_string(), url_decode(v).into());
        }
    }

    Ok(ProxyEntry {
        name,
        protocol: "wireguard".to_string(),
        server: host.to_string(),
        port,
        raw: serde_json::Value::Object(raw),
        engine_compat: engines_both(),
    })
}

// ─── SOCKS5 ──────────────────────────────────────────────────────────────────
//
// Формат: `socks5://[user:password@]host:port#name` (или `socks://...`).
// userinfo может отсутствовать — анонимный SOCKS-сервер.
//
// engine_compat: оба ядра (Xray имеет SOCKS outbound, Mihomo тоже).

fn parse_socks(uri: &str) -> Result<ProxyEntry> {
    let rest = uri
        .strip_prefix("socks5://")
        .or_else(|| uri.strip_prefix("socks://"))
        .unwrap();

    let (rest, name) = split_fragment(rest);
    let (authority, _query) = split_query(rest);

    // userinfo может быть в base64 (SIP-style) или открытым "user:pass"
    let (userinfo, host, port) = if authority.contains('@') {
        split_userinfo_hostport(authority)
            .context("некорректный authority в SOCKS URI")?
    } else {
        // Без userinfo — host:port
        let (h, p) = parse_hostport(authority)
            .context("некорректный host:port в SOCKS URI")?;
        ("", h, p)
    };

    let mut raw = serde_json::Map::new();
    if !userinfo.is_empty() {
        // Пробуем base64-декод (SIP-style). Если не вышло — берём как plaintext.
        let decoded = general_purpose::STANDARD
            .decode(userinfo)
            .or_else(|_| general_purpose::STANDARD_NO_PAD.decode(userinfo))
            .or_else(|_| general_purpose::URL_SAFE.decode(userinfo))
            .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(userinfo))
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .unwrap_or_else(|| url_decode(userinfo));

        if let Some((u, p)) = decoded.split_once(':') {
            raw.insert("username".into(), u.to_string().into());
            raw.insert("password".into(), p.to_string().into());
        } else {
            raw.insert("username".into(), decoded.into());
        }
    }

    Ok(ProxyEntry {
        name,
        protocol: "socks".to_string(),
        server: host.to_string(),
        port,
        raw: serde_json::Value::Object(raw),
        engine_compat: engines_both(),
    })
}

/// `"host:port"` или `"[ipv6]:port"` → (host, port).
fn parse_hostport(s: &str) -> Option<(&str, u16)> {
    let (host, port_str) = if s.starts_with('[') {
        let close = s.find(']')?;
        let port_str = s[close + 1..].strip_prefix(':')?;
        (&s[..=close], port_str)
    } else {
        let colon = s.rfind(':')?;
        (&s[..colon], &s[colon + 1..])
    };
    let port: u16 = port_str.parse().ok()?;
    Some((host, port))
}

// ─── вспомогательные функции ──────────────────────────────────────────────────

/// `"...#fragment"` → `("...", decoded_name)`
fn split_fragment(s: &str) -> (&str, String) {
    match s.rfind('#') {
        Some(i) => (&s[..i], url_decode(&s[i + 1..])),
        None => (s, "Unknown".to_string()),
    }
}

/// `"authority?query"` → `("authority", "query")`
fn split_query(s: &str) -> (&str, &str) {
    match s.find('?') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    }
}

/// `"user@host:port"` → `(user, host, port)`
fn split_userinfo_hostport(s: &str) -> Option<(&str, &str, u16)> {
    let at = s.rfind('@')?;
    let userinfo = &s[..at];
    let host_port = &s[at + 1..];

    let (host, port_str) = if host_port.starts_with('[') {
        // IPv6: [::1]:443
        let close = host_port.find(']')?;
        let port_str = host_port[close + 1..].strip_prefix(':')?;
        (&host_port[..=close], port_str)
    } else {
        let colon = host_port.rfind(':')?;
        (&host_port[..colon], &host_port[colon + 1..])
    };

    let port: u16 = port_str.parse().ok()?;
    Some((userinfo, host, port))
}

/// Декодирует URL-encoding (%XX), включая многобайтовые UTF-8 последовательности.
fn url_decode(s: &str) -> String {
    let bytes_in = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes_in.len());
    let mut i = 0;
    while i < bytes_in.len() {
        if bytes_in[i] == b'%' && i + 3 <= bytes_in.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes_in[i + 1..i + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    i += 3;
                    continue;
                }
            }
        } else if bytes_in[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes_in[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

// ─── plain text URI list ──────────────────────────────────────────────────────

fn parse_plain_uri_list(text: &str) -> Result<Vec<ProxyEntry>> {
    let entries: Vec<ProxyEntry> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| parse_proxy_uri(l).ok())
        .collect();

    if entries.is_empty() {
        bail!("нет URI в plain text");
    }
    Ok(entries)
}

// ─── Xray JSON конфиг as-is ───────────────────────────────────────────────────

/// Парсит Xray JSON: либо одиночный объект-конфиг, либо массив таких объектов.
/// Каждый конфиг становится отдельным ProxyEntry с name = `remarks`.
fn parse_xray_json(text: &str) -> Result<Vec<ProxyEntry>> {
    let json: serde_json::Value =
        serde_json::from_str(text).context("не удалось распарсить Xray JSON")?;

    let configs: Vec<serde_json::Value> = match json {
        serde_json::Value::Array(arr) => arr,
        obj @ serde_json::Value::Object(_) => vec![obj],
        _ => bail!("Xray JSON: ожидался объект или массив объектов"),
    };

    let entries: Vec<ProxyEntry> = configs
        .into_iter()
        .filter(|c| c.get("outbounds").is_some() || c.get("inbounds").is_some())
        .enumerate()
        .map(|(i, cfg)| {
            let name = cfg["remarks"]
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("Xray config #{}", i + 1));

            // Пытаемся «расковырять» JSON и выдать нормализованный ProxyEntry
            // со стандартным протоколом (vless/vmess/trojan/...). Тогда оба
            // ядра смогут поднять сервер: Xray — через свой config-builder,
            // Mihomo — через mihomo_config-builder. Большинство Marzban-style
            // подписок ровно такие — один основной outbound + direct/block.
            //
            // Если в JSON балансер (>1 VPN-outbound), кастомный routing,
            // или экзотический протокол — нормализация невозможна. Тогда
            // остаёмся в режиме «как есть» с engine_compat=xray-only.
            if let Some(normalized) = xray_json_to_normalized_entry(&cfg, &name) {
                return normalized;
            }

            ProxyEntry {
                name,
                protocol: "xray-json".to_string(),
                server: "127.0.0.1".to_string(),
                port: 0,
                raw: cfg,
                // Готовый Xray JSON конфигурируется только Xray-ядром.
                engine_compat: vec!["xray".to_string()],
            }
        })
        .collect();

    if entries.is_empty() {
        bail!("в Xray JSON нет ни одного конфига с inbounds/outbounds");
    }
    Ok(entries)
}

/// Извлекает основной VPN-outbound из готового Xray JSON и пересобирает
/// его в стандартный `ProxyEntry` с `engine_compat = both`. Возвращает
/// `None` если:
/// - в `outbounds` нет VPN-протокола (только direct/block/dns/api);
/// - VPN-outbound'ов больше одного (балансер);
/// - протокол не поддерживается ни Xray, ни Mihomo универсально.
///
/// Поля в `raw` нормализуются под формат, который ожидают URI-парсеры
/// (см. `parse_vless` / `parse_vmess` и т.д.) — чтобы один и тот же
/// `xray_config::build_*` / `mihomo_config::build_*_proxy` работал.
fn xray_json_to_normalized_entry(
    cfg: &serde_json::Value,
    name: &str,
) -> Option<ProxyEntry> {
    let outbounds = cfg.get("outbounds")?.as_array()?;
    let vpn_outbounds: Vec<_> = outbounds
        .iter()
        .filter(|ob| {
            let tag = ob.get("tag").and_then(|v| v.as_str()).unwrap_or("");
            let protocol = ob.get("protocol").and_then(|v| v.as_str()).unwrap_or("");
            !matches!(tag, "direct" | "block" | "dns" | "api")
                && !matches!(protocol, "freedom" | "blackhole" | "dns" | "")
        })
        .collect();

    // Ровно один VPN-outbound — простая запись. >1 = balancer, в этот
    // случай не лезем (теряем routing-логику пересборкой).
    if vpn_outbounds.len() != 1 {
        return None;
    }
    let main = vpn_outbounds[0];
    let protocol_str = main.get("protocol").and_then(|v| v.as_str())?;

    match protocol_str {
        "vless" => normalize_xray_vless(main, name),
        "vmess" => normalize_xray_vmess(main, name),
        "trojan" => normalize_xray_trojan(main, name),
        "shadowsocks" | "ss" => normalize_xray_ss(main, name),
        "hysteria2" => normalize_xray_hy2(main, name),
        "wireguard" => normalize_xray_wg(main, name),
        "socks" => normalize_xray_socks(main, name),
        _ => None,
    }
}

/// Извлечь общие поля streamSettings (network/security/SNI/transport-opts)
/// и записать в `raw` под именами, которые используют URI-парсеры. Без
/// этого `xray_config::build_stream` и `mihomo_config::apply_stream` не
/// смогут понять transport.
fn apply_stream_to_raw(raw: &mut serde_json::Map<String, serde_json::Value>, stream: &serde_json::Value) {
    let network = stream.get("network").and_then(|v| v.as_str()).unwrap_or("tcp");
    raw.insert("type".into(), network.to_string().into());

    let security = stream.get("security").and_then(|v| v.as_str()).unwrap_or("none");
    raw.insert("security".into(), security.to_string().into());

    // TLS settings
    if let Some(tls) = stream.get("tlsSettings") {
        if let Some(sni) = tls.get("serverName").and_then(|v| v.as_str()) {
            raw.insert("sni".into(), sni.to_string().into());
        }
        if let Some(fp) = tls.get("fingerprint").and_then(|v| v.as_str()) {
            raw.insert("fp".into(), fp.to_string().into());
        }
        if let Some(alpn_arr) = tls.get("alpn").and_then(|v| v.as_array()) {
            let joined: Vec<String> = alpn_arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
            if !joined.is_empty() {
                raw.insert("alpn".into(), joined.join(",").into());
            }
        }
        if tls.get("allowInsecure").and_then(|v| v.as_bool()).unwrap_or(false) {
            raw.insert("allowInsecure".into(), true.into());
        }
    }

    // REALITY settings
    if let Some(reality) = stream.get("realitySettings") {
        if let Some(sni) = reality.get("serverName").and_then(|v| v.as_str()) {
            raw.insert("sni".into(), sni.to_string().into());
        }
        if let Some(fp) = reality.get("fingerprint").and_then(|v| v.as_str()) {
            raw.insert("fp".into(), fp.to_string().into());
        }
        if let Some(pbk) = reality.get("publicKey").and_then(|v| v.as_str()) {
            raw.insert("pbk".into(), pbk.to_string().into());
        }
        if let Some(sid) = reality.get("shortId").and_then(|v| v.as_str()) {
            raw.insert("sid".into(), sid.to_string().into());
        }
        if let Some(spx) = reality.get("spiderX").and_then(|v| v.as_str()) {
            raw.insert("spx".into(), spx.to_string().into());
        }
    }

    // ws settings: path + Host header
    if let Some(ws) = stream.get("wsSettings") {
        if let Some(path) = ws.get("path").and_then(|v| v.as_str()) {
            raw.insert("path".into(), path.to_string().into());
        }
        if let Some(host) = ws.get("headers").and_then(|h| h.get("Host")).and_then(|v| v.as_str()) {
            raw.insert("host".into(), host.to_string().into());
        } else if let Some(host) = ws.get("host").and_then(|v| v.as_str()) {
            raw.insert("host".into(), host.to_string().into());
        }
    }

    // grpc settings
    if let Some(grpc) = stream.get("grpcSettings") {
        if let Some(svc) = grpc.get("serviceName").and_then(|v| v.as_str()) {
            raw.insert("serviceName".into(), svc.to_string().into());
        }
        if let Some(mode) = grpc.get("multiMode").and_then(|v| v.as_bool()) {
            raw.insert("mode".into(), if mode { "multi" } else { "gun" }.to_string().into());
        }
    }

    // h2 settings
    if let Some(h2) = stream.get("httpSettings") {
        if let Some(path) = h2.get("path").and_then(|v| v.as_str()) {
            raw.insert("path".into(), path.to_string().into());
        }
        if let Some(host_arr) = h2.get("host").and_then(|v| v.as_array()) {
            if let Some(first) = host_arr.first().and_then(|v| v.as_str()) {
                raw.insert("host".into(), first.to_string().into());
            }
        }
    }

    // xhttp / httpupgrade settings — для 8.A.1
    if let Some(xh) = stream.get("xhttpSettings") {
        if let Some(path) = xh.get("path").and_then(|v| v.as_str()) {
            raw.insert("path".into(), path.to_string().into());
        }
        if let Some(host) = xh.get("host").and_then(|v| v.as_str()) {
            raw.insert("host".into(), host.to_string().into());
        }
        if let Some(mode) = xh.get("mode").and_then(|v| v.as_str()) {
            raw.insert("mode".into(), mode.to_string().into());
        }
    }
    if let Some(hu) = stream.get("httpupgradeSettings") {
        if let Some(path) = hu.get("path").and_then(|v| v.as_str()) {
            raw.insert("path".into(), path.to_string().into());
        }
        if let Some(host) = hu.get("host").and_then(|v| v.as_str()) {
            raw.insert("host".into(), host.to_string().into());
        }
    }
}

fn normalize_xray_vless(ob: &serde_json::Value, name: &str) -> Option<ProxyEntry> {
    let vnext = ob.get("settings")?.get("vnext")?.as_array()?.first()?;
    let server = vnext.get("address")?.as_str()?.to_string();
    let port = vnext.get("port")?.as_u64()? as u16;
    let user = vnext.get("users")?.as_array()?.first()?;
    let uuid = user.get("id")?.as_str()?.to_string();

    let mut raw = serde_json::Map::new();
    raw.insert("uuid".into(), uuid.into());
    if let Some(flow) = user.get("flow").and_then(|v| v.as_str()) {
        if !flow.is_empty() {
            raw.insert("flow".into(), flow.to_string().into());
        }
    }
    if let Some(enc) = user.get("encryption").and_then(|v| v.as_str()) {
        raw.insert("encryption".into(), enc.to_string().into());
    }
    if let Some(stream) = ob.get("streamSettings") {
        apply_stream_to_raw(&mut raw, stream);
    }

    Some(ProxyEntry {
        name: name.to_string(),
        protocol: "vless".to_string(),
        server,
        port,
        raw: serde_json::Value::Object(raw),
        engine_compat: engines_both(),
    })
}

fn normalize_xray_vmess(ob: &serde_json::Value, name: &str) -> Option<ProxyEntry> {
    let vnext = ob.get("settings")?.get("vnext")?.as_array()?.first()?;
    let server = vnext.get("address")?.as_str()?.to_string();
    let port = vnext.get("port")?.as_u64()? as u16;
    let user = vnext.get("users")?.as_array()?.first()?;
    let uuid = user.get("id")?.as_str()?.to_string();

    // VMess JSON URI parser ожидает поля add/port/id/aid/net/tls/sni/host/path/scy
    // (legacy v2rayN base64 формат). Нормализуем сразу под него.
    let mut raw = serde_json::Map::new();
    raw.insert("ps".into(), name.to_string().into());
    raw.insert("add".into(), server.clone().into());
    raw.insert("port".into(), (port as u64).into());
    raw.insert("id".into(), uuid.into());

    let aid = user.get("alterId").and_then(|v| v.as_u64()).unwrap_or(0);
    raw.insert("aid".into(), (aid as u64).into());

    let cipher = user.get("security").and_then(|v| v.as_str()).unwrap_or("auto");
    raw.insert("scy".into(), cipher.to_string().into());

    let stream = ob.get("streamSettings");
    let network = stream
        .and_then(|s| s.get("network"))
        .and_then(|v| v.as_str())
        .unwrap_or("tcp");
    raw.insert("net".into(), network.to_string().into());

    let security = stream
        .and_then(|s| s.get("security"))
        .and_then(|v| v.as_str())
        .unwrap_or("none");
    raw.insert("tls".into(), if security == "tls" { "tls" } else { "" }.to_string().into());

    if let Some(s) = stream {
        if let Some(tls) = s.get("tlsSettings") {
            if let Some(sni) = tls.get("serverName").and_then(|v| v.as_str()) {
                raw.insert("sni".into(), sni.to_string().into());
            }
            if let Some(fp) = tls.get("fingerprint").and_then(|v| v.as_str()) {
                raw.insert("fp".into(), fp.to_string().into());
            }
            if let Some(alpn_arr) = tls.get("alpn").and_then(|v| v.as_array()) {
                let joined: Vec<String> = alpn_arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                if !joined.is_empty() {
                    raw.insert("alpn".into(), joined.join(",").into());
                }
            }
        }
        if let Some(ws) = s.get("wsSettings") {
            if let Some(path) = ws.get("path").and_then(|v| v.as_str()) {
                raw.insert("path".into(), path.to_string().into());
            }
            if let Some(host) = ws
                .get("headers").and_then(|h| h.get("Host"))
                .or_else(|| ws.get("host"))
                .and_then(|v| v.as_str())
            {
                raw.insert("host".into(), host.to_string().into());
            }
        }
        if let Some(grpc) = s.get("grpcSettings") {
            if let Some(svc) = grpc.get("serviceName").and_then(|v| v.as_str()) {
                raw.insert("serviceName".into(), svc.to_string().into());
                raw.insert("path".into(), svc.to_string().into());
            }
        }
    }

    Some(ProxyEntry {
        name: name.to_string(),
        protocol: "vmess".to_string(),
        server,
        port,
        raw: serde_json::Value::Object(raw),
        engine_compat: engines_both(),
    })
}

fn normalize_xray_trojan(ob: &serde_json::Value, name: &str) -> Option<ProxyEntry> {
    let srv = ob.get("settings")?.get("servers")?.as_array()?.first()?;
    let server = srv.get("address")?.as_str()?.to_string();
    let port = srv.get("port")?.as_u64()? as u16;
    let password = srv.get("password")?.as_str()?.to_string();

    let mut raw = serde_json::Map::new();
    raw.insert("password".into(), password.into());
    if let Some(stream) = ob.get("streamSettings") {
        apply_stream_to_raw(&mut raw, stream);
    }

    Some(ProxyEntry {
        name: name.to_string(),
        protocol: "trojan".to_string(),
        server,
        port,
        raw: serde_json::Value::Object(raw),
        engine_compat: engines_both(),
    })
}

fn normalize_xray_ss(ob: &serde_json::Value, name: &str) -> Option<ProxyEntry> {
    let srv = ob.get("settings")?.get("servers")?.as_array()?.first()?;
    let server = srv.get("address")?.as_str()?.to_string();
    let port = srv.get("port")?.as_u64()? as u16;
    let cipher = srv.get("method")?.as_str()?.to_string();
    let password = srv.get("password")?.as_str()?.to_string();

    let mut raw = serde_json::Map::new();
    raw.insert("cipher".into(), cipher.into());
    raw.insert("password".into(), password.into());

    Some(ProxyEntry {
        name: name.to_string(),
        protocol: "ss".to_string(),
        server,
        port,
        raw: serde_json::Value::Object(raw),
        engine_compat: engines_both(),
    })
}

fn normalize_xray_hy2(ob: &serde_json::Value, name: &str) -> Option<ProxyEntry> {
    let srv = ob.get("settings")?.get("servers")?.as_array()?.first()?;
    let server = srv.get("address")?.as_str()?.to_string();
    let port = srv.get("port")?.as_u64()? as u16;
    let password = srv.get("password")?.as_str()?.to_string();

    let mut raw = serde_json::Map::new();
    raw.insert("password".into(), password.into());

    if let Some(obfs) = srv.get("obfs").and_then(|v| v.as_str()) {
        if !obfs.is_empty() {
            raw.insert("obfs".into(), obfs.to_string().into());
        }
    }
    if let Some(obfs_pass) = srv.get("obfs-password").or_else(|| srv.get("obfsPassword")).and_then(|v| v.as_str()) {
        if !obfs_pass.is_empty() {
            raw.insert("obfs-password".into(), obfs_pass.to_string().into());
        }
    }
    if let Some(stream) = ob.get("streamSettings") {
        if let Some(tls) = stream.get("tlsSettings") {
            if let Some(sni) = tls.get("serverName").and_then(|v| v.as_str()) {
                raw.insert("sni".into(), sni.to_string().into());
            }
            if tls.get("allowInsecure").and_then(|v| v.as_bool()).unwrap_or(false) {
                raw.insert("insecure".into(), "1".to_string().into());
            }
            if let Some(alpn_arr) = tls.get("alpn").and_then(|v| v.as_array()) {
                let joined: Vec<String> = alpn_arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                if !joined.is_empty() {
                    raw.insert("alpn".into(), joined.join(",").into());
                }
            }
        }
    }

    Some(ProxyEntry {
        name: name.to_string(),
        protocol: "hysteria2".to_string(),
        server,
        port,
        raw: serde_json::Value::Object(raw),
        engine_compat: engines_both(),
    })
}

fn normalize_xray_wg(ob: &serde_json::Value, name: &str) -> Option<ProxyEntry> {
    let settings = ob.get("settings")?;
    let private_key = settings.get("secretKey")?.as_str()?.to_string();
    let peer = settings.get("peers")?.as_array()?.first()?;
    let endpoint = peer.get("endpoint")?.as_str()?;
    let (server, port) = parse_hostport(endpoint)?;

    let mut raw = serde_json::Map::new();
    raw.insert("private-key".into(), private_key.into());
    if let Some(pubk) = peer.get("publicKey").and_then(|v| v.as_str()) {
        raw.insert("publickey".into(), pubk.to_string().into());
    }
    if let Some(psk) = peer.get("preSharedKey").and_then(|v| v.as_str()) {
        if !psk.is_empty() {
            raw.insert("presharedkey".into(), psk.to_string().into());
        }
    }
    if let Some(addrs) = settings.get("address").and_then(|v| v.as_array()) {
        if let Some(first) = addrs.first().and_then(|v| v.as_str()) {
            raw.insert("address".into(), first.to_string().into());
        }
    }
    if let Some(mtu) = settings.get("mtu").and_then(|v| v.as_u64()) {
        raw.insert("mtu".into(), (mtu as u64).into());
    }
    if let Some(reserved) = settings.get("reserved").and_then(|v| v.as_array()) {
        let joined: Vec<String> = reserved.iter().filter_map(|v| v.as_u64().map(|n| n.to_string())).collect();
        if !joined.is_empty() {
            raw.insert("reserved".into(), joined.join(",").into());
        }
    }

    Some(ProxyEntry {
        name: name.to_string(),
        protocol: "wireguard".to_string(),
        server: server.to_string(),
        port,
        raw: serde_json::Value::Object(raw),
        engine_compat: engines_both(),
    })
}

fn normalize_xray_socks(ob: &serde_json::Value, name: &str) -> Option<ProxyEntry> {
    let srv = ob.get("settings")?.get("servers")?.as_array()?.first()?;
    let server = srv.get("address")?.as_str()?.to_string();
    let port = srv.get("port")?.as_u64()? as u16;

    let mut raw = serde_json::Map::new();
    if let Some(users) = srv.get("users").and_then(|v| v.as_array()) {
        if let Some(user) = users.first() {
            if let Some(u) = user.get("user").and_then(|v| v.as_str()) {
                raw.insert("username".into(), u.to_string().into());
            }
            if let Some(p) = user.get("pass").and_then(|v| v.as_str()) {
                raw.insert("password".into(), p.to_string().into());
            }
        }
    }

    Some(ProxyEntry {
        name: name.to_string(),
        protocol: "socks".to_string(),
        server,
        port,
        raw: serde_json::Value::Object(raw),
        engine_compat: engines_both(),
    })
}

// ─── Clash YAML fallback ──────────────────────────────────────────────────────

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

    let raw = serde_json::to_value(&v)
        .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()));

    // Engine-compat по протоколу. Mihomo-only — только TUIC и AnyTLS;
    // остальное (включая hy2/wireguard, которые поддерживает современный
    // Xray-core) — оба ядра.
    let engine_compat = match protocol.as_str() {
        "tuic" | "anytls" => engines_mihomo_only(),
        _ => engines_both(),
    };

    Ok(ProxyEntry {
        name,
        protocol,
        server,
        port,
        raw,
        engine_compat,
    })
}
