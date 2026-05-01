//! 14.C — crash-dump hook для helper-сервиса.
//!
//! Дублирует `platform::crash_dumps` из main-приложения (чтобы helper
//! как отдельный `bin` не тащил библиотеку клиента целиком). Файлы
//! пишутся в тот же каталог `%LOCALAPPDATA%\NemefistoVPN\crashes\`,
//! но с суффиксом `nemefisto-helper` чтобы не путать с main-крашами.
//!
//! ВАЖНО: при запуске под SCM (Local System) `LOCALAPPDATA` указывает
//! на `C:\Windows\System32\config\systemprofile\AppData\Local\` — туда
//! crash-dump'ы и попадут. Это нормально: файлы доступны админу,
//! `export_diagnostics` со стороны main-app достанет их через
//! `is_helper_crash`-эвристику или общую папку.
//!
//! TODO когда-нибудь: helper-крах писать в общий `%PROGRAMDATA%\NemefistoVPN\`
//! доступный обоим SYSTEM и user-mode.

use std::backtrace::Backtrace;
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::panic;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn crashes_dir() -> Option<PathBuf> {
    // Порядок: LOCALAPPDATA (user-session helper) → PROGRAMDATA
    // (SYSTEM-session). Так main-app точно найдёт хотя бы один.
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return Some(PathBuf::from(local).join("NemefistoVPN").join("crashes"));
    }
    if let Some(program) = std::env::var_os("PROGRAMDATA") {
        return Some(PathBuf::from(program).join("NemefistoVPN").join("crashes"));
    }
    None
}

pub fn install_panic_hook() {
    let prev = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        prev(info);
        if let Err(e) = write_crash_dump(info) {
            eprintln!("[crash-dump] не удалось записать: {e}");
        }
    }));
}

fn write_crash_dump(info: &panic::PanicHookInfo<'_>) -> std::io::Result<()> {
    let dir = crashes_dir()
        .ok_or_else(|| std::io::Error::other("LOCALAPPDATA/PROGRAMDATA не установлен"))?;
    create_dir_all(&dir)?;

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("{ts}-nemefisto-helper.txt"));

    let mut f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)?;

    writeln!(f, "Nemefisto helper crash dump")?;
    writeln!(f, "----------------------")?;
    writeln!(f, "component: nemefisto-helper")?;
    writeln!(f, "version:   {}", env!("CARGO_PKG_VERSION"))?;
    writeln!(f, "timestamp: {ts}")?;
    writeln!(f, "os:        {}", std::env::consts::OS)?;
    writeln!(f, "arch:      {}", std::env::consts::ARCH)?;
    if let Some(loc) = info.location() {
        writeln!(f, "location:  {loc}")?;
    }
    writeln!(f)?;
    writeln!(f, "panic info:")?;
    writeln!(f, "{info}")?;
    writeln!(f)?;
    let bt = Backtrace::force_capture();
    writeln!(f, "backtrace:")?;
    writeln!(f, "{bt}")?;

    Ok(())
}
