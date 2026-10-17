//! Explicit management endpoints; no desktop command or filesystem RPC is exposed.
use crate::{auth::Auth, control::Control};
use salvo::{fs::NamedFile, prelude::*, serve_static::StaticDir};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::sync::Semaphore;

pub struct App {
    pub control: Arc<Control>,
    pub auth: Auth,
    slots: Arc<Semaphore>,
}
impl App {
    pub fn new(control: Arc<Control>, auth: Auth) -> Arc<Self> {
        Arc::new(Self {
            control,
            auth,
            slots: Arc::new(Semaphore::new(64)),
        })
    }
}
// Both read and write endpoints use bounded JSON bodies (avoids sensitive URL query logs).
pub const ENDPOINTS: &[(&str, &str, bool)] = &[
    ("state", "init", false),
    ("events/poll", "poll", false),
    ("messages/recent", "recent_messages", false),
    ("archives/anchors", "archive_anchors", false),
    ("archives/sessions", "archive_sessions", false),
    ("archives/delete-preview", "preview_archive_delete", false),
    ("archives/delete", "execute_archive_delete", true),
    ("archives/delete-status", "archive_delete_status", false),
    ("anchors/import", "add_anchors", true),
    ("anchors/remove", "remove", true),
    ("anchors/rename", "rename", true),
    ("anchors/pc-support", "set_pc_unsupported", true),
    ("monitor/start", "start", true),
    ("monitor/stop", "stop", true),
    ("proxies/settings", "configure_proxy", true),
    ("proxies/status", "proxy_status", false),
    ("proxies/preview", "preview_proxies", false),
    ("proxies/import", "import_proxies", true),
    ("proxies/list", "list_imported_proxies", false),
    ("proxies/remove", "remove_imported_proxies", true),
    ("proxies/check", "refresh_proxies", true),
    ("subscriptions/list", "list_proxy_subscriptions", false),
    ("subscriptions/add", "add_proxy_subscription", true),
    ("subscriptions/refresh", "refresh_proxy_subscription", true),
    ("subscriptions/remove", "remove_proxy_subscription", true),
    ("viewers/refresh", "refresh_viewers", true),
    ("viewers/status", "viewer_refresh_status", false),
    ("analysis/sessions", "quality_sessions", false),
    ("analysis/detail", "quality_detail", false),
    ("analysis/settings", "quality_settings", false),
    ("analysis/rebuild", "quality_rebuild", true),
    ("analysis/pause", "quality_pause", true),
    ("analysis/configure", "quality_configure", true),
    ("analysis/commands", "quality_commands", true),
    ("exports/messages", "export", true),
    ("exports/analysis", "quality_export", true),
    ("exports/cancel", "cancel_export", true),
];
pub fn router(app: Arc<App>, ui: PathBuf) -> Router {
    let mut api = Router::with_path("api/v1")
        .hoop(WithApp(app.clone()))
        .hoop(guard);
    api = api
        .push(Router::with_path("login").post(login))
        .push(Router::with_path("session").get(session))
        .push(Router::with_path("logout").post(logout))
        .push(Router::with_path("files/{id}").get(download));
    for &(path, method, mutation) in ENDPOINTS {
        api = api.push(Router::with_path(path).post(Operation { method, mutation }));
    }
    api = api.push(Router::with_path("{**rest}").goal(api_missing));
    Router::new()
        .hoop(headers)
        .push(Router::with_path("healthz").get(Health(app)))
        .push(api)
        .push(
            Router::with_path("{**path}").get(
                StaticDir::new(ui)
                    .defaults("index.html")
                    .auto_list(false)
                    .include_dot_files(false),
            ),
        )
}
struct WithApp(Arc<App>);
#[async_trait]
impl Handler for WithApp {
    async fn handle(
        &self,
        req: &mut Request,
        depot: &mut Depot,
        res: &mut Response,
        ctrl: &mut FlowCtrl,
    ) {
        depot.insert("app", self.0.clone());
        ctrl.call_next(req, depot, res).await;
    }
}
fn app(depot: &Depot) -> Arc<App> {
    depot.get::<Arc<App>>("app").unwrap().clone()
}
fn error(res: &mut Response, status: u16, text: &str) {
    res.status_code(StatusCode::from_u16(status).unwrap());
    res.render(Json(json!({"error":text})));
}
fn cookie(req: &Request, name: &str) -> String {
    req.header::<String>("cookie")
        .unwrap_or_default()
        .split(';')
        .filter_map(|s| s.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_owned())
        .unwrap_or_default()
}
fn authenticated(req: &Request, a: &Auth) -> Option<Option<String>> {
    if let Some(raw) = req.header::<String>("authorization") {
        return raw
            .strip_prefix("Bearer ")
            .filter(|token| a.bearer(token))
            .map(|_| None);
    }
    a.session(&cookie(req, a.cookie_name())).map(Some)
}
fn permit(req: &Request, res: &mut Response, a: &Auth, mutation: bool) -> bool {
    let Some(csrf) = authenticated(req, a) else {
        error(res, 401, "请先登录 H-live 管理端");
        return false;
    };
    if mutation
        && let Some(csrf) = csrf
        && (req.header::<String>("origin").as_deref() != Some(&a.origin)
            || req.header::<String>("x-h-live-csrf").as_deref() != Some(&csrf))
    {
        error(res, 403, "请求来源或会话校验失败");
        return false;
    }
    true
}
#[handler]
async fn headers(req: &mut Request, depot: &mut Depot, res: &mut Response, ctrl: &mut FlowCtrl) {
    res.headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    res.headers_mut()
        .insert("referrer-policy", "no-referrer".parse().unwrap());
    res.headers_mut()
        .insert("x-frame-options", "DENY".parse().unwrap());
    res.headers_mut().insert("content-security-policy","default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'".parse().unwrap());
    ctrl.call_next(req, depot, res).await;
}
#[handler]
async fn guard(req: &mut Request, depot: &mut Depot, res: &mut Response, ctrl: &mut FlowCtrl) {
    let a = app(depot);
    res.headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    if let Some(origin) = req.header::<String>("origin")
        && origin != a.auth.origin
    {
        error(res, 403, "请求来源无效");
        ctrl.skip_rest();
        return;
    }
    if !a.control.ready.load(Ordering::Acquire) {
        error(res, 503, "服务正在停止");
        ctrl.skip_rest();
        return;
    }
    let Ok(_slot) = a.slots.clone().try_acquire_owned() else {
        error(res, 503, "并发请求过多");
        ctrl.skip_rest();
        return;
    };
    // Set before any parsing, including chunked payloads.
    req.limit_body(if req.uri().path().contains("/proxies/") {
        1100 * 1024
    } else {
        256 * 1024
    });
    ctrl.call_next(req, depot, res).await;
}
async fn body(req: &mut Request, res: &mut Response) -> Option<Value> {
    match tokio::time::timeout(Duration::from_secs(10), req.parse_json::<Value>()).await {
        Ok(Ok(v)) if v.is_object() => Some(v),
        _ => {
            error(res, 400, "JSON 请求无效、超出大小限制或读取超时");
            None
        }
    }
}
#[handler]
async fn login(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let a = app(depot);
    if req.header::<String>("origin").as_deref() != Some(&a.auth.origin) {
        error(res, 403, "登录必须来自配置的管理端地址");
        return;
    }
    let Some(p) = body(req, res).await else {
        return;
    };
    let Some(token) = p["token"].as_str().filter(|s| s.len() <= 100) else {
        error(res, 400, "管理令牌无效");
        return;
    };
    match a.auth.login(token) {
        Ok((token, csrf)) => {
            res.headers_mut()
                .insert("set-cookie", a.auth.cookie(&token, 43200).parse().unwrap());
            res.render(Json(json!({"csrf":csrf,"version":crate::VERSION})));
        }
        Err(code) => error(res, code, "管理令牌无效或尝试过于频繁"),
    }
}
#[handler]
async fn session(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let a = app(depot);
    match authenticated(req, &a.auth) {
        Some(csrf) => res.render(Json(
            json!({"csrf":csrf,"version":crate::VERSION,"instance":a.control.instance}),
        )),
        None => error(res, 401, "请先登录 H-live 管理端"),
    }
}
#[handler]
async fn logout(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let a = app(depot);
    if !permit(req, res, &a.auth, true) {
        return;
    }
    a.auth.logout(&cookie(req, a.auth.cookie_name()));
    res.headers_mut()
        .insert("set-cookie", a.auth.cookie("", 0).parse().unwrap());
    res.render(Json(json!({"ok":true})));
}
#[handler]
async fn api_missing(res: &mut Response) {
    error(res, 404, "管理接口不存在");
}
struct Health(Arc<App>);
#[async_trait]
impl Handler for Health {
    async fn handle(
        &self,
        _req: &mut Request,
        _depot: &mut Depot,
        res: &mut Response,
        _ctrl: &mut FlowCtrl,
    ) {
        if self.0.control.ready.load(Ordering::Acquire) {
            res.render(Json(json!({"ok":true})))
        } else {
            error(res, 503, "stopping")
        }
    }
}
struct Operation {
    method: &'static str,
    mutation: bool,
}
#[async_trait]
impl Handler for Operation {
    async fn handle(
        &self,
        req: &mut Request,
        depot: &mut Depot,
        res: &mut Response,
        _ctrl: &mut FlowCtrl,
    ) {
        let a = app(depot);
        if !permit(req, res, &a.auth, self.mutation) {
            return;
        }
        let Some(mut p) = body(req, res).await else {
            return;
        };
        if p.get("output_path").is_some() {
            error(res, 400, "服务端不接受本机文件路径");
            return;
        }
        if self.method == "poll" {
            if p["instance"] != a.control.instance {
                p["after"] = json!(0);
            }
            p.as_object_mut().unwrap().remove("instance");
        }
        match a.control.request(self.method, p).await {
            Ok(mut v) => {
                if self.mutation {
                    println!("management operation={} result=ok", self.method);
                }
                if ["init", "poll"].contains(&self.method) {
                    v["server_version"] = json!(crate::VERSION);
                    v["instance"] = json!(a.control.instance);
                }
                res.render(Json(v));
            }
            Err(e) => {
                if self.mutation {
                    println!("management operation={} result=error", self.method);
                }
                error(res, 400, &e)
            }
        }
    }
}
#[handler]
async fn download(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let a = app(depot);
    if !permit(req, res, &a.auth, false) {
        return;
    }
    let id = req.param::<String>("id").unwrap_or_default();
    match a.control.file(id).await {
        Ok((path, name)) => {
            NamedFile::builder(path)
                .attached_name(name)
                .send(req.headers(), res)
                .await;
        }
        Err(_) => error(res, 404, "导出文件不存在或尚未完成"),
    }
}
