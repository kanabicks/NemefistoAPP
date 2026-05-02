//! Tauri commands, доступные из фронтенда через `invoke`.

use std::path::PathBuf;

use serde::Serialize;
use tauri::State;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex as AsyncMutex;

/// Параметры активного kill-switch. Сохраняем при successful connect()
/// чтобы переиспользовать при live-toggle настройки kill-switch без
/// необходимости заново резолвить server_host и собирать app-paths.
#[derive(Clone, Debug)]
pub struct KillSwitchContext {
    pub server_ips: Vec<String>,
    pub allow_lan: bool,
    pub allow_app_paths: Vec<String>,
    pub block_dns: bool,
    pub allow_dns_ips: Vec<String>,
    pub strict_mode: bool,
    /// 0.1.3: TUN-режим? Сохраняется чтобы live-toggle re-apply искал
    /// WinTUN-адаптер через retry в helper'е.
    pub expect_tun: bool,
}

/// Tauri-state для контекста активного kill-switch. None = VPN не подключён.
pub struct KillSwitchState(pub AsyncMutex<Option<KillSwitchContext>>);

impl KillSwitchState {
    pub fn new() -> Self {
        Self(AsyncMutex::new(None))
    }
}

use crate::config::mihomo_config::AppRule;
use crate::config::sing_box_config::AntiDpiOptions;
use crate::config::subscription::{fetch_and_parse, SubscriptionMeta};
use crate::config::{mihomo_config, sing_box_config, HwidState, ProxyEntry, SubscriptionState};
use crate::platform;
use crate::vpn;
use crate::vpn::{find_free_port, ping_entry, random_high_port, MihomoState, SingBoxState};

// ─── Helper-функции для TUN-режима ────────────────────────────────────────────

/// Извлечь хост VPN-сервера из ProxyEntry. Используется для kill-switch
/// (резолв в IP перед включением WFP-фильтров) и для логов.
///
/// Логика повторяет `vpn::ping::extract_target`, возвращает только host
/// (без порта). Для `mihomo-profile` берём первую ноду из `raw["proxies"]`.
fn extract_server_host(entry: &ProxyEntry) -> Option<String> {
    if entry.protocol == "mihomo-profile" {
        return entry
            .raw
            .get("proxies")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.iter().find_map(|p| {
                p.get("server")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(String::from)
            }));
    }
    if entry.protocol != "xray-json" {
        if entry.server.is_empty() {
            return None;
        }
        return Some(entry.server.clone());
    }
    let outbounds = entry.raw.get("outbounds")?.as_array()?;
    for ob in outbounds {
        let tag = ob.get("tag").and_then(|v| v.as_str()).unwrap_or("");
        if !tag.starts_with("proxy") {
            continue;
        }
        let proto = ob.get("protocol").and_then(|v| v.as_str()).unwrap_or("");
        let settings = ob.get("settings")?;
        let host = match proto {
            "vless" | "vmess" => settings
                .get("vnext")?
                .as_array()?
                .first()?
                .get("address")?
                .as_str()?,
            "trojan" | "shadowsocks" => settings
                .get("servers")?
                .as_array()?
                .first()?
                .get("address")?
                .as_str()?,
            _ => continue,
        };
        return Some(host.to_string());
    }
    None
}

/// Дополнительные хосты VPN-серверов для bypass-route (mihomo-passthrough).
///
/// В full-mihomo подписке проксей бывает 10-20+. Пользователь может на
/// лету переключаться между ними через external-controller, и каждая —
/// отдельный удалённый хост. Если bypass добавлен только на primary
/// (`extract_server_host`), переключение на любую другую ноду сразу
/// заворачивает её исходящий трафик в TUN → петля.
///
/// Возвращает все хосты КРОМЕ primary (того, что вернул
/// Найти путь к sidecar-бинарю по короткому имени (`sing-box`/`mihomo`).
/// Используется для kill-switch allow-app-id (этап 13.D) — нам нужно
/// разрешить нашим VPN-движкам исходящий трафик.
///
/// Перебирает кандидаты в exe-dir / `binaries` / `resources` / dev
/// `target/{profile}/binaries`. `_app` пока не используется, но
/// зарезервирован под Tauri `app.path()` API.
fn resolve_sidecar_path(_app: &tauri::AppHandle, name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    let triplet = format!("{name}-x86_64-pc-windows-msvc.exe");
    let plain = format!("{name}.exe");

    let mut candidates: Vec<PathBuf> = vec![
        exe_dir.join(&triplet),
        exe_dir.join(&plain),
        exe_dir.join("binaries").join(&triplet),
        exe_dir.join("binaries").join(&plain),
        exe_dir.join("resources").join(&triplet),
        exe_dir.join("resources").join(&plain),
    ];
    if let Some(dev_root) = exe_dir.parent().and_then(|p| p.parent()) {
        candidates.push(dev_root.join("binaries").join(&triplet));
        candidates.push(dev_root.join("binaries").join(&plain));
    }

    candidates.into_iter().find(|c| c.is_file())
}

/// Сгенерировать замаскированное имя TUN-адаптера (этап 12.E).
/// Случайно выбирается шаблон + случайный суффикс. Это не криптостойкая
/// генерация — цель в том чтобы имя адаптера выглядело как обычный
/// системный сетевой интерфейс (детектится приложениями типа МАХ/ВК
/// через `GetAdaptersAddresses` по имени).
fn generate_masked_tun_name() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    // Дешёвая псевдослучайность из времени старта — для уникальности от
    // запуска к запуску её хватает, криптостойкость не требуется.
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let pattern = seed % 3;
    let n = 99 + (seed / 3) % 100; // 99..198
    match pattern {
        0 => format!("wlan{n}"),
        1 => format!("Local Area Connection {}", n - 99),
        _ => format!("Ethernet {}", n - 99),
    }
}

// ─── Результаты команд ────────────────────────────────────────────────────────

/// Возвращается фронтенду после успешного подключения.
///
/// `socks_username` / `socks_password` — заполнены только если включён
/// `auth: password` на SOCKS5 inbound (этап 9.G). Используется в LAN-
/// режиме чтобы UI показал креды для копирования; в TUN-режиме они
/// уже переданы в tun2socks и пользователю не нужны.
#[derive(Serialize)]
pub struct ConnectResult {
    pub socks_port: u16,
    pub http_port: u16,
    pub server_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub socks_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub socks_password: Option<String>,
}

// ─── Подписка ─────────────────────────────────────────────────────────────────

/// Результат загрузки подписки: список серверов + опциональные метаданные
/// из стандартных HTTP-заголовков (этап 8.C).
#[derive(Serialize)]
pub struct SubscriptionResult {
    pub servers: Vec<ProxyEntry>,
    pub meta: Option<SubscriptionMeta>,
}

/// Превью спарсенного и сгенерированного конфига сервера. Используется
/// диалогом «view config» (стрелочка справа на server-row) — пользователь
/// может посмотреть что именно мы получили из подписки и что подсунем
/// движку при connect.
#[derive(Serialize)]
pub struct ServerPreview {
    pub name: String,
    pub protocol: String,
    pub server: String,
    pub port: u16,
    pub engine_compat: Vec<String>,
    /// Сырой JSON/YAML того что пришло с подписки (отформатированно).
    pub raw: String,
    /// Сгенерированный sing-box-конфиг для этого entry (то, что
    /// будет реально записано в `sing-box-config.json` при connect).
    /// `None` если это `mihomo-profile` — там используется raw YAML
    /// напрямую.
    pub generated_singbox: Option<String>,
}

/// Сгенерировать sing-box-JSON для preview (без запуска). Вытащено
/// в отдельную функцию чтобы commands.rs не разбухал.
fn build_singbox_preview(entry: &ProxyEntry) -> Result<Option<String>, String> {
    use crate::config::sing_box_config::{
        build, convert_xray_json_to_singbox, patch_singbox_json, ConvertOptions, PatchOptions,
    };
    let value = if entry.protocol == "xray-json" {
        let opts = ConvertOptions {
            socks_port: 30000,
            http_port: 30000,
            listen: "127.0.0.1",
            tun_mode: false,
            tun_options: None,
            anti_dpi: None,
            socks_auth: None,
        };
        convert_xray_json_to_singbox(&entry.raw, &entry.name, &opts)
            .map_err(|e| format!("конверсия xray-json: {e:#}"))?
    } else if entry.protocol == "singbox-json" {
        let opts = PatchOptions {
            socks_port: 30000,
            listen: "127.0.0.1",
            tun_mode: false,
            tun_options: None,
            socks_auth: None,
        };
        patch_singbox_json(entry.raw.clone(), &opts)
            .map_err(|e| format!("патч sing-box JSON: {e:#}"))?
    } else if entry.protocol == "mihomo-profile" {
        // Для mihomo-profile нет sing-box-конфига — используется raw YAML.
        return Ok(None);
    } else {
        let cfg = build(entry, 30000, 30000, "127.0.0.1", false, None, None, None)
            .map_err(|e| e.to_string())?;
        cfg.json
    };
    Ok(Some(
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?,
    ))
}

/// Превью конфига сервера с указанным индексом — без запуска движка.
/// Используется UI-кнопкой «view config» (стрелочка справа на server-row).
#[tauri::command]
pub fn preview_server_config(
    server_index: usize,
    sub: State<'_, SubscriptionState>,
) -> Result<ServerPreview, String> {
    let entry = {
        let servers = sub.servers.lock().map_err(|e| e.to_string())?;
        servers
            .get(server_index)
            .cloned()
            .ok_or_else(|| format!("сервер #{server_index} не найден в списке"))?
    };

    // Raw — отдаём в человекочитаемом формате. Для mihomo-profile это
    // YAML-строка из raw.yaml; для всех остальных — JSON.
    let raw = if entry.protocol == "mihomo-profile" {
        entry
            .raw
            .get("yaml")
            .and_then(|v| v.as_str())
            .unwrap_or("(пустой YAML)")
            .to_string()
    } else {
        serde_json::to_string_pretty(&entry.raw).map_err(|e| e.to_string())?
    };

    let generated_singbox = build_singbox_preview(&entry)?;

    Ok(ServerPreview {
        name: entry.name,
        protocol: entry.protocol,
        server: entry.server,
        port: entry.port,
        engine_compat: entry.engine_compat,
        raw,
        generated_singbox,
    })
}

/// Скачать подписку по URL, распарсить и сохранить список серверов.
///
/// `hwid_override` — если задан и непустой, используется вместо локально
/// сгенерированного MachineGuid (нужен только для разработки / переноса
/// с другого клиента).
/// `user_agent` — позволяет переопределить дефолт `Happ/2.7.0`.
/// `send_hwid` — если false, заголовок `x-hwid` не отправляется.
#[tauri::command]
pub async fn fetch_subscription(
    url: String,
    hwid_override: Option<String>,
    user_agent: Option<String>,
    send_hwid: Option<bool>,
    hwid: State<'_, HwidState>,
    sub: State<'_, SubscriptionState>,
) -> Result<SubscriptionResult, String> {
    let effective_hwid = hwid_override
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&hwid.0);

    let ua = user_agent.unwrap_or_default();
    let send = send_hwid.unwrap_or(true);

    let (servers, meta) = fetch_and_parse(&url, effective_hwid, &ua, send)
        .await
        .map_err(|e| e.to_string())?;

    *sub.servers.lock().map_err(|e| e.to_string())? = servers.clone();
    *sub.meta.lock().map_err(|e| e.to_string())? = meta.clone();
    Ok(SubscriptionResult { servers, meta })
}

/// Вернуть закешированный список серверов без сетевого запроса.
#[tauri::command]
pub fn get_servers(sub: State<'_, SubscriptionState>) -> Vec<ProxyEntry> {
    sub.servers.lock().map(|g| g.clone()).unwrap_or_default()
}

/// Вернуть закешированные метаданные подписки (трафик, срок).
#[tauri::command]
pub fn get_subscription_meta(sub: State<'_, SubscriptionState>) -> Option<SubscriptionMeta> {
    sub.meta.lock().map(|g| g.clone()).unwrap_or(None)
}

// ─── Подключение ──────────────────────────────────────────────────────────────

/// Подключиться к серверу с указанным индексом в режиме `mode`.
///
/// `mode` = "proxy" — системный SOCKS5 + HTTP прокси через реестр.
/// `mode` = "tun"   — TUN-режим через helper-сервис + tun2socks.
/// `allow_lan` — если `Some(true)`, inbound слушает 0.0.0.0 вместо 127.0.0.1.
///
/// Автоматически находит свободные порты начиная с 1080/1087.
#[tauri::command]
pub async fn connect(
    server_index: usize,
    mode: String,
    engine: Option<String>,
    allow_lan: Option<bool>,
    anti_dpi: Option<AntiDpiOptions>,
    tun_masking: Option<bool>,
    kill_switch: Option<bool>,
    // 13.D step B: блокировка DNS-leak (UDP/TCP 53 кроме VPN-DNS).
    // По дефолту off — может ломать приложения в proxy-режиме.
    dns_leak_protection: Option<bool>,
    // 13.S strict mode для kill-switch: убирает общий allow-app для xray/mihomo,
    // оставляет только allow на server_ips. Direct outbound xray блокируется.
    kill_switch_strict: Option<bool>,
    // 13.Q: если активного routing-профиля нет — применять встроенный
    // минимальный RU-шаблон (geosite:ru → DIRECT, ads → BLOCK).
    auto_apply_minimal_ru_rules: Option<bool>,
    app_rules: Option<Vec<AppRule>>,
    app: tauri::AppHandle,
    mihomo: State<'_, MihomoState>,
    sing_box: State<'_, SingBoxState>,
    mihomo_api: State<'_, vpn::MihomoApiState>,
    sub: State<'_, SubscriptionState>,
    ks_ctx: State<'_, KillSwitchState>,
    routing_store: State<'_, crate::config::routing_store::RoutingStoreState>,
) -> Result<ConnectResult, String> {
    // Долг: TUN 15-секундная задержка первого запроса. Включаем
    // подробное timing-логирование connect-flow чтобы видеть где
    // именно gap. После накопления логов — оптимизируем узкое место
    // (warmup, helper round-trip, tun_start, и т.д.).
    let connect_start = std::time::Instant::now();
    let stamp = |label: &str| {
        let elapsed = connect_start.elapsed().as_millis();
        eprintln!("[connect-timing][+{elapsed}ms] {label}");
    };
    stamp("start");

    // Pre-flight self-healing: если в системе остались наши orphan'ы от
    // упавшей сессии (системный прокси указывает на наш диапазон портов
    // или есть half-default routes), молча чистим перед connect. Иначе
    // следующий xray встретит «сломанную» среду и сам сломается.
    if platform::proxy::is_proxy_pointing_to_us() {
        let _ = platform::proxy::force_clear_system_proxy();
        stamp("preflight: cleared orphan proxy");
    }

    // Клонируем ProxyEntry, чтобы сразу освободить lock на список серверов
    let entry = {
        let servers = sub.servers.lock().map_err(|e| e.to_string())?;
        servers.get(server_index).cloned().ok_or_else(|| {
            format!(
                "сервер #{server_index} не найден в списке (всего серверов: {}). \
                 обновите подписку и выберите сервер заново",
                servers.len()
            )
        })?
    };

    // sing-box миграция (0.1.2): движки только sing-box + mihomo.
    // Дефолт — sing-box (быстрый старт + nativeTUN). Legacy "xray"
    // engine из старого localStorage маппится в "sing-box" — sing-box
    // покрывает все xray-протоколы (vless/vmess/trojan/ss/hy2/wg/socks)
    // ПЛЮС TUIC. engine_compat массив тоже может содержать legacy
    // "xray" — считаем что эти entries совместимы с sing-box.
    let chosen_engine_raw = engine.as_deref().unwrap_or("sing-box");
    let chosen_engine = if chosen_engine_raw == "xray" {
        "sing-box"
    } else {
        chosen_engine_raw
    };
    let engine_ok = match chosen_engine {
        "sing-box" => entry
            .engine_compat
            .iter()
            .any(|e| e == "sing-box" || e == "xray"),
        "mihomo" => entry.engine_compat.iter().any(|e| e == "mihomo"),
        other => {
            return Err(format!(
                "неподдерживаемый движок: {other}; используйте sing-box или mihomo"
            ));
        }
    };
    if !engine_ok {
        return Err(format!(
            "сервер «{}» несовместим с движком {}; поддерживается: {}",
            entry.name,
            chosen_engine,
            entry.engine_compat.join(", ")
        ));
    }

    // sing-box миграция (0.1.2): mihomo+URI+TUN больше не поддерживается
    // (раньше работал через tun2proxy pipeline, который мы выпилили).
    // mihomo built-in TUN работает только когда подписка отдала full
    // mihomo-profile (с tun: enable в YAML). Для URI-серверов mihomo
    // в TUN-режиме — sidecar без TUN-туннеля, не имеет смысла.
    if chosen_engine == "mihomo"
        && mode == "tun"
        && entry.protocol != "mihomo-profile"
    {
        return Err(format!(
            "сервер «{}» (тип {}): Mihomo+TUN работает только для подписок \
             в формате mihomo-profile (full clash YAML). Решения:\n\
             • переключиться на движок sing-box (Settings → движок) — он \
               умеет TUN для всех протоколов\n\
             • или переключить режим на proxy (системный прокси)",
            entry.name, entry.protocol
        ));
    }

    // 9.H — рандомизация портов inbound. Старт с псевдослучайных значений
    // в диапазоне [30000, 60000) вместо фиксированных 1080/1087, чтобы
    // сторонний процесс на машине не мог дёшево детектнуть VPN-клиент
    // сканированием стандартных портов. См. https://habr.com/ru/news/1020902/.
    // У Mihomo один mixed-port на SOCKS5+HTTP, поэтому для него используем
    // одно и то же значение для обоих "портов" (всё равно один сокет).
    let default_socks = find_free_port(random_high_port());
    let _default_http = if chosen_engine == "mihomo" {
        default_socks
    } else {
        find_free_port(random_high_port())
    };
    let lan = allow_lan.unwrap_or(false);
    let listen = if lan { "0.0.0.0" } else { "127.0.0.1" };
    let tun_mode = mode == "tun";

    // 9.G — SOCKS5/HTTP inbound auth. Включаем для TUN-режима всегда
    // (защита от использования прокси посторонними процессами на машине)
    // и для LAN-режима (защита от любого устройства в Wi-Fi сети). В
    // loopback proxy-режиме оставляем noauth — Windows registry для
    // системного прокси не умеет user:pass@host:port, и браузеры будут
    // получать 407 auth challenge на каждый запрос.
    let socks_auth = if tun_mode || lan {
        let pass = uuid::Uuid::new_v4().to_string();
        Some(("nemefisto".to_string(), pass))
    } else {
        None
    };

    // Для TUN-режима получаем имя физического интерфейса. В Xray-конфиге
    // direct-outbound получит `streamSettings.sockopt.interface = <name>` —
    // на Windows это `IP_UNICAST_IF`, который форсит маршрутизацию сокета
    // через указанный интерфейс минуя routing-таблицу (то есть мимо TUN).
    // physic_iface больше не нужен — sing-box и mihomo-profile делают
    // auto_detect_interface сами в built-in TUN. Xray-ветка с
    // sockopt.interface удалена в 0.1.2.
    let _ = tun_mode;

    // sing-box миграция (0.1.2): разветвление по движку.
    // - sing-box (default): JSON-конфиг, mixed inbound (SOCKS5+HTTP) на
    //   одном порту, built-in TUN-inbound в TUN-режиме (helper SYSTEM-spawn).
    // - Mihomo: YAML с mixed-port; mihomo built-in TUN если
    //   подписка пришла как полный mihomo-profile (в URI-режиме TUN
    //   запрещён — см. валидацию выше).
    let (socks_port, http_port) = if chosen_engine == "sing-box" {
        let auth_pair = socks_auth
            .as_ref()
            .map(|(u, p)| (u.as_str(), p.as_str()));

        // 11.F + 13.Q: активный routing-профиль или встроенный минимальный
        // RU-шаблон. Для xray-json/singbox-json (custom config из подписки)
        // профиль НЕ применяем — у пользователя свои правила приоритетнее.
        let active_profile = routing_store
            .inner
            .lock()
            .ok()
            .and_then(|g| g.active().map(|e| e.profile.clone()))
            .or_else(|| {
                if auto_apply_minimal_ru_rules.unwrap_or(false) {
                    Some(crate::config::routing_profile::RoutingProfile::minimal_ru())
                } else {
                    None
                }
            });

        // Маппим AntiDpiOptions из xray-формата в sing-box-формат.
        // Большинство полей совпадают по семантике (имена идентичны),
        // sing-box просто игнорирует те что не поддерживает (см.
        // sing_box_config::apply_anti_dpi_to_outbound).
        let sb_anti_dpi = anti_dpi.as_ref().map(|a| sing_box_config::AntiDpiOptions {
            fragmentation: a.fragmentation,
            fragmentation_packets: a.fragmentation_packets.clone(),
            fragmentation_length: a.fragmentation_length.clone(),
            fragmentation_interval: a.fragmentation_interval.clone(),
            noises: a.noises,
            noises_type: a.noises_type.clone(),
            noises_packet: a.noises_packet.clone(),
            noises_delay: a.noises_delay.clone(),
            server_resolve: a.server_resolve,
            server_resolve_doh: a.server_resolve_doh.clone(),
            server_resolve_bootstrap: a.server_resolve_bootstrap.clone(),
        });

        // TUN-параметры (имя адаптера маскируется по 12.E если включено)
        let tun_options = if tun_mode {
            let interface_name = if tun_masking.unwrap_or(false) {
                generate_masked_tun_name()
            } else {
                format!("nemefisto-{}", std::process::id())
            };
            Some(sing_box_config::TunOptions {
                interface_name,
                address: "198.18.0.1/15".to_string(),
                mtu: 9000,
            })
        } else {
            None
        };

        // Генерим конфиг по типу подписки.
        let mut config_json = if entry.protocol == "xray-json" {
            // Marzban-style: конвертируем xray-JSON → sing-box JSON.
            let opts = sing_box_config::ConvertOptions {
                socks_port: default_socks,
                http_port: default_socks,
                listen,
                tun_mode,
                tun_options: tun_options.as_ref(),
                anti_dpi: sb_anti_dpi.as_ref(),
                socks_auth: auth_pair,
            };
            sing_box_config::convert_xray_json_to_singbox(&entry.raw, &entry.name, &opts)
                .map_err(|e| format!("конверсия xray-json → sing-box: {e:#}"))?
        } else if entry.protocol == "singbox-json" {
            // Remnawave passthrough: минимальные правки (наши inbounds,
            // auth, миграция legacy block/dns outbound'ов).
            let opts = sing_box_config::PatchOptions {
                socks_port: default_socks,
                listen,
                tun_mode,
                tun_options: tun_options.as_ref(),
                socks_auth: auth_pair,
            };
            sing_box_config::patch_singbox_json(entry.raw.clone(), &opts)
                .map_err(|e| format!("патч sing-box JSON: {e:#}"))?
        } else {
            // URI-парсер entries (vless/vmess/trojan/ss/hy2/tuic/wg/socks).
            let cfg = sing_box_config::build(
                &entry,
                default_socks,
                default_socks,
                listen,
                tun_mode,
                tun_options.as_ref(),
                sb_anti_dpi.as_ref(),
                auth_pair,
            )
            .map_err(|e| e.to_string())?;
            cfg.json
        };

        // 11.F + 13.Q: применяем активный routing-профиль (только для
        // URI-entries, у passthrough-конфигов свои правила).
        if entry.protocol != "xray-json" && entry.protocol != "singbox-json" {
            if let Some(p) = active_profile.as_ref() {
                sing_box_config::apply_routing_profile(&mut config_json, p);
            }
        }

        // Гасим другой движок если что-то осталось от прошлого сеанса.
        let _ = mihomo.stop();
        let _ = platform::helper_client::mihomo_stop().await;
        mihomo.mark_helper_spawned(false);

        // Сериализуем JSON в строку.
        let config_str = serde_json::to_string_pretty(&config_json)
            .map_err(|e| format!("сериализация sing-box-конфига: {e}"))?;

        if tun_mode {
            // Built-in TUN: helper SYSTEM-spawn (нужен админ для
            // CreateAdapter WinTUN). Конфиг и data-dir в ProgramData —
            // shared read+write для helper-SYSTEM и Tauri-user.
            let shared_dir = std::path::PathBuf::from(r"C:\ProgramData\NemefistoVPN");
            std::fs::create_dir_all(&shared_dir)
                .map_err(|e| format!("создание ProgramData/NemefistoVPN: {e}"))?;
            let config_path = shared_dir.join("sing-box-config.json");
            std::fs::write(&config_path, &config_str)
                .map_err(|e| format!("запись sing-box-config.json: {e}"))?;
            let exe_path = resolve_sidecar_path(&app, "sing-box")
                .ok_or_else(|| "sing-box binary не найден".to_string())?;
            let config_pstr = config_path.to_string_lossy().into_owned();
            let exe_pstr = exe_path.to_string_lossy().into_owned();
            let data_pstr = shared_dir.to_string_lossy().into_owned();

            if let Err(e) = platform::helper_bootstrap::ensure_running().await {
                return Err(format!("helper-сервис недоступен: {e}"));
            }
            // Гасим прошлый sing-box если что-то осталось
            let _ = platform::helper_client::singbox_stop().await;
            let _ = sing_box.stop();

            platform::helper_client::singbox_start(config_pstr, exe_pstr, data_pstr)
                .await
                .map_err(|e| format!("helper.singbox_start: {e}"))?;
            sing_box.mark_helper_spawned(true);
            *sing_box.mixed_port.lock().map_err(|e| format!("mutex: {e}"))? = default_socks;
            stamp("sing-box built-in TUN: spawned via helper");
        } else {
            // Proxy-режим: Tauri sidecar (user-level OK, нет CreateAdapter).
            sing_box.start_with_config(&app, &config_str, default_socks)?;
            stamp("sing-box: spawned via Tauri sidecar");
        }

        (default_socks, default_socks)
    } else if chosen_engine == "mihomo" {
        let auth_pair = socks_auth
            .as_ref()
            .map(|(u, p)| (u.as_str(), p.as_str()));
        // 8.D: per-process правила. Mihomo получает их через
        // PROCESS-NAME matcher. Xray-ветка ниже их игнорирует —
        // на Windows нет нативной поддержки в Xray (требует kernel-driver,
        // см. план 13.G WFP per-app routing).
        let rules_slice: &[AppRule] = app_rules.as_deref().unwrap_or(&[]);
        // 11.F + 13.Q: активный routing-профиль или встроенный
        // минимальный RU-шаблон (если включён toggle и активного нет).
        let active_profile = routing_store
            .inner
            .lock()
            .ok()
            .and_then(|g| g.active().map(|e| e.profile.clone()))
            .or_else(|| {
                if auto_apply_minimal_ru_rules.unwrap_or(false) {
                    Some(crate::config::routing_profile::RoutingProfile::minimal_ru())
                } else {
                    None
                }
            });
        // 8.F: full-mihomo-passthrough путь. Если в подписке прилетел
        // полный mihomo YAML с proxy-groups — используем его целиком,
        // патчем только наши inbound/auth/external-controller.
        // Иначе — старый путь сборки конфига из ProxyEntry.
        let controller_port = find_free_port(default_socks.saturating_add(1));
        let controller_secret = uuid::Uuid::new_v4().to_string();

        let cfg = if entry.protocol == "mihomo-profile" {
            // raw["yaml"] всегда есть для mihomo-profile (см. subscription.rs)
            let raw_yaml = entry
                .raw
                .get("yaml")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "mihomo-profile без raw.yaml".to_string())?;
            // 13.L: для mihomo-profile в TUN-режиме используем mihomo
            // built-in TUN — он сам управляет адаптером и обходит свой
            // же direct outbound. Без этого петли между half-routes
            // через TUN и mihomo's DIRECT неизбежны.
            //
            // В proxy-режиме (без TUN) — обычный SOCKS-server, наш
            // tun2socks pipeline не запускается.
            let use_builtin_tun = tun_mode;
            let patch = mihomo_config::FullYamlPatch {
                mixed_port: default_socks,
                listen,
                socks_auth: auth_pair,
                external_controller_port: controller_port,
                external_controller_secret: &controller_secret,
                app_rules: rules_slice,
                anti_dpi: anti_dpi.as_ref(),
                use_builtin_tun,
            };
            mihomo_config::patch_full_yaml(raw_yaml, &patch)
                .map_err(|e| format!("патч full-mihomo YAML: {e:#}"))?
        } else {
            mihomo_config::build(
                &entry,
                default_socks,
                listen,
                anti_dpi.as_ref(),
                auth_pair,
                rules_slice,
                active_profile.as_ref(),
            )
            .map_err(|e| e.to_string())?
        };
        // На всякий случай гасим sing-box если он был активен от прошлой
        // сессии на другом движке (не должно бывать, но дешёвая страховка).
        let _ = sing_box.stop();
        let _ = platform::helper_client::singbox_stop().await;
        sing_box.mark_helper_spawned(false);

        // 13.L: для built-in TUN запускаем mihomo через helper-сервис
        // (он SYSTEM, имеет права на CreateAdapter WinTUN). Tauri-main
        // как user-level не справится. Иначе — старый sidecar-spawn.
        let builtin_tun = entry.protocol == "mihomo-profile" && tun_mode;
        if builtin_tun {
            // Конфиг и data-dir в ProgramData — туда у обоих процессов
            // (helper-SYSTEM и Tauri-user) стандартный read+write.
            let shared_dir = std::path::PathBuf::from(r"C:\ProgramData\NemefistoVPN");
            std::fs::create_dir_all(&shared_dir)
                .map_err(|e| format!("создание ProgramData/NemefistoVPN: {e}"))?;
            let config_path = shared_dir.join("mihomo-config.yaml");
            std::fs::write(&config_path, &cfg.yaml)
                .map_err(|e| format!("запись mihomo-config.yaml: {e}"))?;
            let exe_path = resolve_sidecar_path(&app, "mihomo")
                .ok_or_else(|| "mihomo binary не найден".to_string())?;
            let config_str = config_path.to_string_lossy().into_owned();
            let exe_str = exe_path.to_string_lossy().into_owned();
            let data_str = shared_dir.to_string_lossy().into_owned();

            // Гарантируем что helper доступен и нужной версии
            if let Err(e) = platform::helper_bootstrap::ensure_running().await {
                return Err(format!("helper-сервис недоступен: {e}"));
            }
            // Перед стартом — гасим helper-mihomo и Tauri-mihomo если
            // что-то осталось от прошлой сессии.
            let _ = platform::helper_client::mihomo_stop().await;
            let _ = mihomo.stop();

            platform::helper_client::mihomo_start(config_str, exe_str, data_str)
                .await
                .map_err(|e| format!("helper.mihomo_start: {e}"))?;
            mihomo.mark_helper_spawned(true);
            // mixed_port запоминаем для is_xray_running и др.
            *mihomo.mixed_port.lock().map_err(|e| format!("mutex: {e}"))? = cfg.mixed_port;
            stamp("mihomo built-in TUN: spawned via helper");
        } else {
            mihomo.start_with_config(&app, &cfg.yaml, cfg.mixed_port)?;
        }

        // 8.F: сохраняем endpoint controller'а — UI достучится через
        // mihomo_proxies / mihomo_select_proxy / mihomo_delay_test.
        mihomo_api.set(vpn::ControllerEndpoint {
            host: "127.0.0.1".to_string(),
            port: controller_port,
            secret: controller_secret,
        });

        (cfg.mixed_port, cfg.mixed_port)
    } else {
        // sing-box миграция (0.1.2): xray-движок выпилен. Ветка
        // недостижима — chosen_engine validation выше пропускает только
        // sing-box и mihomo. Этот else существует только для
        // exhaustiveness — на случай добавления нового движка.
        unreachable!("неподдерживаемый движок прошёл валидацию: {chosen_engine}");
    };

    stamp("vpn engine started");

    match mode.as_str() {
        "proxy" => {
            platform::proxy::set_system_proxy(socks_port, http_port)
                .map_err(|e| e.to_string())?;
            stamp("system proxy set");
        }
        "tun" => {
            // sing-box миграция (0.1.2): TUN всегда через built-in
            // TUN-inbound движка. sing-box делает это всегда; Mihomo —
            // когда подписка приходит как полный mihomo-profile (full
            // YAML с tun: enable). Для других случаев Mihomo+TUN+URI
            // была проверка ранее в connect (bail с понятным сообщением).
            // Tun2proxy + helper.tun_start выпилены.
            stamp("tun: built-in TUN — движок сам поднимает WinTUN-адаптер");
        }
        other => {
            let _ = mihomo.stop();
            let _ = sing_box.stop();
            return Err(format!("неизвестный режим: {other}"));
        }
    }

    // 13.D — kill switch (настоящий WFP). Поднимаем ПОСЛЕ успешного
    // Xray/TUN — чтобы при ошибке connect не оставить пользователя с
    // заблокированным интернетом.
    if kill_switch.unwrap_or(false) {
        let server_host = extract_server_host(&entry).unwrap_or_else(|| entry.server.clone());

        // Резолвим server_host в IP-адреса ЗДЕСЬ — после включения kill-switch'а
        // DNS уйдёт через VPN-туннель, а его ещё нет (helper-сервис в SYSTEM
        // получит маршруты позже). Если IP в host_str — lookup_host вернёт
        // его как есть.
        let server_ips: Vec<String> =
            tokio::net::lookup_host(format!("{server_host}:0"))
                .await
                .map(|iter| iter.map(|sa| sa.ip().to_string()).collect())
                .unwrap_or_default();
        if server_ips.is_empty() {
            // Fallback — может быть literal IP без формата host:port.
            // Пробуем парсить напрямую.
            if server_host.parse::<std::net::IpAddr>().is_ok() {
                // ОК
            } else {
                let _ = mihomo.stop();
                let _ = sing_box.stop();
                let _ = platform::helper_client::mihomo_stop().await;
                let _ = platform::helper_client::singbox_stop().await;
                let _ = platform::proxy::clear_system_proxy();
                return Err(format!(
                    "kill switch: не удалось резолвить server_host={server_host}"
                ));
            }
        }
        let server_ips = if server_ips.is_empty() {
            vec![server_host.clone()]
        } else {
            server_ips
        };

        // Allow-list: пути к нашим sidecar-бинарям. Без них VPN-движок
        // не сможет соединиться с сервером (хоть IP и в whitelist —
        // FwpmGetAppIdFromFileName0 матчит ИМЕННО по path, не по
        // basename).
        //
        // Tauri 2 в dev-режиме запускает sidecar по triplet-имени
        // (`xray-x86_64-pc-windows-msvc.exe`), но нашeresolve может
        // найти plain (`xray.exe`) который существует рядом — тогда
        // path-mismatch и allow не сработает.
        // Решение: добавляем В АЛЛОУЛИСТ ОБА варианта по факту наличия.
        let mut allow_app_paths: Vec<String> = Vec::new();
        let mut push_if_exists = |p: PathBuf| {
            if p.is_file() {
                allow_app_paths.push(p.to_string_lossy().into_owned());
            }
        };
        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                // Все возможные кандидаты sidecar — добавим все которые
                // существуют. WFP игнорирует дубликаты с разными path
                // как «всё allow».
                for name in ["mihomo", "sing-box"] {
                    push_if_exists(exe_dir.join(format!("{name}.exe")));
                    push_if_exists(
                        exe_dir.join(format!("{name}-x86_64-pc-windows-msvc.exe")),
                    );
                    push_if_exists(exe_dir.join("binaries").join(format!("{name}.exe")));
                    push_if_exists(
                        exe_dir
                            .join("binaries")
                            .join(format!("{name}-x86_64-pc-windows-msvc.exe")),
                    );
                    // Dev: target/{profile}/.. → src-tauri/binaries/
                    if let Some(dev_root) = exe_dir.parent().and_then(|p| p.parent()) {
                        push_if_exists(
                            dev_root.join("binaries").join(format!("{name}.exe")),
                        );
                        push_if_exists(
                            dev_root
                                .join("binaries")
                                .join(format!("{name}-x86_64-pc-windows-msvc.exe")),
                        );
                    }
                }
                // Сам vpn-client.exe — родительский процесс. Tauri может
                // делать outbound (DNS-проверки leak-test, deep-link
                // регистрация, и т.д.), а также на некоторых системах
                // app-id наследуется от parent.
                push_if_exists(exe.clone());
                // helper.exe — не нужен для outbound, но добавим на
                // случай future telemetry.
                push_if_exists(exe_dir.join("nemefisto-helper.exe"));
            }
        }
        // Resolve-функции тоже подключим (на случай если выше что-то
        // упустили). Дедупликация ниже не нужна — WFP ОК с дубликатами.
        if let Some(p) = resolve_sidecar_path(&app, "sing-box") {
            push_if_exists(p);
        }
        if let Some(p) = resolve_sidecar_path(&app, "mihomo") {
            push_if_exists(p);
        }
        // Дедупликация по string чтобы не плодить identical filters.
        allow_app_paths.sort();
        allow_app_paths.dedup();

        // Гарантируем что helper-сервис запущен (если не активен TUN-режим
        // — у нас не было ensure_running).
        if !tun_mode {
            if let Err(e) = platform::helper_bootstrap::ensure_running().await {
                let _ = mihomo.stop();
                let _ = sing_box.stop();
                let _ = platform::helper_client::mihomo_stop().await;
                let _ = platform::helper_client::singbox_stop().await;
                let _ = platform::proxy::clear_system_proxy();
                return Err(format!("kill switch: helper-сервис недоступен: {e}"));
            }
        }
        // 13.D step B: DNS-leak protection. В TUN-режиме разрешаем
        // только VPN-DNS на TUN-gateway (198.18.0.1) — остальной :53
        // блокируется. В proxy-режиме `allow_dns_ips=[]` — пользователь
        // ОЧЕНЬ должен понимать что делает (приложения сломаются если
        // не используют системный прокси для DNS).
        let block_dns = dns_leak_protection.unwrap_or(false);
        let allow_dns_ips: Vec<String> = if block_dns && tun_mode {
            vec!["198.18.0.1".to_string()]
        } else {
            Vec::new()
        };
        let strict = kill_switch_strict.unwrap_or(false);

        if let Err(e) = platform::helper_client::kill_switch_enable(
            server_ips.clone(),
            lan,
            allow_app_paths.clone(),
            block_dns,
            allow_dns_ips.clone(),
            strict,
            tun_mode,
        )
        .await
        {
            // При ошибке откатываем всё — интернет НЕ должен оставаться
            // в полу-заблокированном состоянии.
            let _ = mihomo.stop();
            let _ = sing_box.stop();
            let _ = platform::helper_client::mihomo_stop().await;
            let _ = platform::helper_client::singbox_stop().await;
            let _ = platform::proxy::clear_system_proxy();
            return Err(format!("kill switch не поднялся: {e}"));
        }

        // Сохраняем контекст для live-toggle — пользователь может
        // переключать kill-switch в Settings без disconnect/connect.
        // `kill_switch_apply` команда читает это и заново применяет
        // / снимает фильтры с теми же параметрами.
        *ks_ctx.0.lock().await = Some(KillSwitchContext {
            server_ips,
            allow_lan: lan,
            allow_app_paths,
            block_dns,
            allow_dns_ips,
            strict_mode: strict,
            expect_tun: tun_mode,
        });
        stamp("kill_switch enabled");
    }

    stamp("connect done");

    // В UI возвращаем креды только в LAN-режиме — там клиенты должны
    // ввести их вручную. В TUN-режиме они уже переданы tun2socks; в
    // proxy-режиме их вообще нет (loopback noauth).
    let (resp_user, resp_pass) = if lan {
        match socks_auth {
            Some((u, p)) => (Some(u), Some(p)),
            None => (None, None),
        }
    } else {
        (None, None)
    };

    Ok(ConnectResult {
        socks_port,
        http_port,
        server_name: entry.name,
        socks_username: resp_user,
        socks_password: resp_pass,
    })
}

/// Отключиться: остановить TUN (если был активен), Xray, сбросить системный
/// прокси, выключить kill switch (если был активен).
///
/// Все операции выполняются независимо: ошибка одной не отменяет других.
/// `tun_stop` и `kill_switch_disable` идемпотентны — игнорируют
/// «не запущен» / «helper недоступен». Это важно: при отключении мы
/// должны гарантировать что интернет вернётся, даже если helper исчез.
#[tauri::command]
pub async fn disconnect(
    mihomo: State<'_, MihomoState>,
    sing_box: State<'_, SingBoxState>,
    mihomo_api: State<'_, vpn::MihomoApiState>,
    ks_ctx: State<'_, KillSwitchState>,
) -> Result<(), String> {
    // 1. TUN/built-in TUN — гасим оба helper-spawned движка.
    //    Все идемпотентны.
    let _ = platform::helper_client::mihomo_stop().await;
    mihomo.mark_helper_spawned(false);
    let _ = platform::helper_client::singbox_stop().await;
    sing_box.mark_helper_spawned(false);
    // 2. Kill switch — всегда вызываем, чтобы убрать остатки если
    //    включён был в прошлый сеанс (на случай если краш / повторный
    //    запуск). Helper тихо вернёт ok если он не был enabled.
    let _ = platform::helper_client::kill_switch_disable().await;
    // Очищаем контекст live-toggle — VPN больше не активен.
    *ks_ctx.0.lock().await = None;
    // 8.F: чистим mihomo controller endpoint — UI больше не должен
    // ходить в API мёртвого процесса.
    mihomo_api.clear();

    // 3. Оба движка (один не запущен — stop() для него no-op)
    //    + system proxy.
    let mihomo_err = mihomo.stop().err();
    let singbox_err = sing_box.stop().err();
    let proxy_err = platform::proxy::clear_system_proxy().err().map(|e| e.to_string());

    if let Some(e) = mihomo_err {
        return Err(e);
    }
    if let Some(e) = singbox_err {
        return Err(e);
    }
    if let Some(e) = proxy_err {
        return Err(e);
    }
    Ok(())
}

/// Запущен ли VPN-движок (sing-box или Mihomo) прямо сейчас.
///
/// Имя оставлено `is_xray_running` для совместимости с фронтом —
/// возвращает true если запущен **любой** из двух поддерживаемых
/// движков. Семантика «работает ли VPN», не привязка к конкретному ядру.
#[tauri::command]
pub fn is_xray_running(
    mihomo: State<'_, MihomoState>,
    sing_box: State<'_, SingBoxState>,
) -> bool {
    mihomo.is_running() || sing_box.is_running()
}

/// Обновить tray-icon под текущий VPN-статус (этап 13.A).
///
/// Фронт вызывает при каждом изменении `vpnStore.status`. Backend
/// меняет текст пункта «Подключить/Отключить» в меню трея и tooltip
/// иконки. Фронт также сообщает имя выбранного сервера и есть ли
/// вообще выбор — по этому решаем enabled-state кнопки.
#[tauri::command]
pub fn tray_set_status(
    status: String,
    server_name: Option<String>,
    has_selection: bool,
    app: tauri::AppHandle,
) -> Result<(), String> {
    platform::tray::set_status(&app, &status, server_name.as_deref(), has_selection)
}

// ─── Kill-switch (13.D) ─────────────────────────────────────────────────────

/// Heartbeat для kill-switch watchdog. Фронт зовёт каждые ~20 сек
/// пока vpn running и kill-switch включён. Helper использует это
/// чтобы понять «main жив» — иначе через 60 сек авто-disable фильтры.
/// Не падает если helper не отвечает — это не критично, при
/// нескольких подряд misses сработает watchdog.
#[tauri::command]
pub async fn kill_switch_heartbeat() -> Result<(), String> {
    platform::helper_client::kill_switch_heartbeat()
        .await
        .map_err(|e| e.to_string())
}

/// Аварийный сброс WFP kill-switch. Удаляет все наши фильтры через
/// helper. UI-кнопка «авария» в Settings — для случаев когда
/// kill-switch завис и интернет заблокирован. Идемпотентно: если
/// ничего нет, helper вернёт Ok тихо.
#[tauri::command]
pub async fn kill_switch_force_cleanup() -> Result<(), String> {
    // Гарантируем что helper доступен — иначе предложим запустить вручную
    // через консоль (`nemefisto-helper killswitch-cleanup`).
    if let Err(e) = platform::helper_bootstrap::ensure_running().await {
        return Err(format!("helper-сервис недоступен: {e}"));
    }
    platform::helper_client::kill_switch_force_cleanup()
        .await
        .map_err(|e| e.to_string())
}

/// Полный network recovery — одной кнопкой починить всё, что мы
/// могли натворить.
///
/// 1. WFP-фильтры (наш provider GUID) — через helper.
/// 2. orphan TUN-адаптеры и half-default routes — через helper.
/// 3. Системный прокси — hardened force-clear через двойной щит.
///
/// Каждый шаг независимый: ошибка одного не отменяет других. Возвращает
/// summary что удалось / не удалось — фронт показывает в toast.
///
/// Безопасно вызывать только когда VPN не активен. UI-кнопку показываем
/// в Settings, активную только в `status === "stopped"`.
#[derive(Serialize)]
pub struct RecoveryReport {
    pub kill_switch_cleaned: bool,
    pub orphan_resources_cleaned: bool,
    pub system_proxy_cleared: bool,
    /// Список ошибок шагов которые не отработали — UI покажет в toast.
    pub errors: Vec<String>,
}

#[tauri::command]
pub async fn recover_network() -> RecoveryReport {
    let mut report = RecoveryReport {
        kill_switch_cleaned: false,
        orphan_resources_cleaned: false,
        system_proxy_cleared: false,
        errors: Vec::new(),
    };

    // Helper нужен для шагов 1+2. Если не доступен — пропускаем их и
    // продолжаем с шагом 3, который независим (registry HKCU).
    let helper_alive = platform::helper_bootstrap::ensure_running().await.is_ok();

    if helper_alive {
        match platform::helper_client::kill_switch_force_cleanup().await {
            Ok(()) => report.kill_switch_cleaned = true,
            Err(e) => report.errors.push(format!("kill switch cleanup: {e}")),
        }
        match platform::helper_client::orphan_cleanup().await {
            Ok(()) => report.orphan_resources_cleaned = true,
            Err(e) => report.errors.push(format!("orphan cleanup: {e}")),
        }
    } else {
        report
            .errors
            .push("helper-сервис недоступен: пропустили WFP/TUN cleanup".to_string());
    }

    match platform::proxy::force_clear_system_proxy() {
        Ok(()) => report.system_proxy_cleared = true,
        Err(e) => report.errors.push(format!("system proxy: {e}")),
    }

    report
}

/// 14.E — диагностика остатков прошлой сессии для расширенного
/// `CrashRecoveryDialog`. Один вызов на старте app — фронт решает
/// показывать ли диалог.
///
/// Сигналы:
/// - `was_crashed` — lockfile существовал но PID мёртв (значит прошлая
///   сессия не вышла чисто);
/// - `proxy_orphan` — в реестре HKCU прокси указывает на наш паттерн
///   (`127.0.0.1:port` где port в нашем диапазоне);
/// - `proxy_backup_present` — есть `proxy_backup.json` от прошлого
///   `set_system_proxy`, можно сделать restore;
/// - `tun_orphan` — есть адаптер с префиксом `nemefisto-` (helper
///   обычно их сам чистит при старте, но если helper-сервис не
///   запущен — остаются).
///
/// Если все пять false — фронт диалог не показывает.
///
/// 14.E: добавлено поле `orphan_wfp_filters` — best-effort проверка
/// через helper-сервис. Если helper не запущен или не отвечает —
/// возвращаем false (значит проверить нельзя, лучше не пугать
/// пользователя ложным сигналом).
#[derive(Serialize)]
pub struct RecoveryState {
    pub was_crashed: bool,
    pub proxy_orphan: bool,
    pub proxy_backup_present: bool,
    pub tun_orphan: bool,
    pub orphan_wfp_filters: bool,
}

#[tauri::command]
pub async fn get_recovery_state() -> RecoveryState {
    let proxy_orphan = platform::proxy::is_proxy_pointing_to_us();
    let proxy_backup_present = platform::proxy::has_pending_backup();
    let tun_orphan = platform::network::has_orphan_tun_adapters();

    // 14.E: проверка orphan-фильтров через helper. Делаем с timeout и
    // на любые ошибки (helper не отвечает, не установлен) возвращаем
    // false. Pipe внутри helper_client уже имеет 1-секундный retry-loop;
    // дополнительный timeout оборачивать не обязательно, но для
    // надёжности в случае зависшего pipe — да.
    let orphan_wfp_filters = match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        platform::helper_client::wfp_query_orphan(),
    )
    .await
    {
        Ok(Ok(has)) => has,
        Ok(Err(_)) | Err(_) => false,
    };

    RecoveryState {
        // session_lock мы вызывали в `lib.rs::setup` — но это уже после
        // того как мы перетёрли lockfile своим PID. Поэтому здесь
        // используем простой proxy для «недавно был краш»: либо backup
        // присутствует, либо прокси указывает на нас. Если ничего из
        // этого нет — was_crashed = false (даже если на самом деле
        // был краш в прошлый раз — нам нечего восстанавливать).
        was_crashed: proxy_backup_present
            || proxy_orphan
            || tun_orphan
            || orphan_wfp_filters,
        proxy_orphan,
        proxy_backup_present,
        tun_orphan,
        orphan_wfp_filters,
    }
}

/// Live-toggle kill-switch без disconnect/connect.
///
/// Фронт зовёт когда пользователь меняет переключатель в Settings во
/// время активного VPN. Параметры (server_ips, app-paths, dns) берутся
/// из контекста, сохранённого при connect — пересборка не нужна.
///
/// `enabled=true` без активного контекста (VPN не подключён) — no-op,
/// `false` без контекста — best-effort disable (на случай orphan
/// фильтров от прошлой сессии).
///
/// `strict` опционально обновляет сохранённый strict_mode перед re-apply.
/// Используется при live-toggle 13.S strict-mode toggle в Settings.
#[tauri::command]
pub async fn kill_switch_apply(
    enabled: bool,
    strict: Option<bool>,
    ks_ctx: State<'_, KillSwitchState>,
) -> Result<(), String> {
    // Обновляем strict_mode в контексте если фронт прислал новое значение.
    if let Some(new_strict) = strict {
        if let Some(ctx) = ks_ctx.0.lock().await.as_mut() {
            ctx.strict_mode = new_strict;
        }
    }

    let ctx_opt = ks_ctx.0.lock().await.clone();

    if !enabled {
        // disable — безопасно вызвать всегда, helper-side идемпотентно.
        return platform::helper_client::kill_switch_disable()
            .await
            .map_err(|e| e.to_string());
    }

    let Some(ctx) = ctx_opt else {
        // VPN не подключён — нечего применять. Не ошибка: пользователь
        // мог включить переключатель «впрок» до connect.
        return Ok(());
    };

    // Helper должен быть жив — kill_switch_enable требует pipe.
    if let Err(e) = platform::helper_bootstrap::ensure_running().await {
        return Err(format!("helper-сервис недоступен: {e}"));
    }
    platform::helper_client::kill_switch_enable(
        ctx.server_ips,
        ctx.allow_lan,
        ctx.allow_app_paths,
        ctx.block_dns,
        ctx.allow_dns_ips,
        ctx.strict_mode,
        ctx.expect_tun,
    )
    .await
    .map_err(|e| e.to_string())
}

// ─── Leak-test (13.B + 13.H) ────────────────────────────────────────────────

/// Проверка утечек IP/DNS. Делает два HTTP-запроса параллельно:
/// ipapi.co для public IP/GeoIP и DoH к Cloudflare для DNS-резолвера.
///
/// `socks_port` — наш локальный SOCKS5 inbound (proxy-mode). В tun-mode
/// фронт передаёт `None` и трафик идёт через system route.
///
/// Команда не падает при сетевой ошибке — возвращает структуру с
/// частично заполненными полями, фронт показывает «—» где данных нет.
#[tauri::command]
pub async fn leak_test(
    socks_port: Option<u16>,
) -> Result<crate::vpn::leak_test::LeakTestResult, String> {
    crate::vpn::leak_test::run(socks_port)
        .await
        .map_err(|e| e.to_string())
}

// ─── Floating window (13.O) ─────────────────────────────────────────────────

/// Показать плавающее окно со статусом и скоростью передачи данных.
/// Окно создаётся в `lib.rs` setup всегда, но скрытым; команда лишь
/// делает его видимым. Идемпотентна: повторный вызов на видимом окне —
/// просто .show() (no-op) + setFocus.
#[tauri::command]
pub fn show_floating_window(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    let win = app
        .get_webview_window("floating")
        .ok_or_else(|| "floating-окно не зарегистрировано".to_string())?;
    win.show().map_err(|e| e.to_string())?;
    Ok(())
}

/// Скрыть плавающее окно. Окно остаётся в памяти, повторный
/// `show_floating_window` мгновенный (нет переинициализации webview).
#[tauri::command]
pub fn hide_floating_window(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    let win = app
        .get_webview_window("floating")
        .ok_or_else(|| "floating-окно не зарегистрировано".to_string())?;
    win.hide().map_err(|e| e.to_string())?;
    Ok(())
}

// ─── Crash recovery (9.D) ─────────────────────────────────────────────────────

/// Существует ли backup-файл системного прокси с прошлой сессии.
/// UI вызывает на старте; если true — показывает диалог «обнаружены
/// остатки прошлой сессии, восстановить настройки прокси?».
#[tauri::command]
pub fn has_proxy_backup() -> bool {
    platform::proxy::has_pending_backup()
}

/// Восстановить системный прокси из backup-файла (после краша приложения
/// в режиме proxy). Удаляет backup-файл после успеха.
#[tauri::command]
pub fn restore_proxy_backup() -> Result<(), String> {
    platform::proxy::restore_from_backup().map_err(|e| e.to_string())
}

/// Отбросить backup без применения (пользователь в диалоге выбрал
/// «не восстанавливать»). Текущее состояние реестра остаётся как есть.
#[tauri::command]
pub fn discard_proxy_backup() {
    platform::proxy::discard_backup();
}

// ─── Secure storage (6.A — Credential Manager) ────────────────────────────────

/// Прочитать значение из Windows Credential Manager. Возвращает пустую
/// строку если ключа нет — фронту так удобнее обрабатывать.
#[tauri::command]
pub fn secure_storage_get(key: String) -> Result<String, String> {
    platform::secure_storage::get(&key)
        .map(|v| v.unwrap_or_default())
        .map_err(|e| e.to_string())
}

/// Записать значение в Windows Credential Manager.
#[tauri::command]
pub fn secure_storage_set(key: String, value: String) -> Result<(), String> {
    platform::secure_storage::set(&key, &value).map_err(|e| e.to_string())
}

/// Удалить значение из Windows Credential Manager.
#[tauri::command]
pub fn secure_storage_delete(key: String) -> Result<(), String> {
    platform::secure_storage::delete(&key).map_err(|e| e.to_string())
}

// ─── Autostart (6.B) ──────────────────────────────────────────────────────────

/// Зарегистрирован ли task автозапуска в Windows Task Scheduler.
///
/// 0.1.1 / Bug 4: команда async — `schtasks.exe` блокирует поток до
/// 15 секунд, и старая sync-версия зависала весь UI на это время.
#[tauri::command]
pub async fn autostart_is_enabled() -> bool {
    platform::autostart::is_enabled().await
}

/// Включить автозапуск приложения с системой (создаёт task ONLOGON).
#[tauri::command]
pub async fn autostart_enable() -> Result<(), String> {
    platform::autostart::enable().await.map_err(|e| e.to_string())
}

/// Выключить автозапуск (удаляет task).
#[tauri::command]
pub async fn autostart_disable() -> Result<(), String> {
    platform::autostart::disable().await.map_err(|e| e.to_string())
}

/// Вернуть HWID устройства (Windows MachineGuid либо локально сохранённый UUID).
/// Используется UI для отображения и копирования.
#[tauri::command]
pub fn get_hwid(hwid: State<'_, HwidState>) -> String {
    hwid.0.clone()
}

/// Прочитать последние ~32 КБ логов VPN-движка из всех известных
/// log-файлов (`sing-box-stderr.log`, `mihomo-stderr.log`, плюс
/// helper-side `C:\ProgramData\NemefistoVPN\sing-box.log` /
/// `mihomo.log` если они есть).
///
/// Имя `read_xray_log` оставлено для совместимости с фронтом (UI
/// `LogsBlock`). После 0.1.2 миграции содержимое — sing-box / mihomo,
/// xray больше не используется.
#[tauri::command]
pub fn read_xray_log() -> Result<String, String> {
    use std::io::{Read, Seek, SeekFrom};

    let tmp_dir = std::env::temp_dir().join("NemefistoVPN");
    let prog_dir = std::path::PathBuf::from(r"C:\ProgramData\NemefistoVPN");

    let candidates = [
        tmp_dir.join("sing-box-stderr.log"),
        tmp_dir.join("mihomo-stderr.log"),
        prog_dir.join("sing-box.log"),
        prog_dir.join("mihomo.log"),
    ];

    // Берём самый свежий по mtime из существующих файлов.
    let newest = candidates
        .iter()
        .filter(|p| p.exists())
        .filter_map(|p| {
            p.metadata()
                .and_then(|m| m.modified())
                .ok()
                .map(|t| (p.clone(), t))
        })
        .max_by_key(|(_, t)| *t)
        .map(|(p, _)| p);

    let path = match newest {
        Some(p) => p,
        None => return Ok(String::new()),
    };

    let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    let max = 32 * 1024;
    let start = len.saturating_sub(max);
    file.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let header = format!("=== {} ===\n", path.display());
    Ok(format!(
        "{}{}",
        header,
        String::from_utf8_lossy(&buf)
    ))
}

/// Пинговать все серверы из текущей подписки параллельно (TCP-connect).
///
/// Возвращает массив той же длины и порядка что `get_servers`. Для каждого
/// сервера: время отклика в мс или `None`, если адрес не извлекается /
/// сервер не ответил за 2.5 секунды.
#[tauri::command]
pub async fn ping_servers(
    sub: State<'_, SubscriptionState>,
) -> Result<Vec<Option<u32>>, String> {
    let entries: Vec<ProxyEntry> = {
        let g = sub.servers.lock().map_err(|e| e.to_string())?;
        g.clone()
    };

    let futures = entries.iter().map(ping_entry);
    let results = futures::future::join_all(futures).await;
    Ok(results)
}

// ─── 14.F — export logs для саппорта ──────────────────────────────────────────

/// Собирает диагностический zip-пакет с локальной информацией для саппорта.
/// Без телеметрии — файл сохраняется на диск пользователя, он сам решает
/// кому отправить.
///
/// Содержимое:
/// - `app-info.txt` — версия клиента, OS, CARGO_PKG_VERSION;
/// - `xray-stderr.log` — последние 32 КБ логов Xray (если есть);
/// - `competing-vpns.txt` — список найденных параллельных VPN-клиентов;
/// - `recovery-state.json` — текущее состояние orphan-ресурсов;
/// - `proxy-backup.json` — сохранённый backup системного прокси (если есть).
///
/// Сохраняется в `%USERPROFILE%\Documents\nemefisto-diagnostics-<timestamp>.zip`.
/// Возвращает абсолютный путь — UI показывает в toast с кнопкой
/// «открыть папку» через `tauri-plugin-opener::reveal_item_in_dir`.
#[tauri::command]
pub fn export_diagnostics() -> Result<String, String> {
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};
    use zip::write::SimpleFileOptions;

    let docs = std::env::var_os("USERPROFILE")
        .map(|h| std::path::PathBuf::from(h).join("Documents"))
        .ok_or_else(|| "не удалось определить путь к Documents".to_string())?;
    if !docs.exists() {
        std::fs::create_dir_all(&docs).map_err(|e| e.to_string())?;
    }

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let zip_path = docs.join(format!("nemefisto-diagnostics-{ts}.zip"));

    let file = std::fs::File::create(&zip_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);

    // 1. app-info.txt
    let info = format!(
        "nemefisto version: {}\n\
         OS family: {}\n\
         arch: {}\n\
         timestamp (unix): {}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        ts,
    );
    zip.start_file("app-info.txt", opts).map_err(|e| e.to_string())?;
    zip.write_all(info.as_bytes()).map_err(|e| e.to_string())?;

    // 2. xray-stderr.log (последние 32 КБ)
    let xray_log = std::env::temp_dir()
        .join("NemefistoVPN")
        .join("xray-stderr.log");
    if xray_log.is_file() {
        if let Ok(mut f) = std::fs::File::open(&xray_log) {
            use std::io::{Read, Seek, SeekFrom};
            if let Ok(meta) = f.metadata() {
                let max = 32 * 1024;
                let start = meta.len().saturating_sub(max);
                let _ = f.seek(SeekFrom::Start(start));
                let mut buf = Vec::new();
                if f.read_to_end(&mut buf).is_ok() {
                    let _ = zip.start_file("xray-stderr.log", opts);
                    let _ = zip.write_all(&buf);
                }
            }
        }
    }

    // 3. competing-vpns.txt
    let competing = platform::processes::detect_competing_vpns();
    let competing_text = if competing.is_empty() {
        "(никаких сторонних VPN-процессов не найдено)\n".to_string()
    } else {
        competing.join("\n") + "\n"
    };
    let _ = zip.start_file("competing-vpns.txt", opts);
    let _ = zip.write_all(competing_text.as_bytes());

    // 4. recovery-state.json (без orphan_wfp_filters — оно требует
    // helper round-trip, не нужно в синхронном export-flow)
    let state = RecoveryState {
        proxy_orphan: platform::proxy::is_proxy_pointing_to_us(),
        proxy_backup_present: platform::proxy::has_pending_backup(),
        tun_orphan: platform::network::has_orphan_tun_adapters(),
        orphan_wfp_filters: false,
        was_crashed: false,
    };
    if let Ok(json) = serde_json::to_string_pretty(&state) {
        let _ = zip.start_file("recovery-state.json", opts);
        let _ = zip.write_all(json.as_bytes());
    }

    // 5. proxy_backup.json — если есть
    if let Some(backup) = platform::proxy::read_backup() {
        if let Ok(json) = serde_json::to_string_pretty(&backup) {
            let _ = zip.start_file("proxy-backup.json", opts);
            let _ = zip.write_all(json.as_bytes());
        }
    }

    // 6. 14.C: crash-dump'ы за последние 7 дней. Кладём в zip как
    // crashes/<filename>.txt чтобы саппорт сразу видел стек-трейсы.
    if let Some(dir) = platform::crash_dumps::crashes_dir() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            let week_ago = std::time::SystemTime::now()
                .checked_sub(std::time::Duration::from_secs(7 * 86400))
                .unwrap_or(std::time::UNIX_EPOCH);
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("txt") {
                    continue;
                }
                let modified = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok();
                if let Some(t) = modified {
                    if t < week_ago {
                        continue;
                    }
                }
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if let Ok(content) = std::fs::read(&path) {
                    let _ = zip.start_file(format!("crashes/{name}"), opts);
                    let _ = zip.write_all(&content);
                }
            }
        }
    }

    zip.finish().map_err(|e| e.to_string())?;
    Ok(zip_path.to_string_lossy().into_owned())
}

/// 14.C: количество свежих crash-dump'ов (за неделю). UI на старте
/// показывает toast «обнаружены прошлые крахи, нажмите выгрузить
/// диагностику чтобы поделиться» если > 0.
#[tauri::command]
pub fn count_recent_crashes() -> usize {
    platform::crash_dumps::count_recent_crashes()
}

// ─── 8.F — Mihomo controller API (proxy-groups UI) ───────────────────────────

/// `GET /proxies` через mihomo external-controller. Возвращает список
/// всех нод и групп с `now`/`all`/`history`/`type`.
///
/// Доступно только когда mihomo жив И мы знаем endpoint (заполняется
/// в `connect()` для full-mihomo-профилей). Иначе — ошибка.
#[tauri::command]
pub async fn mihomo_proxies(
    state: State<'_, vpn::MihomoApiState>,
) -> Result<vpn::mihomo_api::ProxiesSnapshot, String> {
    let ep = state
        .get()
        .ok_or_else(|| "mihomo controller не активен".to_string())?;
    vpn::mihomo_api::fetch_proxies(&ep)
        .await
        .map_err(|e| e.to_string())
}

/// `PUT /proxies/:group` — выбрать ноду в select-группе. UI вызывает
/// при клике на ноду; mihomo переключает activeNode без рестарта.
///
/// Сразу после успешного select зовём `DELETE /connections` — это
/// форсит закрытие всех TCP-сессий через старый outbound. Без этого
/// браузер держит keep-alive со старой нодой и трафик продолжает идти
/// через неё, пока сессии не истекут (могут жить минутами). С close-
/// connections новый запрос сразу пойдёт через свежий outbound, как
/// FlClash/Clash Verge.
///
/// Ошибка `close_connections` не блокирует — селект уже применён,
/// просто браузер чуть позже сам подхватит. Логируем и идём дальше.
#[tauri::command]
pub async fn mihomo_select_proxy(
    group: String,
    name: String,
    state: State<'_, vpn::MihomoApiState>,
) -> Result<(), String> {
    let ep = state
        .get()
        .ok_or_else(|| "mihomo controller не активен".to_string())?;
    vpn::mihomo_api::select_proxy(&ep, &group, &name)
        .await
        .map_err(|e| e.to_string())?;
    if let Err(e) = vpn::mihomo_api::close_all_connections(&ep).await {
        eprintln!("[mihomo] close_connections after select failed: {e:#}");
    }
    Ok(())
}

/// `GET /proxies/:name/delay` — измерить latency. Используется в
/// ProxiesPanel для кнопок «test now». URL и timeout берутся
/// разумные дефолты.
#[tauri::command]
pub async fn mihomo_delay_test(
    name: String,
    state: State<'_, vpn::MihomoApiState>,
) -> Result<Option<u32>, String> {
    let ep = state
        .get()
        .ok_or_else(|| "mihomo controller не активен".to_string())?;
    // cdn-cgi/trace — лёгкий 200 OK от Cloudflare, не подвержен
    // throttle'у других сервисов; 5s timeout ловит только реально
    // живые узлы.
    vpn::mihomo_api::delay_test(
        &ep,
        &name,
        "https://cp.cloudflare.com/generate_204",
        5000,
    )
    .await
    .map_err(|e| e.to_string())
}

// ─── 12.D — backup/restore настроек ─────────────────────────────────────────

/// Записать backup-JSON в `%USERPROFILE%\Documents\nemefisto-backup-<ts>.json`.
///
/// Frontend сам собирает JSON (с whitelist'ом полей и `schema_version`),
/// мы лишь сохраняем файл — нет смысла дублировать сериализацию настроек
/// на Rust-стороне. Возвращаем абсолютный путь, который UI показывает
/// в toast.
///
/// Безопасность: ничего opaque-нечитаемого (HWID, Credential Manager
/// записи, токены) сюда не попадёт — это ответственность фронта,
/// который собирает payload.
#[tauri::command]
pub fn export_settings_to_documents(json: String) -> Result<String, String> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let docs = std::env::var_os("USERPROFILE")
        .map(|h| std::path::PathBuf::from(h).join("Documents"))
        .ok_or_else(|| "не удалось определить путь к Documents".to_string())?;
    if !docs.exists() {
        std::fs::create_dir_all(&docs).map_err(|e| e.to_string())?;
    }

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = docs.join(format!("nemefisto-backup-{ts}.json"));
    std::fs::write(&path, json.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// 12.D: скачать backup-JSON по URL (нужен для deep-link
/// `nemefisto://import-from-url/<url>`). Делается с no-proxy чтобы не
/// зацикливаться через активный VPN. Размер ограничен 256 KB —
/// настройки не должны весить больше, любой больший payload — подозрение
/// на mistake/SSRF на large endpoint.
#[tauri::command]
pub async fn fetch_settings_backup(url: String) -> Result<String, String> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("ожидается http(s):// URL".to_string());
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    if let Some(len) = resp.content_length() {
        if len > 256 * 1024 {
            return Err(format!("файл слишком большой: {} байт (>256 КБ)", len));
        }
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > 256 * 1024 {
        return Err(format!("файл слишком большой: {} байт (>256 КБ)", bytes.len()));
    }
    String::from_utf8(bytes.to_vec()).map_err(|e| format!("не UTF-8: {e}"))
}

// ─── 11.C/D/E — управление routing-профилями ──────────────────────────────────

use crate::config::routing_profile::{
    parse_profile_input, ProfileSource, RoutingProfile,
};
use crate::config::routing_store::{
    canonicalize_github_blob, fetch_profile_from_url, RoutingStoreSnapshot, RoutingStoreState,
};

/// Получить snapshot всех профилей и id активного. Один вызов на UI-mount.
#[tauri::command]
pub fn routing_list(state: State<'_, RoutingStoreState>) -> RoutingStoreSnapshot {
    state
        .inner
        .lock()
        .map(|g| g.snapshot())
        .unwrap_or_default()
}

/// Добавить статический профиль из base64/JSON-строки. Возвращает id.
#[tauri::command]
pub fn routing_add_static(
    payload: String,
    state: State<'_, RoutingStoreState>,
) -> Result<String, String> {
    let profile = parse_profile_input(&payload).map_err(|e| e.to_string())?;
    let id = state
        .inner
        .lock()
        .map_err(|e| e.to_string())?
        .add(profile, ProfileSource::Static)
        .map_err(|e| e.to_string())?;
    state.wake.notify_one();
    Ok(id)
}

/// Скачать профиль по URL и добавить как autorouting (с авто-обновлением
/// каждые `interval_hours`). При первом скачивании сразу применяется.
#[tauri::command]
pub async fn routing_add_url(
    url: String,
    interval_hours: u32,
    state: State<'_, RoutingStoreState>,
) -> Result<String, String> {
    let profile = fetch_profile_from_url(&url).await.map_err(|e| e.to_string())?;
    let canonical = canonicalize_github_blob(&url);
    let id = {
        let mut g = state.inner.lock().map_err(|e| e.to_string())?;
        g.add(
            profile,
            ProfileSource::Autorouting {
                url: canonical,
                interval_hours: interval_hours.max(1),
            },
        )
        .map_err(|e| e.to_string())?
    };
    state.wake.notify_one();
    Ok(id)
}

/// Удалить профиль. Если он был активным — активный сбрасывается.
#[tauri::command]
pub fn routing_remove(
    id: String,
    state: State<'_, RoutingStoreState>,
) -> Result<(), String> {
    state
        .inner
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&id)
        .map_err(|e| e.to_string())
}

/// Сделать профиль активным (или сбросить активный если `id=None`).
#[tauri::command]
pub fn routing_set_active(
    id: Option<String>,
    state: State<'_, RoutingStoreState>,
) -> Result<(), String> {
    let mut g = state.inner.lock().map_err(|e| e.to_string())?;
    g.set_active(id.as_deref()).map_err(|e| e.to_string())?;
    drop(g);
    state.wake.notify_one();
    Ok(())
}

/// Принудительно обновить autorouting-профиль (не дожидаясь scheduler-tick).
/// Для статического профиля — no-op.
#[tauri::command]
pub async fn routing_refresh(
    id: String,
    state: State<'_, RoutingStoreState>,
) -> Result<(), String> {
    let entry = {
        let g = state.inner.lock().map_err(|e| e.to_string())?;
        g.snapshot().entries.into_iter().find(|e| e.id == id)
    };
    let Some(entry) = entry else {
        return Err(format!("профиль {id} не найден"));
    };
    match entry.source {
        ProfileSource::Static => Ok(()),
        ProfileSource::Autorouting { url, .. } => {
            let profile = fetch_profile_from_url(&url).await.map_err(|e| e.to_string())?;
            state
                .inner
                .lock()
                .map_err(|e| e.to_string())?
                .update_profile(&id, profile)
                .map_err(|e| e.to_string())?;
            state.wake.notify_one();
            Ok(())
        }
    }
}

/// Принудительное обновление geofiles (.dat-файлов) — для UI-кнопки в
/// разделе routing. Возвращает report что обновилось / что пропустилось
/// по unchanged sha256 / какие были errors.
#[tauri::command]
pub async fn geofiles_refresh(
    state: State<'_, RoutingStoreState>,
) -> Result<crate::config::geofiles::UpdateReport, String> {
    let active = state
        .inner
        .lock()
        .map_err(|e| e.to_string())?
        .active()
        .map(|e| (e.profile.geoip_url.clone(), e.profile.geosite_url.clone()));
    let (geoip, geosite) = active.unwrap_or_default();
    Ok(crate::config::geofiles::update_geofiles_if_changed(&geoip, &geosite).await)
}

/// Текущее состояние geofiles: какие файлы есть, размер, sha256.
#[tauri::command]
pub fn geofiles_status() -> crate::config::geofiles::GeofilesStatus {
    crate::config::geofiles::status()
}

// Suppress dead_code для неиспользуемых пока хелперов из routing_profile.
#[allow(dead_code)]
fn _routing_profile_unused_check(p: &RoutingProfile) -> usize {
    p.direct_sites.len()
}

// ─── 9.B / 9.C — детект конфликтов с другими VPN ──────────────────────────────

/// 9.B — Список запущенных сторонних VPN-клиентов (по whitelist'у имён exe).
///
/// Работает без admin-прав. Возвращает уникальные human-readable имена
/// (например `["Happ", "Clash Verge"]`). Не блокирует connect — UI
/// использует это для предупреждающего баннера.
#[tauri::command]
pub fn detect_competing_vpns() -> Vec<String> {
    platform::processes::detect_competing_vpns()
}

/// 9.C — Список интерфейсов с активными default- или half-default-маршрутами,
/// принадлежащих **не нам** (NextHop ≠ 198.18.0.1) и **не штатному** physic-
/// default'у. Признак того, что параллельно работает другой VPN.
///
/// Возвращает aliases интерфейсов (например `["Wintun Userspace Tunnel"]`).
/// Frontend перед connect показывает toast и не запускает VPN.
#[tauri::command]
pub fn check_routing_conflicts() -> Vec<String> {
    platform::network::detect_routing_conflicts()
}
