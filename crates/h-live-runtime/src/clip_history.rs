//! Durable, indexed clip history. Progress samples stay in the bounded live queue.
use crate::{
    clips::ClipJob,
    storage::{self, Result},
};
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::{path::Path, sync::Mutex, time::Duration};

pub(crate) struct ClipHistory {
    connection: Mutex<Connection>,
}
pub(crate) struct Page {
    pub jobs: Vec<ClipJob>,
    pub total: usize,
    pub active_count: usize,
    pub page: usize,
    pub limit: usize,
}
impl Page {
    pub(crate) fn value(self) -> Value {
        json!({"jobs":self.jobs,"total":self.total,"active_count":self.active_count,"page":self.page,
            "limit":self.limit,"has_more":(self.page+1)*self.limit<self.total,"history":true})
    }
}
fn decode(raw: String) -> Result<ClipJob> {
    serde_json::from_str(&raw).map_err(|_| "下载历史记录格式无效，原数据已保留".into())
}
fn valid(job: &ClipJob) -> bool {
    storage::id(&job.session_id)
        && job.id.len() == 32
        && job.id.bytes().all(|b| b.is_ascii_hexdigit())
        && job.requested_start_seconds.is_finite()
        && job.requested_end_seconds.is_finite()
        && job.requested_start_seconds >= 0.
        && (job.requested_end_seconds > job.requested_start_seconds
            || job.mode == "full_session"
                && job.requested_start_seconds == 0.
                && job.requested_end_seconds == 0.)
}
fn keys(job: &ClipJob) -> Result<(String, f64, String, String)> {
    let time = chrono::DateTime::parse_from_rfc3339(&job.created_at)
        .map_err(|_| "下载记录创建时间无效")?
        .with_timezone(&qingkan_core::archive::shanghai());
    let source = match job.source.as_str() {
        "full_session" => "整场直播",
        "bookmark" => "直播打点",
        "highlight" => "互动亮点",
        _ => "手动切片",
    };
    let search = [
        &job.title,
        &job.anchor_name,
        &job.anchor_id,
        &job.session_id,
        &job.id,
        &job.output_path,
        job.bookmark_id.as_deref().unwrap_or(""),
        source,
    ]
    .join("\n")
    .to_lowercase();
    Ok((
        time.format("%Y-%m-%d").to_string(),
        time.timestamp_millis() as f64 / 1000.,
        path_key(&job.output_path),
        search,
    ))
}
pub(crate) fn path_key(path: &str) -> String {
    #[cfg(windows)]
    {
        path.replace('/', "\\").to_lowercase()
    }
    #[cfg(not(windows))]
    {
        path.into()
    }
}
fn put(connection: &Connection, job: &ClipJob) -> Result<()> {
    if !valid(job) {
        return Err("下载记录无效，原数据已保留".into());
    }
    let (day, epoch, path, search) = keys(job)?;
    let raw = serde_json::to_string(job).map_err(|_| "下载记录无法保存")?;
    connection.execute("INSERT INTO clip_jobs(id,status,created_day,created_epoch,path_key,request_key,search_text,payload) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(id) DO UPDATE SET status=excluded.status,created_day=excluded.created_day,created_epoch=excluded.created_epoch,path_key=excluded.path_key,request_key=excluded.request_key,search_text=excluded.search_text,payload=excluded.payload",
        params![job.id,job.status,day,epoch,path,job.request_key,search,raw]).map_err(|_| "下载历史写入失败，原数据已保留")?;
    Ok(())
}
impl ClipHistory {
    pub(crate) fn open(directory: &Path) -> Result<Self> {
        let mut connection =
            Connection::open(directory.join("clips.sqlite3")).map_err(|_| "无法打开下载历史")?;
        connection
            .busy_timeout(Duration::from_secs(3))
            .map_err(|_| "下载历史不可用")?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
            CREATE TABLE IF NOT EXISTS clip_jobs(id TEXT PRIMARY KEY NOT NULL,status TEXT NOT NULL,created_day TEXT NOT NULL,created_epoch REAL NOT NULL,path_key TEXT NOT NULL,request_key TEXT,search_text TEXT NOT NULL,payload TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS clip_jobs_time ON clip_jobs(created_epoch DESC,id DESC);
            CREATE INDEX IF NOT EXISTS clip_jobs_day ON clip_jobs(created_day,created_epoch DESC,id DESC);
            CREATE INDEX IF NOT EXISTS clip_jobs_status ON clip_jobs(status,created_epoch DESC,id DESC);
            CREATE INDEX IF NOT EXISTS clip_jobs_day_status ON clip_jobs(created_day,status,created_epoch DESC,id DESC);
            CREATE INDEX IF NOT EXISTS clip_jobs_path ON clip_jobs(path_key);
            CREATE UNIQUE INDEX IF NOT EXISTS clip_jobs_request ON clip_jobs(request_key) WHERE request_key IS NOT NULL;
            CREATE TABLE IF NOT EXISTS clip_history_meta(key TEXT PRIMARY KEY NOT NULL,value TEXT NOT NULL);")
            .map_err(|_| "下载历史初始化失败，原数据已保留")?;
        let migrated: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM clip_history_meta WHERE key='legacy_json_v1')",
                [],
                |r| r.get(0),
            )
            .map_err(|_| "无法读取下载历史状态")?;
        if !migrated {
            let path = directory.join("clips.json");
            let jobs: Vec<ClipJob> = match std::fs::metadata(&path) {
                Ok(meta) => {
                    if meta.len() > 16 * 1024 * 1024 {
                        return Err("旧下载记录超过迁移限制，原文件已保留".into());
                    }
                    serde_json::from_slice(
                        &std::fs::read(&path).map_err(|_| "旧下载记录读取失败，原文件已保留")?,
                    )
                    .map_err(|_| "旧下载记录格式无效，原文件已保留")?
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => vec![],
                Err(_) => return Err("旧下载记录不可用，原文件已保留".into()),
            };
            let tx = connection.transaction().map_err(|_| "无法迁移下载历史")?;
            for job in &jobs {
                put(&tx, job)?;
            }
            tx.execute(
                "INSERT INTO clip_history_meta(key,value) VALUES('legacy_json_v1',?1)",
                [storage::now()],
            )
            .map_err(|_| "无法保存下载历史迁移状态")?;
            tx.commit()
                .map_err(|_| "无法提交下载历史迁移，原文件已保留")?;
        }
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
    pub(crate) fn put(&self, job: &ClipJob) -> Result<()> {
        let connection = self.connection.lock().map_err(|_| "下载历史不可用")?;
        put(&connection, job)
    }
    pub(crate) fn put_many(&self, jobs: &[ClipJob]) -> Result<()> {
        let mut connection = self.connection.lock().map_err(|_| "下载历史不可用")?;
        let tx = connection.transaction().map_err(|_| "下载历史不可用")?;
        for job in jobs {
            put(&tx, job)?;
        }
        tx.commit().map_err(|_| "下载历史保存失败".into())
    }
    pub(crate) fn get(&self, id: &str) -> Result<Option<ClipJob>> {
        self.connection
            .lock()
            .map_err(|_| "下载历史不可用")?
            .query_row("SELECT payload FROM clip_jobs WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|_| "读取下载历史失败".to_owned())?
            .map(decode)
            .transpose()
    }
    pub(crate) fn by_request(&self, key: &str) -> Result<Option<ClipJob>> {
        self.connection
            .lock()
            .map_err(|_| "下载历史不可用")?
            .query_row(
                "SELECT payload FROM clip_jobs WHERE request_key=?1",
                [key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| "读取下载请求失败".to_owned())?
            .map(decode)
            .transpose()
    }
    pub(crate) fn has_path(&self, path: &Path) -> Result<bool> {
        self.connection
            .lock()
            .map_err(|_| "下载历史不可用")?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM clip_jobs WHERE path_key=?1)",
                [path_key(&path.to_string_lossy())],
                |r| r.get(0),
            )
            .map_err(|_| "读取下载路径失败".into())
    }
    pub(crate) fn recent(&self, limit: usize) -> Result<Vec<ClipJob>> {
        let connection = self.connection.lock().map_err(|_| "下载历史不可用")?;
        let mut statement = connection
            .prepare("SELECT payload FROM clip_jobs ORDER BY created_epoch DESC,id DESC LIMIT ?1")
            .map_err(|_| "读取下载历史失败")?;
        let rows = statement
            .query_map([limit as i64], |r| r.get(0))
            .map_err(|_| "读取下载历史失败")?
            .collect::<rusqlite::Result<Vec<String>>>()
            .map_err(|_| "读取下载历史失败")?;
        rows.into_iter().map(decode).collect()
    }
    pub(crate) fn interrupt_active(&self) -> Result<()> {
        let jobs = {
            let connection = self.connection.lock().map_err(|_| "下载历史不可用")?;
            let mut statement = connection
                .prepare("SELECT payload FROM clip_jobs WHERE status IN ('queued','running')")
                .map_err(|_| "读取未完成下载失败")?;
            let rows = statement
                .query_map([], |r| r.get(0))
                .map_err(|_| "读取未完成下载失败")?
                .collect::<rusqlite::Result<Vec<String>>>()
                .map_err(|_| "读取未完成下载失败")?;
            rows.into_iter().map(decode).collect::<Result<Vec<_>>>()?
        };
        let jobs = jobs
            .into_iter()
            .map(|mut job| {
                job.status = "failed".into();
                job.phase = "interrupted".into();
                job.error = Some("上次切片因应用退出中断，可重试".into());
                job.updated_at = storage::now();
                job
            })
            .collect::<Vec<_>>();
        self.put_many(&jobs)
    }
    pub(crate) fn query(&self, p: &Value) -> Result<Page> {
        let number = |key: &str, default: u64, maximum: u64| -> Result<usize> {
            p.get(key)
                .map(|v| {
                    v.as_u64()
                        .filter(|n| *n <= maximum)
                        .map(|n| n as usize)
                        .ok_or_else(|| "下载分页参数无效".to_owned())
                })
                .unwrap_or(Ok(default as usize))
        };
        let requested_page = number("page", 0, 1_000_000)?;
        let limit = number("limit", 30, 100)?;
        if limit == 0 {
            return Err("每页须为 1 至 100 个任务".into());
        }
        let mut clauses = vec!["1".to_owned()];
        let mut values = Vec::<SqlValue>::new();
        if let Some(raw) = p.get("date") {
            let raw = raw.as_str().ok_or("下载日期无效")?;
            if !raw.is_empty() {
                let date = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                    .map_err(|_| "下载日期无效")?;
                if date.format("%Y-%m-%d").to_string() != raw {
                    return Err("下载日期无效".into());
                }
                clauses.push("created_day=?".into());
                values.push(raw.to_owned().into());
            }
        }
        if let Some(raw) = p.get("status") {
            match raw.as_str().ok_or("下载状态无效")? {
                "" => {}
                "active" => clauses.push("status IN ('queued','running')".into()),
                status @ ("queued" | "running" | "completed" | "failed" | "cancelled") => {
                    clauses.push("status=?".into());
                    values.push(status.to_owned().into());
                }
                _ => return Err("下载状态无效".into()),
            }
        }
        if let Some(raw) = p.get("search") {
            let query = raw.as_str().ok_or("下载搜索无效")?.trim();
            if query.chars().count() > 200 {
                return Err("下载搜索过长".into());
            }
            if !query.is_empty() {
                clauses.push("instr(search_text,?)>0".into());
                values.push(query.to_lowercase().into());
            }
        }
        let condition = clauses.join(" AND ");
        let mut connection = self.connection.lock().map_err(|_| "下载历史不可用")?;
        let tx = connection.transaction().map_err(|_| "下载历史不可用")?;
        let total = tx
            .query_row(
                &format!("SELECT COUNT(*) FROM clip_jobs WHERE {condition}"),
                params_from_iter(values.iter()),
                |r| r.get::<_, i64>(0),
            )
            .map_err(|_| "统计下载历史失败")? as usize;
        let active_count = tx
            .query_row(
                "SELECT COUNT(*) FROM clip_jobs WHERE status IN ('queued','running')",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map_err(|_| "统计下载任务失败")? as usize;
        let page = requested_page.min(total.saturating_sub(1) / limit);
        values.push((limit as i64).into());
        values.push(((page * limit) as i64).into());
        let jobs = {
            let mut statement=tx.prepare(&format!("SELECT payload FROM clip_jobs WHERE {condition} ORDER BY created_epoch DESC,id DESC LIMIT ? OFFSET ?")).map_err(|_|"读取下载历史失败")?;
            let rows = statement
                .query_map(params_from_iter(values.iter()), |r| r.get(0))
                .map_err(|_| "读取下载历史失败")?
                .collect::<rusqlite::Result<Vec<String>>>()
                .map_err(|_| "读取下载历史失败")?;
            rows.into_iter().map(decode).collect::<Result<Vec<_>>>()?
        };
        tx.commit().map_err(|_| "读取下载历史失败")?;
        Ok(Page {
            jobs,
            total,
            active_count,
            page,
            limit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn job(number: usize, day: &str, status: &str) -> ClipJob {
        serde_json::from_value(json!({"id":format!("{number:032x}"),"session_id":"123","title":format!("历史任务 {number}"),
            "status":status,"phase":status,"progress":if status=="completed" {100.}else{0.},"downloaded_bytes":0,"total_bytes":null,
            "output_path":format!("history-{number}.mp4"),"error":null,"requested_start_seconds":1.,"requested_end_seconds":8.,
            "actual_start_seconds":null,"actual_end_seconds":null,"attempt":1,"created_at":format!("{day}T12:00:00+08:00"),
            "updated_at":format!("{day}T12:00:00+08:00"),"request_key":format!("history:{number}"),
            "anchor_name":"测试主播","source":if number%2==0 {"bookmark"}else{"highlight"}})).unwrap()
    }
    #[test]
    fn legacy_migration_keeps_all_records_once_and_paginated_filters_are_stable() {
        let directory = tempfile::tempdir().unwrap();
        let legacy = (1..=135)
            .map(|n| {
                job(
                    n,
                    if n <= 65 { "2026-10-20" } else { "2026-10-21" },
                    if n == 135 { "queued" } else { "completed" },
                )
            })
            .collect::<Vec<_>>();
        let legacy_bytes = serde_json::to_vec(&legacy).unwrap();
        std::fs::write(directory.path().join("clips.json"), &legacy_bytes).unwrap();
        let history = ClipHistory::open(directory.path()).unwrap();
        let first = history
            .query(&json!({"date":"2026-10-20","limit":30}))
            .unwrap();
        assert_eq!(first.total, 65);
        assert_eq!(first.jobs.len(), 30);
        assert_eq!(first.active_count, 1);
        assert_eq!(first.page, 0);
        let second = history
            .query(&json!({"date":"2026-10-20","page":1,"limit":30}))
            .unwrap();
        assert!(!first
            .jobs
            .iter()
            .any(|j| second.jobs.iter().any(|other| other.id == j.id)));
        let last = history
            .query(&json!({"date":"2026-10-20","page":999}))
            .unwrap();
        assert_eq!(last.page, 2);
        assert_eq!(last.jobs.len(), 5);
        assert_eq!(
            history
                .query(&json!({"search":"直播打点","status":"completed"}))
                .unwrap()
                .total,
            67
        );
        assert_eq!(history.query(&json!({"search":"%_';--"})).unwrap().total, 0);
        assert_eq!(
            history.get(&format!("{:032x}", 1)).unwrap().unwrap().title,
            "历史任务 1"
        );
        assert!(history.by_request("history:1").unwrap().is_some());
        assert!(history.has_path(Path::new("history-1.mp4")).unwrap());
        assert!(history.query(&json!({"limit":101})).is_err());
        assert!(history.query(&json!({"date":"2026-1-1"})).is_err());
        assert_eq!(
            std::fs::read(directory.path().join("clips.json")).unwrap(),
            legacy_bytes
        );
        history.interrupt_active().unwrap();
        assert_eq!(history.query(&json!({"status":"active"})).unwrap().total, 0);
        assert_eq!(
            history
                .get(&format!("{:032x}", 135))
                .unwrap()
                .unwrap()
                .phase,
            "interrupted"
        );
        drop(history);
        std::fs::write(
            directory.path().join("clips.json"),
            b"legacy file no longer governs history",
        )
        .unwrap();
        let reopened = ClipHistory::open(directory.path()).unwrap();
        assert_eq!(reopened.query(&json!({})).unwrap().total, 135);
    }
    #[test]
    fn malformed_legacy_history_is_preserved_and_failed_migration_can_retry() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("clips.json");
        std::fs::write(&path, b"original invalid file").unwrap();
        assert!(ClipHistory::open(directory.path()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original invalid file");
        std::fs::write(
            &path,
            serde_json::to_vec(&vec![job(1, "2026-10-20", "failed")]).unwrap(),
        )
        .unwrap();
        assert_eq!(
            ClipHistory::open(directory.path())
                .unwrap()
                .query(&json!({}))
                .unwrap()
                .total,
            1
        );
    }
}
