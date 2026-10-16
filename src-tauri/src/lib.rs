//! Qingkan desktop composition. Domain processing lives in qingkan-core.
mod backend;
mod desktop;
mod paths;
mod update_policy;
mod url_policy;

use serde_json::json;
use std::{fs, sync::Arc};
use tauri::{Emitter, Manager};

pub fn run() {
    let arguments: Vec<String> = std::env::args().collect();
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
    let close_backend = Arc::clone(&backend);
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .manage(backend)
        .invoke_handler(tauri::generate_handler![
            backend::backend_request,
            desktop::choose_export_path,
            desktop::open_data_folder,
            desktop::open_live_room,
            desktop::window_action,
            desktop::window_state,
            backend::update_install_guard,
            backend::prepare_update_install,
            backend::cancel_update_install
        ])
        .on_window_event(move |window, event| {
            if matches!(event, tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Focused(_)) {
                let _ = window.emit(
                    "window-state",
                    json!({"maximized": window.is_maximized().unwrap_or(false), "focused": window.is_focused().unwrap_or(true)}),
                );
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if !close_backend.begin_close() {
                    return;
                }
                let _ = window.hide();
                let app = window.app_handle().clone();
                let backend = Arc::clone(&close_backend);
                tauri::async_runtime::spawn_blocking(move || {
                    if backend.shutdown().is_ok() { app.exit(0); } else if let Some(window)=app.get_webview_window("main") { let _=window.show(); let _=window.emit("backend-close-failed", "后台尚未确认退出，请等待归档完成后重试"); }
                });
            }
        })
        .run(tauri::generate_context!())
        .expect("无法启动氢看直播");
}
