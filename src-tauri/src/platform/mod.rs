//! Платформо-специфичный код.
//!
//! Всё Windows-зависимое изолировано здесь — для упрощения будущего портирования.

pub mod helper_bootstrap;
pub mod helper_client;
pub mod network;
pub mod proxy;
