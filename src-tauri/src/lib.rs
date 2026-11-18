//! Qingkan desktop composition. Domain processing lives in qingkan-core.
mod backend;
mod desktop;
mod data_backup;
mod lifecycle;
mod paths;
mod compatibility;
mod replay_transport;
mod replay_diagnostic;
#[cfg(windows)]mod native_replay;
#[cfg(windows)]mod native_layers;
#[cfg(windows)]mod replay_cache;
mod update_policy;
mod url_policy;

use serde_json::json;
use std::{fs, sync::{Arc, atomic::{AtomicBool, Ordering}}};
use tauri::{Emitter, Manager};

pub fn run() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.get(1).map(String::as_str) == Some("--replay-diagnostic") {
        replay_diagnostic::run(&arguments);
        return;
    }
    if arguments.get(1).map(String::as_str) == Some("--self-test") {
        let Some(report_path) = arguments.get(2) else {
            std::process::exit(2)
        };
        let result = h_live_runtime::offline_self_test();
        let passed = result.is_ok();
        let report = match result {
            Ok(details) => {
                json!({"ok": true, "version": env!("CARGO_PKG_VERSION"), "checks": details})
            }
            Err(error) => {
                json!({"ok": false, "version": env!("CARGO_PKG_VERSION"), "error": error})
            }
        };
        let written = fs::write(report_path, serde_json::to_vec_pretty(&report).unwrap()).is_ok();
        std::process::exit(if passed && written { 0 } else { 1 });
    }
    let backend = Arc::new(backend::Backend::new());
    let startup_backend = Arc::clone(&backend);
    let quit_ready = Arc::new(AtomicBool::new(false));
    let setup_ready = quit_ready.clone();
    let autostart = arguments.iter().any(|a| a == "--autostart");
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::Builder::new().arg("--autostart").build())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .manage(backend)
        .setup(move |app| {
            lifecycle::setup(app.handle(), setup_ready.clone())?;
            let mut media_tools=app.path().resource_dir()?.join("media-tools");
            #[cfg(debug_assertions)]
            if !media_tools.join("ffmpeg.exe").is_file(){media_tools=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../packaging/tools/media");}
            startup_backend.configure_media_tools(media_tools)?;
            if autostart {
                if let Some(window) = app.get_window("main") { window.hide()?; }
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(error) = startup_backend.initialize() {
                    lifecycle::show(&handle);
                    let _ = handle.emit("backend-close-failed", format!("启动监听失败：{error}"));
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            backend::backend_request,
            data_backup::data_backup_export,
            data_backup::data_backup_anchors,
            data_backup::data_backup_inspect,
            data_backup::data_backup_restore,
            data_backup::data_backup_discard,
            compatibility::compatibility_report,
            replay_transport::replay_media_fetch,
            native_replay::native_replay_open,
            native_replay::native_replay_status,
            native_replay::native_replay_action,
            native_replay::native_replay_bounds,
            native_replay::native_replay_close,
            desktop::choose_clip_path,
            desktop::choose_download_directory,
            desktop::open_clip_folder,
            desktop::choose_export_path,
            desktop::open_data_folder,
            desktop::open_live_room,
            desktop::window_action,
            desktop::window_state,
            lifecycle::autostart_status,
            lifecycle::configure_autostart,
            backend::update_install_guard,
            backend::prepare_update_install,
            backend::cancel_update_install
        ])
        .on_window_event(|window, event| {
            #[cfg(windows)]
            if window.label()=="main" && matches!(event, tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Focused(_) | tauri::WindowEvent::ScaleFactorChanged { .. }) {
                native_replay::maintain_layers(window.app_handle());
            }
            if matches!(event, tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Focused(_)) {
                let _ = window.emit(
                    "window-state",
                    json!({"maximized": window.is_maximized().unwrap_or(false), "focused": window.is_focused().unwrap_or(true)}),
                );
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("无法启动氢看直播")
        .run(move |app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !quit_ready.load(Ordering::Acquire) {
                    api.prevent_exit();
                    lifecycle::quit(app, quit_ready.clone());
                }
            }
        });
}
