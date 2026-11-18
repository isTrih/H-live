//! Isolated metadata probe. Never opens the installed app's data directory.
use h_live_runtime::Runtime;
use serde_json::json;
fn main() {
    let result=(||->Result<serde_json::Value,String>{
        let id=std::env::args().nth(1).ok_or("缺少场次ID")?;
        if !id.bytes().all(|b|b.is_ascii_digit()){return Err("场次ID无效".into());}
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
        let directory=tempfile::Builder::new().prefix("replay-timing-").tempdir_in(root.join("build")).map_err(|_|"创建独立数据目录失败")?;
        let runtime=Runtime::open(directory.path().into())?;
        let work=(||{
            let key=std::env::var("INTERNAL_API_TOKEN").map_err(|_|"需要通过环境提供社区凭据")?;
            runtime.request("community_configure",json!({"enabled":true,"api_key":key}))?;
            let response=runtime.request("session_replay",json!({"session_id":id}))?;
            let replay=&response["replay"];
            let target=root.join("build").join(format!("replay-{id}.json"));
            std::fs::write(target,serde_json::to_vec(&response).map_err(|_|"编码回放失败")?).map_err(|_|"保存本地回放探针失败")?;
            let spans=replay["time_mapping"].as_array();
            Ok(json!({"session_id":id,"available":response["available"],"duration_ms":replay["duration_ms"],"playlist_duration_ms":replay["playlist_duration_ms"],"media_kind":replay["media_kind"],"mapping_segments":spans.map(|s|s.len()),"first_mapping":spans.and_then(|s|s.first()),"last_mapping":spans.and_then(|s|s.last())}))
        })();
        let shutdown=runtime.shutdown();drop(runtime);shutdown?;work
    })();
    match result{Ok(value)=>println!("{value}"),Err(error)=>{eprintln!("{error}");std::process::exit(1);}}
}
