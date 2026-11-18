use h_live_runtime::Runtime;
use serde_json::json;

fn main() {
    let result = (|| -> Result<serde_json::Value, String> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap().parent().unwrap();
        let directory = tempfile::Builder::new().prefix("replay-probe-")
            .tempdir_in(root.join("build")).map_err(|_| "创建临时数据目录失败")?;
        let runtime = Runtime::open(directory.path().into())?;
        let work = (|| {
            runtime.request("community_configure", json!({"enabled":true}))?;
            let replay = runtime.request("session_replay", json!({"session_id":"7691695639532702518"}))?;
            if replay["available"] == true {
                std::fs::write(root.join("build/replay-fixture.json"),
                    serde_json::to_vec(&replay).map_err(|_| "编码回放结果失败")?)
                    .map_err(|_| "保存回放fixture失败")?;
            }
            Ok(json!({"enabled":replay["enabled"],"available":replay["available"],
                "source_kind":replay["replay"]["source_kind"],"media_kind":replay["replay"]["media_kind"],
                "duration_ms":replay["replay"]["duration_ms"],
                "mapping_segments":replay["replay"]["time_mapping"].as_array().map(|x|x.len()),
                "width":replay["replay"]["width"],"height":replay["replay"]["height"]}))
        })();
        let shutdown = runtime.shutdown();
        drop(runtime);
        shutdown?;
        work
    })();
    match result {
        Ok(summary) => println!("{summary}"),
        Err(error) => { eprintln!("{error}"); std::process::exit(1); }
    }
}
