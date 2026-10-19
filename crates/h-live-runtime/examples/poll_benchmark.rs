//! Deterministic idle dashboard polling cost; no platform connections.
#[cfg(not(feature = "test-support"))]
fn main() {
    panic!("enable test-support");
}
#[cfg(feature = "test-support")]
fn main() {
    use h_live_runtime::{
        testing::{add_fake_exits, Simulation},
        Runtime,
    };
    use serde_json::json;
    use std::{sync::Arc, time::Instant};
    let dir = tempfile::tempdir().unwrap();
    let r =
        Runtime::open_with_transport(dir.path().into(), Arc::new(Simulation::default())).unwrap();
    add_fake_exits(&r, 1000).unwrap();
    r.request(
        "add_anchors",
        json!({"values":(1..=300).map(|n|n.to_string()).collect::<Vec<_>>().join("\n")}),
    )
    .unwrap();
    let mut report = Vec::new();
    for include_proxy in [true, false] {
        let began = Instant::now();
        let mut bytes = 0usize;
        for _ in 0..100 {
            let p = r
                .request("poll", json!({"include_proxy":include_proxy}))
                .unwrap();
            bytes += serde_json::to_vec(&p).unwrap().len();
        }
        report.push(json!({"include_proxy":include_proxy,"iterations":100,"elapsed_ms":began.elapsed().as_secs_f64()*1000.,"bytes_per_poll":bytes/100}));
    }
    r.shutdown().unwrap();
    let report = json!({"rooms":300,"proxy_candidates":1000,"results":report});
    if let Some(path) = std::env::args().nth(1) {
        h_live_runtime::storage::atomic(
            std::path::Path::new(&path),
            &serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
    println!("{report}");
}
