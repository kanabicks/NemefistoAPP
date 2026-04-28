//! Генерация JSON-конфига Xray из ProxyEntry.
//!
//! Поддерживаемые протоколы: VLESS (REALITY / TLS), VMess, Trojan, Shadowsocks.
//! Inbound: SOCKS5 + HTTP proxy, оба на 127.0.0.1.
//! Routing: приватные адреса → direct, всё остальное → proxy.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::server::ProxyEntry;

/// Результат генерации: готовый JSON + порты, на которых будет слушать Xray.
pub struct XrayConfig {
    pub json: Value,
    pub socks_port: u16,
    pub http_port: u16,
}

/// Построить конфиг Xray для заданного сервера и портов.
pub fn build(entry: &ProxyEntry, socks_port: u16, http_port: u16) -> Result<XrayConfig> {
    let outbound = build_outbound(entry)
        .with_context(|| format!("ошибка построения outbound для «{}»", entry.name))?;

    let config = json!({
        "log": {
            "loglevel": "warning"
        },
        "dns": {
            "servers": [
                "https+local://1.1.1.1/dns-query",
                "localhost"
            ]
        },
        "inbounds": [
            {
                "tag": "socks-in",
                "listen": "127.0.0.1",
                "port": socks_port,
                "protocol": "socks",
                "settings": {
                    "auth": "noauth",
                    "udp": true
                },
                "sniffing": {
                    "enabled": true,
                    "destOverride": ["http", "tls"]
                }
            },
            {
                "tag": "http-in",
                "listen": "127.0.0.1",
                "port": http_port,
                "protocol": "http",
                "settings": {},
                "sniffing": {
                    "enabled": true,
                    "destOverride": ["http", "tls"]
                }
            }
        ],
        "outbounds": [
            outbound,
            {
                "tag": "direct",
                "protocol": "freedom",
                "settings": {}
            },
            {
                "tag": "block",
                "protocol": "blackhole",
                "settings": {}
            }
        ],
        "routing": {
            "domainStrategy": "IPIfNonMatch",
            "rules": [
                {
                    "type": "field",
                    "ip": [
                        "127.0.0.0/8",
                        "10.0.0.0/8",
                        "172.16.0.0/12",
                        "192.168.0.0/16",
                        "169.254.0.0/16",
                        "::1/128",
                        "fc00::/7"
                    ],
                    "outboundTag": "direct"
                }
            ]
        }
    });

    Ok(XrayConfig { json: config, socks_port, http_port })
}

// ─── Приватные CIDR (замена geoip:private) ────────────────────────────────────

const PRIVATE_CIDR: &[&str] = &[
    "127.0.0.0/8",
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "169.254.0.0/16",
    "::1/128",
    "fc00::/7",
];

/// Подготовить внешний Xray JSON конфиг к запуску на Windows без dat-файлов:
/// - заменяет порты inbounds на socks_port / http_port
/// - убирает Linux-специфичные sockopt (tcpcongestion/bbr, tcpUserTimeout)
/// - заменяет `geoip:private` на список CIDR
/// - удаляет `geosite:private` из правил
///
/// burstObservatory и leastLoad оставляются как есть — они работают на Windows.
///
/// Возвращает пропатченный конфиг.
pub fn patch_xray_json(mut config: Value, socks_port: u16, http_port: u16) -> Value {
    // Обновляем порты inbounds
    if let Some(arr) = config["inbounds"].as_array_mut() {
        for ib in arr.iter_mut() {
            let proto = ib["protocol"].as_str().unwrap_or("").to_string();
            match proto.as_str() {
                "socks" => { ib["port"] = json!(socks_port); }
                "http"  => { ib["port"] = json!(http_port); }
                _ => {}
            }
        }
    }

    // Убираем весь sockopt из outbounds: tcpFastOpen требует прав администратора,
    // tcpcongestion/bbr и tcpKeepAliveIdle — Linux-only. На Windows всё это не нужно.
    if let Some(outbounds) = config["outbounds"].as_array_mut() {
        for ob in outbounds.iter_mut() {
            if let Some(stream) = ob["streamSettings"].as_object_mut() {
                stream.remove("sockopt");
            }
        }
    }

    // Патчим routing.rules
    if let Some(rules) = config["routing"]["rules"].as_array_mut() {
        let mut keep = Vec::with_capacity(rules.len());
        for rule in rules.drain(..) {
            if let Some(patched) = patch_rule(rule) {
                keep.push(patched);
            }
        }
        *rules = keep;
    }

    config
}

/// Патчит одно routing-правило. Возвращает None если правило стало пустым.
fn patch_rule(mut rule: Value) -> Option<Value> {
    // ip: заменяем "geoip:private" на CIDR
    if let Some(ip_arr) = rule["ip"].as_array_mut() {
        let had_private = ip_arr.iter().any(|v| v.as_str() == Some("geoip:private"));
        ip_arr.retain(|v| v.as_str() != Some("geoip:private"));
        if had_private {
            for cidr in PRIVATE_CIDR {
                ip_arr.push(json!(cidr));
            }
        }
    }

    // domain: удаляем "geosite:private"
    if let Some(dom_arr) = rule["domain"].as_array_mut() {
        dom_arr.retain(|v| v.as_str() != Some("geosite:private"));
    }

    // Marzban сериализует отсутствующие matcher-поля как null/[]. Xray ругается
    // "rule has no effective fields" если правило содержит только null-поля.
    // Удаляем все null/пустые matcher-поля до проверки has_match.
    let obj = rule.as_object_mut()?;
    let match_keys = [
        "ip", "domain", "network", "port", "balancerTag", "source", "protocol", "user", "inboundTag",
    ];
    for k in &match_keys {
        let drop = match obj.get(*k) {
            Some(Value::Null) => true,
            Some(Value::Array(a)) => a.is_empty(),
            Some(Value::String(s)) => s.is_empty(),
            _ => false,
        };
        if drop {
            obj.remove(*k);
        }
    }

    // Правило валидно если осталось хотя бы одно поле для матчинга
    let has_match = match_keys.iter().any(|k| obj.contains_key(*k));
    if has_match { Some(rule) } else { None }
}

// ─── диспетчер протоколов ─────────────────────────────────────────────────────

fn build_outbound(entry: &ProxyEntry) -> Result<Value> {
    match entry.protocol.as_str() {
        "vless" => build_vless(entry),
        "vmess" => build_vmess(entry),
        "trojan" => build_trojan(entry),
        "ss" | "shadowsocks" => build_ss(entry),
        "xray-json" => bail!("протокол xray-json используется as-is, без генерации outbound"),
        p => bail!("неподдерживаемый протокол Xray: {p}"),
    }
}

// ─── VLESS ────────────────────────────────────────────────────────────────────

fn build_vless(entry: &ProxyEntry) -> Result<Value> {
    let raw = &entry.raw;
    let uuid = raw["uuid"].as_str().context("uuid обязателен для VLESS")?;
    let flow = raw["flow"].as_str().unwrap_or("");
    let security = raw["security"].as_str().unwrap_or("none");
    let transport = raw["type"].as_str().unwrap_or("tcp");

    let stream = build_stream(transport, security, raw)?;

    Ok(json!({
        "tag": "proxy",
        "protocol": "vless",
        "settings": {
            "vnext": [{
                "address": entry.server,
                "port": entry.port,
                "users": [{
                    "id": uuid,
                    "encryption": "none",
                    "flow": flow
                }]
            }]
        },
        "streamSettings": stream
    }))
}

// ─── VMess ────────────────────────────────────────────────────────────────────

fn build_vmess(entry: &ProxyEntry) -> Result<Value> {
    let raw = &entry.raw;
    let uuid = raw["id"].as_str().context("id обязателен для VMess")?;
    let aid = raw["aid"]
        .as_u64()
        .or_else(|| raw["aid"].as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0);
    let cipher = raw["scy"].as_str().unwrap_or("auto");

    let network = raw["net"].as_str().unwrap_or("tcp");
    let tls_val = raw["tls"].as_str().unwrap_or("");
    let security = if tls_val == "tls" { "tls" } else { "none" };

    let stream = build_stream(network, security, raw)?;

    Ok(json!({
        "tag": "proxy",
        "protocol": "vmess",
        "settings": {
            "vnext": [{
                "address": entry.server,
                "port": entry.port,
                "users": [{
                    "id": uuid,
                    "alterId": aid,
                    "security": cipher
                }]
            }]
        },
        "streamSettings": stream
    }))
}

// ─── Trojan ───────────────────────────────────────────────────────────────────

fn build_trojan(entry: &ProxyEntry) -> Result<Value> {
    let raw = &entry.raw;
    let password = raw["password"].as_str().context("password обязателен для Trojan")?;
    let security = raw["security"].as_str().unwrap_or("tls");
    let transport = raw["type"].as_str().unwrap_or("tcp");

    let stream = build_stream(transport, security, raw)?;

    Ok(json!({
        "tag": "proxy",
        "protocol": "trojan",
        "settings": {
            "servers": [{
                "address": entry.server,
                "port": entry.port,
                "password": password
            }]
        },
        "streamSettings": stream
    }))
}

// ─── Shadowsocks ──────────────────────────────────────────────────────────────

fn build_ss(entry: &ProxyEntry) -> Result<Value> {
    let raw = &entry.raw;
    let method = raw["cipher"]
        .as_str()
        .or_else(|| raw["method"].as_str())
        .context("cipher/method обязателен для Shadowsocks")?;
    let password = raw["password"]
        .as_str()
        .context("password обязателен для Shadowsocks")?;

    Ok(json!({
        "tag": "proxy",
        "protocol": "shadowsocks",
        "settings": {
            "servers": [{
                "address": entry.server,
                "port": entry.port,
                "method": method,
                "password": password
            }]
        }
    }))
}

// ─── streamSettings ───────────────────────────────────────────────────────────

fn build_stream(network: &str, security: &str, raw: &Value) -> Result<Value> {
    let mut s = json!({ "network": network });

    // Security layer
    match security {
        "reality" => {
            let sni = raw["sni"].as_str().unwrap_or("");
            let fp = raw["fp"].as_str().unwrap_or("chrome");
            let pbk = raw["pbk"].as_str().context("pbk обязателен для REALITY")?;
            let sid = raw["sid"].as_str().unwrap_or("");
            s["security"] = "reality".into();
            s["realitySettings"] = json!({
                "serverName": sni,
                "fingerprint": fp,
                "publicKey": pbk,
                "shortId": sid
            });
        }
        "tls" => {
            let sni = raw["sni"]
                .as_str()
                .or_else(|| raw["host"].as_str())
                .unwrap_or("");
            let fp = raw["fp"].as_str().unwrap_or("");
            let alpn = raw["alpn"].as_str().unwrap_or("");
            let mut tls = json!({
                "serverName": sni,
                "allowInsecure": false
            });
            if !fp.is_empty() {
                tls["fingerprint"] = fp.into();
            }
            if !alpn.is_empty() {
                tls["alpn"] = json!([alpn]);
            }
            s["security"] = "tls".into();
            s["tlsSettings"] = tls;
        }
        _ => {
            s["security"] = "none".into();
        }
    }

    // Transport layer
    match network {
        "ws" => {
            let path = raw["path"].as_str().unwrap_or("/");
            let host = raw["host"].as_str().unwrap_or("");
            let mut ws = json!({ "path": path });
            if !host.is_empty() {
                ws["headers"] = json!({ "Host": host });
            }
            s["wsSettings"] = ws;
        }
        "grpc" => {
            let svc = raw["serviceName"]
                .as_str()
                .or_else(|| raw["path"].as_str())
                .unwrap_or("");
            s["grpcSettings"] = json!({
                "serviceName": svc,
                "multiMode": false
            });
        }
        "h2" | "http" => {
            let path = raw["path"].as_str().unwrap_or("/");
            let host = raw["host"].as_str().unwrap_or("");
            s["httpSettings"] = json!({
                "path": path,
                "host": if host.is_empty() { json!([]) } else { json!([host]) }
            });
        }
        "tcp" => {
            // HTTP obfuscation поверх TCP (устаревший, но встречается)
            let header = raw["headerType"]
                .as_str()
                .or_else(|| {
                    // в VLESS raw["type"] = тип транспорта, а не заголовка —
                    // поэтому смотрим только headerType
                    None
                })
                .unwrap_or("none");
            if header == "http" {
                let path = raw["path"].as_str().unwrap_or("/");
                let host = raw["host"].as_str().unwrap_or("");
                s["tcpSettings"] = json!({
                    "header": {
                        "type": "http",
                        "request": {
                            "path": [path],
                            "headers": {
                                "Host": if host.is_empty() { json!([]) } else { json!([host]) }
                            }
                        }
                    }
                });
            }
        }
        _ => {}
    }

    Ok(s)
}
