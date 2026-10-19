//! Small synchronous request interface over bounded async collectors and Rust archive.
use crate::{
    monitor,
    net::RoomInfo,
    proxy::{Pool, ProxyRecord},
    storage::{self, Result, Settings},
};
use futures_util::{stream, StreamExt};
use qingkan_core::{
    archive::{ArchiveOptions, ArchiveStore},
    export::CancelToken,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{sync::Semaphore, task::JoinHandle};
use tokio_util::sync::CancellationToken;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) struct Room {
    pub generation: u64,
    pub value: Value,
    pub route: Option<Arc<crate::proxy::Route>>,
    pub cancel: CancellationToken,
}
pub(crate) struct Job {
    pub value: Value,
    pub cancel: CancelToken,
}
pub(crate) struct State {
    pub config: Settings,
    pub rooms: BTreeMap<String, Room>,
    pub events: VecDeque<Value>,
    pub seq: u64,
    pub rates: VecDeque<u64>,
    pub pool: Pool,
    pub jobs: BTreeMap<String, Job>,
    pub deletions: BTreeMap<String, Job>,
    pub deleting: bool,
    pub subscriptions: BTreeMap<String, Value>,
    pub subscription_tokens: BTreeMap<String, CancellationToken>,
    pub refresh: Value,
}
pub(crate) struct Shared {
    pub threads: Arc<AtomicUsize>,
    pub network: Arc<dyn crate::transport::Transport>,
    pub state: Mutex<State>,
    pub archive: ArchiveStore,
    pub directory: PathBuf,
    pub cancel: CancellationToken,
    pub storage_slots: Arc<Semaphore>,
    pub polling: crate::polling::Polling,
    pub quality_cancel: CancelToken,
    pub quality_error: AtomicBool,
    pub jobs: Mutex<Vec<JoinHandle<()>>>,
    pub proxy_busy: AtomicBool,
    pub logs: Mutex<Option<std::sync::mpsc::SyncSender<String>>>,
}
impl Shared {
    pub fn bind_route(&self, rid: &str, generation: u64, route: Arc<crate::proxy::Route>) {
        if let Some(row) = self
            .state
            .lock()
            .unwrap()
            .rooms
            .get_mut(rid)
            .filter(|r| r.generation == generation)
        {
            row.route = Some(route)
        }
    }
    pub fn row(&self, rid: &str, generation: u64) -> Option<Value> {
        self.state
            .lock()
            .unwrap()
            .rooms
            .get(rid)
            .filter(|r| r.generation == generation)
            .map(|r| r.value.clone())
    }
    pub fn patch(&self, rid: &str, generation: u64, patch: Value) -> bool {
        let mut s = self.state.lock().unwrap();
        if let Some(row) = s.rooms.get_mut(rid).filter(|r| r.generation == generation) {
            for (k, v) in patch.as_object().unwrap() {
                row.value[k] = v.clone()
            }
            true
        } else {
            false
        }
    }
    pub fn patch_info(&self, rid: &str, generation: u64, info: &RoomInfo) -> bool {
        let mut s = self.state.lock().unwrap();
        let Some(row) = s.rooms.get_mut(rid).filter(|r| r.generation == generation) else {
            return false;
        };
        if row.value["name"]
            .as_str()
            .is_some_and(|n| n == rid || n == format!("主播 {rid}"))
            && !info.nickname.is_empty()
        {
            row.value["name"] = json!(info.nickname)
        }
        row.value["is_live"] = json!(info.is_live);
        row.value["live_checked_at"] = json!(storage::now());
        row.value["broadcast_started_at"] = json!(info.started_at);
        row.value["viewer_count"] = json!(info.viewer_count);
        if info.is_live {
            row.value["anchor_id"] = json!(info.anchor_id);
            row.value["session_id"] = json!(info.room_id);
            row.value["title"] = json!(info.title);
        }
        true
    }
    fn event_locked(s: &mut State, mut event: Value) {
        s.seq += 1;
        event["seq"] = json!(s.seq);
        if s.events.len() == 2000 {
            s.events.pop_front();
        }
        s.events.push_back(event);
    }
    pub fn status(&self, rid: &str, generation: u64, state: &str, detail: &str) {
        let mut s = self.state.lock().unwrap();
        let Some(row) = s.rooms.get_mut(rid).filter(|r| r.generation == generation) else {
            return;
        };
        if row.value["state"] == state && row.value["detail"] == detail {
            return;
        }
        row.value["state"] = json!(state);
        row.value["detail"] = json!(detail);
        let name = row.value["name"].clone();
        let event = json!({"kind":"status","rid":rid,"name":name,"state":state,"detail":detail,"timestamp":storage::now()});
        Self::event_locked(&mut s, event.clone());
        drop(s);
        self.write_log(&event);
    }
    pub fn log(&self, rid: &str, detail: &str) {
        let event = json!({"kind":"log","rid":rid,"detail":detail,"timestamp":storage::now()});
        Self::event_locked(&mut self.state.lock().unwrap(), event.clone());
        self.write_log(&event);
    }
    fn write_log(&self, event: &Value) {
        if let Some(sender) = self.logs.lock().unwrap().as_ref() {
            let _ = sender.try_send(event.to_string());
        }
    }
    pub fn message(
        &self,
        rid: &str,
        generation: u64,
        kind: &str,
        payload: &Value,
        time: &str,
        received: &str,
    ) {
        let mut s = self.state.lock().unwrap();
        let Some(row) = s
            .rooms
            .get_mut(rid)
            .filter(|r| r.generation == generation && r.value["active"] == true)
        else {
            return;
        };
        row.value["message_count"] = json!(row.value["message_count"].as_u64().unwrap_or(0) + 1);
        row.value["last_message_at"] = json!(received);
        let event = json!({"kind":"message","rid":rid,"name":row.value["name"],"anchor_id":row.value["anchor_id"],"session_id":row.value["session_id"],"msg_type":kind,"text":payload["text"].as_str().or_else(||payload["content"].as_str()).unwrap_or(""),"timestamp":received,"time_str":if time.len()>=19{&time[11..19]}else{""},"payload":payload});
        let now = crate::proxy::epoch_ms();
        s.rates.push_back(now);
        while s.rates.front().is_some_and(|t| *t + 1000 < now) || s.rates.len() > 10000 {
            s.rates.pop_front();
        }
        Self::event_locked(&mut s, event);
    }
    pub fn records(&self) -> Result<Vec<ProxyRecord>> {
        let s = self.state.lock().unwrap();
        records(&s.config.value)
    }
    pub fn flush_metadata(&self) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        let mut v = s.config.value.clone();
        for (rid, row) in &s.rooms {
            if v["anchors"].get(rid).is_none() {
                continue;
            }
            v["anchors"][rid] = row.value["name"].clone();
            if !v["anchor_metadata"].is_object() {
                v["anchor_metadata"] = json!({})
            }
            v["anchor_metadata"][rid] =
                json!({"anchor_id":row.value["anchor_id"],"session_id":row.value["session_id"]});
        }
        if v != s.config.value {
            s.config.replace(v)?
        }
        Ok(())
    }
    pub fn metrics(&self) -> Value {
        self.metrics_for(true)
    }
    pub(crate) fn metrics_for(&self, include_proxy: bool) -> Value {
        let archive = self.archive.metrics();
        let mut value = {
            let s = self.state.lock().unwrap();
            let (messages, active, connected) = s.rooms.values().fold(
                (0u64, 0usize, 0usize),
                |(messages, active, connected), r| {
                    (
                        messages + r.value["message_count"].as_u64().unwrap_or(0),
                        active + usize::from(r.value["active"] == true),
                        connected + usize::from(r.value["state"] == "connected"),
                    )
                },
            );
            let now = crate::proxy::epoch_ms();
            let mut v = json!({"total_messages":messages,"active":active,"connected":connected,"events_per_second":s.rates.iter().filter(|t|**t+1000>=now).count(),"thread_count":self.threads.load(Ordering::Acquire)+1+usize::from(archive["writer_alive"]==true),"operation_queue":0,"archive_queue":archive["queue_depth"],"archive":archive,"viewer_refresh":s.refresh,"runtime":"rust"});
            if include_proxy {
                v["proxy"] = proxy_snapshot(&s);
            }
            v
        };
        // OS accounting must not hold the collector's shared state lock.
        value["process_rss_mb"] = crate::process::counters()["rss_mb"].clone();
        value
    }
}
pub(crate) fn records(config: &Value) -> Result<Vec<ProxyRecord>> {
    let mut records = BTreeMap::new();
    for subscription in config["proxy_subscriptions"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if subscription["enabled"] == false {
            continue;
        }
        for v in subscription["records"].as_array().into_iter().flatten() {
            let record = ProxyRecord::from_value(v, "http")?;
            records.insert(record.id.clone(), record);
        }
    }
    for v in config["imported_proxies"].as_array().into_iter().flatten() {
        let record = ProxyRecord::from_value(v, "http")?;
        records.insert(record.id.clone(), record);
    }
    if records.len() > 5000 {
        return Err("合并代理超过 5000 条".into());
    }
    Ok(records.into_values().collect())
}
pub(crate) fn proxy_snapshot(s: &State) -> Value {
    let mut p = s.pool.snapshot();
    p["subscriptions"] = subscription_snapshot(s);
    p["include_direct"] = json!(s.pool.include_direct);
    p["leases"]=json!(s.rooms.iter().filter(|(_,r)|r.value["active"]==true).filter_map(|(rid,r)|r.route.as_ref().map(|route|json!({"rid":rid,"route_id":route.id,"kind":if route.proxy.is_some(){"proxy"}else{"direct"},"proxy":route.proxy.as_ref().map(|p|p.endpoint.clone()).unwrap_or("direct".into())}))).collect::<Vec<_>>());
    p
}
pub(crate) fn subscription_snapshot(s: &State) -> Value {
    json!(s.config.value["proxy_subscriptions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| {
            let mut p = s
                .subscriptions
                .get(v["id"].as_str().unwrap_or(""))
                .cloned()
                .unwrap_or(json!({"state":"idle","error":""}));
            for k in ["id", "interval_minutes", "enabled", "updated_at"] {
                p[k] = v[k].clone()
            }
            p["count"] = json!(v["records"].as_array().map(|a| a.len()).unwrap_or(0));
            p["label"] = json!(url::Url::parse(v["url"].as_str().unwrap_or(""))
                .map(|u| format!("{}{}", u.host_str().unwrap_or(""), u.path()))
                .unwrap_or_default());
            p
        })
        .collect::<Vec<_>>())
}
struct RoomTask {
    cancel: CancellationToken,
    handle: JoinHandle<()>,
}
pub struct Runtime {
    pub(crate) rt: tokio::runtime::Runtime,
    pub(crate) shared: Arc<Shared>,
    tasks: tokio::sync::Mutex<BTreeMap<String, RoomTask>>,
    pub(crate) action: Mutex<()>,
    generation: AtomicU64,
    pub(crate) quiescing: AtomicBool,
    shutdown_gate: Mutex<()>,
    closed: AtomicBool,
    background: Mutex<Vec<JoinHandle<()>>>,
    log_worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl Runtime {
    pub fn open(directory: PathBuf) -> Result<Self> {
        Self::open_with_transport(directory, Arc::new(crate::transport::LiveTransport))
    }
    pub fn open_with_transport(
        directory: PathBuf,
        network: Arc<dyn crate::transport::Transport>,
    ) -> Result<Self> {
        Self::open_with_transport_and_secrets(
            directory,
            network,
            Arc::new(storage::SystemProtector),
        )
    }
    pub fn open_with_transport_and_secrets(
        directory: PathBuf,
        network: Arc<dyn crate::transport::Transport>,
        secrets: Arc<dyn storage::SecretProtector>,
    ) -> Result<Self> {
        let config = Settings::load_with_protector(directory.clone(), secrets)?;
        records(&config.value)?;
        let archive = ArchiveStore::open(&directory, ArchiveOptions::default())
            .map_err(|_| "消息库打开或升级失败，原数据已保留")?;
        let threads = Arc::new(AtomicUsize::new(0));
        let starts = threads.clone();
        let stops = threads.clone();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .max_blocking_threads(16)
            .thread_name("h-live-io")
            .on_thread_start(move || {
                starts.fetch_add(1, Ordering::AcqRel);
            })
            .on_thread_stop(move || {
                stops.fetch_sub(1, Ordering::AcqRel);
            })
            .enable_all()
            .build()
            .map_err(|_| "无法启动原生任务运行时")?;
        let mut pool = Pool::new()?;
        pool.enabled = config.value["proxy_pool"]["enabled"]
            .as_bool()
            .unwrap_or(false);
        pool.include_direct = config.value["proxy_pool"]["include_direct"]
            .as_bool()
            .unwrap_or(true);
        pool.source = config.value["proxy_pool"]["source"]
            .as_str()
            .unwrap_or("public")
            .into();
        let (logs, receiver) = std::sync::mpsc::sync_channel::<String>(1024);
        let log_dir = directory.join("logs");
        let log_worker = std::thread::Builder::new()
            .name("h-live-log".into())
            .spawn(move || {
                use std::io::Write;
                let _ = std::fs::create_dir_all(&log_dir);
                let path = log_dir.join("runtime.jsonl");
                for line in receiver {
                    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 8 * 1024 * 1024) {
                        let _ = std::fs::rename(&path, log_dir.join("runtime.previous.jsonl"));
                    }
                    if let Ok(mut file) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                    {
                        let _ = writeln!(file, "{line}");
                    }
                }
            })
            .map_err(|_| "无法启动日志任务")?;
        let polling = crate::polling::Polling::open(&directory);
        let shared = Arc::new(Shared {
            threads,
            network,
            state: Mutex::new(State {
                config,
                rooms: BTreeMap::new(),
                events: VecDeque::new(),
                seq: 0,
                rates: VecDeque::new(),
                pool,
                jobs: BTreeMap::new(),
                deletions: BTreeMap::new(),
                deleting: false,
                subscriptions: BTreeMap::new(),
                subscription_tokens: BTreeMap::new(),
                refresh: json!({"id":"0","active":false}),
            }),
            archive,
            directory,
            cancel: CancellationToken::new(),
            storage_slots: Arc::new(Semaphore::new(8)),
            polling,
            quality_cancel: CancelToken::default(),
            quality_error: AtomicBool::new(false),
            jobs: Mutex::new(vec![]),
            proxy_busy: AtomicBool::new(false),
            logs: Mutex::new(Some(logs)),
        });
        let instance = Self {
            rt,
            shared,
            tasks: tokio::sync::Mutex::new(BTreeMap::new()),
            action: Mutex::new(()),
            generation: AtomicU64::new(0),
            quiescing: AtomicBool::new(false),
            shutdown_gate: Mutex::new(()),
            closed: AtomicBool::new(false),
            background: Mutex::new(vec![]),
            log_worker: Mutex::new(Some(log_worker)),
        };
        instance.background();
        instance.resume_update()?;
        Ok(instance)
    }
    fn background(&self) {
        let shared = self.shared.clone();
        self.background
            .lock()
            .unwrap()
            .push(self.rt.spawn(async move {
                shared.polling.run(&shared.cancel).await;
            }));
        let shared = self.shared.clone();
        self.background
            .lock()
            .unwrap()
            .push(self.rt.spawn(async move {
                while !shared.cancel.is_cancelled() {
                    let a = shared.archive.clone();
                    let c = shared.quality_cancel.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        a.quality_request("step", json!({"limit":128}), c)
                    })
                    .await;
                    shared
                        .quality_error
                        .store(!matches!(result, Ok(Ok(_))), Ordering::Release);
                    if !monitor::pause(0.15, &shared.cancel).await {
                        break;
                    }
                }
            }));
        let shared = self.shared.clone();
        self.background
            .lock()
            .unwrap()
            .push(self.rt.spawn(async move {
                let mut tick = 0u64;
                while monitor::pause(1., &shared.cancel).await {
                    tick += 1;
                    if tick.is_multiple_of(5) {
                        let clone = shared.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            let result = clone.flush_metadata();
                            if clone.polling.flush().is_err() {
                                clone.log("", "开播偏好缓存保存失败，下次继续重试");
                            }
                            result
                        })
                        .await;
                    }
                    crate::maintenance::tick(&shared, tick).await;
                }
            }));
    }
    pub(crate) fn spawn(&self, future: impl std::future::Future<Output = ()> + Send + 'static) {
        let mut jobs = self.shared.jobs.lock().unwrap();
        jobs.retain(|j| !j.is_finished());
        jobs.push(self.rt.spawn(future));
    }
    pub(crate) fn anchors(&self) -> Value {
        let s = self.shared.state.lock().unwrap();
        json!(s.config.value["anchors"].as_object().unwrap().iter().map(|(rid,name)|{let mut row=s.rooms.get(rid).map(|r|r.value.clone()).unwrap_or_else(||json!({"rid":rid,"name":name,"state":"stopped","detail":"尚未开始","active":false,"is_live":null,"anchor_id":s.config.value["anchor_metadata"][rid]["anchor_id"].as_str().unwrap_or(""),"session_id":s.config.value["anchor_metadata"][rid]["session_id"].as_str().unwrap_or(""),"message_count":0,"duration_running":false,"viewer_count":null,"route_kind":"direct"}));row["name"]=name.clone();let unsupported=s.config.value["pc_unsupported_rids"].as_array().is_some_and(|a|a.iter().any(|v|v.as_str()==Some(rid)));row["pc_unsupported"]=json!(unsupported);row["pc_support_source"]=json!(if unsupported{"manual"}else{""});row}).collect::<Vec<_>>())
    }
    pub(crate) fn start(&self, rids: Vec<String>) -> Result<Value> {
        if self.shared.state.lock().unwrap().deleting {
            return Err("消息删除仍在执行".into());
        }
        self.rt.block_on(async {
            let mut tasks = self.tasks.lock().await;
            for rid in &rids {
                if tasks.get(rid).is_some_and(|t| !t.handle.is_finished()) {
                    continue;
                }
                if let Some(task) = tasks.remove(rid) {
                    let _ = task.handle.await;
                }
                let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
                let cancel = self.shared.cancel.child_token();
                let value = {
                    let s = self.shared.state.lock().unwrap();
                    let Some(name) = s.config.value["anchors"].get(rid) else {
                        return Err("直播间不在列表中".into());
                    };
                    json!({"rid":rid,"name":name,"active":true,"state":"starting","detail":"正在排队启动","anchor_id":"","session_id":"","is_live":null,"message_count":0,"viewer_count":null,"duration_seconds":null,"duration_running":false})
                };
                self.shared.state.lock().unwrap().rooms.insert(rid.clone(), Room {
                    generation, value, route: None, cancel: cancel.clone(),
                });
                let handle = self.rt.spawn(monitor::run(self.shared.clone(), rid.clone(), generation, cancel.clone()));
                tasks.insert(rid.clone(), RoomTask { cancel, handle });
            }
            Ok(json!({"accepted":rids.len()}))
        })
    }
    pub(crate) fn stop(&self, rids: &[String]) -> Result<Value> {
        self.rt.block_on(async {
            let mut tasks = self.tasks.lock().await;
            for rid in rids {
                if let Some(task) = tasks.get(rid) {
                    task.cancel.cancel();
                }
            }
            let selected = rids
                .iter()
                .filter_map(|rid| tasks.remove(rid).map(|t| (rid.clone(), t)))
                .collect::<Vec<_>>();
            drop(tasks);
            let results = stream::iter(selected)
                .map(|(rid, mut task)| async move {
                    let stopped = tokio::time::timeout(Duration::from_secs(30), &mut task.handle)
                        .await
                        .is_ok();
                    (rid, task, stopped)
                })
                .buffer_unordered(50)
                .collect::<Vec<_>>()
                .await;
            let mut tasks = self.tasks.lock().await;
            let mut complete = true;
            for (rid, task, stopped) in results {
                if !stopped {
                    tasks.insert(rid, task);
                    complete = false;
                }
            }
            if complete {
                Ok(json!({"accepted":rids.len()}))
            } else {
                Err("部分任务仍在退出或写入；未确认停止，不能重新开始或更新".into())
            }
        })
    }
    pub fn update_readiness(&self) -> Value {
        let s = self.shared.state.lock().unwrap();
        if s.deleting
            || s.refresh["active"] == true
            || s.subscriptions.values().any(|v| v["state"] == "refreshing")
        {
            return json!({"ready":false,"reason":"pending"});
        }
        if s.jobs.values().any(|j| j.value["status"] == "running") {
            return json!({"ready":false,"reason":"exporting"});
        }
        if s.rooms.values().any(|r| r.value["active"] == true) {
            return json!({"ready":false,"reason":"monitoring"});
        }
        json!({"ready":true,"reason":"ready"})
    }
    /// Actual live task handles, independent from last published UI state.
    pub fn running_tasks(&self) -> Vec<String> {
        self.rt.block_on(async {
            self.tasks
                .lock()
                .await
                .iter()
                .filter(|(_, t)| !t.handle.is_finished())
                .map(|(rid, _)| rid.clone())
                .collect()
        })
    }
    /// Configuration lookup does not require opening an archive read snapshot.
    pub fn configured_tasks(&self) -> Vec<String> {
        self.shared.state.lock().unwrap().config.value["anchors"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    }
    pub(crate) fn prepare_update(&self, p: &Value) -> Result<Value> {
        {
            let s = self.shared.state.lock().unwrap();
            if s.deleting || s.jobs.values().any(|j| j.value["status"] == "running") {
                return Ok(json!({"ready":false,"reason":"exporting"}));
            }
        }
        let target = p["target_version"]
            .as_str()
            .filter(|s| {
                s.len() < 80
                    && s.split('.').count() >= 3
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b".+-".contains(&b))
            })
            .ok_or("更新版本无效")?;
        let rids = {
            let s = self.shared.state.lock().unwrap();
            s.rooms
                .iter()
                .filter(|(_, r)| r.value["active"] == true)
                .map(|(rid, _)| rid.clone())
                .collect::<Vec<_>>()
        };
        let intent = json!({"format":1,"from_version":VERSION,"target_version":target,"rids":rids,"nonce":format!("{:032x}",rand::random::<u128>())});
        storage::atomic(
            &self.shared.directory.join("update-resume.json"),
            &serde_json::to_vec(&intent).unwrap(),
        )?;
        self.quiescing.store(true, Ordering::Release);
        if let Err(error) = self.stop(&rids) {
            self.quiescing.store(false, Ordering::Release);
            return Err(error);
        }
        self.shared.flush_metadata()?;
        if self.shared.polling.flush().is_err() {
            self.shared
                .log("", "开播偏好缓存保存失败，消息归档不受影响");
        }
        Ok(json!({"ready":true,"reason":"ready","resume_count":rids.len()}))
    }
    pub(crate) fn resume_update(&self) -> Result<Value> {
        let path = self.shared.directory.join("update-resume.json");
        if !path.exists() {
            return Ok(json!({"recovery_queued":false}));
        }
        if std::fs::metadata(&path)
            .map_err(|_| "更新恢复记录不可读")?
            .len()
            > 32768
        {
            return Err("更新恢复记录无效".into());
        }
        let v: Value =
            serde_json::from_slice(&std::fs::read(&path).map_err(|_| "更新恢复记录不可读")?)
                .map_err(|_| "更新恢复记录无效")?;
        if v["format"] != 1 || v["from_version"] != VERSION && v["target_version"] != VERSION {
            return Ok(json!({"recovery_queued":false}));
        }
        let rids = id_list(&v["rids"], true)?;
        let rids = {
            let s = self.shared.state.lock().unwrap();
            rids.into_iter()
                .filter(|r| s.config.value["anchors"].get(r).is_some())
                .collect::<Vec<_>>()
        };
        self.start(rids)?;
        std::fs::remove_file(path).map_err(|_| "更新恢复确认失败")?;
        Ok(json!({"recovery_queued":true}))
    }
    pub fn shutdown(&self) -> Result<Value> {
        let _shutdown = self.shutdown_gate.lock().map_err(|_| "退出状态不可用")?;
        let _action = self.action.lock().map_err(|_| "后台状态不可用")?;
        self.quiescing.store(true, Ordering::Release);
        if self.closed.load(Ordering::Acquire) {
            return Ok(json!({"closed":true,"all_workers_stopped":true}));
        }
        self.shared.cancel.cancel();
        self.shared.quality_cancel.cancel();
        let ids = self
            .rt
            .block_on(async { self.tasks.lock().await.keys().cloned().collect::<Vec<_>>() });
        self.stop(&ids)?;
        {
            let s = self.shared.state.lock().unwrap();
            for j in s.jobs.values().chain(s.deletions.values()) {
                j.cancel.cancel();
            }
        }
        let jobs = std::mem::take(&mut *self.shared.jobs.lock().unwrap());
        let background = std::mem::take(&mut *self.background.lock().unwrap());
        self.rt.block_on(async {
            let mut pending = jobs.into_iter().chain(background).collect::<Vec<_>>();
            while let Some(mut job) = pending.pop() {
                if tokio::time::timeout(Duration::from_secs(30), &mut job)
                    .await
                    .is_err()
                {
                    pending.push(job);
                    self.shared.jobs.lock().unwrap().extend(pending);
                    return Err("后台任务仍在收尾，未确认退出".into());
                }
            }
            Ok::<_, String>(())
        })?;
        self.shared.flush_metadata()?;
        if self.shared.polling.flush().is_err() {
            self.shared
                .log("", "开播偏好缓存保存失败，消息归档不受影响");
        }
        if !self.shared.archive.close(Duration::from_secs(15)) {
            return Err("归档尚未完成，未确认退出".into());
        }
        self.shared.logs.lock().unwrap().take();
        if let Some(worker) = self.log_worker.lock().unwrap().take() {
            let _ = worker.join();
        }
        self.closed.store(true, Ordering::Release);
        Ok(json!({"closed":true,"all_workers_stopped":true}))
    }
    pub(crate) fn ensure_open(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire)
            || self.quiescing.load(Ordering::Acquire)
            || self.shared.cancel.is_cancelled()
        {
            Err("应用正在退出".into())
        } else {
            Ok(())
        }
    }
}
pub(crate) fn id_list(v: &Value, empty: bool) -> Result<Vec<String>> {
    let a = v
        .as_array()
        .filter(|a| a.len() <= 500 && (empty || !a.is_empty()))
        .ok_or("请选择最多 500 个有效 ID")?;
    let mut out = vec![];
    for v in a {
        let s = v.as_str().filter(|s| storage::id(s)).ok_or("ID 无效")?;
        if !out.iter().any(|v| v == s) {
            out.push(s.into())
        }
    }
    Ok(out)
}

impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
