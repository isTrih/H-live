use h_live_runtime::Runtime;
use serde_json::json;
fn main() {
    let result = (|| -> Result<serde_json::Value, String> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let directory = tempfile::Builder::new()
            .prefix("clip-probe-")
            .tempdir_in(root.join("build"))
            .map_err(|_| "创建临时目录失败")?;
        let runtime = Runtime::open(directory.path().into())?;
        let work = (|| {
            if let Some(cache) = std::env::args().nth(1) {
                let cache = std::path::PathBuf::from(cache);
                if !cache.is_absolute() { return Err("组件验收缓存必须使用绝对路径".into()); }
                runtime.configure_media_components(h_live_runtime::media_components::MediaComponents::new(cache, |_| {}))?;
            } else {
                runtime.configure_media_tools(root.join("packaging/tools/media"))?;
            }
            runtime.request("community_configure", json!({"enabled":true}))?;
            let output = root
                .join("build")
                .join(format!("real-clip-{:08x}.mp4", rand::random::<u32>()));
            let created=runtime.request("clip_create",json!({"session_id":"7691695639532702518","start_seconds":0.,"end_seconds":8.,"title":"真实场次短切片验收","output_path":output}))?;
            let id = created["job_id"].as_str().ok_or("创建任务失败")?;
            runtime.request("clip_cancel", json!({"job_id":id}))?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
            let mut retried = false;
            let mut phase = String::new();
            loop {
                let list = runtime.request("clip_jobs", json!({}))?;
                let job = list["jobs"]
                    .as_array()
                    .and_then(|rows| rows.iter().find(|job| job["id"] == id))
                    .ok_or("任务丢失")?;
                let current = job["phase"].as_str().unwrap_or("");
                if current != phase {
                    println!(
                        "{}",
                        json!({"phase":current,"progress":job["progress"],"attempt":job["attempt"]})
                    );
                    phase = current.into();
                }
                if job["status"] == "cancelled" && !retried {
                    runtime.request("clip_retry", json!({"job_id":id}))?;
                    retried = true;
                    continue;
                }
                if job["status"] == "completed" {
                    return Ok(
                        json!({"completed":true,"attempt":job["attempt"],"output_path":job["output_path"],"requested_start_seconds":job["requested_start_seconds"],"requested_end_seconds":job["requested_end_seconds"],"actual_start_seconds":job["actual_start_seconds"],"actual_end_seconds":job["actual_end_seconds"],"downloaded_bytes":job["downloaded_bytes"]}),
                    );
                }
                if job["status"] == "failed" {
                    return Err(job["error"].as_str().unwrap_or("切片失败").into());
                }
                if std::time::Instant::now() > deadline {
                    runtime.request("clip_cancel", json!({"job_id":id}))?;
                    return Err("切片探针超时".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        })();
        let shutdown = runtime.shutdown();
        drop(runtime);
        shutdown?;
        work
    })();
    match result {
        Ok(result) => println!("{result}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
