//! Append-only лог helper-сервиса в `C:\ProgramData\NemefistoVPN\helper.log`.
//!
//! Helper работает как Windows service — его stdout/stderr куда-то теряются
//! (SCM не сохраняет их по умолчанию). Чтобы пользователь и разработчик
//! могли видеть что происходит внутри (особенно kill-switch decisions),
//! ключевые сообщения дублируем сюда.
//!
//! Best-effort: ошибки записи игнорируются (не валим helper если файл не
//! доступен). Файл переоткрывается на каждое сообщение — медленно, но
//! kill-switch enable случается раз в connect-сессию, не критично.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Дописать строку в helper.log с timestamp'ом + продублировать в stderr
/// (на случай если helper запущен не как сервис, а через `debug`-режим).
///
/// Не возвращает Result — если запись не удалась, helper продолжает
/// работать.
pub fn log(msg: &str) {
    eprintln!("{msg}");

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dir = PathBuf::from(r"C:\ProgramData\NemefistoVPN");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("helper.log");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{ts}] {msg}");
    }
}
