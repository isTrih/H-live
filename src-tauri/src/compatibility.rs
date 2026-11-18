//! Explicit local diagnostics. No network requests, credentials, or user paths.
use crate::backend::Backend;
use serde_json::{json, Value};
use std::{
    io::Read,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use tauri::{Manager, State};

#[tauri::command]
pub(crate) async fn compatibility_report(
    app: tauri::AppHandle,
    state: State<'_, Arc<Backend>>,
) -> Result<Value, String> {
    let backend = state.inner().clone();
    let resources = app
        .path()
        .resource_dir()
        .ok()
        .map(|root| root.join("media-tools"));
    tauri::async_runtime::spawn_blocking(move||{
        let tools=backend.read_operation().and_then(|_operation|backend.get().and_then(|runtime|runtime.request("clip_tools",json!({})))).ok();
        let mut report=json!({"format":1,"checked_at":h_live_runtime::storage::now(),"app_version":env!("CARGO_PKG_VERSION"),
            "platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,
            "webview_version":tauri::webview_version().ok(),
            "system":system_information(),
            "media_tools":{"bundled_files":resources.as_deref().map(tool_presence),
                "clip_available":tools.as_ref().and_then(|v|v["available"].as_bool()).unwrap_or(false),
                "clip_mode":tools.as_ref().and_then(|v|v["mode"].as_str()).unwrap_or("unavailable")},
            "native_player":{"hardware_decoding":null,"video_output":null,"observed":false}});
        if let Some(directory)=std::env::var_os("QINGKAN_MEDIA_TOOLS_DIR"){
            report["media_tools"]["configured_files"]=tool_presence(Path::new(&directory));
        }
        #[cfg(windows)]
        {
            let current=crate::native_replay::snapshot();
            let player=if current["active"]==true{&current}else{&current["last_observed"]};
            report["native_player"]=json!({"active":current["active"],"hardware_decoding":player["hwdec"],
                "hardware_decoding_active":player["hwdec_active"],"video_codec":player["video_codec"],
                "width":player["width"],"height":player["height"],"version":player["mpv_version"],
                "observed":player["frame_ready"]==true,"observed_at":player["observed_at"],"video_output":"d3d11",
                "layer_order_valid":player["layer_order_valid"],"overlay_native_windows":player["overlay_native_windows"],
                "ffmpeg_version":player["ffmpeg_version"],"cache":player["cache"],"performance":player["performance"],
                "observation_note":"解码就绪和图层顺序不等同于画面可见。"});
        }
        Ok(report)
    }).await.map_err(|_|"本机兼容性检查未完成".to_owned())?
}

fn tool_presence(directory: &Path) -> Value {
    json!({
        "ffmpeg":directory.join(format!("ffmpeg{}",std::env::consts::EXE_SUFFIX)).is_file(),
        "ffprobe":directory.join(format!("ffprobe{}",std::env::consts::EXE_SUFFIX)).is_file(),
        "libmpv":directory.join("libmpv-2.dll").is_file(),
    })
}
fn label(value: &Value) -> Value {
    let Some(text) = value.as_str() else {
        return Value::Null;
    };
    // Driver labels should be names/versions, never paths or arbitrary links.
    if text.contains(":\\") || text.contains(":/") || text.starts_with("\\\\") {
        return Value::Null;
    }
    json!(text
        .chars()
        .filter(|c| !c.is_control())
        .take(256)
        .collect::<String>())
}
fn whitelist_system(value: &Value) -> Value {
    let os = &value["os"];
    let graphics=value["graphics"].as_array().map(|cards|cards.iter().take(8).map(|card|json!({
        "name":label(&card["Name"]),"driver_version":label(&card["DriverVersion"]),"video_processor":label(&card["VideoProcessor"]),
    })).collect::<Vec<_>>()).unwrap_or_default();
    json!({"state":"checked","os":{"caption":label(&os["Caption"]),"version":label(&os["Version"]),"build":label(&os["BuildNumber"])},"graphics":graphics})
}
#[cfg(windows)]
fn system_information() -> Value {
    use std::os::windows::process::CommandExt;
    let powershell = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "C:\\Windows".into())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    const QUERY: &str = r#"$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue';[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false);$os=Get-CimInstance -ClassName Win32_OperatingSystem | Select-Object -First 1 -Property Caption,Version,BuildNumber;$graphics=@(Get-CimInstance -ClassName Win32_VideoController | Select-Object -Property Name,DriverVersion,VideoProcessor);@{os=$os;graphics=$graphics}|ConvertTo-Json -Depth 4 -Compress"#;
    let child = std::process::Command::new(powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            QUERY,
        ])
        .creation_flags(0x08000000)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return json!({"state":"unavailable"});
    };
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return json!({"state":"error"});
    };
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        let mut overflow = false;
        loop {
            let count = stdout.read(&mut chunk).ok()?;
            if count == 0 {
                break;
            }
            if bytes.len() + count <= 256 * 1024 {
                bytes.extend_from_slice(&chunk[..count]);
            } else {
                overflow = true;
            }
        }
        (!overflow).then_some(bytes)
    });
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if start.elapsed() >= Duration::from_secs(8) {
            timed_out = true;
            let _ = child.kill();
            break child.wait();
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(_) => {
                let _ = child.kill();
                break child.wait();
            }
        }
    };
    let bytes = reader.join().ok().flatten();
    if timed_out {
        return json!({"state":"timeout"});
    }
    if !status.is_ok_and(|s| s.success()) {
        return json!({"state":"error"});
    }
    bytes
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .map(|value| whitelist_system(&value))
        .unwrap_or_else(|| json!({"state":"error"}))
}
#[cfg(not(windows))]
fn system_information() -> Value {
    json!({"state":"unsupported"})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_report_whitelists_fields_and_excludes_paths_and_machine_identity() {
        let value = whitelist_system(
            &json!({"os":{"Caption":"Windows 11","Version":"10.0","BuildNumber":"26100","CSName":"private-machine"},"graphics":[{"Name":"GPU","DriverVersion":"32.0","VideoProcessor":"C:\\Users\\private\\secret"}],"INTERNAL_API_TOKEN":"private-key"}),
        );
        let text = value.to_string();
        assert!(!text.contains("private"));
        assert_eq!(value["graphics"][0]["name"], "GPU");
        assert!(value["graphics"][0]["video_processor"].is_null());
    }
}
