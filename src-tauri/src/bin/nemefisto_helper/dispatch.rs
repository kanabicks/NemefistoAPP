//! Маршрутизатор JSON-RPC команд helper-сервиса.
//!
//! Вся бизнес-логика (запуск tun2socks, манипуляции с routing) в подмодулях
//! `tun.rs` и `routing.rs`. Здесь — только switch + конверсия ошибок в
//! `Response::Error`.

use super::firewall;
use super::helper_log::log as hlog;
use super::mihomo;
use super::protocol::{Request, Response, PROTOCOL_VERSION};
use super::sing_box;
use super::wfp;

const HELPER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub async fn handle(req: Request) -> Response {
    match req {
        Request::Ping => Response::Pong,
        Request::Version => Response::Version {
            version: HELPER_VERSION.to_string(),
            protocol_version: PROTOCOL_VERSION,
        },
        Request::KillSwitchEnable {
            server_ips,
            allow_lan,
            allow_app_paths,
            block_dns,
            allow_dns_ips,
            strict_mode,
            expect_tun,
        } => {
            let paths: Vec<std::path::PathBuf> = allow_app_paths
                .into_iter()
                .map(std::path::PathBuf::from)
                .collect();
            // 13.D step A: в TUN-режиме ищем активный WinTUN-адаптер для
            // allow-фильтра — без него user-трафик идущий через TUN
            // блокируется (sing-box/mihomo.exe allow покрывает только их
            // собственные шифрованные пакеты к серверу, не proxied трафик
            // user-app'ов). В proxy-режиме (`expect_tun=false`) skip без
            // retry-задержки.
            let tun_if = super::tun::current_tun_interface_index(expect_tun).await;
            if expect_tun && tun_if.is_none() {
                hlog(
                    "[helper-dispatch] WARN: TUN-режим активен, но WinTUN-адаптер не найден — kill-switch заблокирует user-трафик",
                );
            }
            match firewall::enable(
                server_ips,
                allow_lan,
                paths,
                block_dns,
                allow_dns_ips,
                tun_if,
                strict_mode,
            )
            .await
            {
                Ok(()) => Response::Ok,
                Err(e) => Response::err(format!("kill_switch_enable: {e:#}")),
            }
        }
        Request::KillSwitchDisable => match firewall::disable().await {
            Ok(()) => Response::Ok,
            Err(e) => Response::err(format!("kill_switch_disable: {e:#}")),
        },
        Request::KillSwitchHeartbeat => {
            firewall::heartbeat();
            Response::Ok
        }
        Request::KillSwitchForceCleanup => match firewall::disable().await {
            Ok(()) => Response::Ok,
            Err(e) => Response::err(format!("kill_switch_force_cleanup: {e:#}")),
        },
        Request::OrphanCleanup => {
            super::tun::cleanup_orphan_resources().await;
            Response::Ok
        }
        Request::WfpQueryOrphan => match wfp::has_orphan_filters() {
            Ok(has_orphan) => Response::WfpOrphan { has_orphan },
            Err(e) => Response::err(format!("wfp_query_orphan: {e:#}")),
        },
        Request::MihomoStart {
            config_path,
            mihomo_exe_path,
            data_dir,
        } => match mihomo::start(&config_path, &mihomo_exe_path, &data_dir).await {
            Ok(()) => Response::Ok,
            Err(e) => Response::err(format!("mihomo_start: {e:#}")),
        },
        Request::MihomoStop => match mihomo::stop().await {
            Ok(()) => Response::Ok,
            Err(e) => Response::err(format!("mihomo_stop: {e:#}")),
        },
        Request::SingBoxStart {
            config_path,
            singbox_exe_path,
            data_dir,
        } => match sing_box::start(&config_path, &singbox_exe_path, &data_dir).await {
            Ok(()) => Response::Ok,
            Err(e) => Response::err(format!("sing_box_start: {e:#}")),
        },
        Request::SingBoxStop => match sing_box::stop().await {
            Ok(()) => Response::Ok,
            Err(e) => Response::err(format!("sing_box_stop: {e:#}")),
        },
    }
}
