//! Управление конфигурацией: HWID, серверы, подписки, генерация
//! sing-box / Mihomo конфигов.

pub mod geofiles;
pub mod hwid;
pub mod mihomo_config;
pub mod routing_profile;
pub mod routing_store;
pub mod server;
pub mod sing_box_config;
pub mod subscription;

pub use hwid::HwidState;
pub use server::ProxyEntry;
pub use subscription::SubscriptionState;
