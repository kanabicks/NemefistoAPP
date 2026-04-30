//! Платформо-специфичный код.
//!
//! Всё Windows-зависимое изолировано здесь — для упрощения будущего портирования.

pub mod autostart;
pub mod helper_bootstrap;
pub mod helper_client;
pub mod network;
pub mod network_watcher;
pub mod proxy;
pub mod secure_storage;
pub mod tray;
