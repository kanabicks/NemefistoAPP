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
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use super::routing::{self, DefaultRoute};

const TUN_NAME: &str = "nemefisto";
/// Адрес шлюза TUN-интерфейса. tun2socks по умолчанию даёт TUN адрес
/// `198.18.0.1/15`, и этот же IP мы указываем как `gateway` для маршрутов.
const TUN_GATEWAY: &str = "198.18.0.1";
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
    let mut g = STATE.lock().await;
    if g.is_some() {
        bail!("TUN-режим уже запущен");
    }

    // 1. Проверка пути к tun2socks
    let tun2socks_exe = Path::new(tun2socks_path);
    if !tun2socks_exe.is_file() {
        bail!("tun2socks не найден по пути: {tun2socks_path}");
    }

    // 2. Резолв server_ip
    let server_ip = routing::resolve_host_ipv4(server_host)
        .await
        .with_context(|| format!("резолв {server_host}"))?;

    // 3. Текущий default route
    let original = routing::get_default_route()
        .await
        .context("чтение default-route")?;
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
    eprintln!("[helper-tun] TUN-интерфейс {TUN_NAME} поднят (ifIndex {tun_index})");

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

    // 8. DNS на TUN (не fatal — DNS leak возможен но трафик пойдёт)
    if let Err(e) = routing::set_dns(tun_index, dns).await {
        eprintln!("[helper-tun] DNS на TUN не выставился ({e:#}) — продолжаем без него");
    }

    *g = Some(State {
        child,
        server_ip,
        original,
        tun_index,
    });

    eprintln!("[helper-tun] TUN-режим активен");
    Ok(())
}

pub async fn stop() -> Result<()> {
    let mut g = STATE.lock().await;
    let state = g
        .take()
        .ok_or_else(|| anyhow!("TUN-режим не запущен"))?;

    // Сначала маршруты — чтобы трафик уже шёл напрямую пока убиваем tun2socks.
    let _ = full_route_cleanup(&state.server_ip).await;

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
