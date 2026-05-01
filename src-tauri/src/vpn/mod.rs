//! VPN-логика: управление Xray + Mihomo sidecar, поиск свободных портов, пинги.

pub mod leak_test;
mod mihomo;
pub mod mihomo_api;
mod ping;
mod xray;

pub use mihomo::MihomoState;
pub use mihomo_api::{ControllerEndpoint, MihomoApiState};
pub use ping::ping_entry;
pub use xray::{find_free_port, random_high_port, XrayState};
