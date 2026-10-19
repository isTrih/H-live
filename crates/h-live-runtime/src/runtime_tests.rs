#![cfg(feature = "test-support")]
use crate::{testing::Simulation, Runtime};
use serde_json::{json, Value};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn setup() -> (tempfile::TempDir, Runtime) {
    let d = tempfile::tempdir().unwrap();
    let r = Runtime::open_with_transport(
        d.path().into(),
        Arc::new(Simulation {
            live_rooms: 2,
            ..Default::default()
        }),
    )
    .unwrap();
    (d, r)
}
#[test]
fn slim_dashboard_poll_skips_proxy_inventory_but_keeps_collector_health() {
    let (_d, r) = setup();
    let full = r.request("poll", json!({})).unwrap();
    assert!(full["metrics"]["proxy"].is_object());
    let slim = r.request("poll", json!({"include_proxy":false})).unwrap();
    assert!(slim["metrics"].get("proxy").is_none());
    assert!(slim["anchors"].is_array());
    assert!(slim["metrics"]["archive"].is_object());
    assert!(slim["metrics"]["viewer_refresh"].is_object());
    assert!(r.request("proxy_status", json!({})).unwrap().is_object());
    r.shutdown().unwrap();
}
fn until(r: &Runtime, check: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let p = r.request("poll", json!({})).unwrap();
        if check(&p) {
            return p;
        }
        assert!(Instant::now() < deadline, "{}", p["anchors"]);
        std::thread::sleep(Duration::from_millis(40));
    }
}
#[test]
fn offline_poll_starts_at_thirty_seconds_restart_resets_and_live_preferences_persist() {
    let (d, r) = setup();
    r.request("add_anchors", json!({"values":"1\n3"})).unwrap();
    r.request("start", json!({"rids":["1","3"]})).unwrap();
    let p = until(&r, |p| {
        p["anchors"][1]["state"] == "offline" && p["anchors"][0]["state"] == "connected"
    });
    assert_eq!(p["anchors"][1]["poll_interval_seconds"], 30);
    assert_eq!(p["anchors"][1]["offline_checks"], 1);
    assert!(p["anchors"][0]["poll_interval_seconds"].is_null());
    r.request("stop", json!({"rids":["3"]})).unwrap();
    r.request("start", json!({"rids":["3"]})).unwrap();
    let p = until(&r, |p| p["anchors"][1]["state"] == "offline");
    assert_eq!(p["anchors"][1]["offline_checks"], 1);
    r.shutdown().unwrap();
    let cache: Value =
        serde_json::from_slice(&std::fs::read(d.path().join("polling-habits.json")).unwrap())
            .unwrap();
    assert_eq!(cache["aliases"]["1"], "100001");
    assert!(cache["aliases"].get("3").is_none()); // An offline response is never a learned live slot.
}
#[test]
fn production_pipeline_records_chat_ignores_decoded_gift_and_stops() {
    let (_d, r) = setup();
    r.request("add_anchors", json!({"values":"1\n2"})).unwrap();
    r.request("start", json!({"rids":["1","2"]})).unwrap();
    until(&r, |p| {
        p["anchors"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["message_count"].as_u64().unwrap_or(0) >= 3)
    });
    r.request("stop", json!({"rids":["1","2"]})).unwrap();
    assert!(!r
        .request("recent_messages", json!({"type":"chat"}))
        .unwrap()["messages"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(r
        .request("recent_messages", json!({"type":"gift"}))
        .unwrap()["messages"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(r
        .request("set_cookie", json!({"cookie":"synthetic-secret"}))
        .is_err());
    let before = r.shared.archive.metrics()["committed_messages"].clone();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(r.shared.archive.metrics()["committed_messages"], before);
    assert_eq!(
        r.request("quality_sessions", json!({"anchor_id":"100001"}))
            .unwrap()["total"],
        1
    );
    assert_eq!(r.shutdown().unwrap()["all_workers_stopped"], true);
}
#[test]
fn same_session_resumes_duration_without_paused_or_reconnect_setup_time() {
    let (_d, r) = setup();
    r.request("add_anchors", json!({"values":"1"})).unwrap();
    r.request("start", json!({"rids":["1"]})).unwrap();
    until(&r, |p| {
        p["anchors"][0]["message_count"].as_u64().unwrap_or(0) > 1
    });
    r.request("stop", json!({"rids":["1"]})).unwrap();
    let first = r
        .shared
        .archive
        .load_monitor_duration("100001", "200001")
        .unwrap();
    assert!(first > 0.);
    std::thread::sleep(Duration::from_secs(1));
    r.request("start", json!({"rids":["1"]})).unwrap();
    until(&r, |p| {
        p["anchors"][0]["message_count"].as_u64().unwrap_or(0) > 1
    });
    r.request("stop", json!({"rids":["1"]})).unwrap();
    let second = r
        .shared
        .archive
        .load_monitor_duration("100001", "200001")
        .unwrap();
    assert!(second >= first);
    assert!(second - first < 1., "pause counted: {first} -> {second}");
    r.shutdown().unwrap();
}
#[test]
fn update_confirmation_stops_and_cancel_recovers_saved_tasks() {
    let (d, r) = setup();
    r.request("add_anchors", json!({"values":"1"})).unwrap();
    r.request("start", json!({"rids":["1"]})).unwrap();
    until(&r, |p| p["anchors"][0]["state"] == "connected");
    assert_eq!(r.update_readiness()["reason"], "monitoring");
    assert_eq!(
        r.request("prepare_update", json!({"target_version":"0.3.1"}))
            .unwrap()["ready"],
        true
    );
    assert!(d.path().join("update-resume.json").is_file());
    assert!(r.request("start", json!({"rids":["1"]})).is_err());
    assert_eq!(
        r.request("cancel_update", json!({})).unwrap()["recovery_queued"],
        true
    );
    until(&r, |p| p["anchors"][0]["state"] == "connected");
    assert!(!d.path().join("update-resume.json").exists());
    r.shutdown().unwrap();
}
#[test]
fn delete_is_guarded_while_monitoring_and_async_when_stopped() {
    let (_d, r) = setup();
    r.request("add_anchors", json!({"values":"1"})).unwrap();
    r.request("start", json!({"rids":["1"]})).unwrap();
    until(&r, |p| {
        p["anchors"][0]["message_count"].as_u64().unwrap_or(0) > 1
    });
    assert_eq!(
        r.request("preview_archive_delete", json!({"anchor_ids":["100001"]}))
            .unwrap()["ready"],
        false
    );
    r.request("stop", json!({"rids":["1"]})).unwrap();
    let job = r
        .request("execute_archive_delete", json!({"anchor_ids":["100001"]}))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = r
            .request("archive_delete_status", json!({"job_id":job["job_id"]}))
            .unwrap();
        if status["status"] == "completed" {
            break;
        }
        assert!(Instant::now() < deadline, "{status}");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(r
        .request("recent_messages", json!({"type":"chat"}))
        .unwrap()["messages"]
        .as_array()
        .unwrap()
        .is_empty());
    r.shutdown().unwrap();
}
#[test]
fn unknown_legacy_login_fields_are_not_decrypted_or_exposed() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("config.json"),r#"{"anchors":{},"cookie_encrypted":"not-real-base64--legacy","anchor_cookies_encrypted":"opaque","imported_proxies":[],"proxy_subscriptions":[]}"#).unwrap();
    let r = Runtime::open_with_transport(d.path().into(), Arc::new(Simulation::default())).unwrap();
    let v = r.request("init", json!({})).unwrap();
    assert!(!v.to_string().contains("opaque"));
    assert!(!v.to_string().contains("not-real-base64"));
    assert_eq!(v["anchors"], json!([]));
    r.shutdown().unwrap();
}
#[test]
fn complete_native_self_test_includes_real_exports() {
    assert_eq!(
        crate::offline_self_test().unwrap()["csv_xlsx_exports"],
        true
    );
}
