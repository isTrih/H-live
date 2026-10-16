//! Owns the in-process Rust collector and gates shutdown/update requests.
use crate::{paths, update_policy};
use h_live_runtime::Runtime;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::State;

pub(crate) struct Backend {
    runtime: Mutex<Option<Arc<Runtime>>>,
    closing: AtomicBool,
    updating: AtomicBool,
}
impl Backend {
    pub(crate) fn new() -> Self {
        Self {
            runtime: Mutex::new(None),
            closing: AtomicBool::new(false),
            updating: AtomicBool::new(false),
        }
    }
    fn get(&self) -> Result<Arc<Runtime>, String> {
        let mut state = self.runtime.lock().map_err(|_| "后台状态不可用")?;
        if self.closing.load(Ordering::Acquire) || self.updating.load(Ordering::Acquire) {
            return Err("应用正在退出或更新".into());
        }
        if state.is_none() {
            *state = Some(Arc::new(Runtime::open(paths::data_directory()?)?));
        }
        Ok(state.as_ref().unwrap().clone())
    }
    pub(crate) fn begin_close(&self) -> bool {
        !self.closing.swap(true, Ordering::AcqRel)
    }
    pub(crate) fn shutdown(&self) -> Result<(), String> {
        let mut state = self.runtime.lock().map_err(|_| "后台状态不可用")?;
        if let Some(runtime) = state.as_ref() {
            if let Err(error) = runtime.shutdown() {
                self.closing.store(false, Ordering::Release);
                return Err(error);
            }
        }
        state.take();
        Ok(())
    }
}
#[tauri::command]
pub(crate) async fn backend_request(
    state: State<'_, Arc<Backend>>,
    method: String,
    params: Value,
) -> Result<Value, String> {
    if method.is_empty() || method.len() > 80 {
        return Err("无效请求".into());
    }
    let backend = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let runtime = backend.get()?;
        runtime.request(&method, params)
    })
    .await
    .map_err(|_| "后台请求任务异常".to_string())?
}
#[tauri::command]
pub(crate) async fn update_install_guard(state: State<'_, Arc<Backend>>) -> Result<Value, String> {
    let backend = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if backend.closing.load(Ordering::Acquire) || backend.updating.load(Ordering::Acquire) {
            return Ok(json!({"ready":false,"reason":"pending"}));
        }
        let state = backend.runtime.lock().map_err(|_| "后台状态不可用")?;
        Ok(state
            .as_ref()
            .map(|r| update_policy::readiness(r.update_readiness()))
            .unwrap_or(json!({"ready":true,"reason":"ready"})))
    })
    .await
    .map_err(|_| "无法检查后台状态".to_string())?
}
#[tauri::command]
pub(crate) async fn prepare_update_install(
    state: State<'_, Arc<Backend>>,
    target_version: String,
) -> Result<Value, String> {
    let backend = state.inner().clone();
    if backend.updating.swap(true, Ordering::AcqRel) {
        return Ok(json!({"ready":false,"reason":"pending"}));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let result = (|| -> Result<Value, String> {
            let mut state = backend.runtime.lock().map_err(|_| "后台状态不可用")?;
            if backend.closing.load(Ordering::Acquire) {
                return Ok(json!({"ready":false,"reason":"pending"}));
            }
            if let Some(runtime) = state.as_ref() {
                let ready = update_policy::readiness(
                    runtime.request("prepare_update", json!({"target_version":target_version}))?,
                );
                if ready["ready"] != true {
                    return Ok(ready);
                }
                let confirmation = runtime.shutdown()?;
                if !update_policy::shutdown_confirmed(&confirmation) {
                    return Err("原生后台未确认退出".into());
                }
            }
            state.take();
            Ok(json!({"ready":true,"reason":"ready"}))
        })();
        if !matches!(&result,Ok(v) if v["ready"]==true) {
            backend.updating.store(false, Ordering::Release);
            if let Ok(s) = backend.runtime.lock() {
                if let Some(r) = s.as_ref() {
                    let _ = r.request("cancel_update", json!({}));
                }
            }
        }
        result
    })
    .await
    .map_err(|_| "后台未能安全结束，已取消更新".to_string())?
}
#[tauri::command]
pub(crate) async fn cancel_update_install(state: State<'_, Arc<Backend>>) -> Result<(), String> {
    let backend = state.inner().clone();
    backend.updating.store(false, Ordering::Release);
    tauri::async_runtime::spawn_blocking(move || {
        let runtime = backend.get()?;
        runtime.request("cancel_update", json!({})).map(|_| ())
    })
    .await
    .map_err(|_| "恢复监控失败".to_string())?
}
