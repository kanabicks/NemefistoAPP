//! Генерация YAML-конфига Mihomo (Clash Meta) из ProxyEntry — этап 8.B.
//!
//! Симметричен `xray_config.rs`. Возвращает готовую YAML-строку, которая
//! записывается в `%TEMP%\NemefistoVPN\mihomo-config.yaml` и подсовывается
//! Mihomo через `-f <file>`.
//!
//! Поддерживаемые протоколы: всё что умеет Mihomo — vless / vmess / trojan /
//! ss / socks5 / hysteria2 / tuic / wireguard / anytls / mieru.
//!
//! Anti-DPI:
//! - **fragmentation / noises** Mihomo не имеет — игнорируем (UI скрывает
//!   эти секции при `engine = mihomo`);
//! - **server-resolve через DoH** реализуем через `dns.nameserver` (DoH
//!   endpoint) + `dns.default-nameserver` (bootstrap IP).
//!
//! Routing на v1: единое правило `MATCH,PROXY` (всё через прокси), как
//! сейчас в Xray-конфиге. Routing-профили из этапа 11 добавляются позже.
//!
//! Engine API control (HTTP API на 127.0.0.1:9090) включаем с случайным
//! `secret`-паролем — на v1 не используется, но открывает дорогу для
//! 13.I bandwidth-метра и smart auto-failover (13.C).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

use super::server::ProxyEntry;
use super::sing_box_config::AntiDpiOptions;

/// Per-process routing rule (этап 8.D). Принимается из фронта через
/// `connect()` и транслируется в Mihomo `PROCESS-NAME,<exe>,<action>`.
///
/// Frontend кладёт массив таких объектов в payload `connect`; serde
/// разбирает поля через camelCase rename. Xray-движок игнорирует
/// эти правила (UI предупреждает заранее).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRule {
    pub exe: String,
    /// `"proxy"` | `"direct"` | `"block"` — мапится в `PROXY` / `DIRECT` /
    /// `REJECT` правил Mihomo соответственно.
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// Результат генерации: YAML-текст + порт `mixed-port` (SOCKS5 + HTTP).
pub struct MihomoConfig {
    pub yaml: String,
    pub mixed_port: u16,
}

/// Построить mihomo-конфиг для одного сервера.
///
/// `listen` — `127.0.0.1` (loopback) или `0.0.0.0` (LAN).
/// `socks_auth` — `Some((user, pass))` если включён auth для inbound (9.G);
/// иначе `None` (proxy-режим на loopback, без аутентификации).
/// `app_rules` — per-process правила (этап 8.D); пустой slice = no-op.
pub fn build(
    entry: &ProxyEntry,
    mixed_port: u16,
    listen: &str,
    anti_dpi: Option<&AntiDpiOptions>,
    socks_auth: Option<(&str, &str)>,
    app_rules: &[AppRule],
    routing_profile: Option<&super::routing_profile::RoutingProfile>,
) -> Result<MihomoConfig> {
    let proxy = proxy_for_entry(entry)
        .with_context(|| format!("не удалось собрать mihomo-proxy для «{}»", entry.name))?;

    // Имя для proxy внутри Mihomo-конфига должно быть стабильным и не
    // конфликтовать с зарезервированными "DIRECT" / "REJECT" / "PROXY".
    let proxy_name = "VPN-NODE".to_string();
    let mut proxy_map = proxy;
    proxy_map.insert("name".into(), proxy_name.clone().into());

    let mut root = Mapping::new();

    // ── Inbound ──────────────────────────────────────────────────────
    root.insert("mixed-port".into(), (mixed_port as u64).into());
    root.insert("allow-lan".into(), (listen == "0.0.0.0").into());
    root.insert("bind-address".into(), listen.to_string().into());

    // 9.G: SOCKS5 auth для TUN/LAN. Mihomo принимает массив строк
    // вида "user:pass". Если auth не задан — секция отсутствует.
    if let Some((user, pass)) = socks_auth {
        let mut auth = Vec::new();
        auth.push(Value::from(format!("{user}:{pass}")));
        root.insert("authentication".into(), Value::Sequence(auth));
    }

    // ── Базовое поведение ────────────────────────────────────────────
    root.insert("mode".into(), "rule".into());
    root.insert("log-level".into(), "info".into());
    root.insert("ipv6".into(), false.into());

    // 8.D: per-process routing требует `find-process-mode: always` —
    // Mihomo при каждом новом соединении проверяет какой процесс его
    // создал (через WMI / iptables-conntrack lookup на других ОС).
    // Включаем только если правила непустые — иначе лишний overhead.
    if !app_rules.is_empty() {
        root.insert("find-process-mode".into(), "always".into());
    }

    // External controller — чтобы потенциально можно было дёргать API
    // (для 13.I bandwidth-метра, 13.C smart failover). Secret —
    // случайный, через UUID, чтобы посторонние процессы не достучались.
    root.insert(
        "external-controller".into(),
        format!("127.0.0.1:{}", mixed_port + 1).into(),
    );
    root.insert("secret".into(), uuid::Uuid::new_v4().to_string().into());

    // ── DNS ──────────────────────────────────────────────────────────
    // Включаем всегда, чтобы предотвратить DNS-leak (аналог Prizrak-Box
    // DNS rewrite). При активном anti-DPI server-resolve — берём DoH
    // и bootstrap из настроек, иначе разумные дефолты.
    root.insert("dns".into(), Value::Mapping(build_dns(anti_dpi)));

    // ── Proxies / proxy-groups / rules ───────────────────────────────
    root.insert(
        "proxies".into(),
        Value::Sequence(vec![Value::Mapping(proxy_map)]),
    );

    let mut group = Mapping::new();
    group.insert("name".into(), "PROXY".into());
    group.insert("type".into(), "select".into());
    group.insert(
        "proxies".into(),
        Value::Sequence(vec![Value::String(proxy_name)]),
    );
    root.insert(
        "proxy-groups".into(),
        Value::Sequence(vec![Value::Mapping(group)]),
    );

    // 8.D: per-process правила (если заданы) идут перед routing-профилем
    // и MATCH'ем — чтобы перехватить трафик конкретных процессов
    // первыми. Action нормализуется: proxy→PROXY, direct→DIRECT,
    // block→REJECT.
    let mut rules: Vec<Value> = Vec::new();
    for r in app_rules {
        if r.exe.trim().is_empty() {
            continue;
        }
        let target = match r.action.as_str() {
            "direct" => "DIRECT",
            "block" => "REJECT",
            _ => "PROXY",
        };
        rules.push(Value::String(format!("PROCESS-NAME,{},{}", r.exe.trim(), target)));
    }

    // 11.F: правила из routing-профиля. block — первый (override любого
    // direct/proxy), потом direct, потом proxy. После — MATCH с дефолтом
    // от GlobalProxy.
    let default_action = if let Some(p) = routing_profile {
        for r in mihomo_rules_from_profile(p) {
            rules.push(Value::String(r));
        }
        if p.global_proxy.0 { "PROXY" } else { "DIRECT" }
    } else {
        "PROXY"
    };
    rules.push(Value::String(format!("MATCH,{default_action}")));
    root.insert("rules".into(), Value::Sequence(rules));

    let yaml = serde_yaml::to_string(&Value::Mapping(root))
        .context("сериализация mihomo YAML")?;

    Ok(MihomoConfig { yaml, mixed_port })
}

/// 11.F: преобразовать правила routing-профиля в Mihomo-формат строк.
///
/// Маппинг:
/// - `geosite:ru` → `GEOSITE,ru,DIRECT`
/// - `geoip:ru` → `GEOIP,ru,DIRECT,no-resolve`
/// - `1.2.3.4/24` или `::1/128` → `IP-CIDR,...,DIRECT,no-resolve`
/// - конкретный IP без / → `IP-CIDR,IP/32,...,no-resolve`
/// - домен типа `example.com` → `DOMAIN-SUFFIX,example.com,DIRECT`
/// - `*.example.com` → `DOMAIN-SUFFIX,example.com,DIRECT`
/// - `keyword:word` → `DOMAIN-KEYWORD,word,DIRECT`
///
/// Order: block → direct → proxy (block перебивает остальное).
fn mihomo_rules_from_profile(p: &super::routing_profile::RoutingProfile) -> Vec<String> {
    let mut out = Vec::new();
    push_site_rules(&mut out, &p.block_sites, "REJECT");
    push_ip_rules(&mut out, &p.block_ip, "REJECT");
    push_site_rules(&mut out, &p.direct_sites, "DIRECT");
    push_ip_rules(&mut out, &p.direct_ip, "DIRECT");
    push_site_rules(&mut out, &p.proxy_sites, "PROXY");
    push_ip_rules(&mut out, &p.proxy_ip, "PROXY");
    out
}

fn push_site_rules(out: &mut Vec<String>, sites: &[String], action: &str) {
    for s in sites {
        let s = s.trim();
        if s.is_empty() {
            continue;
        }
        if let Some(rest) = s.strip_prefix("geosite:") {
            out.push(format!("GEOSITE,{rest},{action}"));
        } else if let Some(rest) = s.strip_prefix("keyword:") {
            out.push(format!("DOMAIN-KEYWORD,{rest},{action}"));
        } else if let Some(rest) = s.strip_prefix("*.") {
            out.push(format!("DOMAIN-SUFFIX,{rest},{action}"));
        } else if let Some(rest) = s.strip_prefix("regex:") {
            // Mihomo не имеет regex matcher — пропускаем с warning
            eprintln!("[mihomo-rules] regex не поддерживается, skip: {rest}");
        } else if s.starts_with("domain:") {
            out.push(format!("DOMAIN,{},{action}", &s[7..]));
        } else if s.contains('.') {
            // Простой domain — DOMAIN-SUFFIX (включает все subdomains)
            out.push(format!("DOMAIN-SUFFIX,{s},{action}"));
        } else {
            // Без точки — обрабатываем как DOMAIN-KEYWORD
            out.push(format!("DOMAIN-KEYWORD,{s},{action}"));
        }
    }
}

fn push_ip_rules(out: &mut Vec<String>, ips: &[String], action: &str) {
    for s in ips {
        let s = s.trim();
        if s.is_empty() {
            continue;
        }
        if let Some(rest) = s.strip_prefix("geoip:") {
            out.push(format!("GEOIP,{rest},{action},no-resolve"));
        } else if s.contains('/') {
            // Уже CIDR
            let kind = if s.contains(':') { "IP-CIDR6" } else { "IP-CIDR" };
            out.push(format!("{kind},{s},{action},no-resolve"));
        } else if let Ok(addr) = s.parse::<std::net::IpAddr>() {
            // Конкретный IP без префикса — добавляем /32 или /128
            let suffix = if addr.is_ipv6() { "/128" } else { "/32" };
            let kind = if addr.is_ipv6() { "IP-CIDR6" } else { "IP-CIDR" };
            out.push(format!("{kind},{s}{suffix},{action},no-resolve"));
        } else {
            eprintln!("[mihomo-rules] невалидный IP/CIDR, skip: {s}");
        }
    }
}

/// Собрать DNS-секцию. С DoH-резолвом (10.C) если активен, иначе
/// минимальные дефолты с бутстрапом на 1.1.1.1 / 8.8.8.8.
fn build_dns(anti_dpi: Option<&AntiDpiOptions>) -> Mapping {
    let mut dns = Mapping::new();
    dns.insert("enable".into(), true.into());
    dns.insert("listen".into(), "0.0.0.0:0".into()); // не открываем DNS-сервер наружу
    dns.insert("ipv6".into(), false.into());
    dns.insert("enhanced-mode".into(), "redir-host".into());

    let dpi_resolve = anti_dpi.map(|d| d.server_resolve).unwrap_or(false);

    if dpi_resolve {
        let bootstrap = anti_dpi.map(|d| d.server_resolve_bootstrap.as_str()).unwrap_or("1.1.1.1");
        let doh = anti_dpi.map(|d| d.server_resolve_doh.as_str())
            .unwrap_or("https://cloudflare-dns.com/dns-query");

        dns.insert(
            "default-nameserver".into(),
            Value::Sequence(vec![bootstrap.into()]),
        );
        dns.insert(
            "nameserver".into(),
            Value::Sequence(vec![doh.into()]),
        );
    } else {
        dns.insert(
            "default-nameserver".into(),
            Value::Sequence(vec!["1.1.1.1".into(), "8.8.8.8".into()]),
        );
        dns.insert(
            "nameserver".into(),
            Value::Sequence(vec![
                "https://cloudflare-dns.com/dns-query".into(),
                "https://dns.google/dns-query".into(),
            ]),
        );
    }
    dns
}

// ─── Per-protocol mappers ────────────────────────────────────────────────────

/// Главная точка входа: переводит `ProxyEntry` в один YAML-mapping в
/// формате mihomo. Для записей из clash YAML — passthrough; для записей
/// из URI-парсеров — собираем поля по протоколу.
fn proxy_for_entry(entry: &ProxyEntry) -> Result<Mapping> {
    // Записи из clash YAML кладут в `raw` всё mapping подряд, включая
    // топ-уровневые поля name/server/port/type. URI-парсеры таких полей
    // в raw не пишут (имя/сервер/порт хранятся отдельно в ProxyEntry).
    // Используем «есть `name` в raw» как маркер clash-shape.
    let from_yaml = entry.raw.get("name").and_then(|v| v.as_str()).is_some();
    if from_yaml {
        return passthrough_proxy(entry);
    }

    match entry.protocol.as_str() {
        "vless" => build_vless_proxy(entry),
        "vmess" => build_vmess_proxy(entry),
        "trojan" => build_trojan_proxy(entry),
        "ss" => build_ss_proxy(entry),
        "hysteria2" => build_hysteria2_proxy(entry),
        "tuic" => build_tuic_proxy(entry),
        "wireguard" => build_wireguard_proxy(entry),
        "socks" => build_socks_proxy(entry),
        other => bail!("протокол '{other}' не поддерживается mihomo-конвертером"),
    }
}

/// Прямая конверсия raw JSON-mapping → YAML mapping. Используется когда
/// запись пришла из clash YAML и уже имеет нужную форму.
fn passthrough_proxy(entry: &ProxyEntry) -> Result<Mapping> {
    let yaml: Value = serde_yaml::to_value(&entry.raw)
        .context("конверсия JSON→YAML для passthrough proxy")?;
    let mut map = match yaml {
        Value::Mapping(m) => m,
        _ => bail!("raw entry не является объектом"),
    };
    // Принудительно проставляем server/port/name — на случай если в raw
    // лёгкое расхождение с обновлённым ProxyEntry.
    map.insert("name".into(), entry.name.clone().into());
    map.insert("server".into(), entry.server.clone().into());
    map.insert("port".into(), (entry.port as u64).into());
    // Ензайно гарантируем тип — clash YAML всегда содержит `type`,
    // но подстраховаться полезно.
    if !map.contains_key(Value::from("type")) {
        map.insert("type".into(), entry.protocol.clone().into());
    }
    Ok(map)
}

// ── helpers ──────────────────────────────────────────────────────────────

fn s(raw: &serde_json::Value, key: &str) -> Option<String> {
    raw.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn b(raw: &serde_json::Value, key: &str) -> Option<bool> {
    if let Some(v) = raw.get(key) {
        if let Some(b) = v.as_bool() {
            return Some(b);
        }
        if let Some(s) = v.as_str() {
            return Some(matches!(s, "1" | "true" | "yes"));
        }
        if let Some(n) = v.as_u64() {
            return Some(n != 0);
        }
    }
    None
}

fn base_proxy(entry: &ProxyEntry, type_name: &str) -> Mapping {
    let mut m = Mapping::new();
    m.insert("type".into(), type_name.into());
    m.insert("server".into(), entry.server.clone().into());
    m.insert("port".into(), (entry.port as u64).into());
    m.insert("udp".into(), true.into());
    m
}

/// Применить общие TLS/network/transport поля из URI-формата к
/// proxy-mapping. Универсально для vless/vmess/trojan.
fn apply_stream(map: &mut Mapping, raw: &serde_json::Value) {
    // network: tcp / ws / grpc / h2 / httpupgrade / xhttp
    let network = s(raw, "type").unwrap_or_else(|| "tcp".to_string());
    map.insert("network".into(), network.clone().into());

    // TLS / REALITY
    let security = s(raw, "security").unwrap_or_default();
    let tls_on = !security.is_empty() && security != "none";
    if tls_on {
        map.insert("tls".into(), true.into());
    }
    if let Some(sni) = s(raw, "sni") {
        if !sni.is_empty() {
            map.insert("servername".into(), sni.into());
        }
    }
    if let Some(fp) = s(raw, "fp") {
        if !fp.is_empty() {
            map.insert("client-fingerprint".into(), fp.into());
        }
    }
    if let Some(alpn) = s(raw, "alpn") {
        if !alpn.is_empty() {
            let arr: Vec<Value> = alpn.split(',').map(|s| Value::from(s.trim().to_string())).collect();
            map.insert("alpn".into(), Value::Sequence(arr));
        }
    }
    if b(raw, "allowInsecure").unwrap_or(false) || b(raw, "insecure").unwrap_or(false) {
        map.insert("skip-cert-verify".into(), true.into());
    }
    // REALITY (vless only обычно)
    if security == "reality" {
        let mut ro = Mapping::new();
        if let Some(pbk) = s(raw, "pbk") {
            ro.insert("public-key".into(), pbk.into());
        }
        if let Some(sid) = s(raw, "sid") {
            ro.insert("short-id".into(), sid.into());
        }
        if !ro.is_empty() {
            map.insert("reality-opts".into(), Value::Mapping(ro));
        }
    }

    // ws-opts
    if network == "ws" {
        let mut ws = Mapping::new();
        if let Some(path) = s(raw, "path") {
            ws.insert("path".into(), path.into());
        }
        if let Some(host) = s(raw, "host") {
            if !host.is_empty() {
                let mut headers = Mapping::new();
                headers.insert("Host".into(), host.into());
                ws.insert("headers".into(), Value::Mapping(headers));
            }
        }
        if !ws.is_empty() {
            map.insert("ws-opts".into(), Value::Mapping(ws));
        }
    }
    // grpc-opts
    if network == "grpc" {
        let mut g = Mapping::new();
        if let Some(svc) = s(raw, "serviceName").or_else(|| s(raw, "path")) {
            g.insert("grpc-service-name".into(), svc.into());
        }
        if !g.is_empty() {
            map.insert("grpc-opts".into(), Value::Mapping(g));
        }
    }
    // h2-opts
    if network == "h2" {
        let mut h = Mapping::new();
        if let Some(path) = s(raw, "path") {
            h.insert("path".into(), path.into());
        }
        if let Some(host) = s(raw, "host") {
            if !host.is_empty() {
                h.insert("host".into(), Value::Sequence(vec![host.into()]));
            }
        }
        if !h.is_empty() {
            map.insert("h2-opts".into(), Value::Mapping(h));
        }
    }
}

fn build_vless_proxy(entry: &ProxyEntry) -> Result<Mapping> {
    let raw = &entry.raw;
    let mut m = base_proxy(entry, "vless");
    let uuid = s(raw, "uuid").context("vless: uuid обязателен")?;
    m.insert("uuid".into(), uuid.into());

    if let Some(flow) = s(raw, "flow") {
        if !flow.is_empty() {
            m.insert("flow".into(), flow.into());
        }
    }
    apply_stream(&mut m, raw);
    Ok(m)
}

fn build_vmess_proxy(entry: &ProxyEntry) -> Result<Mapping> {
    let raw = &entry.raw;
    let mut m = base_proxy(entry, "vmess");

    let uuid = s(raw, "id").context("vmess: id (uuid) обязателен")?;
    m.insert("uuid".into(), uuid.into());

    let aid = raw
        .get("aid")
        .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
        .unwrap_or(0);
    m.insert("alterId".into(), (aid as u64).into());

    let cipher = s(raw, "scy").unwrap_or_else(|| "auto".to_string());
    m.insert("cipher".into(), cipher.into());

    // network в vmess JSON хранится в поле "net", security в "tls".
    // Создаём synthetic raw где имена нормализованы под apply_stream
    // (которая ждёт "type" / "security").
    let mut synth = serde_json::Map::new();
    if let Some(net) = s(raw, "net") {
        synth.insert("type".into(), net.into());
    }
    if let Some(tls) = s(raw, "tls") {
        if tls == "tls" || tls == "1" || tls == "true" {
            synth.insert("security".into(), "tls".into());
        }
    }
    for k in ["sni", "fp", "alpn", "host", "path", "serviceName"] {
        if let Some(v) = s(raw, k) {
            synth.insert(k.into(), v.into());
        }
    }
    let synth_v = serde_json::Value::Object(synth);
    apply_stream(&mut m, &synth_v);
    Ok(m)
}

fn build_trojan_proxy(entry: &ProxyEntry) -> Result<Mapping> {
    let raw = &entry.raw;
    let mut m = base_proxy(entry, "trojan");

    let password = s(raw, "password").context("trojan: password обязателен")?;
    m.insert("password".into(), password.into());

    if let Some(sni) = s(raw, "sni") {
        if !sni.is_empty() {
            m.insert("sni".into(), sni.into());
        }
    }
    if let Some(alpn) = s(raw, "alpn") {
        if !alpn.is_empty() {
            let arr: Vec<Value> = alpn.split(',').map(|s| Value::from(s.trim().to_string())).collect();
            m.insert("alpn".into(), Value::Sequence(arr));
        }
    }
    if b(raw, "allowInsecure").unwrap_or(false) {
        m.insert("skip-cert-verify".into(), true.into());
    }
    // Сетевой транспорт + ws/grpc opts
    apply_stream(&mut m, raw);
    Ok(m)
}

fn build_ss_proxy(entry: &ProxyEntry) -> Result<Mapping> {
    let raw = &entry.raw;
    let mut m = base_proxy(entry, "ss");
    let cipher = s(raw, "cipher").context("ss: cipher обязателен")?;
    let password = s(raw, "password").context("ss: password обязателен")?;
    m.insert("cipher".into(), cipher.into());
    m.insert("password".into(), password.into());
    Ok(m)
}

fn build_hysteria2_proxy(entry: &ProxyEntry) -> Result<Mapping> {
    let raw = &entry.raw;
    let mut m = base_proxy(entry, "hysteria2");

    let password = s(raw, "password").context("hysteria2: password обязателен")?;
    m.insert("password".into(), password.into());

    if let Some(obfs) = s(raw, "obfs") {
        if !obfs.is_empty() {
            m.insert("obfs".into(), obfs.into());
        }
    }
    if let Some(obfs_pass) = s(raw, "obfs-password").or_else(|| s(raw, "obfsPassword")) {
        if !obfs_pass.is_empty() {
            m.insert("obfs-password".into(), obfs_pass.into());
        }
    }
    if let Some(sni) = s(raw, "sni") {
        if !sni.is_empty() {
            m.insert("sni".into(), sni.into());
        }
    }
    if b(raw, "insecure").unwrap_or(false) {
        m.insert("skip-cert-verify".into(), true.into());
    }
    let alpn = s(raw, "alpn").unwrap_or_else(|| "h3".to_string());
    let arr: Vec<Value> = alpn.split(',').map(|s| Value::from(s.trim().to_string())).collect();
    m.insert("alpn".into(), Value::Sequence(arr));
    Ok(m)
}

fn build_tuic_proxy(entry: &ProxyEntry) -> Result<Mapping> {
    let raw = &entry.raw;
    let mut m = base_proxy(entry, "tuic");

    if let Some(uuid) = s(raw, "uuid") {
        if !uuid.is_empty() {
            m.insert("uuid".into(), uuid.into());
        }
    }
    if let Some(password) = s(raw, "password") {
        m.insert("password".into(), password.into());
    }
    if let Some(sni) = s(raw, "sni") {
        if !sni.is_empty() {
            m.insert("sni".into(), sni.into());
        }
    }
    let alpn = s(raw, "alpn").unwrap_or_else(|| "h3".to_string());
    let arr: Vec<Value> = alpn.split(',').map(|s| Value::from(s.trim().to_string())).collect();
    m.insert("alpn".into(), Value::Sequence(arr));

    if let Some(cc) = s(raw, "congestion_control").or_else(|| s(raw, "congestion-controller")) {
        m.insert("congestion-controller".into(), cc.into());
    } else {
        m.insert("congestion-controller".into(), "bbr".into());
    }
    if let Some(udp_mode) = s(raw, "udp_relay_mode").or_else(|| s(raw, "udp-relay-mode")) {
        m.insert("udp-relay-mode".into(), udp_mode.into());
    } else {
        m.insert("udp-relay-mode".into(), "native".into());
    }
    if b(raw, "disable_sni").unwrap_or(false) {
        m.insert("disable-sni".into(), true.into());
    }
    if b(raw, "allow_insecure").or_else(|| b(raw, "insecure")).unwrap_or(false) {
        m.insert("skip-cert-verify".into(), true.into());
    }
    m.insert("reduce-rtt".into(), true.into());
    m.insert("fast-open".into(), true.into());
    Ok(m)
}

fn build_wireguard_proxy(entry: &ProxyEntry) -> Result<Mapping> {
    let raw = &entry.raw;
    let mut m = base_proxy(entry, "wireguard");

    let priv_key = s(raw, "private-key")
        .or_else(|| s(raw, "privatekey"))
        .context("wireguard: private-key обязателен")?;
    m.insert("private-key".into(), priv_key.into());

    if let Some(pubk) = s(raw, "publickey").or_else(|| s(raw, "public-key")) {
        m.insert("public-key".into(), pubk.into());
    }

    // Адрес интерфейса. URI хранит как "10.0.0.2/32" в "address".
    if let Some(addr) = s(raw, "address").or_else(|| s(raw, "ip")) {
        let ip_only = addr.split('/').next().unwrap_or(&addr).to_string();
        m.insert("ip".into(), ip_only.into());
    }
    if let Some(psk) = s(raw, "presharedkey").or_else(|| s(raw, "preshared-key")) {
        if !psk.is_empty() {
            m.insert("preshared-key".into(), psk.into());
        }
    }
    if let Some(mtu) = raw.get("mtu").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))) {
        m.insert("mtu".into(), mtu.into());
    }
    if let Some(reserved) = s(raw, "reserved") {
        // "0,0,0" → [0,0,0]
        let nums: Vec<Value> = reserved
            .split(',')
            .filter_map(|s| s.trim().parse::<u64>().ok())
            .map(Value::from)
            .collect();
        if !nums.is_empty() {
            m.insert("reserved".into(), Value::Sequence(nums));
        }
    }
    m.insert("remote-dns-resolve".into(), true.into());
    Ok(m)
}

fn build_socks_proxy(entry: &ProxyEntry) -> Result<Mapping> {
    let raw = &entry.raw;
    let mut m = base_proxy(entry, "socks5");
    if let Some(user) = s(raw, "username") {
        m.insert("username".into(), user.into());
    }
    if let Some(pass) = s(raw, "password") {
        m.insert("password".into(), pass.into());
    }
    Ok(m)
}

// ─── 8.F: passthrough full mihomo YAML ───────────────────────────────────────

/// Параметры patch'а — наши значения которые обязательно должны
/// попасть в финальный YAML, даже если у провайдера в подписке стоят
/// другие.
pub struct FullYamlPatch<'a> {
    /// Порт `mixed-port` (SOCKS5 + HTTP в одном). Перезаписываем
    /// провайдерский — нам нужен наш random port для рандомизации
    /// (9.H) и чтобы tun2socks-pipeline знал точный адрес.
    pub mixed_port: u16,
    /// `127.0.0.1` или `0.0.0.0` (LAN). Из настроек.
    pub listen: &'a str,
    /// SOCKS5 password-auth для inbound (9.G). Перезаписывает
    /// провайдерскую `authentication`. None в proxy-режиме без auth.
    pub socks_auth: Option<(&'a str, &'a str)>,
    /// Порт для external-controller (mihomo HTTP API). Используется
    /// для bandwidth-метра, smart failover, group-switching через
    /// `mihomo_api`. Random secret генерится автоматически.
    pub external_controller_port: u16,
    pub external_controller_secret: &'a str,
    /// Per-process правила пользователя (8.D). Добавляются ПЕРЕД
    /// провайдерскими — наш override приоритетнее.
    pub app_rules: &'a [AppRule],
    /// Anti-DPI: пока full-passthrough игнорирует (mihomo-only — DoH
    /// resolve можно поднять патчем `dns.nameserver` если нужен,
    /// но в подписке-профиле обычно DNS уже сконфигурен. Если нужно
    /// принудительно — пользователь явно включит в Settings).
    pub anti_dpi: Option<&'a AntiDpiOptions>,
    /// 0.1.2 / 13.L: использовать встроенный TUN-режим mihomo вместо
    /// нашего pipeline `external tun2socks via helper`.
    ///
    /// Когда `true`:
    /// - Сохраняем `tun.enable: true` из YAML (НЕ переписываем в false);
    /// - Mihomo сам создаёт WinTUN-адаптер, ставит routing-таблицу,
    ///   биндит DIRECT-outbound к физ-интерфейсу, обходит сам себя
    ///   на уровне sockopt (никаких петель — mihomo знает свой TUN);
    /// - Tauri-main НЕ дёргает helper.tun_start — mihomo всё делает сам.
    ///
    /// Когда `false` (default — proxy-режим или fallback):
    /// - Переписываем `tun.enable: false`, mihomo работает как обычный
    ///   SOCKS5/HTTP-сервер на mixed-port;
    /// - В TUN-режиме helper поднимает наш tun2socks и направляет
    ///   трафик в mihomo SOCKS5 (старый путь).
    ///
    /// **Требование**: mihomo должен быть запущен с правами админа
    /// (создание WinTUN-адаптера). Для этого запускаем mihomo через
    /// helper-сервис (он SYSTEM), а не напрямую как Tauri sidecar.
    pub use_builtin_tun: bool,
}

/// 8.F: применяет patch к полному mihomo-YAML из подписки и возвращает
/// готовый текст для запуска. Стратегия — **сохранить максимум** того
/// что прислал провайдер, перезаписав только то, что нам критично:
///
/// - `mixed-port` / `bind-address` — наш inbound порт (9.H рандомизация);
/// - `socks-port` / `port` / `redir-port` — удаляем (используем только
///   mixed-port, чтобы не разводить лишние порты);
/// - `authentication` — наша SOCKS-auth (9.G), перезаписываем;
/// - `external-controller` + `secret` — наши, иначе UI не достучится;
/// - `tun.enable` → `false` — наш helper управляет WinTUN через
///   tun2socks; mihomo built-in TUN был бы конфликтом (отложено в 13.L);
/// - `log-level` → `info` если был `silent` (нам нужны логи);
/// - `app_rules` пользователя (8.D) добавляются в начало `rules` блока.
///
/// **Сохраняется** провайдерское: `proxies`, `proxy-groups`,
/// `proxy-providers`, `rule-providers`, `dns`, `hosts`, `rules`,
/// `tun.exclude-address`, `tun.stack`, `tun.auto-route`,
/// `nameserver-policy`, `fake-ip-filter` и т.д.
pub fn patch_full_yaml(raw_yaml: &str, patch: &FullYamlPatch) -> Result<MihomoConfig> {
    let mut value: Value = serde_yaml::from_str(raw_yaml)
        .context("не удалось распарсить full mihomo YAML")?;
    let root = value
        .as_mapping_mut()
        .context("YAML root — не mapping")?;

    // ── inbound: единственный mixed-port на нашем порту ───────────────
    root.insert(
        "mixed-port".into(),
        (patch.mixed_port as u64).into(),
    );
    // Удаляем дублирующие порты — mixed-port покрывает SOCKS5 и HTTP
    root.remove(&Value::String("socks-port".into()));
    root.remove(&Value::String("port".into()));
    root.remove(&Value::String("redir-port".into()));
    root.remove(&Value::String("tproxy-port".into()));

    root.insert("allow-lan".into(), (patch.listen == "0.0.0.0").into());
    root.insert(
        "bind-address".into(),
        Value::String(patch.listen.to_string()),
    );

    // ── SOCKS-auth (9.G) ──────────────────────────────────────────────
    if let Some((user, pass)) = patch.socks_auth {
        root.insert(
            "authentication".into(),
            Value::Sequence(vec![Value::String(format!("{user}:{pass}"))]),
        );
        // skip-auth-prefixes для loopback можно оставить если был —
        // но обычно в подписочных конфигах его нет.
    } else {
        // Удаляем чужую auth — мы хотим контролировать кто подключается
        // к нашему inbound. Если auth не задан, оставляем noauth (только
        // loopback по умолчанию).
        root.remove(&Value::String("authentication".into()));
    }

    // ── external-controller (для mihomo_api) ──────────────────────────
    root.insert(
        "external-controller".into(),
        Value::String(format!("127.0.0.1:{}", patch.external_controller_port)),
    );
    root.insert(
        "secret".into(),
        Value::String(patch.external_controller_secret.to_string()),
    );

    // ── log-level ─────────────────────────────────────────────────────
    // Стратегия:
    //   - `silent` / отсутствует → форсим `warning` (по умолчанию мы
    //     режем шум, но оставляем error/warning видимыми);
    //   - любое явное значение (`info` / `debug`) — оставляем как есть.
    //     Если провайдер прислал `info`, значит ему нужны verbose-логи
    //     для диагностики (rule-decisions, provider-loads, и т.п.);
    //     перезаписывать его выбор плохая идея — мы тогда теряем
    //     причину когда mihomo внезапно умирает на init-фазе.
    //
    // На прод-релизе можно подумать про toggle «debug logs» в Settings,
    // но сейчас пользователь сам решает уровень через YAML.
    let current_log = root
        .get("log-level")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if current_log.is_empty() || current_log == "silent" {
        root.insert("log-level".into(), "warning".into());
    }

    // ── tun.enable: зависит от режима ─────────────────────────────────
    // **builtin-TUN путь (13.L)**: оставляем `tun.enable: true` — mihomo
    // сам создаст WinTUN, поставит маршруты, обработает DIRECT через
    // auto-detect-interface. Никакого нашего tun2socks/half-route'а.
    //
    // **внешний tun2socks путь** (default): принудительно
    // `tun.enable: false` — mihomo работает только как SOCKS-server,
    // тоннель управляется нашим helper'ом.
    if let Some(tun) = root
        .get_mut(&Value::String("tun".into()))
        .and_then(|v| v.as_mapping_mut())
    {
        if patch.use_builtin_tun {
            tun.insert("enable".into(), true.into());
            // auto-detect-interface жизненно важен — он скажет mihomo
            // какой физ-интерфейс использовать для bypass'а собственного
            // TUN'а в DIRECT-outbound. Без него mihomo не знает куда
            // привязать direct-сокет, петля.
            tun.insert("auto-detect-interface".into(), true.into());
            // auto-route: пусть mihomo сам ставит half-routes/0.0.0.0
            tun.entry(Value::String("auto-route".into()))
                .or_insert_with(|| true.into());
        } else {
            tun.insert("enable".into(), false.into());
        }
    } else if patch.use_builtin_tun {
        // Подписка не имеет `tun:` секции — собираем минимально-
        // рабочую с нашими дефолтами.
        let mut tun_map = Mapping::new();
        tun_map.insert("enable".into(), true.into());
        tun_map.insert("stack".into(), "mixed".into());
        tun_map.insert("auto-route".into(), true.into());
        tun_map.insert("auto-detect-interface".into(), true.into());
        root.insert("tun".into(), Value::Mapping(tun_map));
    }

    // ── ipv6 — оставляем как у провайдера ─────────────────────────────
    // (он сам решает; обычно false для mihomo-профилей)

    // ── find-process-mode для app-rules (8.D) ─────────────────────────
    if !patch.app_rules.is_empty() {
        root.insert("find-process-mode".into(), "always".into());
    }

    // ── Префиксные правила (наш приоритет) ────────────────────────────
    let mut prefix_rules: Vec<Value> = Vec::new();
    for r in patch.app_rules {
        if r.exe.trim().is_empty() {
            continue;
        }
        let target = match r.action.as_str() {
            "direct" => "DIRECT",
            "block" => "REJECT",
            _ => "PROXY",
        };
        prefix_rules.push(Value::String(format!(
            "PROCESS-NAME,{},{}",
            r.exe.trim(),
            target
        )));
    }

    // Anti-DPI server-resolve через DoH: если включён, подставляем в
    // dns.nameserver чтобы мiomo резолвил VPN-сервера через DoH.
    if let Some(anti) = patch.anti_dpi {
        if anti.server_resolve && !anti.server_resolve_doh.is_empty() {
            let dns = root
                .entry(Value::String("dns".into()))
                .or_insert_with(|| Value::Mapping(Mapping::new()));
            if let Some(dns_map) = dns.as_mapping_mut() {
                dns_map.insert("enable".into(), true.into());
                dns_map.insert(
                    "nameserver".into(),
                    Value::Sequence(vec![Value::String(
                        anti.server_resolve_doh.clone(),
                    )]),
                );
                if !anti.server_resolve_bootstrap.is_empty() {
                    dns_map.insert(
                        "default-nameserver".into(),
                        Value::Sequence(vec![Value::String(
                            anti.server_resolve_bootstrap.clone(),
                        )]),
                    );
                }
            }
        }
    }

    // ── Префиксные правила в начало rules ─────────────────────────────
    if !prefix_rules.is_empty() {
        let rules_entry = root
            .entry(Value::String("rules".into()))
            .or_insert_with(|| Value::Sequence(Vec::new()));
        if let Some(seq) = rules_entry.as_sequence_mut() {
            // Вставляем наши rules перед существующими — сохраняя порядок
            let mut combined = prefix_rules;
            combined.extend(seq.drain(..));
            *seq = combined;
        }
    }

    // mode=rule по умолчанию (если провайдер забыл) — иначе mihomo
    // войдёт в global mode (всё через PROXY) и наши rules не сработают.
    if !root.contains_key("mode") {
        root.insert("mode".into(), "rule".into());
    }

    let yaml = serde_yaml::to_string(&value)
        .context("сериализация патченного YAML")?;
    Ok(MihomoConfig {
        yaml,
        mixed_port: patch.mixed_port,
    })
}

#[cfg(test)]
mod patch_tests {
    use super::*;

    fn base_patch<'a>() -> FullYamlPatch<'a> {
        FullYamlPatch {
            mixed_port: 31000,
            listen: "127.0.0.1",
            socks_auth: Some(("nemefisto", "secret-pass")),
            external_controller_port: 31001,
            external_controller_secret: "test-secret-uuid",
            app_rules: &[],
            anti_dpi: None,
            use_builtin_tun: false,
        }
    }

    /// 8.F: provider's mixed-port должен быть перезаписан нашим. Также
    /// удаляем дублирующие SOCKS-port/redir-port чтобы mihomo не
    /// поднимал лишние inbound'ы.
    #[test]
    fn patch_overrides_inbound_ports() {
        let yaml = r#"
mixed-port: 7890
socks-port: 7891
redir-port: 7892
proxies: []
proxy-groups:
  - name: select
    type: select
    proxies: []
"#;
        let cfg = patch_full_yaml(yaml, &base_patch()).expect("patch ok");
        let v: Value = serde_yaml::from_str(&cfg.yaml).unwrap();
        let m = v.as_mapping().unwrap();
        assert_eq!(m["mixed-port"].as_u64(), Some(31000));
        assert!(!m.contains_key("socks-port"), "socks-port должен быть удалён");
        assert!(!m.contains_key("redir-port"), "redir-port должен быть удалён");
    }

    /// 8.F: tun.enable → false; остальные поля tun секции (stack,
    /// exclude-address) сохраняются для будущего 13.L.
    #[test]
    fn patch_disables_tun_keeping_other_fields() {
        let yaml = r#"
tun:
  enable: true
  stack: mixed
  auto-route: true
  exclude-address:
    - 1.2.3.4/32
proxies: []
proxy-groups:
  - name: select
    type: select
    proxies: []
"#;
        let cfg = patch_full_yaml(yaml, &base_patch()).expect("patch ok");
        let v: Value = serde_yaml::from_str(&cfg.yaml).unwrap();
        let tun = v.as_mapping().unwrap()["tun"].as_mapping().unwrap();
        assert_eq!(tun["enable"].as_bool(), Some(false));
        assert_eq!(tun["stack"].as_str(), Some("mixed"));
        assert!(tun.contains_key("exclude-address"));
    }

    /// 8.F: app_rules (PROCESS-NAME) попадают в начало rules списка —
    /// перед провайдерскими. Также set find-process-mode=always.
    #[test]
    fn patch_prepends_app_rules() {
        let yaml = r#"
proxies: []
proxy-groups:
  - name: select
    type: select
    proxies: []
rules:
  - DOMAIN-SUFFIX,example.com,DIRECT
  - MATCH,select
"#;
        let mut p = base_patch();
        let rules_owned = vec![AppRule {
            exe: "telegram.exe".into(),
            action: "proxy".into(),
            comment: None,
        }];
        p.app_rules = &rules_owned;
        let cfg = patch_full_yaml(yaml, &p).expect("patch ok");
        let v: Value = serde_yaml::from_str(&cfg.yaml).unwrap();
        let root = v.as_mapping().unwrap();
        assert_eq!(root["find-process-mode"].as_str(), Some("always"));
        let rules = root["rules"].as_sequence().unwrap();
        assert!(
            rules[0]
                .as_str()
                .unwrap()
                .starts_with("PROCESS-NAME,telegram.exe"),
            "первая rule должна быть наша app-rule"
        );
        assert_eq!(rules.last().unwrap().as_str(), Some("MATCH,select"));
    }

    /// 8.F: external-controller перезаписывается нашим (для mihomo_api).
    /// Чужой secret игнорируется.
    #[test]
    fn patch_sets_external_controller() {
        let yaml = r#"
external-controller: 127.0.0.1:9999
secret: provider-secret
proxies: []
proxy-groups:
  - name: select
    type: select
    proxies: []
"#;
        let cfg = patch_full_yaml(yaml, &base_patch()).expect("patch ok");
        let v: Value = serde_yaml::from_str(&cfg.yaml).unwrap();
        let m = v.as_mapping().unwrap();
        assert_eq!(
            m["external-controller"].as_str(),
            Some("127.0.0.1:31001")
        );
        assert_eq!(m["secret"].as_str(), Some("test-secret-uuid"));
    }

    /// 8.F: provider's authentication перезаписывается нашим SOCKS-auth.
    #[test]
    fn patch_overrides_authentication() {
        let yaml = r#"
authentication:
  - bad-user:bad-pass
  - other-user:other-pass
proxies: []
proxy-groups:
  - name: select
    type: select
    proxies: []
"#;
        let cfg = patch_full_yaml(yaml, &base_patch()).expect("patch ok");
        let v: Value = serde_yaml::from_str(&cfg.yaml).unwrap();
        let auth = v.as_mapping().unwrap()["authentication"]
            .as_sequence()
            .unwrap();
        assert_eq!(auth.len(), 1);
        assert_eq!(auth[0].as_str(), Some("nemefisto:secret-pass"));
    }

    /// 0.1.2: реальная подписка пользователя с load-balance подгруппой,
    /// select-обёрткой, rule-providers и сложными OR-правилами + emoji
    /// в имени группы. Регрессия — раньше тестов с такой структурой
    /// не было, и можно было поломать парсинг при правке patch_full_yaml.
    #[test]
    fn patches_complex_real_world_subscription() {
        let yaml = r#"
mixed-port: 7890
mode: rule
log-level: info
tun:
  enable: true
  stack: mixed
  auto-route: true
dns:
  enable: true
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  nameserver:
    - 1.1.1.1
proxies:
  - {name: 'germany', type: vless, server: de.x.com, port: 443}
  - {name: 'latvia',  type: vless, server: lv.x.com, port: 443}
proxy-groups:
  - name: "🇪🇺  Fastest"
    type: load-balance
    url: https://cp.cloudflare.com/generate_204
    interval: 600
    strategy: consistent-hashing
    proxies:
      - germany
      - latvia
  - name: 'ariyvpn'
    type: 'select'
    proxies:
      - "🇪🇺  Fastest"
rule-providers:
  geosite-ru:
    type: http
    behavior: domain
    format: mrs
    url: https://github.com/MetaCubeX/meta-rules-dat/raw/meta/geo/geosite/category-ru.mrs
    path: ./geosite-ru.mrs
    interval: 86400
  geoip-ru:
    type: http
    behavior: ipcidr
    format: mrs
    url: https://github.com/MetaCubeX/meta-rules-dat/raw/meta/geo/geoip/ru.mrs
    path: ./geoip-ru.mrs
    interval: 86400
rules:
  - IP-CIDR,3.68.63.139/32,DIRECT,no-resolve
  - PROCESS-NAME,FortiClient.exe,DIRECT
  - DOMAIN-SUFFIX,sportlevel.com,DIRECT
  - OR,((RULE-SET,geosite-ru),(RULE-SET,geoip-ru)),DIRECT
  - MATCH,ariyvpn
"#;
        let cfg = patch_full_yaml(yaml, &base_patch())
            .expect("реальная подписка должна патчиться");
        // log-level: info в подписке — сохраняется как есть. Только
        // silent/missing мы форсим в warning.
        assert!(
            cfg.yaml.contains("log-level: info"),
            "log-level=info из подписки должен сохраниться"
        );
        // tun.enable должно быть false — наш tun2socks pipeline активный
        assert!(cfg.yaml.contains("enable: false"));
        // rule-providers должны сохраниться целиком
        assert!(cfg.yaml.contains("geosite-ru"));
        assert!(cfg.yaml.contains("geoip-ru"));
        assert!(cfg.yaml.contains("category-ru.mrs"));
        // rules в исходном порядке (мы только префиксы добавляем)
        let pos_iprule = cfg.yaml.find("3.68.63.139").expect("ip-cidr rule");
        let pos_match = cfg.yaml.find("MATCH,ariyvpn").expect("match rule");
        assert!(pos_iprule < pos_match, "порядок rules сохраняется");
        // proxy-group с emoji в имени — должна пройти YAML round-trip
        // без потери символов
        assert!(cfg.yaml.contains("🇪🇺"));
        // OR-rule с rule-set'ами должно сохраниться как строка
        assert!(cfg.yaml.contains("RULE-SET,geosite-ru"));
        // ariyvpn group должна сохраниться
        assert!(cfg.yaml.contains("ariyvpn"));
    }

    /// 13.L: `use_builtin_tun=true` сохраняет `tun.enable: true` из
    /// подписки и форсит `auto-detect-interface: true`. Это позволяет
    /// mihomo самостоятельно создать WinTUN, поставить маршруты, и
    /// корректно bypass'ить собственный TUN на DIRECT-outbound'е.
    #[test]
    fn patch_keeps_tun_enabled_for_builtin_tun_mode() {
        let yaml = r#"
proxies: []
proxy-groups:
  - name: select
    type: select
    proxies: []
tun:
  enable: true
  stack: mixed
  auto-route: true
"#;
        let mut p = base_patch();
        p.use_builtin_tun = true;
        let cfg = patch_full_yaml(yaml, &p).expect("patch ok");
        let v: Value = serde_yaml::from_str(&cfg.yaml).unwrap();
        let tun = v.as_mapping().unwrap()["tun"].as_mapping().unwrap();
        assert_eq!(tun["enable"], Value::Bool(true), "tun.enable=true сохранён");
        assert_eq!(
            tun["auto-detect-interface"],
            Value::Bool(true),
            "auto-detect-interface форсирован для bypass-а DIRECT"
        );
    }

    /// 13.L: `use_builtin_tun=true` для подписки БЕЗ tun-секции —
    /// собираем минимальную с разумными дефолтами. mihomo не должен
    /// упасть на отсутствии `tun:` блока когда мы хотим built-in.
    #[test]
    fn patch_synthesizes_tun_for_builtin_when_missing() {
        let yaml = r#"
proxies: []
proxy-groups:
  - name: select
    type: select
    proxies: []
"#;
        let mut p = base_patch();
        p.use_builtin_tun = true;
        let cfg = patch_full_yaml(yaml, &p).expect("patch ok");
        let v: Value = serde_yaml::from_str(&cfg.yaml).unwrap();
        let tun = v.as_mapping().unwrap()["tun"].as_mapping().unwrap();
        assert_eq!(tun["enable"], Value::Bool(true));
        assert_eq!(tun["auto-detect-interface"], Value::Bool(true));
        assert_eq!(tun["auto-route"], Value::Bool(true));
    }
}
