//! Desktop-only dialogs, window controls and validated external links.
use crate::{paths::data_directory, url_policy};
use serde_json::{json, Value};
use std::{fs, path::PathBuf, process::Command};
use tauri::Manager;

#[tauri::command]
pub(crate) async fn choose_download_directory(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let window=app.get_window("main").ok_or("窗口不可用")?;
    tauri::async_runtime::spawn_blocking(move || {
        Ok(rfd::FileDialog::new().set_parent(&window).set_title("选择切片下载目录")
            .pick_folder().map(|path|path.to_string_lossy().into_owned()))
    }).await.map_err(|_|"无法打开目录选择对话框".to_owned())?
}

#[tauri::command]
pub(crate) async fn choose_clip_path(
    app: tauri::AppHandle,
    default_name: String,
) -> Result<Option<String>, String> {
    let safe_name = PathBuf::from(default_name)
        .file_name().and_then(|name| name.to_str()).unwrap_or("直播切片.mp4").to_owned();
    let window = app.get_window("main").ok_or("窗口不可用")?;
    tauri::async_runtime::spawn_blocking(move || {
        let chosen = rfd::FileDialog::new().set_parent(&window)
            .set_title("保存直播切片").add_filter("MP4 视频", &["mp4"])
            .set_file_name(safe_name).save_file();
        if let Some(path) = chosen {
            if !path.extension().and_then(|v| v.to_str()).is_some_and(|v| v.eq_ignore_ascii_case("mp4")) {
                return Err("请选择以 .mp4 结尾的文件名".into());
            }
            Ok(Some(path.to_string_lossy().into_owned()))
        } else { Ok(None) }
    }).await.map_err(|_| "无法打开保存对话框".to_owned())?
}

#[tauri::command]
pub(crate) async fn open_clip_folder(
    state: tauri::State<'_, std::sync::Arc<crate::backend::Backend>>,
    path: String,
    job_id: Option<String>,
) -> Result<(), String> {
    let backend = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = backend.read_operation()?;
        let runtime = backend.get()?;
        let tasks = if let Some(id)=job_id {
            let record=runtime.request("clip_job",json!({"job_id":id}))?;
            json!({"jobs":[record["job"].clone()]})
        }else{runtime.request("clip_jobs", json!({"limit":100}))?};
        let requested = PathBuf::from(path).canonicalize().map_err(|_| "切片文件不存在")?;
        if !requested.is_file() || !tasks["jobs"].as_array().is_some_and(|jobs| jobs.iter().any(|job| {
            job["status"] == "completed" && job["output_path"].as_str()
                .and_then(|p| PathBuf::from(p).canonicalize().ok()).is_some_and(|p| p == requested)
        })) { return Err("只能打开已完成切片所在的目录".into()); }
        #[cfg(windows)]
        Command::new("explorer.exe").arg("/select,").arg(&requested).spawn()
            .map_err(|_| "无法打开切片目录")?;
        Ok(())
    }).await.map_err(|_| "无法打开切片目录".to_owned())?
}

#[tauri::command]
pub(crate) async fn choose_export_path(
    app: tauri::AppHandle,
    format: String,
    default_name: String,
) -> Result<Option<String>, String> {
    let extension = match format.as_str() {
        "xlsx" => "xlsx",
        "csv" => "csv",
        _ => return Err("仅支持 XLSX 和 CSV".into()),
    };
    let safe_name = PathBuf::from(default_name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("直播消息")
        .to_owned();
    let window = app.get_window("main").ok_or("窗口不可用")?;
    tauri::async_runtime::spawn_blocking(move || {
        let chosen = rfd::FileDialog::new()
            .set_parent(&window)
            .set_title("导出月度消息")
            .add_filter(extension.to_uppercase(), &[extension])
            .set_file_name(safe_name)
            .save_file();
        if let Some(path) = chosen {
            // The Windows dialog adds the default suffix and confirms overwrite.
            // Do not change the path afterwards and accidentally overwrite a
            // different file which the user never confirmed in that dialog.
            if !path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case(extension))
            {
                return Err(format!("请选择以 .{extension} 结尾的文件名"));
            }
            Ok(Some(path.to_string_lossy().to_string()))
        } else {
            Ok(None)
        }
    })
    .await
    .map_err(|_| "无法打开保存对话框".to_string())?
}

#[tauri::command]
pub(crate) fn open_data_folder() -> Result<(), String> {
    let directory = data_directory()?;
    fs::create_dir_all(&directory).map_err(|_| "无法创建数据目录")?;
    #[cfg(windows)]
    Command::new("explorer.exe")
        .arg(directory)
        .spawn()
        .map_err(|_| "无法打开数据目录")?;
    Ok(())
}

#[tauri::command]
pub(crate) fn open_live_room(rid: String) -> Result<(), String> {
    let url = url_policy::live_room_url(&rid)?;
    #[cfg(windows)]
    {
        let operation: Vec<u16> = "open\0".encode_utf16().collect();
        let target: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
        let result = unsafe {
            windows_sys::Win32::UI::Shell::ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        };
        if result as isize <= 32 {
            return Err("无法打开系统默认浏览器，请检查浏览器设置".into());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        Err("此版本仅支持 Windows".into())
    }
}

#[tauri::command]
pub(crate) fn window_state(app: tauri::AppHandle) -> Result<Value, String> {
    let window = app.get_window("main").ok_or("窗口不可用")?;
    Ok(
        json!({"maximized": window.is_maximized().map_err(|_| "无法读取窗口状态")?, "focused": window.is_focused().unwrap_or(true)}),
    )
}

#[tauri::command]
pub(crate) fn window_action(app: tauri::AppHandle, action: String) -> Result<Value, String> {
    let window = app.get_window("main").ok_or("窗口不可用")?;
    match action.as_str() {
        "minimize" => window.minimize(),
        "toggle_maximize" => {
            if window.is_maximized().map_err(|_| "无法读取窗口状态")? {
                window.unmaximize()
            } else {
                window.maximize()
            }
        }
        "close" => window.close(), // CloseRequested hides to the notification tray.
        _ => return Err("无效的窗口操作".into()),
    }
    .map_err(|_| "无法更新窗口状态")?;
    Ok(json!({"maximized": window.is_maximized().unwrap_or(false)}))
}
