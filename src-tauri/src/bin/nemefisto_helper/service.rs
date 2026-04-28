//! Регистрация Windows-сервиса через Service Control Manager.
//!
//! `install` — добавляет сервис, ставит автозапуск, сразу стартует.
//! `uninstall` — останавливает и удаляет.
//! `service_main` — точка входа, которую SCM вызывает при старте сервиса.

use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use windows_service::{
    define_windows_service,
    service::{
        ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
        ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};

use super::pipe;
use super::protocol::{SERVICE_DESCRIPTION, SERVICE_DISPLAY_NAME, SERVICE_NAME};

const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

// ─── install / uninstall ──────────────────────────────────────────────────────

pub fn install() -> Result<()> {
    let manager_access = ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)
        .context("не удалось открыть Service Control Manager (нужны admin-права)")?;

    let exe_path = std::env::current_exe()
        .context("не удалось получить путь к собственному exe")?;

    let service_info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_DISPLAY_NAME),
        service_type: SERVICE_TYPE,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe_path,
        // SCM вызывает `nemefisto-helper.exe service` — флаг для main-а.
        launch_arguments: vec![OsString::from("service")],
        dependencies: vec![],
        account_name: None, // SYSTEM
        account_password: None,
    };

    let service = service_manager
        .create_service(
            &service_info,
            ServiceAccess::CHANGE_CONFIG | ServiceAccess::START,
        )
        .context("не удалось создать сервис")?;

    service
        .set_description(SERVICE_DESCRIPTION)
        .context("не удалось установить описание сервиса")?;

    service.start(&[] as &[&str]).context("не удалось запустить сервис")?;

    println!("сервис «{SERVICE_NAME}» установлен и запущен");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let manager_access = ServiceManagerAccess::CONNECT;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)
        .context("не удалось открыть Service Control Manager (нужны admin-права)")?;

    let service_access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE;
    let service = service_manager
        .open_service(SERVICE_NAME, service_access)
        .context("сервис не найден")?;

    // Пытаемся остановить, если запущен — игнорируем ошибки stopped→stopped
    let status = service.query_status();
    if let Ok(status) = status {
        if status.current_state != ServiceState::Stopped {
            let _ = service.stop();
            // Ждём остановку до 10 секунд
            for _ in 0..40 {
                std::thread::sleep(Duration::from_millis(250));
                if let Ok(s) = service.query_status() {
                    if s.current_state == ServiceState::Stopped {
                        break;
                    }
                }
            }
        }
    }

    service.delete().context("не удалось удалить сервис")?;
    println!("сервис «{SERVICE_NAME}» удалён");
    Ok(())
}

// ─── service entry-point ──────────────────────────────────────────────────────

define_windows_service!(ffi_service_main, my_service_main);

/// Запустить процесс как сервис (вызывается из main-а если args[1] == "service").
/// SCM вызовет ffi_service_main → my_service_main.
pub fn run_as_service() -> Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .map_err(|e| anyhow!("service_dispatcher::start: {e}"))
}

/// Тело сервиса. Вызывается SCM. Запускает tokio runtime + named pipe сервер.
fn my_service_main(_arguments: Vec<OsString>) {
    if let Err(e) = service_loop() {
        eprintln!("[helper-service] фатальная ошибка: {e:#}");
    }
}

fn service_loop() -> Result<()> {
    // Флаг shutdown, который выставит SCM при ServiceControl::Stop
    let shutdown = Arc::new(AtomicBool::new(false));

    let shutdown_for_handler = shutdown.clone();
    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                shutdown_for_handler.store(true, Ordering::SeqCst);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

    // Сообщаем SCM что мы стартанули
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    // Запускаем tokio runtime + pipe сервер
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("не удалось создать tokio runtime")?;

    let pipe_result = rt.block_on(async move {
        pipe::run_pipe_server(shutdown.clone()).await
    });

    // Сообщаем SCM что мы остановились (любой исход)
    let exit_code = if pipe_result.is_ok() { 0 } else { 1 };
    let _ = status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(exit_code),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    });

    pipe_result
}
