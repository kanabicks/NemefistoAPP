//! Управление конфигурацией: HWID, серверы, подписки.

pub mod hwid;
pub mod server;
pub mod subscription;

pub use hwid::HwidState;
pub use server::ProxyEntry;
pub use subscription::SubscriptionState;
