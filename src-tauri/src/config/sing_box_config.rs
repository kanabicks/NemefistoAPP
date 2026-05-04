//! Генерация JSON-конфига sing-box из ProxyEntry.
//!
//! Поддерживаемые протоколы: VLESS (REALITY/Vision/TLS), VMess, Trojan,
//! Shadowsocks, Hysteria2, TUIC, WireGuard, SOCKS5.
//!
//! Inbound в proxy-режиме: `mixed` (SOCKS5+HTTP в одном порту).
//! Inbound в TUN-режиме: `tun` (auto_route, auto_detect_interface, gVisor stack)
//! плюс `mixed` для warmup/leak-test/per-app-routing-fallback.
//!
//! Routing: приватные адреса → direct, всё остальное → proxy. Финальное
//! правило `final: "proxy"` гарантирует что ничего не утечёт по дефолту
//! sing-box (по умолчанию `final` = direct).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::server::ProxyEntry;

/// Результат генерации: готовый JSON + порт mixed-inbound.
///
/// `socks_port` и `http_port` указывают на ОДИН и тот же порт mixed-inbound —
/// он принимает и SOCKS5 (greeting 0x05), и HTTP CONNECT/GET (любой ASCII-метод).
/// Дублируем поле для совместимости с существующим API (xray возвращал два
/// разных порта). После выпиливания xray можно слить в один `mixed_port`.
pub struct SingBoxConfig {
    pub json: Value,
    pub socks_port: u16,
    pub http_port: u16,
}

/// Anti-DPI обвязка sing-box (этап 10 переписанный под sing-box).
///
/// Поля те же что у Xray-варианта (`xray_config::AntiDpiOptions`), чтобы
/// существующие настройки/заголовки подписки не пришлось мигрировать.
/// Но семантика применяется через sing-box-нативные механизмы:
///
/// - Фрагментация → `outbound.tls_fragment` (sing-box 1.12+) либо
///   `tls.fragment` булевый флаг для legacy-режима.
/// - Шумы (UDP) → `outbound.udp_over_tcp` отключаем + добавляем
///   `udp_noises` через `dialer.detour` цепочку (sing-box 1.13+).
/// - Server-resolve через DoH → `dns.servers[].address` с явным
///   `address_resolver` для bootstrap'а.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntiDpiOptions {
    pub fragmentation: bool,
    pub fragmentation_packets: String,
    pub fragmentation_length: String,
    pub fragmentation_interval: String,
    pub noises: bool,
    pub noises_type: String,
    pub noises_packet: String,
    pub noises_delay: String,
    pub server_resolve: bool,
    pub server_resolve_doh: String,
    pub server_resolve_bootstrap: String,
}

/// Mux (multiplexing) для sing-box outbound (Happ-like feature).
///
/// Sing-box умеет мультиплексировать stream-based протоколы (vless/vmess/
/// trojan/ss/socks) одним TCP-соединением через `smux`/`yamux`/`h2mux`.
/// Это уменьшает overhead на TLS-handshake (одна TLS-сессия на много
/// логических потоков) и помогает обходить connection-rate-limit'ы.
///
/// Не применимо для:
/// - **hysteria2 / tuic** — у них свой stream multiplexing над QUIC;
/// - **wireguard** — UDP-only, не stream;
/// - **xray-json / singbox-json passthrough** — конфиги из подписки
///   мы не патчим, mux должен прийти от провайдера.
///
/// Серверная сторона должна тоже включить совместимый mux (3x-ui /
/// Marzban / x-ui это умеют). Если сервер не поддерживает mux, клиент
/// fall-back'нется на обычный mode (sing-box делает это автоматически).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MuxOptions {
    /// Включить мультиплексирование. Если `false`, остальные поля
    /// игнорируются и `multiplex` блок не добавляется в outbound.
    pub enabled: bool,
    /// Протокол мультиплексирования: `smux` (default, рекомендуется),
    /// `yamux` (более fault-tolerant) или `h2mux` (HTTP/2 фреймы).
    /// Пустая строка = `smux`.
    pub protocol: String,
    /// Максимальное число параллельных потоков на одно TCP-соединение.
    /// Sing-box default — 0 (unlimited). Рекомендуется 4-16. 0 = пусть
    /// решает sing-box.
    pub max_streams: u32,
}

/// Параметры built-in TUN inbound. Используется только когда `tun_mode=true`.
/// helper SYSTEM-spawn'ит sing-box и tun inbound создаёт WinTUN-адаптер.
#[derive(Debug, Clone)]
pub struct TunOptions {
    /// Имя WinTUN-адаптера. Дефолт `nemefisto-<pid>`. С 12.E может быть
    /// замаскирован под `wlan99` / `Local Area Connection N` / `Ethernet N`.
    pub interface_name: String,
    /// CIDR адрес TUN-интерфейса. Используем `198.18.0.1/15` (RFC 2544
    /// benchmark range — гарантированно не пересекается с домашними сетями).
    /// Совпадает с диапазоном который раньше использовал tun2proxy/mihomo.
    pub address: String,
    /// MTU. 9000 — стандартный default для TUN-стэка с поддержкой
    /// jumbo-frames в gVisor. WinTUN жёстко ограничивает до этого
    /// значения.
    pub mtu: u32,
}

impl Default for TunOptions {
    fn default() -> Self {
        Self {
            interface_name: format!("nemefisto-{}", std::process::id()),
            address: "198.18.0.1/15".to_string(),
            mtu: 9000,
        }
    }
}

/// Построить конфиг sing-box для заданного сервера.
///
/// `socks_port` и `http_port` — порты mixed-inbound. На практике передаём
/// одно и то же значение в оба (mixed слушает один порт).
///
/// `listen` — `"127.0.0.1"` для loopback или `"0.0.0.0"` для LAN-доступа.
///
/// `tun_mode`: если `true`, в `inbounds` добавляется `tun`-inbound (требует
/// SYSTEM-привилегий — должен запускаться через helper-сервис).
///
/// `tun_options` — параметры built-in TUN. Используется только при
/// `tun_mode=true`. Если `None` и `tun_mode=true` — берём дефолты.
///
/// `anti_dpi` — обвязка анти-DPI. Применяется к outbound (TLS-fragment,
/// noises) и DNS (DoH-resolve адреса VPN-сервера).
///
/// `socks_auth` — пара `(user, pass)` для mixed-inbound. `None` = без auth.
/// В TUN/LAN-режимах обязательно (защита из 9.G).
///
/// `mux` — параметры мультиплексирования (Mux feature). Применяется только
/// для stream-based протоколов (vless/vmess/trojan/ss/socks). Для
/// hysteria2/tuic/wireguard игнорируется (у них свой multiplexing).
pub fn build(
    entry: &ProxyEntry,
    socks_port: u16,
    http_port: u16,
    listen: &str,
    tun_mode: bool,
    tun_options: Option<&TunOptions>,
    anti_dpi: Option<&AntiDpiOptions>,
    socks_auth: Option<(&str, &str)>,
    mux: Option<&MuxOptions>,
) -> Result<SingBoxConfig> {
    let (mut proxy_outbound, mut endpoints) = build_outbound_or_endpoint(entry)
        .with_context(|| format!("ошибка построения outbound для «{}»", entry.name))?;

    apply_anti_dpi_to_outbound(&mut proxy_outbound, anti_dpi);
    apply_mux_to_outbound(&mut proxy_outbound, mux, &entry.protocol);

    // Стандартный набор: proxy + direct. Действия `block` и `dns` в
    // sing-box 1.11+ выполнены как rule actions ("reject" / "hijack-dns")
    // вместо отдельных outbound'ов — отдельные `block`/`dns` outbound'ы
    // объявлены deprecated.
    let mut outbounds: Vec<Value> = Vec::with_capacity(2);
    outbounds.push(proxy_outbound);
    outbounds.push(json!({ "type": "direct", "tag": "direct" }));

    let inbounds = build_inbounds(socks_port, listen, tun_mode, tun_options, socks_auth);
    let dns = build_dns(entry, anti_dpi);
    let route = build_route(tun_mode);

    let mut config = json!({
        "log": { "level": "warn", "timestamp": true },
        "dns": dns,
        "inbounds": inbounds,
        "outbounds": outbounds,
        "route": route,
    });

    // WireGuard в sing-box 1.11+ переехал из outbounds в endpoints
    // (top-level массив). Если был сгенерирован endpoint — вставляем его
    // и заменяем proxy-outbound на selector, ссылающийся на endpoint-tag.
    if !endpoints.is_empty() {
        // Заменяем placeholder-outbound на selector с tag="proxy", который
        // указывает на реальный wg endpoint (tag="proxy-wg").
        if let Some(ob_arr) = config["outbounds"].as_array_mut() {
            if let Some(idx) = ob_arr.iter().position(|o| o["tag"] == "proxy") {
                ob_arr[idx] = json!({
                    "type": "selector",
                    "tag": "proxy",
                    "outbounds": ["proxy-wg"],
                    "default": "proxy-wg"
                });
            }
        }
        config["endpoints"] = json!(endpoints.drain(..).collect::<Vec<_>>());
    }

    Ok(SingBoxConfig { json: config, socks_port, http_port })
}

// ─── inbounds ────────────────────────────────────────────────────────────────

fn build_inbounds(
    mixed_port: u16,
    listen: &str,
    tun_mode: bool,
    tun_options: Option<&TunOptions>,
    socks_auth: Option<(&str, &str)>,
) -> Vec<Value> {
    let mut inbounds: Vec<Value> = Vec::with_capacity(2);

    // mixed: SOCKS5 + HTTP в одном listener'е. sing-box определяет
    // протокол по первому байту (0x05 → SOCKS, любой ASCII-метод → HTTP).
    //
    // ВАЖНО: `sniff: true` на inbound удалён в sing-box 1.13.0 — теперь
    // это route rule action "sniff" (см. build_route).
    let mut mixed = json!({
        "type": "mixed",
        "tag": "mixed-in",
        "listen": listen,
        "listen_port": mixed_port,
    });
    if let Some((user, pass)) = socks_auth {
        mixed["users"] = json!([{ "username": user, "password": pass }]);
    }
    inbounds.push(mixed);

    // tun: built-in TUN inbound с auto_route. Спавнится из helper'а
    // (нужен SYSTEM для CreateAdapter WinTUN). Если tun_options не задан —
    // берём дефолты. tun.auto_redirect и auto_detect_interface остались
    // на inbound — это TUN-специфичные поля, не относящиеся к sniffing.
    if tun_mode {
        let opts = tun_options.cloned().unwrap_or_default();
        inbounds.push(json!({
            "type": "tun",
            "tag": "tun-in",
            "interface_name": opts.interface_name,
            "address": [opts.address],
            "mtu": opts.mtu,
            "auto_route": true,
            "strict_route": false,
            "stack": "gvisor",
        }));
    }

    inbounds
}

// ─── DNS ─────────────────────────────────────────────────────────────────────
//
// sing-box 1.12+ использует новый DNS-формат: вместо `address: "https://..."`
// нужен явный `type: "https" | "udp" | "local" | ...` + структурированные
// поля (`server`, `server_port`, `path`, `domain_resolver`). Legacy-формат
// deprecated в 1.12 и будет удалён в 1.14 — пишем сразу в новом.
// См. https://sing-box.sagernet.org/migration/#migrate-to-new-dns-server-formats

fn build_dns(entry: &ProxyEntry, anti_dpi: Option<&AntiDpiOptions>) -> Value {
    // 10.C: если включён DoH-resolve адреса VPN-сервера — выставляем
    // DoH-сервер с bootstrap'ом (UDP-DNS на bootstrap-IP, который резолвит
    // имя DoH-эндпоинта). Иначе — стандартный DoH через 1.1.1.1.
    if let Some(ad) = anti_dpi.filter(|a| a.server_resolve && !a.server_resolve_doh.is_empty()) {
        let (doh_host, doh_path) = parse_doh_url(&ad.server_resolve_doh);
        let bootstrap = if ad.server_resolve_bootstrap.is_empty() {
            "1.1.1.1".to_string()
        } else {
            ad.server_resolve_bootstrap.clone()
        };
        return json!({
            "servers": [
                {
                    "type": "https",
                    "tag": "doh",
                    "server": doh_host,
                    "server_port": 443,
                    "path": doh_path,
                    "domain_resolver": "bootstrap",
                    "detour": "direct",
                },
                {
                    "type": "udp",
                    "tag": "bootstrap",
                    "server": bootstrap,
                    "detour": "direct",
                },
            ],
            "rules": [
                { "domain": [entry.server.clone()], "server": "doh" }
            ],
            "final": "doh",
            "strategy": "ipv4_only",
        });
    }

    json!({
        "servers": [
            {
                "type": "https",
                "tag": "doh",
                "server": "1.1.1.1",
                "server_port": 443,
                "path": "/dns-query",
                "domain_resolver": "local",
                "detour": "proxy",
            },
            { "type": "local", "tag": "local" }
        ],
        "rules": [],
        "final": "doh",
        "strategy": "ipv4_only",
    })
}

/// Распарсить `https://cloudflare-dns.com/dns-query` → `("cloudflare-dns.com", "/dns-query")`.
/// Если URL некорректен — fallback на cloudflare.
fn parse_doh_url(url: &str) -> (String, String) {
    let after_proto = url.split("://").nth(1).unwrap_or("cloudflare-dns.com/dns-query");
    if let Some(slash) = after_proto.find('/') {
        let host = &after_proto[..slash];
        let path = &after_proto[slash..];
        (host.to_string(), path.to_string())
    } else {
        (after_proto.to_string(), "/dns-query".to_string())
    }
}

// ─── route ───────────────────────────────────────────────────────────────────

fn build_route(tun_mode: bool) -> Value {
    let mut rules: Vec<Value> = Vec::new();

    // sniff действие — в sing-box 1.11+ заменило inbound.sniff поля.
    // Извлекает SNI/Host/QUIC-SNI из первых пакетов соединения для
    // последующего matching по domain'у в правилах ниже.
    rules.push(json!({ "action": "sniff" }));

    // DNS-перехват: пакеты на :53 захватываются и возвращаются через
    // dns.* конфигурацию (новое action `hijack-dns`, заменило
    // legacy `outbound: "dns-out"`).
    rules.push(json!({
        "protocol": "dns",
        "action": "hijack-dns"
    }));

    // Приватные адреса (LAN, loopback, link-local) → direct. Без этого
    // в TUN-режиме все локальные обращения завернутся в туннель и
    // отвалятся.
    rules.push(json!({
        "ip_cidr": [
            "127.0.0.0/8",
            "10.0.0.0/8",
            "172.16.0.0/12",
            "192.168.0.0/16",
            "169.254.0.0/16",
            "::1/128",
            "fe80::/10",
            "fc00::/7"
        ],
        "action": "route",
        "outbound": "direct"
    }));

    // В TUN-режиме блокируем QUIC (UDP/443) — большинство анти-DPI настроек
    // обходят DPI на TLS handshake, а не на QUIC. Браузер сделает fallback
    // на TCP TLS и пойдёт через VPN с DPI-обходом. Это поведение из
    // industry-стандарта (mihomo делает то же самое в default-конфиге).
    if tun_mode {
        rules.push(json!({
            "network": "udp",
            "port": [443],
            "action": "reject"
        }));
    }

    json!({
        "rules": rules,
        "final": "proxy",
        "auto_detect_interface": true,
        // sing-box 1.12+: dial-операции которые получают hostname (адрес
        // VPN-сервера, DoH-эндпоинт и т.д.) обязаны иметь явный domain_resolver.
        // Используем "local" — системный DNS через direct outbound. Иначе на
        // bootstrap'е получим chicken-and-egg (нельзя резолвить через DoH
        // пока DoH не подключился).
        "default_domain_resolver": { "server": "local" },
    })
}

// ─── anti-DPI применение ─────────────────────────────────────────────────────

/// Применить anti-DPI настройки к proxy-outbound. Меняет outbound in-place.
///
/// **Ограничения sing-box upstream (1.13.x):**
/// - **TLS фрагментация**: только булевый флаг `tls.fragment = true`. Тонкая
///   настройка размера/задержки (как в Xray `freedom-fragment`) НЕ
///   поддерживается. Поле `fragmentation_length`/`fragmentation_interval`
///   игнорируется — sing-box использует свои оптимальные значения.
///
/// - **UDP шумы (noises)**: в upstream sing-box НЕ поддерживаются
///   (есть только в hiddify-форке). Поле `noises_*` игнорируется,
///   мы выводим предупреждение в stderr — пользователь должен
///   использовать встроенный `obfs: salamander` Hysteria2 вместо noises.
///
/// - **server_resolve через DoH**: применяется в `build_dns()`, не здесь.
fn apply_anti_dpi_to_outbound(outbound: &mut Value, anti_dpi: Option<&AntiDpiOptions>) {
    let Some(ad) = anti_dpi else { return };

    // Фрагментация TLS ClientHello — sing-box upstream (1.13.x) использует
    // `tls.fragment` как boolean. Применимо только если у outbound есть
    // tls-объект (vless+tls/reality, trojan+tls, hy2-tls и т.д.).
    if ad.fragmentation {
        if let Some(tls) = outbound.get_mut("tls") {
            if tls.is_object() {
                tls["fragment"] = json!(true);
            }
        }
    }

    // UDP-noises не поддерживаются в upstream sing-box. Пишем
    // в stderr один раз — для диагностики, чтобы пользователь не думал
    // что они применились.
    if ad.noises {
        eprintln!(
            "[sing-box anti-DPI] noises включены в настройках, но upstream \
             sing-box их не поддерживает — рекомендуется использовать \
             встроенный obfs: salamander для Hysteria2 или другой движок"
        );
    }
}

// ─── Mux (multiplexing) применение ───────────────────────────────────────────

/// Применить mux (multiplexing) к stream-based outbound.
///
/// **Применимо к**: vless / vmess / trojan / shadowsocks / socks.
/// **НЕ применимо к**: hysteria2 / tuic (свой stream multiplexing над QUIC),
/// wireguard (UDP-only, не stream).
///
/// При несовместимом протоколе тихо игнорируем — пользователь включил
/// глобальный toggle, а конкретный сервер выбрал hy2/tuic/wg. Менять
/// семантику toggle на "сервер не поддерживает" — лишний шум в UI.
///
/// Sing-box schema (1.11+):
/// ```json
/// "multiplex": {
///   "enabled": true,
///   "protocol": "smux",
///   "max_streams": 8
/// }
/// ```
///
/// Если `max_streams = 0` — поле опускаем, sing-box использует свой
/// дефолт (unlimited). `protocol` пустой → подставляем `smux`.
fn apply_mux_to_outbound(outbound: &mut Value, mux: Option<&MuxOptions>, protocol: &str) {
    let Some(mux) = mux else { return };
    if !mux.enabled {
        return;
    }

    // Только stream-based протоколы. Список нормализован в нижнем регистре.
    let supports_mux = matches!(
        protocol,
        "vless" | "vmess" | "trojan" | "shadowsocks" | "ss" | "socks"
    );
    if !supports_mux {
        return;
    }

    let proto = if mux.protocol.trim().is_empty() {
        "smux"
    } else {
        mux.protocol.as_str()
    };

    let mut multiplex = json!({
        "enabled": true,
        "protocol": proto,
    });
    if mux.max_streams > 0 {
        multiplex["max_streams"] = json!(mux.max_streams);
    }
    outbound["multiplex"] = multiplex;
}

// ─── ограничения transport ───────────────────────────────────────────────────

/// Проверка transport на совместимость с upstream sing-box.
///
/// Возвращает Err для XHTTP — этот transport реализован только в xray-core.
/// Upstream SagerNet/sing-box (наша 1.13.11 сборка) его НЕ умеет;
/// `transport: { type: "xhttp" }` валит `sing-box check` с FATAL
/// "unknown transport type: xhttp".
///
/// Hiddify-sing-box форк имеет XHTTP-патч, но на него мы не переходим —
/// проект не получает регулярных обновлений. Для долгосрочной поддержки
/// XHTTP остаются варианты:
/// 1. Переключиться на движок **Mihomo** (умеет XHTTP с 1.18+, наш в bundle).
/// 2. Hybrid-режим xray+sing-box (rare, требует возврата xray-core в bundle).
fn bail_if_unsupported_transport(transport: &str, protocol: &str, entry_name: &str) -> Result<()> {
    if transport == "xhttp" {
        bail!(
            "сервер «{entry_name}» ({protocol}+xhttp): upstream sing-box не \
             поддерживает XHTTP transport. Решения:\n\
             • переключиться на движок **Mihomo** (Settings → движок) — он умеет XHTTP\n\
             • или выбрать другой сервер из подписки (tcp/ws/grpc/h2/httpupgrade)"
        );
    }
    Ok(())
}

// ─── диспетчер протоколов ────────────────────────────────────────────────────

/// Возвращает (outbound, endpoints). Для большинства протоколов endpoints
/// будет пустым массивом, а outbound — реальный JSON. Для wireguard в
/// sing-box 1.11+ outbound заменяется placeholder'ом, а реальная
/// конфигурация уезжает в endpoints[0].
fn build_outbound_or_endpoint(entry: &ProxyEntry) -> Result<(Value, Vec<Value>)> {
    match entry.protocol.as_str() {
        "vless" => Ok((build_vless(entry)?, Vec::new())),
        "vmess" => Ok((build_vmess(entry)?, Vec::new())),
        "trojan" => Ok((build_trojan(entry)?, Vec::new())),
        "ss" | "shadowsocks" => Ok((build_ss(entry)?, Vec::new())),
        "socks" | "socks5" => Ok((build_socks(entry)?, Vec::new())),
        "hysteria2" | "hy2" => Ok((build_hysteria2(entry)?, Vec::new())),
        "tuic" => Ok((build_tuic(entry)?, Vec::new())),
        "wireguard" | "wg" => {
            let endpoint = build_wireguard_endpoint(entry)?;
            // Placeholder outbound (будет заменён selector'ом в build()).
            let placeholder = json!({ "type": "direct", "tag": "proxy" });
            Ok((placeholder, vec![endpoint]))
        }
        "xray-json" => bail!(
            "протокол xray-json должен обрабатываться через convert_xray_json_to_singbox(), \
             а не через build()"
        ),
        "singbox-json" => bail!(
            "протокол singbox-json должен обрабатываться через patch_singbox_json(), \
             а не через build()"
        ),
        "anytls" => bail!(
            "протокол anytls поддерживает только Mihomo — выберите движок mihomo \
             или другой сервер из подписки"
        ),
        p => bail!("неподдерживаемый sing-box протокол: {p}"),
    }
}

// ─── VLESS ───────────────────────────────────────────────────────────────────

fn build_vless(entry: &ProxyEntry) -> Result<Value> {
    let raw = &entry.raw;
    let uuid = raw["uuid"].as_str().context("uuid обязателен для VLESS")?;
    let flow = raw["flow"].as_str().unwrap_or("");
    let security = raw["security"].as_str().unwrap_or("none");
    let transport = raw["type"].as_str().unwrap_or("tcp");
    bail_if_unsupported_transport(transport, "VLESS", &entry.name)?;

    let mut out = json!({
        "type": "vless",
        "tag": "proxy",
        "server": entry.server,
        "server_port": entry.port,
        "uuid": uuid,
        "packet_encoding": "xudp",
    });
    if !flow.is_empty() {
        out["flow"] = flow.into();
    }

    if let Some(tls) = build_tls(security, raw) {
        out["tls"] = tls;
    }
    if let Some(transport_obj) = build_transport(transport, raw) {
        out["transport"] = transport_obj;
    }

    Ok(out)
}

// ─── VMess ───────────────────────────────────────────────────────────────────

fn build_vmess(entry: &ProxyEntry) -> Result<Value> {
    let raw = &entry.raw;
    let uuid = raw["id"].as_str().context("id обязателен для VMess")?;
    let aid = raw["aid"]
        .as_u64()
        .or_else(|| raw["aid"].as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0);
    let cipher = raw["scy"].as_str().unwrap_or("auto");
    let network = raw["net"].as_str().unwrap_or("tcp");
    bail_if_unsupported_transport(network, "VMess", &entry.name)?;
    let tls_val = raw["tls"].as_str().unwrap_or("");
    let security = if tls_val == "tls" { "tls" } else { "none" };

    let mut out = json!({
        "type": "vmess",
        "tag": "proxy",
        "server": entry.server,
        "server_port": entry.port,
        "uuid": uuid,
        "alter_id": aid,
        "security": cipher,
        "packet_encoding": "xudp",
    });

    if let Some(tls) = build_tls(security, raw) {
        out["tls"] = tls;
    }
    if let Some(transport_obj) = build_transport(network, raw) {
        out["transport"] = transport_obj;
    }

    Ok(out)
}

// ─── Trojan ──────────────────────────────────────────────────────────────────

fn build_trojan(entry: &ProxyEntry) -> Result<Value> {
    let raw = &entry.raw;
    let password = raw["password"]
        .as_str()
        .context("password обязателен для Trojan")?;
    // Trojan по умолчанию использует TLS (без TLS не имеет смысла).
    let security = raw["security"].as_str().unwrap_or("tls");
    let transport = raw["type"].as_str().unwrap_or("tcp");
    bail_if_unsupported_transport(transport, "Trojan", &entry.name)?;

    let mut out = json!({
        "type": "trojan",
        "tag": "proxy",
        "server": entry.server,
        "server_port": entry.port,
        "password": password,
    });

    if let Some(tls) = build_tls(security, raw) {
        out["tls"] = tls;
    }
    if let Some(transport_obj) = build_transport(transport, raw) {
        out["transport"] = transport_obj;
    }

    Ok(out)
}

// ─── Shadowsocks ─────────────────────────────────────────────────────────────

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
        "type": "shadowsocks",
        "tag": "proxy",
        "server": entry.server,
        "server_port": entry.port,
        "method": method,
        "password": password,
    }))
}

// ─── SOCKS5 ──────────────────────────────────────────────────────────────────

fn build_socks(entry: &ProxyEntry) -> Result<Value> {
    let raw = &entry.raw;
    let user = raw["username"].as_str().unwrap_or("");
    let password = raw["password"].as_str().unwrap_or("");

    let mut out = json!({
        "type": "socks",
        "tag": "proxy",
        "server": entry.server,
        "server_port": entry.port,
        "version": "5",
    });
    if !user.is_empty() {
        out["username"] = user.into();
        out["password"] = password.into();
    }

    Ok(out)
}

// ─── Hysteria2 ───────────────────────────────────────────────────────────────

fn build_hysteria2(entry: &ProxyEntry) -> Result<Value> {
    let raw = &entry.raw;
    let password = raw["password"]
        .as_str()
        .context("password обязателен для Hysteria2")?;

    let mut out = json!({
        "type": "hysteria2",
        "tag": "proxy",
        "server": entry.server,
        "server_port": entry.port,
        "password": password,
    });

    // Обфускация salamander — маскировка QUIC-пакетов под рандом-мусор.
    let obfs_type = raw["obfs"].as_str().unwrap_or("");
    if !obfs_type.is_empty() {
        let obfs_password = raw["obfs-password"]
            .as_str()
            .or_else(|| raw["obfs_password"].as_str())
            .unwrap_or("");
        out["obfs"] = json!({
            "type": obfs_type,
            "password": obfs_password,
        });
    }

    // TLS обязателен для Hysteria2 (h3 over TLS 1.3).
    let sni = raw["sni"]
        .as_str()
        .or_else(|| raw["peer"].as_str())
        .or_else(|| raw["host"].as_str())
        .unwrap_or("");
    let insecure = raw["insecure"]
        .as_str()
        .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
        || raw["allowInsecure"].as_bool().unwrap_or(false);

    let mut tls = json!({
        "enabled": true,
        "insecure": insecure,
        "alpn": ["h3"],
    });
    if !sni.is_empty() {
        tls["server_name"] = sni.into();
    }
    out["tls"] = tls;

    Ok(out)
}

// ─── TUIC ────────────────────────────────────────────────────────────────────

fn build_tuic(entry: &ProxyEntry) -> Result<Value> {
    let raw = &entry.raw;
    let uuid = raw["uuid"]
        .as_str()
        .context("uuid обязателен для TUIC")?;
    let password = raw["password"].as_str().unwrap_or("");
    let congestion = raw["congestion_control"]
        .as_str()
        .or_else(|| raw["congestion-control"].as_str())
        .unwrap_or("bbr");
    let udp_relay_mode = raw["udp_relay_mode"]
        .as_str()
        .or_else(|| raw["udp-relay-mode"].as_str())
        .unwrap_or("native");

    let mut out = json!({
        "type": "tuic",
        "tag": "proxy",
        "server": entry.server,
        "server_port": entry.port,
        "uuid": uuid,
        "password": password,
        "congestion_control": congestion,
        "udp_relay_mode": udp_relay_mode,
    });

    let sni = raw["sni"]
        .as_str()
        .or_else(|| raw["peer"].as_str())
        .unwrap_or("");
    let insecure = raw["insecure"]
        .as_str()
        .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let alpn_str = raw["alpn"].as_str().unwrap_or("h3");

    let mut tls = json!({
        "enabled": true,
        "insecure": insecure,
        "alpn": alpn_str.split(',').map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>(),
    });
    if !sni.is_empty() {
        tls["server_name"] = sni.into();
    }
    out["tls"] = tls;

    Ok(out)
}

// ─── WireGuard (endpoint в sing-box 1.11+) ───────────────────────────────────

fn build_wireguard_endpoint(entry: &ProxyEntry) -> Result<Value> {
    let raw = &entry.raw;
    let private_key = raw["private-key"]
        .as_str()
        .or_else(|| raw["privateKey"].as_str())
        .or_else(|| raw["secretKey"].as_str())
        .context("private-key обязателен для WireGuard")?;
    let public_key = raw["publickey"]
        .as_str()
        .or_else(|| raw["publicKey"].as_str())
        .context("publickey (peer) обязателен для WireGuard")?;

    let addr_raw = raw["address"].as_str().unwrap_or("10.0.0.2/32");
    let addresses: Vec<String> = addr_raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    let mut peer = json!({
        "address": entry.server,
        "port": entry.port,
        "public_key": public_key,
        "allowed_ips": ["0.0.0.0/0", "::/0"],
    });
    if let Some(psk) = raw["presharedkey"]
        .as_str()
        .or_else(|| raw["preSharedKey"].as_str())
    {
        if !psk.is_empty() {
            peer["pre_shared_key"] = psk.into();
        }
    }
    if let Some(reserved_str) = raw["reserved"].as_str() {
        let nums: Vec<u8> = reserved_str
            .split(',')
            .filter_map(|s| s.trim().parse::<u8>().ok())
            .collect();
        if nums.len() == 3 {
            peer["reserved"] = json!(nums);
        }
    }

    let mut endpoint = json!({
        "type": "wireguard",
        "tag": "proxy-wg",
        "address": addresses,
        "private_key": private_key,
        "peers": [peer],
    });

    if let Some(mtu) = raw["mtu"]
        .as_u64()
        .or_else(|| raw["mtu"].as_str().and_then(|s| s.parse().ok()))
    {
        endpoint["mtu"] = json!(mtu);
    }

    Ok(endpoint)
}

// ─── TLS ─────────────────────────────────────────────────────────────────────

/// Построить tls-объект для sing-box outbound. Возвращает None если
/// security="none" — sing-box не любит явный `tls.enabled=false`.
fn build_tls(security: &str, raw: &Value) -> Option<Value> {
    match security {
        "reality" => {
            let sni = raw["sni"].as_str().unwrap_or("");
            let fp = raw["fp"].as_str().unwrap_or("chrome");
            let pbk = raw["pbk"].as_str().unwrap_or("");
            let sid = raw["sid"].as_str().unwrap_or("");
            // pbk обязателен; если его нет — REALITY невозможен. Возвращаем
            // tls без reality (sing-box ругнётся при handshake — это
            // нормальная сигнализация о битом конфиге).
            if pbk.is_empty() {
                return Some(json!({
                    "enabled": true,
                    "server_name": sni,
                    "utls": { "enabled": true, "fingerprint": fp },
                }));
            }
            let mut tls = json!({
                "enabled": true,
                "server_name": sni,
                "utls": { "enabled": true, "fingerprint": fp },
                "reality": {
                    "enabled": true,
                    "public_key": pbk,
                    "short_id": sid,
                }
            });
            // spx (spider-x) опциональный, но если задан — пробрасываем
            if let Some(spx) = raw["spx"].as_str() {
                if !spx.is_empty() {
                    tls["reality"]["spider_x"] = spx.into();
                }
            }
            Some(tls)
        }
        "tls" => {
            let sni = raw["sni"]
                .as_str()
                .or_else(|| raw["host"].as_str())
                .unwrap_or("");
            let fp = raw["fp"].as_str().unwrap_or("");
            let alpn = raw["alpn"].as_str().unwrap_or("");
            let insecure = raw["allowInsecure"].as_bool().unwrap_or(false)
                || raw["insecure"]
                    .as_str()
                    .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
                    .unwrap_or(false);

            let mut tls = json!({
                "enabled": true,
                "insecure": insecure,
            });
            if !sni.is_empty() {
                tls["server_name"] = sni.into();
            }
            if !alpn.is_empty() {
                let alpn_arr: Vec<&str> = alpn.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
                tls["alpn"] = json!(alpn_arr);
            }
            if !fp.is_empty() {
                tls["utls"] = json!({ "enabled": true, "fingerprint": fp });
            }
            Some(tls)
        }
        _ => None,
    }
}

// ─── Transport ───────────────────────────────────────────────────────────────

/// Построить transport-объект для sing-box outbound. Возвращает None для
/// "tcp" (sing-box по умолчанию использует TCP, явное указание не нужно).
fn build_transport(network: &str, raw: &Value) -> Option<Value> {
    match network {
        "ws" => {
            let path = raw["path"].as_str().unwrap_or("/");
            let host = raw["host"].as_str().unwrap_or("");
            let mut ws = json!({
                "type": "ws",
                "path": path,
            });
            if !host.is_empty() {
                ws["headers"] = json!({ "Host": host });
            }
            Some(ws)
        }
        "grpc" => {
            let svc = raw["serviceName"]
                .as_str()
                .or_else(|| raw["path"].as_str())
                .unwrap_or("");
            Some(json!({
                "type": "grpc",
                "service_name": svc,
            }))
        }
        "h2" | "http" => {
            let path = raw["path"].as_str().unwrap_or("/");
            let host = raw["host"].as_str().unwrap_or("");
            let mut h = json!({
                "type": "http",
                "path": path,
            });
            if !host.is_empty() {
                h["host"] = json!([host]);
            }
            Some(h)
        }
        "httpupgrade" => {
            let path = raw["path"].as_str().unwrap_or("/");
            let host = raw["host"].as_str().unwrap_or("");
            let mut hu = json!({
                "type": "httpupgrade",
                "path": path,
            });
            if !host.is_empty() {
                hu["host"] = host.into();
            }
            Some(hu)
        }
        // XHTTP — Xray-родной transport. Upstream sing-box (1.13.11)
        // НЕ имеет XHTTP в стандартной сборке (только в форках hiddify-
        // sing-box / sing-box-extra). Возвращаем None — это приведёт к
        // fallback на default-tcp transport, и sing-box-check не примет
        // конфиг с unknown transport. Отлавливаем выше — в build_<proto>
        // дают bail если raw["type"]=="xhttp".
        "xhttp" => None,
        // tcp / unknown → default behavior (без transport-объекта).
        _ => None,
    }
}

// ─── 11.F — Применение routing-профиля к sing-box-конфигу ────────────────────

use super::routing_profile::{BoolString, RoutingProfile};

/// Расширить уже построенный sing-box-конфиг правилами из routing-профиля.
///
/// Маппинг geosite/geoip:
/// - `geosite:XX` → `geosite: ["XX"]`
/// - `geoip:XX` → `geoip: ["XX"]`
/// - конкретный домен `example.com` → `domain: ["example.com"]`
/// - `*.example.com` → `domain_suffix: ["example.com"]`
/// - CIDR `10.0.0.0/8` → `ip_cidr: ["10.0.0.0/8"]`
///
/// Правила добавляются ПОСЛЕ DNS-перехвата и private-direct (которые
/// уже есть в дефолтных rules), но ПЕРЕД финальным `final: "proxy"`.
pub fn apply_routing_profile(cfg: &mut Value, profile: &RoutingProfile) {
    // Соберём все rule_set entries (geosite-XX / geoip-XX) из всех групп правил.
    // Дедупликация — один tag не добавляется дважды.
    let mut all_sites = Vec::new();
    all_sites.extend(profile.direct_sites.iter().cloned());
    all_sites.extend(profile.proxy_sites.iter().cloned());
    all_sites.extend(profile.block_sites.iter().cloned());
    let mut all_ips = Vec::new();
    all_ips.extend(profile.direct_ip.iter().cloned());
    all_ips.extend(profile.proxy_ip.iter().cloned());
    all_ips.extend(profile.block_ip.iter().cloned());
    let new_rule_sets = collect_rule_sets(&all_sites, &all_ips);

    let route = match cfg.get_mut("route").and_then(|r| r.as_object_mut()) {
        Some(r) => r,
        None => {
            cfg["route"] = json!({ "rules": [] });
            cfg["route"].as_object_mut().unwrap()
        }
    };

    // Добавляем rule_set entries в route.rule_set[]. Дедуплицируем по tag.
    if !new_rule_sets.is_empty() {
        let rule_set = route
            .entry("rule_set".to_string())
            .or_insert_with(|| json!([]));
        if let Some(arr) = rule_set.as_array_mut() {
            let existing_tags: std::collections::HashSet<String> = arr
                .iter()
                .filter_map(|e| e.get("tag").and_then(|v| v.as_str()).map(String::from))
                .collect();
            for entry in new_rule_sets {
                let tag = entry.get("tag").and_then(|v| v.as_str()).unwrap_or("");
                if !existing_tags.contains(tag) {
                    arr.push(entry);
                }
            }
        }
    }

    let rules = route
        .entry("rules".to_string())
        .or_insert_with(|| json!([]));
    let arr = match rules.as_array_mut() {
        Some(a) => a,
        None => {
            *rules = json!([]);
            rules.as_array_mut().unwrap()
        }
    };

    // Block — первым (превалирует над всеми остальными).
    if let Some(r) = make_singbox_rule("block", &profile.block_sites, &profile.block_ip) {
        arr.push(r);
    }
    if let Some(r) = make_singbox_rule("direct", &profile.direct_sites, &profile.direct_ip) {
        arr.push(r);
    }
    if let Some(r) = make_singbox_rule("proxy", &profile.proxy_sites, &profile.proxy_ip) {
        arr.push(r);
    }

    // GlobalProxy=false → final = direct (всё что не сматчилось → direct).
    // Иначе — final остаётся "proxy" (выставлен в build_route).
    if profile.global_proxy == BoolString(false) {
        route.insert("final".to_string(), json!("direct"));
    }
}

// ─── 8.D — Per-process правила (sing-box нативный process_name) ──────────────

use super::mihomo_config::AppRule;

/// Применить per-process правила пользователя (8.D) к sing-box-конфигу.
/// Используется в обеих ветках: URI-entries после `build()`, xray-json
/// после `convert_xray_json_to_singbox()`, singbox-json после
/// `patch_singbox_json()`. Единая точка применения — поведение
/// одинаковое для всех источников конфигурации.
///
/// sing-box нативно резолвит PID процесса по соединению. На Windows —
/// через `GetExtendedTcpTable` (proxy-режим, mixed inbound) и через
/// packet→PID lookup в TUN-стеке (TUN-режим). В отличие от Mihomo
/// `find-process-mode: always` поллинга нет — короткоживущие процессы
/// тоже ловятся.
///
/// Маппинг action → JSON:
/// - `proxy` → `{"process_name": [...], "action": "route", "outbound": "proxy"}`
/// - `direct` → `{"process_name": [...], "action": "route", "outbound": "direct"}`
/// - `block` → `{"process_name": [...], "action": "reject"}`
///
/// Правила с одинаковым action группируются в один rule (массив
/// `process_name`). Это компактнее и совпадает с тем как Mihomo
/// формирует свой rules-блок.
///
/// **Позиция в `route.rules`**: сразу после непрерывной серии
/// `action: "sniff"` и `action: "hijack-dns"` в начале массива. Перед
/// private-direct, рулами routing-профиля и финальным `final`. User
/// override имеет наивысший приоритет — как в Mihomo (8.D), где
/// app-rules префиксуют провайдерские.
///
/// Пустой `app_rules` или все `exe` пустые → no-op.
pub fn apply_app_rules(cfg: &mut Value, app_rules: &[AppRule]) {
    // Группируем по action; пустые exe (после trim) пропускаем.
    let mut by_proxy: Vec<String> = Vec::new();
    let mut by_direct: Vec<String> = Vec::new();
    let mut by_block: Vec<String> = Vec::new();
    for r in app_rules {
        let exe = r.exe.trim();
        if exe.is_empty() {
            continue;
        }
        match r.action.as_str() {
            "proxy" => by_proxy.push(exe.to_string()),
            "direct" => by_direct.push(exe.to_string()),
            "block" => by_block.push(exe.to_string()),
            _ => {} // unknown action — игнорируем (валидация на UI)
        }
    }
    if by_proxy.is_empty() && by_direct.is_empty() && by_block.is_empty() {
        return;
    }

    // Находим / создаём route.rules массив.
    let route = match cfg.get_mut("route").and_then(|r| r.as_object_mut()) {
        Some(r) => r,
        None => {
            cfg["route"] = json!({ "rules": [] });
            cfg["route"].as_object_mut().unwrap()
        }
    };
    let rules = route
        .entry("rules".to_string())
        .or_insert_with(|| json!([]));
    let arr = match rules.as_array_mut() {
        Some(a) => a,
        None => {
            *rules = json!([]);
            rules.as_array_mut().unwrap()
        }
    };

    // Точка вставки: после непрерывной серии sniff/hijack-dns в
    // начале. Эти actions обязаны идти первыми, иначе sing-box не
    // успеет отснифить SNI / перехватить DNS до user-правил.
    let mut insert_at = 0usize;
    while insert_at < arr.len() {
        let action = arr[insert_at]
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if action == "sniff" || action == "hijack-dns" {
            insert_at += 1;
        } else {
            break;
        }
    }

    // Порядок: block → direct → proxy. Block первым потому что это
    // самый строгий action — если приложение есть и в block, и в
    // другом списке (юзер ошибся), сработает block.
    let mut to_insert: Vec<Value> = Vec::with_capacity(3);
    if !by_block.is_empty() {
        to_insert.push(json!({
            "process_name": by_block,
            "action": "reject",
        }));
    }
    if !by_direct.is_empty() {
        to_insert.push(json!({
            "process_name": by_direct,
            "action": "route",
            "outbound": "direct",
        }));
    }
    if !by_proxy.is_empty() {
        to_insert.push(json!({
            "process_name": by_proxy,
            "action": "route",
            "outbound": "proxy",
        }));
    }
    for (i, rule) in to_insert.into_iter().enumerate() {
        arr.insert(insert_at + i, rule);
    }
}

/// Имя rule_set entry в sing-box-конфиге. Формат: `geosite-XX`/`geoip-XX`.
/// Через сравнение этих тегов в `apply_routing_profile` мы дедуплицируем
/// массив `route.rule_set` (один и тот же rule-set добавляется только раз
/// независимо от того в скольких правилах используется).
fn geosite_rule_set_tag(category: &str) -> String {
    format!("geosite-{}", category)
}
fn geoip_rule_set_tag(category: &str) -> String {
    format!("geoip-{}", category)
}

/// URL'ы remote rule-set'ов на GitHub CDN (sagernet/sing-geosite + sing-geoip).
/// Эти .srs-файлы скачиваются sing-box'ом при первом обращении (через
/// `download_detour: direct`) и кешируются в memory + диске в `~/.config/sing-box/`.
fn geosite_rule_set_url(category: &str) -> String {
    format!(
        "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-{}.srs",
        category
    )
}
fn geoip_rule_set_url(category: &str) -> String {
    format!(
        "https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/geoip-{}.srs",
        category
    )
}

/// Сделать rule_set entry для добавления в `route.rule_set[]`.
fn make_rule_set_entry(tag: &str, url: &str) -> Value {
    json!({
        "type": "remote",
        "tag": tag,
        "format": "binary",
        "url": url,
        "download_detour": "direct",
    })
}

fn make_singbox_rule(outbound: &str, sites: &[String], ips: &[String]) -> Option<Value> {
    if sites.is_empty() && ips.is_empty() {
        return None;
    }
    // sing-box 1.11+: outbound="block" заменён на action="reject".
    let mut rule = if outbound == "block" {
        json!({ "action": "reject" })
    } else {
        json!({ "action": "route", "outbound": outbound })
    };

    // Разделяем sites на rule_set / domain / domain_suffix / domain_regex / domain_keyword
    let mut rule_sets: Vec<String> = Vec::new();
    let mut domain: Vec<String> = Vec::new();
    let mut domain_suffix: Vec<String> = Vec::new();
    let mut domain_regex: Vec<String> = Vec::new();
    let mut domain_keyword: Vec<String> = Vec::new();
    for s in sites {
        if let Some(rest) = s.strip_prefix("geosite:") {
            // private — нет в SagerNet/sing-geosite (404). Покрывается
            // встроенным private-direct правилом в build_route().
            if rest == "private" {
                continue;
            }
            rule_sets.push(geosite_rule_set_tag(rest));
        } else if let Some(rest) = s.strip_prefix("domain:") {
            domain.push(rest.to_string());
        } else if let Some(rest) = s.strip_prefix("full:") {
            domain.push(rest.to_string());
        } else if let Some(rest) = s.strip_prefix("regexp:") {
            // ВАЖНО: regex кладём в domain_regex (PCRE-style matching),
            // НЕ в domain_keyword (substring matching) — иначе паттерн
            // `\.ru$` вообще ничего не сматчит. С этим багом RU-direct
            // правила в Marzban-подписках были полностью сломаны.
            domain_regex.push(rest.to_string());
        } else if let Some(rest) = s.strip_prefix("keyword:") {
            domain_keyword.push(rest.to_string());
        } else if s.starts_with("*.") {
            domain_suffix.push(s[2..].to_string());
        } else if s.contains('.') {
            domain.push(s.clone());
        } else {
            domain_keyword.push(s.clone());
        }
    }

    // ip_cidr / geoip
    let mut ip_cidr: Vec<String> = Vec::new();
    for ip in ips {
        if let Some(rest) = ip.strip_prefix("geoip:") {
            if rest == "private" {
                continue; // см. комментарий выше
            }
            rule_sets.push(geoip_rule_set_tag(rest));
        } else {
            ip_cidr.push(ip.clone());
        }
    }

    if !rule_sets.is_empty() {
        rule["rule_set"] = json!(rule_sets);
    }
    if !domain.is_empty() {
        rule["domain"] = json!(domain);
    }
    if !domain_suffix.is_empty() {
        rule["domain_suffix"] = json!(domain_suffix);
    }
    if !domain_regex.is_empty() {
        rule["domain_regex"] = json!(domain_regex);
    }
    if !domain_keyword.is_empty() {
        rule["domain_keyword"] = json!(domain_keyword);
    }
    if !ip_cidr.is_empty() {
        rule["ip_cidr"] = json!(ip_cidr);
    }

    Some(rule)
}

/// Собрать массив rule_set entries для всех geosite:XX / geoip:XX токенов
/// встретившихся в sites/ips. Дедупликация — один tag не добавляется дважды.
///
/// Пропускает `geosite:private` и `geoip:private` — этих rule-set'ов **нет**
/// в репозитории `SagerNet/sing-geosite`/`sing-geoip` (sing-box возвращает
/// 404 при попытке скачать). Они означают «приватные адреса» (RFC 1918,
/// loopback и т.д.) — это уже покрыто нашим встроенным `ip_cidr` правилом
/// в `build_route()` / `translate_xray_rule_to_singbox()`.
fn collect_rule_sets(sites: &[String], ips: &[String]) -> Vec<Value> {
    let mut entries: Vec<Value> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for s in sites {
        if let Some(rest) = s.strip_prefix("geosite:") {
            if rest == "private" {
                continue;
            }
            let tag = geosite_rule_set_tag(rest);
            if seen.insert(tag.clone()) {
                entries.push(make_rule_set_entry(&tag, &geosite_rule_set_url(rest)));
            }
        }
    }
    for ip in ips {
        if let Some(rest) = ip.strip_prefix("geoip:") {
            if rest == "private" {
                continue;
            }
            let tag = geoip_rule_set_tag(rest);
            if seen.insert(tag.clone()) {
                entries.push(make_rule_set_entry(&tag, &geoip_rule_set_url(rest)));
            }
        }
    }
    entries
}

// ─── Конверсия Xray-JSON → sing-box JSON ─────────────────────────────────────

/// Опции для `convert_xray_json_to_singbox`. Передаются в результирующий
/// sing-box-конфиг (заменяют любые xray-inbounds на наши mixed/tun).
#[derive(Debug, Clone)]
pub struct ConvertOptions<'a> {
    pub socks_port: u16,
    pub http_port: u16,
    pub listen: &'a str,
    pub tun_mode: bool,
    pub tun_options: Option<&'a TunOptions>,
    pub anti_dpi: Option<&'a AntiDpiOptions>,
    pub socks_auth: Option<(&'a str, &'a str)>,
}

/// Конвертировать готовый Xray JSON в sing-box JSON.
///
/// Поддерживаются три типа конфигов:
/// - **Single-outbound** (Marzban single-server): 1 VPN outbound + direct,
///   любые `routing.rules[]`. Outbound получает tag="proxy".
/// - **Multiple-outbounds без balancer** (редкий случай): N outbound'ов,
///   `routing.rules[]` ссылаются по `outboundTag` на конкретные tags.
///   Сохраняем оригинальные tags (typically xray использует первый
///   matching outboundTag — это поведение sing-box dispatch также по
///   tag-у matching).
/// - **Balancer-конфиги** (Marzban "fastest" / Happ "Auto_Europe"):
///   несколько VPN-outbound'ов + `routing.balancers[]` с tag и
///   selector. Каждый balancer становится **`urltest`** outbound в
///   sing-box (period RTT-test, выбор min-latency). `selector`
///   фильтрует sub-outbound'ы по prefix-match. `fallbackTag` →
///   `default` поле urltest. Rules с `balancerTag` → ссылка на urltest;
///   rules с `outboundTag` → ссылка на конкретный outbound (например
///   habr.com → первый сервер, не через balancer).
///
/// Замечание: оригинальные tags VPN-outbound'ов сохраняются "как есть"
/// (proxy, proxy-2, proxy-3, ...). Если xray-balancer имеет тот же tag
/// что и outbound (что было бы конфликтом в sing-box), такая ситуация
/// в реальных Marzban/Happ-конфигах не встречается — у balancer всегда
/// собственное имя (Auto_Europe, и т.д.).
pub fn convert_xray_json_to_singbox(
    xray_json: &Value,
    name: &str,
    opts: &ConvertOptions,
) -> Result<Value> {
    let outbounds = xray_json
        .get("outbounds")
        .and_then(|v| v.as_array())
        .context("Xray JSON: ожидается массив outbounds[]")?;

    // VPN-outbound'ы: всё кроме direct/block/dns/api.
    let vpn_obs: Vec<&Value> = outbounds
        .iter()
        .filter(|ob| {
            let tag = ob.get("tag").and_then(|v| v.as_str()).unwrap_or("");
            let proto = ob.get("protocol").and_then(|v| v.as_str()).unwrap_or("");
            !matches!(tag, "direct" | "block" | "dns" | "api")
                && !matches!(proto, "freedom" | "blackhole" | "dns" | "")
        })
        .collect();

    if vpn_obs.is_empty() {
        bail!("Xray JSON: не найден ни один VPN-outbound");
    }

    // Парсим routing.balancers[] (Happ/Marzban "fastest"-эндпоинт).
    // Каждый balancer станет urltest-outbound'ом в sing-box.
    let xray_balancers = xray_json
        .get("routing")
        .and_then(|r| r.get("balancers"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let has_xray_balancer = !xray_balancers.is_empty();

    // Из xray.burstObservatory.pingConfig.interval тащим частоту проверки
    // RTT — Remnawave / Marzban-templates обычно ставят 1m (агрессивнее
    // чем sing-box default 5m). Если поле есть и валидное — применяем
    // ко всем urltest'ам, иначе fallback 5m.
    let urltest_interval = xray_json
        .get("burstObservatory")
        .and_then(|b| b.get("pingConfig"))
        .and_then(|p| p.get("interval"))
        .and_then(|v| v.as_str())
        .unwrap_or("5m")
        .to_string();

    // Конвертируем VPN-outbound'ы — сохраняем ОРИГИНАЛЬНЫЕ tags.
    // Если у двух outbound'ов одинаковый tag (артефакт ошибки в подписке),
    // переименовываем во второй и далее `<tag>-N`.
    let mut sb_outbounds: Vec<Value> = Vec::new();
    let mut sb_endpoints: Vec<Value> = Vec::new();
    let mut all_sub_tags: Vec<String> = Vec::new();
    let mut first_entry: Option<ProxyEntry> = None;
    let mut seen_tags: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (i, ob) in vpn_obs.iter().enumerate() {
        let original_tag = ob.get("tag").and_then(|v| v.as_str()).unwrap_or("");
        // Уникальный tag: оригинальный если не пустой и не дубликат, иначе
        // synthetic. Используем `proxy-N` индексирование с 1.
        let sub_tag: String = if !original_tag.is_empty() && !seen_tags.contains(original_tag) {
            original_tag.to_string()
        } else {
            let mut n = i + 1;
            loop {
                let candidate = format!("proxy-{n}");
                if !seen_tags.contains(&candidate) {
                    break candidate;
                }
                n += 1;
            }
        };
        seen_tags.insert(sub_tag.clone());

        // Пытаемся сконвертить outbound. Если он использует unsupported
        // transport (XHTTP) или экзотический протокол — пропускаем
        // конкретный outbound с warning, не валим всю подписку. В
        // balancer-конфигах часто 1 из 6 серверов на XHTTP, остальные
        // на tcp/ws — пропустим только проблемный.
        let entry = match xray_outbound_to_proxy_entry(ob, &sub_tag) {
            Ok(e) => e,
            Err(e) => {
                eprintln!(
                    "[xray→sing-box] пропускаю outbound[{i}] tag={original_tag:?}: {e:#}"
                );
                continue;
            }
        };

        let (mut converted, mut endpoints) = match build_outbound_or_endpoint(&entry) {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "[xray→sing-box] пропускаю outbound[{i}] tag={original_tag:?}: {e:#}"
                );
                continue;
            }
        };

        all_sub_tags.push(sub_tag.clone());
        if first_entry.is_none() {
            first_entry = Some(entry.clone());
        }

        if !endpoints.is_empty() {
            // wireguard: реальный конфиг в endpoint, outbound — placeholder.
            for ep in endpoints.iter_mut() {
                ep["tag"] = json!(sub_tag.clone());
            }
            sb_endpoints.append(&mut endpoints);
        } else {
            converted["tag"] = json!(sub_tag.clone());
            apply_anti_dpi_to_outbound(&mut converted, opts.anti_dpi);
            sb_outbounds.push(converted);
        }
    }

    if all_sub_tags.is_empty() {
        bail!(
            "Xray JSON: все VPN-outbound'ы используют unsupported transports (XHTTP) \
             или экзотические протоколы. Для XHTTP-only подписок используйте Mihomo \
             (умеет XHTTP) или подождите hybrid-режим с xray-core."
        );
    }

    // Создаём urltest для каждого xray-balancer'а.
    // Если у xray НЕТ balancers, но >1 outbound — это редкий случай
    // (multiple servers с прямым outboundTag-mapping в rules). Не
    // создаём synthetic urltest — rules будут ссылаться на конкретные
    // tags напрямую.
    for balancer in &xray_balancers {
        let bal_tag = balancer
            .get("tag")
            .and_then(|v| v.as_str())
            .unwrap_or("auto");
        // selector — массив prefix'ов tags, или единичная строка.
        let selectors: Vec<String> = balancer
            .get("selector")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        // Фильтруем sub-tags по prefix-match. Если selectors пустой —
        // все.
        let mut bal_outbounds: Vec<String> = if selectors.is_empty() {
            all_sub_tags.clone()
        } else {
            all_sub_tags
                .iter()
                .filter(|t| selectors.iter().any(|s| t.starts_with(s.as_str())))
                .cloned()
                .collect()
        };
        if bal_outbounds.is_empty() {
            // Селектор ничего не сматчил — пропускаем balancer (вместо
            // ошибки, чтобы конвертация не падала). В route.rules ссылка
            // на этот tag станет dead-tag, sing-box check ругнётся.
            continue;
        }
        // xray.balancer.fallbackTag — outbound который balancer
        // использует когда нет test-data (например первое подключение,
        // до завершения первого RTT-теста). У sing-box urltest нет
        // отдельного поля для fallback'а; вместо этого он использует
        // **первый** в `outbounds[]` пока тесты не сошлись. Поэтому
        // переставляем fallback-tag в начало списка.
        if let Some(fallback) = balancer.get("fallbackTag").and_then(|v| v.as_str()) {
            if let Some(pos) = bal_outbounds.iter().position(|t| t == fallback) {
                if pos != 0 {
                    let item = bal_outbounds.remove(pos);
                    bal_outbounds.insert(0, item);
                }
            }
        }
        let urltest = json!({
            "type": "urltest",
            "tag": bal_tag,
            "outbounds": bal_outbounds,
            "url": "https://www.gstatic.com/generate_204",
            "interval": urltest_interval,
            "tolerance": 50,
        });
        sb_outbounds.push(urltest);
    }

    // Backward-compat: если у xray НЕТ balancers И больше одного VPN-
    // outbound, и rules ссылаются на `balancerTag` или несуществующие
    // tags — нет хорошего mapping'а. Создаём synthetic "auto" urltest на
    // случай если правила опираются на него. В реальных Marzban/Happ-
    // конфигах balancer всегда декларируется явно.
    if !has_xray_balancer && vpn_obs.len() > 1 {
        sb_outbounds.push(json!({
            "type": "urltest",
            "tag": "auto",
            "outbounds": all_sub_tags.clone(),
            "url": "https://www.gstatic.com/generate_204",
            "interval": urltest_interval,
            "tolerance": 50,
        }));
    }

    sb_outbounds.push(json!({ "type": "direct", "tag": "direct" }));

    let entry_for_dns = first_entry
        .clone()
        .ok_or_else(|| anyhow::anyhow!("внутренняя ошибка: нет first_entry"))?;

    let inbounds = build_inbounds(opts.socks_port, opts.listen, opts.tun_mode, opts.tun_options, opts.socks_auth);
    let dns = build_dns(&entry_for_dns, opts.anti_dpi);
    let _ = name;
    let endpoints: Vec<Value> = sb_endpoints;
    let outbounds_arr = sb_outbounds;
    let _ = entry_for_dns;
    let entry: ProxyEntry = first_entry.unwrap();
    // is_balancer оставлен для совместимости с кодом ниже (selector replace).
    let is_balancer = has_xray_balancer || vpn_obs.len() > 1;

    // Транслируем xray routing.rules → sing-box route.rules. Префиксуем
    // sniff/hijack-dns/private-direct (как в build_route).
    let mut sb_rules: Vec<Value> = Vec::new();
    sb_rules.push(json!({ "action": "sniff" }));
    sb_rules.push(json!({ "protocol": "dns", "action": "hijack-dns" }));
    sb_rules.push(json!({
        "ip_cidr": [
            "127.0.0.0/8", "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16",
            "169.254.0.0/16", "::1/128", "fe80::/10", "fc00::/7"
        ],
        "action": "route",
        "outbound": "direct"
    }));
    if opts.tun_mode {
        sb_rules.push(json!({
            "network": "udp",
            "port": [443],
            "action": "reject"
        }));
    }

    // Собираем rule_set entries из всех geosite:/geoip: токенов
    // встретившихся в xray rules, и одновременно транслируем правила.
    // Также ищем catch-all (последнее правило с network=tcp+udp/all и
    // без других matchers'ов) — оно становится `route.final` в sing-box,
    // а не отдельным правилом.
    let mut all_sites: Vec<String> = Vec::new();
    let mut all_ips: Vec<String> = Vec::new();
    let mut final_outbound: Option<String> = None;
    if let Some(rules) = xray_json.get("routing").and_then(|r| r.get("rules")).and_then(|v| v.as_array()) {
        for rule in rules {
            if let Some(dom) = rule.get("domain").and_then(|v| v.as_array()) {
                for d in dom {
                    if let Some(s) = d.as_str() { all_sites.push(s.to_string()); }
                }
            }
            if let Some(ip) = rule.get("ip").and_then(|v| v.as_array()) {
                for i in ip {
                    if let Some(s) = i.as_str() { all_ips.push(s.to_string()); }
                }
            }
            // Catch-all: только outboundTag/balancerTag + опциональный
            // network=tcp,udp/tcp/udp, без domain/ip/source/port/etc.
            // → не добавляем как rule, ставим как `route.final`. В Happ-
            // балансере последнее правило catch-all balancerTag.
            if is_xray_rule_catchall(rule) {
                let tag = rule
                    .get("outboundTag")
                    .and_then(|v| v.as_str())
                    .or_else(|| rule.get("balancerTag").and_then(|v| v.as_str()));
                if let Some(t) = tag {
                    final_outbound = Some(t.to_string());
                }
                continue;
            }
            if let Some(translated) = translate_xray_rule_to_singbox(rule) {
                sb_rules.push(translated);
            }
        }
    }

    let rule_sets = collect_rule_sets(&all_sites, &all_ips);

    // route.final: либо catch-all из xray, либо первый balancer-tag,
    // либо "proxy" (default first outbound), либо первый sub-tag.
    let final_tag = final_outbound
        .or_else(|| {
            xray_balancers
                .first()
                .and_then(|b| b.get("tag").and_then(|v| v.as_str()).map(String::from))
        })
        .or_else(|| all_sub_tags.first().cloned())
        .unwrap_or_else(|| "proxy".to_string());

    let mut route = json!({
        "rules": sb_rules,
        "final": final_tag,
        "auto_detect_interface": true,
        "default_domain_resolver": { "server": "local" },
    });
    if !rule_sets.is_empty() {
        route["rule_set"] = json!(rule_sets);
    }

    // Если в xray.routing.domainStrategy = "AsIs" — транслируем как nothing
    // (по умолчанию sing-box без resolve). Иначе игнорируем — sing-box использует
    // совершенно другую модель split-DNS.
    let _ = route.get_mut("final"); // suppress unused

    let mut config = json!({
        "log": { "level": "warn", "timestamp": true },
        "dns": dns,
        "inbounds": inbounds,
        "outbounds": outbounds_arr,
        "route": route,
    });

    if !endpoints.is_empty() {
        // Single-wg случай: оригинальный build_outbound_or_endpoint вернул
        // placeholder direct outbound с tag="proxy" + endpoint с tag="proxy-wg".
        // Заменяем placeholder на selector → endpoint. В balancer-случае
        // endpoint'ам уже присвоены индивидуальные sub_tags в цикле выше,
        // и `outbounds[].proxy` это уже urltest — placeholder'а нет, replace
        // будет no-op (что нормально).
        if !is_balancer {
            if let Some(ob_arr) = config["outbounds"].as_array_mut() {
                if let Some(idx) = ob_arr.iter().position(|o| o["tag"] == "proxy") {
                    ob_arr[idx] = json!({
                        "type": "selector",
                        "tag": "proxy",
                        "outbounds": ["proxy-wg"],
                        "default": "proxy-wg"
                    });
                }
            }
        }
        config["endpoints"] = json!(endpoints);
    }

    let _ = entry;
    Ok(config)
}

/// Catch-all xray-rule: только outboundTag/balancerTag (опционально
/// network=tcp,udp/tcp/udp), без domain/ip/source/port/protocol matchers.
/// В нашем sing-box такие правила выражаются через `route.final` —
/// нет смысла добавлять их в `route.rules` (sing-box принимает rule
/// без matchers, но семантически чище через final).
fn is_xray_rule_catchall(rule: &Value) -> bool {
    let has_tag = rule.get("outboundTag").is_some() || rule.get("balancerTag").is_some();
    if !has_tag {
        return false;
    }
    // Проверяем что нет других matcher-полей.
    let matcher_fields = [
        "domain", "ip", "source", "sourceIp", "port", "sourcePort",
        "protocol", "user", "inboundTag", "attrs",
    ];
    for k in matcher_fields {
        let has_value = rule
            .get(k)
            .map(|v| match v {
                Value::Null => false,
                Value::Array(a) => !a.is_empty(),
                Value::String(s) => !s.is_empty(),
                _ => true,
            })
            .unwrap_or(false);
        if has_value {
            return false;
        }
    }
    true
}

/// Перевести одно правило xray-routing → sing-box route.rule.
/// Возвращает None если правило стало пустым после очистки (например,
/// содержало только `geosite:private` который мы не транслируем — есть
/// уже private-direct правило по умолчанию).
fn translate_xray_rule_to_singbox(rule: &Value) -> Option<Value> {
    // Marzban-balancer-конфиги ссылаются на `balancerTag` вместо
    // `outboundTag`. У нас balancer становится urltest-outbound с тем
    // же tag (см. convert_xray_json_to_singbox), так что просто берём
    // оба варианта.
    let outbound_tag = rule
        .get("outboundTag")
        .and_then(|v| v.as_str())
        .or_else(|| rule.get("balancerTag").and_then(|v| v.as_str()))?;

    // sing-box 1.11+: outbound="block" заменён на action="reject".
    let mut sb_rule = if outbound_tag == "block" {
        json!({ "action": "reject" })
    } else {
        json!({ "action": "route", "outbound": outbound_tag })
    };

    let mut has_match = false;

    // domain → rule_set (geosite:XX) / domain / suffix / regex / keyword
    if let Some(dom_arr) = rule.get("domain").and_then(|v| v.as_array()) {
        let mut rule_sets: Vec<String> = Vec::new();
        let mut domain: Vec<String> = Vec::new();
        let mut suffix: Vec<String> = Vec::new();
        let mut regex: Vec<String> = Vec::new();
        let mut keyword: Vec<String> = Vec::new();

        for d in dom_arr {
            let s = match d.as_str() {
                Some(s) => s,
                None => continue,
            };
            if s == "geosite:private" {
                continue; // уже покрыто private-direct
            }
            if let Some(rest) = s.strip_prefix("geosite:") {
                rule_sets.push(geosite_rule_set_tag(rest));
            } else if let Some(rest) = s.strip_prefix("domain:") {
                domain.push(rest.to_string());
            } else if let Some(rest) = s.strip_prefix("full:") {
                domain.push(rest.to_string());
            } else if let Some(rest) = s.strip_prefix("regexp:") {
                // regex → domain_regex (НЕ keyword: substring-matching не
                // подходит для PCRE-паттернов типа `\.ru$`).
                regex.push(rest.to_string());
            } else if let Some(rest) = s.strip_prefix("keyword:") {
                keyword.push(rest.to_string());
            } else if s.starts_with("*.") {
                suffix.push(s[2..].to_string());
            } else if s.contains('.') {
                suffix.push(s.to_string());
            } else {
                keyword.push(s.to_string());
            }
        }
        if !rule_sets.is_empty() { sb_rule["rule_set"] = json!(rule_sets); has_match = true; }
        if !domain.is_empty() { sb_rule["domain"] = json!(domain); has_match = true; }
        if !suffix.is_empty() { sb_rule["domain_suffix"] = json!(suffix); has_match = true; }
        if !regex.is_empty() { sb_rule["domain_regex"] = json!(regex); has_match = true; }
        if !keyword.is_empty() { sb_rule["domain_keyword"] = json!(keyword); has_match = true; }
    }

    // ip → ip_cidr / rule_set (geoip:XX)
    if let Some(ip_arr) = rule.get("ip").and_then(|v| v.as_array()) {
        let mut ip_cidr: Vec<String> = Vec::new();
        let mut extra_rule_sets: Vec<String> = Vec::new();
        for ip in ip_arr {
            let s = match ip.as_str() {
                Some(s) => s,
                None => continue,
            };
            if s == "geoip:private" {
                continue;
            }
            if let Some(rest) = s.strip_prefix("geoip:") {
                extra_rule_sets.push(geoip_rule_set_tag(rest));
            } else {
                ip_cidr.push(s.to_string());
            }
        }
        if !extra_rule_sets.is_empty() {
            // Сливаем с уже существующим rule_set (из domain-секции выше)
            let existing = sb_rule.get("rule_set").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let mut merged: Vec<String> = existing.iter().filter_map(|v| v.as_str().map(String::from)).collect();
            merged.extend(extra_rule_sets);
            sb_rule["rule_set"] = json!(merged);
            has_match = true;
        }
        if !ip_cidr.is_empty() { sb_rule["ip_cidr"] = json!(ip_cidr); has_match = true; }
    }

    // network: tcp,udp / port: 443 — транслируем как есть
    if let Some(net) = rule.get("network").and_then(|v| v.as_str()) {
        let nets: Vec<&str> = net.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
        if nets.len() == 1 {
            sb_rule["network"] = json!(nets[0]);
            has_match = true;
        }
        // Если оба tcp+udp — не задаём (default = всё).
    }
    if let Some(port_arr) = rule.get("port").and_then(|v| v.as_array()) {
        sb_rule["port"] = port_arr.clone().into();
        has_match = true;
    } else if let Some(port_str) = rule.get("port").and_then(|v| v.as_str()) {
        // Xray поддерживает "443,80" или "1000-2000"
        let ports: Vec<u16> = port_str
            .split(',')
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        if !ports.is_empty() {
            sb_rule["port"] = json!(ports);
            has_match = true;
        }
    }

    if has_match { Some(sb_rule) } else { None }
}

/// Собрать минимальный ProxyEntry из xray-outbound (только server/port/protocol/raw).
/// Используется внутри `convert_xray_json_to_singbox`.
fn xray_outbound_to_proxy_entry(ob: &Value, name: &str) -> Result<ProxyEntry> {
    let protocol = ob
        .get("protocol")
        .and_then(|v| v.as_str())
        .context("xray-outbound без protocol")?;

    let (server, port, raw) = match protocol {
        "vless" | "vmess" => {
            let vnext = ob
                .get("settings")
                .and_then(|s| s.get("vnext"))
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .context("vless/vmess: settings.vnext[0] не найден")?;
            let server = vnext.get("address").and_then(|v| v.as_str()).context("address")?.to_string();
            let port = vnext.get("port").and_then(|v| v.as_u64()).context("port")? as u16;
            let user = vnext
                .get("users")
                .and_then(|u| u.as_array())
                .and_then(|a| a.first())
                .context("users[0]")?;

            let mut raw = serde_json::Map::new();
            if protocol == "vless" {
                if let Some(uuid) = user.get("id").and_then(|v| v.as_str()) {
                    raw.insert("uuid".into(), uuid.to_string().into());
                }
                if let Some(flow) = user.get("flow").and_then(|v| v.as_str()) {
                    if !flow.is_empty() {
                        raw.insert("flow".into(), flow.to_string().into());
                    }
                }
            } else {
                // vmess
                if let Some(uuid) = user.get("id").and_then(|v| v.as_str()) {
                    raw.insert("id".into(), uuid.to_string().into());
                }
                let aid = user.get("alterId").and_then(|v| v.as_u64()).unwrap_or(0);
                raw.insert("aid".into(), aid.into());
                let scy = user.get("security").and_then(|v| v.as_str()).unwrap_or("auto");
                raw.insert("scy".into(), scy.to_string().into());
            }

            // Apply streamSettings to raw — наш build_vless/build_vmess читает
            // флаги transport/security оттуда же где URI-парсер.
            if let Some(stream) = ob.get("streamSettings") {
                apply_xray_stream_to_raw(&mut raw, stream, protocol);
            }
            (server, port, Value::Object(raw))
        }
        "trojan" => {
            let server_obj = ob
                .get("settings")
                .and_then(|s| s.get("servers"))
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .context("trojan: settings.servers[0] не найден")?;
            let server = server_obj.get("address").and_then(|v| v.as_str()).context("address")?.to_string();
            let port = server_obj.get("port").and_then(|v| v.as_u64()).context("port")? as u16;
            let password = server_obj.get("password").and_then(|v| v.as_str()).unwrap_or("");

            let mut raw = serde_json::Map::new();
            raw.insert("password".into(), password.to_string().into());
            if let Some(stream) = ob.get("streamSettings") {
                apply_xray_stream_to_raw(&mut raw, stream, "trojan");
            }
            (server, port, Value::Object(raw))
        }
        "shadowsocks" => {
            let server_obj = ob
                .get("settings")
                .and_then(|s| s.get("servers"))
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .context("ss: settings.servers[0] не найден")?;
            let server = server_obj.get("address").and_then(|v| v.as_str()).context("address")?.to_string();
            let port = server_obj.get("port").and_then(|v| v.as_u64()).context("port")? as u16;
            let method = server_obj.get("method").and_then(|v| v.as_str()).unwrap_or("");
            let password = server_obj.get("password").and_then(|v| v.as_str()).unwrap_or("");

            let mut raw = serde_json::Map::new();
            raw.insert("cipher".into(), method.to_string().into());
            raw.insert("password".into(), password.to_string().into());
            (server, port, Value::Object(raw))
        }
        "hysteria2" => {
            let server_obj = ob
                .get("settings")
                .and_then(|s| s.get("servers"))
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .context("hysteria2: settings.servers[0] не найден")?;
            let server = server_obj.get("address").and_then(|v| v.as_str()).context("address")?.to_string();
            let port = server_obj.get("port").and_then(|v| v.as_u64()).context("port")? as u16;
            let password = server_obj.get("password").and_then(|v| v.as_str()).unwrap_or("");

            let mut raw = serde_json::Map::new();
            raw.insert("password".into(), password.to_string().into());

            if let Some(obfs) = server_obj.get("obfs") {
                if let Some(t) = obfs.get("type").and_then(|v| v.as_str()) {
                    raw.insert("obfs".into(), t.to_string().into());
                }
                if let Some(p) = obfs.get("password").and_then(|v| v.as_str()) {
                    raw.insert("obfs-password".into(), p.to_string().into());
                }
            }
            if let Some(tls) = ob.get("streamSettings").and_then(|s| s.get("tlsSettings")) {
                if let Some(sni) = tls.get("serverName").and_then(|v| v.as_str()) {
                    raw.insert("sni".into(), sni.to_string().into());
                }
                if tls.get("allowInsecure").and_then(|v| v.as_bool()).unwrap_or(false) {
                    raw.insert("allowInsecure".into(), true.into());
                }
            }
            (server, port, Value::Object(raw))
        }
        "wireguard" => {
            let settings = ob.get("settings").context("wireguard: settings не найден")?;
            let peers = settings.get("peers").and_then(|v| v.as_array()).context("wg peers")?;
            let peer = peers.first().context("wg peers пуст")?;

            let endpoint = peer.get("endpoint").and_then(|v| v.as_str()).context("wg endpoint")?;
            let (server, port) = endpoint
                .rsplit_once(':')
                .and_then(|(h, p)| Some((h.to_string(), p.parse::<u16>().ok()?)))
                .context("wg endpoint некорректен")?;

            let mut raw = serde_json::Map::new();
            if let Some(sk) = settings.get("secretKey").and_then(|v| v.as_str()) {
                raw.insert("private-key".into(), sk.to_string().into());
            }
            if let Some(pk) = peer.get("publicKey").and_then(|v| v.as_str()) {
                raw.insert("publickey".into(), pk.to_string().into());
            }
            if let Some(addr) = settings.get("address").and_then(|v| v.as_array()) {
                let joined: Vec<String> = addr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                if !joined.is_empty() {
                    raw.insert("address".into(), joined.join(",").into());
                }
            }
            if let Some(mtu) = settings.get("mtu").and_then(|v| v.as_u64()) {
                raw.insert("mtu".into(), mtu.into());
            }
            if let Some(reserved) = settings.get("reserved").and_then(|v| v.as_array()) {
                let nums: Vec<String> = reserved.iter().filter_map(|v| v.as_u64()).map(|n| n.to_string()).collect();
                if !nums.is_empty() {
                    raw.insert("reserved".into(), nums.join(",").into());
                }
            }
            (server, port, Value::Object(raw))
        }
        "socks" => {
            let server_obj = ob
                .get("settings")
                .and_then(|s| s.get("servers"))
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .context("socks: settings.servers[0] не найден")?;
            let server = server_obj.get("address").and_then(|v| v.as_str()).context("address")?.to_string();
            let port = server_obj.get("port").and_then(|v| v.as_u64()).context("port")? as u16;

            let mut raw = serde_json::Map::new();
            if let Some(users) = server_obj.get("users").and_then(|v| v.as_array()) {
                if let Some(u0) = users.first() {
                    if let Some(user) = u0.get("user").and_then(|v| v.as_str()) {
                        raw.insert("username".into(), user.to_string().into());
                    }
                    if let Some(pass) = u0.get("pass").and_then(|v| v.as_str()) {
                        raw.insert("password".into(), pass.to_string().into());
                    }
                }
            }
            (server, port, Value::Object(raw))
        }
        p => bail!("конверсия xray→sing-box: неподдерживаемый протокол {p}"),
    };

    // Маппинг xray-protocol → наш внутренний (для build_outbound_or_endpoint)
    let internal_proto = match protocol {
        "shadowsocks" => "ss",
        other => other,
    }
    .to_string();

    Ok(ProxyEntry {
        name: name.to_string(),
        protocol: internal_proto,
        server,
        port,
        raw,
        engine_compat: vec!["sing-box".to_string()],
    })
}

/// Локальная копия логики apply_stream_to_raw из subscription.rs,
/// перенесённая сюда чтобы не делать функцию pub. Поведение должно
/// совпадать — оба места используют одни и те же raw-имена полей.
fn apply_xray_stream_to_raw(
    raw: &mut serde_json::Map<String, Value>,
    stream: &Value,
    _protocol: &str,
) {
    let network = stream.get("network").and_then(|v| v.as_str()).unwrap_or("tcp");
    raw.insert("type".into(), network.to_string().into());

    let security = stream.get("security").and_then(|v| v.as_str()).unwrap_or("none");
    raw.insert("security".into(), security.to_string().into());

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
    if let Some(ws) = stream.get("wsSettings") {
        if let Some(path) = ws.get("path").and_then(|v| v.as_str()) {
            raw.insert("path".into(), path.to_string().into());
        }
        if let Some(host) = ws.get("headers").and_then(|h| h.get("Host")).and_then(|v| v.as_str())
            .or_else(|| ws.get("host").and_then(|v| v.as_str()))
        {
            raw.insert("host".into(), host.to_string().into());
        }
    }
    if let Some(grpc) = stream.get("grpcSettings") {
        if let Some(svc) = grpc.get("serviceName").and_then(|v| v.as_str()) {
            raw.insert("serviceName".into(), svc.to_string().into());
            raw.insert("path".into(), svc.to_string().into());
        }
    }
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
    if let Some(hu) = stream.get("httpupgradeSettings") {
        if let Some(path) = hu.get("path").and_then(|v| v.as_str()) {
            raw.insert("path".into(), path.to_string().into());
        }
        if let Some(host) = hu.get("host").and_then(|v| v.as_str()) {
            raw.insert("host".into(), host.to_string().into());
        }
    }
}

// ─── Passthrough sing-box JSON (для Remnawave) ───────────────────────────────

/// Опции для патча passthrough-sing-box-конфига.
pub struct PatchOptions<'a> {
    pub socks_port: u16,
    pub listen: &'a str,
    pub tun_mode: bool,
    pub tun_options: Option<&'a TunOptions>,
    pub socks_auth: Option<(&'a str, &'a str)>,
}

/// Применить наши минимальные правки к готовому sing-box JSON конфигу
/// (Remnawave-style). Алгоритм — заменить inbounds на наши mixed/tun,
/// сохранить outbounds/route/dns как есть.
///
/// Это passthrough-режим: вся логика split-routing / DNS / endpoints
/// остаётся такой какой её прислала панель. Мы только подменяем точки
/// входа (как Mihomo full-yaml passthrough).
pub fn patch_singbox_json(
    raw_json: Value,
    opts: &PatchOptions,
) -> Result<Value> {
    let mut config = raw_json;

    // Заменяем inbounds — наши mixed/tun. Старые inbounds у Remnawave
    // обычно тоже mixed/tun, но с другими портами/auth.
    config["inbounds"] = json!(build_inbounds(
        opts.socks_port,
        opts.listen,
        opts.tun_mode,
        opts.tun_options,
        opts.socks_auth,
    ));

    // Гарантируем что есть direct outbound. block/dns в sing-box 1.11+
    // заменены на rule actions ("reject"/"hijack-dns"), отдельные
    // outbound'ы для них больше не нужны (и deprecated).
    let outbounds = config
        .get_mut("outbounds")
        .and_then(|v| v.as_array_mut())
        .context("sing-box JSON без outbounds[]")?;

    let has_direct = outbounds.iter().any(|o| o.get("tag").and_then(|v| v.as_str()) == Some("direct"));
    if !has_direct {
        outbounds.push(json!({ "type": "direct", "tag": "direct" }));
    }
    // Удаляем deprecated block/dns outbound'ы если они были — наши новые
    // правила используют actions, а panel может прислать legacy outbound'ы.
    outbounds.retain(|o| {
        let t = o.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let tag = o.get("tag").and_then(|v| v.as_str()).unwrap_or("");
        t != "block" && t != "dns" && tag != "block" && tag != "dns-out"
    });

    // Гарантируем что route.rules содержит sniff + hijack-dns + private-direct.
    // Если у Remnawave уже есть свои rules — добавляем наши только если
    // они отсутствуют (проверка по action и protocol).
    let route = config
        .as_object_mut()
        .unwrap()
        .entry("route".to_string())
        .or_insert_with(|| json!({}));
    if !route.is_object() {
        *route = json!({});
    }
    let route_obj = route.as_object_mut().unwrap();
    let rules = route_obj.entry("rules".to_string()).or_insert_with(|| json!([]));
    let rules_arr = rules.as_array_mut().context("route.rules не массив")?;

    // Заменяем legacy outbound: "block" / "dns-out" в существующих
    // правилах на новые actions (если panel прислала старый формат).
    for r in rules_arr.iter_mut() {
        if let Some(obj) = r.as_object_mut() {
            let outbound = obj.get("outbound").and_then(|v| v.as_str()).map(String::from);
            if let Some(ob) = outbound {
                if ob == "block" {
                    obj.remove("outbound");
                    obj.insert("action".to_string(), json!("reject"));
                } else if ob == "dns-out" {
                    obj.remove("outbound");
                    obj.insert("action".to_string(), json!("hijack-dns"));
                }
            }
        }
    }

    let has_sniff = rules_arr.iter().any(|r| r.get("action").and_then(|v| v.as_str()) == Some("sniff"));
    let has_dns_hijack = rules_arr.iter().any(|r| {
        r.get("protocol").and_then(|v| v.as_str()) == Some("dns")
            && r.get("action").and_then(|v| v.as_str()) == Some("hijack-dns")
    });

    let mut prepend: Vec<Value> = Vec::new();
    prepend.push(json!({
        "ip_cidr": [
            "127.0.0.0/8", "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16",
            "169.254.0.0/16", "::1/128", "fe80::/10", "fc00::/7"
        ],
        "action": "route",
        "outbound": "direct"
    }));
    if !has_dns_hijack {
        prepend.insert(0, json!({ "protocol": "dns", "action": "hijack-dns" }));
    }
    if !has_sniff {
        prepend.insert(0, json!({ "action": "sniff" }));
    }
    for r in prepend.into_iter().rev() {
        rules_arr.insert(0, r);
    }

    // auto_detect_interface=true — обязательно для built-in TUN на Windows.
    route_obj.insert("auto_detect_interface".to_string(), json!(true));

    // final → "proxy" если не задано (Remnawave обычно задаёт).
    if !route_obj.contains_key("final") {
        route_obj.insert("final".to_string(), json!("proxy"));
    }

    // log: гарантируем минимальный лог для диагностики.
    if !config.as_object().unwrap().contains_key("log") {
        config["log"] = json!({ "level": "warn", "timestamp": true });
    }

    Ok(config)
}

// ─── Тесты ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Валидный 43-символьный base64-URL public_key для REALITY (нули).
    /// sing-box валидирует длину через base64.RawURLEncoding.DecodeString
    /// и требует ровно 32 байта на выходе.
    const TEST_REALITY_PUBKEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn vless_entry() -> ProxyEntry {
        ProxyEntry {
            name: "test".to_string(),
            protocol: "vless".to_string(),
            server: "de4.example.com".to_string(),
            port: 443,
            raw: json!({
                "uuid": "12345678-1234-1234-1234-123456789012",
                "flow": "xtls-rprx-vision",
                "security": "reality",
                "type": "tcp",
                "sni": "google.com",
                "fp": "chrome",
                "pbk": TEST_REALITY_PUBKEY,
                "sid": "01ab",
            }),
            engine_compat: vec!["sing-box".to_string()],
        }
    }

    #[test]
    fn build_vless_reality_basic() {
        let entry = vless_entry();
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None).unwrap();

        let outbounds = cfg.json["outbounds"].as_array().unwrap();
        let proxy = outbounds.iter().find(|o| o["tag"] == "proxy").unwrap();
        assert_eq!(proxy["type"], "vless");
        assert_eq!(proxy["server"], "de4.example.com");
        assert_eq!(proxy["server_port"], 443);
        assert_eq!(proxy["uuid"], "12345678-1234-1234-1234-123456789012");
        assert_eq!(proxy["flow"], "xtls-rprx-vision");
        assert_eq!(proxy["tls"]["enabled"], true);
        assert_eq!(proxy["tls"]["server_name"], "google.com");
        assert_eq!(proxy["tls"]["reality"]["enabled"], true);
        assert_eq!(proxy["tls"]["reality"]["public_key"], TEST_REALITY_PUBKEY);
        assert_eq!(proxy["tls"]["reality"]["short_id"], "01ab");
        assert_eq!(proxy["tls"]["utls"]["fingerprint"], "chrome");
    }

    #[test]
    fn build_includes_only_proxy_and_direct_outbounds() {
        // sing-box 1.11+: block и dns outbound'ы deprecated, заменены
        // на rule actions reject/hijack-dns. Должны быть только proxy + direct.
        let entry = vless_entry();
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None).unwrap();
        let outbounds = cfg.json["outbounds"].as_array().unwrap();
        assert_eq!(outbounds.len(), 2);
        assert!(outbounds.iter().any(|o| o["tag"] == "proxy"));
        assert!(outbounds.iter().any(|o| o["tag"] == "direct" && o["type"] == "direct"));
        assert!(!outbounds.iter().any(|o| o["type"] == "block"));
        assert!(!outbounds.iter().any(|o| o["type"] == "dns"));
    }

    #[test]
    fn build_proxy_mode_has_only_mixed_inbound() {
        let entry = vless_entry();
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None).unwrap();
        let inbounds = cfg.json["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 1);
        assert_eq!(inbounds[0]["type"], "mixed");
        assert_eq!(inbounds[0]["listen"], "127.0.0.1");
        assert_eq!(inbounds[0]["listen_port"], 30000);
    }

    #[test]
    fn build_tun_mode_adds_tun_inbound() {
        let entry = vless_entry();
        let tun_opts = TunOptions {
            interface_name: "nemefisto-test".to_string(),
            address: "198.18.0.1/15".to_string(),
            mtu: 9000,
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", true, Some(&tun_opts), None, None, None).unwrap();
        let inbounds = cfg.json["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 2);
        let tun = inbounds.iter().find(|i| i["type"] == "tun").unwrap();
        assert_eq!(tun["interface_name"], "nemefisto-test");
        assert_eq!(tun["auto_route"], true);
        assert_eq!(tun["stack"], "gvisor");
        // auto_detect_interface перенесён с inbound на route в sing-box 1.11+.
        assert_eq!(cfg.json["route"]["auto_detect_interface"], true);
    }

    #[test]
    fn build_socks_auth_adds_users() {
        let entry = vless_entry();
        let cfg = build(
            &entry, 30000, 30000, "127.0.0.1", false, None, None, Some(("user", "pass")), None
        ).unwrap();
        let mixed = &cfg.json["inbounds"][0];
        let users = mixed["users"].as_array().unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0]["username"], "user");
        assert_eq!(users[0]["password"], "pass");
    }

    #[test]
    fn build_no_socks_auth_omits_users() {
        let entry = vless_entry();
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None).unwrap();
        let mixed = &cfg.json["inbounds"][0];
        assert!(mixed.get("users").is_none());
    }

    #[test]
    fn build_route_has_sniff_dns_intercept_private_direct() {
        let entry = vless_entry();
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None).unwrap();
        let rules = cfg.json["route"]["rules"].as_array().unwrap();
        // Первое — sniff action (sing-box 1.11+ заменил inbound.sniff)
        assert_eq!(rules[0]["action"], "sniff");
        // Второе — DNS hijack
        assert_eq!(rules[1]["protocol"], "dns");
        assert_eq!(rules[1]["action"], "hijack-dns");
        // Третье — private adresses → direct
        let private = &rules[2];
        let cidrs = private["ip_cidr"].as_array().unwrap();
        assert!(cidrs.iter().any(|c| c == "192.168.0.0/16"));
        assert_eq!(private["action"], "route");
        assert_eq!(private["outbound"], "direct");
        // final = proxy
        assert_eq!(cfg.json["route"]["final"], "proxy");
    }

    #[test]
    fn build_tun_mode_blocks_quic() {
        let entry = vless_entry();
        let tun_opts = TunOptions::default();
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", true, Some(&tun_opts), None, None, None).unwrap();
        let rules = cfg.json["route"]["rules"].as_array().unwrap();
        assert!(rules.iter().any(|r| {
            r["network"] == "udp" && r["port"][0] == 443 && r["action"] == "reject"
        }));
    }

    #[test]
    fn build_proxy_mode_does_not_block_quic() {
        let entry = vless_entry();
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None).unwrap();
        let rules = cfg.json["route"]["rules"].as_array().unwrap();
        assert!(!rules.iter().any(|r| r["action"] == "reject"));
    }

    // ─── 8.D — apply_app_rules ───────────────────────────────────────────────

    fn rule(exe: &str, action: &str) -> AppRule {
        AppRule {
            exe: exe.to_string(),
            action: action.to_string(),
            comment: None,
        }
    }

    #[test]
    fn apply_app_rules_empty_is_noop() {
        let entry = vless_entry();
        let mut cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None)
            .unwrap()
            .json;
        let before = cfg["route"]["rules"].as_array().unwrap().len();
        apply_app_rules(&mut cfg, &[]);
        assert_eq!(cfg["route"]["rules"].as_array().unwrap().len(), before);
    }

    #[test]
    fn apply_app_rules_proxy_outbound() {
        let entry = vless_entry();
        let mut cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None)
            .unwrap()
            .json;
        apply_app_rules(&mut cfg, &[rule("telegram.exe", "proxy")]);
        let rules = cfg["route"]["rules"].as_array().unwrap();
        let proxy_rule = rules
            .iter()
            .find(|r| r["outbound"] == "proxy" && r["action"] == "route")
            .expect("должно быть route+proxy правило");
        assert_eq!(proxy_rule["process_name"][0], "telegram.exe");
    }

    #[test]
    fn apply_app_rules_direct_outbound() {
        let entry = vless_entry();
        let mut cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None)
            .unwrap()
            .json;
        apply_app_rules(&mut cfg, &[rule("steam.exe", "direct")]);
        let rules = cfg["route"]["rules"].as_array().unwrap();
        // Ищем именно user-правило (с process_name), а не private-direct.
        let direct_rule = rules
            .iter()
            .find(|r| r.get("process_name").is_some() && r["outbound"] == "direct")
            .expect("должно быть route+direct правило с process_name");
        assert_eq!(direct_rule["action"], "route");
        assert_eq!(direct_rule["process_name"][0], "steam.exe");
    }

    #[test]
    fn apply_app_rules_block_action_uses_reject() {
        let entry = vless_entry();
        let mut cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None)
            .unwrap()
            .json;
        apply_app_rules(&mut cfg, &[rule("ads.exe", "block")]);
        let rules = cfg["route"]["rules"].as_array().unwrap();
        let block_rule = rules
            .iter()
            .find(|r| r.get("process_name").is_some() && r["action"] == "reject")
            .expect("должно быть reject-правило с process_name");
        assert_eq!(block_rule["process_name"][0], "ads.exe");
        // reject — без outbound
        assert!(block_rule.get("outbound").is_none());
    }

    #[test]
    fn apply_app_rules_groups_same_action_into_single_rule() {
        let entry = vless_entry();
        let mut cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None)
            .unwrap()
            .json;
        apply_app_rules(
            &mut cfg,
            &[
                rule("telegram.exe", "proxy"),
                rule("discord.exe", "proxy"),
                rule("steam.exe", "direct"),
            ],
        );
        let rules = cfg["route"]["rules"].as_array().unwrap();
        let proxy_rule = rules
            .iter()
            .find(|r| r.get("process_name").is_some() && r["outbound"] == "proxy")
            .unwrap();
        let names = proxy_rule["process_name"].as_array().unwrap();
        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|n| n == "telegram.exe"));
        assert!(names.iter().any(|n| n == "discord.exe"));

        let direct_rule = rules
            .iter()
            .find(|r| r.get("process_name").is_some() && r["outbound"] == "direct")
            .unwrap();
        let direct_names = direct_rule["process_name"].as_array().unwrap();
        assert_eq!(direct_names.len(), 1);
        assert_eq!(direct_names[0], "steam.exe");
    }

    #[test]
    fn apply_app_rules_inserts_after_sniff_and_dns() {
        // Порядок должен быть: sniff (0) → hijack-dns (1) → user-rules (2..)
        // → private-direct → ... → final.
        let entry = vless_entry();
        let mut cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None)
            .unwrap()
            .json;
        apply_app_rules(&mut cfg, &[rule("foo.exe", "proxy")]);
        let rules = cfg["route"]["rules"].as_array().unwrap();
        assert_eq!(rules[0]["action"], "sniff");
        assert_eq!(rules[1]["action"], "hijack-dns");
        // Третий — наш user-rule (proxy)
        assert_eq!(rules[2]["process_name"][0], "foo.exe");
        assert_eq!(rules[2]["outbound"], "proxy");
        // Четвёртый — private-direct (которая была раньше на index=2)
        assert!(rules[3]["ip_cidr"].is_array());
        assert_eq!(rules[3]["outbound"], "direct");
    }

    #[test]
    fn apply_app_rules_block_first_then_direct_then_proxy() {
        // Внутри нашего блока порядок: block → direct → proxy. Block
        // первым, чтобы при пересекающихся exe (юзер ошибся, exe в
        // двух списках) сработал самый строгий action.
        let entry = vless_entry();
        let mut cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None)
            .unwrap()
            .json;
        apply_app_rules(
            &mut cfg,
            &[
                rule("p.exe", "proxy"),
                rule("b.exe", "block"),
                rule("d.exe", "direct"),
            ],
        );
        let rules = cfg["route"]["rules"].as_array().unwrap();
        // [0]=sniff, [1]=hijack-dns, [2]=block, [3]=direct, [4]=proxy
        assert_eq!(rules[2]["action"], "reject");
        assert_eq!(rules[2]["process_name"][0], "b.exe");
        assert_eq!(rules[3]["outbound"], "direct");
        assert_eq!(rules[3]["process_name"][0], "d.exe");
        assert_eq!(rules[4]["outbound"], "proxy");
        assert_eq!(rules[4]["process_name"][0], "p.exe");
    }

    #[test]
    fn apply_app_rules_skips_empty_exe() {
        let entry = vless_entry();
        let mut cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None)
            .unwrap()
            .json;
        apply_app_rules(
            &mut cfg,
            &[
                rule("", "proxy"),       // пустой → skip
                rule("   ", "proxy"),    // только пробелы → skip
                rule("real.exe", "proxy"),
            ],
        );
        let rules = cfg["route"]["rules"].as_array().unwrap();
        let proxy_rule = rules
            .iter()
            .find(|r| r.get("process_name").is_some() && r["outbound"] == "proxy")
            .unwrap();
        let names = proxy_rule["process_name"].as_array().unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0], "real.exe");
    }

    #[test]
    fn apply_app_rules_unknown_action_is_ignored() {
        let entry = vless_entry();
        let mut cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None)
            .unwrap()
            .json;
        let before = cfg["route"]["rules"].as_array().unwrap().len();
        apply_app_rules(&mut cfg, &[rule("foo.exe", "wat")]);
        // Никаких правил не добавилось — unknown action отфильтрован,
        // итоговый массив пустой → no-op.
        assert_eq!(cfg["route"]["rules"].as_array().unwrap().len(), before);
    }

    #[test]
    fn apply_app_rules_works_on_minimal_config_without_route() {
        // Defensive: если у config'а нет route-объекта — apply_app_rules
        // не должна паниковать, должна создать route.rules сама.
        let mut cfg = json!({});
        apply_app_rules(&mut cfg, &[rule("x.exe", "proxy")]);
        let rules = cfg["route"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["process_name"][0], "x.exe");
        assert_eq!(rules[0]["outbound"], "proxy");
    }

    // ─── Mux (multiplexing) ──────────────────────────────────────────────

    #[test]
    fn mux_disabled_does_not_add_field() {
        let entry = vless_entry();
        let mux = MuxOptions {
            enabled: false,
            protocol: "smux".to_string(),
            max_streams: 8,
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, Some(&mux))
            .unwrap();
        let proxy = cfg.json["outbounds"]
            .as_array().unwrap().iter()
            .find(|o| o["tag"] == "proxy").unwrap();
        assert!(proxy.get("multiplex").is_none());
    }

    #[test]
    fn mux_none_does_not_add_field() {
        let entry = vless_entry();
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None)
            .unwrap();
        let proxy = cfg.json["outbounds"]
            .as_array().unwrap().iter()
            .find(|o| o["tag"] == "proxy").unwrap();
        assert!(proxy.get("multiplex").is_none());
    }

    #[test]
    fn mux_enabled_adds_smux_with_max_streams() {
        let entry = vless_entry();
        let mux = MuxOptions {
            enabled: true,
            protocol: "smux".to_string(),
            max_streams: 8,
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, Some(&mux))
            .unwrap();
        let proxy = cfg.json["outbounds"]
            .as_array().unwrap().iter()
            .find(|o| o["tag"] == "proxy").unwrap();
        let m = proxy.get("multiplex").expect("multiplex должен быть");
        assert_eq!(m["enabled"], true);
        assert_eq!(m["protocol"], "smux");
        assert_eq!(m["max_streams"], 8);
    }

    #[test]
    fn mux_max_streams_zero_omits_field() {
        let entry = vless_entry();
        let mux = MuxOptions {
            enabled: true,
            protocol: "yamux".to_string(),
            max_streams: 0,
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, Some(&mux))
            .unwrap();
        let proxy = cfg.json["outbounds"]
            .as_array().unwrap().iter()
            .find(|o| o["tag"] == "proxy").unwrap();
        let m = proxy.get("multiplex").unwrap();
        assert_eq!(m["enabled"], true);
        assert_eq!(m["protocol"], "yamux");
        assert!(m.get("max_streams").is_none(), "max_streams=0 → поле опускается");
    }

    #[test]
    fn mux_empty_protocol_defaults_to_smux() {
        let entry = vless_entry();
        let mux = MuxOptions {
            enabled: true,
            protocol: "".to_string(),
            max_streams: 4,
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, Some(&mux))
            .unwrap();
        let proxy = cfg.json["outbounds"]
            .as_array().unwrap().iter()
            .find(|o| o["tag"] == "proxy").unwrap();
        assert_eq!(proxy["multiplex"]["protocol"], "smux");
    }

    #[test]
    fn mux_ignored_for_hysteria2() {
        // hy2 имеет свой stream multiplexing над QUIC — наш mux не нужен.
        let entry = ProxyEntry {
            name: "hy2".to_string(),
            protocol: "hysteria2".to_string(),
            server: "h.example.com".to_string(),
            port: 443,
            raw: json!({ "password": "pass", "sni": "h.example.com" }),
            engine_compat: vec!["sing-box".to_string()],
        };
        let mux = MuxOptions {
            enabled: true,
            protocol: "smux".to_string(),
            max_streams: 8,
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, Some(&mux))
            .unwrap();
        let proxy = cfg.json["outbounds"]
            .as_array().unwrap().iter()
            .find(|o| o["tag"] == "proxy").unwrap();
        assert!(proxy.get("multiplex").is_none(), "mux не применяется к hy2");
    }

    #[test]
    fn mux_applied_to_trojan_and_shadowsocks() {
        let mux = MuxOptions {
            enabled: true,
            protocol: "smux".to_string(),
            max_streams: 4,
        };
        for (proto, raw) in [
            ("trojan", json!({ "password": "pwd", "sni": "t.example.com" })),
            ("shadowsocks", json!({ "method": "aes-256-gcm", "password": "pwd" })),
        ] {
            let entry = ProxyEntry {
                name: format!("{proto}-test"),
                protocol: proto.to_string(),
                server: format!("{proto}.example.com"),
                port: 443,
                raw,
                engine_compat: vec!["sing-box".to_string()],
            };
            let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, Some(&mux))
                .unwrap();
            let proxy = cfg.json["outbounds"]
                .as_array().unwrap().iter()
                .find(|o| o["tag"] == "proxy").unwrap();
            assert!(
                proxy.get("multiplex").is_some(),
                "mux должен применяться к {proto}"
            );
        }
    }

    #[test]
    fn build_vmess_with_ws_transport() {
        let entry = ProxyEntry {
            name: "test".to_string(),
            protocol: "vmess".to_string(),
            server: "vmess.example.com".to_string(),
            port: 8080,
            raw: json!({
                "id": "uuid-abc",
                "aid": 0,
                "scy": "auto",
                "net": "ws",
                "tls": "tls",
                "sni": "vmess.example.com",
                "path": "/path",
                "host": "vmess.example.com",
            }),
            engine_compat: vec!["sing-box".to_string()],
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None).unwrap();
        let proxy = cfg.json["outbounds"]
            .as_array().unwrap().iter()
            .find(|o| o["tag"] == "proxy").unwrap();
        assert_eq!(proxy["type"], "vmess");
        assert_eq!(proxy["uuid"], "uuid-abc");
        assert_eq!(proxy["alter_id"], 0);
        assert_eq!(proxy["transport"]["type"], "ws");
        assert_eq!(proxy["transport"]["path"], "/path");
        assert_eq!(proxy["transport"]["headers"]["Host"], "vmess.example.com");
        assert_eq!(proxy["tls"]["enabled"], true);
    }

    #[test]
    fn build_trojan_with_grpc_transport() {
        let entry = ProxyEntry {
            name: "test".to_string(),
            protocol: "trojan".to_string(),
            server: "trojan.example.com".to_string(),
            port: 443,
            raw: json!({
                "password": "pass123",
                "type": "grpc",
                "security": "tls",
                "sni": "trojan.example.com",
                "serviceName": "trojansvc",
            }),
            engine_compat: vec!["sing-box".to_string()],
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None).unwrap();
        let proxy = cfg.json["outbounds"]
            .as_array().unwrap().iter()
            .find(|o| o["tag"] == "proxy").unwrap();
        assert_eq!(proxy["type"], "trojan");
        assert_eq!(proxy["password"], "pass123");
        assert_eq!(proxy["transport"]["type"], "grpc");
        assert_eq!(proxy["transport"]["service_name"], "trojansvc");
    }

    #[test]
    fn build_vless_with_xhttp_transport_bails() {
        // XHTTP transport НЕ поддерживается upstream sing-box. build()
        // должен bail'нуться с понятным сообщением.
        let entry = ProxyEntry {
            name: "test-xhttp".to_string(),
            protocol: "vless".to_string(),
            server: "x.example.com".to_string(),
            port: 443,
            raw: json!({
                "uuid": "12345678-1234-1234-1234-123456789012",
                "type": "xhttp",
                "path": "/p",
            }),
            engine_compat: vec!["sing-box".to_string()],
        };
        let result = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None);
        assert!(result.is_err());
        let msg = format!("{:#}", result.err().unwrap());
        assert!(msg.contains("XHTTP"), "ожидаем понятное сообщение про XHTTP: {msg}");
        assert!(msg.contains("test-xhttp"), "сообщение должно включать имя сервера: {msg}");
    }

    #[test]
    fn convert_xray_json_skips_xhttp_in_balancer() {
        // Balancer с mixed transports: 2 vless+tcp + 1 vless+xhttp.
        // Должны успешно сконвертить 2 рабочих и пропустить xhttp с warning.
        let xray = json!({
            "outbounds": [
                {"tag": "proxy", "protocol": "vless",
                    "settings": {"vnext": [{"address": "ok1.example.com", "port": 443,
                        "users": [{"id": "11111111-1111-1111-1111-111111111111"}]}]},
                    "streamSettings": {"network": "tcp"}},
                {"tag": "proxy-2", "protocol": "vless",
                    "settings": {"vnext": [{"address": "ok2.example.com", "port": 443,
                        "users": [{"id": "22222222-2222-2222-2222-222222222222"}]}]},
                    "streamSettings": {"network": "tcp"}},
                {"tag": "proxy-3", "protocol": "vless",
                    "settings": {"vnext": [{"address": "xhttp.example.com", "port": 443,
                        "users": [{"id": "33333333-3333-3333-3333-333333333333"}]}]},
                    "streamSettings": {"network": "xhttp", "xhttpSettings": {"path": "/p"}}},
                {"tag": "direct", "protocol": "freedom"}
            ]
        });
        let opts = ConvertOptions {
            socks_port: 30000, http_port: 30000, listen: "127.0.0.1",
            tun_mode: false, tun_options: None, anti_dpi: None, socks_auth: None,
        };
        let sb = convert_xray_json_to_singbox(&xray, "mixed", &opts).unwrap();
        let outbounds = sb["outbounds"].as_array().unwrap();
        // proxy и proxy-2 — успешно сконвертированы
        assert!(outbounds.iter().any(|o| o["tag"] == "proxy" && o["server"] == "ok1.example.com"));
        assert!(outbounds.iter().any(|o| o["tag"] == "proxy-2" && o["server"] == "ok2.example.com"));
        // proxy-3 (xhttp) — пропущен
        assert!(!outbounds.iter().any(|o| o["tag"] == "proxy-3"));
    }

    #[test]
    fn convert_xray_json_all_xhttp_bails() {
        // Если ВСЕ outbound'ы на xhttp — нечего конвертировать, bail.
        let xray = json!({
            "outbounds": [
                {"tag": "p1", "protocol": "vless",
                    "settings": {"vnext": [{"address": "x.example.com", "port": 443,
                        "users": [{"id": "11111111-1111-1111-1111-111111111111"}]}]},
                    "streamSettings": {"network": "xhttp"}},
                {"tag": "direct", "protocol": "freedom"}
            ]
        });
        let opts = ConvertOptions {
            socks_port: 30000, http_port: 30000, listen: "127.0.0.1",
            tun_mode: false, tun_options: None, anti_dpi: None, socks_auth: None,
        };
        let err = convert_xray_json_to_singbox(&xray, "all-xhttp", &opts).unwrap_err();
        assert!(err.to_string().contains("XHTTP") || err.to_string().contains("unsupported"));
    }

    #[test]
    fn build_shadowsocks() {
        let entry = ProxyEntry {
            name: "test".to_string(),
            protocol: "ss".to_string(),
            server: "ss.example.com".to_string(),
            port: 8388,
            raw: json!({
                "cipher": "aes-256-gcm",
                "password": "ss-pass",
            }),
            engine_compat: vec!["sing-box".to_string()],
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None).unwrap();
        let proxy = cfg.json["outbounds"]
            .as_array().unwrap().iter()
            .find(|o| o["tag"] == "proxy").unwrap();
        assert_eq!(proxy["type"], "shadowsocks");
        assert_eq!(proxy["method"], "aes-256-gcm");
        assert_eq!(proxy["password"], "ss-pass");
    }

    #[test]
    fn build_hysteria2_with_obfs_salamander() {
        let entry = ProxyEntry {
            name: "test".to_string(),
            protocol: "hysteria2".to_string(),
            server: "hy2.example.com".to_string(),
            port: 443,
            raw: json!({
                "password": "hy2-pass",
                "sni": "hy2.example.com",
                "obfs": "salamander",
                "obfs-password": "obfs-secret",
            }),
            engine_compat: vec!["sing-box".to_string()],
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None).unwrap();
        let proxy = cfg.json["outbounds"]
            .as_array().unwrap().iter()
            .find(|o| o["tag"] == "proxy").unwrap();
        assert_eq!(proxy["type"], "hysteria2");
        assert_eq!(proxy["password"], "hy2-pass");
        assert_eq!(proxy["obfs"]["type"], "salamander");
        assert_eq!(proxy["obfs"]["password"], "obfs-secret");
        assert_eq!(proxy["tls"]["alpn"][0], "h3");
    }

    #[test]
    fn build_tuic() {
        let entry = ProxyEntry {
            name: "test".to_string(),
            protocol: "tuic".to_string(),
            server: "tuic.example.com".to_string(),
            port: 443,
            raw: json!({
                "uuid": "tuic-uuid",
                "password": "tuic-pass",
                "sni": "tuic.example.com",
                "congestion_control": "bbr",
                "udp_relay_mode": "native",
                "alpn": "h3",
            }),
            engine_compat: vec!["sing-box".to_string()],
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None).unwrap();
        let proxy = cfg.json["outbounds"]
            .as_array().unwrap().iter()
            .find(|o| o["tag"] == "proxy").unwrap();
        assert_eq!(proxy["type"], "tuic");
        assert_eq!(proxy["uuid"], "tuic-uuid");
        assert_eq!(proxy["congestion_control"], "bbr");
        assert_eq!(proxy["tls"]["alpn"][0], "h3");
    }

    #[test]
    fn build_wireguard_uses_endpoints() {
        let entry = ProxyEntry {
            name: "test".to_string(),
            protocol: "wireguard".to_string(),
            server: "wg.example.com".to_string(),
            port: 51820,
            raw: json!({
                "private-key": "PRIVKEY",
                "publickey": "PUBKEY",
                "address": "10.0.0.2/32",
                "mtu": 1420,
            }),
            engine_compat: vec!["sing-box".to_string()],
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None).unwrap();
        // proxy outbound заменён на selector
        let proxy = cfg.json["outbounds"]
            .as_array().unwrap().iter()
            .find(|o| o["tag"] == "proxy").unwrap();
        assert_eq!(proxy["type"], "selector");
        assert_eq!(proxy["default"], "proxy-wg");

        let endpoints = cfg.json["endpoints"].as_array().unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0]["type"], "wireguard");
        assert_eq!(endpoints[0]["tag"], "proxy-wg");
        assert_eq!(endpoints[0]["private_key"], "PRIVKEY");
        assert_eq!(endpoints[0]["peers"][0]["address"], "wg.example.com");
        assert_eq!(endpoints[0]["peers"][0]["port"], 51820);
    }

    #[test]
    fn build_socks_with_auth() {
        let entry = ProxyEntry {
            name: "test".to_string(),
            protocol: "socks".to_string(),
            server: "socks.example.com".to_string(),
            port: 1080,
            raw: json!({
                "username": "user",
                "password": "pass",
            }),
            engine_compat: vec!["sing-box".to_string()],
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None).unwrap();
        let proxy = cfg.json["outbounds"]
            .as_array().unwrap().iter()
            .find(|o| o["tag"] == "proxy").unwrap();
        assert_eq!(proxy["type"], "socks");
        assert_eq!(proxy["username"], "user");
        assert_eq!(proxy["password"], "pass");
    }

    #[test]
    fn build_anti_dpi_fragmentation_sets_tls_fragment_flag() {
        // sing-box upstream поддерживает только булевый tls.fragment,
        // без тонкой настройки размера/задержки.
        let entry = vless_entry();
        let ad = AntiDpiOptions {
            fragmentation: true,
            fragmentation_packets: "tlshello".to_string(),
            fragmentation_length: "10-20".to_string(),
            fragmentation_interval: "10-20".to_string(),
            ..Default::default()
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, Some(&ad), None, None).unwrap();
        let proxy = cfg.json["outbounds"]
            .as_array().unwrap().iter()
            .find(|o| o["tag"] == "proxy").unwrap();
        assert_eq!(proxy["tls"]["fragment"], true);
    }

    #[test]
    fn build_anti_dpi_fragmentation_no_tls_outbound_is_noop() {
        // Если outbound без TLS (например plain SOCKS5) — fragment просто
        // игнорируется, никаких лишних полей.
        let entry = ProxyEntry {
            name: "socks".to_string(),
            protocol: "socks".to_string(),
            server: "s.example.com".to_string(),
            port: 1080,
            raw: json!({}),
            engine_compat: vec!["sing-box".to_string()],
        };
        let ad = AntiDpiOptions {
            fragmentation: true,
            ..Default::default()
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, Some(&ad), None, None).unwrap();
        let proxy = cfg.json["outbounds"]
            .as_array().unwrap().iter()
            .find(|o| o["tag"] == "proxy").unwrap();
        assert!(proxy.get("tls").is_none());
    }

    #[test]
    fn build_anti_dpi_noises_does_not_add_unsupported_field() {
        // Upstream sing-box не поддерживает noises — поле НЕ должно
        // попадать в outbound (иначе sing-box check ругнётся).
        let entry = ProxyEntry {
            name: "hy2".to_string(),
            protocol: "hysteria2".to_string(),
            server: "hy2.example.com".to_string(),
            port: 443,
            raw: json!({"password": "p", "sni": "hy2.example.com"}),
            engine_compat: vec!["sing-box".to_string()],
        };
        let ad = AntiDpiOptions {
            noises: true,
            noises_type: "rand".to_string(),
            noises_packet: "50-100".to_string(),
            noises_delay: "10-20".to_string(),
            ..Default::default()
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, Some(&ad), None, None).unwrap();
        let proxy = cfg.json["outbounds"]
            .as_array().unwrap().iter()
            .find(|o| o["tag"] == "proxy").unwrap();
        assert!(proxy.get("udp_noises").is_none());
    }

    #[test]
    fn build_anti_dpi_doh_resolve() {
        let entry = vless_entry();
        let ad = AntiDpiOptions {
            server_resolve: true,
            server_resolve_doh: "https://cloudflare-dns.com/dns-query".to_string(),
            server_resolve_bootstrap: "1.1.1.1".to_string(),
            ..Default::default()
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, Some(&ad), None, None).unwrap();
        let dns = &cfg.json["dns"];
        let servers = dns["servers"].as_array().unwrap();
        let doh = servers.iter().find(|s| s["tag"] == "doh").unwrap();
        assert_eq!(doh["type"], "https");
        assert_eq!(doh["server"], "cloudflare-dns.com");
        assert_eq!(doh["path"], "/dns-query");
        assert_eq!(doh["domain_resolver"], "bootstrap");

        let bootstrap = servers.iter().find(|s| s["tag"] == "bootstrap").unwrap();
        assert_eq!(bootstrap["type"], "udp");
        assert_eq!(bootstrap["server"], "1.1.1.1");

        let rules = dns["rules"].as_array().unwrap();
        assert!(rules.iter().any(|r| {
            r["domain"][0] == "de4.example.com" && r["server"] == "doh"
        }));
    }

    // ─── Тесты конверсии xray-json → sing-box ─────────────────────────────────

    #[test]
    fn convert_xray_json_regex_goes_to_domain_regex() {
        // Регрессия: подписки часто содержат regex-правила вида
        // `regexp:\.ru$`, `regexp:(^|\.)(yandex|...)\.com$` для split-routing
        // RU-доменов на direct. Они ДОЛЖНЫ попадать в `domain_regex` (PCRE
        // matching), а не в `domain_keyword` (substring matching) —
        // иначе `\.ru$` не сматчит ни одного домена и весь .ru-трафик
        // пойдёт через VPN.
        let xray = json!({
            "outbounds": [
                {"tag": "proxy", "protocol": "vless",
                    "settings": {"vnext": [{"address": "x.example.com", "port": 443,
                        "users": [{"id": "12345678-1234-1234-1234-123456789012"}]}]}},
                {"tag": "direct", "protocol": "freedom"}
            ],
            "routing": {"rules": [
                {"type": "field",
                 "domain": ["regexp:\\.ru$", "regexp:(^|\\.)(yandex|vk)\\.com$"],
                 "outboundTag": "direct"}
            ]}
        });
        let opts = ConvertOptions {
            socks_port: 30000, http_port: 30000, listen: "127.0.0.1",
            tun_mode: false, tun_options: None, anti_dpi: None, socks_auth: None,
        };
        let sb = convert_xray_json_to_singbox(&xray, "regex-test", &opts).unwrap();

        let rules = sb["route"]["rules"].as_array().unwrap();
        let regex_rule = rules.iter().find(|r| {
            r["action"] == "route" && r["outbound"] == "direct" && r.get("domain_regex").is_some()
        }).expect("должно быть domain_regex правило");
        let regex_arr: Vec<&str> = regex_rule["domain_regex"].as_array().unwrap()
            .iter().filter_map(|v| v.as_str()).collect();
        assert!(regex_arr.contains(&"\\.ru$"));
        assert!(regex_arr.contains(&"(^|\\.)(yandex|vk)\\.com$"));
        // Главное: НЕ в domain_keyword (там substring, regex не работает)
        assert!(regex_rule.get("domain_keyword").is_none());
    }

    #[test]
    fn convert_xray_json_skips_private_rule_sets() {
        // Регрессия: подписки часто содержат `geosite:private` /
        // `geoip:private` — для xray это валидно, но в sing-box нет
        // таких rule-set'ов в SagerNet/sing-geosite (404 при скачивании,
        // sing-box падает с FATAL). Должны их тихо пропускать —
        // приватные адреса покрыты встроенным private-direct правилом.
        let xray = json!({
            "outbounds": [
                {"tag": "proxy", "protocol": "vless",
                    "settings": {"vnext": [{"address": "x.example.com", "port": 443,
                        "users": [{"id": "12345678-1234-1234-1234-123456789012"}]}]}},
                {"tag": "direct", "protocol": "freedom"},
                {"tag": "block", "protocol": "blackhole"}
            ],
            "routing": {"rules": [
                {"type": "field", "domain": ["geosite:private"], "outboundTag": "direct"},
                {"type": "field", "ip": ["geoip:private"], "outboundTag": "direct"},
                {"type": "field", "domain": ["geosite:ru"], "outboundTag": "direct"}
            ]}
        });
        let opts = ConvertOptions {
            socks_port: 30000, http_port: 30000, listen: "127.0.0.1",
            tun_mode: false, tun_options: None, anti_dpi: None, socks_auth: None,
        };
        let sb = convert_xray_json_to_singbox(&xray, "private-test", &opts).unwrap();

        // route.rule_set содержит только geosite-ru (НЕ private)
        let entries = sb["route"]["rule_set"].as_array().unwrap();
        let tags: Vec<&str> = entries.iter()
            .filter_map(|e| e.get("tag").and_then(|v| v.as_str())).collect();
        assert!(tags.contains(&"geosite-ru"));
        assert!(!tags.iter().any(|t| t.contains("private")),
            "geosite-private/geoip-private не должны попадать в rule_set entries: {tags:?}");
    }

    #[test]
    fn convert_xray_json_marzban_vless_reality() {
        // Реальный пример Marzban: VLESS+REALITY+Vision с custom routing.rules
        // (geosite:ru → direct). Конверсия должна сохранить и outbound и rules.
        let xray = json!({
            "outbounds": [
                {
                    "tag": "proxy",
                    "protocol": "vless",
                    "settings": {
                        "vnext": [{
                            "address": "de4.nemefisto.online",
                            "port": 443,
                            "users": [{
                                "id": "abcd-1234",
                                "encryption": "none",
                                "flow": "xtls-rprx-vision"
                            }]
                        }]
                    },
                    "streamSettings": {
                        "network": "tcp",
                        "security": "reality",
                        "realitySettings": {
                            "serverName": "google.com",
                            "fingerprint": "chrome",
                            "publicKey": "PUBKEY",
                            "shortId": "01ab"
                        }
                    }
                },
                {"tag": "direct", "protocol": "freedom"},
                {"tag": "block", "protocol": "blackhole"}
            ],
            "routing": {
                "domainStrategy": "IPIfNonMatch",
                "rules": [
                    {"type": "field", "domain": ["geosite:ru", "habr.com"], "outboundTag": "direct"},
                    {"type": "field", "ip": ["geoip:ru"], "outboundTag": "direct"}
                ]
            }
        });

        let opts = ConvertOptions {
            socks_port: 30000,
            http_port: 30000,
            listen: "127.0.0.1",
            tun_mode: false,
            tun_options: None,
            anti_dpi: None,
            socks_auth: None,
        };

        let sb = convert_xray_json_to_singbox(&xray, "marzban-test", &opts).unwrap();

        // Outbound: vless+reality
        let outbounds = sb["outbounds"].as_array().unwrap();
        let proxy = outbounds.iter().find(|o| o["tag"] == "proxy").unwrap();
        assert_eq!(proxy["type"], "vless");
        assert_eq!(proxy["server"], "de4.nemefisto.online");
        assert_eq!(proxy["uuid"], "abcd-1234");
        assert_eq!(proxy["flow"], "xtls-rprx-vision");
        assert_eq!(proxy["tls"]["reality"]["public_key"], "PUBKEY");

        // Route rules: должна быть транслирована geosite:ru + habr.com → direct
        // Через rule_set + domain_suffix.
        let rules = sb["route"]["rules"].as_array().unwrap();
        let user_direct = rules.iter().find(|r| {
            r["action"] == "route"
                && r["outbound"] == "direct"
                && r.get("rule_set").is_some()
                && r.get("domain_suffix").is_some()
        }).expect("должно быть rule_set/domain_suffix-direct правило");
        let rule_set: Vec<&str> = user_direct["rule_set"].as_array().unwrap()
            .iter().filter_map(|v| v.as_str()).collect();
        assert!(rule_set.contains(&"geosite-ru"));
        assert_eq!(user_direct["domain_suffix"][0], "habr.com");

        // Второе правило: geoip:ru → direct (через rule_set)
        let geoip_direct = rules.iter().find(|r| {
            r["action"] == "route"
                && r["outbound"] == "direct"
                && r.get("rule_set").is_some()
                && r.get("domain_suffix").is_none()
        }).expect("должно быть geoip-only direct правило");
        let rule_set_geoip: Vec<&str> = geoip_direct["rule_set"].as_array().unwrap()
            .iter().filter_map(|v| v.as_str()).collect();
        assert!(rule_set_geoip.contains(&"geoip-ru"));

        // route.rule_set entries добавлены
        let rule_set_entries = sb["route"]["rule_set"].as_array().unwrap();
        let tags: Vec<&str> = rule_set_entries.iter()
            .filter_map(|e| e.get("tag").and_then(|v| v.as_str())).collect();
        assert!(tags.contains(&"geosite-ru"));
        assert!(tags.contains(&"geoip-ru"));
    }

    #[test]
    fn convert_xray_json_happ_balancer_full() {
        // Happ-style "fastest"-конфиг: первый outbound tag="proxy"
        // (default fallback), остальные proxy-2/3. Balancer "Auto_Europe"
        // selector=["proxy"] фильтрует по prefix-match. fallbackTag → default.
        // Rules: habr.com → outboundTag "proxy" (первый сервер), catch-all
        // через balancerTag "Auto_Europe".
        let xray = json!({
            "outbounds": [
                {"tag": "proxy", "protocol": "vless",
                    "settings": {"vnext": [{"address": "lv.example.com", "port": 443,
                        "users": [{"id": "11111111-1111-1111-1111-111111111111",
                            "encryption": "none", "flow": "xtls-rprx-vision"}]}]}},
                {"tag": "proxy-2", "protocol": "vless",
                    "settings": {"vnext": [{"address": "ne.example.com", "port": 443,
                        "users": [{"id": "11111111-1111-1111-1111-111111111111",
                            "encryption": "none", "flow": "xtls-rprx-vision"}]}]}},
                {"tag": "proxy-3", "protocol": "vless",
                    "settings": {"vnext": [{"address": "de.example.com", "port": 443,
                        "users": [{"id": "11111111-1111-1111-1111-111111111111",
                            "encryption": "none", "flow": "xtls-rprx-vision"}]}]}},
                {"tag": "direct", "protocol": "freedom"},
                {"tag": "block", "protocol": "blackhole"}
            ],
            "routing": {"balancers": [
                {"tag": "Auto_Europe", "selector": ["proxy"], "fallbackTag": "proxy",
                    "strategy": {"type": "leastLoad"}}
            ], "rules": [
                {"type": "field", "domain": ["habr.com", "4pda.to"], "outboundTag": "proxy"},
                {"type": "field", "balancerTag": "Auto_Europe", "network": "tcp,udp"}
            ]}
        });
        let opts = ConvertOptions {
            socks_port: 30000, http_port: 30000, listen: "127.0.0.1",
            tun_mode: false, tun_options: None, anti_dpi: None, socks_auth: None,
        };
        let sb = convert_xray_json_to_singbox(&xray, "fastest", &opts).unwrap();

        let outbounds = sb["outbounds"].as_array().unwrap();
        // Оригинальные tags сохранены: proxy / proxy-2 / proxy-3
        assert!(outbounds.iter().any(|o| o["tag"] == "proxy" && o["type"] == "vless"
            && o["server"] == "lv.example.com"));
        assert!(outbounds.iter().any(|o| o["tag"] == "proxy-2" && o["server"] == "ne.example.com"));
        assert!(outbounds.iter().any(|o| o["tag"] == "proxy-3" && o["server"] == "de.example.com"));

        // Balancer "Auto_Europe" → urltest. selector=["proxy"] сматчил proxy/proxy-2/proxy-3.
        // fallbackTag="proxy" → переставлен в начало outbounds[] (sing-box urltest
        // использует первый при отсутствии test-data, что эмулирует xray fallbackTag).
        let urltest = outbounds.iter().find(|o| o["type"] == "urltest").unwrap();
        assert_eq!(urltest["tag"], "Auto_Europe");
        assert!(urltest.get("default").is_none(), "field 'default' не существует у urltest");
        let utl_obs: Vec<&str> = urltest["outbounds"].as_array().unwrap()
            .iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(utl_obs[0], "proxy", "fallbackTag должен быть первым");
        assert!(utl_obs.contains(&"proxy-2"));
        assert!(utl_obs.contains(&"proxy-3"));

        // habr.com rule → outbound: "proxy" (первый сервер, не balancer)
        let rules = sb["route"]["rules"].as_array().unwrap();
        let habr_rule = rules.iter().find(|r| {
            r.get("domain_suffix").and_then(|v| v.as_array())
                .map(|a| a.iter().any(|x| x.as_str() == Some("habr.com"))).unwrap_or(false)
        }).expect("habr rule должен быть");
        assert_eq!(habr_rule["outbound"], "proxy");

        // Catch-all (balancerTag) → route.final = "Auto_Europe"
        assert_eq!(sb["route"]["final"], "Auto_Europe");
    }

    #[test]
    fn convert_xray_json_duplicate_tags_get_renamed() {
        // Если в подписке два outbound'а с одинаковым tag (артефакт
        // ошибки экспорта) — первый сохраняет оригинальный tag, второй
        // получает synthetic `proxy-N`.
        let xray = json!({
            "outbounds": [
                {"tag": "proxy", "protocol": "vless",
                    "settings": {"vnext": [{"address": "a.example.com", "port": 443,
                        "users": [{"id": "11111111-1111-1111-1111-111111111111"}]}]}},
                {"tag": "proxy", "protocol": "vless",
                    "settings": {"vnext": [{"address": "b.example.com", "port": 443,
                        "users": [{"id": "22222222-2222-2222-2222-222222222222"}]}]}},
                {"tag": "direct", "protocol": "freedom"}
            ]
        });
        let opts = ConvertOptions {
            socks_port: 30000, http_port: 30000, listen: "127.0.0.1",
            tun_mode: false, tun_options: None, anti_dpi: None, socks_auth: None,
        };
        let sb = convert_xray_json_to_singbox(&xray, "x", &opts).unwrap();

        let outbounds = sb["outbounds"].as_array().unwrap();
        // Первый сохраняет "proxy", второй синтетический "proxy-2"
        let proxy_tags: Vec<&str> = outbounds.iter()
            .filter(|o| o["type"] == "vless")
            .filter_map(|o| o["tag"].as_str())
            .collect();
        assert_eq!(proxy_tags, vec!["proxy", "proxy-2"]);
        // Synthetic urltest "auto" (нет xray-balancers, но >1 outbound)
        let urltest = outbounds.iter().find(|o| o["type"] == "urltest").unwrap();
        assert_eq!(urltest["tag"], "auto");
    }

    #[test]
    fn translate_balancer_tag_rule() {
        // Balancer-rules используют balancerTag вместо outboundTag.
        // tag matching: убедимся что выбирается balancerTag когда
        // outboundTag отсутствует (типичный Marzban balancer-rule).
        let rule = json!({
            "type": "field",
            "balancerTag": "proxy",
            "network": "tcp"
        });
        let translated = translate_xray_rule_to_singbox(&rule).unwrap();
        assert_eq!(translated["action"], "route");
        assert_eq!(translated["outbound"], "proxy");
        assert_eq!(translated["network"], "tcp");
    }

    #[test]
    fn convert_xray_json_no_vpn_outbound_fails() {
        let xray = json!({
            "outbounds": [
                {"tag": "direct", "protocol": "freedom"},
                {"tag": "block", "protocol": "blackhole"}
            ]
        });
        let opts = ConvertOptions {
            socks_port: 30000, http_port: 30000, listen: "127.0.0.1",
            tun_mode: false, tun_options: None, anti_dpi: None, socks_auth: None,
        };
        let err = convert_xray_json_to_singbox(&xray, "x", &opts).unwrap_err();
        assert!(err.to_string().contains("VPN-outbound"));
    }

    // ─── Тесты passthrough sing-box JSON ──────────────────────────────────────

    #[test]
    fn patch_singbox_keeps_outbounds_and_route() {
        // Remnawave-style минимальный sing-box JSON.
        let raw = json!({
            "log": { "level": "info" },
            "dns": { "servers": [{"type": "https", "tag": "g", "server": "8.8.8.8"}], "rules": [], "final": "g" },
            "inbounds": [
                {"type": "tun", "tag": "tun-in", "interface_name": "old"}
            ],
            "outbounds": [
                {
                    "type": "vless",
                    "tag": "proxy",
                    "server": "remna.example.com",
                    "server_port": 443,
                    "uuid": "remna-uuid"
                }
            ],
            "route": {
                "rules": [
                    {"action": "route", "geosite": ["ru"], "outbound": "proxy"}
                ],
                "final": "proxy"
            }
        });

        let opts = PatchOptions {
            socks_port: 30000,
            listen: "127.0.0.1",
            tun_mode: true,
            tun_options: Some(&TunOptions {
                interface_name: "nemefisto-1234".to_string(),
                address: "198.18.0.1/15".to_string(),
                mtu: 9000,
            }),
            socks_auth: Some(("u", "p")),
        };
        let patched = patch_singbox_json(raw, &opts).unwrap();

        // inbounds заменены на наши mixed+tun
        let inbounds = patched["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 2);
        assert!(inbounds.iter().any(|i| i["type"] == "mixed" && i["listen_port"] == 30000));
        assert!(inbounds.iter().any(|i| i["type"] == "tun" && i["interface_name"] == "nemefisto-1234"));

        // outbounds: оригинальный proxy + добавленный direct (block/dns не нужны)
        let outbounds = patched["outbounds"].as_array().unwrap();
        assert!(outbounds.iter().any(|o| o["type"] == "vless" && o["uuid"] == "remna-uuid"));
        assert!(outbounds.iter().any(|o| o["type"] == "direct" && o["tag"] == "direct"));
        assert!(!outbounds.iter().any(|o| o["type"] == "block"));
        assert!(!outbounds.iter().any(|o| o["type"] == "dns"));

        // route.rules: наши sniff/hijack-dns/private-direct добавлены В НАЧАЛО
        let rules = patched["route"]["rules"].as_array().unwrap();
        assert_eq!(rules[0]["action"], "sniff");
        assert_eq!(rules[1]["action"], "hijack-dns");
        assert_eq!(rules[2]["action"], "route");
        assert_eq!(rules[2]["outbound"], "direct");
        // Last rule — оригинальный
        assert_eq!(rules.last().unwrap()["geosite"][0], "ru");

        // auto_detect_interface обязательно
        assert_eq!(patched["route"]["auto_detect_interface"], true);
    }

    #[test]
    fn patch_singbox_strips_legacy_block_and_dns_outbounds() {
        // Если panel прислала legacy block/dns outbound'ы — мы их удаляем
        // (deprecated в 1.11+). И legacy `outbound: "block"` rule
        // транслируем в `action: "reject"`.
        let raw = json!({
            "outbounds": [
                {"type": "vless", "tag": "proxy", "server": "x", "server_port": 443, "uuid": "u"},
                {"type": "direct", "tag": "direct"},
                {"type": "block", "tag": "block"},
                {"type": "dns", "tag": "dns-out"}
            ],
            "route": {"rules": [
                {"domain": ["ads.example.com"], "outbound": "block"},
                {"protocol": "dns", "outbound": "dns-out"}
            ], "final": "proxy"}
        });
        let opts = PatchOptions {
            socks_port: 30000, listen: "127.0.0.1", tun_mode: false,
            tun_options: None, socks_auth: None,
        };
        let patched = patch_singbox_json(raw, &opts).unwrap();
        let outbounds = patched["outbounds"].as_array().unwrap();
        // ровно один direct, нет block/dns
        assert_eq!(outbounds.iter().filter(|o| o["tag"] == "direct").count(), 1);
        assert!(!outbounds.iter().any(|o| o["type"] == "block"));
        assert!(!outbounds.iter().any(|o| o["type"] == "dns"));

        // legacy outbound: "block" → action: "reject"
        let rules = patched["route"]["rules"].as_array().unwrap();
        let block_rule = rules.iter().find(|r| {
            r.get("domain").and_then(|d| d.get(0)).and_then(|v| v.as_str()) == Some("ads.example.com")
        }).expect("block rule");
        assert_eq!(block_rule["action"], "reject");
        assert!(block_rule.get("outbound").is_none());
    }

    // ─── 11.F apply_routing_profile ───────────────────────────────────────────

    #[test]
    fn apply_routing_profile_adds_direct_and_block() {
        let entry = vless_entry();
        let mut cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None).unwrap();
        let profile = RoutingProfile {
            name: "test".to_string(),
            global_proxy: BoolString(true),
            domain_strategy: super::super::routing_profile::DomainStrategy::IpIfNonMatch,
            direct_sites: vec!["geosite:ru".to_string(), "*.habr.com".to_string()],
            direct_ip: vec!["geoip:ru".to_string(), "10.0.0.0/8".to_string()],
            proxy_sites: vec![],
            proxy_ip: vec![],
            block_sites: vec!["geosite:category-ads-all".to_string()],
            block_ip: vec![],
            ..Default::default()
        };
        apply_routing_profile(&mut cfg.json, &profile);

        let rules = cfg.json["route"]["rules"].as_array().unwrap();

        // Block правило с geosite category-ads-all → action: "reject" + rule_set
        let block_rule = rules.iter().find(|r| r["action"] == "reject").unwrap();
        let block_rs: Vec<&str> = block_rule["rule_set"].as_array().unwrap()
            .iter().filter_map(|v| v.as_str()).collect();
        assert!(block_rs.contains(&"geosite-category-ads-all"));

        // Direct правила: встроенный private-direct + наш user-добавленный
        let direct_rules: Vec<_> = rules.iter()
            .filter(|r| r["action"] == "route" && r["outbound"] == "direct")
            .collect();
        let user_direct = direct_rules.iter()
            .find(|r| r.get("rule_set").is_some())
            .unwrap();
        let user_rs: Vec<&str> = user_direct["rule_set"].as_array().unwrap()
            .iter().filter_map(|v| v.as_str()).collect();
        assert!(user_rs.contains(&"geosite-ru"));
        assert!(user_rs.contains(&"geoip-ru"));
        assert_eq!(user_direct["domain_suffix"][0], "habr.com");
        assert_eq!(user_direct["ip_cidr"][0], "10.0.0.0/8");

        // route.rule_set entries должны включать geosite-ru, geosite-category-ads-all, geoip-ru
        let rule_set_entries = cfg.json["route"]["rule_set"].as_array().unwrap();
        let tags: Vec<&str> = rule_set_entries.iter()
            .filter_map(|e| e.get("tag").and_then(|v| v.as_str())).collect();
        assert!(tags.contains(&"geosite-ru"));
        assert!(tags.contains(&"geosite-category-ads-all"));
        assert!(tags.contains(&"geoip-ru"));
    }

    #[test]
    fn apply_routing_profile_global_proxy_false_changes_final() {
        let entry = vless_entry();
        let mut cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None).unwrap();
        let profile = RoutingProfile {
            name: "test".to_string(),
            global_proxy: BoolString(false),
            ..Default::default()
        };
        apply_routing_profile(&mut cfg.json, &profile);
        assert_eq!(cfg.json["route"]["final"], "direct");
    }

    // ─── Smoke-тесты с реальным sing-box.exe ──────────────────────────────────
    //
    // Эти тесты валидируют сгенерированный JSON через `sing-box check`.
    // Запуск явный: `cargo test --lib config::sing_box_config::tests::smoke -- --ignored`
    // Требуют `binaries/sing-box-x86_64-pc-windows-msvc.exe`.

    fn singbox_check(cfg: &Value, label: &str) {
        // Путь до бинаря — через CARGO_MANIFEST_DIR (src-tauri/).
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR не задан — тест должен запускаться через cargo test");
        let exe = std::path::PathBuf::from(manifest_dir)
            .join("binaries")
            .join("sing-box-x86_64-pc-windows-msvc.exe");
        if !exe.exists() {
            panic!("sing-box.exe не найден по пути {} — пропусти smoke-тест если он не нужен", exe.display());
        }

        let tmp = std::env::temp_dir().join(format!("nemefisto-singbox-check-{}.json", label));
        std::fs::write(&tmp, serde_json::to_string_pretty(cfg).unwrap()).unwrap();

        let out = std::process::Command::new(&exe)
            .args(["check", "-c"])
            .arg(&tmp)
            .output()
            .expect("не удалось запустить sing-box check");

        if !out.status.success() {
            eprintln!("--- config ({}) ---\n{}", label,
                serde_json::to_string_pretty(cfg).unwrap());
            eprintln!("--- sing-box stderr ---\n{}", String::from_utf8_lossy(&out.stderr));
            panic!("sing-box check не принял конфиг ({})", label);
        }
    }

    #[test]
    #[ignore]
    fn smoke_vless_reality_proxy_mode() {
        let entry = vless_entry();
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None).unwrap();
        singbox_check(&cfg.json, "vless-reality-proxy");
    }

    #[test]
    #[ignore]
    fn smoke_vless_reality_tun_mode() {
        let entry = vless_entry();
        let tun = TunOptions {
            interface_name: "nemefisto-test".to_string(),
            address: "198.18.0.1/15".to_string(),
            mtu: 9000,
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", true, Some(&tun), None, Some(("u","p")), None).unwrap();
        singbox_check(&cfg.json, "vless-reality-tun");
    }

    #[test]
    #[ignore]
    fn smoke_hysteria2_with_obfs_and_noises() {
        let entry = ProxyEntry {
            name: "hy2".to_string(),
            protocol: "hysteria2".to_string(),
            server: "hy2.example.com".to_string(),
            port: 443,
            raw: json!({"password": "p", "sni": "hy2.example.com",
                "obfs": "salamander", "obfs-password": "s"}),
            engine_compat: vec!["sing-box".to_string()],
        };
        let ad = AntiDpiOptions {
            noises: true,
            noises_type: "rand".to_string(),
            noises_packet: "50-100".to_string(),
            noises_delay: "10-20".to_string(),
            ..Default::default()
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, Some(&ad), None, None).unwrap();
        singbox_check(&cfg.json, "hy2-noises");
    }

    #[test]
    #[ignore]
    fn smoke_wireguard_endpoint() {
        // 32 байта base64-стандарт (44 символа с '=' padding) —
        // wireguard ключи требуют именно standard base64.
        let entry = ProxyEntry {
            name: "wg".to_string(),
            protocol: "wireguard".to_string(),
            server: "wg.example.com".to_string(),
            port: 51820,
            raw: json!({
                "private-key": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                "publickey": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                "address": "10.0.0.2/32",
                "mtu": 1420,
            }),
            engine_compat: vec!["sing-box".to_string()],
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None).unwrap();
        singbox_check(&cfg.json, "wireguard");
    }

    #[test]
    #[ignore]
    fn smoke_vless_grpc() {
        // VLESS+TLS+gRPC — типичный transport для Marzban-серверов.
        let entry = ProxyEntry {
            name: "grpc-test".to_string(),
            protocol: "vless".to_string(),
            server: "grpc.example.com".to_string(),
            port: 443,
            raw: json!({
                "uuid": "12345678-1234-1234-1234-123456789012",
                "security": "tls",
                "type": "grpc",
                "sni": "grpc.example.com",
                "fp": "chrome",
                "alpn": "h2",
                "serviceName": "GunService",
            }),
            engine_compat: vec!["sing-box".to_string()],
        };
        let cfg = build(&entry, 30000, 30000, "127.0.0.1", false, None, None, None, None).unwrap();
        singbox_check(&cfg.json, "vless-grpc");
    }

    #[test]
    #[ignore]
    fn smoke_marzban_balancer_xray_json() {
        // Реальный balancer-конфиг от Marzban "fastest"-эндпоинта:
        // несколько vless-outbound'ов + routing.balancers с leastPing.
        let xray = json!({
            "outbounds": [
                {"tag": "proxy-1", "protocol": "vless",
                    "settings": {"vnext": [{"address": "de1.example.com", "port": 443,
                        "users": [{"id": "11111111-1111-1111-1111-111111111111",
                            "encryption": "none", "flow": "xtls-rprx-vision"}]}]},
                    "streamSettings": {"network": "tcp", "security": "reality",
                        "realitySettings": {"serverName": "google.com", "fingerprint": "chrome",
                            "publicKey": TEST_REALITY_PUBKEY, "shortId": "01"}}},
                {"tag": "proxy-2", "protocol": "vless",
                    "settings": {"vnext": [{"address": "nl1.example.com", "port": 443,
                        "users": [{"id": "22222222-2222-2222-2222-222222222222",
                            "encryption": "none", "flow": "xtls-rprx-vision"}]}]},
                    "streamSettings": {"network": "tcp", "security": "reality",
                        "realitySettings": {"serverName": "google.com", "fingerprint": "chrome",
                            "publicKey": TEST_REALITY_PUBKEY, "shortId": "02"}}},
                {"tag": "direct", "protocol": "freedom"},
                {"tag": "block", "protocol": "blackhole"}
            ],
            "routing": {"balancers": [
                {"tag": "proxy", "selector": ["proxy-"], "strategy": {"type": "leastPing"}}
            ], "rules": [
                {"type": "field", "balancerTag": "proxy", "network": "tcp"},
                {"type": "field", "domain": ["geosite:ru", "habr.com"], "outboundTag": "direct"}
            ]}
        });
        let opts = ConvertOptions {
            socks_port: 30000, http_port: 30000, listen: "127.0.0.1",
            tun_mode: false, tun_options: None, anti_dpi: None, socks_auth: None,
        };
        let cfg = convert_xray_json_to_singbox(&xray, "balancer", &opts).unwrap();
        singbox_check(&cfg, "marzban-balancer");
    }

    #[test]
    #[ignore]
    fn smoke_marzban_xray_json_converted() {
        let xray = json!({
            "outbounds": [
                {"tag": "proxy", "protocol": "vless",
                    "settings": {"vnext": [{"address": "de4.example.com", "port": 443,
                        "users": [{"id": "12345678-1234-1234-1234-123456789012",
                            "encryption": "none", "flow": "xtls-rprx-vision"}]}]},
                    "streamSettings": {"network": "tcp", "security": "reality",
                        "realitySettings": {"serverName": "google.com", "fingerprint": "chrome",
                            "publicKey": TEST_REALITY_PUBKEY,
                            "shortId": "01ab"}}},
                {"tag": "direct", "protocol": "freedom"},
                {"tag": "block", "protocol": "blackhole"}
            ],
            "routing": {"domainStrategy": "IPIfNonMatch", "rules": [
                {"type": "field", "domain": ["geosite:ru", "habr.com", "*.4pda.to"], "outboundTag": "direct"},
                {"type": "field", "ip": ["geoip:ru", "10.0.0.0/8"], "outboundTag": "direct"}
            ]}
        });
        let opts = ConvertOptions {
            socks_port: 30000, http_port: 30000, listen: "127.0.0.1",
            tun_mode: false, tun_options: None, anti_dpi: None, socks_auth: None,
        };
        let cfg = convert_xray_json_to_singbox(&xray, "marzban", &opts).unwrap();
        singbox_check(&cfg, "marzban-xray-converted");
    }

    #[test]
    #[ignore]
    fn smoke_remnawave_passthrough() {
        // Реалистичный Remnawave-формат: новый DNS (type/server), новый
        // route (action/rule_set), modern sing-box 1.13 conventions.
        let raw = json!({
            "log": {"level": "info"},
            "dns": {
                "servers": [
                    {"type": "https", "tag": "google", "server": "8.8.8.8", "domain_resolver": "local"},
                    {"type": "local", "tag": "local"}
                ],
                "rules": [],
                "final": "google",
                "strategy": "ipv4_only"
            },
            "inbounds": [{"type": "tun", "tag": "tun-in", "interface_name": "remna",
                "address": ["172.19.0.1/30"], "auto_route": true, "stack": "gvisor"}],
            "outbounds": [
                {"type": "vless", "tag": "proxy", "server": "remna.example.com",
                    "server_port": 443, "uuid": "12345678-1234-1234-1234-123456789012",
                    "flow": "xtls-rprx-vision",
                    "tls": {"enabled": true, "server_name": "google.com",
                        "reality": {"enabled": true,
                            "public_key": TEST_REALITY_PUBKEY,
                            "short_id": "0001"},
                        "utls": {"enabled": true, "fingerprint": "chrome"}}}
            ],
            "route": {
                "rules": [{"action": "route", "rule_set": ["geosite-ru"], "outbound": "direct"}],
                "rule_set": [{
                    "type": "remote",
                    "tag": "geosite-ru",
                    "format": "binary",
                    "url": "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-ru.srs",
                    "download_detour": "direct"
                }],
                "final": "proxy",
                "auto_detect_interface": true,
                "default_domain_resolver": {"server": "local"}
            }
        });
        let tun = TunOptions::default();
        let opts = PatchOptions {
            socks_port: 30000, listen: "127.0.0.1",
            tun_mode: true, tun_options: Some(&tun), socks_auth: Some(("u", "p"))
        };
        let cfg = patch_singbox_json(raw, &opts).unwrap();
        singbox_check(&cfg, "remna-passthrough");
    }
}
