//! Tauri commands, доступные из фронтенда через `invoke`.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use tauri::State;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
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
}

/// Tauri-state для контекста активного kill-switch. None = VPN не подключён.
pub struct KillSwitchState(pub AsyncMutex<Option<KillSwitchContext>>);

impl KillSwitchState {
    pub fn new() -> Self {
        Self(AsyncMutex::new(None))
    }
}

use crate::config::mihomo_config::AppRule;
use crate::config::subscription::{fetch_and_parse, SubscriptionMeta};
use crate::config::xray_config::{self, AntiDpiOptions};
use crate::config::{mihomo_config, HwidState, ProxyEntry, SubscriptionState};
use crate::platform;
use crate::vpn::{find_free_port, ping_entry, random_high_port, MihomoState, XrayState};

/// Имя файла с triplet-суффиксом — формат, в котором лежит исходный
/// бинарь в `binaries/`, и в котором Tauri (большинство версий) кладёт
/// его в bundle рядом с основным exe.
const TUN2SOCKS_FILENAME: &str = "tun2socks-x86_64-pc-windows-msvc.exe";
/// На случай если конкретная версия Tauri стрипает triplet после bundle.
const TUN2SOCKS_FILENAME_PLAIN: &str = "tun2socks.exe";

/// Прогревочный запрос через SOCKS5 чтобы заставить Xray установить
/// upstream-соединение с VPN-сервером до того как мы перенаправим в TUN
/// весь системный трафик. Без прогрева первый user-запрос ждёт burstObservatory
/// probes + REALITY-handshake = 10-20 секунд видимой задержки.
///
/// Цикл: TCP-connect к 127.0.0.1:socks_port → SOCKS5 NoAuth handshake →
/// CONNECT cloudflare.com:443 → читаем ответ. Xray в этот момент:
///   1. Запускает burstObservatory probes (если есть balancer).
///   2. Выбирает best outbound.
///   3. Делает REALITY/TLS handshake к VPN-серверу.
///   4. Открывает upstream TCP к cloudflare.com через VPN.
///   5. Возвращает SOCKS5 success.
///
/// После этого вся машинерия Xray «горячая» и user-запросы идут мгновенно.
async fn warmup_xray(socks_port: u16) -> Result<(), String> {
    // Ждём пока Xray откроет 1080 (он стартует асинхронно)
    let connect_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut stream = loop {
        match TcpStream::connect(("127.0.0.1", socks_port)).await {
            Ok(s) => break s,
            Err(_) if tokio::time::Instant::now() < connect_deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(e) => return Err(format!("Xray не открыл SOCKS5 за 3с: {e}")),
        }
    };

    // SOCKS5 handshake (no auth)
    let timeout = Duration::from_secs(15);
    tokio::time::timeout(timeout, async {
        stream.write_all(&[0x05, 0x01, 0x00]).await
            .map_err(|e| format!("write greeting: {e}"))?;
        let mut greet_resp = [0u8; 2];
        stream.read_exact(&mut greet_resp).await
            .map_err(|e| format!("read greeting: {e}"))?;
        if greet_resp != [0x05, 0x00] {
            return Err(format!("неожиданный greeting-ответ: {greet_resp:?}"));
        }

        // CONNECT 1.1.1.1:443 (используем IP чтобы не зависеть от DNS)
        let request: [u8; 10] = [
            0x05,                           // SOCKS5
            0x01,                           // CONNECT
            0x00,                           // reserved
            0x01,                           // ATYP = IPv4
            1, 1, 1, 1,                     // 1.1.1.1
            0x01, 0xBB,                     // port 443
        ];
        stream.write_all(&request).await
            .map_err(|e| format!("write connect: {e}"))?;
        let mut resp_head = [0u8; 4];
        stream.read_exact(&mut resp_head).await
            .map_err(|e| format!("read connect-response head: {e}"))?;
        if resp_head[1] != 0x00 {
            return Err(format!("SOCKS5 CONNECT failed: код {}", resp_head[1]));
        }
        // Дочитываем addr+port (зависит от ATYP в resp_head[3])
        let to_skip = match resp_head[3] {
            0x01 => 4 + 2, // IPv4
            0x03 => {
                let mut len_buf = [0u8; 1];
                stream.read_exact(&mut len_buf).await
                    .map_err(|e| format!("read domain len: {e}"))?;
                len_buf[0] as usize + 2
            }
            0x04 => 16 + 2, // IPv6
            _ => return Err(format!("неожиданный ATYP в response: {}", resp_head[3])),
        };
        let mut skip_buf = vec![0u8; to_skip];
        let _ = stream.read_exact(&mut skip_buf).await;
        Ok(())
    })
    .await
    .map_err(|_| format!("warmup-запрос не завершился за {}с", timeout.as_secs()))?
}

// ─── Helper-функции для TUN-режима ────────────────────────────────────────────

/// Извлечь хост VPN-сервера из ProxyEntry для bypass-route.
/// Логика повторяет `vpn::ping::extract_target`, но возвращает только host.
fn extract_server_host(entry: &ProxyEntry) -> Option<String> {
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

/// Найти `tun2socks` в нескольких возможных локациях:
///   1. `<exe-dir>/tun2socks-<triplet>.exe`          — стандартный bundle
///                                                     externalBin;
///   2. `<exe-dir>/tun2socks.exe`                    — на случай если
///                                                     Tauri стрипает
///                                                     triplet;
///   3. `<exe-dir>/binaries/tun2socks-<triplet>.exe` — старый layout
///                                                     для совместимости;
///   4. `<exe-dir>/../../binaries/...`               — dev из
///                                                     target/{debug,
///                                                     release}/.
/// Найти путь к sidecar-бинарю по короткому имени (`xray`/`mihomo`).
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

fn resolve_tun2socks_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    // Production-кандидаты
    let mut candidates: Vec<PathBuf> = vec![
        exe_dir.join(TUN2SOCKS_FILENAME),
        exe_dir.join(TUN2SOCKS_FILENAME_PLAIN),
        exe_dir.join("binaries").join(TUN2SOCKS_FILENAME),
        exe_dir.join("resources").join(TUN2SOCKS_FILENAME),
    ];

    // Dev: target/{profile}/ → подняться до src-tauri/, оттуда в binaries/
    if let Some(dev_root) = exe_dir.parent().and_then(|p| p.parent()) {
        candidates.push(dev_root.join("binaries").join(TUN2SOCKS_FILENAME));
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
    xray: State<'_, XrayState>,
    mihomo: State<'_, MihomoState>,
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
        servers
            .get(server_index)
            .cloned()
            .ok_or_else(|| format!("сервер #{server_index} не найден в списке"))?
    };

    // 8.B — выбор движка. Дефолт xray. Проверяем что сервер поддерживает
    // выбранное ядро; иначе возвращаем понятную ошибку (UI должен предложить
    // переключить движок до повторного клика).
    let chosen_engine = engine.as_deref().unwrap_or("xray");
    if !entry.engine_compat.iter().any(|e| e == chosen_engine) {
        return Err(format!(
            "сервер «{}» несовместим с движком {}; поддерживается: {}",
            entry.name,
            chosen_engine,
            entry.engine_compat.join(", ")
        ));
    }

    // 9.H — рандомизация портов inbound. Старт с псевдослучайных значений
    // в диапазоне [30000, 60000) вместо фиксированных 1080/1087, чтобы
    // сторонний процесс на машине не мог дёшево детектнуть VPN-клиент
    // сканированием стандартных портов. См. https://habr.com/ru/news/1020902/.
    // У Mihomo один mixed-port на SOCKS5+HTTP, поэтому для него используем
    // одно и то же значение для обоих "портов" (всё равно один сокет).
    let default_socks = find_free_port(random_high_port());
    let default_http = if chosen_engine == "mihomo" {
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
    let physic_iface = if tun_mode {
        platform::network::get_default_route_interface_name()
    } else {
        None
    };

    // Для TUN-режима готовим параметры до старта Xray, чтобы при ошибке
    // (нет helper-а / tun2socks) не пришлось гасить уже запущенный процесс.
    let tun_params = if mode == "tun" {
        let server_host = extract_server_host(&entry).ok_or_else(|| {
            "не удалось определить хост сервера для TUN-режима".to_string()
        })?;
        let tun2socks_path = resolve_tun2socks_path().ok_or_else(|| {
            format!(
                "{TUN2SOCKS_FILENAME} не найден ни рядом с приложением, ни в src-tauri/binaries/"
            )
        })?;
        let path_str = tun2socks_path
            .to_str()
            .ok_or_else(|| "путь к tun2socks содержит не-UTF-8 символы".to_string())?
            .to_string();
        Some((server_host, path_str))
    } else {
        None
    };

    // 8.B: разветвление по движку. Под Mihomo генерим YAML и запускаем
    // mihomo sidecar; под Xray — текущий путь с JSON и xray sidecar.
    // socks_port и http_port одинаковые для Mihomo (один mixed-port).
    let (socks_port, http_port) = if chosen_engine == "mihomo" {
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
        let cfg = mihomo_config::build(
            &entry,
            default_socks,
            listen,
            anti_dpi.as_ref(),
            auth_pair,
            rules_slice,
            active_profile.as_ref(),
        )
        .map_err(|e| e.to_string())?;
        // На всякий случай гасим Xray если он был активен от прошлой сессии
        // на другом движке (не должно бывать, но дешёвая страховка).
        let _ = xray.stop();
        mihomo.start_with_config(&app, &cfg.yaml, cfg.mixed_port)?;
        (cfg.mixed_port, cfg.mixed_port)
    } else {
        // xray-json: патчим внешний конфиг (порты + listen)
        // иначе: генерируем конфиг из ProxyEntry
        let (mut config_json, sp, hp) = if entry.protocol == "xray-json" {
            let patched = xray_config::patch_xray_json(
                entry.raw.clone(),
                default_socks,
                default_http,
                listen,
                tun_mode,
                physic_iface.as_deref(),
            );
            (patched, default_socks, default_http)
        } else {
            let auth_pair = socks_auth
                .as_ref()
                .map(|(u, p)| (u.as_str(), p.as_str()));
            let cfg = xray_config::build(
                &entry,
                default_socks,
                default_http,
                listen,
                tun_mode,
                physic_iface.as_deref(),
                anti_dpi.as_ref(),
                auth_pair,
            )
            .map_err(|e| e.to_string())?;
            (cfg.json, cfg.socks_port, cfg.http_port)
        };

        // 11.F + 13.Q: применить активный routing-профиль или встроенный
        // минимальный RU-шаблон. Для xray-json (custom config из подписки)
        // — НЕ применяем, у пользователя свои правила приоритетнее.
        if entry.protocol != "xray-json" {
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
            if let Some(p) = active_profile.as_ref() {
                xray_config::apply_routing_profile(&mut config_json, p);
            }
        }

        // Запускаем Xray ДО поднятия TUN — иначе резолв server-а в Xray уйдёт
        // через TUN, а tun2socks не сможет соединиться с upstream-Xray.
        let _ = mihomo.stop();
        xray.start_with_config(&app, &config_json, sp, hp)?;
        (sp, hp)
    };

    stamp("xray/mihomo started");

    match mode.as_str() {
        "proxy" => {
            platform::proxy::set_system_proxy(socks_port, http_port)
                .map_err(|e| e.to_string())?;
            stamp("system proxy set");
        }
        "tun" => {
            // Прогреваем Xray ДО перенаправления трафика в TUN. Иначе первый
            // user-запрос ждёт burstObservatory + REALITY handshake (10-20с).
            stamp("tun: warmup_xray begin");
            if let Err(e) = warmup_xray(socks_port).await {
                eprintln!("[connect] warmup_xray не удался ({e}) — продолжаем, первый запрос может тормозить");
            }
            stamp("tun: warmup_xray done");

            // Гарантируем что helper-сервис запущен. Если нет — будет UAC
            // и авто-установка. После первого согласия пользователя сервис
            // ставится с AutoStart и больше UAC не запрашивает.
            if let Err(e) = platform::helper_bootstrap::ensure_running().await {
                let _ = xray.stop();
                let _ = mihomo.stop();
                return Err(format!("helper-сервис недоступен: {e}"));
            }
            stamp("tun: helper ensure_running done");

            // tun_params гарантировано Some — установлено выше
            let (server_host, tun2socks_path) = tun_params.unwrap();
            // Для маскировки TUN-имени (12.E) генерируем нейтральное имя
            // только для этого запуска. Helper создаст адаптер с ним.
            let tun_name_override = if tun_masking.unwrap_or(false) {
                Some(generate_masked_tun_name())
            } else {
                None
            };
            // SOCKS5 креды (9.G): tun2socks подключится с auth.
            let (auth_user, auth_pass) = match &socks_auth {
                Some((u, p)) => (Some(u.clone()), Some(p.clone())),
                None => (None, None),
            };
            stamp("tun: tun_start IPC begin");
            if let Err(e) = platform::helper_client::tun_start(
                socks_port,
                server_host,
                "1.1.1.1".to_string(),
                tun2socks_path,
                auth_user,
                auth_pass,
                tun_name_override,
            )
            .await
            {
                // Откатываем активный движок если TUN не поднялся
                let _ = xray.stop();
                let _ = mihomo.stop();
                return Err(format!(
                    "TUN-режим не запустился: {e}\n\nПроверьте services.msc → NemefistoHelper, и что tun2socks.exe + wintun.dll лежат рядом друг с другом."
                ));
            }
            stamp("tun: tun_start IPC done");
        }
        other => {
            let _ = xray.stop();
            let _ = mihomo.stop();
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
                let _ = xray.stop();
                let _ = mihomo.stop();
                let _ = platform::proxy::clear_system_proxy();
                if tun_mode {
                    let _ = platform::helper_client::tun_stop().await;
                }
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
                for name in ["xray", "mihomo"] {
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
                // tun2socks (для TUN-режима)
                push_if_exists(exe_dir.join("tun2socks.exe"));
                push_if_exists(
                    exe_dir.join("tun2socks-x86_64-pc-windows-msvc.exe"),
                );
                if let Some(dev_root) = exe_dir.parent().and_then(|p| p.parent()) {
                    push_if_exists(
                        dev_root
                            .join("binaries")
                            .join("tun2socks-x86_64-pc-windows-msvc.exe"),
                    );
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
        if let Some(p) = resolve_sidecar_path(&app, "xray") {
            push_if_exists(p);
        }
        if let Some(p) = resolve_sidecar_path(&app, "mihomo") {
            push_if_exists(p);
        }
        if let Some(p) = resolve_tun2socks_path() {
            push_if_exists(p);
        }
        // Дедупликация по string чтобы не плодить identical filters.
        allow_app_paths.sort();
        allow_app_paths.dedup();

        // Гарантируем что helper-сервис запущен (если не активен TUN-режим
        // — у нас не было ensure_running).
        if !tun_mode {
            if let Err(e) = platform::helper_bootstrap::ensure_running().await {
                let _ = xray.stop();
                let _ = mihomo.stop();
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
        )
        .await
        {
            // При ошибке откатываем всё — интернет НЕ должен оставаться
            // в полу-заблокированном состоянии.
            let _ = xray.stop();
            let _ = mihomo.stop();
            let _ = platform::proxy::clear_system_proxy();
            if tun_mode {
                let _ = platform::helper_client::tun_stop().await;
            }
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
    xray: State<'_, XrayState>,
    mihomo: State<'_, MihomoState>,
    ks_ctx: State<'_, KillSwitchState>,
) -> Result<(), String> {
    // 1. TUN
    let _ = platform::helper_client::tun_stop().await;
    // 2. Kill switch — всегда вызываем, чтобы убрать остатки если
    //    включён был в прошлый сеанс (на случай если краш / повторный
    //    запуск). Helper тихо вернёт ok если он не был enabled.
    let _ = platform::helper_client::kill_switch_disable().await;
    // Очищаем контекст live-toggle — VPN больше не активен.
    *ks_ctx.0.lock().await = None;

    // 3. Оба движка (один точно не запущен — stop() для него no-op)
    //    + system proxy. Параллельно выполняем чтобы быстрый disconnect.
    let xray_err = xray.stop().err();
    let mihomo_err = mihomo.stop().err();
    let proxy_err = platform::proxy::clear_system_proxy().err().map(|e| e.to_string());

    if let Some(e) = xray_err {
        return Err(e);
    }
    if let Some(e) = mihomo_err {
        return Err(e);
    }
    if let Some(e) = proxy_err {
        return Err(e);
    }
    Ok(())
}

/// Запущен ли VPN-движок (Xray или Mihomo) прямо сейчас.
///
/// Имя оставлено `is_xray_running` для совместимости с фронтом — после
/// этапа 8.B функция возвращает true если запущен **любой** из двух
/// поддерживаемых движков. Семантика «работает ли VPN», не привязка к
/// конкретному ядру.
#[tauri::command]
pub fn is_xray_running(
    xray: State<'_, XrayState>,
    mihomo: State<'_, MihomoState>,
) -> bool {
    xray.is_running() || mihomo.is_running()
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
#[tauri::command]
pub fn autostart_is_enabled() -> bool {
    platform::autostart::is_enabled()
}

/// Включить автозапуск приложения с системой (создаёт task ONLOGON).
#[tauri::command]
pub fn autostart_enable() -> Result<(), String> {
    platform::autostart::enable().map_err(|e| e.to_string())
}

/// Выключить автозапуск (удаляет task).
#[tauri::command]
pub fn autostart_disable() -> Result<(), String> {
    platform::autostart::disable().map_err(|e| e.to_string())
}

/// Вернуть HWID устройства (Windows MachineGuid либо локально сохранённый UUID).
/// Используется UI для отображения и копирования.
#[tauri::command]
pub fn get_hwid(hwid: State<'_, HwidState>) -> String {
    hwid.0.clone()
}

/// Прочитать последние N байт лога Xray (`%TEMP%\NemefistoVPN\xray-stderr.log`).
///
/// Возвращает строку из последних 32 КБ файла. Если файл не существует —
/// пустую строку. Используется UI для отображения логов.
#[tauri::command]
pub fn read_xray_log() -> Result<String, String> {
    use std::io::{Read, Seek, SeekFrom};

    let path = std::env::temp_dir()
        .join("NemefistoVPN")
        .join("xray-stderr.log");

    if !path.exists() {
        return Ok(String::new());
    }

    let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    let max = 32 * 1024;
    let start = len.saturating_sub(max);
    file.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
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
