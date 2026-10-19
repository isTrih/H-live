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
fn concurrent_rooms_share_one_anonymous_visitor_per_exit() {
    let d = tempfile::tempdir().unwrap();
    let simulation = Arc::new(Simulation {
        live_rooms: 3,
        ..Default::default()
    });
    let counters = simulation.counters.clone();
    let r = Runtime::open_with_transport(d.path().into(), simulation).unwrap();
    r.request("add_anchors", json!({"values":"1\n2\n3"}))
        .unwrap();
    r.request("start", json!({"rids":["1","2","3"]})).unwrap();
    until(&r, |p| p["metrics"]["connected"] == 3);
    assert_eq!(
        counters.visitors.load(std::sync::atomic::Ordering::Acquire),
        1,
        "same exit must not fetch the anonymous homepage once per room"
    );
    r.shutdown().unwrap();
}

#[test]
fn anonymous_cache_is_exit_scoped_expiring_invalidatable_and_cancelable() {
    let dir = tempfile::tempdir().unwrap();
    let simulation = Arc::new(Simulation::default());
    let counters = simulation.counters.clone();
    let r = Runtime::open_with_transport(dir.path().into(), simulation).unwrap();
    r.rt.block_on(async {
        use crate::proxy::{ProxyRecord, Route};
        use tokio_util::sync::CancellationToken;
        let direct = Route::new(None).unwrap();
        let proxy = Route::new(Some(
            ProxyRecord::from_line("8.8.8.8:8080", "http").unwrap(),
        ))
        .unwrap();
        let cancel = CancellationToken::new();
        let first = r.shared.visitor(&direct, &cancel).await.unwrap();
        r.shared.visitor(&proxy, &cancel).await.unwrap();
        assert_eq!(
            counters.visitors.load(std::sync::atomic::Ordering::Acquire),
            2
        );
        let wrong = crate::net::Visitor {
            cookie: "old".into(),
            did: "old".into(),
        };
        r.shared.invalidate_visitor(&direct, Some(&wrong)).await;
        r.shared.visitor(&direct, &cancel).await.unwrap();
        assert_eq!(
            counters.visitors.load(std::sync::atomic::Ordering::Acquire),
            2
        );
        r.shared.invalidate_visitor(&direct, Some(&first)).await;
        r.shared.visitor(&direct, &cancel).await.unwrap();
        assert_eq!(
            counters.visitors.load(std::sync::atomic::Ordering::Acquire),
            3
        );
        direct.visitor.lock().await.as_mut().unwrap().0 =
            tokio::time::Instant::now() - Duration::from_secs(601);
        r.shared.visitor(&direct, &cancel).await.unwrap();
        assert_eq!(
            counters.visitors.load(std::sync::atomic::Ordering::Acquire),
            4
        );
        let lock = direct.visitor.lock().await;
        cancel.cancel();
        assert!(matches!(
            tokio::time::timeout(
                Duration::from_millis(100),
                r.shared.visitor(&direct, &cancel)
            )
            .await
            .unwrap(),
            Err(crate::net::Error::Cancelled)
        ));
        drop(lock);
        r.shared
            .visitor(&direct, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            counters.visitors.load(std::sync::atomic::Ordering::Acquire),
            4
        );
    });
    r.shutdown().unwrap();
}

#[test]
fn activity_lifecycle_updates_dashboard_without_counting_notifications_as_rounds() {
    let dir = tempfile::tempdir().unwrap();
    let r = Runtime::open_with_transport(
        dir.path().into(),
        Arc::new(Simulation {
            live_rooms: 1,
            interval_ms: 600,
            activity_lifecycle: true,
            ..Default::default()
        }),
    )
    .unwrap();
    r.request("add_anchors", json!({"values":"1"})).unwrap();
    r.request("start", json!({"rids":["1"]})).unwrap();
    let first = until(&r, |p| {
        p["anchors"][0]["activities"]["red_packet"]["active_until"].is_number()
    });
    assert_eq!(first["anchors"][0]["activities"]["red_packet"]["total"], 1);
    let ended = until(&r, |p| {
        p["anchors"][0]["activities"]["red_packet"]["total"] == 1
            && p["anchors"][0]["activities"]["red_packet"]["active_until"].is_null()
    });
    assert_eq!(
        ended["anchors"][0]["activities"]["red_packet"]["uncertain"],
        false
    );
    r.request("stop", json!({"rids":["1"]})).unwrap();
    let messages = r
        .request("recent_messages", json!({"type":"activity"}))
        .unwrap();
    assert_eq!(messages["messages"].as_array().unwrap().len(), 1);
    r.request("start", json!({"rids":["1"]})).unwrap();
    let resumed = until(&r, |p| p["anchors"][0]["state"] == "connected");
    assert_eq!(
        resumed["anchors"][0]["activities"]["red_packet"]["total"],
        1
    );
    assert!(resumed["anchors"][0]["activities"]["red_packet"]["active_until"].is_null());
    r.shutdown().unwrap();
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
