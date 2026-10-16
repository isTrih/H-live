//! Bounded live diagnostic. Saves aggregate counts only, never message content.
use h_live_runtime::{storage, Runtime};
use serde_json::json;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
fn main() -> Result<(), String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 3 {
        return Err("usage: probe ROOM SECONDS REPORT_PATH".into());
    }
    let rid = storage::room_id(&args[0])?;
    let seconds = args[1]
        .parse::<u64>()
        .ok()
        .filter(|n| (5..=120).contains(n))
        .ok_or("seconds must be 5..120")?;
    let output = PathBuf::from(&args[2]);
    let directory = tempfile::tempdir().map_err(|_| "cannot create isolated data directory")?;
    let runtime = Runtime::open(directory.path().into())?;
    runtime.request("add_anchors", json!({"values":rid}))?;
    runtime.request("start", json!({"rids":[rid]}))?;
    let began = Instant::now();
    let mut connected = false;
    let mut states = std::collections::BTreeSet::new();
    let mut messages = 0;
    while began.elapsed().as_secs() < seconds {
        let p = runtime.request("poll", json!({}))?;
        if let Some(a) = p["anchors"].as_array().and_then(|a| a.first()) {
            let state = a["state"].as_str().unwrap_or("unknown");
            if state == "connected" && !connected {
                println!("WSS connected; monitoring anonymous messages.");
                connected = true;
            }
            states.insert(state.to_owned());
            messages = a["message_count"].as_u64().unwrap_or(0);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    runtime.request("stop", json!({"rids":[rid]}))?;
    runtime.shutdown()?;
    let db = rusqlite::Connection::open_with_flags(
        directory.path().join("messages.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|_| "cannot inspect isolated archive")?;
    let mut statement = db
        .prepare("SELECT message_type,COUNT(*) FROM messages GROUP BY message_type")
        .map_err(|_| "aggregate query failed")?;
    let counts = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|_| "aggregate query failed")?
        .collect::<std::result::Result<std::collections::BTreeMap<_, _>, _>>()
        .map_err(|_| "aggregate query failed")?;
    let report = json!({"version":h_live_runtime::VERSION,"connected":connected,"seconds":began.elapsed().as_secs_f64(),"states":states,"recorded_messages":messages,"message_type_counts":counts,"no_login_cookie":true,"no_gift_recording":true,"all_workers_stopped":true,"payload_content_saved_to_report":false});
    storage::atomic(&output, &serde_json::to_vec_pretty(&report).unwrap())?;
    println!("{}", report);
    Ok(())
}
