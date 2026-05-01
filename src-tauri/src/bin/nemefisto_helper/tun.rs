//! Управление tun2socks-процессом + системным routing-ом.
//!
//! Жизненный цикл при `start`:
//!   1. Резолв `server_host` → IPv4.
//!   2. Чтение текущего default-route (gateway + ifIndex + имя интерфейса).
//!   3. Запуск `tun2socks.exe` с TUN-устройством `nemefisto`.
//!   4. Ожидание появления интерфейса в системе.
//!   5. Добавление bypass-route на VPN-сервер через старый шлюз.
//!   6. Добавление двух «половинок дефолта» (`0.0.0.0/1` и `128.0.0.0/1`)
//!      через TUN — это перебивает старый default по приоритету, не удаляя его.
//!   7. Установка DNS на TUN-интерфейс.
//!
//! При `stop` всё откатывается в обратном порядке. Удаление маршрутов
//! идемпотентно — повторный `stop` на чистом state не падает.
//!
//! При неожиданной смерти tun2socks (например, его убили вручную) маршруты
//! остаются «висеть». Для recovery планируется отдельный watcher (TODO).

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use super::routing::{self, DefaultRoute};

/// Простой helper для лог-таймингов: показывает прошедшее время
/// от начального `Instant` и от прошлой записи.
struct Timing {
    start: Instant,
    last: Instant,
}
impl Timing {
    fn new() -> Self {
        let n = Instant::now();
        Self { start: n, last: n }
    }
    fn step(&mut self, label: &str) {
        let now = Instant::now();
        let total = now.duration_since(self.start).as_millis();
        let delta = now.duration_since(self.last).as_millis();
        eprintln!("[helper-tun][+{total}ms / Δ{delta}ms] {label}");
        self.last = now;
    }
}

/// Префикс имени TUN-адаптера. Полное имя — `<префикс><pid>` чтобы
/// каждый новый запуск получал свежий уникальный адаптер. Так zombie
/// от kill -9 (или предыдущего падения) не блокирует создание нового:
/// WinTUN ругается «interface ... not ready» если есть orphan с таким
/// же именем, а с уникальным — конфликта нет.
const TUN_NAME_PREFIX: &str = "nemefisto-";
/// Адрес TUN-интерфейса. На Windows назначается helper-ом через
/// `CreateUnicastIpAddressEntry` — tun2socks этого не делает (Windows-only
/// особенность, на Linux/Darwin tun2socks назначает сам через ioctl).
const TUN_GATEWAY: &str = "198.18.0.1";
const TUN_PREFIX_LEN: u8 = 15;
const HALF_LOW_DST: &str = "0.0.0.0";
const HALF_HIGH_DST: &str = "128.0.0.0";
const HALF_MASK: &str = "128.0.0.0";
const HOST_MASK: &str = "255.255.255.255";

#[allow(dead_code)]
struct State {
    child: Child,
    server_ip: String,
    /// DNS-IP для bypass-route — чтобы при stop удалить тот же что мы добавили.
    dns: String,
    /// Сохраняем оригинальный default-route чтобы ssh-debugging знал что было.
    /// Удалять / восстанавливать его не нужно (мы используем half-routes).
    original: DefaultRoute,
    tun_index: u32,
    /// Полное имя TUN-адаптера для текущей сессии (например `nemefisto-12345`).
    tun_name: String,
}

static STATE: Mutex<Option<State>> = Mutex::const_new(None);

/// Текущий interface-index активного TUN-адаптера. Используется
/// firewall.rs (kill-switch step A) чтобы добавить allow-фильтр для
/// всего что идёт через TUN — нет надобности перечислять server_ip
/// и app-id отдельно, любой TUN-трафик пропускается.
///
/// Возвращает `None` если TUN не активен (proxy-mode).
pub async fn current_tun_interface_index() -> Option<u32> {
    let g = STATE.lock().await;
    g.as_ref().map(|s| s.tun_index)
}

/// 9.E — Cleanup orphan TUN-ресурсов на старте helper-сервиса.
///
/// После аварийного завершения (kernel panic, kill -9, hardware crash)
/// в системе могут остаться:
///   1. WinTUN-адаптеры с префиксом `nemefisto-` — обычно их убирает
///      tun2socks при exit, но при kill-9 они «зависают» orphan'ами
///      и блокируют создание нового адаптера.
///   2. Half-default routes (`0.0.0.0/1` и `128.0.0.0/1` через
///      `198.18.0.1`) — наш приём перебить системный default. При
///      нормальном `stop` они снимаются, при аварии остаются и
///      ломают любые сетевые подключения пока не удалить вручную.
///
/// Best-effort: каждая операция игнорирует свои ошибки. Запускается
/// **в фоне** (`tokio::spawn` в service_loop) чтобы не блокировать
/// открытие pipe-сервера на 2-5 секунд PowerShell-старта. Первое
/// подключение клиента может прийти раньше cleanup'а — это безопасно
/// благодаря guard'у на STATE: если TUN активен (client уже сделал
/// `tun_start`), мы не трогаем nemefisto-* адаптеры.
pub async fn cleanup_orphan_resources() {
    // Guard: если TUN активен — кто-то быстрее нас сделал tun_start,
    // не удаляем потенциально живой адаптер с тем же префиксом. Маршруты
    // тоже не трогаем — они сейчас в работе.
    if STATE.lock().await.is_some() {
        eprintln!("[helper-tun] cleanup orphan пропущен: TUN активен");
        return;
    }

    // 1. Удаляем все nemefisto-* адаптеры. PowerShell wildcard
    // обрабатывает Get-NetAdapter сам, безопасных имён это не
    // затронет — префикс уникален для нашего приложения.
    let wildcard = format!("{TUN_NAME_PREFIX}*");
    if let Err(e) = routing::cleanup_orphan_tun(&wildcard).await {
        eprintln!("[helper-tun] cleanup_orphan_tun({wildcard}) → {e}");
    }

    // 2. Удаляем half-default routes только наши (NextHop=198.18.0.1).
    // Если другой VPN использует те же префиксы с другим gateway —
    // мы их не тронем (проверка по nexthop в delete_route_with_nexthop).
    if let Err(e) =
        routing::delete_route_with_nexthop(HALF_LOW_DST, HALF_MASK, TUN_GATEWAY).await
    {
        eprintln!("[helper-tun] cleanup orphan {HALF_LOW_DST}/1 → {e}");
    }
    if let Err(e) =
        routing::delete_route_with_nexthop(HALF_HIGH_DST, HALF_MASK, TUN_GATEWAY).await
    {
        eprintln!("[helper-tun] cleanup orphan {HALF_HIGH_DST}/1 → {e}");
    }
}

pub async fn start(
    socks_port: u16,
    server_host: &str,
    dns: &str,
    tun2socks_path: &str,
    socks_username: Option<&str>,
    socks_password: Option<&str>,
    tun_name_override: Option<&str>,
) -> Result<()> {
    let mut t = Timing::new();
    let mut g = STATE.lock().await;
    if g.is_some() {
        bail!("TUN-режим уже запущен");
    }
    t.step("lock acquired");

    // 1. Проверка пути к tun2socks
    let tun2socks_exe = Path::new(tun2socks_path);
    if !tun2socks_exe.is_file() {
        bail!("tun2socks не найден по пути: {tun2socks_path}");
    }

    // Имя TUN-адаптера. По умолчанию — `nemefisto-<pid>` для уникальности
    // и упрощения cleanup-а. Если пользователь включил «маскировку TUN»
    // (12.E), main-app передаёт сюда замаскированное имя из набора
    // wlan99 / Local Area Connection N / Ethernet N — оно уже содержит
    // случайный суффикс и от запуска к запуску разное.
    let tun_name = tun_name_override
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{TUN_NAME_PREFIX}{}", std::process::id()));
    eprintln!("[helper-tun] имя TUN-адаптера: {tun_name}");

    // 2. Резолв server_ip
    let server_ip = routing::resolve_host_ipv4(server_host)
        .await
        .with_context(|| format!("резолв {server_host}"))?;
    t.step(&format!("resolve {server_host} → {server_ip}"));

    // 3. Текущий default route
    let original = routing::get_default_route()
        .await
        .context("чтение default-route")?;
    t.step("get_default_route");
    eprintln!(
        "[helper-tun] default route: {} via {} (ifIndex {} «{}»)",
        "0.0.0.0/0", original.gateway, original.if_index, original.interface_name
    );

    // 4. Подготовка лог-файла tun2socks. Сервис работает от SYSTEM, %TEMP%
    //    у него — `C:\Windows\Temp\`, что неудобно. Пишем в ProgramData
    //    чтобы admin-pwsh мог легко прочитать при отладке.
    let log_dir = std::path::PathBuf::from(r"C:\ProgramData\NemefistoVPN");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("tun2socks.log");
    let log_file = std::fs::File::create(&log_path)
        .with_context(|| format!("создание {}", log_path.display()))?;
    let log_clone = log_file.try_clone().context("клонирование файлового хендла")?;

    // 4a. Best-effort cleanup всех orphan-адаптеров с нашим префиксом.
    //     Если PowerShell-cmdlet есть — удалит мусор от прошлых падений.
    //     Если нет (некоторые сборки Windows) — silently fails, и мы всё
    //     равно не зависнем потому что новый адаптер получает уникальное имя.
    if let Err(e) = routing::cleanup_orphan_tun(&format!("{TUN_NAME_PREFIX}*")).await {
        eprintln!("[helper-tun] cleanup_orphan_tun failed (non-fatal): {e:#}");
    }
    t.step("orphan adapter cleanup (best-effort)");

    // 5. Спавн tun2socks. Лог пишем в файл — увидим причину если интерфейс
    //    не поднимется. Уровень debug для диагностики.
    //    Если для SOCKS5 inbound заданы креды (этап 9.G), передаём
    //    `socks5://user:pass@127.0.0.1:port`; иначе noauth.
    let proxy_url = match (socks_username, socks_password) {
        (Some(user), Some(pass)) if !user.is_empty() && !pass.is_empty() => {
            format!("socks5://{user}:{pass}@127.0.0.1:{socks_port}")
        }
        _ => format!("socks5://127.0.0.1:{socks_port}"),
    };
    eprintln!(
        "[helper-tun] запускаем tun2socks → {} (auth: {}), лог: {}",
        // Замокировано для логов чтобы пароль не попадал в plaintext-лог
        if socks_username.is_some() {
            format!("socks5://***:***@127.0.0.1:{socks_port}")
        } else {
            format!("socks5://127.0.0.1:{socks_port}")
        },
        socks_username.is_some(),
        log_path.display()
    );
    // ВНИМАНИЕ: НЕ ПЕРЕДАЁМ `-interface` сюда. tun2socks подключается
    // только к 127.0.0.1:1080 (Xray SOCKS5), это loopback и не зависит от
    // default route. А с `-interface Ethernet` tun2socks вешает на свой
    // UDP-сокет `IP_UNICAST_IF=Ethernet` — и Windows возвращает ошибку
    // `wsasendto: address not valid in its context` при попытке отправить
    // на 127.0.0.1 (loopback недостижим через Ethernet). Это рушит ВСЁ
    // UDP-проксирование, включая DNS — каждый DNS-запрос терялся, Windows
    // resolver уходил в retry-цикл и давал ~15с задержку на первом запросе.
    let child = Command::new(tun2socks_exe)
        .args([
            "-device",
            &format!("tun://{tun_name}"),
            "-proxy",
            &proxy_url,
            "-loglevel",
            "debug",
        ])
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_clone))
        .spawn()
        .context("не удалось запустить tun2socks")?;

    t.step("tun2socks spawned");

    // 6. Ждём появления TUN-интерфейса (15 сек — впервые WinTUN может
    //    инициализироваться долго).
    let tun_index = match routing::wait_for_interface(&tun_name, Duration::from_secs(15)).await {
        Ok(idx) => idx,
        Err(e) => {
            kill_child(child).await;
            // Прикладываем хвост лога tun2socks к ошибке для диагностики.
            let log_tail = std::fs::read_to_string(&log_path)
                .map(|s| s.lines().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"))
                .unwrap_or_else(|_| "(лог tun2socks недоступен)".to_string());
            return Err(e).context(format!("TUN не поднялся за 15 сек. tun2socks log:\n{log_tail}"));
        }
    };
    t.step(&format!("wait_for_interface ({tun_name}, ifIndex {tun_index})"));

    // 6a. Назначить IP на TUN — без него ОС считает интерфейс «без подсети»
    //     и `CreateIpForwardEntry2` для NextHop=198.18.0.1 возвращает 1168.
    let tun_ip: std::net::Ipv4Addr = TUN_GATEWAY.parse().unwrap();
    let mut child = child;
    if let Some(status) = child.try_wait().context("проверка статуса tun2socks")? {
        let log_tail = std::fs::read_to_string(&log_path)
            .map(|s| s.lines().rev().take(30).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"))
            .unwrap_or_else(|_| "(лог tun2socks недоступен)".to_string());
        bail!(
            "tun2socks умер сразу после старта (exit {status}). Лог:\n{log_tail}"
        );
    }
    if let Err(e) = routing::assign_ip(tun_index, tun_ip, TUN_PREFIX_LEN).await {
        kill_child(child).await;
        return Err(e).context("assign_ip на TUN");
    }
    t.step(&format!("ip {TUN_GATEWAY}/{TUN_PREFIX_LEN} назначен"));

    // 6. Bypass-route на VPN-сервер через старый gateway
    if let Err(e) = routing::add_route(
        &server_ip,
        HOST_MASK,
        &original.gateway,
        1,
        Some(original.if_index),
    )
    .await
    {
        let _ = full_route_cleanup(&server_ip).await;
        kill_child(child).await;
        return Err(e).context("bypass-route на сервер");
    }
    t.step("bypass-route added");

    // 6a. Bypass-route на DNS-сервер тоже через старый gateway. Иначе
    //     DNS-запросы идут через TUN → tun2socks → Xray → VPN-сервер,
    //     и при первом коннекте Xray ещё не успел раскачаться, в итоге
    //     Windows resolver висит ~15 секунд по timeout. Bypass даёт
    //     быстрый resolve ценой DNS-leak (стандартная практика VPN).
    if let Err(e) = routing::add_route(
        dns,
        HOST_MASK,
        &original.gateway,
        1,
        Some(original.if_index),
    )
    .await
    {
        eprintln!("[helper-tun] bypass на DNS не удался ({e:#}) — продолжаем без него");
    } else {
        t.step("bypass-route на DNS added");
    }

    // 7. Half-default routes через TUN
    if let Err(e) = routing::add_route(HALF_LOW_DST, HALF_MASK, TUN_GATEWAY, 1, Some(tun_index)).await {
        let _ = full_route_cleanup(&server_ip).await;
        kill_child(child).await;
        return Err(e).context("half-route 0.0.0.0/1");
    }
    if let Err(e) = routing::add_route(HALF_HIGH_DST, HALF_MASK, TUN_GATEWAY, 1, Some(tun_index)).await {
        let _ = full_route_cleanup(&server_ip).await;
        kill_child(child).await;
        return Err(e).context("half-route 128.0.0.0/1");
    }
    t.step("half-routes added");

    // 8. DNS на TUN-интерфейсе. На physic не трогаем — это конфликтовало
    //     с Windows-resolver и удлиняло первый запрос. Bypass-route на DNS
    //     (см. шаг 6a) уже даёт быстрый отклик на 1.1.1.1 минуя VPN.
    if let Err(e) = routing::set_dns(tun_index, dns).await {
        eprintln!("[helper-tun] DNS на TUN не выставился ({e:#}) — продолжаем без него");
    }
    t.step("dns set on TUN");

    // 8b. Сбросить кеш резолвера: иначе старые ответы будут пытаться
    //     достучаться до cached IP через теперь-уже-другой routing.
    if let Err(e) = routing::flush_dns_cache().await {
        eprintln!("[helper-tun] flush_dns_cache failed: {e:#}");
    }
    t.step("dns cache flushed");

    *g = Some(State {
        child,
        server_ip,
        dns: dns.to_string(),
        original,
        tun_index,
        tun_name,
    });

    eprintln!("[helper-tun] TUN-режим активен (полное время старта см. выше)");
    Ok(())
}

pub async fn stop() -> Result<()> {
    let mut g = STATE.lock().await;
    let state = g
        .take()
        .ok_or_else(|| anyhow!("TUN-режим не запущен"))?;

    // 1. Сначала маршруты — чтобы трафик уже шёл напрямую пока убиваем tun2socks.
    let _ = full_route_cleanup(&state.server_ip).await;
    let _ = routing::delete_route(&state.dns, HOST_MASK).await;

    // 2. Снять IP с TUN (если интерфейс ещё существует).
    let tun_ip: std::net::Ipv4Addr = TUN_GATEWAY.parse().unwrap();
    let _ = routing::unassign_ip(state.tun_index, tun_ip, TUN_PREFIX_LEN).await;

    // 3. Сбросить DNS-кеш чтобы старые ответы (через TUN-DNS) не зависли.
    let _ = routing::flush_dns_cache().await;

    // 4. Убить tun2socks (он сам уберёт WinTUN-интерфейс).
    kill_child(state.child).await;

    eprintln!("[helper-tun] TUN-режим остановлен");
    Ok(())
}

/// Удалить все маршруты которые мы добавляли при start. Идемпотентно.
async fn full_route_cleanup(server_ip: &str) {
    let _ = routing::delete_route(HALF_HIGH_DST, HALF_MASK).await;
    let _ = routing::delete_route(HALF_LOW_DST, HALF_MASK).await;
    let _ = routing::delete_route(server_ip, HOST_MASK).await;
}

async fn kill_child(mut child: Child) {
    if let Err(e) = child.kill().await {
        eprintln!("[helper-tun] не удалось убить tun2socks: {e}");
    }
    let _ = child.wait().await;
}
