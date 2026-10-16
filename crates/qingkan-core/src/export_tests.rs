use super::*;
use crate::archive::{ArchiveOptions, MessageRecord};
use serde_json::json;
use std::{
    io::Read,
    thread,
    time::{Duration, Instant},
};
struct Fixture {
    dir: tempfile::TempDir,
    store: ArchiveStore,
    serial: u64,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = ArchiveStore::open(dir.path(), ArchiveOptions::default()).unwrap();
        Self {
            dir,
            store,
            serial: 0,
        }
    }
    fn add(&mut self, id: &str, name: &str, content: &str, month: &str) {
        self.serial += 1;
        let r:MessageRecord=serde_json::from_value(json!({"anchor_id":id,"anchor_name":name,"web_rid":"111","session_id":"987654321098765432109","sender_id":"00123456789012345678901234567890","sender_name":"观众😀","message_type":"chat","content":content,"message_time":format!("{month}-15T10:11:12.123+08:00"),"received_at":"2027-01-15T10:11:13+08:00","time_source":"server","message_id":self.serial.to_string()})).unwrap();
        self.store.record_message(r, String::new(), None).unwrap();
    }
    fn options(&self, ids: &[&str], format: &str) -> ExportOptions {
        ExportOptions {
            anchor_ids: ids.iter().map(|s| s.to_string()).collect(),
            month: "2027-01".into(),
            output_path: self.dir.path().join(format!("out.{format}")),
            format: format.into(),
            types: vec!["chat".into()],
        }
    }
    fn run(&self, ids: &[&str], format: &str) -> ExportReport {
        export_month(
            self.dir.path(),
            self.options(ids, format),
            &CancelToken::default(),
        )
        .unwrap()
    }
    fn part(&self, name: &str) -> String {
        let mut zip =
            zip::ZipArchive::new(File::open(self.dir.path().join("out.xlsx")).unwrap()).unwrap();
        let mut text = String::new();
        zip.by_name(name)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        text
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.store.close(Duration::from_secs(2));
    }
}
#[test]
fn newer_unicode_case_pairs_receive_distinct_sheet_names() {
    let mut f = Fixture::new();
    f.add("1", "\u{1c90}", "one", "2027-01");
    f.add("2", "\u{10d0}", "two", "2027-01");
    f.run(&["1", "2"], "xlsx");
    let book = f.part("xl/workbook.xml");
    assert!(book.contains("name=\"ა (2)\""));
}
#[test]
fn seven_columns_long_ids_names_and_no_formulas() {
    let mut f = Fixture::new();
    f.add("1", "主播甲", "=formula & <中文> 😀\r\nnext", "2027-01");
    let report = f.run(&["1"], "xlsx");
    assert_eq!(report.row_count, 1);
    let sheet = f.part("xl/worksheets/sheet1.xml");
    assert_eq!(sheet.matches("t=\"inlineStr\"").count(), 14);
    assert!(!sheet.contains("<f>"));
    assert!(sheet.contains("00123456789012345678901234567890"));
    assert!(sheet.contains("观众😀"));
    assert!(sheet.contains("发消息人名字"));
    assert!(sheet.contains("A1:G2"));
    assert!(sheet.contains("&amp; &lt;中文&gt; 😀&#13;\nnext"));
    assert!(sheet.contains("2027-01-15 10:11:12.123000+08:00"));
}
#[test]
fn sheet_groups_have_selected_order_and_current_name_but_historical_rows() {
    let mut f = Fixture::new();
    f.add("1", "旧甲", "a", "2027-01");
    f.add("2", "乙", "b", "2027-01");
    f.add("1", "新甲", "later", "2027-02");
    let report = f.run(&["2", "1", "3"], "xlsx");
    assert_eq!((report.row_count, report.sheet_count), (2, 3));
    let book = f.part("xl/workbook.xml");
    assert!(book.find("name=\"乙\"").unwrap() < book.find("name=\"新甲\"").unwrap());
    assert!(book.contains("name=\"主播 3\""));
    assert!(f.part("xl/worksheets/sheet2.xml").contains("旧甲"));
    assert!(!f.part("xl/worksheets/sheet2.xml").contains("later"));
    assert_eq!(
        f.part("xl/worksheets/sheet3.xml").matches("<row ").count(),
        1
    );
}
#[test]
fn identical_casefold_names_and_utf16_constraints() {
    let mut f = Fixture::new();
    for (id, name) in [
        ("1", "Straße"),
        ("2", "STRASSE"),
        ("3", "History"),
        ("4", ""),
        ("5", "😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀"),
        ("6", "a/\\:*?[]"),
        ("7", "名字\"&<tag>_x0041_"),
    ] {
        f.add(id, name, "unused", "2026-12");
    }
    let report = f.run(&["1", "2", "3", "4", "5", "6", "7"], "xlsx");
    assert_eq!((report.sheet_count, report.row_count), (7, 0));
    let book = f.part("xl/workbook.xml");
    for name in [
        "Straße",
        "STRASSE (2)",
        "History (2)",
        "主播 4",
        "a_______",
        "名字&quot;&amp;&lt;tag&gt;_x005F_x0041_",
    ] {
        assert!(book.contains(&format!("name=\"{name}\"")), "{book}");
    }
    assert!(book.contains(&format!("name=\"{}\"", "😀".repeat(15))));
}
#[test]
fn overflow_does_not_mix_ids_or_steal_reserved_names() {
    let mut f = Fixture::new();
    for n in 0..5 {
        f.add("1", "主播", &format!("a-{n}"), "2027-01");
    }
    for n in 0..3 {
        f.add("2", "主播 (2)", &format!("b-{n}"), "2027-01");
    }
    let report = export_snapshot(
        &f.store.snapshot().unwrap(),
        f.options(&["1", "2", "3"], "xlsx"),
        &CancelToken::default(),
        3,
    )
    .unwrap();
    assert_eq!((report.sheet_count, report.row_count), (6, 8));
    let book = f.part("xl/workbook.xml");
    assert!(book.contains("name=\"主播 (2)\" sheetId=\"4\""));
    for n in 1..=3 {
        let s = f.part(&format!("xl/worksheets/sheet{n}.xml"));
        assert!(s.contains("a-"));
        assert!(!s.contains("b-"));
        assert!(s.matches("<row ").count() <= 3);
    }
    assert_eq!(
        f.part("xl/worksheets/sheet6.xml").matches("<row ").count(),
        1
    );
}
#[test]
fn snapshot_pins_metadata_and_all_anchor_rows() {
    let mut f = Fixture::new();
    f.add("1", "甲", "before-A", "2027-01");
    f.add("2", "乙", "before-B", "2027-01");
    let snapshot = f.store.snapshot().unwrap();
    f.add("2", "改名", "after-B", "2027-01");
    let report = export_snapshot(
        &snapshot,
        f.options(&["1", "2"], "xlsx"),
        &CancelToken::default(),
        EXCEL_MAX_ROWS,
    )
    .unwrap();
    assert_eq!(report.row_count, 2);
    assert!(f.part("xl/workbook.xml").contains("name=\"乙\""));
    assert!(!f.part("xl/worksheets/sheet2.xml").contains("after-B"));
}
#[test]
fn csv_bom_formula_defense_and_exact_quoted_multiline_content() {
    let mut f = Fixture::new();
    for value in [
        "=1",
        " +SUM(1)",
        "\u{feff}\t @SUM(1)",
        "\tword",
        "safe,\"quoted\"\r\ntext",
    ] {
        f.add("1", "主播", value, "2027-01");
    }
    let report = f.run(&["1"], "csv");
    assert_eq!(report.formula_protected_count, 4);
    let bytes = std::fs::read(f.dir.path().join("out.csv")).unwrap();
    assert!(bytes.starts_with(&[0xef, 0xbb, 0xbf]));
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("' +SUM(1)"));
    assert!(text.contains("\"safe,\"\"quoted\"\"\r\ntext\""));
    assert!(text.contains("00123456789012345678901234567890"));
}
#[test]
fn invalid_xml_and_long_cells_preserve_destination_and_cleanup() {
    for content in [
        "before\u{1}after".into(),
        "字".repeat(32768),
        "😀".repeat(16384),
    ] {
        let mut f = Fixture::new();
        f.add("1", "主播", &content, "2027-01");
        let options = f.options(&["1"], "xlsx");
        std::fs::write(&options.output_path, b"old").unwrap();
        assert!(matches!(
            export_month(f.dir.path(), options.clone(), &CancelToken::default()),
            Err(ExportError::Data(_))
        ));
        assert_eq!(std::fs::read(&options.output_path).unwrap(), b"old");
        assert!(!std::fs::read_dir(f.dir.path()).unwrap().any(|p| p
            .unwrap()
            .path()
            .extension()
            .is_some_and(|e| e == "tmp")));
        assert_eq!(f.run(&["1"], "csv").row_count, 1);
    }
}
#[test]
fn literal_ooxml_tokens_are_escaped_once_in_cells_and_names() {
    let mut f = Fixture::new();
    f.add("1", "_x0041_", "_x0041_ _x005F_", "2027-01");
    f.run(&["1"], "xlsx");
    assert!(f.part("xl/workbook.xml").contains("_x005F_x0041_"));
    assert!(f
        .part("xl/worksheets/sheet1.xml")
        .contains("_x005F_x0041_ _x005F_x005F_"));
}
#[test]
fn missing_time_and_sender_leave_blank_cells_and_month_fallback() {
    let f = Fixture::new();
    let row:MessageRecord=serde_json::from_value(json!({"anchor_id":"1","anchor_name":"主播","session_id":"2","sender_id":"","sender_name":"","message_type":"chat","content":"missing","message_time":"","received_at":"2026-12-31T16:00:00+00:00","time_source":"missing","message_id":"1"})).unwrap();
    f.store.record_message(row, String::new(), None).unwrap();
    let report = f.run(&["1"], "csv");
    assert_eq!(
        (
            report.row_count,
            report.missing_time_count,
            report.missing_sender_count
        ),
        (1, 1, 1)
    );
    let text = std::fs::read_to_string(f.dir.path().join("out.csv")).unwrap();
    assert!(text.contains("主播,2,,,弹幕,missing,\r\n"));
}
#[test]
fn cancel_before_export_and_failed_publish_preserve_existing_target() {
    let f = Fixture::new();
    let options = f.options(&["1"], "xlsx");
    std::fs::write(&options.output_path, b"old").unwrap();
    let token = CancelToken::default();
    token.cancel();
    assert!(matches!(
        export_month(f.dir.path(), options.clone(), &token),
        Err(ExportError::Cancelled)
    ));
    assert_eq!(std::fs::read(&options.output_path).unwrap(), b"old");
    std::fs::remove_file(&options.output_path).unwrap();
    std::fs::create_dir(&options.output_path).unwrap();
    assert!(export_month(f.dir.path(), options.clone(), &CancelToken::default()).is_err());
    assert!(options.output_path.is_dir());
    assert!(!std::fs::read_dir(f.dir.path()).unwrap().any(|p| p
        .unwrap()
        .path()
        .extension()
        .is_some_and(|e| e == "tmp")));
}
#[test]
fn cancellation_during_streaming_preserves_file_and_returns_promptly() {
    let mut f = Fixture::new();
    f.add("1", "主播", &"abcdefgh".repeat(1000), "2027-01");
    let connection = rusqlite::Connection::open(f.store.path()).unwrap();
    connection.execute_batch("INSERT INTO messages(anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,content,message_time,received_at,time_source,message_id,message_time_epoch,received_at_epoch,payload_json) SELECT anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,content,message_time,received_at,time_source,'',message_time_epoch,received_at_epoch,payload_json FROM messages CROSS JOIN (WITH RECURSIVE nums(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM nums WHERE n<5000) SELECT n FROM nums)").unwrap();
    drop(connection);
    let options = f.options(&["1"], "xlsx");
    std::fs::write(&options.output_path, b"old").unwrap();
    let token = CancelToken::default();
    let child_token = token.clone();
    let path = f.dir.path().to_path_buf();
    let child_options = options.clone();
    let handle = thread::spawn(move || export_month(&path, child_options, &child_token));
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if std::fs::read_dir(f.dir.path())
            .unwrap()
            .any(|p| p.unwrap().path().extension().is_some_and(|e| e == "tmp"))
        {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    token.cancel();
    assert!(matches!(
        handle.join().unwrap(),
        Err(ExportError::Cancelled)
    ));
    assert_eq!(std::fs::read(&options.output_path).unwrap(), b"old");
}
