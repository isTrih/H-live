use super::*;

#[test]
fn rotation_replaces_the_previous_file_and_keeps_both_logs_bounded() {
    let directory = tempfile::tempdir().unwrap();
    let current = directory.path().join("runtime.jsonl");
    let previous = directory.path().join("runtime.previous.jsonl");
    std::fs::write(&current, vec![b'x'; RUNTIME_LOG_LIMIT as usize]).unwrap();
    std::fs::write(&previous, b"older log").unwrap();
    append_runtime_log(directory.path(), "new warning").unwrap();
    assert_eq!(
        std::fs::metadata(&previous).unwrap().len(),
        RUNTIME_LOG_LIMIT
    );
    assert_eq!(std::fs::read_to_string(&current).unwrap(), "new warning\n");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
    append_runtime_log(directory.path(), &"x".repeat(16 * 1024 + 1)).unwrap();
    assert_eq!(std::fs::read_to_string(&current).unwrap(), "new warning\n");
}

#[test]
fn failed_rotation_does_not_allow_the_current_log_to_grow() {
    let directory = tempfile::tempdir().unwrap();
    let current = directory.path().join("runtime.jsonl");
    std::fs::write(&current, vec![b'x'; RUNTIME_LOG_LIMIT as usize]).unwrap();
    std::fs::create_dir(directory.path().join("runtime.previous.jsonl")).unwrap();
    assert!(append_runtime_log(directory.path(), "warning").is_err());
    assert_eq!(std::fs::metadata(current).unwrap().len(), RUNTIME_LOG_LIMIT);
}

#[cfg(feature = "test-support")]
fn open(directory: &std::path::Path) -> Runtime {
    Runtime::open_with_transport(
        directory.into(),
        Arc::new(crate::testing::Simulation::default()),
    )
    .unwrap()
}

#[cfg(feature = "test-support")]
fn add_test_room(runtime: &Runtime) {
    runtime
        .request("add_anchors", json!({"values":"1"}))
        .unwrap();
    runtime.shared.state.lock().unwrap().rooms.insert(
        "1".into(),
        Room {
            generation: 7,
            value: json!({"rid":"1","name":"测试主播","active":false,"state":"starting","anchor_id":"11","session_id":"101"}),
            route: None,
            cancel: CancellationToken::new(),
        },
    );
}

#[cfg(feature = "test-support")]
#[test]
fn normal_logging_updates_room_state_without_recording_routine_polling() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = open(directory.path());
    add_test_room(&runtime);
    assert_eq!(
        runtime.request("init", json!({})).unwrap()["settings"]["debug_logging"],
        false
    );
    for state in [
        "starting",
        "waiting_connection",
        "offline",
        "connecting",
        "connected",
        "stopped",
    ] {
        runtime.shared.status("1", 7, state, "常规状态");
    }
    runtime.shared.log("1", "平台确认本场直播结束");
    let poll = runtime.request("poll", json!({})).unwrap();
    assert!(poll["events"].as_array().unwrap().is_empty());
    assert_eq!(poll["anchors"][0]["state"], "stopped");
    assert_eq!(poll["settings"]["debug_logging"], false);

    runtime.shared.status("1", 7, "error", "磁盘写入失败");
    runtime.shared.status("1", 7, "retrying", "网络连接失败");
    runtime.shared.warning("", "偏好保存失败");
    let poll = runtime.request("poll", json!({})).unwrap();
    assert_eq!(poll["events"].as_array().unwrap().len(), 3);
    runtime.shutdown().unwrap();
    let raw = std::fs::read_to_string(directory.path().join("logs/runtime.jsonl")).unwrap();
    assert!(raw.contains("磁盘写入失败"));
    assert!(raw.contains("网络连接失败"));
    assert!(raw.contains("偏好保存失败"));
    assert!(!raw.contains("常规状态"));
    assert!(!raw.contains("平台确认本场直播结束"));
}

#[cfg(feature = "test-support")]
#[test]
fn debug_toggle_is_immediate_persistent_and_preserves_messages_and_errors() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = open(directory.path());
    add_test_room(&runtime);
    assert_eq!(
        runtime
            .request("save_settings", json!({"debug_logging":true}))
            .unwrap()["debug_logging"],
        true
    );
    runtime.shared.status("1", 7, "offline", "调试轮询状态");
    runtime.shared.log("1", "调试生命周期日志");
    assert_eq!(
        runtime.request("poll", json!({})).unwrap()["events"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    runtime
        .shared
        .state
        .lock()
        .unwrap()
        .rooms
        .get_mut("1")
        .unwrap()
        .value["active"] = json!(true);
    runtime.shared.message(
        "1",
        7,
        "chat",
        &json!({"text":"private-chat-content","token":"fixture-secret"}),
        "",
        "2026-10-23T14:00:00+08:00",
    );
    runtime.shared.status("1", 7, "error", "实际异常");
    runtime
        .shared
        .state
        .lock()
        .unwrap()
        .rooms
        .get_mut("1")
        .unwrap()
        .value["active"] = json!(false);
    assert_eq!(
        runtime
            .request("save_settings", json!({"debug_logging":false}))
            .unwrap()["debug_logging"],
        false
    );
    runtime.shared.status("1", 7, "offline", "关闭后轮询状态");
    runtime.shared.log("1", "关闭后生命周期日志");
    let poll = runtime.request("poll", json!({})).unwrap();
    let events = poll["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(
        poll["dropped"], 0,
        "intentionally hidden diagnostics are not lost events"
    );
    assert!(events.iter().any(|e| e["kind"] == "message"));
    assert!(events.iter().any(|e| e["state"] == "error"));
    let cursor = poll["cursor"].as_u64().unwrap();
    let next = runtime.request("poll", json!({"after":cursor})).unwrap();
    assert_eq!(next["cursor"], cursor);
    assert!(next["events"].as_array().unwrap().is_empty());
    assert_eq!(next["dropped"], 0);
    runtime.shutdown().unwrap();
    let raw = std::fs::read_to_string(directory.path().join("logs/runtime.jsonl")).unwrap();
    assert!(raw.contains("调试轮询状态"));
    assert!(!raw.contains("关闭后轮询状态"));
    assert!(!raw.contains("关闭后生命周期日志"));
    assert!(!raw.contains("private-chat-content"));
    assert!(!raw.contains("fixture-secret"));
    drop(runtime);
    let reopened = open(directory.path());
    assert_eq!(
        reopened.request("init", json!({})).unwrap()["settings"]["debug_logging"],
        false
    );
    reopened
        .request("save_settings", json!({"debug_logging":true}))
        .unwrap();
    reopened.shutdown().unwrap();
    drop(reopened);
    let enabled = open(directory.path());
    assert_eq!(
        enabled.request("init", json!({})).unwrap()["settings"]["debug_logging"],
        true
    );
    enabled.shutdown().unwrap();
}

#[cfg(feature = "test-support")]
#[test]
fn proxy_is_disabled_by_default_but_explicit_enablement_survives_restart() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = open(directory.path());
    assert_eq!(
        runtime.request("init", json!({})).unwrap()["settings"]["proxy_enabled"],
        false
    );
    runtime
        .request(
            "configure_proxy",
            json!({"enabled":true,"source":"imported"}),
        )
        .unwrap();
    runtime.shutdown().unwrap();
    drop(runtime);
    let reopened = open(directory.path());
    assert_eq!(
        reopened.request("init", json!({})).unwrap()["settings"]["proxy_enabled"],
        true
    );
    for bad in [
        json!({}),
        json!({"debug_logging":"true"}),
        json!({"debug_logging":1}),
    ] {
        assert!(reopened.request("save_settings", bad).is_err());
    }
    assert_eq!(
        reopened.request("init", json!({})).unwrap()["settings"]["debug_logging"],
        false
    );
    reopened.shutdown().unwrap();
}

#[cfg(feature = "test-support")]
#[test]
fn disabled_proxy_skips_automatic_subscription_fetches_but_allows_manual_refresh() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = open(directory.path());
    runtime.shared.state.lock().unwrap().config.value["proxy_subscriptions"] = json!([
        {"id":"manual","url":"https://127.0.0.1/fixture","enabled":true,"updated_at":"","interval_minutes":60,"records":[]},
        {"id":"automatic","url":"https://127.0.0.1/fixture","enabled":true,"updated_at":"","interval_minutes":60,"records":[]}
    ]);
    runtime
        .rt
        .block_on(crate::maintenance::tick(&runtime.shared, 1));
    assert!(
        runtime
            .shared
            .state
            .lock()
            .unwrap()
            .subscriptions
            .is_empty(),
        "disabled pool must not automatically refresh subscriptions"
    );
    assert!(!runtime.shared.proxy_busy.load(Ordering::Acquire));
    assert!(
        runtime.rt.block_on(async {
            crate::maintenance::refresh_subscription(&runtime.shared, "manual")
        })
    );
    assert!(
        runtime
            .shared
            .state
            .lock()
            .unwrap()
            .subscriptions
            .contains_key("manual")
    );
    {
        let mut state = runtime.shared.state.lock().unwrap();
        state.pool.enabled = true;
        state.pool.source = "imported".into();
    }
    runtime
        .rt
        .block_on(crate::maintenance::tick(&runtime.shared, 2));
    assert!(
        runtime
            .shared
            .state
            .lock()
            .unwrap()
            .subscriptions
            .contains_key("automatic")
    );
    runtime.shutdown().unwrap();
}

#[cfg(feature = "test-support")]
#[test]
fn cursor_reports_actual_bounded_queue_eviction() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = open(directory.path());
    for i in 0..2002 {
        runtime.shared.warning("", &format!("fixture-warning-{i}"));
    }
    let poll = runtime.request("poll", json!({"limit":1})).unwrap();
    assert_eq!(poll["dropped"], 2);
    assert_eq!(poll["events"][0]["seq"], 3);
    assert_eq!(poll["cursor"], 3);
    let next = runtime
        .request("poll", json!({"after":3,"limit":1}))
        .unwrap();
    assert_eq!(next["events"][0]["seq"], 4);
    assert_eq!(next["dropped"], 0);
    runtime.shutdown().unwrap();
}
