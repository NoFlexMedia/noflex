// SPDX-License-Identifier: AGPL-3.0-only
//
// The server as a Windows service. The installer registers it with
// `noflex-server service install`, and the service manager then starts
// `noflex-server service run`, which reports to it and stops the server when
// told to. Part of the binary rather than the library, since it runs `main`'s
// loop.

use std::{
    ffi::OsString,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use tokio::sync::Notify;
use windows_service::{
    define_windows_service,
    service::{
        ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
        ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
        ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};

/// What the service manager, `sc` and the installer call it.
const NAME: &str = "NoFlex";

/// How long `service uninstall` waits for a running server to stop.
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
static STOP: Notify = Notify::const_new();

/// Runs `noflex-server service install|uninstall|run`.
///
/// # Errors
///
/// Unknown arguments, or the service manager refusing: installing needs an
/// administrator, and `run` works only when the service manager started it.
pub fn command(args: &[String]) -> anyhow::Result<()> {
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["service", "install"] => install(),
        ["service", "uninstall"] => uninstall(),
        ["service", "run"] => service_dispatcher::start(NAME, ffi_service_main)
            .context("connect to the service manager; `service run` is for it alone"),
        _ => bail!("usage: noflex-server [service install|uninstall|run]"),
    }
}

/// Resolves once the service manager asks the server to stop, and never when
/// the server isn't a service.
pub async fn stopped() {
    let notified = STOP.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();
    if !STOP_REQUESTED.load(Ordering::SeqCst) {
        notified.await;
    }
}

/// Registers the service to start with Windows, as LocalSystem, and to be
/// restarted when it fails, as systemd's unit is.
fn install() -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .context("open the service manager")?;
    let info = ServiceInfo {
        name: NAME.into(),
        display_name: "NoFlex media server".into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: std::env::current_exe().context("find this executable")?,
        launch_arguments: vec!["service".into(), "run".into()],
        dependencies: Vec::new(),
        account_name: None,
        account_password: None,
    };
    let service = manager
        .create_service(&info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START)
        .context("create the service")?;
    service
        .set_description("Streams your films, shows and music to browsers and the NoFlex apps.")
        .context("describe the service")?;
    let restart = ServiceAction {
        action_type: ServiceActionType::Restart,
        delay: Duration::from_secs(5),
    };
    // A service that isn't restarted still runs, so this doesn't fail the
    // install. Wine, for one, lacks the second call.
    let restarts = service
        .update_failure_actions(ServiceFailureActions {
            reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 60 * 60)),
            reboot_msg: None,
            command: None,
            actions: Some(vec![restart; 3]),
        })
        // Exiting with an error counts, not only crashing.
        .and_then(|()| service.set_failure_actions_on_non_crash_failures(true));
    if let Err(error) = restarts {
        eprintln!("installed, but won't be restarted after a failure: {error}");
    }
    Ok(())
}

/// Stops the service if it's running, then removes it.
fn uninstall() -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("open the service manager")?;
    let service = manager
        .open_service(
            NAME,
            ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
        )
        .context("open the service")?;
    if service.query_status()?.current_state != ServiceState::Stopped {
        service.stop().context("stop the service")?;
        let started = Instant::now();
        while service.query_status()?.current_state != ServiceState::Stopped {
            if started.elapsed() > STOP_TIMEOUT {
                bail!("the service didn't stop within {STOP_TIMEOUT:?}");
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    service.delete().context("delete the service")?;
    Ok(())
}

define_windows_service!(ffi_service_main, service_main);

fn service_main(_arguments: Vec<OsString>) {
    // Before the server's own logging starts, there's nowhere else to say it.
    if let Err(error) = run_service() {
        eprintln!("service failed: {error:#}");
    }
}

fn run_service() -> anyhow::Result<()> {
    let status = service_control_handler::register(NAME, |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            STOP_REQUESTED.store(true, Ordering::SeqCst);
            STOP.notify_waiters();
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })
    .context("register with the service manager")?;
    let report = |state, controls_accepted, exit_code| {
        status.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted,
            exit_code,
            checkpoint: 0,
            wait_hint: Duration::from_secs(10),
            process_id: None,
        })
    };
    report(
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        ServiceExitCode::Win32(0),
    )?;
    let served = crate::serve();
    let code = if served.is_ok() { 0 } else { 1 };
    report(
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        ServiceExitCode::ServiceSpecific(code),
    )?;
    served
}
