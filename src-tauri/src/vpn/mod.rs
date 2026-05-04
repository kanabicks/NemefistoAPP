//! VPN-логика: управление Xray + Mihomo sidecar, поиск свободных портов, пинги.

pub mod connection_ping;
pub mod leak_test;
mod mihomo;
pub mod mihomo_api;
mod ping;
mod sing_box;
mod ports;

pub use mihomo::MihomoState;
pub use mihomo_api::{ControllerEndpoint, MihomoApiState};
pub use ping::ping_entry;
pub use ports::{find_free_port, random_high_port};
pub use sing_box::SingBoxState;
