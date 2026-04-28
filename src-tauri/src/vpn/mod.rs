//! VPN-логика: управление Xray sidecar, поиск свободных портов, пинги.

mod ping;
mod xray;

pub use ping::ping_entry;
pub use xray::{find_free_port, XrayState};
