//! Маршрутизатор JSON-RPC команд helper-сервиса.
//!
//! Вся бизнес-логика (запуск tun2socks, манипуляции с routing) в подмодулях
//! `tun.rs` и `routing.rs`. Здесь — только switch + конверсия ошибок в
//! `Response::Error`.

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
    }
}
