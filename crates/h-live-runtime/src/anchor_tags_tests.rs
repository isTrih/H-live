use super::*;
use crate::{net::RoomInfo, runtime::Room, storage::Settings};
use qingkan_core::archive::MessageRecord;
use tokio_util::sync::CancellationToken;

#[test]
fn anchor_tag_normalization_is_bounded_unicode_aware_and_rejects_invalid_payloads() {
    assert_eq!(
        normalize(&json!(["  游戏 ", "游戏", "精选", "😀".repeat(24)])).unwrap(),
        vec!["游戏".to_owned(), "精选".to_owned(), "😀".repeat(24)]
    );
    assert_eq!(normalize(&json!([])).unwrap(), Vec::<String>::new());
    assert_eq!(normalize(&json!(vec!["same"; 21])).unwrap(), vec!["same"]);
    for value in [
        Value::Null,
        json!("标签"),
        json!({}),
        json!([3]),
        json!([""]),
        json!(["   "]),
        json!(["a\nb"]),
        json!(["a,b"]),
        json!(["a，b"]),
        json!(["中".repeat(25)]),
        json!((0..21).map(|i| format!("t{i}")).collect::<Vec<_>>()),
    ] {
        assert!(normalize(&value).is_err(), "{value}");
    }
    assert_eq!(canonical(&Value::Null).unwrap(), json!({}));
    assert_eq!(
        canonical(&json!({"11":[" a ","a"],"room:1":[]})).unwrap(),
        json!({"11":["a"],"room:1":[]})
    );
    for value in [
        json!([]),
        json!({"":["tag"]}),
        json!({"room:0":["tag"]}),
        json!({"room:../1":["tag"]}),
        json!({"11":null}),
    ] {
        assert!(validate(&value).is_err());
    }
    assert_eq!(
        canonical(&json!({"MS4wLjAB_legacy.sec-uid":["存档"],"历史身份":[]})).unwrap(),
        json!({"MS4wLjAB_legacy.sec-uid":["存档"],"历史身份":[]})
    );
    assert!(canonical(&json!({"bad\u{7f}identity":["tag"]})).is_err());
    assert!(canonical(&json!({("😀".repeat(81)):["tag"]})).is_err());
}

#[test]
fn anchor_tag_add_edit_clear_and_restart_are_atomic_and_compatible_with_old_configs() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(directory.path().into()).unwrap();
    runtime
        .request(
            "add_anchors",
            json!({"values":["1","2"],"tags":[" 游戏 ","游戏","重点"]}),
        )
        .unwrap();
    for method in ["init", "poll"] {
        let value = runtime.request(method, json!({})).unwrap();
        assert_eq!(value["anchors"][0]["tags"], json!(["游戏", "重点"]));
        assert_eq!(value["anchors"][1]["tags"], json!(["游戏", "重点"]));
    }
    runtime
        .request("add_anchors", json!({"values":"1"}))
        .unwrap();
    for tags in [json!([]), json!(["不覆盖已有标签"])] {
        let response = runtime
            .request("add_anchors", json!({"values":"1","tags":tags}))
            .unwrap();
        assert_eq!(response["added"], json!([]));
        assert_eq!(
            runtime.request("poll", json!({})).unwrap()["anchors"][0]["tags"],
            json!(["游戏", "重点"])
        );
    }
    assert_eq!(
        runtime.request("poll", json!({})).unwrap()["anchors"][0]["tags"],
        json!(["游戏", "重点"])
    );
    for value in [
        json!({"rids":["1"],"tags":null}),
        json!({"rids":["1"],"tags":"bad"}),
        json!({"rids":["1","9"],"tags":["cannot-apply"]}),
        json!({"rids":[],"tags":["cannot-apply"]}),
    ] {
        assert!(runtime.request("set_anchor_tags", value).is_err());
    }
    assert!(runtime
        .request("add_anchors", json!({"values":"3","tags":[99]}))
        .is_err());
    assert!(runtime
        .request("add_anchors", json!({"values":["3",99],"tags":["tag"]}))
        .is_err());
    assert_eq!(runtime.configured_tasks(), vec!["1", "2"]);
    runtime
        .request("set_anchor_tags", json!({"rids":["1","2"],"tags":["精选"]}))
        .unwrap();
    runtime
        .request("set_anchor_tags", json!({"rids":["2"],"tags":[]}))
        .unwrap();
    runtime.shutdown().unwrap();
    drop(runtime);
    let runtime = Runtime::open(directory.path().into()).unwrap();
    let rows = runtime.request("init", json!({})).unwrap()["anchors"].clone();
    assert_eq!(rows[0]["tags"], json!(["精选"]));
    assert_eq!(rows[1]["tags"], json!([]));
    runtime.request("remove", json!({"rids":["1"]})).unwrap();
    runtime
        .request("add_anchors", json!({"values":"1"}))
        .unwrap();
    assert_eq!(
        runtime.request("poll", json!({})).unwrap()["anchors"][0]["tags"],
        json!([])
    );
    runtime.shutdown().unwrap();
}

fn room(runtime: &Runtime, rid: &str, anchor: &str) {
    runtime.shared.state.lock().unwrap().rooms.insert(rid.into(),Room{generation:1,value:json!({"name":format!("主播 {rid}"),"anchor_id":anchor,"session_id":"","is_live":null}),route:None,cancel:CancellationToken::new()});
}
fn record(runtime: &Runtime, anchor: &str, session: &str) {
    let now = storage::now();
    runtime
        .shared
        .archive
        .record_message(
            MessageRecord {
                anchor_id: anchor.into(),
                anchor_name: format!("主播{anchor}"),
                web_rid: "1".into(),
                session_id: session.into(),
                sender_id: "31".into(),
                sender_name: "观众".into(),
                message_type: "chat".into(),
                content: "tag fixture".into(),
                message_time: now.clone(),
                received_at: now,
                time_source: "server".into(),
                message_id: format!("m{session}"),
                payload: json!({}),
            },
            "tag fixture".into(),
            None,
        )
        .unwrap();
}
fn archive_tags(runtime: &Runtime, anchor: &str) -> Value {
    runtime
        .request("archive_anchors", json!({}))
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["anchor_id"] == anchor)
        .unwrap()["tags"]
        .clone()
}

#[test]
fn anchor_tags_follow_confirmed_identity_and_survive_removal_without_reusing_room_labels() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(directory.path().into()).unwrap();
    runtime
        .request("add_anchors", json!({"values":"1","tags":["主播A"]}))
        .unwrap();
    room(&runtime, "1", "");
    assert!(runtime.shared.patch_info(
        "1",
        1,
        &RoomInfo {
            anchor_id: "11".into(),
            ..Default::default()
        }
    ));
    runtime.shared.flush_metadata().unwrap();
    {
        let state = runtime.shared.state.lock().unwrap();
        assert_eq!(state.config.value["anchor_tags"]["11"], json!(["主播A"]));
        assert!(state.config.value["anchor_tags"].get("room:1").is_none());
    }
    record(&runtime, "11", "21");
    record(&runtime, "12", "22");
    assert_eq!(archive_tags(&runtime, "11"), json!(["主播A"]));
    assert!(runtime.shared.patch_info(
        "1",
        1,
        &RoomInfo {
            anchor_id: "12".into(),
            ..Default::default()
        }
    ));
    runtime.shared.flush_metadata().unwrap();
    assert_eq!(
        runtime.request("poll", json!({})).unwrap()["anchors"][0]["tags"],
        json!([])
    );
    assert_eq!(archive_tags(&runtime, "11"), json!(["主播A"]));
    assert_eq!(archive_tags(&runtime, "12"), json!([]));
    runtime
        .request("set_anchor_tags", json!({"rids":["1"],"tags":["主播B"]}))
        .unwrap();
    runtime.request("remove", json!({"rids":["1"]})).unwrap();
    assert_eq!(archive_tags(&runtime, "11"), json!(["主播A"]));
    assert_eq!(archive_tags(&runtime, "12"), json!(["主播B"]));
    let init = runtime.request("init", json!({})).unwrap();
    assert!(init["anchors"].as_array().unwrap().is_empty());
    assert!(init["archive_anchors"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["tags"].is_array()));
    runtime.shutdown().unwrap();
    drop(runtime);
    let runtime = Runtime::open(directory.path().into()).unwrap();
    assert_eq!(archive_tags(&runtime, "11"), json!(["主播A"]));
    assert_eq!(archive_tags(&runtime, "12"), json!(["主播B"]));
    runtime.shutdown().unwrap();
}

#[test]
fn anchor_tag_pending_assignments_do_not_migrate_from_remembered_or_invalid_identity() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(directory.path().into()).unwrap();
    runtime
        .request("add_anchors", json!({"values":"1","tags":["待确认"]}))
        .unwrap();
    room(&runtime, "1", "11");
    runtime.shared.flush_metadata().unwrap();
    assert!(
        runtime.shared.state.lock().unwrap().config.value["anchor_tags"]
            .get("11")
            .is_none()
    );
    runtime
        .request(
            "set_anchor_tags",
            json!({"rids":["1"],"tags":["待确认编辑"]}),
        )
        .unwrap();
    assert!(runtime.shared.patch_info("1", 1, &RoomInfo::default()));
    runtime.shared.flush_metadata().unwrap();
    let state = runtime.shared.state.lock().unwrap();
    assert_eq!(
        state.config.value["anchor_tags"]["room:1"],
        json!(["待确认编辑"])
    );
    assert!(state.config.value["anchor_tags"].get("11").is_none());
    drop(state);
    runtime.shutdown().unwrap();
    drop(runtime);
    let runtime = Runtime::open(directory.path().into()).unwrap();
    runtime
        .request(
            "set_anchor_tags",
            json!({"rids":["1"],"tags":["重启后仍待确认"]}),
        )
        .unwrap();
    let state = runtime.shared.state.lock().unwrap();
    assert_eq!(
        state.config.value["anchor_tags"]["room:1"],
        json!(["重启后仍待确认"])
    );
    assert!(state.config.value["anchor_tags"].get("11").is_none());
    drop(state);
    runtime.shutdown().unwrap();
}

#[test]
fn invalid_persisted_anchor_tags_fail_without_rewriting_the_original_config() {
    let directory = tempfile::tempdir().unwrap();
    let bytes =
        serde_json::to_vec(&json!({"anchors":{},"anchor_tags":{"11":["bad,tag"]}})).unwrap();
    std::fs::write(directory.path().join("config.json"), &bytes).unwrap();
    assert!(Settings::load(directory.path().into()).is_err());
    assert_eq!(
        std::fs::read(directory.path().join("config.json")).unwrap(),
        bytes
    );
}

#[test]
fn anchor_tag_explicit_edit_wins_over_pending_labels_for_another_confirmed_entrance() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(directory.path().into()).unwrap();
    runtime
        .request("add_anchors", json!({"values":"1","tags":["原标签"]}))
        .unwrap();
    room(&runtime, "1", "");
    runtime.shared.patch_info(
        "1",
        1,
        &RoomInfo {
            anchor_id: "11".into(),
            ..Default::default()
        },
    );
    runtime.shared.flush_metadata().unwrap();
    runtime
        .request("add_anchors", json!({"values":"2","tags":["待迁移旧标签"]}))
        .unwrap();
    room(&runtime, "2", "");
    runtime.shared.patch_info(
        "2",
        1,
        &RoomInfo {
            anchor_id: "11".into(),
            ..Default::default()
        },
    );
    // Both rows now refer to the same confirmed person, before the five-second metadata flush.
    runtime
        .request("set_anchor_tags", json!({"rids":["1"],"tags":["最新标签"]}))
        .unwrap();
    runtime.shared.flush_metadata().unwrap();
    let rows = runtime.request("poll", json!({})).unwrap()["anchors"].clone();
    assert_eq!(rows[0]["tags"], json!(["最新标签"]));
    assert_eq!(rows[1]["tags"], json!(["最新标签"]));
    let state = runtime.shared.state.lock().unwrap();
    assert!(state.config.value["anchor_tags"].get("room:2").is_none());
    drop(state);
    runtime.shutdown().unwrap();
}

#[test]
fn anchor_tag_late_confirmation_keeps_stable_labels_including_an_explicit_clear() {
    for edited in [json!(["最新标签"]), json!([])] {
        let directory = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(directory.path().into()).unwrap();
        runtime
            .request("add_anchors", json!({"values":"1","tags":["原标签"]}))
            .unwrap();
        room(&runtime, "1", "");
        runtime.shared.patch_info(
            "1",
            1,
            &RoomInfo {
                anchor_id: "11".into(),
                ..Default::default()
            },
        );
        runtime.shared.flush_metadata().unwrap();
        runtime
            .request(
                "add_anchors",
                json!({"values":"2","tags":["身份仍未知的旧标签"]}),
            )
            .unwrap();
        // This entrance is still unresolved when the confirmed person is explicitly edited.
        runtime
            .request("set_anchor_tags", json!({"rids":["1"],"tags":edited}))
            .unwrap();
        room(&runtime, "2", "");
        runtime.shared.patch_info(
            "2",
            1,
            &RoomInfo {
                anchor_id: "11".into(),
                ..Default::default()
            },
        );
        runtime.shared.flush_metadata().unwrap();
        let rows = runtime.request("poll", json!({})).unwrap()["anchors"].clone();
        assert_eq!(rows[0]["tags"], edited);
        assert_eq!(rows[1]["tags"], edited);
        let state = runtime.shared.state.lock().unwrap();
        assert_eq!(state.config.value["anchor_tags"]["11"], edited);
        assert!(state.config.value["anchor_tags"].get("room:2").is_none());
        drop(state);
        runtime.shutdown().unwrap();
    }
}
