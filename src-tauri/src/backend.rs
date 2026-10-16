//! Serializes collector requests and guards application shutdown/update.
use crate::{engine::Engine, update_policy};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::State;

pub(crate) struct Backend {
    engine: Mutex<Option<Engine>>,
    closing: AtomicBool,
    updating: AtomicBool,
}

#[tauri::command]
pub(crate) async fn backend_request(
    app: tauri::AppHandle,
    state: State<'_, Arc<Backend>>,
    method: String,
    params: Value,
) -> Result<Value, String> {
    if method.is_empty() || method.len() > 80 {
        return Err("无效请求".into());
    }
    if state.closing.load(Ordering::Acquire) || state.updating.load(Ordering::Acquire) {
        return Err("应用正在退出".into());
    }
    let backend = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let mut engine = backend.engine.lock().map_err(|_| "后台状态不可用")?;
        // A request queued before CloseRequested must not create a fresh child
        // after the close worker has already shut the previous engine down.
        if backend.closing.load(Ordering::Acquire) || backend.updating.load(Ordering::Acquire) {
            return Err("应用正在退出".into());
        }
        if engine.as_mut().is_some_and(|child| child.has_exited()) {
            engine.take();
        }
        if engine.is_none() {
            *engine = Some(Engine::start(&app)?);
        }
        engine.as_mut().unwrap().request(&method, params)
    })
    .await
    .map_err(|_| "后台请求任务异常".to_string())?
}

#[tauri::command]
pub(crate) async fn update_install_guard(state: State<'_, Arc<Backend>>) -> Result<Value, String> {
    let backend = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let mut engine = backend.engine.lock().map_err(|_| "后台状态不可用")?;
        if backend.closing.load(Ordering::Acquire) || backend.updating.load(Ordering::Acquire) {
            return Ok(json!({"ready":false,"reason":"closing"}));
        }
        match engine.as_mut() {
            Some(engine) => Ok(update_policy::readiness(
                engine.request("update_readiness", json!({}))?,
            )),
            None => Ok(json!({"ready":true,"reason":"ready"})),
        }
    })
    .await
    .map_err(|_| "无法检查后台状态".to_string())?
}

#[tauri::command]
pub(crate) async fn prepare_update_install(
    state: State<'_, Arc<Backend>>,
    target_version: String,
) -> Result<Value, String> {
    let backend = Arc::clone(state.inner());
    if backend.updating.swap(true, Ordering::AcqRel) {
        return Ok(json!({"ready":false,"reason":"pending"}));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let result = (|| -> Result<Value, String> {
            let mut engine = backend.engine.lock().map_err(|_| "后台状态不可用")?;
            if backend.closing.load(Ordering::Acquire) {
                return Ok(json!({"ready":false,"reason":"closing"}));
            }
            if let Some(child) = engine.as_mut() {
                let ready = update_policy::readiness(
                    child.request("prepare_update", json!({"target_version":target_version}))?,
                );
                if ready["ready"] != true {
                    return Ok(ready);
                }
                child.shutdown_for_update()?;
                // The child has exited successfully, so releasing its Job Object
                // cannot interrupt an archive transaction or a pending export.
                engine.take();
            }
            Ok(json!({"ready":true,"reason":"ready"}))
        })();
        if !matches!(&result, Ok(value) if value["ready"] == true) {
            backend.updating.store(false, Ordering::Release);
        }
        result
    })
    .await
    .map_err(|_| "无法安全结束后台，已取消更新".to_string())?
}

#[tauri::command]
pub(crate) async fn cancel_update_install(state: State<'_, Arc<Backend>>) -> Result<(), String> {
    // Installer launch failed. The next normal request can start a fresh engine.
    state.updating.store(false, Ordering::Release);
    let backend = Arc::clone(state.inner());
    let _ = tauri::async_runtime::spawn_blocking(move || {
        if let Ok(mut engine) = backend.engine.lock() {
            if let Some(child) = engine.as_mut() {
                if child.has_exited() {
                    engine.take();
                } else {
                    let _ = child.request("cancel_update", json!({}));
                }
            }
        }
    })
    .await;
    Ok(())
}

impl Backend {
    pub(crate) fn new() -> Self {
        Self {
            engine: Mutex::new(None),
            closing: AtomicBool::new(false),
            updating: AtomicBool::new(false),
        }
    }

    pub(crate) fn begin_close(&self) -> bool {
        !self.closing.swap(true, Ordering::AcqRel)
    }

    pub(crate) fn shutdown(&self) {
        if let Ok(mut state) = self.engine.lock() {
            if let Some(mut engine) = state.take() {
                engine.shutdown();
            }
        }
    }
}
