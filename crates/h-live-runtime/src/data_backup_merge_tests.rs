use super::*;
use qingkan_core::archive::MessageRecord;

const SECRET: &str = "EXCLUDED_ANCHOR_12_PRIVATE_PAYLOAD_9f34a752";
fn record(runtime: &Runtime, anchor: &str, session: &str, id: &str, content: &str) {
    runtime
        .shared
        .archive
        .record_message(
            MessageRecord {
                anchor_id: anchor.into(),
                anchor_name: format!("主播{anchor}"),
                web_rid: if anchor == "11" { "1" } else { "11" }.into(),
                session_id: session.into(),
                sender_id: "31".into(),
                sender_name: "观众".into(),
                message_type: "chat".into(),
                content: content.into(),
                message_time: "2026-10-26T12:00:00+08:00".into(),
                received_at: "2026-10-26T12:00:01+08:00".into(),
                time_source: "server".into(),
                message_id: id.into(),
                payload: json!({"content":content}),
            },
            "备份测试".into(),
            None,
        )
        .unwrap();
}
fn fixture(records: &[(&str, &str, &str, &str)]) -> TempDir {
    let directory = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(directory.path().into()).unwrap();
    for &(anchor, session, id, content) in records {
        record(&runtime, anchor, session, id, content);
    }
    runtime.flush_for_backup().unwrap();
    runtime.shutdown().unwrap();
    directory
}
fn package(directory: &Path, selected: Option<Vec<String>>) -> std::path::PathBuf {
    let output = directory.join(format!("backup-{:08x}.hbackup", rand::random::<u32>()));
    DataBackup::export_selected(
        directory,
        &output,
        json!({"livescope.replay-offset.11:21":"1","livescope.replay-offset.12:22":"2"}),
        selected,
    )
    .unwrap();
    output
}
fn clip(directory: &Path, id: &str, anchor: &str, session: &str, title: &str, status: &str) {
    let job:crate::clips::ClipJob=serde_json::from_value(json!({"id":id,"anchor_id":anchor,"anchor_name":anchor,"session_id":session,"title":title,"status":status,"phase":status,"progress":0.,"downloaded_bytes":0,"total_bytes":null,"output_path":format!("X:/local/{id}.mp4"),"error":null,"requested_start_seconds":1.,"requested_end_seconds":10.,"actual_start_seconds":null,"actual_end_seconds":null,"attempt":1,"created_at":storage::now(),"updated_at":storage::now()})).unwrap();
    crate::clip_history::ClipHistory::open(directory)
        .unwrap()
        .put(&job)
        .unwrap();
}
fn mark(id: &str, anchor: &str, session: &str) -> Value {
    json!({"id":id,"anchor_id":anchor,"session_id":session,"room_id":"1","anchor_name":anchor,"title":"打点","note":"","marked_at_epoch":1792987200.,"marked_at":"2026-10-26T12:00:00+08:00","created_at":"2026-10-26T12:00:00+08:00","updated_at":"2026-10-26T12:00:00+08:00","broadcast_started_at":null})
}

#[test]
fn selected_backup_is_physically_isolated_and_numeric_room_collisions_do_not_expand_scope() {
    let source = fixture(&[
        ("11", "21", "a", "keep"),
        ("12", "22", "b", &SECRET.repeat(200)),
    ]);
    let mut config = read_json(&source.path().join("config.json"), MAX_JSON).unwrap();
    config["anchors"] = json!({"1":"所选主播","11":"其他主播"});
    config["anchor_metadata"] =
        json!({"1":{"anchor_id":"11","session_id":"21"},"11":{"anchor_id":"12","session_id":"22"}});
    config["anchor_tags"] = json!({"11":["精选主播"],"12":["未选主播私有标签"],"1":["另一稳定身份"],"room:11":["未选房间私有标签"]});
    config["pc_unsupported_rids"] = json!(["1", "11"]);
    write_json(&source.path().join("config.json"), &config).unwrap();
    write_json(&source.path().join("session-marks.json"),&json!({"format":1,"marks":[mark("11111111111111111111111111111111","11","21"),mark("22222222222222222222222222222222","12","22")]})).unwrap();
    write_json(
        &source.path().join("polling-habits.json"),
        &json!({"version":1,"aliases":{"1":"11","11":"12"},"anchors":{"11":{"1":1},"12":{"2":2}}}),
    )
    .unwrap();
    clip(
        source.path(),
        "11111111111111111111111111111111",
        "11",
        "21",
        "keep",
        "completed",
    );
    clip(
        source.path(),
        "22222222222222222222222222222222",
        "12",
        "22",
        SECRET,
        "completed",
    );
    for name in ["messages.sqlite3", "clips.sqlite3"] {
        writable(&source.path().join(name))
            .unwrap()
            .execute_batch("ANALYZE;")
            .unwrap();
    }
    let roster = DataBackup::anchors(source.path()).unwrap();
    assert_eq!(roster["anchors"].as_array().unwrap().len(), 3);
    assert_eq!(roster["anchors"][0]["tags"], json!(["精选主播"]));
    assert!(DataBackup::export_selected(
        source.path(),
        &source.path().join("empty.hbackup"),
        json!({}),
        Some(vec![])
    )
    .is_err());
    let output = package(source.path(), Some(vec!["11".into()]));
    let prepared = DataBackup::prepare(&output, source.path()).unwrap();
    assert_eq!(prepared.summary()["counts"]["anchors"], 1);
    assert_eq!(prepared.summary()["counts"]["messages"], 1);
    assert_eq!(prepared.summary()["counts"]["downloads"], 1);
    for name in ["messages.sqlite3", "clips.sqlite3"] {
        let bytes = fs::read(prepared.directory.path().join(name)).unwrap();
        assert!(
            !bytes.windows(SECRET.len()).any(|b| b == SECRET.as_bytes()),
            "deleted data leaked in {name}"
        );
    }
    let config = read_json(&prepared.directory.path().join("config.json"), MAX_JSON).unwrap();
    assert_eq!(config["anchors"], json!({"1":"所选主播"}));
    assert!(config["anchor_metadata"].get("11").is_none());
    assert_eq!(config["anchor_tags"], json!({"11":["精选主播"]}));
    let preferences = read_json(
        &prepared.directory.path().join("preferences.json"),
        MAX_JSON,
    )
    .unwrap();
    assert!(preferences.get("livescope.replay-offset.12:22").is_none());
    let marks = read_json(
        &prepared.directory.path().join("session-marks.json"),
        MAX_JSON,
    )
    .unwrap();
    assert_eq!(marks["marks"].as_array().unwrap().len(), 1);
    let habits = read_json(
        &prepared.directory.path().join("polling-habits.json"),
        MAX_JSON,
    )
    .unwrap();
    assert!(habits["anchors"].get("12").is_none());
    assert!(habits["aliases"].get("11").is_none());
}

#[test]
fn additive_restore_keeps_local_conflicts_remaps_references_and_is_idempotent() {
    let source = fixture(&[
        ("11", "21", "shared", "source conflict"),
        ("11", "21", "new", "new"),
        ("11", "21", "", "without ID"),
    ]);
    let target = fixture(&[
        ("11", "21", "shared", "local conflict"),
        ("12", "22", "local", "local only"),
    ]);
    let mut source_config = read_json(&source.path().join("config.json"), MAX_JSON).unwrap();
    source_config["anchor_tags"] =
        json!({"11":[" 共同标签 ","来源新增","共同标签"],"13":["来源其它主播"]});
    write_json(&source.path().join("config.json"), &source_config).unwrap();
    let source_db = writable(&source.path().join("messages.sqlite3")).unwrap();
    source_db.execute_batch("INSERT INTO session_monitor_durations VALUES('11','21',60,'2026-10-26T12:00:00+08:00'); INSERT INTO activity_rounds SELECT id,anchor_id,session_id,'lottery',1792987300,0,1 FROM messages WHERE message_id='new'; INSERT INTO activity_aliases SELECT anchor_id,session_id,'lottery','activity-1',id FROM messages WHERE message_id='new'; INSERT INTO quality_manual VALUES('11','21','rule','rule');").unwrap();
    drop(source_db);
    let target_db = writable(&target.path().join("messages.sqlite3")).unwrap();
    target_db.execute_batch("INSERT INTO session_monitor_durations VALUES('11','21',120,'2026-10-26T12:01:00+08:00');").unwrap();
    drop(target_db);
    let mut config = read_json(&target.path().join("config.json"), MAX_JSON).unwrap();
    config["anchors"] = json!({"1":"本地名字","11":"本地其它主播"});
    config["download_naming"] = json!({"root_directory":"X:/keep","directory_template":"keep","filename_template":"keep.mp4"});
    config["highlight_api_key_encrypted"] = json!("keep-key");
    config["startup_monitoring"] = json!(false);
    config["anchor_tags"] = json!({"11":["本地标签","共同标签"],"12":["本地其它主播"]});
    write_json(&target.path().join("config.json"), &config).unwrap();
    let same = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    clip(
        source.path(),
        same,
        "11",
        "21",
        "incoming conflict",
        "running",
    );
    clip(target.path(), same, "11", "21", "local clip", "completed");
    clip(
        source.path(),
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "11",
        "21",
        "incoming task",
        "running",
    );
    write_json(
        &target.path().join("session-marks.json"),
        &json!({"format":1,"marks":[mark("11111111111111111111111111111111","11","21")]}),
    )
    .unwrap();
    write_json(
        &source.path().join("session-marks.json"),
        &json!({"format":1,"marks":[mark("22222222222222222222222222222222","11","21")]}),
    )
    .unwrap();
    let output = package(source.path(), None);
    let first = DataBackup::prepare(&output, source.path())
        .unwrap()
        .restore(target.path())
        .unwrap();
    assert_eq!(first["added"]["messages"], 2);
    assert_eq!(first["added"]["downloads"], 1);
    assert_eq!(first["added"]["marks"], 0);
    let second = DataBackup::prepare(&output, source.path())
        .unwrap()
        .restore(target.path())
        .unwrap();
    assert_eq!(second["added"]["messages"], 0);
    assert_eq!(second["added"]["downloads"], 0);
    let conn = readonly(&target.path().join("messages.sqlite3")).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        conn.query_row(
            "SELECT content FROM messages WHERE message_id='shared'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "local conflict"
    );
    assert_eq!(
        conn.query_row(
            "SELECT accumulated_seconds FROM session_monitor_durations",
            [],
            |r| r.get::<_, f64>(0)
        )
        .unwrap(),
        120.
    );
    assert_eq!(
        conn.query_row(
            "SELECT m.message_id FROM activity_aliases a JOIN messages m ON m.id=a.message_row_id",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "new"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    drop(conn);
    let config_after = read_json(&target.path().join("config.json"), MAX_JSON).unwrap();
    assert_eq!(
        config_after["anchor_tags"],
        json!({"11":["本地标签","共同标签","来源新增"],"12":["本地其它主播"],"13":["来源其它主播"]})
    );
    for key in [
        "anchors",
        "download_naming",
        "highlight_api_key_encrypted",
        "startup_monitoring",
    ] {
        assert_eq!(config_after[key], config[key]);
    }
    let history = crate::clip_history::ClipHistory::open(target.path()).unwrap();
    let local = history.get(same).unwrap().unwrap();
    assert_eq!(local.title, "local clip");
    assert_eq!(local.output_path, format!("X:/local/{same}.mp4"));
    let incoming = history
        .get("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        .unwrap()
        .unwrap();
    assert_eq!(incoming.status, "failed");
    assert!(incoming
        .output_path
        .starts_with(target.path().to_str().unwrap()));
}

fn activity(directory: &Path, id: &str, kind: &str, round: &str, draw: Option<i64>) {
    let conn = writable(&directory.join("messages.sqlite3")).unwrap();
    let payload =
        json!({"activity_kind":kind,"activity_id":round,"activity_details":{"draw_at":draw}})
            .to_string();
    conn.execute("INSERT INTO messages(anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,content,message_time,received_at,time_source,message_id,message_time_epoch,received_at_epoch,payload_json) VALUES('11','A','1','21','','','activity','activity','2026-10-26T12:00:00+08:00','2026-10-26T12:00:01+08:00','server',?1,1792987200,1792987201,?2)",rusqlite::params![id,payload]).unwrap();
    let row = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO activity_aliases VALUES('11','21',?1,?2,?3)",
        rusqlite::params![kind, round, row],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO activity_rounds VALUES(?1,'11','21',?2,?3,0,1)",
        rusqlite::params![row, kind, draw],
    )
    .unwrap();
}

#[test]
fn cross_machine_activity_alias_and_draw_identities_preserve_one_round_with_or_without_message_id()
{
    for kind in ["lottery", "red_packet"] {
        for source_id in ["source-notification", ""] {
            let source = fixture(&[("11", "21", "base", "base")]);
            let target = fixture(&[("11", "21", "base", "base")]);
            let draw = (kind == "lottery").then_some(1792987300);
            activity(source.path(), source_id, kind, "round-1", draw);
            activity(
                target.path(),
                "local-notification",
                kind,
                if kind == "lottery" {
                    "different-alias"
                } else {
                    "round-1"
                },
                draw,
            );
            let source_db = writable(&source.path().join("messages.sqlite3")).unwrap();
            source_db.execute("INSERT INTO activity_aliases SELECT anchor_id,session_id,kind,'extra-alias',message_row_id FROM activity_aliases",[]).unwrap();
            drop(source_db);
            let output = package(source.path(), None);
            for _ in 0..2 {
                let result = DataBackup::prepare(&output, source.path())
                    .unwrap()
                    .restore(target.path())
                    .unwrap();
                assert_eq!(result["added"]["messages"], 0, "{kind} {source_id}");
            }
            let conn = readonly(&target.path().join("messages.sqlite3")).unwrap();
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(*) FROM messages WHERE message_type='activity'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
            assert_eq!(
                conn.query_row("SELECT COUNT(*) FROM activity_rounds", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(DISTINCT message_row_id) FROM activity_aliases",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(*) FROM activity_aliases WHERE activity_id='extra-alias'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
            assert_eq!(
                conn.query_row(
                    "SELECT message_id FROM messages WHERE message_type='activity'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "local-notification"
            );
        }
    }
}

#[test]
fn missing_server_ids_use_multiset_ordinals_without_losing_legitimate_duplicates() {
    let source = fixture(&[
        ("11", "21", "", "same"),
        ("11", "21", "", "same"),
        ("11", "21", "", "same"),
    ]);
    let target = fixture(&[("11", "21", "", "same")]);
    let output = package(source.path(), None);
    for expected_added in [2, 0] {
        let result = DataBackup::prepare(&output, source.path())
            .unwrap()
            .restore(target.path())
            .unwrap();
        assert_eq!(result["added"]["messages"], expected_added);
        let conn = readonly(&target.path().join("messages.sqlite3")).unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
    }
}

#[test]
fn new_sessions_preserve_analysis_with_rebound_message_references() {
    let source = fixture(&[("11", "21", "new", "new")]);
    let target = fixture(&[("12", "22", "local", "local")]);
    let conn = writable(&source.path().join("messages.sqlite3")).unwrap();
    conn.execute_batch("INSERT OR IGNORE INTO quality_registry(anchor_id,session_id,anchor_name,web_rid,title,first_seen,last_seen,started_at) SELECT anchor_id,session_id,anchor_name,web_rid,title,first_seen,last_seen,started_at FROM sessions; DELETE FROM quality_results; DELETE FROM quality_stats; DELETE FROM quality_users; INSERT INTO quality_results SELECT id,1,anchor_id,session_id,'31','new',1792987200,1,'normal' FROM messages; INSERT INTO quality_stats VALUES('11','21',1,1,1,0,0); INSERT INTO quality_users VALUES('11','21',1,'31',1,1);").unwrap();
    drop(conn);
    let output = package(source.path(), None);
    DataBackup::prepare(&output, source.path())
        .unwrap()
        .restore(target.path())
        .unwrap();
    let conn = readonly(&target.path().join("messages.sqlite3")).unwrap();
    let id: i64 = conn
        .query_row("SELECT id FROM messages WHERE message_id='new'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_ne!(id, 1);
    assert_eq!(
        conn.query_row(
            "SELECT message_id FROM quality_results WHERE anchor_id='11'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        id
    );
    assert_eq!(
        conn.query_row(
            "SELECT analyzed FROM quality_stats WHERE anchor_id='11'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn ambiguous_historic_room_aliases_do_not_export_current_unselected_room_configuration() {
    let source = fixture(&[("11", "21", "a", "a"), ("12", "22", "b", "b")]);
    let conn = writable(&source.path().join("messages.sqlite3")).unwrap();
    conn.execute_batch("UPDATE anchors SET web_rid='1'; UPDATE quality_registry SET web_rid='1';")
        .unwrap();
    drop(conn);
    let mut config = read_json(&source.path().join("config.json"), MAX_JSON).unwrap();
    config["anchors"] = json!({"1":"当前房间B"});
    config["anchor_metadata"] = json!({});
    write_json(&source.path().join("config.json"), &config).unwrap();
    let output = package(source.path(), Some(vec!["11".into()]));
    let prepared = DataBackup::prepare(&output, source.path()).unwrap();
    let config = read_json(&prepared.directory.path().join("config.json"), MAX_JSON).unwrap();
    assert_eq!(config["anchors"], json!({}));
    assert_eq!(prepared.summary()["counts"]["messages"], 1);
}

#[test]
fn tag_backup_config_is_normalized_portable_and_accepts_legacy_imports() {
    let emoji = "😀".repeat(24);
    let raw =
        json!({"anchors":{},"anchor_metadata":{},"anchor_tags":{"11":[" 精选 ","精选",emoji]}});
    let portable = portable_config(&raw).unwrap();
    assert_eq!(portable["anchor_tags"]["11"], json!(["精选", emoji]));
    let mut legacy = portable.clone();
    legacy.as_object_mut().unwrap().remove("anchor_tags");
    validate_config(&legacy).unwrap();
    let local = json!({"anchors":{},"anchor_metadata":{},"anchor_tags":{"11":["本地"]}});
    let merged = merge_config(&legacy, &local, Path::new("C:/new-data")).unwrap();
    assert_eq!(merged["anchor_tags"], local["anchor_tags"]);
    for invalid in [
        json!("标签"),
        json!([null]),
        json!([""]),
        json!(["\n"]),
        json!(["a\nb"]),
        json!(["x".repeat(25)]),
        json!(vec!["标签"; 201]),
    ] {
        let mut raw = raw.clone();
        raw["anchor_tags"]["11"] = invalid;
        assert!(portable_config(&raw).is_err());
    }
}

#[test]
fn merging_tags_preserves_local_order_capacity_and_room_ownership() {
    let existing: Vec<_> = (0..20).map(|index| format!("本地{index}")).collect();
    let mut local = json!({"anchors":{"1":"本地A","2":"本地B"},"anchor_metadata":{"1":{"anchor_id":"11"},"2":{"anchor_id":"12"}},"anchor_tags":{"11":existing,"room:1":["原房间"],"room:2":["本地房间"]}});
    let imported = portable_config(&json!({"anchors":{"1":"别的主播","2":"未知身份","3":"新主播"},"anchor_metadata":{"1":{"anchor_id":"99"},"3":{"anchor_id":"13"}},"anchor_tags":{"11":["超出上限不添加"],"12":["新增标签"],"room:1":["不兼容房间标签"],"room:2":["未知来源房间标签"],"room:3":["新房间"]}})).unwrap();
    config(&mut local, &imported).unwrap();
    assert_eq!(local["anchor_tags"]["11"], json!(existing));
    assert_eq!(local["anchor_tags"]["12"], json!(["新增标签"]));
    assert_eq!(local["anchor_tags"]["room:1"], json!(["原房间"]));
    assert_eq!(local["anchor_tags"]["room:2"], json!(["本地房间"]));
    assert_eq!(local["anchor_tags"]["room:3"], json!(["新房间"]));
    let first = local.clone();
    config(&mut local, &imported).unwrap();
    assert_eq!(local, first);
}

#[test]
fn unresolved_room_tag_roster_and_partial_export_never_match_numeric_stable_ids() {
    let source = fixture(&[]);
    let mut config = read_json(&source.path().join("config.json"), MAX_JSON).unwrap();
    config["anchors"] = json!({"1":"待确认主播"});
    config["anchor_metadata"] = json!({});
    config["anchor_tags"] = json!({"room:1":["待确认"],"1":["另一个人"]});
    write_json(&source.path().join("config.json"), &config).unwrap();
    let roster = DataBackup::anchors(source.path()).unwrap();
    assert_eq!(roster["anchors"][0]["anchor_id"], "room:1");
    assert_eq!(roster["anchors"][0]["tags"], json!(["待确认"]));
    let output = package(source.path(), Some(vec!["room:1".into()]));
    let prepared = DataBackup::prepare(&output, source.path()).unwrap();
    let config = read_json(&prepared.directory.path().join("config.json"), MAX_JSON).unwrap();
    assert_eq!(config["anchor_tags"], json!({"room:1":["待确认"]}));
}

#[test]
fn pending_room_tags_remain_an_explicit_scope_after_a_remembered_identity_is_restored() {
    for pending in [json!(["等待网络确认"]), json!([])] {
        let source = fixture(&[("11", "21", "a", "旧主播历史")]);
        let mut config = read_json(&source.path().join("config.json"), MAX_JSON).unwrap();
        config["anchors"] = json!({"1":"重新添加的房间"});
        config["anchor_metadata"] = json!({"1":{"anchor_id":"11","session_id":"21"}});
        config["anchor_tags"] = json!({"11":["已确认旧主播"],"room:1":pending});
        write_json(&source.path().join("config.json"), &config).unwrap();
        let roster = DataBackup::anchors(source.path()).unwrap();
        let rows = roster["anchors"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["anchor_id"], "11");
        assert_eq!(rows[0]["tags"], json!(["已确认旧主播"]));
        assert_eq!(rows[1]["anchor_id"], "room:1");
        assert_eq!(rows[1]["tags"], pending);

        let historical = package(source.path(), Some(vec!["11".into()]));
        let historical = DataBackup::prepare(&historical, source.path()).unwrap();
        let config = read_json(&historical.directory.path().join("config.json"), MAX_JSON).unwrap();
        assert_eq!(config["anchor_tags"], json!({"11":["已确认旧主播"]}));
        assert_eq!(config["anchors"], json!({}));
        assert_eq!(config["anchor_metadata"], json!({}));
        assert_eq!(historical.summary()["counts"]["messages"], 1);

        let room = package(source.path(), Some(vec!["room:1".into()]));
        let room = DataBackup::prepare(&room, source.path()).unwrap();
        let config = read_json(&room.directory.path().join("config.json"), MAX_JSON).unwrap();
        assert_eq!(config["anchor_tags"], json!({"room:1":pending}));
        assert_eq!(config["anchors"], json!({"1":"重新添加的房间"}));
        assert_eq!(config["anchor_metadata"], json!({}));
        assert_eq!(room.summary()["counts"]["messages"], 0);
    }
}

#[test]
fn opaque_historic_anchor_tags_survive_partial_backup_and_incremental_restore() {
    let source = fixture(&[("11", "21", "a", "历史导入数据")]);
    let identity = "legacy.anchor-11";
    let conn = writable(&source.path().join("messages.sqlite3")).unwrap();
    conn.execute_batch("BEGIN; PRAGMA defer_foreign_keys=ON;").unwrap();
    for table in &expected_shape(true).unwrap().tables {
        if table.columns.iter().any(|column| column.0 == "anchor_id") {
            conn.execute(
                &format!(
                    "UPDATE {} SET anchor_id=?1 WHERE anchor_id='11'",
                    table.name
                ),
                [identity],
            )
            .unwrap();
        }
    }
    conn.execute_batch("COMMIT;").unwrap();
    drop(conn);
    let mut config = read_json(&source.path().join("config.json"), MAX_JSON).unwrap();
    config["anchor_tags"] = json!({});
    config["anchor_tags"][identity] = json!(["旧档案标签"]);
    write_json(&source.path().join("config.json"), &config).unwrap();
    let rows = DataBackup::anchors(source.path()).unwrap();
    assert_eq!(rows["anchors"][0]["anchor_id"], identity);
    assert_eq!(rows["anchors"][0]["tags"], json!(["旧档案标签"]));
    let output = package(source.path(), Some(vec![identity.into()]));
    let target = fixture(&[]);
    DataBackup::prepare(&output, source.path())
        .unwrap()
        .restore(target.path())
        .unwrap();
    let config = read_json(&target.path().join("config.json"), MAX_JSON).unwrap();
    assert_eq!(config["anchor_tags"][identity], json!(["旧档案标签"]));
    assert!(storage::Settings::load(target.path().into()).is_ok());
}

#[test]
fn reliable_identity_mapping_uses_the_covering_partial_unique_index() {
    let directory = fixture(&[("11", "21", "a", "a")]);
    let conn = writable(&directory.path().join("messages.sqlite3")).unwrap();
    conn.execute(
        "ATTACH DATABASE ?1 AS incoming",
        [directory
            .path()
            .join("messages.sqlite3")
            .to_string_lossy()
            .as_ref()],
    )
    .unwrap();
    let plan=conn.prepare("EXPLAIN QUERY PLAN SELECT i.id,m.id FROM incoming.messages i JOIN main.messages m ON m.anchor_id=i.anchor_id AND m.session_id=i.session_id AND m.message_id=i.message_id WHERE i.message_id<>'' AND m.message_id<>''").unwrap().query_map([],|r|r.get::<_,String>(3)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap().join("\n");
    assert!(plan.contains("messages_server_identity"), "{plan}");
    assert!(
        !plan.contains("SEARCH m USING INDEX messages_quality_session"),
        "{plan}"
    );
}
