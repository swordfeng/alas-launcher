// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{env::set_current_dir, fs, net::TcpStream, os::unix::process::CommandExt as _, path::PathBuf, process::{Child, Command}, thread::sleep, time::Duration};

#[cfg(unix)]
use nix::sys::signal::{killpg, Signal};
#[cfg(unix)]
use nix::unistd::{Pid, setpgid};
use tauri::{Manager, Url};
#[cfg(windows)]
use winapi::um::consoleapi::GenerateConsoleCtrlEvent;

use wait_timeout::ChildExt;
use anyhow::{anyhow, Result};
use tracing::{info, warn};
use serde_json::Value as JsonValue;

fn alas_repo_dir() -> Result<PathBuf> {
    let mut app_local_dir =
        dirs::data_local_dir().ok_or_else(|| anyhow!("Unknown OS-specific data dir"))?;
    app_local_dir.push("alas-launcher");
    std::fs::create_dir_all(&app_local_dir)?;
    Ok(app_local_dir)
}

fn git_init() -> Result<()> {
    info!("Starting git initialization...");
    // Remove any existing content in the current directory
    for entry in fs::read_dir(".")? {
        let path = entry?.path();
        if path.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }

    let status = Command::new("git")
        .args(["clone", "https://github.com/LmeSzinc/AzurLaneAutoScript.git", "."])
        .status()?;

    if !status.success() {
        return Err(anyhow!("Failed to clone repository"));
    }

    Ok(())
}

fn git_update() -> Result<()> {
    let status = Command::new("python")
        .args(["-c", "import deploy.git; deploy.git.GitManager().git_install()"])
        .status()?;
    if !status.success() {
        return Err(anyhow!("Failed to update repository"));
    }
    Ok(())
}

fn setup_alas_repo() -> Result<()> {
    info!("Starting setup for ALAS repository...");
    let dir = alas_repo_dir()?;
    info!("Working dir is {:?}", &dir);
    set_current_dir(&dir)?;
    if git_update().is_err() {
        warn!("Git update failed, initializing repository...");
        git_init()?;
        git_update()?;
    }
    Ok(())
}

fn get_deploy_config() -> Option<JsonValue> {
    let config_content = fs::read_to_string("./config/deploy.yaml").ok()?;
    let config: JsonValue = serde_yaml::from_str(&config_content).ok()?;
    Some(config)
}

struct ManagedBackend {
    child: Option<Child>,
}

impl ManagedBackend {
    fn new(port: u16) -> Result<Self> {
        let mut command = Command::new("python");
        command.args(["gui.py", "--host", "127.0.0.1", "--port", &port.to_string()]);

        // Create a new process group
        #[cfg(unix)]
        unsafe {
            command.pre_exec(|| {
                setpgid(Pid::from_raw(0), Pid::from_raw(0))?; // Set the process group ID to the same as the PID
                Ok(())
            });
        }
        let mut backend = Self { child: Some(command.spawn()?) };

        let address = format!("127.0.0.1:{}", port).parse().unwrap();
        let start_time = std::time::Instant::now();
        while start_time.elapsed() < Duration::from_secs(5) {
            if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
                // Port is ready
                return Ok(backend);
            }
            // Wait for a short duration before retrying
            sleep(Duration::from_millis(100));
        }

        match backend.kill() {
            Ok(_) => {}
            Err(e) => warn!("Failed to kill gui.py process: {:?}", e),
        }
        Err(anyhow!("Timeout waiting for port {} to be ready", port))
    }


    fn kill(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            #[cfg(unix)]
            {
                // Send SIGINT to the process group
                killpg(Pid::from_raw(child.id() as i32), Signal::SIGINT)?;
            }
            #[cfg(windows)]
            {
                // Send CTRL+C event to the process
                unsafe {
                    if GenerateConsoleCtrlEvent(winapi::um::wincon::CTRL_C_EVENT, child.id()) == 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
            }
            if child.wait_timeout(Duration::from_secs(5))?.is_none() {
                // If the process didn't exit, kill it
                warn!("gui.py didn't exit, killing it...");
                child.kill()?;
                child.wait()?;
            }
        }
        Ok(())
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    setup_alas_repo()?;

    let port = get_deploy_config().as_ref()
        .and_then(|config| config.get("Deploy"))
        .and_then(|deploy| deploy.get("Webui"))
        .and_then(|webui| webui.get("WebuiPort"))
        .and_then(|port| port.as_u64());
    if port.is_none() {
        warn!("WebuiPort not found in config, using default port 22267");
    }
    let port = port.unwrap_or(22267) as u16;
    info!("Starting gui.py on http://127.0.0.1:{}/", port);
    let mut backend = ManagedBackend::new(port)?;

    info!("Starting Webview...");
    tauri::Builder::default()
        .build(tauri::generate_context!())?
        .run(move |app_handle, event| {
            match event {
                tauri::RunEvent::Ready => {
                    info!("Webview is ready");
                    app_handle.get_webview_window("main").unwrap().navigate(Url::parse(&format!("http://127.0.0.1:{}/", port)).unwrap()).unwrap();
                }
                tauri::RunEvent::ExitRequested { .. } => {
                    info!("Webview closed, shutting down backend...");
                    if let Err(e) = backend.kill() {
                        warn!("Failed to kill backend process: {:?}", e);
                    }
                    app_handle.exit(0);
                }
                _  => {}
            };
        });
    Ok(())
}
