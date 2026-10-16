//! Deterministic synthetic activity fixtures; no captured room/user data.
use qingkan_core::protocol::{decode_frame, decode_message};
use std::io::Write;
fn var(mut n: u64) -> Vec<u8> {
    let mut v = vec![];
    while n > 127 {
        v.push(n as u8 & 127 | 128);
        n >>= 7;
    }
    v.push(n as u8);
    v
}
fn blob(tag: u64, b: &[u8]) -> Vec<u8> {
    [var(tag << 3 | 2), var(b.len() as u64), b.to_vec()].concat()
}
fn frame(method: &str, payload: &[u8]) -> Vec<u8> {
    let envelope = [blob(1, method.as_bytes()), blob(2, payload)].concat();
    let response = blob(1, &envelope);
    let mut gzip = flate2::write::GzEncoder::new(vec![], flate2::Compression::default());
    gzip.write_all(&response).unwrap();
    blob(8, &gzip.finish().unwrap())
}
#[test]
fn synthetic_lottery_sync_is_an_activity_instead_of_silently_ignored() {
    let raw = include_bytes!("fixtures/lottery_sync_synthetic.bin");
    let event = decode_message("WebcastRoomDataSyncMessage", raw, "100000012000")
        .unwrap()
        .expect("Synthetic lottery sync must not be ignored");
    assert_eq!(event.kind, "activity");
    assert_eq!(event.payload["activity_kind"], "lottery");
    assert_eq!(event.payload["activity_stage"], "sync");
    assert!(!event.payload["activity_id"].as_str().unwrap().is_empty());
    assert_eq!(event.payload["sender_id"], "");
    assert_eq!(event.payload["sender_name"], "");
    assert!(event.payload["activity_details"]["draw_at"].is_number());
    let decoded = decode_frame(
        &frame("WebcastRoomDataSyncMessage", raw),
        "100000012000",
    )
    .unwrap();
    assert_eq!(decoded.events.len(), 1);
}
#[test]
fn unrelated_sync_and_zero_lottery_are_not_fabricated_activities() {
    let unrelated = [blob(3, b"OtherSync"), blob(5, &[8, 1])].concat();
    assert!(
        decode_message("WebcastRoomDataSyncMessage", &unrelated, "111")
            .unwrap()
            .is_none()
    );
    let empty = [blob(3, b"LotteryInfoSyncData"), blob(5, &[])].concat();
    let event = decode_message("WebcastRoomDataSyncMessage", &empty, "111").unwrap();
    assert!(event.is_none() || event.unwrap().payload["activity_stage"] == "cleared");
}
#[test]
fn synthetic_red_packet_preview_is_not_lost_or_mislabeled_as_a_lottery() {
    let raw = include_bytes!("fixtures/red_packet_preview_synthetic.bin");
    let e = decode_message("WebcastPreviewCjRpMessage", raw, "100000012000")
        .unwrap()
        .expect("Synthetic red packet preview must not be ignored");
    assert_eq!(e.payload["activity_kind"], "red_packet");
    assert_eq!(e.payload["activity_stage"], "preview");
    assert_eq!(e.payload["sender_id"], "");
    assert!(e.payload["activity_details"]["currency"].is_string());
    assert!(e.payload["activity_details"]["total_amount_raw"].is_string());
    assert!(e.payload["activity_details"]["icon"].is_null());
}
