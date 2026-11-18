//! Owns the in-process Rust collector and gates shutdown/update requests.
use crate::{paths, update_policy};
use h_live_runtime::Runtime;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, RwLock, RwLockReadGuard,
};
use tauri::State;

pub(crate) struct Backend {
    runtime: Mutex<Option<Arc<Runtime>>>,
    media_tools: Mutex<Option<std::path::PathBuf>>,
    closing: AtomicBool,
    updating: AtomicBool,
    launched: AtomicBool,
    // Restores wait for every IPC holding a Runtime/SQLite handle to finish.
    pub(crate) operations: RwLock<()>,
    pub(crate) backups: Mutex<Option<h_live_runtime::data_backup::PreparedBackup>>,
}
impl Backend {
    pub(crate) fn new() -> Self {
        Self {
            runtime: Mutex::new(None),
            media_tools: Mutex::new(None),
            closing: AtomicBool::new(false),
            updating: AtomicBool::new(false),
            launched: AtomicBool::new(false),
            operations: RwLock::new(()),
            backups: Mutex::new(None),
        }
    }
    pub(crate) fn get(&self) -> Result<Arc<Runtime>, String> {
        let mut state = self.runtime.lock().map_err(|_| "后台状态不可用")?;
        if self.closing.load(Ordering::Acquire) || self.updating.load(Ordering::Acquire) {
            return Err("应用正在退出或更新".into());
        }
        if state.is_none() {
            let directory = paths::data_directory()?;
            h_live_runtime::data_backup::recover_restore(&directory)?;
            let runtime = Arc::new(Runtime::open(directory)?);
            if let Some(directory) = self.media_tools.lock().map_err(|_| "媒体工具配置不可用")?.as_ref() {
                runtime.configure_media_tools(directory.clone())?;
            }
            if !self.launched.load(Ordering::Acquire) {
                runtime.request("start_on_launch", json!({}))?;
                self.launched.store(true, Ordering::Release);
            }
            *state = Some(runtime);
        }
        Ok(state.as_ref().unwrap().clone())
    }
    pub(crate) fn initialize(&self) -> Result<(), String> {
        let _operation = self.read_operation()?;
        self.get().map(|_| ())
    }
    pub(crate) fn read_operation(&self) -> Result<RwLockReadGuard<'_, ()>, String> {
        self.operations.read().map_err(|_| "数据操作状态不可用".into())
    }
    pub(crate) fn restore_backup(&self, token: &str) -> Result<Value, String> {
        let _operation = self.operations.write().map_err(|_| "数据操作状态不可用")?;
        if self.closing.load(Ordering::Acquire) || self.updating.load(Ordering::Acquire) {
            return Err("应用正在退出或更新，请稍后恢复".into());
        }
        let mut pending = self.backups.lock().map_err(|_| "备份状态不可用")?;
        let backup = pending.as_ref().filter(|backup| backup.token() == token)
            .ok_or("备份预览已失效，请重新选择备份文件")?;
        let runtime = self.get()?;
        let readiness = runtime.update_readiness();
        if readiness["ready"] != true && readiness["reason"] != "monitoring" {
            return Err("请等待下载、导出和后台分析完成后再恢复数据".into());
        }
        let previous_tasks = runtime.running_tasks();
        if Arc::strong_count(&runtime) != 2 {
            return Err("仍有录播或数据请求未结束，请稍后恢复".into());
        }
        // No new reader can enter; drop the owned instance only after shutdown
        // confirms its writers and worker threads have stopped.
        runtime.shutdown()?;
        self.runtime.lock().map_err(|_| "后台状态不可用")?.take();
        match Arc::try_unwrap(runtime) {
            Ok(runtime) => drop(runtime),
            Err(runtime) => {
                *self.runtime.lock().map_err(|_| "后台状态不可用")? = Some(runtime);
                return Err("后台数据连接尚未释放，请重新打开应用后恢复".into());
            },
        }
        let directory = paths::data_directory()?;
        let result = backup.restore(&directory);
        // A restore attempt consumes its preview, including a rolled-back one.
        pending.take();
        drop(pending);
        // Importing records must not start newly imported monitoring tasks.
        // Resume only the tasks this process had running before the merge.
        self.launched.store(true, Ordering::Release);
        let resumed = self.get().and_then(|runtime| {
            if !previous_tasks.is_empty() { runtime.request("start", json!({"rids":previous_tasks}))?; }
            Ok(runtime)
        });
        match (result, resumed) {
            (Ok(mut result), Ok(_)) => { result["restored"] = json!(true); Ok(result) },
            (Ok(mut result), Err(error)) => {
                // The restore itself committed. Let the UI reload and expose a
                // recoverable startup failure instead of offering to reapply it.
                result["restored"] = json!(true);
                result["startup_error"] = json!(error);
                Ok(result)
            },
            (Err(error), Err(startup)) => Err(format!("{error}；后台恢复失败：{startup}，请重新打开应用；备份预览已失效")),
            (Err(error), Ok(_)) => Err(format!("{error}；备份预览已失效，请重新选择")),
        }
    }
    pub(crate) fn configure_media_tools(&self, directory: std::path::PathBuf) -> Result<(), String> {
        *self.media_tools.lock().map_err(|_| "媒体工具配置不可用")? = Some(directory);
        Ok(())
    }
    pub(crate) fn begin_close(&self) -> bool {
        !self.closing.swap(true, Ordering::AcqRel)
    }
    pub(crate) fn shutdown(&self) -> Result<(), String> {
        let _operation = self.operations.write().map_err(|_| "数据操作状态不可用")?;
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
        let _operation = backend.read_operation()?;
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
        let _operation = backend.read_operation()?;
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
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = backend.operations.write().map_err(|_| "数据操作状态不可用")?;
        if backend.updating.swap(true, Ordering::AcqRel) {
            return Ok(json!({"ready":false,"reason":"pending"}));
        }
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
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = backend.read_operation()?;
        backend.updating.store(false, Ordering::Release);
        let runtime = backend.get()?;
        runtime.request("cancel_update", json!({})).map(|_| ())
    })
    .await
    .map_err(|_| "恢复监控失败".to_string())?
}

#[cfg(test)]
mod backup_gate_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn import_waits_for_in_flight_runtime_users() {
        let backend = Arc::new(Backend::new());
        let reader = backend.read_operation().unwrap();
        let (sent, received) = std::sync::mpsc::channel();
        let other = backend.clone();
        let worker = std::thread::spawn(move || {
            let _exclusive = other.operations.write().unwrap();
            sent.send(()).unwrap();
        });
        assert!(received.recv_timeout(Duration::from_millis(30)).is_err());
        drop(reader);
        received.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn expired_preview_does_not_open_or_stop_runtime() {
        let backend = Backend::new();
        assert!(backend.restore_backup("expired").unwrap_err().contains("失效"));
        assert!(backend.runtime.lock().unwrap().is_none());
        assert!(!backend.launched.load(Ordering::Acquire));
    }
}
