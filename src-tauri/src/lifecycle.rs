//! Native launch registration and notification-area lifecycle.
use crate::backend::Backend;
use serde_json::{json, Value};
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use tauri::{menu::{Menu, MenuItem}, tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent}, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;

pub(crate) fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub(crate) fn quit(app: &tauri::AppHandle, ready: Arc<AtomicBool>) {
    let backend = app.state::<Arc<Backend>>().inner().clone();
    if !backend.begin_close() { return; }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if backend.shutdown().is_ok() {
            ready.store(true, Ordering::Release);
            app.exit(0);
        } else {
            show(&app);
            let _ = app.emit("backend-close-failed", "归档尚未完成，请稍后从托盘重试退出");
        }
    });
}

pub(crate) fn setup(app: &tauri::AppHandle, ready: Arc<AtomicBool>) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "打开氢看直播", true, None::<&str>)?;
    let exit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &exit])?;
    let icon = app.default_window_icon().ok_or_else(|| tauri::Error::AssetNotFound("应用托盘图标".into()))?;
    TrayIconBuilder::with_id("main-tray")
        .icon(icon.clone()).tooltip("氢看直播 · 后台监听")
        .menu(&menu).show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "open" => show(app),
            "quit" => quit(app, ready.clone()),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(event, TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }) {
                show(tray.app_handle());
            }
        }).build(app)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn autostart_status(app: tauri::AppHandle) -> Result<Value, String> {
    app.autolaunch().is_enabled().map(|enabled| json!({"enabled":enabled})).map_err(|_| "无法读取开机启动设置".into())
}

#[tauri::command]
pub(crate) fn configure_autostart(app: tauri::AppHandle, enabled: bool) -> Result<Value, String> {
    let manager = app.autolaunch();
    if enabled { manager.enable() } else { manager.disable() }.map_err(|_| "无法保存开机启动设置，请检查系统权限")?;
    autostart_status(app)
}
