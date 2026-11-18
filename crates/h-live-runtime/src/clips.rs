//! Independent single-worker clip queue. Persistent records contain no remote URLs or keys.
use crate::{
    clip_naming::{self, NamingContext, NamingSettings},
    runtime::{Runtime, Shared},
    storage::{self, Result},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ClipJob {
    #[serde(default = "clip_mode")]
    pub mode: String,
    pub id: String,
    pub session_id: String,
    pub title: String,
    pub status: String,
    pub phase: String,
    pub progress: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub output_path: String,
    pub error: Option<String>,
    pub requested_start_seconds: f64,
    pub requested_end_seconds: f64,
    pub actual_start_seconds: Option<f64>,
    pub actual_end_seconds: Option<f64>,
    pub attempt: u32,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default = "manual_source")]
    pub source: String,
    #[serde(default)]
    pub bookmark_id: Option<String>,
    #[serde(default)]
    pub request_key: Option<String>,
    #[serde(default)]
    pub anchor_id: String,
    #[serde(default)]
    pub anchor_name: String,
    #[serde(default)]
    pub session_date: String,
    #[serde(default)]
    pub source_identity: Option<String>,
    #[serde(default)]
    pub source_fingerprint: Option<String>,
}
fn manual_source() -> String {
    "manual".into()
}
fn clip_mode() -> String {
    "clip".into()
}
struct Entry {
    job: ClipJob,
    cancel: CancellationToken,
}
#[derive(Clone)]
pub(crate) struct MediaTools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}
pub(crate) struct ClipService {
    pub shared: Arc<Shared>,
    entries: Mutex<Vec<Entry>>,
    history: Result<crate::clip_history::ClipHistory>,
    tools: Mutex<Option<MediaTools>>,
    notify: Notify,
}
impl ClipService {
    pub(crate) fn open(shared: Arc<Shared>) -> Arc<Self> {
        let opened =
            crate::clip_history::ClipHistory::open(&shared.directory).and_then(|history| {
                history.interrupt_active()?;
                let jobs = history.recent(100)?;
                Ok((history, jobs))
            });
        let (history, mut jobs) = match opened {
            Ok((history, jobs)) => (Ok(history), jobs),
            Err(error) => (
                Err(format!(
                    "下载历史暂不可用：{error}。原数据已保留，请检查数据目录后重启。"
                )),
                vec![],
            ),
        };
        jobs.reverse();
        let tools = std::env::var_os("QINGKAN_MEDIA_TOOLS_DIR")
            .and_then(|p| tools_from_directory(Path::new(&p)).ok())
            .or_else(|| {
                #[cfg(debug_assertions)]
                {
                    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent()?.parent()?;
                    tools_from_directory(&root.join("packaging/tools/media")).ok()
                }
                #[cfg(not(debug_assertions))]
                {
                    None
                }
            });
        Arc::new(Self {
            shared,
            entries: Mutex::new(
                jobs.into_iter()
                    .map(|job| Entry {
                        job,
                        cancel: CancellationToken::new(),
                    })
                    .collect(),
            ),
            history,
            tools: Mutex::new(tools),
            notify: Notify::new(),
        })
    }
    fn history(&self) -> Result<&crate::clip_history::ClipHistory> {
        self.history.as_ref().map_err(Clone::clone)
    }
    pub fn set_tools(&self, directory: &Path) -> Result<()> {
        *self.tools.lock().map_err(|_| "媒体工具状态不可用")? =
            Some(tools_from_directory(directory)?);
        Ok(())
    }
    pub(crate) fn tools(&self) -> Result<MediaTools> {
        self.history()?;
        self.tools
            .lock()
            .map_err(|_| "媒体工具状态不可用")?
            .clone()
            .ok_or_else(|| "请先安装媒体工具组件，再创建切片".into())
    }
    pub(crate) fn update(&self, id: &str, change: impl FnOnce(&mut ClipJob)) {
        if let Ok(mut entries) = self.entries.lock() {
            if let Some(entry) = entries.iter_mut().find(|e| e.job.id == id) {
                let before = entry.job.status.clone();
                let identity = entry.job.source_identity.clone();
                let fingerprint = entry.job.source_fingerprint.clone();
                let output = entry.job.output_path.clone();
                let end = entry.job.requested_end_seconds;
                change(&mut entry.job);
                entry.job.updated_at = storage::now();
                // Fractional progress and byte counters are transient. A queued,
                // running or terminal state change is the durable checkpoint.
                if before != entry.job.status
                    || identity != entry.job.source_identity
                    || fingerprint != entry.job.source_fingerprint
                    || output != entry.job.output_path
                    || end != entry.job.requested_end_seconds
                {
                    if let Ok(history) = self.history() {
                        let _ = history.put(&entry.job);
                    }
                }
            }
        }
    }
    pub(crate) fn busy(&self) -> bool {
        self.entries.lock().is_ok_and(|entries| {
            entries
                .iter()
                .any(|e| matches!(e.job.status.as_str(), "queued" | "running"))
        })
    }
    pub(crate) fn cancel_all(&self) {
        let mut changed = Vec::new();
        if let Ok(mut entries) = self.entries.lock() {
            for entry in entries.iter_mut() {
                entry.cancel.cancel();
                if entry.job.status == "queued" {
                    entry.job.status = "cancelled".into();
                    entry.job.phase = "cancelled".into();
                    entry.job.updated_at = storage::now();
                    changed.push(entry.job.clone());
                }
            }
        }
        if let Ok(history) = self.history() {
            let _ = history.put_many(&changed);
        }
        self.notify.notify_one();
    }
    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> Result<Value> {
        self.query(&json!({}))
    }
    pub(crate) fn query(&self, params: &Value) -> Result<Value> {
        let mut page = self.history()?.query(params)?;
        let entries = self.entries.lock().map_err(|_| "切片任务状态不可用")?;
        for job in &mut page.jobs {
            if let Some(entry) = entries.iter().find(|e| e.job.id == job.id) {
                *job = entry.job.clone();
            }
        }
        Ok(page.value())
    }
    pub(crate) fn job(&self, id: &str) -> Result<ClipJob> {
        let entries = self.entries.lock().map_err(|_| "切片任务状态不可用")?;
        if let Some(entry) = entries.iter().find(|e| e.job.id == id) {
            return Ok(entry.job.clone());
        }
        self.history()?
            .get(id)?
            .ok_or_else(|| "找不到切片任务".into())
    }
    fn create(&self, mut p: Value) -> Result<Value> {
        self.tools()?;
        if self
            .shared
            .state
            .lock()
            .map_err(|_| "社区状态不可用")?
            .config
            .value["community_enabled"]
            != true
        {
            return Err("请先在设置中开启社区生态".into());
        }
        let session = p["session_id"]
            .as_str()
            .filter(|s| storage::id(s))
            .ok_or("场次 ID 无效")?
            .to_owned();
        let mode = match p.get("mode") {
            None => "clip",
            Some(v) => v.as_str().ok_or("下载模式无效")?,
        };
        if !matches!(mode, "clip" | "full_session") {
            return Err("下载模式无效".into());
        }
        let full = mode == "full_session";
        if !full && p["source"] == "full_session" {
            return Err("整场来源必须使用整场下载模式".into());
        }
        if full
            && (p.get("start_seconds").is_some()
                || p.get("end_seconds").is_some()
                || p.get("bookmark_id").is_some_and(|v| !v.is_null()))
        {
            return Err("整场下载不接受切片起止时间或打点".into());
        }
        let mode = mode.to_owned();
        if full {
            p["source"] = json!("full_session");
        }
        let start = if full {
            0.
        } else {
            p["start_seconds"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.)
                .ok_or("切片起点无效")?
        };
        let end = if full {
            0.
        } else {
            p["end_seconds"]
                .as_f64()
                .filter(|n| n.is_finite() && *n > start && *n - start >= 0.1 && *n - start <= 900.)
                .ok_or("切片长度须为 0.1 秒至 15 分钟")?
        };
        let title = p["title"]
            .as_str()
            .unwrap_or(if full { "整场直播" } else { "直播切片" })
            .trim();
        if title.chars().count() > 120 || title.chars().any(char::is_control) {
            return Err("切片标题无效".into());
        }
        let request_key = p
            .get("request_key")
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_str()
                    .filter(|s| {
                        !s.is_empty()
                            && s.len() <= 160
                            && s.bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"-_: .".contains(&b))
                    })
                    .map(str::to_owned)
                    .ok_or_else(|| "切片请求标识无效".to_owned())
            })
            .transpose()?;
        let id = format!("{:032x}", rand::random::<u128>());
        let now = storage::now();
        let context =
            NamingContext::for_clip(&self.shared, &p, &session, title, start, end, &id, &now)?;
        let naming = NamingSettings::load(&self.shared)?;
        let mut entries = self.entries.lock().map_err(|_| "切片任务状态不可用")?;
        if let Some(key) = &request_key {
            if let Some(job) = self.history()?.by_request(key)? {
                let job = entries
                    .iter()
                    .find(|e| e.job.id == job.id)
                    .map(|e| e.job.clone())
                    .unwrap_or(job);
                if job.session_id != session
                    || job.mode != mode
                    || job.requested_start_seconds != start
                    || !full && job.requested_end_seconds != end
                    || job.bookmark_id != context.bookmark_id
                    || job.source != context.source
                {
                    return Err("切片请求标识已用于另一段内容".into());
                }
                return Ok(json!({"job_id":job.id,"job":job,"existing":true}));
            }
        }
        if entries
            .iter()
            .filter(|e| matches!(e.job.status.as_str(), "queued" | "running"))
            .count()
            >= 10
        {
            return Err("最多同时保留 10 个待处理切片，请等待或取消任务".into());
        }
        let output = match p.get("output_path").filter(|v| !v.is_null()) {
            Some(value) => output_path(value.as_str().ok_or("切片保存位置无效")?)?,
            None if full => PathBuf::new(),
            None => clip_naming::target(&naming, &context, &self.shared.directory, |candidate| {
                self.history()?.has_path(candidate)
            })?,
        };
        let job = ClipJob {
            mode,
            id: id.clone(),
            session_id: session.into(),
            title: title.into(),
            status: "queued".into(),
            phase: "queued".into(),
            progress: 0.,
            downloaded_bytes: 0,
            total_bytes: None,
            output_path: output.to_string_lossy().into(),
            error: None,
            requested_start_seconds: start,
            requested_end_seconds: end,
            actual_start_seconds: None,
            actual_end_seconds: None,
            attempt: 1,
            created_at: now.clone(),
            updated_at: now,
            source: context.source,
            bookmark_id: context.bookmark_id,
            request_key,
            anchor_id: context.anchor_id,
            anchor_name: context.anchor_name,
            session_date: context.session_date,
            source_identity: None,
            source_fingerprint: None,
        };
        if entries.iter().any(|e| {
            !job.output_path.is_empty()
                && matches!(e.job.status.as_str(), "queued" | "running")
                && clip_naming::same_path(&e.job.output_path, Path::new(&job.output_path))
        }) {
            return Err("该保存位置已有排队切片".into());
        }
        while entries.len() >= 100 {
            if let Some(index) = entries
                .iter()
                .position(|e| !matches!(e.job.status.as_str(), "queued" | "running"))
            {
                entries.remove(index);
            } else {
                break;
            }
        }
        self.history()?.put(&job)?;
        entries.push(Entry {
            job: job.clone(),
            cancel: CancellationToken::new(),
        });
        drop(entries);
        self.notify.notify_one();
        Ok(json!({"job_id":id,"job":job}))
    }
    pub(crate) fn resolve_full_session(&self, job: &ClipJob, duration: f64) -> Result<ClipJob> {
        if job.mode != "full_session"
            || !duration.is_finite()
            || duration <= 0.
            || duration > 31. * 86400.
        {
            return Err("整场录像时长无效".into());
        }
        // A refreshed signed URL must retain the path already reserved by this
        // task, even when the worker began with an unresolved queued snapshot.
        let mut resolved = self.job(&job.id)?;
        resolved.requested_start_seconds = 0.;
        resolved.requested_end_seconds = duration;
        if resolved.output_path.is_empty() {
            let context = NamingContext {
                anchor_id: job.anchor_id.clone(),
                anchor_name: job.anchor_name.clone(),
                session_id: job.session_id.clone(),
                session_date: job.session_date.clone(),
                title: job.title.clone(),
                source: "full_session".into(),
                bookmark_id: None,
                start: 0.,
                end: duration,
                job_id: job.id.clone(),
                created_at: job.created_at.clone(),
            };
            let output = clip_naming::target(
                &NamingSettings::load(&self.shared)?,
                &context,
                &self.shared.directory,
                |candidate| self.history()?.has_path(candidate),
            )?;
            resolved.output_path = output.to_string_lossy().into();
        }
        if Path::new(&resolved.output_path).exists() {
            return Err("目标文件已存在，不会覆盖".into());
        }
        self.update(&job.id, |record| {
            record.requested_end_seconds = duration;
            record.output_path = resolved.output_path.clone();
        });
        Ok(resolved)
    }
    fn cancel(&self, id: &str) -> Result<Value> {
        self.history()?;
        let job = {
            let mut entries = self.entries.lock().map_err(|_| "切片任务状态不可用")?;
            let entry = entries
                .iter_mut()
                .find(|e| e.job.id == id)
                .ok_or("找不到切片任务")?;
            if matches!(entry.job.status.as_str(), "queued" | "running") {
                entry.cancel.cancel();
                if entry.job.status == "queued" {
                    entry.job.status = "cancelled".into();
                    entry.job.phase = "cancelled".into();
                } else {
                    entry.job.phase = "cancelling".into();
                }
            }
            self.history()?.put(&entry.job)?;
            entry.job.clone()
        };
        self.notify.notify_one();
        Ok(json!({"job_id":id,"job":job}))
    }
    fn retry(&self, id: &str) -> Result<Value> {
        self.tools()?;
        if self
            .shared
            .state
            .lock()
            .map_err(|_| "社区状态不可用")?
            .config
            .value["community_enabled"]
            != true
        {
            return Err("请先在设置中开启社区生态".into());
        }
        let job = {
            let mut entries = self.entries.lock().map_err(|_| "切片任务状态不可用")?;
            if entries
                .iter()
                .filter(|e| matches!(e.job.status.as_str(), "queued" | "running"))
                .count()
                >= 10
            {
                return Err("切片队列已满".into());
            }
            let mut job = entries
                .iter()
                .find(|e| e.job.id == id)
                .map(|e| e.job.clone())
                .or(self.history()?.get(id)?)
                .ok_or("找不到切片任务")?;
            if !matches!(job.status.as_str(), "failed" | "cancelled") {
                return Err("仅失败或已取消切片可重试".into());
            }
            if Path::new(&job.output_path).exists() {
                return Err("目标文件已存在，请保留原文件并重新选择保存位置".into());
            }
            if entries.iter().any(|e| {
                !job.output_path.is_empty()
                    && e.job.id != id
                    && matches!(e.job.status.as_str(), "queued" | "running")
                    && clip_naming::same_path(&e.job.output_path, Path::new(&job.output_path))
            }) {
                return Err("该保存位置已有排队切片".into());
            }
            job.status = "queued".into();
            job.phase = "queued".into();
            job.error = None;
            job.progress = 0.;
            job.downloaded_bytes = 0;
            job.total_bytes = None;
            job.updated_at = storage::now();
            job.attempt = job.attempt.saturating_add(1);
            self.history()?.put(&job)?;
            if let Some(entry) = entries.iter_mut().find(|e| e.job.id == id) {
                entry.job = job.clone();
                entry.cancel = CancellationToken::new();
            } else {
                if entries.len() >= 100 {
                    if let Some(index) = entries
                        .iter()
                        .position(|e| !matches!(e.job.status.as_str(), "queued" | "running"))
                    {
                        entries.remove(index);
                    }
                }
                entries.push(Entry {
                    job: job.clone(),
                    cancel: CancellationToken::new(),
                });
            }
            job
        };
        self.notify.notify_one();
        Ok(json!({"job_id":id,"job":job}))
    }
    pub(crate) async fn worker(self: Arc<Self>) {
        let Ok(history) = self.history() else {
            return;
        };
        loop {
            if self.shared.cancel.is_cancelled() {
                self.cancel_all();
                break;
            }
            let next = {
                let mut entries = self.entries.lock().unwrap();
                entries.iter_mut().find_map(|entry| {
                    if entry.job.status != "queued" {
                        return None;
                    }
                    entry.job.status = "running".into();
                    entry.job.phase = "resolving".into();
                    if history.put(&entry.job).is_err() {
                        entry.job.status = "failed".into();
                        entry.job.phase = "failed".into();
                        entry.job.error = Some("无法保存下载状态，请检查数据目录后重试".into());
                        let _ = history.put(&entry.job);
                        return None;
                    }
                    Some((entry.job.clone(), entry.cancel.clone()))
                })
            };
            let Some((job, cancel)) = next else {
                tokio::select! {_=self.shared.cancel.cancelled()=>{},_=self.notify.notified()=>{}}
                continue;
            };
            let result = crate::clip_worker::run(&self, &job, &cancel).await;
            self.update(&job.id, |record| match result {
                Ok((start, end)) => {
                    record.status = "completed".into();
                    record.phase = "completed".into();
                    record.progress = 100.;
                    record.actual_start_seconds = Some(start);
                    record.actual_end_seconds = Some(end);
                    record.error = None;
                }
                Err(error) => {
                    let cancelled = cancel.is_cancelled() || self.shared.cancel.is_cancelled();
                    record.status = if cancelled { "cancelled" } else { "failed" }.into();
                    record.phase = record.status.clone();
                    record.error = if cancelled { None } else { Some(error) };
                }
            });
        }
    }
}
fn tools_from_directory(directory: &Path) -> Result<MediaTools> {
    let directory = std::fs::canonicalize(directory).map_err(|_| "媒体工具目录不可用")?;
    let executable = |name: &str| -> Result<PathBuf> {
        let path = directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        let path = std::fs::canonicalize(path).map_err(|_| "媒体工具组件不完整")?;
        if path.parent() != Some(directory.as_path()) || !path.is_file() {
            return Err("媒体工具路径无效".into());
        }
        Ok(path)
    };
    Ok(MediaTools {
        ffmpeg: executable("ffmpeg")?,
        ffprobe: executable("ffprobe")?,
    })
}
fn output_path(raw: &str) -> Result<PathBuf> {
    let path = Path::new(raw);
    if !path.is_absolute()
        || path
            .extension()
            .is_none_or(|s| !s.eq_ignore_ascii_case("mp4"))
        || raw.chars().any(char::is_control)
        || raw.len() > 4096
    {
        return Err("请选择完整的 MP4 保存路径".into());
    }
    let parent = std::fs::canonicalize(path.parent().ok_or("保存目录无效")?)
        .map_err(|_| "切片保存目录不存在")?;
    let filename = path.file_name().ok_or("切片文件名无效")?;
    clip_naming::validate_explicit_filename(filename.to_str().ok_or("切片文件名无效")?)?;
    let output = parent.join(filename);
    if output.exists() {
        return Err("目标文件已存在；不会覆盖已有文件".into());
    }
    Ok(output)
}
impl Runtime {
    pub fn configure_media_tools(&self, directory: PathBuf) -> Result<()> {
        self.ensure_open()?;
        self.clips.set_tools(&directory)
    }
    pub(crate) fn clip_request(&self, method: &str, p: Value) -> Result<Value> {
        match method {
            "clip_tools" => Ok(match self.clips.tools() {
                Ok(_) => {
                    json!({"available":true,"mode":"copy","max_duration_seconds":900,"full_session_available":true,"max_full_session_duration_seconds":31*86400,"max_full_session_bytes":64_u64*1024*1024*1024,"supports":["hls_vod","full_session"]})
                }
                Err(error) => json!({"available":false,"error":error}),
            }),
            "clip_jobs" => self.clips.query(&p),
            "clip_job" => {
                Ok(json!({"job":self.clips.job(p["job_id"].as_str().ok_or("缺少切片任务 ID")?)?}))
            }
            "clip_create" => {
                let mut p = p;
                if p["source"] == "bookmark" {
                    let mark = self
                        .session_marks
                        .get(p["bookmark_id"].as_str().ok_or("缺少打点 ID")?)?;
                    if p["session_id"] != mark.session_id
                        || p.get("anchor_id")
                            .is_some_and(|a| !a.is_null() && a != &mark.anchor_id)
                    {
                        return Err("打点与直播场次不一致".into());
                    }
                    p["anchor_id"] = json!(mark.anchor_id);
                    p["anchor_name"] = json!(mark.anchor_name);
                    p["session_date"] = json!(mark
                        .broadcast_started_at
                        .and_then(|seconds| chrono::DateTime::from_timestamp(
                            seconds.floor() as i64,
                            0
                        ))
                        .map(|time| time
                            .with_timezone(&qingkan_core::archive::shanghai())
                            .format("%Y-%m-%d")
                            .to_string())
                        .unwrap_or_else(|| mark.marked_at.chars().take(10).collect()));
                    if p["title"]
                        .as_str()
                        .is_none_or(|title| title.trim().is_empty())
                    {
                        p["title"] = json!(if mark.title.is_empty() {
                            "直播打点".into()
                        } else {
                            mark.title
                        });
                    }
                } else if p.get("bookmark_id").is_some_and(|id| !id.is_null()) {
                    return Err("打点切片须使用打点来源".into());
                }
                self.clips.create(p)
            }
            "clip_cancel" => self
                .clips
                .cancel(p["job_id"].as_str().ok_or("缺少切片任务 ID")?),
            "clip_retry" => self
                .clips
                .retry(p["job_id"].as_str().ok_or("缺少切片任务 ID")?),
            _ => Err("未知切片操作".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Protector;
    impl crate::storage::SecretProtector for Protector {
        fn protect(&self, bytes: &[u8]) -> Result<Vec<u8>> {
            Ok(bytes.iter().map(|b| b ^ 0xa5).collect())
        }
        fn unprotect(&self, bytes: &[u8]) -> Result<Vec<u8>> {
            self.protect(bytes)
        }
    }
    fn setup() -> (tempfile::TempDir, Runtime, Arc<ClipService>) {
        let directory = tempfile::tempdir().unwrap();
        let runtime = Runtime::open_with_transport_and_secrets(
            directory.path().into(),
            Arc::new(crate::transport::LiveTransport),
            Arc::new(Protector),
        )
        .unwrap();
        runtime
            .request(
                "community_configure",
                json!({"enabled":true,"api_key":"synthetic-clip-key"}),
            )
            .unwrap();
        let tools = directory.path().join("tools");
        std::fs::create_dir(&tools).unwrap();
        for name in ["ffmpeg", "ffprobe"] {
            std::fs::write(
                tools.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)),
                [],
            )
            .unwrap();
        }
        let service = ClipService::open(runtime.shared.clone());
        service.set_tools(&tools).unwrap();
        (directory, runtime, service)
    }
    #[test]
    fn queue_cancel_retry_and_restart_do_not_persist_provider_secrets() {
        let (_directory, runtime, service) = setup();
        let created = service
            .create(json!({"session_id":"123","start_seconds":1.,"end_seconds":8.}))
            .unwrap();
        let id = created["job_id"].as_str().unwrap();
        assert_eq!(service.cancel(id).unwrap()["job"]["status"], "cancelled");
        assert_eq!(service.retry(id).unwrap()["job"]["attempt"], 2);
        let restored = ClipService::open(runtime.shared.clone());
        let jobs = restored.snapshot().unwrap();
        assert_eq!(jobs["jobs"][0]["status"], "failed");
        assert_eq!(jobs["jobs"][0]["phase"], "interrupted");
        let disk =
            serde_json::to_string(&restored.history().unwrap().recent(100).unwrap()).unwrap();
        assert!(!disk.contains("synthetic-clip-key"));
        assert!(!disk.contains("https://"));
        service.cancel_all();
        runtime.shutdown().unwrap();
    }
    #[test]
    fn full_session_queues_without_fake_end_then_resolves_naming_from_real_vod_duration() {
        let (directory, runtime, service) = setup();
        runtime.request("download_naming_save", json!({"root_directory":directory.path(),"directory_template":"{source}/{anchor}","filename_template":"{session_id}_{duration}"})).unwrap();
        let request = json!({"mode":"full_session","session_id":"123","anchor_name":"测试主播","request_key":"full:123:first"});
        let created = service.create(request.clone()).unwrap();
        let id = created["job_id"].as_str().unwrap();
        assert_eq!(created["job"]["requested_end_seconds"], 0.);
        assert_eq!(created["job"]["output_path"], "");
        let resolved = service
            .resolve_full_session(&service.job(id).unwrap(), 14_400.)
            .unwrap();
        assert_eq!(
            service
                .resolve_full_session(
                    &serde_json::from_value(created["job"].clone()).unwrap(),
                    14_400.
                )
                .unwrap()
                .output_path,
            resolved.output_path,
            "URL refresh retains the originally reserved path"
        );
        assert_eq!(resolved.requested_end_seconds, 14_400.);
        assert!(resolved.output_path.ends_with("123_14400.mp4"));
        assert!(resolved.output_path.contains("整场"));
        assert_eq!(service.create(request.clone()).unwrap()["job_id"], id);
        service.cancel(id).unwrap();
        assert_eq!(service.retry(id).unwrap()["job"]["mode"], "full_session");
        let disk = serde_json::to_string(&service.history().unwrap().recent(100).unwrap()).unwrap();
        assert!(!disk.contains("synthetic-clip-key"));
        assert!(!disk.contains("https://"));
        for invalid in [
            json!({"mode":"full_session","session_id":"123","end_seconds":100}),
            json!({"mode":"unknown","session_id":"123"}),
            json!({"mode":3,"session_id":"123"}),
            json!({"mode":"full_session","session_id":"123","bookmark_id":"00000000000000000000000000000000"}),
        ] {
            assert!(service.create(invalid).is_err());
        }
        let mut mixed = request;
        mixed["mode"] = json!("clip");
        mixed["start_seconds"] = json!(0.);
        mixed["end_seconds"] = json!(30.);
        assert!(service.create(mixed).unwrap_err().contains("另一段"));
        runtime.shutdown().unwrap();
    }
    #[test]
    fn unresolved_full_session_survives_restart_and_retry() {
        let (_directory, runtime, service) = setup();
        let created = service
            .create(json!({"mode":"full_session","session_id":"123"}))
            .unwrap();
        let restored = ClipService::open(runtime.shared.clone());
        let tools = service.tools().unwrap();
        restored.set_tools(tools.ffmpeg.parent().unwrap()).unwrap();
        let id = created["job_id"].as_str().unwrap();
        assert_eq!(restored.job(id).unwrap().requested_end_seconds, 0.);
        assert_eq!(restored.job(id).unwrap().phase, "interrupted");
        assert_eq!(restored.retry(id).unwrap()["job"]["mode"], "full_session");
        runtime.shutdown().unwrap();
    }
    #[test]
    fn protects_existing_output_and_update_waits_for_clip_queue() {
        let (directory, runtime, service) = setup();
        let output = directory.path().join("keep.mp4");
        std::fs::write(&output, b"original").unwrap();
        assert!(service
            .create(
                json!({"session_id":"123","start_seconds":0.,"end_seconds":2.,"output_path":output})
            )
            .is_err());
        assert_eq!(std::fs::read(output).unwrap(), b"original");
        let job = service
            .create(json!({"session_id":"123","start_seconds":0.,"end_seconds":2.}))
            .unwrap();
        let entry = service.entries.lock().unwrap().remove(0);
        runtime.clips.entries.lock().unwrap().push(entry);
        assert_eq!(runtime.update_readiness()["reason"], "exporting");
        assert_eq!(
            runtime
                .prepare_update(&json!({"target_version":"0.4.0"}))
                .unwrap()["ready"],
            false
        );
        runtime.shutdown().unwrap();
        assert_eq!(
            runtime.clips.snapshot().unwrap()["jobs"][0]["status"],
            "cancelled"
        );
        assert!(job["job_id"].is_string());
    }
    #[test]
    fn configured_folders_reserve_unique_names_and_deduplicate_bulk_requests() {
        let (directory, runtime, service) = setup();
        runtime.request("download_naming_save", json!({
            "root_directory":directory.path(), "directory_template":"{date}/{source}/{anchor}",
            "filename_template":"{title}_{start}-{end}.mp4"
        })).unwrap();
        let request = json!({"session_id":"123", "start_seconds":120., "end_seconds":150.,
            "title":"精彩互动", "anchor_name":"测试主播", "source":"highlight", "request_key":"bulk:1:120-150"});
        let first = service.create(request.clone()).unwrap();
        let again = service.create(request.clone()).unwrap();
        assert_eq!(again["job_id"], first["job_id"]);
        assert_eq!(again["existing"], true);
        assert_eq!(
            service.snapshot().unwrap()["jobs"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let mut changed = request.clone();
        changed["end_seconds"] = json!(160.);
        assert!(service.create(changed).unwrap_err().contains("另一段"));
        let mut next = request;
        next["request_key"] = json!("bulk:2:120-150");
        let second = service.create(next).unwrap();
        let first_path = Path::new(first["job"]["output_path"].as_str().unwrap());
        let second_path = Path::new(second["job"]["output_path"].as_str().unwrap());
        assert!(first_path.parent().unwrap().is_dir());
        assert_eq!(
            first_path.file_name().unwrap(),
            "精彩互动_00-02-00-00-02-30.mp4"
        );
        assert_eq!(
            second_path.file_name().unwrap(),
            "精彩互动_00-02-00-00-02-30 (2).mp4"
        );
        assert_eq!(second["job"]["source"], "highlight");
        let restored = ClipService::open(runtime.shared.clone());
        assert_eq!(
            restored.snapshot().unwrap()["jobs"][0]["request_key"],
            "bulk:2:120-150"
        );
        runtime.shutdown().unwrap();
    }
    #[test]
    fn naming_settings_survive_restart_and_preview_never_creates_folders() {
        let (directory, runtime, service) = setup();
        let config = json!({"root_directory":directory.path(),"directory_template":"{year}/{month}/{anchor}","filename_template":"{title}"});
        runtime
            .request("download_naming_save", config.clone())
            .unwrap();
        let preview = runtime
            .request(
                "download_naming_preview",
                json!({"context":{"title":"NUL","session_date":"2026-10-25"}}),
            )
            .unwrap();
        assert!(preview["output_path"]
            .as_str()
            .unwrap()
            .ends_with("_NUL.mp4"));
        assert!(!directory.path().join("2026").exists());
        assert!(runtime
            .request(
                "download_naming_save",
                json!({"directory_template":"../outside"})
            )
            .is_err());
        let missing = directory.path().join("must-not-create");
        assert!(runtime
            .request("download_naming_save", json!({"root_directory":missing}))
            .is_err());
        assert!(!missing.exists());
        service.cancel_all();
        runtime.shutdown().unwrap();
        drop(runtime);
        let restarted = Runtime::open_with_transport_and_secrets(
            directory.path().into(),
            Arc::new(crate::transport::LiveTransport),
            Arc::new(Protector),
        )
        .unwrap();
        let loaded = restarted
            .request("download_naming_settings", json!({}))
            .unwrap();
        assert_eq!(loaded["settings"], config);
        assert!(loaded["variables"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["key"] == "bookmark_id"));
        restarted.shutdown().unwrap();
    }
    #[test]
    fn bookmark_download_rejects_another_session_or_forged_anchor() {
        let (_directory, runtime, _service) = setup();
        runtime.shared.state.lock().unwrap().rooms.insert(
            "123".into(),
            crate::runtime::Room {
                generation: 1,
                value: json!({"name":"打点主播", "anchor_id":"456", "session_id":"789",
                "is_live":true, "active":true, "state":"connected"}),
                route: None,
                cancel: CancellationToken::new(),
            },
        );
        let mark = runtime
            .request(
                "mark_create",
                json!({"anchor_id":"456", "session_id":"789", "title":"精彩互动"}),
            )
            .unwrap();
        let mut clip = json!({"session_id":"790", "source":"bookmark", "bookmark_id":mark["mark"]["id"],
            "start_seconds":10., "end_seconds":20.});
        assert!(runtime
            .request("clip_create", clip.clone())
            .unwrap_err()
            .contains("不一致"));
        clip["session_id"] = json!("789");
        clip["anchor_id"] = json!("999");
        assert!(runtime
            .request("clip_create", clip.clone())
            .unwrap_err()
            .contains("不一致"));
        clip["source"] = json!("manual");
        assert!(runtime
            .request("clip_create", clip)
            .unwrap_err()
            .contains("打点来源"));
        assert_eq!(
            runtime.request("clip_jobs", json!({})).unwrap()["jobs"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        runtime.shutdown().unwrap();
    }
    #[test]
    fn runtime_bookmark_download_after_restart_uses_saved_rules_without_network() {
        let (directory, runtime, _service) = setup();
        runtime.shared.state.lock().unwrap().rooms.insert("123".into(), crate::runtime::Room {
            generation: 1,
            value: json!({"name":"持久打点主播", "anchor_id":"456", "session_id":"789",
                "is_live":true,"active":true,"state":"connected",
                "broadcast_started_at":chrono::DateTime::parse_from_rfc3339("2026-10-20T12:00:00+08:00").unwrap().timestamp()}),
            route: None, cancel:CancellationToken::new(),
        });
        runtime
            .request(
                "download_naming_save",
                json!({"root_directory":directory.path(),
            "directory_template":"{date}/{anchor}","filename_template":"{title}"}),
            )
            .unwrap();
        let mark = runtime
            .request(
                "mark_create",
                json!({"anchor_id":"456","session_id":"789","title":"精选高光"}),
            )
            .unwrap();
        runtime.shutdown().unwrap();
        drop(runtime);

        let reopened = Runtime::open_with_transport_and_secrets(
            directory.path().into(),
            Arc::new(crate::transport::LiveTransport),
            Arc::new(Protector),
        )
        .unwrap();
        // Test the real public request route and persistence, with the independent
        // media worker removed before queuing anything. No recording API is called.
        let workers = reopened
            .shared
            .jobs
            .lock()
            .unwrap()
            .drain(..)
            .collect::<Vec<_>>();
        for worker in &workers {
            worker.abort();
        }
        reopened.rt.block_on(async {
            for worker in workers {
                let _ = worker.await;
            }
        });
        reopened
            .configure_media_tools(directory.path().join("tools"))
            .unwrap();
        let mut request = json!({"session_id":"789","bookmark_id":mark["mark"]["id"],
            "source":"bookmark","start_seconds":12.,"end_seconds":42.,"request_key":"runtime:first"});
        let first = reopened.request("clip_create", request.clone()).unwrap();
        let duplicate = reopened.request("clip_create", request.clone()).unwrap();
        assert_eq!(duplicate["job_id"], first["job_id"]);
        assert_eq!(duplicate["existing"], true);
        request["request_key"] = json!("runtime:second");
        let second = reopened.request("clip_create", request).unwrap();
        assert_eq!(first["job"]["anchor_id"], "456");
        assert_eq!(first["job"]["anchor_name"], "持久打点主播");
        assert_eq!(first["job"]["title"], "精选高光");
        assert_eq!(first["job"]["session_date"], "2026-10-20");
        let first_path = Path::new(first["job"]["output_path"].as_str().unwrap());
        assert!(first_path.ends_with(
            Path::new("2026-10-20")
                .join("持久打点主播")
                .join("精选高光.mp4")
        ));
        assert!(first_path.parent().unwrap().is_dir());
        assert!(!first_path.exists());
        assert_eq!(
            Path::new(second["job"]["output_path"].as_str().unwrap())
                .file_name()
                .unwrap(),
            "精选高光 (2).mp4"
        );
        assert_eq!(
            reopened.request("clip_jobs", json!({})).unwrap()["jobs"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let saved = serde_json::to_value(
            reopened
                .clips
                .history()
                .unwrap()
                .get(first["job_id"].as_str().unwrap())
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(saved["bookmark_id"], mark["mark"]["id"]);
        assert_eq!(saved["request_key"], "runtime:first");
        reopened.shutdown().unwrap();
    }
    #[test]
    fn history_outlives_hot_queue_and_old_failed_jobs_can_retry_without_duplicate_paths() {
        let (_directory, runtime, service) = setup();
        let first_request = json!({"session_id":"123","start_seconds":1.,"end_seconds":8.,"title":"旧任务",
            "request_key":"retained:first"});
        let first = service.create(first_request.clone()).unwrap();
        let first_id = first["job_id"].as_str().unwrap();
        service.cancel(first_id).unwrap();
        for number in 0..105 {
            let created=service.create(json!({"session_id":"123","start_seconds":1.,"end_seconds":8.,"title":format!("新任务 {number}")})).unwrap();
            service.cancel(created["job_id"].as_str().unwrap()).unwrap();
        }
        assert_eq!(service.entries.lock().unwrap().len(), 100);
        assert!(!service
            .entries
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.job.id == first_id));
        let old = service
            .query(&json!({"search":"旧任务","status":"cancelled"}))
            .unwrap();
        assert_eq!(old["total"], 1);
        assert_eq!(old["jobs"][0]["id"], first_id);
        let retry = service.retry(first_id).unwrap();
        assert_eq!(retry["job"]["attempt"], 2);
        assert_eq!(retry["job"]["output_path"], first["job"]["output_path"]);
        let duplicate = service.create(first_request).unwrap();
        assert_eq!(duplicate["job_id"], first_id);
        assert_eq!(duplicate["existing"], true);
        assert_eq!(service.query(&json!({})).unwrap()["total"], 106);
        assert_eq!(service.query(&json!({})).unwrap()["active_count"], 1);
        service.cancel_all();
        runtime.shutdown().unwrap();
    }
    #[test]
    fn progress_polling_is_live_without_rewriting_persistent_history() {
        let (_directory, runtime, service) = setup();
        let created = service
            .create(json!({"session_id":"123","start_seconds":1.,"end_seconds":8.}))
            .unwrap();
        let id = created["job_id"].as_str().unwrap();
        for progress in 1..=50 {
            service.update(id, |job| {
                job.progress = progress as f64;
                job.downloaded_bytes = progress * 1024;
            });
        }
        assert_eq!(
            service
                .history()
                .unwrap()
                .get(id)
                .unwrap()
                .unwrap()
                .progress,
            0.
        );
        assert_eq!(
            service.query(&json!({})).unwrap()["jobs"][0]["progress"],
            50.
        );
        service.update(id, |job| job.status = "completed".into());
        assert_eq!(
            service
                .history()
                .unwrap()
                .get(id)
                .unwrap()
                .unwrap()
                .progress,
            50.
        );
        assert_eq!(
            service.history().unwrap().get(id).unwrap().unwrap().status,
            "completed"
        );
        service.cancel_all();
        runtime.shutdown().unwrap();
    }
    #[cfg(feature = "test-support")]
    #[test]
    fn damaged_download_history_does_not_block_monitoring_and_preserves_original_bytes() {
        for (name, original) in [
            ("clips.json", b"{original invalid JSON".as_slice()),
            ("clips.sqlite3", b"original invalid SQLite bytes".as_slice()),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join(name);
            std::fs::write(&path, original).unwrap();
            let runtime = Runtime::open_with_transport_and_secrets(
                directory.path().into(),
                Arc::new(crate::testing::Simulation::default()),
                Arc::new(Protector),
            )
            .unwrap();
            assert!(runtime.request("init", json!({})).is_ok());
            assert!(runtime
                .request("clip_jobs", json!({}))
                .unwrap_err()
                .contains("下载历史暂不可用"));
            assert!(runtime
                .request(
                    "clip_create",
                    json!({"session_id":"123","start_seconds":0.,"end_seconds":1.})
                )
                .unwrap_err()
                .contains("下载历史暂不可用"));
            runtime
                .request("add_anchors", json!({"values":"1"}))
                .unwrap();
            runtime.request("start", json!({"rids":["1"]})).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                if runtime.request("poll", json!({})).unwrap()["metrics"]["connected"] == 1 {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "monitor should connect independently of downloads"
                );
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            runtime.request("stop", json!({"rids":["1"]})).unwrap();
            runtime.shutdown().unwrap();
            assert_eq!(std::fs::read(path).unwrap(), original);
        }
    }
}
