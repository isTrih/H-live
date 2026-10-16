//! Offline benchmark through the same production collector/database interface.
#[cfg(not(feature = "test-support"))]
fn main() {
    panic!("enable test-support");
}
#[cfg(feature = "test-support")]
fn main() -> Result<(), String> {
    use h_live_runtime::{
        process, storage,
        testing::{add_fake_exits, Simulation},
        Runtime,
    };
    use serde_json::json;
    use std::{
        sync::{atomic::Ordering, Arc},
        time::{Duration, Instant},
    };
    let output = std::env::args().nth(1).ok_or("benchmark REPORT_PATH")?;
    let directory = tempfile::tempdir().map_err(|_| "temp dir failed")?;
    let simulated = Arc::new(Simulation::default());
    let counters = simulated.counters.clone();
    let before = process::counters();
    let began = Instant::now();
    let runtime = Runtime::open_with_transport(directory.path().into(), simulated)?;
    add_fake_exits(&runtime, 50)?;
    let rids = (1..=300).map(|n| n.to_string()).collect::<Vec<_>>();
    runtime.request("add_anchors", json!({"values":rids.join("\n")}))?;
    runtime.request("start", json!({"rids":rids}))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut max_threads = 0;
    let mut peak_rss = 0f64;
    while counters.active.load(Ordering::Acquire) < 50 {
        let p = runtime.request("poll", json!({}))?;
        max_threads = max_threads.max(p["metrics"]["thread_count"].as_u64().unwrap_or(0));
        peak_rss = peak_rss.max(process::counters()["rss_mb"].as_f64().unwrap_or(0.));
        if Instant::now() > deadline {
            return Err("did not reach 50 simulated connections".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let startup_seconds = began.elapsed().as_secs_f64();
    std::thread::sleep(Duration::from_secs(8));
    let old_connections = counters.connections.load(Ordering::Acquire);
    counters
        .force_disconnect_through
        .store(old_connections, Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(20);
    while counters.connections.load(Ordering::Acquire) < old_connections + 50 {
        let p = runtime.request("poll", json!({}))?;
        max_threads = max_threads.max(p["metrics"]["thread_count"].as_u64().unwrap_or(0));
        peak_rss = peak_rss.max(process::counters()["rss_mb"].as_f64().unwrap_or(0.));
        if Instant::now() > deadline {
            return Err("reconnect wave did not recover".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_secs(2));
    let metrics = runtime.request("poll", json!({}))?["metrics"].clone();
    let stop = Instant::now();
    runtime.request("stop", json!({"rids":rids}))?;
    let stop_seconds = stop.elapsed().as_secs_f64();
    runtime.shutdown()?;
    let db = rusqlite::Connection::open_with_flags(
        directory.path().join("messages.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|_| "read failed")?;
    let chats: i64 = db
        .query_row(
            "SELECT count(*) FROM messages WHERE message_type='chat'",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "read failed")?;
    let gifts: i64 = db
        .query_row(
            "SELECT count(*) FROM messages WHERE message_type='gift'",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "read failed")?;
    let after = process::counters();
    let wall = began.elapsed().as_secs_f64();
    let cpu = after["cpu_seconds"]
        .as_f64()
        .zip(before["cpu_seconds"].as_f64())
        .map(|(a, b)| (a - b) / wall * 100.);
    let report = json!({"ok":counters.peak.load(Ordering::Acquire)==50&&counters.active.load(Ordering::Acquire)==0&&chats>0&&gifts==0,"scope":"offline simulated transport through production monitor, decoder, archive, quality and route admission; not 50 real Douyin rooms","configured_tasks":300,"simulated_live_rooms":50,"exits_including_direct":51,"peak_connections":counters.peak.load(Ordering::Acquire),"connections_created":counters.connections.load(Ordering::Acquire),"all_connections_closed":counters.active.load(Ordering::Acquire)==0,"startup_to_50_seconds":startup_seconds,"stop_seconds":stop_seconds,"wall_seconds":wall,"average_process_cpu_percent_of_one_core":cpu,"peak_observed_rss_mb":peak_rss,"max_collector_threads":max_threads,"chat_rows":chats,"gift_rows":gifts,"generated_frames":counters.frames.load(Ordering::Acquire),"archive":metrics["archive"],"all_workers_stopped":true});
    storage::atomic(
        std::path::Path::new(&output),
        &serde_json::to_vec_pretty(&report).unwrap(),
    )?;
    println!("{report}");
    if report["ok"] != true {
        return Err("benchmark invariant failed".into());
    }
    Ok(())
}
