// No default console window createion on Windows
#![windows_subsystem = "windows"]

mod backend;
mod setup;
mod window_util;

use std::{
    fs,
    sync::{Arc, Mutex},
    thread::{self},
};

use anyhow::{anyhow, Result};
use base64::{prelude::BASE64_STANDARD, Engine};
use tauri::{
    webview::{PageLoadEvent, PageLoadPayload},
    Manager, Url, WebviewWindow,
};
use tauri_plugin_dialog::{DialogExt, FilePath};
use tracing::{error, info, warn};

use crate::{
    backend::ManagedBackend,
    setup::{get_deploy_config, setup_alas_repo, setup_environment},
};

fn main() -> Result<()> {
    #[cfg(windows)]
    unsafe {
        use crate::window_util::HAS_CONSOLE;
        use std::sync::atomic::Ordering;
        use winapi::um::wincon::{AttachConsole, ATTACH_PARENT_PROCESS};
        HAS_CONSOLE.store(AttachConsole(ATTACH_PARENT_PROCESS) != 0, Ordering::Relaxed);
    }
    tracing_subscriber::fmt::init();
    setup_environment()?;

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

    let backend = Arc::new(Mutex::new(None));

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
                    let app_handle = app_handle.clone();
                    let backend = backend.clone();
                    thread::spawn(move || {
                        let splash = app_handle.get_webview_window("splash").unwrap();
                        let status_updater = |text: &str| {
                            let content = format!("Loading ALAS, please wait..\n\n{}", text);
                            let url = Url::parse(&text_to_splash(&content)).unwrap();
                            splash.navigate(url).unwrap();
                        };
                        status_updater("Initialize ALAS");
                        if let Err(e) = setup_alas_repo(&status_updater) {
                            error!("{e}");
                            let content = format!("Failed loading ALAS, reason: {}\n\nPlease run alas-launcher from terminal for detailed logs", e);
                            let url = Url::parse(&text_to_splash(&content)).unwrap();
                            splash.navigate(url).unwrap();
                            return;
                        }
                        info!("Starting gui.py on http://127.0.0.1:{}/", port);
                        status_updater("Starting GUI");
                        let b = ManagedBackend::new(port).unwrap();
                        *backend.lock().unwrap() = Some(b);
                        splash.destroy().unwrap();
                        info!("Webview is ready");
                        let window = app_handle.get_webview_window("main").unwrap();
                        window
                            .navigate(Url::parse(&format!("http://127.0.0.1:{}/", port)).unwrap())
                            .unwrap();
                        window.show().unwrap();
                    });
                }
                tauri::RunEvent::ExitRequested { .. } => {
                    info!("Webview closed, shutting down backend...");
                    if let Some(ref mut b) = *backend.lock().unwrap() {
                        if let Err(e) = b.terminate() {
                            warn!("Failed to terminate backend process: {:?}", e);
                        }
                    }
                }
                tauri::RunEvent::WindowEvent { label, event: tauri::WindowEvent::CloseRequested { .. }, .. } => {
                    info!("Window {} closed", label);
                    app_handle.exit(0);
                }
                _ => {}
            };
        });
    Ok(())
}

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
        }
    }
}

fn page_load_injector(webview: WebviewWindow, payload: PageLoadPayload<'_>) {
    if payload.event() == PageLoadEvent::Finished {
        info!(
            "Injecting saveFile function to loaded page: {}",
            payload.url()
        );
        let injected_js = r#"
if (!window.alas_launcher_injected) {
    window.alas_launcher_injected = true;
    (function () {
        // Prevent going back
        history.pushState(null, document.title, location.href);
        window.addEventListener('popstate', event => {
            history.pushState(null, document.title, location.href);
        });
        // Overwrite original saveAs function
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
        if let Err(e) = webview.eval(injected_js) {
            error!("Failed to inject JS to webview: {:?}", e);
        }
    }
}

fn text_to_splash(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => {} // drop CR, keep LF
            other => out.push(other),
        }
    }
    let html = format!(
        r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<style>
  /* fill viewport and hide any scrollbars */
  html,body{{height:100%;margin:0;padding:0;overflow:hidden;background:#fff;color:#111;font-family:system-ui,-apple-system,Segoe UI,Roboto,"Helvetica Neue",Arial;}}
  /* make PRE fill the whole page, add inner padding, clip overflow (no scrollbars) */
  pre{{position:fixed;inset:0;margin:0;padding:20px;box-sizing:border-box;background:#f6f8fa;overflow:hidden;white-space:pre-wrap;word-break:break-word;font-family:Menlo,monospace;font-size:13px;line-height:1.45;}}
  /* remove default focus outlines or user agent scrollbars if present */
  ::-webkit-scrollbar{{display:none;}}
</style>
</head>
<body><pre>{}</pre></body>
</html>"#,
        out
    );

    let b64 = BASE64_STANDARD.encode(html.as_bytes());
    format!("data:text/html;charset=utf-8;base64,{}", b64)
}
