//! Платформо-специфичный код.
//!
//! Всё Windows-зависимое изолировано здесь — для упрощения будущего портирования.

pub mod autostart;
pub mod bandwidth;
pub mod crash_dumps;
pub mod helper_bootstrap;
pub mod helper_client;
pub mod network;
pub mod network_watcher;
pub mod process_memory;
pub mod processes;
pub mod proxy;
pub mod session_lock;
pub mod secure_storage;
pub mod tray;
