//! Kill switch через Windows Firewall (этап 6.D / 13.D).
//!
//! Минимальная реализация: меняем default outbound policy всех профилей
//! Windows Firewall на `block`, и добавляем allow-rules для loopback,
//! LAN, IP VPN-сервера и нескольких public DNS (для resolv'а в момент
//! reconnect). При выключении — восстанавливаем `allow` policy и
//! удаляем наши rules.
//!
//! Преимущество перед block-all firewall rule: tun2socks и xray тоже
//! пишут трафик, и нам нужно их пропустить. С default-block policy +
//! allow-rules это получается естественно.
//!
//! ⚠️ Risk: если приложение крашнется когда kill switch активен,
//! default-policy останется `blockoutbound` → у пользователя нет
//! интернета. Восстановление вручную через admin-PowerShell:
//! `netsh advfirewall set allprofiles firewallpolicy allowinbound,allowoutbound`
//! и удаление NemefistoKillSwitch_* rules.
//!
//! TODO 13.D: переписать через WFP API напрямую (Windows Filtering
//! Platform) — не требует менять глобальную firewall policy, более
//! гранулярный контроль.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::process::Command;

const RULE_PREFIX: &str = "NemefistoKillSwitch_";
const NETSH_TIMEOUT: Duration = Duration::from_secs(10);

/// Выполнить `netsh ...` с таймаутом и собрать stderr для context.
async fn run_netsh(args: &[&str]) -> Result<()> {
    let mut cmd = Command::new("netsh");
    cmd.args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let child = cmd.spawn().context("не удалось запустить netsh")?;
    let output = tokio::time::timeout(NETSH_TIMEOUT, child.wait_with_output())
        .await
        .context("netsh таймаут")?
        .context("ожидание netsh")?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        bail!(
            "netsh {} завершился с кодом {:?}: {}",
            args.join(" "),
            output.status.code(),
            err.trim()
        );
    }
    Ok(())
}

/// Включить kill switch:
/// - default outbound policy = block;
/// - allow loopback / LAN / VPN-сервер / 1.1.1.1+8.8.8.8.
pub async fn enable(server_ip: &str) -> Result<()> {
    eprintln!("[firewall] kill switch ON, allow VPN={server_ip}");

    // Сначала добавляем allow-rules — пока default policy ещё allowoutbound,
    // это просто избыточные правила. После смены policy на block они
    // становятся «islands of permission».
    run_netsh(&[
        "advfirewall",
        "firewall",
        "add",
        "rule",
        &format!("name={RULE_PREFIX}Loopback"),
        "dir=out",
        "action=allow",
        "remoteip=127.0.0.0/8",
        "enable=yes",
    ])
    .await?;

    run_netsh(&[
        "advfirewall",
        "firewall",
        "add",
        "rule",
        &format!("name={RULE_PREFIX}LAN"),
        "dir=out",
        "action=allow",
        "remoteip=LocalSubnet",
        "enable=yes",
    ])
    .await?;

    run_netsh(&[
        "advfirewall",
        "firewall",
        "add",
        "rule",
        &format!("name={RULE_PREFIX}VPN"),
        "dir=out",
        "action=allow",
        &format!("remoteip={server_ip}"),
        "enable=yes",
    ])
    .await?;

    // DNS — Cloudflare и Google. Нужно для повторного резолва VPN-сервера
    // если он по домену (когда мы сюда передаём IP, эти rules — страховка
    // на случай если IP сменился в подписке).
    run_netsh(&[
        "advfirewall",
        "firewall",
        "add",
        "rule",
        &format!("name={RULE_PREFIX}DNS_Cloudflare"),
        "dir=out",
        "action=allow",
        "remoteip=1.1.1.1",
        "enable=yes",
    ])
    .await?;

    run_netsh(&[
        "advfirewall",
        "firewall",
        "add",
        "rule",
        &format!("name={RULE_PREFIX}DNS_Google"),
        "dir=out",
        "action=allow",
        "remoteip=8.8.8.8",
        "enable=yes",
    ])
    .await?;

    // Меняем дефолт-policy всех профилей на blockoutbound.
    run_netsh(&[
        "advfirewall",
        "set",
        "allprofiles",
        "firewallpolicy",
        "blockinbound,blockoutbound",
    ])
    .await?;

    Ok(())
}

/// Выключить kill switch: восстановить default-allow и удалить наши rules.
pub async fn disable() -> Result<()> {
    eprintln!("[firewall] kill switch OFF");

    // Сначала возвращаем дефолт-policy чтобы у пользователя сразу
    // появился интернет (даже если последующие удаления rules упадут).
    let _ = run_netsh(&[
        "advfirewall",
        "set",
        "allprofiles",
        "firewallpolicy",
        "blockinbound,allowoutbound",
    ])
    .await;

    // Удаляем наши rules. Каждый delete независим — если какого-то rule
    // нет (повторный disable / частичный enable), errors игнорируем.
    let rules = [
        "Loopback",
        "LAN",
        "VPN",
        "DNS_Cloudflare",
        "DNS_Google",
    ];
    for r in rules {
        let _ = run_netsh(&[
            "advfirewall",
            "firewall",
            "delete",
            "rule",
            &format!("name={RULE_PREFIX}{r}"),
        ])
        .await;
    }

    Ok(())
}
