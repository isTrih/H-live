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

struct DeferredRoomSimulation {
    simulation: Simulation,
    defer_rooms: std::sync::atomic::AtomicBool,
}
impl crate::transport::Transport for DeferredRoomSimulation {
    fn visitor<'a>(
        &'a self,
        route: &'a crate::proxy::Route,
        cancel: &'a tokio_util::sync::CancellationToken,
    ) -> futures_util::future::BoxFuture<'a, Result<crate::net::Visitor, crate::net::Error>> {
        crate::transport::Transport::visitor(&self.simulation, route, cancel)
    }
    fn room<'a>(
        &'a self,
        route: &'a crate::proxy::Route,
        visitor: &'a crate::net::Visitor,
        rid: &'a str,
        cancel: &'a tokio_util::sync::CancellationToken,
    ) -> futures_util::future::BoxFuture<'a, Result<crate::net::RoomInfo, crate::net::Error>> {
        Box::pin(async move {
            if self.defer_rooms.load(std::sync::atomic::Ordering::Acquire) {
                cancel.cancelled().await;
                Err(crate::net::Error::Cancelled)
            } else {
                crate::transport::Transport::room(&self.simulation, route, visitor, rid, cancel)
                    .await
            }
        })
    }
    fn connect<'a>(
        &'a self,
        route: Arc<crate::proxy::Route>,
        visitor: &'a crate::net::Visitor,
        rid: &'a str,
        cancel: &'a tokio_util::sync::CancellationToken,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<Box<dyn crate::transport::MessageSocket>, crate::net::Error>,
    > {
        crate::transport::Transport::connect(&self.simulation, route, visitor, rid, cancel)
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
fn restarting_then_monitoring_an_offline_room_preserves_its_archived_identity() {
    let dir = tempfile::tempdir().unwrap();
    let first = Runtime::open_with_transport(
        dir.path().into(),
        Arc::new(Simulation {
            live_rooms: 1,
            ..Default::default()
        }),
    )
    .unwrap();
    first.request("add_anchors", json!({"values":"1"})).unwrap();
    first.request("start", json!({"rids":["1"]})).unwrap();
    let live = until(&first, |p| {
        p["anchors"][0]["message_count"].as_u64().unwrap_or(0) >= 2
    });
    assert_eq!(live["anchors"][0]["anchor_id"], "100001");
    assert_eq!(live["anchors"][0]["session_id"], "200001");
    first.shutdown().unwrap();
    drop(first);

    let restarted = Runtime::open_with_transport(
        dir.path().into(),
        Arc::new(Simulation {
            live_rooms: 0,
            ..Default::default()
        }),
    )
    .unwrap();
    let init = restarted.request("init", json!({})).unwrap();
    let archived_anchor = init["archive_anchors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|anchor| anchor["web_rid"] == "1")
        .unwrap();
    assert_eq!(archived_anchor["anchor_id"], "100001");
    assert_eq!(
        init["anchors"][0]["anchor_id"],
        archived_anchor["anchor_id"]
    );
    assert!(init["anchors"][0]["is_live"].is_null());
    assert_eq!(init["anchors"][0]["active"], false);
    assert_ne!(init["anchors"][0]["duration_running"], true);

    restarted.request("start", json!({"rids":["1"]})).unwrap();
    let offline = until(&restarted, |p| p["anchors"][0]["state"] == "offline");
    let anchor = &offline["anchors"][0];
    assert_eq!(anchor["is_live"], false);
    assert_ne!(anchor["duration_running"], true);
    assert_eq!(
        anchor["anchor_id"], archived_anchor["anchor_id"],
        "offline monitoring must retain the saved identity used by the archive"
    );
    let sessions = restarted
        .request("quality_sessions", json!({"anchor_id":anchor["anchor_id"]}))
        .unwrap();
    assert_eq!(sessions["total"], 1);
    assert_eq!(sessions["sessions"][0]["session_id"], "200001");
    let timeline = restarted
        .request(
            "session_timeline",
            json!({"anchor_id":anchor["anchor_id"],"session_id":sessions["sessions"][0]["session_id"]}),
        )
        .unwrap();
    assert!(timeline["chat_count"].as_u64().unwrap() >= 2);
    restarted.shutdown().unwrap();
    drop(restarted);

    let reopened = Runtime::open_with_transport(
        dir.path().into(),
        Arc::new(Simulation {
            live_rooms: 0,
            ..Default::default()
        }),
    )
    .unwrap();
    let init = reopened.request("init", json!({})).unwrap();
    assert_eq!(init["anchors"][0]["anchor_id"], "100001");
    assert!(init["anchors"][0]["is_live"].is_null());
    reopened.shutdown().unwrap();
}
#[test]
fn restarting_repairs_missing_or_empty_room_metadata_from_the_archive() {
    let dir = tempfile::tempdir().unwrap();
    let first = Runtime::open_with_transport(
        dir.path().into(),
        Arc::new(Simulation {
            live_rooms: 1,
            ..Default::default()
        }),
    )
    .unwrap();
    first.request("add_anchors", json!({"values":"1"})).unwrap();
    first.request("start", json!({"rids":["1"]})).unwrap();
    until(&first, |p| {
        p["anchors"][0]["message_count"].as_u64().unwrap_or(0) >= 2
    });
    first.shutdown().unwrap();
    drop(first);

    for missing in [true, false] {
        let config_path = dir.path().join("config.json");
        let mut config: Value =
            serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
        if missing {
            config.as_object_mut().unwrap().remove("anchor_metadata");
        } else {
            config["anchor_metadata"] = json!({"1":{"anchor_id":"","session_id":""}});
        }
        std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();

        let reopened = Runtime::open_with_transport(
            dir.path().into(),
            Arc::new(Simulation {
                live_rooms: 0,
                ..Default::default()
            }),
        )
        .unwrap();
        let init = reopened.request("init", json!({})).unwrap();
        let anchor = &init["anchors"][0];
        assert_eq!(
            anchor["anchor_id"],
            "100001",
            "archive web_rid must repair {} metadata before any network request",
            if missing { "missing" } else { "empty" }
        );
        assert_eq!(anchor["session_id"], "200001");
        assert!(anchor["is_live"].is_null());
        assert!(anchor["viewer_count"].is_null());
        assert_ne!(anchor["duration_running"], true);
        let sessions = reopened
            .request("quality_sessions", json!({"anchor_id":anchor["anchor_id"]}))
            .unwrap();
        assert_eq!(sessions["total"], 1);
        let timeline = reopened
            .request(
                "session_timeline",
                json!({"anchor_id":anchor["anchor_id"],"session_id":sessions["sessions"][0]["session_id"]}),
            )
            .unwrap();
        assert!(timeline["chat_count"].as_u64().unwrap() >= 2);
        reopened.shutdown().unwrap();
        drop(reopened);
        let saved: Value = serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
        assert_eq!(saved["anchor_metadata"]["1"]["anchor_id"], "100001");
    }
}

#[test]
fn metadata_flush_does_not_replace_saved_identity_with_a_temporary_blank() {
    let (_dir, r) = setup();
    r.request("add_anchors", json!({"values":"1"})).unwrap();
    {
        let mut state = r.shared.state.lock().unwrap();
        let mut config = state.config.value.clone();
        config["anchor_metadata"] = json!({"1":{"anchor_id":"100001","session_id":"200001"}});
        state.config.replace(config).unwrap();
        state.rooms.insert(
            "1".into(),
            crate::runtime::Room {
                generation: 1,
                value: json!({"name":"主播 1","anchor_id":"","session_id":""}),
                route: None,
                cancel: tokio_util::sync::CancellationToken::new(),
            },
        );
    }
    for (anchor, session) in [("", ""), ("100001", ""), ("", "200001")] {
        {
            let mut state = r.shared.state.lock().unwrap();
            let room = state.rooms.get_mut("1").unwrap();
            room.value["anchor_id"] = json!(anchor);
            room.value["session_id"] = json!(session);
        }
        r.shared.flush_metadata().unwrap();
        let state = r.shared.state.lock().unwrap();
        assert_eq!(
            state.config.value["anchor_metadata"]["1"]["anchor_id"],
            "100001"
        );
        assert_eq!(
            state.config.value["anchor_metadata"]["1"]["session_id"],
            "200001"
        );
    }
    r.shutdown().unwrap();
}

#[test]
fn quick_stop_start_keeps_identity_before_metadata_flush_or_next_room_response() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(DeferredRoomSimulation {
        simulation: Simulation {
            live_rooms: 1,
            ..Default::default()
        },
        defer_rooms: std::sync::atomic::AtomicBool::new(false),
    });
    let r = Runtime::open_with_transport(dir.path().into(), transport.clone()).unwrap();
    r.request("add_anchors", json!({"values":"1"})).unwrap();
    r.request("start", json!({"rids":["1"]})).unwrap();
    until(&r, |p| p["anchors"][0]["state"] == "connected");
    transport
        .defer_rooms
        .store(true, std::sync::atomic::Ordering::Release);
    r.request("stop", json!({"rids":["1"]})).unwrap();
    // Make the unflushed-config case explicit instead of depending on the timer.
    {
        let mut state = r.shared.state.lock().unwrap();
        let mut config = state.config.value.clone();
        config["anchor_metadata"] = json!({});
        state.config.replace(config).unwrap();
    }
    r.request("start", json!({"rids":["1"]})).unwrap();
    let poll = r.request("poll", json!({})).unwrap();
    assert_eq!(poll["anchors"][0]["anchor_id"], "100001");
    assert_eq!(poll["anchors"][0]["session_id"], "200001");
    assert!(poll["anchors"][0]["is_live"].is_null());
    assert!(poll["anchors"][0]["viewer_count"].is_null());
    assert_ne!(poll["anchors"][0]["duration_running"], true);
    r.request("stop", json!({"rids":["1"]})).unwrap();
    r.shutdown().unwrap();
    let saved: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("config.json")).unwrap()).unwrap();
    assert_eq!(saved["anchor_metadata"]["1"]["anchor_id"], "100001");
    assert_eq!(saved["anchor_metadata"]["1"]["session_id"], "200001");
}

#[test]
fn restarting_disambiguates_archive_sessions_only_with_a_known_anchor_and_keeps_custom_name() {
    let dir = tempfile::tempdir().unwrap();
    let first = Runtime::open_with_transport(
        dir.path().into(),
        Arc::new(Simulation {
            live_rooms: 0,
            ..Default::default()
        }),
    )
    .unwrap();
    first.request("add_anchors", json!({"values":"1"})).unwrap();
    first
        .request("rename", json!({"rid":"1","name":"我的自定义主播名"}))
        .unwrap();
    for (anchor, session, timestamp) in [
        ("100001", "200001", "2026-10-22T14:00:00+08:00"),
        ("100002", "200002", "2026-10-23T14:00:00+08:00"),
    ] {
        let record = serde_json::from_value(json!({
            "anchor_id":anchor,"anchor_name":format!("归档主播 {anchor}"),"web_rid":"1",
            "session_id":session,"sender_id":"9001","sender_name":"合成观众",
            "message_type":"chat","content":"合成历史消息","message_id":session,
            "message_time":timestamp,"received_at":timestamp,"time_source":"server","payload":{}
        }))
        .unwrap();
        first
            .shared
            .archive
            .record_message(record, "合成场次".into(), Some(timestamp.into()))
            .unwrap();
    }
    first.shutdown().unwrap();
    drop(first);

    for known_anchor in ["", "100001", "100002"] {
        let config_path = dir.path().join("config.json");
        let mut config: Value =
            serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
        config["anchor_metadata"] = if known_anchor.is_empty() {
            json!({})
        } else {
            json!({"1":{"anchor_id":known_anchor,"session_id":""}})
        };
        std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        let reopened = Runtime::open_with_transport(
            dir.path().into(),
            Arc::new(Simulation {
                live_rooms: 0,
                ..Default::default()
            }),
        )
        .unwrap();
        let init = reopened.request("init", json!({})).unwrap();
        let room = &init["anchors"][0];
        assert_eq!(room["name"], "我的自定义主播名");
        assert_eq!(room["anchor_id"], known_anchor);
        assert_eq!(
            room["session_id"],
            match known_anchor {
                "100001" => "200001",
                "100002" => "200002",
                _ => "",
            }
        );
        assert!(room["is_live"].is_null());
        assert!(room["viewer_count"].is_null());
        assert_eq!(init["archive_anchors"].as_array().unwrap().len(), 2);
        reopened.shutdown().unwrap();
    }
}

#[test]
fn restarting_recovers_a_known_anchor_across_aliases_without_matching_unknown_rooms_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let first = Runtime::open_with_transport(
        dir.path().into(),
        Arc::new(Simulation {
            live_rooms: 0,
            ..Default::default()
        }),
    )
    .unwrap();
    first
        .request("add_anchors", json!({"values":"1\n3"}))
        .unwrap();
    first
        .request("rename", json!({"rid":"3","name":"同名主播"}))
        .unwrap();
    {
        let mut state = first.shared.state.lock().unwrap();
        let mut config = state.config.value.clone();
        config["anchor_metadata"] = json!({"1":{"anchor_id":"100001","session_id":""}});
        state.config.replace(config).unwrap();
    }
    let timestamp = "2026-10-23T14:00:00+08:00";
    let record = serde_json::from_value(json!({
        "anchor_id":"100001","anchor_name":"同名主播","web_rid":"2",
        "session_id":"200001","sender_id":"9001","sender_name":"合成观众",
        "message_type":"chat","content":"合成历史消息","message_id":"200001",
        "message_time":timestamp,"received_at":timestamp,"time_source":"server","payload":{}
    }))
    .unwrap();
    first
        .shared
        .archive
        .record_message(record, "合成场次".into(), Some(timestamp.into()))
        .unwrap();
    first.shutdown().unwrap();
    drop(first);

    let reopened = Runtime::open_with_transport(
        dir.path().into(),
        Arc::new(Simulation {
            live_rooms: 0,
            ..Default::default()
        }),
    )
    .unwrap();
    let init = reopened.request("init", json!({})).unwrap();
    assert_eq!(init["archive_anchors"][0]["web_rid"], "2");
    let rooms = init["anchors"].as_array().unwrap();
    let known = rooms.iter().find(|room| room["rid"] == "1").unwrap();
    assert_eq!(known["anchor_id"], "100001");
    assert_eq!(known["session_id"], "200001");
    assert!(known["is_live"].is_null());
    let unknown = rooms.iter().find(|room| room["rid"] == "3").unwrap();
    assert_eq!(unknown["name"], "同名主播");
    assert_eq!(unknown["anchor_id"], "");
    assert_eq!(unknown["session_id"], "");
    reopened.shutdown().unwrap();
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
fn native_launch_starts_configured_rooms_and_respects_persisted_opt_out() {
    let (d, r) = setup();
    r.request("add_anchors", json!({"values":"1\n2"})).unwrap();
    assert_eq!(r.request("startup_settings", json!({})).unwrap()["startup_monitoring"], true);
    r.request("start_on_launch", json!({})).unwrap();
    until(&r, |p| p["anchors"].as_array().unwrap().iter().all(|a| a["state"] == "connected"));
    assert_eq!(r.running_tasks().len(), 2);
    r.request("start_on_launch", json!({})).unwrap();
    assert_eq!(r.running_tasks().len(), 2);
    r.request("configure_startup", json!({"startup_monitoring":false})).unwrap();
    assert!(r.request("configure_startup", json!({"startup_monitoring":"yes"})).is_err());
    r.shutdown().unwrap(); drop(r);
    let r = Runtime::open_with_transport(d.path().into(), Arc::new(Simulation::default())).unwrap();
    assert_eq!(r.request("startup_settings", json!({})).unwrap()["startup_monitoring"], false);
    r.request("start_on_launch", json!({})).unwrap();
    assert!(r.running_tasks().is_empty());
    r.shutdown().unwrap();
}
#[test]
fn update_restart_restores_monitoring_even_if_normal_launch_monitoring_is_disabled() {
    let (d, r) = setup();
    r.request("add_anchors", json!({"values":"1"})).unwrap();
    r.request("configure_startup", json!({"startup_monitoring":false})).unwrap();
    r.request("start", json!({"rids":["1"]})).unwrap();
    until(&r, |p| p["anchors"][0]["state"] == "connected");
    r.request("prepare_update", json!({"target_version":env!("CARGO_PKG_VERSION")})).unwrap();
    r.shutdown().unwrap(); drop(r);
    let r = Runtime::open_with_transport(d.path().into(), Arc::new(Simulation {live_rooms:2,..Default::default()})).unwrap();
    r.request("start_on_launch", json!({})).unwrap();
    until(&r, |p| p["anchors"][0]["state"] == "connected");
    assert_eq!(r.running_tasks(), vec!["1".to_string()]);
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
