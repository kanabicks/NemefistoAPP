//! Управление конфигурацией: HWID, серверы, подписки, генерация Xray-конфигов.

pub mod hwid;
pub mod server;
pub mod subscription;
pub mod xray_config;

pub use hwid::HwidState;
pub use server::ProxyEntry;
pub use subscription::SubscriptionState;
