use h_live_runtime::Runtime;
use serde_json::json;
fn main() {
    let result = (|| -> Result<serde_json::Value, String> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let fixture: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("build/replay-fixture.json"))
                .map_err(|_| "读取fixture失败")?,
        )
        .map_err(|_| "fixture无效")?;
        let url = url::Url::parse(
            fixture["replay"]["play_url"]
                .as_str()
                .ok_or("fixture缺少URL")?,
        )
        .map_err(|_| "fixture URL无效")?;
        let directory = tempfile::Builder::new()
            .prefix("replay-media-probe-")
            .tempdir_in(root.join("build"))
            .map_err(|_| "创建临时目录失败")?;
        let runtime = Runtime::open(directory.path().into())?;
        let work = (|| {
            runtime.request("community_configure", json!({"enabled":true}))?;
            let executor = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|_| "创建探针线程失败")?;
            executor.block_on(async{
                let start=std::time::Instant::now();
                let playlist=runtime.replay_media_fetch(url.as_str(),None,None).await?;
                let playlist_ms=start.elapsed().as_millis();
                let text=String::from_utf8(playlist).map_err(|_|"清单编码无效")?;
                let segment=text.lines().map(str::trim).find(|line|!line.is_empty()&&!line.starts_with('#')).ok_or("清单缺少首段")?;
                let segment=url.join(segment).map_err(|_|"首段地址无效")?;
                let start=std::time::Instant::now();
                let bytes=runtime.replay_media_fetch(segment.as_str(),Some(0),Some(65536)).await?;
                let first_segment_ms=start.elapsed().as_millis();
                let len=bytes.len();
                std::fs::write(root.join("build/real-replay-ts-probe.bin"),bytes).map_err(|_|"保存本地首段样本失败")?;
                Ok(json!({"native_no_proxy":true,"playlist_ms":playlist_ms,"segment_ms":first_segment_ms,"sampled_bytes":len,
                    "playlist_host":url.host_str(),"segment_host":segment.host_str()}))
            })
        })();
        let shutdown = runtime.shutdown();
        drop(runtime);
        shutdown?;
        work
    })();
    match result {
        Ok(summary) => println!("{summary}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
