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

#[cfg(unix)]
use command_group::Signal;
use command_group::{CommandGroup, GroupChild};
use tauri::{Manager, Url};

use anyhow::{anyhow, Result};
use serde_json::Value as JsonValue;
use tracing::{info, warn};

fn alas_repo_dir() -> Result<PathBuf> {
    // Assuming portable, the executable is in the same directory as the repo
    Ok(std::env::current_exe()?.parent().unwrap().to_path_buf())
}

fn prepend_path_to_env(key: &str, path: PathBuf) {
    let mut paths = Vec::new();
    paths.push(path);
    if let Some(ref old_path) = &std::env::var_os(key) {
        paths.extend(std::env::split_paths(old_path).into_iter());
    }
    std::env::set_var(key, std::env::join_paths(paths).unwrap());
}

#[cfg(unix)]
fn setup_environment() -> Result<()> {
    prepend_path_to_env("PATH", alas_repo_dir()?.join("toolkit").join("libexec").join("git-core"));
    prepend_path_to_env("PATH", alas_repo_dir()?.join("toolkit").join("bin"));
    prepend_path_to_env("LD_LIBRARY_PATH", alas_repo_dir()?.join("toolkit").join("lib"));
    Ok(())
}

#[cfg(windows)]
fn setup_environment() -> Result<()> {
    prepend_path_to_env("PATH", alas_repo_dir()?.join("toolkit").join("git").join("cmd"));
    prepend_path_to_env("PATH", alas_repo_dir()?.join("toolkit").join("Scripts"));
    prepend_path_to_env("PATH", alas_repo_dir()?.join("toolkit"));
    Ok(())
}

fn setup_alas_repo() -> Result<()> {
    info!("Starting setup for ALAS repository...");
    let dir = alas_repo_dir()?;
    info!("ALAS dir is {:?}", &dir);
    set_current_dir(&dir)?;
    git_update()?;
    Ok(())
}

fn get_deploy_config() -> Option<JsonValue> {
    let config_content = fs::read_to_string("./config/deploy.yaml").ok()?;
    let config: JsonValue = serde_yaml::from_str(&config_content).ok()?;
    Some(config)
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

struct ManagedBackend {
    child: Option<GroupChild>,
}

impl ManagedBackend {
    fn new(port: u16) -> Result<Self> {
        let mut command = Command::new("python");
        command.args(["gui.py", "--host", "127.0.0.1", "--port", &port.to_string()]);
        let mut group = command.group();
        #[cfg(all(windows, not(debug_assertions)))]
        {
            use winapi::um::winbase::CREATE_NO_WINDOW;
            group.creation_flags(CREATE_NO_WINDOW);
        }
        let res = Self { child: Some(group.spawn()?) };

        let address = format!("127.0.0.1:{}", port).parse().unwrap();
        let start_time = std::time::Instant::now();
        while start_time.elapsed() < Duration::from_secs(60) {
            if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
                return Ok(res);
            }
            sleep(Duration::from_millis(100));
        }
        Err(anyhow!("Timeout waiting for port {} to be ready", port))
    }

    fn terminate(&mut self) -> Result<ExitStatus> {
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

impl Drop for ManagedBackend {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            match child.kill() {
                Ok(_) => {}
                Err(e) => warn!("Failed to kill gui.py process: {:?}", e),
            }
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    setup_environment()?;
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
                    if let Err(e) = backend.terminate() {
                        warn!("Failed to terminate backend process: {:?}", e);
                    }
                }
                _ => {}
            };
        });
    Ok(())
}
