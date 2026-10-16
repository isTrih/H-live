use crate::{storage::now, Runtime};
use qingkan_core::archive::MessageRecord;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub fn run() -> Result<Value, String> {
    let directory = tempfile::tempdir().map_err(|_| "无法创建独立自检目录")?;
    let runtime = Runtime::open(directory.path().into())?;
    let initial = runtime.request("init", json!({}))?;
    if initial["anchors"] != json!([]) || initial["version"] != crate::VERSION {
        return Err("首次安装或版本自检失败".into());
    }
    if runtime
        .request("set_cookie", json!({"cookie":"synthetic-rejected"}))
        .is_ok()
    {
        return Err("已移除的登录接口仍可调用".into());
    }
    let record = MessageRecord {
        anchor_id: "111".into(),
        anchor_name: "合成主播".into(),
        web_rid: "222".into(),
        session_id: "333".into(),
        sender_id: "444".into(),
        sender_name: "合成观众".into(),
        message_type: "chat".into(),
        content: "=合成导出验证 😀".into(),
        message_time: "2026-10-01T12:00:00+08:00".into(),
        received_at: now(),
        time_source: "server".into(),
        message_id: "1".into(),
        payload: json!({}),
    };
    runtime
        .shared
        .archive
        .record_message(record, String::new(), None)
        .map_err(|_| "原生写入自检失败")?;
    for format in ["csv", "xlsx"] {
        let path = directory.path().join(format!("self-test.{format}"));
        let job = runtime.request(
            "export",
            json!({"anchor_ids":["111"],"month":"2026-10","format":format,"output_path":path}),
        )?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let p = runtime.request("poll", json!({}))?;
            let status = p["export_jobs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["id"] == job["id"])
                .ok_or("导出任务丢失")?;
            if status["status"] == "completed" {
                break;
            }
            if status["status"] != "running" || Instant::now() > deadline {
                return Err("原生导出自检失败".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !path.is_file() {
            return Err("导出文件未生成".into());
        }
    }
    let recent = runtime.request("recent_messages", json!({"type":"chat"}))?;
    if recent["messages"].as_array().map(|a| a.len()) != Some(1) {
        return Err("消息查询自检失败".into());
    }
    let stopped = runtime.shutdown()?;
    if stopped["closed"] != true {
        return Err("原生后台退出自检失败".into());
    }
    Ok(
        json!({"runtime_version":crate::VERSION,"native_version":crate::VERSION,"processing":"rust","in_process":true,"no_python_node_v8_sidecar":true,"initial_empty":true,"login_api_removed":true,"archive_write_read":true,"csv_xlsx_exports":true,"all_workers_stopped":true,"closed":true}),
    )
}
