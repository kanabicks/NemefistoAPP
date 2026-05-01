//! 14.C — crash dumps через `std::panic::set_hook`.
//!
//! При панике (которая в проде шла бы в `/dev/null` без логов tracing) мы
//! пишем структурированный дамп в
//! `%LOCALAPPDATA%\NemefistoVPN\crashes\<unix_ts>-<component>.txt`:
//!
//!  - сообщение паники + локация (file:line:col);
//!  - полный backtrace (`std::backtrace::Backtrace::force_capture`);
//!  - метаданные (версия, OS, arch).
//!
//! Файлы локальные, никуда не отправляются. Пользователь может приложить
//! их в bug-report через `export_diagnostics` (zip-архив).
//!
//! `install_panic_hook` вызывается ОДИН раз — в начале `lib::run()` для
//! main-процесса и в начале `nemefisto_helper::main()` для helper'а.

use std::backtrace::Backtrace;
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::panic;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Каталог `%LOCALAPPDATA%\NemefistoVPN\crashes\`. Возвращает `None`
/// если переменной `LOCALAPPDATA` нет (запуск из неинтерактивной
/// сессии без user-profile, что у нас в принципе не должно случаться,
/// но лучше деградировать gracefully чем падать в самом hook'е).
pub fn crashes_dir() -> Option<PathBuf> {
    let appdata = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(appdata).join("NemefistoVPN").join("crashes"))
}

/// Установить глобальный panic-hook. Сохраняет предыдущий хук
/// (обычно дефолтный, выводящий в stderr) и вызывает его перед
/// записью файла — чтобы локально через `npm run tauri dev` или
/// helper-eventlog паника всё ещё была видна в консоли.
///
/// `component` — короткое имя для имени файла: `vpn-client`,
/// `nemefisto-helper`. Помогает отличить main-крах от helper-краха.
pub fn install_panic_hook(component: &'static str) {
    let prev = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        // Сначала зовём дефолтный hook — он печатает в stderr,
        // не теряем поведение для разработки.
        prev(info);
        // Потом — наш файловый дамп. Ошибки записи только логируем
        // в stderr, не паникуем повторно (иначе recursive panic).
        if let Err(e) = write_crash_dump(component, info) {
            eprintln!("[crash-dump] не удалось записать: {e}");
        }
    }));
}

fn write_crash_dump(
    component: &str,
    info: &panic::PanicHookInfo<'_>,
) -> std::io::Result<()> {
    let dir = crashes_dir()
        .ok_or_else(|| std::io::Error::other("LOCALAPPDATA не установлен"))?;
    create_dir_all(&dir)?;

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("{ts}-{component}.txt"));

    let mut f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)?;

    writeln!(f, "Nemefisto crash dump")?;
    writeln!(f, "----------------------")?;
    writeln!(f, "component: {component}")?;
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

    // force_capture() работает даже без RUST_BACKTRACE=1 — не зависит
    // от env, что важно для production crash-репортов.
    let bt = Backtrace::force_capture();
    writeln!(f, "backtrace:")?;
    writeln!(f, "{bt}")?;

    Ok(())
}

/// Сколько свежих crash-файлов лежит в каталоге. Используется при
/// старте приложения для UI-сигнала «у вас были крахи». Считаем
/// файлы за последние 7 дней — старые не интересны.
pub fn count_recent_crashes() -> usize {
    let Some(dir) = crashes_dir() else {
        return 0;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let week_ago = SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(7 * 86400))
        .unwrap_or(UNIX_EPOCH);
    entries
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "txt")
                .unwrap_or(false)
        })
        .filter(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .map(|t| t >= week_ago)
                .unwrap_or(false)
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Хук должен быть устанавливаемым многократно — `take_hook` просто
    /// возвращает предыдущий, а наш `set_hook` его оборачивает. Без
    /// этого свойства тесты, использующие `install_panic_hook`, не
    /// смогут сосуществовать в одном бинарнике.
    #[test]
    fn install_is_idempotent() {
        install_panic_hook("test");
        install_panic_hook("test");
        // Если панике-stack бы переполнился — тест бы упал в панике
        // (или в SO). Дошли — значит ok.
    }

    #[test]
    fn crashes_dir_returns_some() {
        // На Windows LOCALAPPDATA всегда есть в test-runner'е,
        // на CI без переменной — пропускаем.
        if std::env::var_os("LOCALAPPDATA").is_some() {
            assert!(crashes_dir().is_some());
        }
    }

    #[test]
    fn count_recent_returns_usize() {
        // Не паника даже если каталога нет.
        let _ = count_recent_crashes();
    }
}
