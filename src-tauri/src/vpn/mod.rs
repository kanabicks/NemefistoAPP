//! VPN-логика: state machine коннекта, управление Xray sidecar и TUN.
//!
//! На Этапе 1 здесь только обёртка над Xray-процессом. На Этапе 5 модуль
//! расширится до полноценной state machine (Idle → Warming → Ready → ...).

mod xray;

pub use xray::XrayState;
