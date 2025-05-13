// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    env::set_current_dir,
    fs,
    net::TcpStream,
    path::PathBuf,
    process::{Command, ExitStatus},
    thread::sleep,
    time::Duration,
};

use command_group::{CommandGroup, GroupChild};
#[cfg(unix)]
use command_group::Signal;
use tauri::{Manager, Url};

use anyhow::{anyhow, Result};
use serde_json::Value as JsonValue;
use tracing::{info, warn};

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
        .args([
            "clone",
            "https://github.com/LmeSzinc/AzurLaneAutoScript.git",
            ".",
        ])
        .status()?;

    if !status.success() {
        return Err(anyhow!("Failed to clone repository"));
    }

    Ok(())
}

fn git_update() -> Result<()> {
    let status = Command::new("python")
        .args([
            "-c",
            "import deploy.git; deploy.git.GitManager().git_install()",
        ])
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
    child: Option<GroupChild>,
}

impl ManagedBackend {
    fn new(port: u16) -> Result<Self> {
        let mut command = Command::new("python");
        command.args(["gui.py", "--host", "127.0.0.1", "--port", &port.to_string()]);
        let mut group = command.group();
        #[cfg(windows)]
        {
            use winapi::um::winbase::CREATE_NO_WINDOW;
            group.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = group.spawn()?;

        let address = format!("127.0.0.1:{}", port).parse().unwrap();
        let start_time = std::time::Instant::now();
        while start_time.elapsed() < Duration::from_secs(60) {
            if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
                return Ok(Self { child: Some(child) });
            }
            sleep(Duration::from_millis(100));
        }

        match child.kill() {
            Ok(_) => {}
            Err(e) => warn!("Failed to kill gui.py process: {:?}", e),
        }
        Err(anyhow!("Timeout waiting for port {} to be ready", port))
    }

    fn kill(&mut self) -> Result<ExitStatus> {
        if let Some(mut child) = self.child.take() {
            #[cfg(unix)]
            {
                use command_group::UnixChildExt;
                let _ = child.signal(Signal::SIGTERM);
                let start_time = std::time::Instant::now();
                while start_time.elapsed() < Duration::from_millis(500) {
                    if let Ok(Some(exit_status)) = child.try_wait() {
                        return Ok(exit_status);
                    }
                    sleep(Duration::from_millis(100));
                }
                warn!("gui.py didn't exit, killing it...");
            }
            child.kill()?;
            Ok(child.wait()?)
        } else {
            Ok(ExitStatus::default())
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    setup_alas_repo()?;

    let port = get_deploy_config()
        .as_ref()
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
                    app_handle
                        .get_webview_window("main")
                        .unwrap()
                        .navigate(Url::parse(&format!("http://127.0.0.1:{}/", port)).unwrap())
                        .unwrap();
                }
                tauri::RunEvent::ExitRequested { .. } => {
                    info!("Webview closed, shutting down backend...");
                    if let Err(e) = backend.kill() {
                        warn!("Failed to kill backend process: {:?}", e);
                    }
                    app_handle.exit(0);
                    std::process::exit(0);
                }
                _ => {}
            };
        });
    Ok(())
}
