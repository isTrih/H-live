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
async fn debug_logging_settings_route_requires_authentication_and_updates_poll_settings() {
    let d = tempfile::tempdir().unwrap();
    let c = open(d.path());
    let ui = tempfile::tempdir().unwrap();
    let s = service(c.clone(), ui.path());
    let unauthenticated = TestClient::post("http://localhost:8080/api/v1/settings")
        .json(&json!({"debug_logging":true}))
        .send(&s)
        .await;
    assert_eq!(unauthenticated.status_code, Some(StatusCode::UNAUTHORIZED));
    let mut initial = post(&s, "state", json!({})).await;
    assert_eq!(initial.take_json::<Value>().await.unwrap()["settings"]["debug_logging"], false);
    let mut updated = post(&s, "settings", json!({"debug_logging":true})).await;
    assert_eq!(updated.status_code, Some(StatusCode::OK));
    assert_eq!(updated.take_json::<Value>().await.unwrap()["debug_logging"], true);
    let mut poll = post(&s, "events/poll", json!({})).await;
    assert_eq!(poll.take_json::<Value>().await.unwrap()["settings"]["debug_logging"], true);
    let invalid = post(&s, "settings", json!({"debug_logging":"true"})).await;
    assert_eq!(invalid.status_code, Some(StatusCode::BAD_REQUEST));
    c.shutdown().await.unwrap();
}

#[tokio::test]
async fn global_highlight_routes_keep_keys_encrypted_and_require_authentication() {
    let d = tempfile::tempdir().unwrap();
    let c = open(d.path());
    let ui = tempfile::tempdir().unwrap();
    let s = service(c.clone(), ui.path());
    for path in [
        "analysis/timeline",
        "highlights/settings",
        "highlights/configure",
        "highlights/models",
        "highlights/analyze",
        "highlights/status",
        "highlights/cancel",
    ] {
        let response = TestClient::post(format!("http://localhost:8080/api/v1/{path}"))
            .json(&json!({}))
            .send(&s)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::UNAUTHORIZED));
    }
    let mut response = post(&s,"highlights/configure",json!({"base_url":"https://provider.example/v1","api_key":"synthetic-model-key","model":""})).await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let public: Value = response.take_json().await.unwrap();
    assert_eq!(public["key_configured"], true);
    assert!(public.get("api_key").is_none());
    assert!(!public.to_string().contains("synthetic-model-key"));
    assert!(
        !std::fs::read_to_string(d.path().join("config.json"))
            .unwrap()
            .contains("synthetic-model-key")
    );
    let mut response = post(&s, "highlights/settings", json!({})).await;
    assert_eq!(response.take_json::<Value>().await.unwrap(), public);
    c.shutdown().await.unwrap();
}

#[tokio::test]
async fn community_routes_require_admin_auth_csrf_and_keep_replay_keys_private() {
    let d = tempfile::tempdir().unwrap();
    let c = open(d.path());
    let ui = tempfile::tempdir().unwrap();
    let s = service(c.clone(), ui.path());
    for path in ["community/settings", "community/configure", "replays/session", "replays/session/timing"] {
        let response = TestClient::post(format!("http://localhost:8080/api/v1/{path}"))
            .json(&json!({}))
            .send(&s)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::UNAUTHORIZED));
    }
    let mut configured = post(&s, "community/configure", json!({"enabled":false,"api_key":"synthetic-community-key"})).await;
    assert_eq!(configured.status_code, Some(StatusCode::OK));
    let settings = configured.take_json::<Value>().await.unwrap();
    assert_eq!(settings, json!({"enabled":false,"key_configured":true}));
    assert!(!std::fs::read_to_string(d.path().join("config.json")).unwrap().contains("synthetic-community-key"));
    let mut current = post(&s, "community/settings", json!({})).await;
    assert_eq!(current.take_json::<Value>().await.unwrap(), settings);
    let mut replay = post(&s, "replays/session", json!({"session_id":"101"})).await;
    assert_eq!(replay.status_code, Some(StatusCode::OK));
    assert_eq!(replay.take_json::<Value>().await.unwrap(), json!({"enabled":false,"available":false,"replay":null}));
    let invalid = post(&s, "replays/session", json!({"session_id":"../101"})).await;
    assert_eq!(invalid.status_code, Some(StatusCode::BAD_REQUEST));
    let timing = post(&s, "replays/session/timing", json!({"session_id":"101","play_url":"https://v26-star.xingtu.cn/a.m3u8"})).await;
    assert_eq!(timing.status_code, Some(StatusCode::BAD_REQUEST), "feature-off timing must fail before connecting");

    let mut login = TestClient::post("http://localhost:8080/api/v1/login")
        .add_header("origin", "http://localhost:8080", true)
        .json(&json!({"token":URL_SAFE_NO_PAD.encode([1;32])}))
        .send(&s)
        .await;
    let cookie = login.headers().get("set-cookie").unwrap().to_str().unwrap().split(';').next().unwrap().to_owned();
    let login = login.take_json::<Value>().await.unwrap();
    for path in ["community/configure", "replays/session", "replays/session/timing"] {
        let response = TestClient::post(format!("http://localhost:8080/api/v1/{path}"))
            .add_header("cookie", cookie.clone(), true)
            .json(&json!({"session_id":"101","enabled":false}))
            .send(&s)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::FORBIDDEN));
    }
    let mut replay = TestClient::post("http://localhost:8080/api/v1/replays/session")
        .add_header("cookie", cookie, true)
        .add_header("origin", "http://localhost:8080", true)
        .add_header("x-h-live-csrf", login["csrf"].as_str().unwrap(), true)
        .json(&json!({"session_id":"101"}))
        .send(&s)
        .await;
    assert_eq!(replay.status_code, Some(StatusCode::OK));
    assert_eq!(replay.take_json::<Value>().await.unwrap()["available"], false);
    c.shutdown().await.unwrap();
}

#[tokio::test]
async fn live_marks_http_routes_require_auth_csrf_and_preserve_crud_after_stop() {
    async fn cookie_post(
        service: &Service,
        path: &str,
        params: &Value,
        cookie: &str,
        origin: Option<&str>,
        csrf: Option<&str>,
    ) -> Response {
        let mut request = TestClient::post(format!("http://localhost:8080/api/v1/{path}"))
            .add_header("cookie", cookie, true);
        if let Some(origin) = origin {
            request = request.add_header("origin", origin, true);
        }
        if let Some(csrf) = csrf {
            request = request.add_header("x-h-live-csrf", csrf, true);
        }
        request.json(params).send(service).await
    }

    let d = tempfile::tempdir().unwrap();
    let c = open(d.path());
    let ui = tempfile::tempdir().unwrap();
    let s = service(c.clone(), ui.path());
    for path in ["marks/list", "marks/create", "marks/update", "marks/delete"] {
        let response = TestClient::post(format!("http://localhost:8080/api/v1/{path}"))
            .json(&json!({}))
            .send(&s)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::UNAUTHORIZED), "{path}");
    }

    c.request("add_anchors", json!({"values":"1"})).await.unwrap();
    c.request("start", json!({"rids":["1"]})).await.unwrap();
    let connected = wait(&c, |p| p["metrics"]["connected"] == 1).await;
    let anchor = connected["anchors"][0]["anchor_id"].as_str().unwrap();
    let session = connected["anchors"][0]["session_id"].as_str().unwrap();
    assert_eq!(anchor, "100001");
    assert_eq!(session, "200001");

    let mut login = TestClient::post("http://localhost:8080/api/v1/login")
        .add_header("origin", "http://localhost:8080", true)
        .json(&json!({"token":URL_SAFE_NO_PAD.encode([1;32])}))
        .send(&s)
        .await;
    assert_eq!(login.status_code, Some(StatusCode::OK));
    let cookie = login.headers().get("set-cookie").unwrap().to_str().unwrap()
        .split(';').next().unwrap().to_owned();
    let login = login.take_json::<Value>().await.unwrap();
    let csrf = login["csrf"].as_str().unwrap();
    let create = json!({"anchor_id":anchor,"session_id":session,"title":"精彩转折","note":"稍后剪辑","request_key":"http-mark-1","marked_at_epoch":1});
    for (path, params) in [
        ("marks/create", create.clone()),
        ("marks/update", json!({"id":"0123456789abcdef0123456789abcdef","title":"改名"})),
        ("marks/delete", json!({"id":"0123456789abcdef0123456789abcdef"})),
    ] {
        for (origin, token) in [
            (None, Some(csrf)),
            (Some("http://localhost:8080"), None),
            (Some("https://evil.example"), Some(csrf)),
        ] {
            let response = cookie_post(&s, path, &params, &cookie, origin, token).await;
            assert_eq!(response.status_code, Some(StatusCode::FORBIDDEN), "{path}, origin={origin:?}, csrf_present={}", token.is_some());
        }
    }
    let scope = json!({"anchor_id":anchor,"session_id":session});
    let mut listed = cookie_post(&s, "marks/list", &scope, &cookie, None, None).await;
    assert_eq!(listed.status_code, Some(StatusCode::OK));
    assert_eq!(listed.take_json::<Value>().await.unwrap()["total"], 0, "rejected writes must never reach the runtime");

    let before = chrono::Utc::now().timestamp_millis() as f64 / 1000.;
    let mut created = cookie_post(&s, "marks/create", &create, &cookie, Some("http://localhost:8080"), Some(csrf)).await;
    assert_eq!(created.status_code, Some(StatusCode::OK));
    let created = created.take_json::<Value>().await.unwrap();
    assert_eq!(created["created"], true);
    let mark = &created["mark"];
    let id = mark["id"].as_str().unwrap();
    assert_eq!(mark["anchor_id"], anchor);
    assert_eq!(mark["session_id"], session);
    assert!(mark["marked_at_epoch"].as_f64().unwrap() >= before, "the HTTP caller cannot override the live wall-clock time");

    c.request("stop", json!({"rids":["1"]})).await.unwrap();
    let mut retry = cookie_post(&s, "marks/create", &create, &cookie, Some("http://localhost:8080"), Some(csrf)).await;
    assert_eq!(retry.status_code, Some(StatusCode::OK));
    let retry = retry.take_json::<Value>().await.unwrap();
    assert_eq!(retry["created"], false);
    assert_eq!(retry["mark"]["id"], id, "retry after stopping is idempotent");

    let mut listed = post(&s, "marks/list", scope.clone()).await;
    assert_eq!(listed.status_code, Some(StatusCode::OK));
    let listed = listed.take_json::<Value>().await.unwrap();
    assert_eq!(listed["total"], 1);
    assert_eq!(listed["marks"][0]["id"], id);
    let mut updated = cookie_post(&s, "marks/update", &json!({"id":id,"title":"精选片段","note":"记录补充\n下一行"}), &cookie, Some("http://localhost:8080"), Some(csrf)).await;
    assert_eq!(updated.status_code, Some(StatusCode::OK));
    let updated = updated.take_json::<Value>().await.unwrap();
    assert_eq!(updated["mark"]["title"], "精选片段");
    assert_eq!(updated["mark"]["note"], "记录补充\n下一行");
    assert_eq!(updated["mark"]["marked_at_epoch"], mark["marked_at_epoch"]);
    let mut deleted = cookie_post(&s, "marks/delete", &json!({"id":id}), &cookie, Some("http://localhost:8080"), Some(csrf)).await;
    assert_eq!(deleted.status_code, Some(StatusCode::OK));
    assert_eq!(deleted.take_json::<Value>().await.unwrap()["deleted"], true);
    let mut listed = post(&s, "marks/list", scope).await;
    assert_eq!(listed.take_json::<Value>().await.unwrap()["total"], 0);
    c.shutdown().await.unwrap();
}

#[tokio::test]
async fn slow_authenticated_model_discovery_does_not_block_monitor_stop() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::oneshot,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let (accepted_tx, accepted_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let mut bytes = [0u8; 1024];
            let count = socket.read(&mut bytes).await.unwrap();
            assert!(count > 0);
            request.extend_from_slice(&bytes[..count]);
        }
        assert!(String::from_utf8_lossy(&request).starts_with("GET /v1/models HTTP/1.1"));
        accepted_tx.send(()).unwrap();
        release_rx.await.unwrap();
        let body = json!({"data":[{"id":"slow-model"}]}).to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    let d = tempfile::tempdir().unwrap();
    let c = open(d.path());
    let ui = tempfile::tempdir().unwrap();
    let s = service(c.clone(), ui.path());
    c.request("add_anchors", json!({"values":"1"}))
        .await
        .unwrap();
    c.request("start", json!({"rids":["1"]})).await.unwrap();
    wait(&c, |p| p["metrics"]["connected"] == 1).await;
    let configured = post(
        &s,
        "highlights/configure",
        json!({"base_url":base,"api_key":"synthetic-key","model":""}),
    )
    .await;
    assert_eq!(configured.status_code, Some(StatusCode::OK));
    let models = post(&s, "highlights/models", json!({}));
    tokio::pin!(models);
    tokio::select! {
        _=&mut models=>panic!("model discovery completed before the upstream was released"),
        accepted=accepted_rx=>accepted.unwrap(),
    }
    // The provider response is held while the authenticated Stop route must
    // finish through the same server actor, so this catches actor serialization.
    let stopped = tokio::time::timeout(
        Duration::from_secs(1),
        post(&s, "monitor/stop", json!({"rids":["1"]})),
    )
    .await;
    release_tx.send(()).unwrap();
    let mut models = models.await;
    assert_eq!(models.status_code, Some(StatusCode::OK));
    assert_eq!(
        models.take_json::<Value>().await.unwrap()["models"],
        json!([{"id":"slow-model"}])
    );
    let mut stopped = stopped.expect("server Stop was blocked by model discovery");
    assert_eq!(stopped.status_code, Some(StatusCode::OK));
    assert_eq!(stopped.take_json::<Value>().await.unwrap()["accepted"], 1);
    upstream.await.unwrap();
    c.shutdown().await.unwrap();
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
