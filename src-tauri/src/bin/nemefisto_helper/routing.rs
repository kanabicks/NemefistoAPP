//! Wrapper над системными командами Windows для управления маршрутизацией:
//!
//! - Получение текущего default-route (gateway + ifIndex + имя интерфейса)
//! - Резолв host → IPv4
//! - Ожидание появления сетевого интерфейса по имени
//! - Добавление / удаление маршрута через `route`
//! - Установка DNS на интерфейс через `Set-DnsClientServerAddress`
//!
//! Все операции бросают `anyhow::Error` с описанием. Удаление маршрута
//! идемпотентно — отсутствие маршрута не считается ошибкой.

use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use tokio::process::Command as AsyncCommand;

#[derive(Debug, Clone)]
pub struct DefaultRoute {
    pub gateway: String,
    pub if_index: u32,
    pub interface_name: String,
}

#[derive(Deserialize)]
struct DefaultRouteRaw {
    gateway: String,
    if_index: u32,
    interface_name: String,
}

/// Прочитать текущий IPv4 default-route с минимальной метрикой.
/// Используется для двух целей:
///   1. Имя физического интерфейса (для tun2socks `-interface`).
///   2. Шлюз для bypass-route на сам VPN-сервер.
pub async fn get_default_route() -> Result<DefaultRoute> {
    // Берём все default-routes (NextHop != 0.0.0.0), сортируем по сумме
    // RouteMetric + ifMetric, берём первый. Это совпадает с логикой
    // выбора маршрута Windows.
    let script = r#"
$r = Get-NetRoute -DestinationPrefix '0.0.0.0/0' -AddressFamily IPv4 -ErrorAction SilentlyContinue |
      Where-Object { $_.NextHop -ne '0.0.0.0' -and $_.NextHop -ne '::' } |
      Sort-Object -Property @{ Expression = { $_.RouteMetric + $_.InterfaceMetric }} |
      Select-Object -First 1
if (-not $r) { exit 1 }
$a = Get-NetAdapter -InterfaceIndex $r.InterfaceIndex -ErrorAction Stop
@{
    gateway = $r.NextHop
    if_index = [int]$r.InterfaceIndex
    interface_name = $a.Name
} | ConvertTo-Json -Compress
"#;
    let out = run_powershell(script).await
        .context("не удалось получить current default route")?;

    let raw: DefaultRouteRaw = serde_json::from_str(out.trim())
        .with_context(|| format!("невалидный JSON от PS: {out:?}"))?;

    Ok(DefaultRoute {
        gateway: raw.gateway,
        if_index: raw.if_index,
        interface_name: raw.interface_name,
    })
}

/// Резолвит хост в IPv4. Если уже IP — возвращает как есть.
pub async fn resolve_host_ipv4(host: &str) -> Result<String> {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(host.to_string());
    }
    let host_owned = host.to_string();
    let ip = tokio::task::spawn_blocking(move || -> Result<String> {
        use std::net::ToSocketAddrs;
        let addrs = (host_owned.as_str(), 443u16)
            .to_socket_addrs()
            .with_context(|| format!("резолв {host_owned} не удался"))?;
        for a in addrs {
            if let std::net::IpAddr::V4(v4) = a.ip() {
                return Ok(v4.to_string());
            }
        }
        Err(anyhow!("резолв {host_owned} не дал ни одного IPv4"))
    })
    .await??;
    Ok(ip)
}

/// Дождаться появления интерфейса с заданным именем. Возвращает ifIndex.
pub async fn wait_for_interface(name: &str, timeout: Duration) -> Result<u32> {
    let deadline = Instant::now() + timeout;
    let script = format!(
        r#"$a = Get-NetAdapter -Name '{name}' -ErrorAction SilentlyContinue
           if ($a) {{ $a.ifIndex }}"#
    );
    loop {
        if let Ok(out) = run_powershell(&script).await {
            if let Ok(idx) = out.trim().parse::<u32>() {
                return Ok(idx);
            }
        }
        if Instant::now() >= deadline {
            bail!("интерфейс «{name}» не появился за {:?}", timeout);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Добавить маршрут через `route add`. `mask` для /32 = "255.255.255.255".
pub async fn add_route(
    destination: &str,
    mask: &str,
    gateway: &str,
    metric: u16,
    if_index: Option<u32>,
) -> Result<()> {
    let mut args: Vec<String> = vec![
        "add".into(),
        destination.into(),
        "mask".into(),
        mask.into(),
        gateway.into(),
        "metric".into(),
        metric.to_string(),
    ];
    if let Some(i) = if_index {
        args.push("if".into());
        args.push(i.to_string());
    }

    let output = AsyncCommand::new("route")
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("route add не удалось запустить")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!("route add {destination}: {} {}", stderr.trim(), stdout.trim());
    }
    Ok(())
}

/// Удалить маршрут. Если маршрута нет — игнорируем (идемпотентно).
pub async fn delete_route(destination: &str, mask: &str) -> Result<()> {
    let _ = AsyncCommand::new("route")
        .args(["delete", destination, "mask", mask])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
    Ok(())
}

/// Поставить DNS-сервер на сетевой интерфейс.
pub async fn set_dns(if_index: u32, dns: &str) -> Result<()> {
    let script = format!(
        r#"Set-DnsClientServerAddress -InterfaceIndex {if_index} -ServerAddresses '{dns}' -ErrorAction Stop"#
    );
    run_powershell(&script).await.context("Set-DnsClientServerAddress")?;
    Ok(())
}

async fn run_powershell(script: &str) -> Result<String> {
    let output = AsyncCommand::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("powershell не запустился")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("powershell exit={}: {}", output.status, stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
