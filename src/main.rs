// No default console window createion on Windows
#![windows_subsystem = "windows"]

use std::{
    cell::Cell,
    env::set_current_dir,
    fs,
    net::TcpStream,
    path::PathBuf,
    process::{Command, ExitStatus},
    thread::sleep,
    time::Duration,
};

use anyhow::{anyhow, Result};
use base64::{prelude::BASE64_STANDARD, Engine};
use command_group::{CommandGroup, GroupChild};
use serde_json::Value as JsonValue;
use tauri::{
    webview::{PageLoadEvent, PageLoadPayload},
    Manager, Url, WebviewWindow,
};
use tauri_plugin_dialog::{DialogExt, FilePath};
use tracing::{error, info, warn};

fn alas_repo_dir() -> PathBuf {
    // Always check if this is a typical same-folder portable distribution
    let exe_folder = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let mut installer_py = exe_folder.clone();
    installer_py.extend(["deploy", "installer.py"]);
    if fs::exists(installer_py).unwrap() {
        return exe_folder;
    }
    // If it's MacOS, it could be ALAS.app/Contents/AzurLaneAutoScript
    #[cfg(target_os = "macos")]
    {
        use std::ffi::OsStr;
        if exe_folder.file_name() == Some(&OsStr::new("MacOS")) {
            let mut repo_folder = exe_folder;
            repo_folder.pop();
            repo_folder.push("AzurLaneAutoScript");
            if fs::exists(&repo_folder).unwrap() {
                return repo_folder;
            }
        }
    }
    panic!("Cannot find ALAS repo folder");
}

fn prepend_path_to_env(key: &str, path: PathBuf) {
    let mut paths = Vec::new();
    paths.push(path);
    if let Some(ref old_path) = &std::env::var_os(key) {
        paths.extend(std::env::split_paths(old_path));
    }
    std::env::set_var(key, std::env::join_paths(paths).unwrap());
}

#[cfg(unix)]
fn setup_environment() -> Result<()> {
    prepend_path_to_env(
        "PATH",
        alas_repo_dir()
            .join("toolkit")
            .join("libexec")
            .join("git-core"),
    );
    prepend_path_to_env("PATH", alas_repo_dir().join("toolkit").join("bin"));
    prepend_path_to_env(
        "LD_LIBRARY_PATH",
        alas_repo_dir().join("toolkit").join("lib"),
    );
    Ok(())
}

#[cfg(windows)]
fn setup_environment() -> Result<()> {
    prepend_path_to_env(
        "PATH",
        alas_repo_dir().join("toolkit").join("git").join("cmd"),
    );
    prepend_path_to_env("PATH", alas_repo_dir().join("toolkit").join("Scripts"));
    prepend_path_to_env("PATH", alas_repo_dir().join("toolkit"));
    Ok(())
}

#[cfg(target_os = "linux")]
fn setup_git_ca_bundle() {
    let cert_file = openssl_probe::probe().cert_file;
    if let Some(file) = cert_file.as_ref().and_then(|f| f.to_str()) {
        let _ = Command::new("git")
            .args(["config", "--local", "http.sslCAInfo", file])
            .status();
    }
}

fn setup_alas_repo() -> Result<()> {
    info!("Starting setup for ALAS repository...");
    let dir = alas_repo_dir();
    info!("ALAS dir is {:?}", &dir);
    set_current_dir(&dir)?;
    #[cfg(target_os = "linux")]
    setup_git_ca_bundle();
    // Similar setup to deploy/installer.py
    atomic_failure_cleanup("./config")?;
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
        .create_no_window()
        .status()?;
    if !status.success() {
        return Err(anyhow!("Failed to update repository"));
    }
    Ok(())
}

fn atomic_failure_cleanup(path: &str) -> Result<()> {
    let _ = Command::new("python")
        .args([
            "-c",
            "import sys; from deploy.atomic import atomic_failure_cleanup; atomic_failure_cleanup(sys.argv[1])",
            path,
        ])
        .create_no_window()
        .status()?;
    Ok(())
}

struct ManagedBackend {
    child: Option<GroupChild>,
}

impl ManagedBackend {
    fn new(port: u16) -> Result<Self> {
        let child = Command::new("python")
            .args(["gui.py", "--host", "127.0.0.1", "--port", &port.to_string()])
            .group()
            .create_no_window()
            .spawn()?;
        let res = Self { child: Some(child) };

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
                use command_group::{Signal, UnixChildExt};
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

static ALAS_LANUCHER_INJECTION_JS: &'static str = r#"
if (!window.alas_launcher_injected) {
    window.alas_launcher_injected = true;
    (function () {
        // const origSaveAs = window.saveAs;
        window.saveAs = function (blob, filename) {
            const reader = new FileReader();
            reader.onload = async () => {
                const data = reader.result.split(',')[1];
                console.log(data);
                window.__TAURI__.core.invoke('save_as', { filename, data });
            };
            reader.readAsDataURL(blob);
        };
    })();
}
"#;

#[tauri::command]
fn save_as(app_handle: tauri::AppHandle, filename: &str, data: &str) {
    match BASE64_STANDARD.decode(data) {
        Ok(decoded_data) => app_handle
            .dialog()
            .file()
            .set_file_name(filename)
            .save_file(move |path| {
                let result: Result<()> = (move || {
                    let file_path = path
                        .as_ref()
                        .and_then(FilePath::as_path)
                        .ok_or_else(|| anyhow!("Invalid file path {:?}", &path))?;
                    fs::write(file_path, &decoded_data)?;
                    info!("Saved file to {:?}", file_path);
                    Ok(())
                })();
                if let Err(e) = result {
                    error!("Failed to save file: {:?}", e);
                }
            }),
        Err(e) => {
            error!("Failed to decode file content: {:?}", e);
            return;
        }
    };
}

fn page_load_injector(webview: WebviewWindow, payload: PageLoadPayload<'_>) {
    match payload.event() {
        PageLoadEvent::Finished => {
            info!(
                "Injecting saveFile function to loaded page: {}",
                payload.url()
            );
            if let Err(e) = webview.eval(ALAS_LANUCHER_INJECTION_JS) {
                error!("Failed to inject JS to webview: {:?}", e);
            }
        }
        _ => {}
    }
}

fn main() -> Result<()> {
    #[cfg(windows)]
    unsafe {
        use winapi::um::wincon::{AttachConsole, ATTACH_PARENT_PROCESS};
        HAS_CONSOLE.store(AttachConsole(ATTACH_PARENT_PROCESS) != 0, Ordering::Relaxed);
    }
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

    let mut backend = Cell::new(None);

    info!("Starting Webview...");
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![save_as])
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            let _ = app
                .get_webview_window("main")
                .and_then(|w| w.set_focus().ok());
        }))
        .setup(|app| {
            tauri::WebviewWindowBuilder::from_config(
                app,
                app.config()
                    .app
                    .windows
                    .iter()
                    .find(|w| w.label == "main")
                    .unwrap(),
            )?
            .on_page_load(page_load_injector)
            .build()?;
            Ok(())
        })
        .build(tauri::generate_context!())?
        .run(move |app_handle, event| {
            match event {
                tauri::RunEvent::Ready => {
                    info!("Starting gui.py on http://127.0.0.1:{}/", port);
                    let b = ManagedBackend::new(port).unwrap();
                    backend.set(Some(b));
                    info!("Webview is ready");
                    let window = app_handle.get_webview_window("main").unwrap();
                    window
                        .navigate(Url::parse(&format!("http://127.0.0.1:{}/", port)).unwrap())
                        .unwrap();
                    window.show().unwrap();
                }
                tauri::RunEvent::ExitRequested { .. } => {
                    info!("Webview closed, shutting down backend...");
                    if let Some(b) = backend.get_mut() {
                        if let Err(e) = b.terminate() {
                            warn!("Failed to terminate backend process: {:?}", e);
                        }
                    }
                }
                _ => {}
            };
        });
    Ok(())
}

////// Utility: hide console windows at start (Windows)
#[cfg(windows)]
use command_group::builder::CommandGroupBuilder;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
static HAS_CONSOLE: AtomicBool = AtomicBool::new(false);

trait CreateNoWindow {
    fn create_no_window(&mut self) -> &mut Self;
}

#[cfg(windows)]
impl CreateNoWindow for Command {
    fn create_no_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        use winapi::um::winbase::CREATE_NO_WINDOW;
        if !HAS_CONSOLE.load(Ordering::Relaxed) {
            self.creation_flags(CREATE_NO_WINDOW)
        } else {
            self
        }
    }
}

#[cfg(windows)]
impl<T> CreateNoWindow for CommandGroupBuilder<'_, T> {
    fn create_no_window(&mut self) -> &mut Self {
        use winapi::um::winbase::CREATE_NO_WINDOW;
        if !HAS_CONSOLE.load(Ordering::Relaxed) {
            self.creation_flags(CREATE_NO_WINDOW)
        } else {
            self
        }
    }
}

#[cfg(not(windows))]
impl<T> CreateNoWindow for T {
    fn create_no_window(&mut self) -> &mut Self {
        self
    }
}
