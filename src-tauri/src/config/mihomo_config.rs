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
use super::xray_config::AntiDpiOptions;

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

    // 8.D: per-process правила (если заданы) идут перед `MATCH,PROXY`,
    // чтобы перехватить трафик конкретных процессов до общего fallback'a.
    // Action нормализуется: proxy→PROXY, direct→DIRECT, block→REJECT.
    let mut rules: Vec<Value> = Vec::new();
    for r in app_rules {
        if r.exe.trim().is_empty() {
            continue;
        }
        let target = match r.action.as_str() {
            "direct" => "DIRECT",
            "block" => "REJECT",
            _ => "PROXY", // дефолт + явный "proxy"
        };
        rules.push(Value::String(format!("PROCESS-NAME,{},{}", r.exe.trim(), target)));
    }
    // Routing-профили (этап 11) добавят geosite/geoip позже. Пока —
    // всё что не попало в PROCESS-NAME правила идёт через прокси.
    rules.push(Value::String("MATCH,PROXY".to_string()));
    root.insert("rules".into(), Value::Sequence(rules));

    let yaml = serde_yaml::to_string(&Value::Mapping(root))
        .context("сериализация mihomo YAML")?;

    Ok(MihomoConfig { yaml, mixed_port })
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
