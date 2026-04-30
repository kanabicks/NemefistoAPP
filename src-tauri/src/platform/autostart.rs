//! Управление автозапуском приложения (этап 6.B).
//!
//! Используем Windows Task Scheduler через `schtasks.exe` — task создаётся
//! с триггером ON-LOGON для текущего пользователя и стартует Nemefisto при
//! входе в систему. Преимущество перед `HKCU\...\Run`: не требует UAC при
//! установке (текущий user-scope), переживает обновление приложения, и
//! пользователь видит/может удалить task через стандартный UI Windows.
//!
//! Альтернатива через registry-Run-key:
//! - Плюс: тривиально (одна запись в реестре);
//! - Минус: запускается с тем же UAC-уровнем что приложение в момент
//!   создания. У нас приложение НЕ требует admin (только helper-сервис),
//!   так что разницы практически нет. Task Scheduler выбран ради
//!   будущей совместимости с RunHighest, если понадобится.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};

const TASK_NAME: &str = "Nemefisto VPN Autostart";

/// Полный путь к exe текущего приложения для регистрации в task.
fn current_exe() -> Result<PathBuf> {
    std::env::current_exe().context("не удалось определить путь к exe")
}

/// Включить автозапуск: создать (или пересоздать) task в планировщике.
/// Запуск — `ONLOGON` для текущего пользователя.
pub fn enable() -> Result<()> {
    let exe = current_exe()?;
    let exe_str = exe
        .to_str()
        .context("путь к exe содержит не-UTF-8 символы")?;

    // /F = принудительная перезапись если task уже существует
    // /SC ONLOGON = триггер при входе пользователя
    // /RL LIMITED = обычные пользовательские права (без UAC)
    let status = Command::new("schtasks.exe")
        .args([
            "/Create",
            "/F",
            "/TN",
            TASK_NAME,
            "/TR",
            &format!("\"{exe_str}\""),
            "/SC",
            "ONLOGON",
            "/RL",
            "LIMITED",
        ])
        .status()
        .context("не удалось запустить schtasks.exe")?;

    if !status.success() {
        anyhow::bail!("schtasks /Create вернул код {:?}", status.code());
    }
    Ok(())
}

/// Выключить автозапуск: удалить task. Если его нет — тихо успех.
pub fn disable() -> Result<()> {
    let status = Command::new("schtasks.exe")
        .args(["/Delete", "/F", "/TN", TASK_NAME])
        .status()
        .context("не удалось запустить schtasks.exe")?;

    // schtasks /Delete /F возвращает 0 если task удалён, и не-0 если его
    // не было. Второе для нас не ошибка — состояние и так «выключено».
    let _ = status;
    Ok(())
}

/// Проверить, зарегистрирован ли task в планировщике.
pub fn is_enabled() -> bool {
    let output = match Command::new("schtasks.exe")
        .args(["/Query", "/TN", TASK_NAME])
        .output()
    {
        Ok(o) => o,
        Err(_) => return false,
    };
    output.status.success()
}
