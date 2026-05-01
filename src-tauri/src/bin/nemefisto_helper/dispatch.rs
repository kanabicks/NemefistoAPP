//! Маршрутизатор JSON-RPC команд helper-сервиса.
//!
//! Вся бизнес-логика (запуск tun2socks, манипуляции с routing) в подмодулях
//! `tun.rs` и `routing.rs`. Здесь — только switch + конверсия ошибок в
//! `Response::Error`.

use super::firewall;
use super::protocol::{Request, Response};
use super::tun;

const HELPER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub async fn handle(req: Request) -> Response {
    match req {
        Request::Ping => Response::Pong,
        Request::Version => Response::Version {
            version: HELPER_VERSION.to_string(),
        },
        Request::TunStart {
            socks_port,
            server_host,
            dns,
            tun2socks_path,
            socks_username,
            socks_password,
            tun_name_override,
        } => match tun::start(
            socks_port,
            &server_host,
            &dns,
            &tun2socks_path,
            socks_username.as_deref(),
            socks_password.as_deref(),
            tun_name_override.as_deref(),
        )
        .await
        {
            Ok(()) => Response::Ok,
            Err(e) => Response::err(format!("tun_start: {e:#}")),
        },
        Request::TunStop => match tun::stop().await {
            Ok(()) => Response::Ok,
            Err(e) => Response::err(format!("tun_stop: {e:#}")),
        },
        Request::KillSwitchEnable {
            server_ips,
            allow_lan,
            allow_app_paths,
            block_dns,
            allow_dns_ips,
        } => {
            let paths: Vec<std::path::PathBuf> = allow_app_paths
                .into_iter()
                .map(std::path::PathBuf::from)
                .collect();
            // 13.D step A: если TUN-режим активен, добавляем allow для
            // его interface index. helper сам знает свой TUN — IPC не
            // расширяем. В proxy-режиме `tun_if` будет None.
            let tun_if = super::tun::current_tun_interface_index().await;
            match firewall::enable(
                server_ips,
                allow_lan,
                paths,
                block_dns,
                allow_dns_ips,
                tun_if,
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
    }
}
