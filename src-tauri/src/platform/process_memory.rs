//! Подсчёт памяти VPN-движков (этап 13.X — memory monitor на главном экране).
//!
//! Перебираем процессы через `EnumProcesses` (как в `processes.rs` для
//! detect_competing_vpns), для каждого с подходящим именем (sing-box / mihomo)
//! читаем Working Set через `K32GetProcessMemoryInfo`. Не требует admin —
//! `PROCESS_QUERY_LIMITED_INFORMATION` достаточно для memory-counters.
//!
//! Возвращает агрегированный размер в байтах. UI делит на 1024² и
//! показывает в МБ. Polling — раз в секунду, синхронно с bandwidth.

#[cfg(windows)]
pub fn engine_memory_bytes() -> Option<u64> {
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE};
    use windows_sys::Win32::System::ProcessStatus::{
        EnumProcesses, K32GetModuleBaseNameW, K32GetProcessMemoryInfo,
        PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
    };

    // Имена exe нашего sidecar pipeline'а. Tauri-shell спавнит без
    // переименования, поэтому имя содержит target-triple (например
    // `sing-box-x86_64-pc-windows-msvc.exe`). Helper-spawn (TUN-режим)
    // запускает тот же exe с тем же именем. Сравнение по `starts_with`
    // переживает обе формы.
    const ENGINE_PREFIXES: &[&str] = &["sing-box", "mihomo"];

    unsafe {
        let mut pids = vec![0u32; 4096];
        let mut bytes_returned: u32 = 0;
        let cb = (pids.len() * std::mem::size_of::<u32>()) as u32;
        if EnumProcesses(pids.as_mut_ptr(), cb, &mut bytes_returned) == 0 {
            return None;
        }
        let count = (bytes_returned as usize) / std::mem::size_of::<u32>();
        pids.truncate(count);

        let mut total: u64 = 0;
        let mut found = false;
        for &pid in &pids {
            if pid == 0 {
                continue;
            }
            // PROCESS_VM_READ нужен для GetProcessMemoryInfo; QUERY_LIMITED
            // достаточно для GetModuleBaseName. Объединяем флаги.
            let h: HANDLE = OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
                FALSE,
                pid,
            );
            if h.is_null() {
                continue;
            }

            let mut name_buf = [0u16; 260];
            let len = K32GetModuleBaseNameW(
                h,
                std::ptr::null_mut(),
                name_buf.as_mut_ptr(),
                name_buf.len() as u32,
            );
            if len == 0 {
                CloseHandle(h);
                continue;
            }
            let name = String::from_utf16_lossy(&name_buf[..len as usize]).to_lowercase();
            let stem = name.strip_suffix(".exe").unwrap_or(&name);

            let is_engine = ENGINE_PREFIXES.iter().any(|p| stem.starts_with(p));
            if !is_engine {
                CloseHandle(h);
                continue;
            }

            let mut counters: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            let cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            if K32GetProcessMemoryInfo(h, &mut counters, cb) != 0 {
                // WorkingSetSize — текущий resident memory в байтах
                // (то что реально лежит в RAM). Это самая близкая
                // метрика к «сколько памяти жрёт процесс» в Task Manager.
                total = total.saturating_add(counters.WorkingSetSize as u64);
                found = true;
            }
            CloseHandle(h);
        }

        if found {
            Some(total)
        } else {
            None
        }
    }
}

#[cfg(not(windows))]
pub fn engine_memory_bytes() -> Option<u64> {
    None
}
