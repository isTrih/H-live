//! Desktop backup dialogs and opaque, validated import previews.
use crate::{backend::Backend, paths};
use h_live_runtime::data_backup::DataBackup;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{Manager, State};

#[tauri::command]
pub(crate) async fn data_backup_export(
    app: tauri::AppHandle,
    state: State<'_, Arc<Backend>>,
    preferences: Value,
    anchor_ids: Option<Vec<String>>,
) -> Result<Option<Value>, String> {
    if anchor_ids.as_ref().is_some_and(|ids| ids.is_empty() || ids.len() > 10000 || ids.iter().any(|id| id.is_empty() || id.len() > 80)) {
        return Err("请选择有效的主播备份范围".into());
    }
    let backend = state.inner().clone();
    let window = app.get_window("main").ok_or("窗口不可用")?;
    tauri::async_runtime::spawn_blocking(move || {
        let scope = if anchor_ids.is_some() { "部分主播-" } else { "" };
        let name = format!("氢看直播-{scope}{}.qingkan-backup.zip", chrono_name());
        let Some(path) = rfd::FileDialog::new().set_parent(&window)
            .set_title("导出数据备份").add_filter("氢看直播备份", &["zip"])
            .set_file_name(name).save_file() else { return Ok(None); };
        if !path.extension().and_then(|v|v.to_str()).is_some_and(|v|v.eq_ignore_ascii_case("zip")) {
            return Err("请选择以 .zip 结尾的备份文件名".into());
        }
        let _operation = backend.read_operation()?;
        let _pending = backend.backups.lock().map_err(|_| "备份状态不可用")?;
        let runtime = backend.get()?;
        runtime.flush_for_backup()?;
        let result = DataBackup::export_selected(&paths::data_directory()?, &path, preferences, anchor_ids)?;
        Ok(Some(json!({"path":result["path"],"bytes":result["bytes"],"summary":result})))
    }).await.map_err(|_| "导出备份任务异常".to_owned())?
}

#[tauri::command]
pub(crate) async fn data_backup_anchors(
    state: State<'_, Arc<Backend>>,
) -> Result<Value, String> {
    let backend = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = backend.read_operation()?;
        let runtime = backend.get()?;
        runtime.flush_for_backup()?;
        DataBackup::anchors(&paths::data_directory()?)
    }).await.map_err(|_| "读取备份主播失败".to_owned())?
}

#[tauri::command]
pub(crate) async fn data_backup_inspect(
    app: tauri::AppHandle,
    state: State<'_, Arc<Backend>>,
) -> Result<Option<Value>, String> {
    let backend = state.inner().clone();
    let window = app.get_window("main").ok_or("窗口不可用")?;
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = rfd::FileDialog::new().set_parent(&window)
            .set_title("选择氢看直播备份").add_filter("氢看直播备份", &["zip"])
            .pick_file() else { return Ok(None); };
        let _operation = backend.read_operation()?;
        let mut pending = backend.backups.lock().map_err(|_| "备份状态不可用")?;
        // A failed replacement preview must not leave the previous token armed.
        pending.take();
        let directory = paths::data_directory()?;
        let prepared = DataBackup::prepare(&path, &directory)?;
        let result = json!({"token":prepared.token(), "summary":prepared.summary(),
            "file_name":path.file_name().and_then(|v|v.to_str()).unwrap_or("备份文件")});
        *pending = Some(prepared);
        Ok(Some(result))
    }).await.map_err(|_| "读取备份任务异常".to_owned())?
}

#[tauri::command]
pub(crate) async fn data_backup_discard(
    state: State<'_, Arc<Backend>>, token: String,
) -> Result<(), String> {
    let backend = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = backend.read_operation()?;
        let mut pending = backend.backups.lock().map_err(|_| "备份状态不可用")?;
        if pending.as_ref().is_some_and(|backup|backup.token() == token) { pending.take(); }
        Ok(())
    }).await.map_err(|_| "取消备份预览失败".to_owned())?
}

#[tauri::command]
pub(crate) async fn data_backup_restore(
    state: State<'_, Arc<Backend>>, token: String,
) -> Result<Value, String> {
    let backend = state.inner().clone();
    if token.len() != 32 || !token.bytes().all(|v|v.is_ascii_hexdigit()) {
        return Err("备份预览已失效，请重新选择备份文件".into());
    }
    tauri::async_runtime::spawn_blocking(move || backend.restore_backup(&token))
        .await.map_err(|_| "恢复备份任务异常，请重新打开应用".to_owned())?
}

fn chrono_name() -> String {
    // ISO timestamp is produced by the runtime in the app's default time zone.
    h_live_runtime::storage::now().chars().take(19)
        .filter(|c|c.is_ascii_digit() || *c=='-').collect()
}
