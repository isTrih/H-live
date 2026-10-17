//! One bounded actor owns the synchronous runtime, including construction and drop.
use fs2::FileExt;
use h_live_runtime::{
    Runtime,
    storage::{self, Result, SecretProtector},
    transport::{LiveTransport, Transport},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(deny_unknown_fields)]
struct Saved {
    desired: BTreeSet<String>,
    exports: BTreeMap<String, Export>,
}
#[derive(Serialize, Deserialize, Clone)]
struct Export {
    file: String,
    name: String,
    created: i64,
    job: Value,
}
enum Command {
    Request(String, Value, oneshot::Sender<Result<Value>>),
    File(String, oneshot::Sender<Result<(PathBuf, String)>>),
    Shutdown(oneshot::Sender<Result<Value>>),
}
pub struct Control {
    tx: mpsc::SyncSender<Command>,
    join: Mutex<Option<thread::JoinHandle<()>>>,
    pub ready: Arc<AtomicBool>,
    pub instance: String,
}
struct Alive(Arc<AtomicBool>);
impl Drop for Alive {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Control {
    pub fn open(directory: PathBuf, secrets: Arc<dyn SecretProtector>) -> Result<Arc<Self>> {
        Self::open_with_transport(directory, secrets, Arc::new(LiveTransport))
    }
    pub fn open_with_transport(
        directory: PathBuf,
        secrets: Arc<dyn SecretProtector>,
        network: Arc<dyn Transport>,
    ) -> Result<Arc<Self>> {
        let (tx, rx) = mpsc::sync_channel(64);
        let (started, start) = mpsc::sync_channel(1);
        let ready = Arc::new(AtomicBool::new(true));
        let alive = ready.clone();
        let join = thread::Builder::new()
            .name("server-control".into())
            .spawn(move || {
                let _alive = Alive(alive);
                let mut actor = match Actor::open(directory, secrets, network) {
                    Ok(a) => {
                        let _ = started.send(Ok(()));
                        a
                    }
                    Err(e) => {
                        let _ = started.send(Err(e));
                        return;
                    }
                };
                let mut tick = Instant::now();
                loop {
                    match rx.recv_timeout(Duration::from_millis(500)) {
                        Ok(Command::Request(method, p, reply)) => {
                            let _ = reply.send(actor.request(&method, p));
                        }
                        Ok(Command::File(id, reply)) => {
                            let _ = reply.send(actor.file(&id));
                        }
                        Ok(Command::Shutdown(reply)) => {
                            let result = actor.close();
                            let _ = reply.send(result);
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            let _ = actor.close();
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    if tick.elapsed() >= Duration::from_secs(5) {
                        actor.tick();
                        tick = Instant::now();
                    }
                }
            })
            .map_err(|_| "无法创建服务控制线程")?;
        start.recv().map_err(|_| "服务初始化意外退出")??;
        Ok(Arc::new(Self {
            tx,
            join: Mutex::new(Some(join)),
            ready,
            instance: format!("{:032x}", rand::random::<u128>()),
        }))
    }
    pub async fn request(&self, method: &str, p: Value) -> Result<Value> {
        if !self.ready.load(Ordering::Acquire) {
            return Err("服务正在停止".into());
        }
        let (tx, rx) = oneshot::channel();
        self.tx
            .try_send(Command::Request(method.into(), p, tx))
            .map_err(|_| "服务繁忙，请稍后重试")?;
        rx.await.map_err(|_| "控制线程不可用".to_string())?
    }
    pub async fn file(&self, id: String) -> Result<(PathBuf, String)> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .try_send(Command::File(id, tx))
            .map_err(|_| "服务繁忙")?;
        rx.await.map_err(|_| "控制线程不可用".to_string())?
    }
    pub async fn shutdown(&self) -> Result<Value> {
        self.ready.store(false, Ordering::Release);
        let (tx, rx) = oneshot::channel();
        let sender = self.tx.clone();
        tokio::task::spawn_blocking(move || sender.send(Command::Shutdown(tx)))
            .await
            .map_err(|_| "控制线程收尾失败")?
            .map_err(|_| "控制线程已退出")?;
        let result = rx.await.map_err(|_| "控制线程意外退出".to_string())?;
        let join = self.join.lock().unwrap().take();
        if let Some(j) = join {
            tokio::task::spawn_blocking(move || j.join())
                .await
                .map_err(|_| "控制线程收尾失败")?
                .map_err(|_| "控制线程异常退出")?;
        }
        result
    }
}
struct Actor {
    runtime: Runtime,
    saved: Saved,
    state_path: PathBuf,
    exports: PathBuf,
    _lock: File,
    last_error: Option<String>,
}
impl Actor {
    fn open(
        directory: PathBuf,
        secrets: Arc<dyn SecretProtector>,
        network: Arc<dyn Transport>,
    ) -> Result<Self> {
        fs::create_dir_all(&directory).map_err(|_| "数据目录不可写")?;
        let directory = directory.canonicalize().map_err(|_| "数据目录无效")?;
        let private = directory.join(".server");
        let exports = directory.join("exports");
        for p in [&private, &exports] {
            if p.exists() && !fs::symlink_metadata(p).map_err(|_| "目录不可读")?.is_dir() {
                return Err("服务目录不能为符号链接或普通文件".into());
            }
            fs::create_dir_all(p).map_err(|_| "目录不可写")?;
        }
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(private.join("lock"))
            .map_err(|_| "数据锁不可写")?;
        lock.try_lock_exclusive()
            .map_err(|_| "此数据目录已被另一个服务占用")?;
        let state_path = private.join("state.json");
        let mut saved: Saved = if state_path.exists() {
            let info = fs::symlink_metadata(&state_path).map_err(|_| "服务状态不可读")?;
            if !info.is_file() || info.len() > 2 * 1024 * 1024 {
                return Err("服务状态文件无效".into());
            }
            serde_json::from_slice(&fs::read(&state_path).map_err(|_| "服务状态不可读")?)
                .map_err(|_| "服务状态已损坏，原文件已保留")?
        } else {
            Saved::default()
        };
        if saved.desired.len() > 500 || saved.exports.len() > 200 {
            return Err("服务状态超出容量限制".into());
        }
        for e in saved.exports.values_mut() {
            if e.job["status"] == "running" {
                e.job["status"] = json!("error");
                e.job["error"] = json!("服务重启，导出已中断，请重新导出");
            }
        }
        let runtime = Runtime::open_with_transport_and_secrets(directory, network, secrets)?;
        let mut a = Self {
            runtime,
            saved,
            state_path,
            exports,
            _lock: lock,
            last_error: None,
        };
        // Do not inherit desktop updater resume intent. Desired server tasks are authoritative.
        let running = a.runtime.running_tasks();
        if !running.is_empty() {
            a.runtime.request("stop", json!({"rids":running}))?;
        }
        a.reconcile()?;
        a.save()?;
        Ok(a)
    }
    fn save(&self) -> Result<()> {
        storage::atomic(
            &self.state_path,
            &serde_json::to_vec(&self.saved).map_err(|_| "状态编码失败")?,
        )?;
        #[cfg(unix)]
        File::open(self.state_path.parent().unwrap())
            .and_then(|f| f.sync_all())
            .map_err(|_| "状态目录同步失败")?;
        Ok(())
    }
    fn reconcile(&mut self) -> Result<()> {
        let known: BTreeSet<String> = self.runtime.configured_tasks().into_iter().collect();
        self.saved.desired.retain(|r| known.contains(r));
        let running = self.runtime.running_tasks();
        let missing: Vec<_> = self
            .saved
            .desired
            .iter()
            .filter(|r| !running.contains(r))
            .cloned()
            .collect();
        if !missing.is_empty() {
            self.runtime.request("start", json!({"rids":missing}))?;
        }
        Ok(())
    }
    fn refresh_jobs(&mut self) -> Result<()> {
        let p = self.runtime.request("poll", json!({"limit":1}))?;
        let mut changed = false;
        for job in p["export_jobs"].as_array().into_iter().flatten() {
            if let Some(e) = job["id"]
                .as_str()
                .and_then(|id| self.saved.exports.get_mut(id))
            {
                let mut clean = job.clone();
                if let Some(r) = clean["report"].as_object_mut() {
                    r.remove("path");
                }
                if e.job != clean {
                    e.job = clean;
                    changed = true;
                }
            }
        }
        let now = chrono::Utc::now().timestamp();
        let expired: Vec<_> = self
            .saved
            .exports
            .iter()
            .filter(|(_, e)| now - e.created > 7 * 86400 && e.job["status"] != "running")
            .map(|(id, e)| (id.clone(), e.file.clone()))
            .collect();
        for (id, file) in expired {
            self.saved.exports.remove(&id);
            if safe_file(&file) {
                let _ = fs::remove_file(self.exports.join(file));
            }
            changed = true;
        }
        if changed {
            self.save()?;
        }
        Ok(())
    }
    fn tick(&mut self) {
        self.last_error = self.reconcile().and_then(|_| self.refresh_jobs()).err();
    }
    fn jobs(&self) -> Value {
        json!(
            self.saved
                .exports
                .iter()
                .map(|(id, e)| {
                    let mut j = e.job.clone();
                    j["download_url"] = if j["status"] == "completed" {
                        json!(format!("/api/v1/files/{id}"))
                    } else {
                        Value::Null
                    };
                    j["file_name"] = json!(e.name);
                    j
                })
                .collect::<Vec<_>>()
        )
    }
    fn request(&mut self, method: &str, mut p: Value) -> Result<Value> {
        if ["start", "stop", "remove"].contains(&method) {
            let ids = p["rids"]
                .as_array()
                .filter(|a| a.len() <= 500)
                .ok_or("任务列表无效")?;
            let known = self.runtime.configured_tasks();
            let mut next = self.saved.desired.clone();
            for id in ids {
                let id = id
                    .as_str()
                    .filter(|s| storage::id(s) && known.iter().any(|a| a == *s))
                    .ok_or("任务不在主播列表中")?;
                if method == "start" {
                    next.insert(id.into());
                } else {
                    next.remove(id);
                }
            }
            let previous = std::mem::replace(&mut self.saved.desired, next);
            if let Err(e) = self.save() {
                self.saved.desired = previous;
                return Err(e);
            }
            // Intent remains durable even if runtime start fails; reconciliation retries it.
            return self.runtime.request(method, p);
        }
        if method == "export" || method == "quality_export" {
            self.refresh_jobs()?;
            if self.saved.exports.len() >= 100 {
                return Err("导出记录已达 100 条，请等待七天保留期清理".into());
            }
            let format = p["format"].as_str().unwrap_or("xlsx").to_owned();
            if !["xlsx", "csv"].contains(&format.as_str()) {
                return Err("导出格式无效".into());
            }
            let file = format!("{:032x}.{format}", rand::random::<u128>());
            let name = p
                .as_object_mut()
                .unwrap()
                .remove("file_name")
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("直播消息.{format}"));
            let name = safe_download_name(&name, &format);
            p["output_path"] = json!(self.exports.join(&file));
            let result = self.runtime.request(method, p)?;
            let id = result["id"].as_str().ok_or("导出任务编号无效")?.to_owned();
            self.saved.exports.insert(
                id.clone(),
                Export {
                    file,
                    name,
                    created: chrono::Utc::now().timestamp(),
                    job: json!({"id":id,"job_id":id,"status":"running"}),
                },
            );
            if let Err(e) = self.save() {
                let _ = self.runtime.request("cancel_export", json!({"job_id":id}));
                self.saved.exports.remove(&id);
                return Err(e);
            }
            return Ok(result);
        }
        if method == "poll" {
            self.refresh_jobs()?;
        }
        let mut result = self.runtime.request(method, p)?;
        if method == "poll" {
            result["export_jobs"] = self.jobs();
            result["controller_error"] = json!(self.last_error);
        }
        Ok(result)
    }
    fn file(&mut self, id: &str) -> Result<(PathBuf, String)> {
        self.refresh_jobs()?;
        let e = self
            .saved
            .exports
            .get(id)
            .filter(|e| e.job["status"] == "completed" && safe_file(&e.file))
            .ok_or("导出文件不存在或尚未完成")?;
        let p = self.exports.join(&e.file);
        if !fs::symlink_metadata(&p)
            .map_err(|_| "导出文件不存在")?
            .is_file()
        {
            return Err("导出文件无效".into());
        }
        Ok((p, e.name.clone()))
    }
    fn close(&mut self) -> Result<Value> {
        let result = self.runtime.shutdown();
        self.refresh_jobs().ok();
        self.save()?;
        result
    }
}
fn safe_file(s: &str) -> bool {
    let Some((stem, ext)) = s.rsplit_once('.') else {
        return false;
    };
    stem.len() == 32
        && stem.bytes().all(|b| b.is_ascii_hexdigit())
        && ["csv", "xlsx"].contains(&ext)
}
fn safe_download_name(s: &str, ext: &str) -> String {
    let name: String = s
        .chars()
        .filter(|c| !c.is_control() && !"/\\<>:\"|?*".contains(*c))
        .take(160)
        .collect();
    let stem = name
        .trim_end_matches(".csv")
        .trim_end_matches(".xlsx")
        .trim();
    format!(
        "{}.{ext}",
        if stem.is_empty() {
            "直播消息"
        } else {
            stem
        }
    )
}
