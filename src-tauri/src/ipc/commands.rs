//! Tauri commands, доступные из фронтенда через `invoke`.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use tauri::State;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::config::subscription::fetch_and_parse;
use crate::config::xray_config;
use crate::config::{HwidState, ProxyEntry, SubscriptionState};
use crate::platform;
use crate::vpn::{find_free_port, ping_entry, XrayState};

const TUN2SOCKS_FILENAME: &str = "tun2socks-x86_64-pc-windows-msvc.exe";

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

/// Найти `tun2socks-x86_64-pc-windows-msvc.exe` либо рядом с текущим exe
/// (release-сборка), либо в `<exe_dir>/../../binaries/` (dev из target/debug).
fn resolve_tun2socks_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    // 1. Production: рядом с exe или в подпапке binaries/
    for candidate in [
        exe_dir.join(TUN2SOCKS_FILENAME),
        exe_dir.join("binaries").join(TUN2SOCKS_FILENAME),
    ] {
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    // 2. Dev: target/{profile}/ → подняться до src-tauri/, оттуда в binaries/
    let dev_path = exe_dir
        .parent()? // target/{profile}
        .parent()? // target
        .join("binaries")
        .join(TUN2SOCKS_FILENAME);
    if dev_path.is_file() {
        return Some(dev_path);
    }

    None
}

// ─── Результаты команд ────────────────────────────────────────────────────────

/// Возвращается фронтенду после успешного подключения.
#[derive(Serialize)]
pub struct ConnectResult {
    pub socks_port: u16,
    pub http_port: u16,
    pub server_name: String,
}

// ─── Подписка ─────────────────────────────────────────────────────────────────

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
) -> Result<Vec<ProxyEntry>, String> {
    let effective_hwid = hwid_override
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&hwid.0);

    let ua = user_agent.unwrap_or_default();
    let send = send_hwid.unwrap_or(true);

    let servers = fetch_and_parse(&url, effective_hwid, &ua, send)
        .await
        .map_err(|e| e.to_string())?;

    *sub.servers.lock().map_err(|e| e.to_string())? = servers.clone();
    Ok(servers)
}

/// Вернуть закешированный список серверов без сетевого запроса.
#[tauri::command]
pub fn get_servers(sub: State<'_, SubscriptionState>) -> Vec<ProxyEntry> {
    sub.servers.lock().map(|g| g.clone()).unwrap_or_default()
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
    allow_lan: Option<bool>,
    app: tauri::AppHandle,
    xray: State<'_, XrayState>,
    sub: State<'_, SubscriptionState>,
) -> Result<ConnectResult, String> {
    // Клонируем ProxyEntry, чтобы сразу освободить lock на список серверов
    let entry = {
        let servers = sub.servers.lock().map_err(|e| e.to_string())?;
        servers
            .get(server_index)
            .cloned()
            .ok_or_else(|| format!("сервер #{server_index} не найден в списке"))?
    };

    let default_socks = find_free_port(1080);
    let default_http = find_free_port(1087);
    let listen = if allow_lan.unwrap_or(false) { "0.0.0.0" } else { "127.0.0.1" };
    let tun_mode = mode == "tun";

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

    // xray-json: патчим внешний конфиг (порты + listen)
    // иначе: генерируем конфиг из ProxyEntry
    let (config_json, socks_port, http_port) = if entry.protocol == "xray-json" {
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
        let cfg = xray_config::build(
            &entry,
            default_socks,
            default_http,
            listen,
            tun_mode,
            physic_iface.as_deref(),
        )
        .map_err(|e| e.to_string())?;
        (cfg.json, cfg.socks_port, cfg.http_port)
    };

    // Запускаем Xray ДО поднятия TUN — иначе резолв server-а в Xray уйдёт
    // через TUN, а tun2socks не сможет соединиться с upstream-Xray.
    xray.start_with_config(&app, &config_json, socks_port, http_port)?;

    match mode.as_str() {
        "proxy" => {
            platform::proxy::set_system_proxy(socks_port, http_port)
                .map_err(|e| e.to_string())?;
        }
        "tun" => {
            // Прогреваем Xray ДО перенаправления трафика в TUN. Иначе первый
            // user-запрос ждёт burstObservatory + REALITY handshake (10-20с).
            if let Err(e) = warmup_xray(socks_port).await {
                eprintln!("[connect] warmup_xray не удался ({e}) — продолжаем, первый запрос может тормозить");
            }

            // Гарантируем что helper-сервис запущен. Если нет — будет UAC
            // и авто-установка. После первого согласия пользователя сервис
            // ставится с AutoStart и больше UAC не запрашивает.
            if let Err(e) = platform::helper_bootstrap::ensure_running().await {
                let _ = xray.stop();
                return Err(format!("helper-сервис недоступен: {e}"));
            }

            // tun_params гарантировано Some — установлено выше
            let (server_host, tun2socks_path) = tun_params.unwrap();
            if let Err(e) = platform::helper_client::tun_start(
                socks_port,
                server_host,
                "1.1.1.1".to_string(),
                tun2socks_path,
            )
            .await
            {
                // Откатываем Xray если TUN не поднялся
                let _ = xray.stop();
                return Err(format!(
                    "TUN-режим не запустился: {e}\n\nПроверьте services.msc → NemefistoHelper, и что tun2socks.exe + wintun.dll лежат рядом друг с другом."
                ));
            }
        }
        other => {
            let _ = xray.stop();
            return Err(format!("неизвестный режим: {other}"));
        }
    }

    Ok(ConnectResult {
        socks_port,
        http_port,
        server_name: entry.name,
    })
}

/// Отключиться: остановить TUN (если был активен), Xray и сбросить системный прокси.
///
/// Все три операции выполняются независимо: ошибка одной не отменяет других.
/// `tun_stop` идемпотентен — игнорирует «не запущен» / «helper недоступен».
#[tauri::command]
pub async fn disconnect(xray: State<'_, XrayState>) -> Result<(), String> {
    // 1. TUN: всегда пытаемся; ошибки тихо проглатываем (helper может не стоять)
    let _ = platform::helper_client::tun_stop().await;

    // 2. Xray + system proxy
    let xray_err = xray.stop().err();
    let proxy_err = platform::proxy::clear_system_proxy().err().map(|e| e.to_string());

    if let Some(e) = xray_err {
        return Err(e);
    }
    if let Some(e) = proxy_err {
        return Err(e);
    }
    Ok(())
}

/// Запущен ли Xray прямо сейчас.
#[tauri::command]
pub fn is_xray_running(xray: State<'_, XrayState>) -> bool {
    xray.is_running()
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
