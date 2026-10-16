//! Inspect only user-supplied room IDs; no playback, chat or login.
use h_live_runtime::{net, proxy::Route, storage};
use serde_json::json;
fn main() -> Result<(), String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() < 2 {
        return Err("survey REPORT_PATH ROOM...".into());
    }
    let path = std::path::PathBuf::from(&args[0]);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "runtime failed")?;
    let rows=rt.block_on(async{let route=Route::new(None)?;let cancel=tokio_util::sync::CancellationToken::new();let guest=net::visitor(&route,&cancel).await.map_err(|e|e.to_string())?;let mut rows=vec![];for raw in &args[1..]{let rid=storage::room_id(raw)?;match net::room(&route,&guest,&rid,&cancel).await{Ok(room)=>rows.push(json!({"rid":rid,"live":room.is_live,"real_session_known":!room.room_id.is_empty()})),Err(error)=>{rows.push(json!({"rid":rid,"error":error.to_string()}));if matches!(error,net::Error::Status(429|444)){break}}}}Ok::<_,String>(rows)})?;
    let report = json!({"rooms":rows,"guest_only":true});
    storage::atomic(&path, &serde_json::to_vec_pretty(&report).unwrap())?;
    println!("{report}");
    Ok(())
}
