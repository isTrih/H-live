#[cfg(not(feature = "test-support"))]
fn main() {
    panic!("enable test-support")
}
#[cfg(feature = "test-support")]
fn main() {
    use h_live_runtime::{testing::Simulation, Runtime};
    use serde_json::json;
    use std::{
        sync::{atomic::Ordering, Arc},
        time::{Duration, Instant},
    };
    let directory = tempfile::tempdir().unwrap();
    let simulation = Arc::new(Simulation {
        live_rooms: 12,
        ..Default::default()
    });
    let counters = simulation.counters.clone();
    let r = Runtime::open_with_transport(directory.path().into(), simulation).unwrap();
    let ids = (1..=12).map(|n| n.to_string()).collect::<Vec<_>>();
    r.request("add_anchors", json!({"values":ids.join("\n")}))
        .unwrap();
    let start = Instant::now();
    r.request("start", json!({"rids":ids})).unwrap();
    while counters.active.load(Ordering::Acquire) < 12 {
        assert!(start.elapsed() < Duration::from_secs(45));
        std::thread::sleep(Duration::from_millis(20));
    }
    let seconds = start.elapsed().as_secs_f64();
    r.shutdown().unwrap();
    let result = json!({"rooms":12,"exits":1,"startup_seconds":seconds,"anonymous_homepage_calls":counters.visitors.load(Ordering::Acquire),"all_sockets_closed":counters.active.load(Ordering::Acquire)==0,"scope":"synthetic transport; actual route rate limits; no real platform rooms"});
    if let Some(path) = std::env::args().nth(1) {
        h_live_runtime::storage::atomic(
            std::path::Path::new(&path),
            &serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap()
    }
    println!("{result}");
}
