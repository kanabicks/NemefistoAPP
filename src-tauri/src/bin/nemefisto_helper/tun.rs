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

const TUN_NAME: &str = "nemefisto";
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
    /// Сохраняем оригинальный default-route чтобы ssh-debugging знал что было.
    /// Удалять / восстанавливать его не нужно (мы используем half-routes).
    original: DefaultRoute,
    tun_index: u32,
}

static STATE: Mutex<Option<State>> = Mutex::const_new(None);

pub async fn start(
    socks_port: u16,
    server_host: &str,
    dns: &str,
    tun2socks_path: &str,
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

    // 5. Спавн tun2socks. Лог пишем в файл — увидим причину если интерфейс
    //    не поднимется. Уровень debug для диагностики.
    eprintln!(
        "[helper-tun] запускаем tun2socks → socks5://127.0.0.1:{socks_port}, лог: {}",
        log_path.display()
    );
    let child = Command::new(tun2socks_exe)
        .args([
            "-device",
            &format!("tun://{TUN_NAME}"),
            "-proxy",
            &format!("socks5://127.0.0.1:{socks_port}"),
            // Имя физического интерфейса — для bypass самого tun2socks
            // (чтобы он подключался к Xray не через свой же TUN).
            "-interface",
            &original.interface_name,
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
    let tun_index = match routing::wait_for_interface(TUN_NAME, Duration::from_secs(15)).await {
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
    t.step(&format!("wait_for_interface ({TUN_NAME}, ifIndex {tun_index})"));

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

    // 8. DNS на TUN (не fatal — DNS leak возможен но трафик пойдёт)
    if let Err(e) = routing::set_dns(tun_index, dns).await {
        eprintln!("[helper-tun] DNS на TUN не выставился ({e:#}) — продолжаем без него");
    }
    t.step("dns set");

    *g = Some(State {
        child,
        server_ip,
        original,
        tun_index,
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

    // 2. Снять IP с TUN (если интерфейс ещё существует).
    let tun_ip: std::net::Ipv4Addr = TUN_GATEWAY.parse().unwrap();
    let _ = routing::unassign_ip(state.tun_index, tun_ip, TUN_PREFIX_LEN).await;

    // 3. Убить tun2socks (он сам уберёт WinTUN-интерфейс).
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
