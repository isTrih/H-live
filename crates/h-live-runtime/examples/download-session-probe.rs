//! Diagnostic: download eight seconds through the real clip worker in an isolated data directory.
//! API credentials and signed media URLs are never printed or written to diagnostic output.
use h_live_runtime::{storage, Runtime};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn run() -> Result<Value, String> {
    let session = std::env::args()
        .nth(1)
        .filter(|value| storage::id(value))
        .ok_or("请提供真实直播场次 ID")?;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let directory = tempfile::Builder::new()
        .prefix("download-origin-probe-")
        .tempdir_in(root.join("build"))
        .map_err(|_| "创建隔离诊断目录失败")?;
    let runtime = Runtime::open(directory.path().into())?;
    let work = (|| {
        runtime.configure_media_tools(root.join("packaging/tools/media"))?;
        runtime.request("community_configure", json!({"enabled":true}))?;
        let output = root.join("build").join(format!(
            "download-origin-{session}-{:08x}.mp4",
            rand::random::<u32>()
        ));
        let created = runtime.request("clip_create", json!({
            "session_id":session,"start_seconds":0.0,"end_seconds":8.0,
            "title":"下载来源诊断","output_path":output
        }))?;
        let id = created["job_id"].as_str().ok_or("创建下载诊断任务失败")?;
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut phase = String::new();
        loop {
            let current = runtime.request("clip_job", json!({"job_id":id}))?;
            let job = &current["job"];
            let current_phase = job["phase"].as_str().unwrap_or("");
            if current_phase != phase {
                println!("{}", json!({"phase":current_phase,"progress":job["progress"]}));
                phase = current_phase.into();
            }
            if job["status"] == "completed" {
                return Ok(json!({"completed":true,"session_id":session,
                    "requested_seconds":8,"downloaded_bytes":job["downloaded_bytes"],
                    "actual_start_seconds":job["actual_start_seconds"],
                    "actual_end_seconds":job["actual_end_seconds"],"output_path":job["output_path"]}));
            }
            if job["status"] == "failed" || job["status"] == "cancelled" {
                return Err(job["error"].as_str().unwrap_or("下载诊断已取消").into());
            }
            if Instant::now() > deadline {
                runtime.request("clip_cancel", json!({"job_id":id}))?;
                return Err("下载诊断超时，已取消临时任务".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    })();
    runtime.shutdown()?;
    drop(runtime);
    work
}

fn main() {
    match run() {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
