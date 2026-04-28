//! SECURITY_ATTRIBUTES для named pipe который должен быть доступен
//! user-mode процессам. По умолчанию pipe, созданный сервисом от SYSTEM,
//! доступен только администраторам.
//!
//! Мы используем NULL DACL — это явно «доступ разрешён всем локальным
//! пользователям». Атакующий локальный non-admin пользователь сможет слать
//! команды helper-у, но это базовое ограничение любого VPN-клиента: тот,
//! кто залогинен, и так может управлять сетью своего профиля.
//!
//! Альтернатива (более безопасная) — DACL разрешающий только Authenticated
//! Users группе, либо только конкретным SID-ам. Сделаем при необходимости.

use std::ffi::c_void;
use std::mem::size_of;

use windows_sys::Win32::Security::{
    InitializeSecurityDescriptor, SetSecurityDescriptorDacl, PSECURITY_DESCRIPTOR,
    SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR,
};

const SECURITY_DESCRIPTOR_REVISION: u32 = 1;

/// Контейнер для SECURITY_DESCRIPTOR + SECURITY_ATTRIBUTES со стабильным адресом.
/// Передаётся в `ServerOptions::create_with_security_attributes_raw`.
pub struct PipeSecurity {
    /// SD держим живым: на него указывает `sa.lpSecurityDescriptor`.
    /// Само поле не читается — отсюда `allow(dead_code)`.
    #[allow(dead_code)]
    sd: Box<SECURITY_DESCRIPTOR>,
    sa: Box<SECURITY_ATTRIBUTES>,
}

impl PipeSecurity {
    /// NULL DACL: все процессы на этой машине могут open-ить pipe.
    pub fn permissive() -> Self {
        // Инициализируем SD и устанавливаем NULL DACL (это **не** «empty DACL»
        // который блокирует всех — это явный «нет ограничений»).
        let mut sd: Box<SECURITY_DESCRIPTOR> = Box::new(unsafe { std::mem::zeroed() });
        unsafe {
            let psd: PSECURITY_DESCRIPTOR = sd.as_mut() as *mut _ as PSECURITY_DESCRIPTOR;
            // BOOL = i32 в windows-sys; 1 = TRUE
            assert_ne!(InitializeSecurityDescriptor(psd, SECURITY_DESCRIPTOR_REVISION), 0,
                "InitializeSecurityDescriptor failed");
            assert_ne!(SetSecurityDescriptorDacl(psd, 1, std::ptr::null_mut(), 0), 0,
                "SetSecurityDescriptorDacl failed");
        }

        let sa = Box::new(SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.as_mut() as *mut _ as *mut c_void,
            bInheritHandle: 0,
        });

        Self { sd, sa }
    }

    /// Указатель на SECURITY_ATTRIBUTES для tokio API.
    pub fn as_attrs_ptr(&mut self) -> *mut c_void {
        self.sa.as_mut() as *mut _ as *mut c_void
    }
}
