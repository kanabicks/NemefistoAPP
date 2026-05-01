//! Безопасная Rust-обёртка над Windows Filtering Platform API (этап 13.D).
//!
//! WFP позволяет добавлять filter'ы на уровне ядра Windows для inbound/
//! outbound трафика. Используется для kill-switch'а: блокируем весь
//! исходящий трафик кроме явно разрешённого (loopback, LAN, VPN-сервер,
//! наши процессы).
//!
//! ## Защита от orphan-фильтров
//!
//! Самая опасная ситуация: helper упал с активными block-all фильтрами →
//! интернет у пользователя заблокирован до ручного вмешательства.
//! Защищаемся **тремя слоями**:
//!
//! 1. **DYNAMIC session** (`FWPM_SESSION_FLAG_DYNAMIC`) — все объекты
//!    (provider, sublayer, filters) добавленные в этой сессии умирают
//!    автоматически когда engine-handle закрывается. Если helper-процесс
//!    краш-нул, OS закрывает handles и WFP сама убирает наши фильтры.
//! 2. **Транзакции** (`FwpmTransactionBegin/Commit/Abort0`) — добавление
//!    идёт пачкой. При ошибке в середине — `Abort` откатывает уже
//!    добавленное, не оставляя half-applied state.
//! 3. **Cleanup на старте** (`cleanup_provider`) — при запуске helper'а
//!    в persistent-engine удаляем любые объекты с нашим
//!    `NEMEFISTO_PROVIDER_GUID` — страховка если DYNAMIC по какой-то
//!    причине не сработал (теоретически невозможно, но WFP — серьёзный
//!    API, перестраховка не лишняя).

use std::ffi::OsStr;
use std::iter::once;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;

use anyhow::{anyhow, bail, Context, Result};

use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
use windows_sys::Win32::NetworkManagement::WindowsFilteringPlatform::*;
use windows_sys::Win32::System::Rpc::RPC_C_AUTHN_WINNT;

/// GUID нашего provider'а — постоянная метка чтобы при cleanup мы могли
/// найти именно «наши» объекты, не задев чужие WFP-фильтры (Defender,
/// другие VPN, etc).
pub const NEMEFISTO_PROVIDER_GUID: GUID = GUID {
    data1: 0xc6f1_bd86,
    data2: 0xc5e9,
    data3: 0x4e7a,
    data4: [0x9d, 0x7a, 0x2d, 0x81, 0xd6, 0xe4, 0xa2, 0xc1],
};

/// GUID sublayer'а — наша группа фильтров. Высокий weight чтобы они
/// рассматривались ДО windows-default рулежа (например allow-all из
/// Mullvad/NordVPN если оба активны).
pub const NEMEFISTO_SUBLAYER_GUID: GUID = GUID {
    data1: 0xc6f1_bd87,
    data2: 0xc5e9,
    data3: 0x4e7a,
    data4: [0x9d, 0x7a, 0x2d, 0x81, 0xd6, 0xe4, 0xa2, 0xc2],
};

// Веса фильтров живут в firewall.rs — он единственный потребитель
// и сам решает какой weight присвоить какому правилу.

/// Преобразует Rust-строку в null-terminated UTF-16 (для PWSTR).
fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(once(0)).collect()
}

/// RAII-обёртка над WFP engine handle. Drop закрывает handle —
/// для DYNAMIC session это автоматически удаляет все добавленные объекты.
pub struct WfpEngine {
    handle: HANDLE,
}

impl WfpEngine {
    /// Открыть engine с DYNAMIC-флагом — фильтры этой сессии умирают
    /// при закрытии handle (в т.ч. при crash процесса).
    /// Используется для apply kill-switch'а.
    pub fn open_dynamic() -> Result<Self> {
        Self::open_internal(true)
    }

    /// Открыть persistent engine (без DYNAMIC). Используется только для
    /// cleanup_provider — чтобы найти и удалить orphan'ы с прошлых
    /// инкарнаций helper'а.
    pub fn open_persistent() -> Result<Self> {
        Self::open_internal(false)
    }

    fn open_internal(dynamic: bool) -> Result<Self> {
        unsafe {
            let mut session: FWPM_SESSION0 = std::mem::zeroed();
            if dynamic {
                session.flags = FWPM_SESSION_FLAG_DYNAMIC;
            }
            // displayData можно не заполнять для session — это metadata
            // для GUI-инструментов вроде wfp.exe, не для логики.

            let mut handle: HANDLE = ptr::null_mut();
            // RPC_C_AUTHN_WINNT (10) — рекомендация MSDN для FwpmEngineOpen0
            // на локальном engine. RPC_C_AUTHN_DEFAULT тоже работает,
            // но WINNT эксплицитнее.
            let rc = FwpmEngineOpen0(
                ptr::null(),
                RPC_C_AUTHN_WINNT as u32,
                ptr::null_mut(),
                &session,
                &mut handle,
            );
            if rc != ERROR_SUCCESS {
                bail!("FwpmEngineOpen0 failed: 0x{:08x}", rc);
            }
            Ok(Self { handle })
        }
    }

    /// Выполнить closure внутри WFP-транзакции. Commit при Ok, Abort при Err.
    /// Все наши фильтры добавляем под одной транзакцией — не получится
    /// half-applied state (например, есть block-all но не успели добавить
    /// allow-VPN — это бы заблокировало пользователя).
    pub fn transaction<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce(&Self) -> Result<()>,
    {
        unsafe {
            let rc = FwpmTransactionBegin0(self.handle, 0);
            if rc != ERROR_SUCCESS {
                bail!("FwpmTransactionBegin0 failed: 0x{:08x}", rc);
            }
        }
        match f(self) {
            Ok(()) => unsafe {
                let rc = FwpmTransactionCommit0(self.handle);
                if rc != ERROR_SUCCESS {
                    // Commit упал — пытаемся abort на всякий случай.
                    let _ = FwpmTransactionAbort0(self.handle);
                    bail!("FwpmTransactionCommit0 failed: 0x{:08x}", rc);
                }
                Ok(())
            },
            Err(e) => unsafe {
                let _ = FwpmTransactionAbort0(self.handle);
                Err(e)
            },
        }
    }

    /// Добавить provider. В DYNAMIC session не персистентен.
    pub fn add_provider(&self, key: GUID, name: &str) -> Result<()> {
        let name_w = to_wide(name);
        unsafe {
            let mut provider: FWPM_PROVIDER0 = std::mem::zeroed();
            provider.providerKey = key;
            provider.displayData.name = name_w.as_ptr() as *mut u16;

            let rc = FwpmProviderAdd0(self.handle, &provider, ptr::null_mut());
            if rc != ERROR_SUCCESS {
                bail!("FwpmProviderAdd0 failed: 0x{:08x}", rc);
            }
        }
        Ok(())
    }

    /// Добавить sublayer. Привязан к provider'у — при cleanup
    /// удаляется автоматически вместе с ним.
    pub fn add_sublayer(
        &self,
        key: GUID,
        provider_key: GUID,
        name: &str,
        weight: u16,
    ) -> Result<()> {
        let name_w = to_wide(name);
        // providerKey — указатель на mutable GUID. Делаем local copy.
        let mut provider_key_copy = provider_key;
        unsafe {
            let mut sublayer: FWPM_SUBLAYER0 = std::mem::zeroed();
            sublayer.subLayerKey = key;
            sublayer.displayData.name = name_w.as_ptr() as *mut u16;
            sublayer.providerKey = &mut provider_key_copy;
            sublayer.weight = weight;

            let rc = FwpmSubLayerAdd0(self.handle, &sublayer, ptr::null_mut());
            if rc != ERROR_SUCCESS {
                bail!("FwpmSubLayerAdd0 failed: 0x{:08x}", rc);
            }
        }
        Ok(())
    }

    /// Базовый billed: filter без conditions = match-all в layer.
    /// Используется для block-all fallback'а в самом низу sublayer'а.
    pub fn add_filter_block_all(
        &self,
        layer: GUID,
        sublayer_key: GUID,
        name: &str,
    ) -> Result<()> {
        // weight = 0 — самый низкий, любые allow-фильтры с >0 перебивают.
        self.add_filter(layer, sublayer_key, name, 0, FWP_ACTION_BLOCK, &mut [])
    }

    /// Allow-фильтр для IPv4 подсети (`addr/mask`). Адрес/маска в
    /// host byte order — `10.0.0.0` = `0x0A000000`.
    pub fn add_filter_allow_v4_subnet(
        &self,
        layer: GUID,
        sublayer_key: GUID,
        name: &str,
        weight: u8,
        addr: u32,
        mask: u32,
    ) -> Result<()> {
        let mut addr_mask = FWP_V4_ADDR_AND_MASK { addr, mask };
        let mut conditions: [FWPM_FILTER_CONDITION0; 1] = unsafe { [std::mem::zeroed()] };
        conditions[0].fieldKey = FWPM_CONDITION_IP_REMOTE_ADDRESS;
        conditions[0].matchType = FWP_MATCH_EQUAL;
        conditions[0].conditionValue.r#type = FWP_V4_ADDR_MASK;
        conditions[0].conditionValue.Anonymous.v4AddrMask = &mut addr_mask;
        self.add_filter(layer, sublayer_key, name, weight, FWP_ACTION_PERMIT, &mut conditions)
    }

    /// Allow для IPv6 подсети. `addr` — 16 байт, `prefix_length` 0..=128.
    pub fn add_filter_allow_v6_subnet(
        &self,
        layer: GUID,
        sublayer_key: GUID,
        name: &str,
        weight: u8,
        addr: [u8; 16],
        prefix_length: u8,
    ) -> Result<()> {
        let mut addr_mask = FWP_V6_ADDR_AND_MASK {
            addr,
            prefixLength: prefix_length,
        };
        let mut conditions: [FWPM_FILTER_CONDITION0; 1] = unsafe { [std::mem::zeroed()] };
        conditions[0].fieldKey = FWPM_CONDITION_IP_REMOTE_ADDRESS;
        conditions[0].matchType = FWP_MATCH_EQUAL;
        conditions[0].conditionValue.r#type = FWP_V6_ADDR_MASK;
        conditions[0].conditionValue.Anonymous.v6AddrMask = &mut addr_mask;
        self.add_filter(layer, sublayer_key, name, weight, FWP_ACTION_PERMIT, &mut conditions)
    }

    /// Allow для одного IPv4 адреса (хост, /32).
    pub fn add_filter_allow_v4_addr(
        &self,
        layer: GUID,
        sublayer_key: GUID,
        name: &str,
        weight: u8,
        addr: u32,
    ) -> Result<()> {
        self.add_filter_allow_v4_subnet(
            layer,
            sublayer_key,
            name,
            weight,
            addr,
            0xFFFF_FFFF,
        )
    }

    /// Allow для одного IPv6 адреса (/128).
    pub fn add_filter_allow_v6_addr(
        &self,
        layer: GUID,
        sublayer_key: GUID,
        name: &str,
        weight: u8,
        addr: [u8; 16],
    ) -> Result<()> {
        self.add_filter_allow_v6_subnet(layer, sublayer_key, name, weight, addr, 128)
    }

    /// Allow для одного IPv4 адреса + конкретного протокола+порта.
    /// Используется для DNS-leak protection: разрешаем VPN-DNS:53/UDP,
    /// потом блокируем все остальные :53.
    /// `protocol` — `IPPROTO_UDP=17` или `IPPROTO_TCP=6`.
    pub fn add_filter_allow_v4_addr_port_proto(
        &self,
        layer: GUID,
        sublayer_key: GUID,
        name: &str,
        weight: u8,
        addr: u32,
        port: u16,
        protocol: u8,
    ) -> Result<()> {
        let mut addr_mask = FWP_V4_ADDR_AND_MASK {
            addr,
            mask: 0xFFFF_FFFF,
        };
        let mut conditions: [FWPM_FILTER_CONDITION0; 3] = unsafe { [std::mem::zeroed(); 3] };
        conditions[0].fieldKey = FWPM_CONDITION_IP_REMOTE_ADDRESS;
        conditions[0].matchType = FWP_MATCH_EQUAL;
        conditions[0].conditionValue.r#type = FWP_V4_ADDR_MASK;
        conditions[0].conditionValue.Anonymous.v4AddrMask = &mut addr_mask;
        conditions[1].fieldKey = FWPM_CONDITION_IP_REMOTE_PORT;
        conditions[1].matchType = FWP_MATCH_EQUAL;
        conditions[1].conditionValue.r#type = FWP_UINT16;
        conditions[1].conditionValue.Anonymous.uint16 = port;
        conditions[2].fieldKey = FWPM_CONDITION_IP_PROTOCOL;
        conditions[2].matchType = FWP_MATCH_EQUAL;
        conditions[2].conditionValue.r#type = FWP_UINT8;
        conditions[2].conditionValue.Anonymous.uint8 = protocol;
        self.add_filter(layer, sublayer_key, name, weight, FWP_ACTION_PERMIT, &mut conditions)
    }

    /// Block по протоколу+порту без условия на адрес. Используется
    /// для DNS-leak: блокируем весь :53/UDP+TCP кроме того что
    /// разрешили выше с большим weight.
    pub fn add_filter_block_port_proto(
        &self,
        layer: GUID,
        sublayer_key: GUID,
        name: &str,
        weight: u8,
        port: u16,
        protocol: u8,
    ) -> Result<()> {
        let mut conditions: [FWPM_FILTER_CONDITION0; 2] = unsafe { [std::mem::zeroed(); 2] };
        conditions[0].fieldKey = FWPM_CONDITION_IP_REMOTE_PORT;
        conditions[0].matchType = FWP_MATCH_EQUAL;
        conditions[0].conditionValue.r#type = FWP_UINT16;
        conditions[0].conditionValue.Anonymous.uint16 = port;
        conditions[1].fieldKey = FWPM_CONDITION_IP_PROTOCOL;
        conditions[1].matchType = FWP_MATCH_EQUAL;
        conditions[1].conditionValue.r#type = FWP_UINT8;
        conditions[1].conditionValue.Anonymous.uint8 = protocol;
        self.add_filter(layer, sublayer_key, name, weight, FWP_ACTION_BLOCK, &mut conditions)
    }

    /// Allow для всего трафика через указанный сетевой интерфейс
    /// (по local interface index — IfIndex). Используется для
    /// per-interface kill-switch (step A): любой исходящий через
    /// TUN-адаптер автоматически разрешён, без необходимости
    /// перечислять IP сервера или app-id.
    ///
    /// Это Mullvad-style решение: вместо «allow если dest=server_ip»
    /// делаем «allow если ушло через TUN-адаптер».
    pub fn add_filter_allow_local_interface_index(
        &self,
        layer: GUID,
        sublayer_key: GUID,
        name: &str,
        weight: u8,
        if_index: u32,
    ) -> Result<()> {
        let mut conditions: [FWPM_FILTER_CONDITION0; 1] = unsafe { [std::mem::zeroed()] };
        conditions[0].fieldKey = FWPM_CONDITION_INTERFACE_INDEX;
        conditions[0].matchType = FWP_MATCH_EQUAL;
        conditions[0].conditionValue.r#type = FWP_UINT32;
        conditions[0].conditionValue.Anonymous.uint32 = if_index;
        self.add_filter(layer, sublayer_key, name, weight, FWP_ACTION_PERMIT, &mut conditions)
    }

    /// Allow для процесса по абсолютному пути к exe. Использует
    /// `FwpmGetAppIdFromFileName0` чтобы получить app-id (security blob),
    /// потом строит фильтр с `FWPM_CONDITION_ALE_APP_ID`.
    pub fn add_filter_allow_app(
        &self,
        layer: GUID,
        sublayer_key: GUID,
        name: &str,
        weight: u8,
        exe_path: &Path,
    ) -> Result<()> {
        let path_w = to_wide(&exe_path.to_string_lossy());

        let mut blob_ptr: *mut FWP_BYTE_BLOB = ptr::null_mut();
        unsafe {
            let rc = FwpmGetAppIdFromFileName0(path_w.as_ptr(), &mut blob_ptr);
            if rc != ERROR_SUCCESS {
                bail!(
                    "FwpmGetAppIdFromFileName0 failed for {}: 0x{:08x}",
                    exe_path.display(),
                    rc
                );
            }
            if blob_ptr.is_null() {
                bail!("FwpmGetAppIdFromFileName0 returned null blob");
            }

            // RAII для blob — освободим в любом случае.
            struct BlobGuard(*mut FWP_BYTE_BLOB);
            impl Drop for BlobGuard {
                fn drop(&mut self) {
                    if !self.0.is_null() {
                        unsafe { FwpmFreeMemory0(&mut (self.0 as *mut std::ffi::c_void)) };
                    }
                }
            }
            let _guard = BlobGuard(blob_ptr);

            let mut conditions: [FWPM_FILTER_CONDITION0; 1] = [std::mem::zeroed()];
            conditions[0].fieldKey = FWPM_CONDITION_ALE_APP_ID;
            conditions[0].matchType = FWP_MATCH_EQUAL;
            conditions[0].conditionValue.r#type = FWP_BYTE_BLOB_TYPE;
            conditions[0].conditionValue.Anonymous.byteBlob = blob_ptr;

            self.add_filter(
                layer,
                sublayer_key,
                name,
                weight,
                FWP_ACTION_PERMIT,
                &mut conditions,
            )
        }
    }

    /// Низкоуровневый add_filter — внутреннее API.
    fn add_filter(
        &self,
        layer: GUID,
        sublayer_key: GUID,
        name: &str,
        weight: u8,
        action: u32,
        conditions: &mut [FWPM_FILTER_CONDITION0],
    ) -> Result<()> {
        let name_w = to_wide(name);
        let mut provider_key_copy = NEMEFISTO_PROVIDER_GUID;
        unsafe {
            let mut filter: FWPM_FILTER0 = std::mem::zeroed();
            filter.layerKey = layer;
            filter.subLayerKey = sublayer_key;
            filter.displayData.name = name_w.as_ptr() as *mut u16;
            filter.providerKey = &mut provider_key_copy;

            // Weight как FWP_UINT8 — простая 8-битная шкала.
            filter.weight.r#type = FWP_UINT8;
            filter.weight.Anonymous.uint8 = weight;

            filter.action.r#type = action;

            filter.numFilterConditions = conditions.len() as u32;
            filter.filterCondition = if conditions.is_empty() {
                ptr::null_mut()
            } else {
                conditions.as_mut_ptr()
            };

            let rc =
                FwpmFilterAdd0(self.handle, &filter, ptr::null_mut(), ptr::null_mut());
            if rc != ERROR_SUCCESS {
                bail!("FwpmFilterAdd0({}) failed: 0x{:08x}", name, rc);
            }
        }
        Ok(())
    }
}

impl Drop for WfpEngine {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { FwpmEngineClose0(self.handle) };
            self.handle = ptr::null_mut();
        }
    }
}

/// Cleanup orphan-объектов с прошлых инкарнаций helper'а.
///
/// Открывает persistent engine, под транзакцией удаляет sublayer и
/// provider — все принадлежащие им фильтры удаляются каскадно. Идемпотентно:
/// если ничего нашего нет, ошибки `FWP_E_*_NOT_FOUND` игнорируются.
///
/// Должно вызываться при старте helper-сервиса до того как принять
/// любые команды. Это страховка — DYNAMIC session уже должна была
/// убрать всё, но если по какому-то редкому сценарию (kernel panic
/// в момент crash и т.п.) фильтры остались, тут мы их добиваем.
pub fn cleanup_provider() -> Result<()> {
    // FWP_E_FILTER_NOT_FOUND = 0x80320005, _PROVIDER_NOT_FOUND = 0x80320007,
    // _SUBLAYER_NOT_FOUND = 0x80320006
    const FWP_E_PROVIDER_NOT_FOUND: u32 = 0x8032_0007;
    const FWP_E_SUBLAYER_NOT_FOUND: u32 = 0x8032_0006;

    let engine = WfpEngine::open_persistent().context("cleanup: open engine")?;
    engine.transaction(|e| {
        unsafe {
            // Порядок: sublayer → provider. Удаление sublayer удаляет
            // все его фильтры автоматически.
            let rc = FwpmSubLayerDeleteByKey0(e.handle, &NEMEFISTO_SUBLAYER_GUID);
            if rc != ERROR_SUCCESS && rc != FWP_E_SUBLAYER_NOT_FOUND {
                return Err(anyhow!("delete sublayer: 0x{:08x}", rc));
            }
            let rc = FwpmProviderDeleteByKey0(e.handle, &NEMEFISTO_PROVIDER_GUID);
            if rc != ERROR_SUCCESS && rc != FWP_E_PROVIDER_NOT_FOUND {
                return Err(anyhow!("delete provider: 0x{:08x}", rc));
            }
        }
        Ok(())
    })?;
    Ok(())
}

