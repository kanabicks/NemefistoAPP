//! VPN-логика: управление Xray + Mihomo sidecar, поиск свободных портов, пинги.

pub mod leak_test;
mod mihomo;
mod ping;
mod xray;

pub use mihomo::MihomoState;
pub use ping::ping_entry;
pub use xray::{find_free_port, random_high_port, XrayState};
