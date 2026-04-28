//! VPN-логика: управление Xray sidecar, поиск свободных портов.

mod xray;

pub use xray::{find_free_port, XrayState};
