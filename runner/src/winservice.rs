//! Windows service entry point (`csr windows-service`), started by the service manager.

#[cfg(windows)]
mod imp {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::process::ExitCode;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    };
    use windows_service::service_control_handler::{
        self, ServiceControlHandlerResult, ServiceStatusHandle,
    };
    use windows_service::{define_windows_service, service_dispatcher};

    const NAME: &str = "CSR"; // ignored by the SCM for an own-process service, but required by the API

    struct Ctx {
        cfg: Result<crate::config::Config, String>,
        config_path: PathBuf,
        data_root: PathBuf,
    }

    static CTX: Mutex<Option<Ctx>> = Mutex::new(None);
    define_windows_service!(ffi_service_main, service_main);

    pub fn run(
        cfg: Result<crate::config::Config, String>,
        config_path: PathBuf,
        data_root: PathBuf,
    ) -> ExitCode {
        *CTX.lock().unwrap() = Some(Ctx {
            cfg,
            config_path,
            data_root,
        });
        match service_dispatcher::start(NAME, ffi_service_main) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("csr windows-service is started by the Windows service manager (services.msc), not from a console: {e}");
                ExitCode::from(1)
            }
        }
    }

    fn status(
        state: ServiceState,
        accept: ServiceControlAccept,
        code: ServiceExitCode,
        hint: Duration,
    ) -> ServiceStatus {
        ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: code,
            checkpoint: 0,
            wait_hint: hint,
            process_id: None,
        }
    }

    fn service_main(_args: Vec<OsString>) {
        let Some(ctx) = CTX.lock().unwrap().take() else {
            return;
        };
        let cell: Arc<Mutex<Option<ServiceStatusHandle>>> = Arc::new(Mutex::new(None));
        let in_handler = cell.clone();
        let handler = move |control: ServiceControl| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                if let Some(h) = *in_handler.lock().unwrap() {
                    let _ = h.set_service_status(status(
                        ServiceState::StopPending,
                        ServiceControlAccept::empty(),
                        ServiceExitCode::Win32(0),
                        Duration::from_secs(30),
                    ));
                }
                crate::run::request_shutdown();
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        };
        let Ok(handle) = service_control_handler::register(NAME, handler) else {
            return;
        };
        *cell.lock().unwrap() = Some(handle);
        let _ = handle.set_service_status(status(
            ServiceState::Running,
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            ServiceExitCode::Win32(0),
            Duration::default(),
        ));
        let error_file = ctx.data_root.join("service-error.log");
        let result = match ctx.cfg {
            Ok(cfg) => crate::runtime().block_on(crate::run::run(
                cfg,
                ctx.config_path,
                ctx.data_root,
                false,
            )),
            Err(e) => Err(e),
        };
        let code = match result {
            Ok(()) => ServiceExitCode::Win32(0),
            Err(e) => {
                let _ = std::fs::write(&error_file, format!("{e}\n"));
                ServiceExitCode::ServiceSpecific(1)
            }
        };
        let _ = handle.set_service_status(status(
            ServiceState::Stopped,
            ServiceControlAccept::empty(),
            code,
            Duration::default(),
        ));
    }
}

#[cfg(windows)]
pub use imp::run;

#[cfg(not(windows))]
pub fn run(
    _cfg: Result<crate::config::Config, String>,
    _config_path: std::path::PathBuf,
    _data_root: std::path::PathBuf,
) -> std::process::ExitCode {
    eprintln!("csr windows-service is only available on Windows");
    std::process::ExitCode::from(2)
}
