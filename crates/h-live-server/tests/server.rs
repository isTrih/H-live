#![cfg(feature = "test-support")]
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use h_live_runtime::testing::Simulation;
use h_live_server::{auth::Auth, control::Control, http, secrets::ServerProtector};
use salvo::{
    prelude::*,
    test::{ResponseExt, TestClient},
};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
fn open(path: &Path) -> Arc<Control> {
    Control::open_with_transport(
        path.into(),
        Arc::new(ServerProtector::from_key([2; 32])),
        Arc::new(Simulation {
            live_rooms: 1,
            ..Default::default()
        }),
    )
    .unwrap()
}
fn service(c: Arc<Control>, ui: &Path) -> Service {
    Service::new(http::router(
        http::App::new(c, Auth::new([1; 32], "http://localhost:8080").unwrap()),
        ui.into(),
    ))
}
async fn post(s: &Service, path: &str, p: Value) -> Response {
    TestClient::post(format!("http://localhost:8080/api/v1/{path}"))
        .bearer_auth(URL_SAFE_NO_PAD.encode([1; 32]))
        .json(&p)
        .send(s)
        .await
}
async fn wait(c: &Control, check: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let p = c.request("poll", json!({})).await.unwrap();
        if check(&p) {
            return p;
        }
        assert!(Instant::now() < deadline, "{}", p);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn cookie_auth_requires_origin_csrf_and_revocation() {
    let d = tempfile::tempdir().unwrap();
    let c = open(d.path());
    let ui = tempfile::tempdir().unwrap();
    let s = service(c.clone(), ui.path());
    let unauth = TestClient::post("http://localhost:8080/api/v1/state")
        .json(&json!({}))
        .send(&s)
        .await;
    assert_eq!(unauth.status_code, Some(StatusCode::UNAUTHORIZED));
    let mut login = TestClient::post("http://localhost:8080/api/v1/login")
        .add_header("origin", "http://localhost:8080", true)
        .json(&json!({"token":URL_SAFE_NO_PAD.encode([1;32])}))
        .send(&s)
        .await;
    assert_eq!(login.status_code, Some(StatusCode::OK));
    let cookie = login
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
    let cookie = cookie.split(';').next().unwrap().to_owned();
    let v: Value = login.take_json().await.unwrap();
    let bad = TestClient::post("http://localhost:8080/api/v1/anchors/import")
        .add_header("cookie", cookie.clone(), true)
        .json(&json!({"values":"1"}))
        .send(&s)
        .await;
    assert_eq!(bad.status_code, Some(StatusCode::FORBIDDEN));
    let good = TestClient::post("http://localhost:8080/api/v1/anchors/import")
        .add_header("cookie", cookie.clone(), true)
        .add_header("origin", "http://localhost:8080", true)
        .add_header("x-h-live-csrf", v["csrf"].as_str().unwrap(), true)
        .json(&json!({"values":"1"}))
        .send(&s)
        .await;
    assert_eq!(good.status_code, Some(StatusCode::OK));
    let bad_bearer = TestClient::post("http://localhost:8080/api/v1/state")
        .add_header("cookie", cookie.clone(), true)
        .bearer_auth("wrong")
        .json(&json!({}))
        .send(&s)
        .await;
    assert_eq!(bad_bearer.status_code, Some(StatusCode::UNAUTHORIZED));
    let logout = TestClient::post("http://localhost:8080/api/v1/logout")
        .add_header("cookie", cookie.clone(), true)
        .add_header("origin", "http://localhost:8080", true)
        .add_header("x-h-live-csrf", v["csrf"].as_str().unwrap(), true)
        .send(&s)
        .await;
    assert_eq!(logout.status_code, Some(StatusCode::OK));
    let revoked = TestClient::get("http://localhost:8080/api/v1/session")
        .add_header("cookie", cookie, true)
        .send(&s)
        .await;
    assert_eq!(revoked.status_code, Some(StatusCode::UNAUTHORIZED));
    c.shutdown().await.unwrap();
}
#[tokio::test]
async fn http_rejects_arbitrary_commands_paths_bodies_and_wrong_origin() {
    let d = tempfile::tempdir().unwrap();
    let c = open(d.path());
    let ui = tempfile::tempdir().unwrap();
    std::fs::write(ui.path().join("index.html"), "only-public-ui").unwrap();
    let s = service(c.clone(), ui.path());
    for route in ["shutdown", "backend_request", "prepare_update"] {
        assert_eq!(
            post(&s, route, json!({})).await.status_code,
            Some(StatusCode::NOT_FOUND)
        );
    }
    assert_eq!(
        post(&s, "exports/messages", json!({"output_path":"/etc/passwd"}))
            .await
            .status_code,
        Some(StatusCode::BAD_REQUEST)
    );
    let big = "x".repeat(1200 * 1024);
    assert_eq!(
        post(&s, "proxies/preview", json!({"text":big}))
            .await
            .status_code,
        Some(StatusCode::BAD_REQUEST)
    );
    let cross = TestClient::post("http://localhost:8080/api/v1/state")
        .bearer_auth(URL_SAFE_NO_PAD.encode([1; 32]))
        .add_header("origin", "https://evil.example", true)
        .json(&json!({}))
        .send(&s)
        .await;
    assert_eq!(cross.status_code, Some(StatusCode::FORBIDDEN));
    let index = TestClient::get("http://localhost:8080/").send(&s).await;
    assert_eq!(index.status_code, Some(StatusCode::OK));
    let file = TestClient::get("http://localhost:8080/api/v1/files/unknown")
        .send(&s)
        .await;
    assert_eq!(file.status_code, Some(StatusCode::UNAUTHORIZED));
    assert_eq!(
        post(&s, "state", json!({})).await.status_code,
        Some(StatusCode::OK)
    );
    c.shutdown().await.unwrap();
}
#[tokio::test]
async fn durable_monitor_restart_stop_and_archive_export() {
    let d = tempfile::tempdir().unwrap();
    let c = open(d.path());
    assert!(
        Control::open_with_transport(
            d.path().into(),
            Arc::new(ServerProtector::from_key([2; 32])),
            Arc::new(Simulation::default())
        )
        .is_err()
    );
    c.request("add_anchors", json!({"values":"1"}))
        .await
        .unwrap();
    c.request("start", json!({"rids":["1"]})).await.unwrap();
    wait(&c, |p| {
        p["anchors"][0]["message_count"].as_u64().unwrap_or(0) > 2
    })
    .await;
    c.shutdown().await.unwrap();
    assert!(!c.ready.load(std::sync::atomic::Ordering::Acquire));
    drop(c);
    let c = open(d.path());
    wait(&c, |p| {
        p["anchors"][0]["message_count"].as_u64().unwrap_or(0) > 2
    })
    .await;
    c.request("stop", json!({"rids":["1"]})).await.unwrap();
    assert!(
        !c.request("recent_messages", json!({"type":"chat"}))
            .await
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        c.request("recent_messages", json!({"type":"gift"}))
            .await
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        c.request("quality_sessions", json!({"anchor_id":"100001"}))
            .await
            .unwrap()["total"],
        1
    );
    let month = chrono::Utc::now()
        .with_timezone(&qingkan_core::archive::shanghai())
        .format("%Y-%m")
        .to_string();
    let job=c.request("export",json!({"anchor_ids":["100001"],"month":month,"format":"xlsx","file_name":"主播甲.xlsx"})).await.unwrap();
    let p = wait(&c, |p| {
        p["export_jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|j| j["id"] == job["id"] && j["status"] == "completed")
    })
    .await;
    assert!(p["export_jobs"][0]["report"].get("path").is_none());
    let id = job["id"].as_str().unwrap();
    let (file, name) = c.file(id.into()).await.unwrap();
    assert!(file.starts_with(d.path().canonicalize().unwrap()));
    assert_eq!(name, "主播甲.xlsx");
    assert!(std::fs::metadata(&file).unwrap().len() > 0);
    c.shutdown().await.unwrap();
    drop(c);
    let c = open(d.path());
    assert_eq!(
        c.request("init", json!({})).await.unwrap()["anchors"][0]["active"],
        false
    );
    assert!(c.file(id.into()).await.unwrap().0.is_file());
    c.shutdown().await.unwrap();
}
#[tokio::test]
async fn failed_intent_persistence_never_starts_monitor() {
    let d = tempfile::tempdir().unwrap();
    let c = open(d.path());
    c.request("add_anchors", json!({"values":"1"}))
        .await
        .unwrap();
    let state = d.path().join(".server/state.json");
    let old = std::fs::read(&state).unwrap();
    std::fs::remove_file(&state).unwrap();
    std::fs::create_dir(&state).unwrap();
    assert!(c.request("start", json!({"rids":["1"]})).await.is_err());
    assert_eq!(
        c.request("init", json!({})).await.unwrap()["anchors"][0]["active"],
        false
    );
    std::fs::remove_dir(&state).unwrap();
    std::fs::write(&state, old).unwrap();
    c.shutdown().await.unwrap();
}
#[tokio::test]
async fn encrypted_proxy_configuration_survives_server_restart() {
    let d = tempfile::tempdir().unwrap();
    let c = open(d.path());
    let imported=c.request("import_proxies",json!({"text":"http://synthetic-user:synthetic-pass@8.8.8.8:8080","default_protocol":"http"})).await.unwrap();
    assert_eq!(imported["valid"], 1);
    c.shutdown().await.unwrap();
    drop(c);
    let config = std::fs::read_to_string(d.path().join("config.json")).unwrap();
    assert!(!config.contains("synthetic-pass"));
    assert!(
        Control::open_with_transport(
            d.path().into(),
            Arc::new(ServerProtector::from_key([3; 32])),
            Arc::new(Simulation::default())
        )
        .is_err()
    );
    let c = open(d.path());
    assert_eq!(
        c.request("list_imported_proxies", json!({})).await.unwrap()["items"][0]["auth_present"],
        true
    );
    c.shutdown().await.unwrap();
}
