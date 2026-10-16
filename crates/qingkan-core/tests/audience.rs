use qingkan_core::{archive::*, export::CancelToken};
use serde_json::{json, Value};
fn rpc(s: &ArchiveStore, action: &str, p: Value) -> Value {
    s.quality_request(action, p, CancelToken::default())
        .unwrap()
}
fn sample(s: &ArchiveStore, count: i64, time: &str) {
    rpc(
        s,
        "audience_record",
        json!({"anchor_id":"11","session_id":"101","viewer_count":count,"observed_at":time}),
    );
}
fn overview(s: &ArchiveStore) -> Value {
    rpc(s, "detail", json!({"anchor_id":"11","session_id":"101"}))["overview"].clone()
}
#[test]
fn minute_buckets_do_not_weight_frequent_events_or_fill_monitoring_gaps() {
    let d = tempfile::tempdir().unwrap();
    let s = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    rpc(
        &s,
        "register",
        json!({"anchor_id":"11","session_id":"101","web_rid":"111"}),
    );
    assert!(overview(&s)["peak_viewers"].is_null());
    sample(&s, 400, "2026-10-16T12:00:05+08:00");
    sample(&s, 200, "2026-10-16T12:00:50+08:00");
    sample(&s, 100, "2026-10-16T12:00:01+08:00"); // older packet cannot replace minute's latest count
    sample(&s, 0, "2026-10-16T12:01:20+08:00");
    sample(&s, 100, "2026-10-16T12:30:20+08:00");
    let row = overview(&s);
    assert_eq!(row["peak_viewers"], 400);
    assert!((row["minute_avg_viewers"].as_f64().unwrap() - 400. / 3.).abs() < 0.0001);
    assert_eq!(row["audience_sample_minutes"], 3);
    assert_eq!(row["speakers_to_online_rate"], 0.);
    assert!(s.close(std::time::Duration::from_secs(2)));
    let s = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    assert_eq!(overview(&s)["peak_viewers"], 400);
    rpc(&s, "register", json!({"anchor_id":"11","session_id":"102"}));
    assert!(
        rpc(&s, "detail", json!({"anchor_id":"11","session_id":"102"}))["overview"]["peak_viewers"]
            .is_null()
    );
}
#[test]
fn invalid_values_unknown_sessions_and_zero_denominators_remain_unknown() {
    let d = tempfile::tempdir().unwrap();
    let s = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    let base = json!({"anchor_id":"11","session_id":"101","viewer_count":1,"observed_at":"2026-10-16T12:00:00+08:00"});
    assert!(s
        .quality_request("audience_record", base.clone(), CancelToken::default())
        .is_err());
    rpc(&s, "register", json!({"anchor_id":"11","session_id":"101"}));
    for bad in [
        json!(true),
        json!(-1),
        json!(1.2),
        json!(9_007_199_254_740_992i64),
    ] {
        let mut p = base.clone();
        p["viewer_count"] = bad;
        assert!(s
            .quality_request("audience_record", p, CancelToken::default())
            .is_err());
    }
    sample(&s, 0, "2026-10-16T12:00:00+08:00");
    assert!(overview(&s)["speakers_to_online_rate"].is_null());
    assert_eq!(overview(&s)["peak_viewers"], 0);
}

#[test]
fn speaker_ratio_exports_and_chat_deletion_use_persisted_audience_without_rewriting_messages() {
    let d = tempfile::tempdir().unwrap();
    let s = ArchiveStore::open(d.path(), ArchiveOptions::default()).unwrap();
    rpc(&s, "register", json!({"anchor_id":"11","session_id":"101"}));
    sample(&s, 1, "2026-10-16T12:00:00+08:00");
    for id in ["1", "2"] {
        let m=serde_json::from_value(json!({"anchor_id":"11","session_id":"101","web_rid":"111","anchor_name":"主播","sender_id":id,"sender_name":"观众","message_type":"chat","content":"谢谢主播","message_id":id,"message_time":"2026-10-16T12:00:00+08:00","received_at":"2026-10-16T12:00:01+08:00","time_source":"server","payload":{}})).unwrap();
        s.record_message(m, String::new(), None).unwrap();
    }
    for _ in 0..20 {
        if rpc(&s, "step", json!({}))["processed"] == 0 {
            break;
        }
    }
    assert_eq!(overview(&s)["speakers_to_online_rate"], 2.);
    assert_eq!(overview(&s)["valid_speakers_to_online_rate"], 2.);
    let out = d.path().join("analysis.csv");
    qingkan_core::export::export_quality(d.path(), "11", "", &out, "csv", &CancelToken::default())
        .unwrap();
    let text = std::fs::read_to_string(out).unwrap();
    assert!(text.contains("最高在线人数"));
    assert!(text.contains("有效发言占比"));
    assert!(text.contains("200.00%"));
    let xlsx = d.path().join("analysis.xlsx");
    qingkan_core::export::export_quality(
        d.path(),
        "11",
        "",
        &xlsx,
        "xlsx",
        &CancelToken::default(),
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(xlsx).unwrap()).unwrap();
    let mut sheet = String::new();
    std::io::Read::read_to_string(
        &mut zip.by_name("xl/worksheets/sheet1.xml").unwrap(),
        &mut sheet,
    )
    .unwrap();
    assert!(sheet.contains("A1:R2"));
    assert!(sheet.contains("最高在线人数"));
    s.execute_delete(&ArchiveDeleteFilter {
        anchor_ids: vec!["11".into()],
        month: None,
        session_ids: None,
    })
    .unwrap();
    for _ in 0..20 {
        if rpc(&s, "step", json!({}))["processed"] == 0 {
            break;
        }
    }
    let row = overview(&s);
    assert_eq!(row["peak_viewers"], 1);
    assert_eq!(row["chat_messages"], 0);
    assert!(row["speakers_to_online_rate"].is_null());
}
