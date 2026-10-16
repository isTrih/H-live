//! Upgrade a synthetic predecessor fixture; never points at real user data.
use h_live_runtime::Runtime;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
fn rows(path: &std::path::Path) -> Result<Value, String> {
    let db =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| "open failed")?;
    let mut result = json!({});
    for table in [
        "anchors",
        "sessions",
        "messages",
        "session_monitor_durations",
        "audience_minutes",
        "sqlite_sequence",
    ] {
        let mut statement = db
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1,2"))
            .map_err(|_| "query failed")?;
        let names = statement
            .column_names()
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let mut cursor = statement.query([]).map_err(|_| "query failed")?;
        let mut out = vec![];
        while let Some(row) = cursor.next().map_err(|_| "read failed")? {
            let mut value = json!({});
            for (i, name) in names.iter().enumerate() {
                value[name] = match row.get_ref(i).map_err(|_| "column failed")? {
                    rusqlite::types::ValueRef::Null => Value::Null,
                    rusqlite::types::ValueRef::Integer(n) => json!(n),
                    rusqlite::types::ValueRef::Real(n) => json!(n),
                    rusqlite::types::ValueRef::Text(s) => {
                        json!(std::str::from_utf8(s).map_err(|_| "utf8 failed")?)
                    }
                    _ => return Err("unexpected blob".into()),
                };
            }
            out.push(value);
        }
        result[table] = json!(out);
    }
    Ok(result)
}
fn main() -> Result<(), String> {
    let path = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("upgrade_contract SYNTHETIC_FIXTURE_DIR")?,
    )
    .canonicalize()
    .map_err(|_| "fixture missing")?;
    if !path.join("before.json").is_file() {
        return Err("require explicit synthetic before.json fixture".into());
    }
    let expected: Value = serde_json::from_slice(
        &std::fs::read(path.join("before.json")).map_err(|_| "read failed")?,
    )
    .map_err(|_| "fixture invalid")?;
    let runtime = Runtime::open(path.clone())?;
    let init = runtime.request("init", json!({}))?;
    if init["anchors"].as_array().map(|v| v.len()) != Some(1) {
        return Err("anchors not preserved".into());
    }
    let proxies = runtime.request("list_imported_proxies", json!({}))?;
    if proxies["items"][0]["auth_present"] != true
        || proxies.to_string().contains("synthetic-proxy-pass")
    {
        return Err("proxy migration/redaction failed".into());
    }
    let gift = runtime.request("recent_messages", json!({"type":"gift"}))?;
    if gift["messages"].as_array().map(|a| a.len()) != Some(1) {
        return Err("legacy gift unavailable".into());
    }
    for format in ["csv", "xlsx"] {
        let job=runtime.request("export",json!({"anchor_ids":["123"],"month":"2026-10","format":format,"output_path":path.join(format!("migration.{format}"))}))?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let p = runtime.request("poll", json!({}))?;
            let j = p["export_jobs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["id"] == job["id"])
                .unwrap();
            if j["status"] == "completed" {
                break;
            }
            if j["status"] != "running" || Instant::now() > deadline {
                return Err("legacy export failed".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    runtime.request("rename", json!({"rid":"111","name":"迁移合成主播"}))?;
    runtime.shutdown()?;
    let current = rows(&path.join("messages.sqlite3"))?;
    if current != expected {
        return Err("historical table values changed".into());
    }
    let raw = std::fs::read_to_string(path.join("config.json")).map_err(|_| "config missing")?;
    if raw.contains("synthetic-proxy-pass") || raw.contains("synthetic-login-never-used") {
        return Err("plaintext secret stored".into());
    }
    let report = json!({"ok":true,"from":"0.2.4","to":h_live_runtime::VERSION,"exact_historical_tables_unchanged":["anchors","sessions","messages","session_monitor_durations","audience_minutes","sqlite_sequence"],"dpapi_proxy_config_readable_and_reprotected":true,"legacy_login_not_used":true,"legacy_gift_readable_and_exportable":true,"csv_and_xlsx_exports":true,"all_workers_stopped":true,"synthetic_fixture_only":true});
    h_live_runtime::storage::atomic(
        &path.join("report.json"),
        &serde_json::to_vec_pretty(&report).unwrap(),
    )?;
    println!("{report}");
    Ok(())
}
