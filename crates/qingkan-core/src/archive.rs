//! Durable content archive. Callers observe a result only after a whole batch commits.
use chrono::{DateTime, FixedOffset, NaiveDate, SecondsFormat, TimeZone, Utc};
use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use rusqlite::{params, params_from_iter, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const CONTENT_TYPES: [&str; 8] = [
    "chat", "gift", "like", "member", "social", "fansclub", "emoji", "activity",
];
pub const SCHEMA_VERSION: i64 = 3;
#[path = "activity_archive.rs"]
mod activity_archive;
#[path = "audience.rs"]
mod audience;
#[path = "quality.rs"]
pub mod quality;
#[path = "recent.rs"]
mod recent;
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("{0}")]
    Invalid(String),
    #[error("消息库已关闭")]
    Closed,
    #[error("{0}")]
    Database(String),
    #[error("消息归档工作线程已停止")]
    WorkerStopped,
    #[error("消息删除已取消，未提交的更改已回滚")]
    DeleteCancelled,
    #[error("消息库升级已取消，未提交的更改已回滚")]
    MigrationCancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
impl From<rusqlite::Error> for ArchiveError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error.to_string())
    }
}
pub type Result<T> = std::result::Result<T, ArchiveError>;
pub fn shanghai() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).unwrap()
}
pub fn identity(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || !value.bytes().all(|c| c.is_ascii_digit())
        || value.bytes().all(|c| c == b'0')
    {
        return Err(ArchiveError::Invalid(
            "缺少有效的真实主播 ID 或直播场次 ID".into(),
        ));
    }
    Ok(())
}
pub fn month_bounds(month: &str) -> Result<(f64, f64)> {
    let invalid = || ArchiveError::Invalid("月份必须为有效 YYYY-MM".into());
    if month.len() != 7
        || month.as_bytes()[4] != b'-'
        || !month
            .bytes()
            .enumerate()
            .all(|(i, c)| i == 4 || c.is_ascii_digit())
    {
        return Err(invalid());
    }
    let year: i32 = month[..4].parse().map_err(|_| invalid())?;
    let number: u32 = month[5..].parse().map_err(|_| invalid())?;
    if !(1..=9999).contains(&year) || !(1..=12).contains(&number) || (year == 9999 && number == 12)
    {
        return Err(invalid());
    }
    let start = NaiveDate::from_ymd_opt(year, number, 1)
        .ok_or_else(invalid)?
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let end = NaiveDate::from_ymd_opt(
        year + if number == 12 { 1 } else { 0 },
        if number == 12 { 1 } else { number + 1 },
        1,
    )
    .ok_or_else(invalid)?
    .and_hms_opt(0, 0, 0)
    .unwrap();
    Ok((
        shanghai()
            .from_local_datetime(&start)
            .single()
            .unwrap()
            .timestamp() as f64,
        shanghai()
            .from_local_datetime(&end)
            .single()
            .unwrap()
            .timestamp() as f64,
    ))
}
pub fn parse_time(value: &str) -> Result<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(&value.replacen(' ', "T", 1))
        .map(|v| v.with_timezone(&shanghai()))
        .map_err(|_| ArchiveError::Invalid("时间必须为包含时区的 ISO 时间".into()))
}
fn seconds(value: DateTime<FixedOffset>) -> f64 {
    value.timestamp() as f64 + f64::from(value.timestamp_subsec_nanos()) / 1e9
}
fn validate_duration(seconds: f64) -> Result<()> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(ArchiveError::Invalid(
            "累计监控时长必须为非负有限秒数".into(),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageRecord {
    pub anchor_id: String,
    #[serde(default)]
    pub anchor_name: String,
    #[serde(default)]
    pub web_rid: String,
    pub session_id: String,
    #[serde(default)]
    pub sender_id: String,
    #[serde(default)]
    pub sender_name: String,
    pub message_type: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub message_time: String,
    pub received_at: String,
    pub time_source: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default = "empty_payload")]
    pub payload: Value,
}
fn empty_payload() -> Value {
    json!({})
}
struct Prepared {
    record: MessageRecord,
    sent: Option<f64>,
    received: f64,
    payload: String,
    title: String,
    started_at: Option<String>,
    reply: Sender<std::result::Result<bool, String>>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Anchor {
    pub anchor_id: String,
    pub anchor_name: String,
    pub web_rid: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct StoredMessage {
    pub id: i64,
    #[serde(flatten)]
    pub record: MessageRecord,
    pub message_time_epoch: Option<f64>,
    pub received_at_epoch: f64,
    pub payload_json: String,
}
pub(crate) fn stored_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredMessage> {
    Ok(StoredMessage {
        id: row.get(0)?,
        record: MessageRecord {
            anchor_id: row.get(1)?,
            anchor_name: row.get(2)?,
            web_rid: row.get(3)?,
            session_id: row.get(4)?,
            sender_id: row.get(5)?,
            sender_name: row.get(6)?,
            message_type: row.get(7)?,
            content: row.get(8)?,
            message_time: row.get(9)?,
            received_at: row.get(10)?,
            time_source: row.get(11)?,
            message_id: row.get(12)?,
            payload: Value::Null,
        },
        message_time_epoch: row.get(13)?,
        received_at_epoch: row.get(14)?,
        payload_json: row.get(15)?,
    })
}
const METADATA_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS anchors (anchor_id TEXT PRIMARY KEY NOT NULL,anchor_name TEXT NOT NULL,web_rid TEXT NOT NULL,first_seen TEXT NOT NULL,last_seen TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS sessions (anchor_id TEXT NOT NULL,session_id TEXT NOT NULL,anchor_name TEXT NOT NULL,web_rid TEXT NOT NULL,title TEXT NOT NULL,first_seen TEXT NOT NULL,last_seen TEXT NOT NULL,started_at TEXT,PRIMARY KEY(anchor_id,session_id));
"#;
const MESSAGES_SCHEMA: &str = r#"
CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT,anchor_id TEXT NOT NULL,anchor_name TEXT NOT NULL,web_rid TEXT NOT NULL,session_id TEXT NOT NULL,sender_id TEXT NOT NULL,sender_name TEXT NOT NULL,message_type TEXT NOT NULL,content TEXT NOT NULL,message_time TEXT NOT NULL,received_at TEXT NOT NULL,time_source TEXT NOT NULL CHECK(time_source IN ('server','missing')),message_id TEXT NOT NULL,message_time_epoch REAL,received_at_epoch REAL NOT NULL,payload_json TEXT NOT NULL,CHECK(anchor_id<>'' AND session_id<>''),CHECK(message_type IN ('chat','gift','like','member','social','fansclub','emoji','activity')),FOREIGN KEY(anchor_id,session_id) REFERENCES sessions(anchor_id,session_id));
"#;
const INDEX_SCHEMA: &str = r#"
CREATE UNIQUE INDEX IF NOT EXISTS messages_server_identity ON messages(anchor_id,session_id,message_id) WHERE message_id<>'';
CREATE INDEX IF NOT EXISTS messages_anchor_month ON messages(anchor_id,COALESCE(message_time_epoch,received_at_epoch));
"#;
const DURATION_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS session_monitor_durations (
    anchor_id TEXT NOT NULL,session_id TEXT NOT NULL,
    accumulated_seconds REAL NOT NULL CHECK(accumulated_seconds>=0),
    last_checkpoint_at TEXT NOT NULL,PRIMARY KEY(anchor_id,session_id));
"#;
const MESSAGE_COLUMNS: &str = "id,anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,content,message_time,received_at,time_source,message_id,message_time_epoch,received_at_epoch,payload_json";

fn schema_version(conn: &Connection) -> Result<i64> {
    let version = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?;
    if !(0..=SCHEMA_VERSION).contains(&version) {
        return Err(ArchiveError::Invalid(
            "消息库版本较新，请使用更新版本的应用".into(),
        ));
    }
    Ok(version)
}

// SQLite's create/copy/drop/rename procedure changes the CHECK without editing
// sqlite_schema directly. All data and schema edits are one IMMEDIATE transaction.
fn initialize_schema(
    conn: &mut Connection,
    cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
) -> Result<()> {
    if schema_version(conn)? == SCHEMA_VERSION
        && conn
            .prepare("SELECT 1 FROM sqlite_schema WHERE type='table' AND name='quality_registry'")?
            .exists([])?
        && conn.prepare("SELECT 1 FROM sqlite_schema WHERE type='table' AND name='audience_minutes'")?.exists([])?
        && conn.prepare("SELECT 1 FROM sqlite_schema WHERE type='index' AND name='messages_lottery_draw'")?.exists([])?
        && conn.prepare("SELECT 1 FROM sqlite_schema WHERE type='table' AND name='activity_rounds'")?.exists([])?
        && conn.prepare("SELECT 1 FROM sqlite_schema WHERE type='table' AND name='activity_aliases'")?.exists([])?
        && conn.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE type='index' AND name IN ('messages_type_recent','messages_room_recent','messages_room_type_recent')",[],|r|r.get::<_,i64>(0))?==3
    {
        return Ok(());
    }
    if cancelled() {
        return Err(ArchiveError::MigrationCancelled);
    }
    // Must be outside the transaction. This prevents inbound ON DELETE actions
    // from firing when the old table is dropped; every FK is checked before commit.
    conn.execute_batch("PRAGMA foreign_keys=OFF;")?;
    let stop = cancelled.clone();
    conn.progress_handler(1000, Some(move || stop()))?;
    let result = (|| -> Result<()> {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // A concurrent opener may have already migrated while we waited for the lock.
        match schema_version(&tx)? {
            SCHEMA_VERSION => (),
            0 => {
                if tx
                    .prepare("SELECT 1 FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'")?
                    .exists([])?
                {
                    return Err(ArchiveError::Invalid(
                        "未识别的消息库结构，已保留原数据库".into(),
                    ));
                }
                tx.execute_batch(METADATA_SCHEMA)?;
                tx.execute_batch(MESSAGES_SCHEMA)?;
                tx.execute_batch(INDEX_SCHEMA)?;
            }
            1 => migrate_v1(&tx)?,
            2 => (),
            _ => unreachable!(),
        }
        // Additive in unreleased v2: earlier workspace v2 archives may lack this
        // independent table. It intentionally has no message/session FK: a task
        // can run without receiving a message, and clearing chat keeps its time.
        tx.execute_batch(DURATION_SCHEMA)?;
        quality::initialize(&tx)?;
        tx.execute_batch(audience::SCHEMA)?;
        tx.execute_batch(recent::INDEXES)?;
        activity_archive::initialize(&tx)?;
        if tx.prepare("PRAGMA foreign_key_check")?.exists([])? {
            return Err(ArchiveError::Invalid(
                "消息库关系校验失败，升级未提交".into(),
            ));
        }
        if cancelled() {
            return Err(ArchiveError::MigrationCancelled);
        }
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.commit()?;
        Ok(())
    })();
    // An interrupted SQLite VM can also interrupt Transaction's drop rollback.
    // Disable the progress callback first, then explicitly ensure rollback completes.
    conn.progress_handler(0, None::<fn() -> bool>)?;
    if !conn.is_autocommit() {
        conn.execute_batch("ROLLBACK;")?;
    }
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    if result.is_err() && cancelled() {
        return Err(ArchiveError::MigrationCancelled);
    }
    result
}

fn migrate_v1(conn: &Connection) -> Result<()> {
    let columns = conn
        .prepare("PRAGMA table_info(messages)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if columns.join(",") != MESSAGE_COLUMNS {
        return Err(ArchiveError::Invalid(
            "未识别的消息表结构，升级未提交".into(),
        ));
    }
    let sequence: Option<i64> = conn
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name='messages'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let indexes = conn.prepare("SELECT sql FROM sqlite_schema WHERE tbl_name='messages' AND type='index' AND sql IS NOT NULL ORDER BY name")?
        .query_map([], |r| r.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
    let triggers = conn
        .prepare("SELECT name,sql FROM sqlite_schema WHERE type='trigger' ORDER BY name")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    // Views keep referring to the same columns and name. Temporarily removing
    // them avoids ALTER TABLE reparsing a reference during the drop/rename gap.
    let views = conn
        .prepare("SELECT name,sql FROM sqlite_schema WHERE type='view' ORDER BY name")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    // Triggers can reference messages from another table or be INSTEAD OF
    // triggers on a view. Preserve their original SQL without firing them on copy.
    for (name, _) in &triggers {
        conn.execute_batch(&format!("DROP TRIGGER \"{}\";", name.replace('"', "\"\"")))?;
    }
    for (name, _) in &views {
        conn.execute_batch(&format!("DROP VIEW \"{}\";", name.replace('"', "\"\"")))?;
    }
    conn.execute_batch(&MESSAGES_SCHEMA.replacen(
        "CREATE TABLE messages",
        "CREATE TABLE messages_v2_migration",
        1,
    ))?;
    conn.execute_batch(&format!("INSERT INTO messages_v2_migration({MESSAGE_COLUMNS}) SELECT {MESSAGE_COLUMNS} FROM messages;"))?;
    conn.execute_batch(
        "DROP TABLE messages; ALTER TABLE messages_v2_migration RENAME TO messages;",
    )?;
    // Copying surviving IDs alone loses the high-water mark if the largest ID
    // was previously deleted. Restore both its exact value and absence on empty DBs.
    conn.execute("DELETE FROM sqlite_sequence WHERE name='messages'", [])?;
    if let Some(seq) = sequence {
        conn.execute(
            "INSERT INTO sqlite_sequence(name,seq) VALUES('messages',?)",
            [seq],
        )?;
    }
    for sql in indexes {
        conn.execute_batch(&sql)?;
    }
    for (_, sql) in views {
        conn.execute_batch(&sql)?;
    }
    for (_, sql) in triggers {
        conn.execute_batch(&sql)?;
    }
    Ok(())
}
fn connect(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(10))?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;")?;
    Ok(conn)
}
#[derive(Clone, Copy)]
pub struct ArchiveOptions {
    pub queue_capacity: usize,
    pub batch_size: usize,
    pub batch_wait: Duration,
}
impl Default for ArchiveOptions {
    fn default() -> Self {
        Self {
            queue_capacity: 4096,
            batch_size: 256,
            batch_wait: Duration::from_millis(4),
        }
    }
}
#[derive(Default)]
struct Stats {
    committed: AtomicU64,
    duplicates: AtomicU64,
    batches: AtomicU64,
    high_water: AtomicU64,
    largest: AtomicU64,
    failed: AtomicU64,
    pending: AtomicU64,
    alive: AtomicBool,
}
struct State {
    sender: Option<Sender<Prepared>>,
    thread: Option<JoinHandle<()>>,
    closed: bool,
}
struct Inner {
    path: PathBuf,
    options: ArchiveOptions,
    state: Mutex<State>,
    stats: Arc<Stats>,
    // A record holds a read permit through its commit acknowledgement. A
    // destructive operation exclusively waits for every accepted write.
    mutation_gate: RwLock<()>,
}
#[derive(Clone)]
pub struct ArchiveStore {
    inner: Arc<Inner>,
}
impl ArchiveStore {
    pub fn open(data_dir: impl AsRef<Path>, options: ArchiveOptions) -> Result<Self> {
        Self::open_cancellable(data_dir, options, || false)
    }
    pub fn open_cancellable(
        data_dir: impl AsRef<Path>,
        options: ArchiveOptions,
        cancelled: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Result<Self> {
        if options.queue_capacity == 0 || options.batch_size == 0 {
            return Err(ArchiveError::Invalid("归档队列及批次参数无效".into()));
        }
        std::fs::create_dir_all(&data_dir)?;
        let path = data_dir.as_ref().join("messages.sqlite3");
        let mut conn = connect(&path)?;
        schema_version(&conn)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        initialize_schema(&mut conn, Arc::new(cancelled))?;
        Ok(Self {
            inner: Arc::new(Inner {
                path,
                options,
                state: Mutex::new(State {
                    sender: None,
                    thread: None,
                    closed: false,
                }),
                stats: Arc::new(Stats::default()),
                mutation_gate: RwLock::new(()),
            }),
        })
    }
    pub fn path(&self) -> &Path {
        &self.inner.path
    }
    pub fn load_monitor_duration(&self, anchor_id: &str, session_id: &str) -> Result<f64> {
        identity(anchor_id)?;
        identity(session_id)?;
        let snapshot = self.snapshot()?;
        let seconds: Option<f64> = snapshot.connection.query_row(
            "SELECT accumulated_seconds FROM session_monitor_durations WHERE anchor_id=? AND session_id=?",
            params![anchor_id, session_id], |r| r.get(0),
        ).optional()?;
        let seconds = seconds.unwrap_or(0.0);
        validate_duration(seconds)?;
        Ok(seconds)
    }
    pub fn save_monitor_duration(
        &self,
        anchor_id: &str,
        session_id: &str,
        seconds: f64,
    ) -> Result<()> {
        identity(anchor_id)?;
        identity(session_id)?;
        validate_duration(seconds)?;
        let _permit = self
            .inner
            .mutation_gate
            .read()
            .map_err(|_| ArchiveError::WorkerStopped)?;
        // Serialize close with this short synchronous checkpoint; returning means
        // the observed accumulated time has committed, not merely been queued.
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| ArchiveError::WorkerStopped)?;
        if state.closed {
            return Err(ArchiveError::Closed);
        }
        let conn = connect(self.path())?;
        conn.execute("INSERT INTO session_monitor_durations(anchor_id,session_id,accumulated_seconds,last_checkpoint_at)
            VALUES(?,?,?,?) ON CONFLICT(anchor_id,session_id) DO UPDATE SET
            accumulated_seconds=excluded.accumulated_seconds,last_checkpoint_at=excluded.last_checkpoint_at
            WHERE excluded.accumulated_seconds>=session_monitor_durations.accumulated_seconds",
            params![anchor_id,session_id,seconds,Utc::now().to_rfc3339_opts(SecondsFormat::Millis,true)])?;
        Ok(())
    }
    pub fn record_message(
        &self,
        mut record: MessageRecord,
        title: String,
        started_at: Option<String>,
    ) -> Result<bool> {
        let _permit = self
            .inner
            .mutation_gate
            .read()
            .map_err(|_| ArchiveError::WorkerStopped)?;
        identity(&record.anchor_id)?;
        identity(&record.session_id)?;
        if !CONTENT_TYPES.contains(&record.message_type.as_str()) {
            return Err(ArchiveError::Invalid(
                "观众统计与运行日志不能写入内容消息库".into(),
            ));
        }
        let received = parse_time(&record.received_at)?;
        let received_epoch = seconds(received);
        record.received_at = received.to_rfc3339_opts(SecondsFormat::Millis, false);
        let sent = if record.message_time.is_empty() {
            if record.time_source != "missing" {
                return Err(ArchiveError::Invalid("缺失发送时间必须明确标识".into()));
            }
            None
        } else {
            if record.time_source != "server" {
                return Err(ArchiveError::Invalid("无效时间来源".into()));
            }
            let t = parse_time(&record.message_time)?;
            record.message_time = t.to_rfc3339_opts(SecondsFormat::Millis, false);
            Some(seconds(t))
        };
        let payload = serde_json::to_string(&record.payload)?;
        let (tx, rx) = bounded(1);
        let mut request = Prepared {
            record,
            sent,
            received: received_epoch,
            payload,
            title,
            started_at,
            reply: tx,
        };
        loop {
            let mut state = self
                .inner
                .state
                .lock()
                .map_err(|_| ArchiveError::WorkerStopped)?;
            if state.closed {
                return Err(ArchiveError::Closed);
            }
            if state.thread.is_none() {
                let (sender, receiver) = bounded(self.inner.options.queue_capacity);
                let stats = self.inner.stats.clone();
                let path = self.inner.path.clone();
                let options = self.inner.options;
                stats.alive.store(true, Ordering::Release);
                let handle = thread::Builder::new()
                    .name("native-message-archive".into())
                    .spawn(move || writer(receiver, path, options, stats));
                match handle {
                    Ok(handle) => {
                        state.sender = Some(sender);
                        state.thread = Some(handle);
                    }
                    Err(error) => {
                        self.inner.stats.alive.store(false, Ordering::Release);
                        return Err(error.into());
                    }
                }
            }
            self.inner.stats.pending.fetch_add(1, Ordering::AcqRel);
            match state
                .sender
                .as_ref()
                .ok_or(ArchiveError::WorkerStopped)?
                .try_send(request)
            {
                Ok(()) => {
                    self.inner.stats.high_water.fetch_max(
                        state.sender.as_ref().unwrap().len() as u64,
                        Ordering::Relaxed,
                    );
                    break;
                }
                Err(TrySendError::Full(returned)) => {
                    self.inner.stats.pending.fetch_sub(1, Ordering::AcqRel);
                    request = returned;
                    drop(state);
                    thread::sleep(Duration::from_millis(1));
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.inner.stats.pending.fetch_sub(1, Ordering::AcqRel);
                    return Err(ArchiveError::WorkerStopped);
                }
            }
        }
        rx.recv()
            .map_err(|_| ArchiveError::WorkerStopped)?
            .map_err(ArchiveError::Database)
    }
    pub fn snapshot(&self) -> Result<ArchiveSnapshot> {
        ArchiveSnapshot::open(&self.inner.path)
    }
    pub fn list_anchors(&self) -> Result<Vec<Anchor>> {
        self.snapshot()?.list_anchors(None)
    }
    pub fn list_sessions(
        &self,
        selection: &ArchiveDeleteFilter,
        limit: usize,
        offset: usize,
    ) -> Result<Value> {
        if !(1..=500).contains(&limit) || offset > 1_000_000 {
            return Err(ArchiveError::Invalid("场次分页参数无效".into()));
        }
        let (condition, values) = selection.filters()?;
        let snapshot = self.snapshot()?;
        let grouped = format!("SELECT anchor_id,session_id,COUNT(*) AS records_count FROM messages WHERE {condition} GROUP BY anchor_id,session_id");
        let total: i64 = snapshot.connection.query_row(
            &format!("SELECT COUNT(*) FROM ({grouped})"),
            params_from_iter(values.iter()),
            |r| r.get(0),
        )?;
        let mut statement = snapshot.connection.prepare(&format!("SELECT s.anchor_id,COALESCE(a.anchor_name,s.anchor_name),s.session_id,s.started_at,s.first_seen,s.last_seen,s.title,m.records_count FROM ({grouped}) m JOIN sessions s ON s.anchor_id=m.anchor_id AND s.session_id=m.session_id LEFT JOIN anchors a ON a.anchor_id=s.anchor_id ORDER BY s.first_seen DESC,s.anchor_id,s.session_id LIMIT ? OFFSET ?"))?;
        let mut paged = values;
        paged.extend([(limit as i64).into(), (offset as i64).into()]);
        let rows = statement
            .query_map(params_from_iter(paged), |r| {
                Ok(json!({
                    "anchor_id":r.get::<_,String>(0)?,"anchor_name":r.get::<_,String>(1)?,
                    "session_id":r.get::<_,String>(2)?,"started_at":r.get::<_,Option<String>>(3)?,
                    "first_seen":r.get::<_,String>(4)?,"last_seen":r.get::<_,String>(5)?,
                    "title":r.get::<_,String>(6)?,"records_count":r.get::<_,i64>(7)?
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let next = offset + rows.len();
        Ok(
            json!({"sessions":rows,"total":total,"next_offset":if next < total as usize { Some(next) } else { None }}),
        )
    }
    pub fn preview_delete(&self, selection: &ArchiveDeleteFilter) -> Result<Value> {
        let (condition, values) = selection.filters()?;
        let snapshot = self.snapshot()?;
        deletion_preview(&snapshot.connection, selection, &condition, &values)
    }
    pub fn execute_delete(&self, selection: &ArchiveDeleteFilter) -> Result<Value> {
        self.execute_delete_cancellable(selection, || false)
    }
    pub fn execute_delete_cancellable(
        &self,
        selection: &ArchiveDeleteFilter,
        cancelled: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Result<Value> {
        let (condition, values) = selection.filters()?;
        let cancelled = Arc::new(cancelled);
        let _permit = loop {
            if cancelled() {
                return Err(ArchiveError::DeleteCancelled);
            }
            match self.inner.mutation_gate.try_write() {
                Ok(permit) => break permit,
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    return Err(ArchiveError::WorkerStopped)
                }
                Err(std::sync::TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(2)),
            }
        };
        if self
            .inner
            .state
            .lock()
            .map_err(|_| ArchiveError::WorkerStopped)?
            .closed
        {
            return Err(ArchiveError::Closed);
        }
        let mut connection = connect(&self.inner.path)?;
        // Avoid a long uninterruptible external-database busy wait. The shared
        // mutation gate has already drained this instance's own writer.
        connection.busy_timeout(Duration::from_millis(200))?;
        let check = cancelled.clone();
        connection.progress_handler(1000, Some(move || check()))?;
        let result = (|| -> Result<Value> {
            let tx =
                connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            // Remember only affected sessions, so deleting one month never removes
            // unrelated metadata and a session spanning months remains available.
            tx.execute(&format!("CREATE TEMP TABLE delete_sessions AS SELECT DISTINCT anchor_id,session_id FROM messages WHERE {condition}"), params_from_iter(values.iter()))?;
            let deleted_records = tx.execute(
                &format!("DELETE FROM messages WHERE {condition}"),
                params_from_iter(values.iter()),
            )?;
            quality::after_delete(&tx)?;
            let deleted_sessions = tx.execute("DELETE FROM sessions WHERE EXISTS(SELECT 1 FROM delete_sessions d WHERE d.anchor_id=sessions.anchor_id AND d.session_id=sessions.session_id) AND NOT EXISTS(SELECT 1 FROM messages m WHERE m.anchor_id=sessions.anchor_id AND m.session_id=sessions.session_id)", [])?;
            tx.execute("DELETE FROM anchors WHERE EXISTS(SELECT 1 FROM delete_sessions d WHERE d.anchor_id=anchors.anchor_id) AND NOT EXISTS(SELECT 1 FROM sessions s WHERE s.anchor_id=anchors.anchor_id)", [])?;
            let remaining_records: i64 =
                tx.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))?;
            if cancelled() {
                return Err(ArchiveError::DeleteCancelled);
            }
            tx.commit()?;
            Ok(
                json!({"deleted_records":deleted_records,"deleted_sessions":deleted_sessions,"remaining_records":remaining_records}),
            )
        })();
        connection.progress_handler(0, None::<fn() -> bool>)?;
        if result.is_err() {
            // An interrupt can also interrupt Drop's rollback. Disable the
            // handler first and explicitly finish any remaining transaction.
            if !connection.is_autocommit() {
                connection.execute_batch("ROLLBACK;")?;
            }
            if cancelled() {
                return Err(ArchiveError::DeleteCancelled);
            }
        }
        result
    }
    pub fn metrics(&self) -> Value {
        let stats = &self.inner.stats;
        let queue = self
            .inner
            .state
            .lock()
            .ok()
            .and_then(|s| s.sender.as_ref().map(Sender::len))
            .unwrap_or(0);
        json!({"mode":"native_bounded_batch_writer","committed_messages":stats.committed.load(Ordering::Relaxed),"duplicates":stats.duplicates.load(Ordering::Relaxed),"batches":stats.batches.load(Ordering::Relaxed),"queue_high_water":stats.high_water.load(Ordering::Relaxed),"largest_batch":stats.largest.load(Ordering::Relaxed),"failed_batches":stats.failed.load(Ordering::Relaxed),"pending_requests":stats.pending.load(Ordering::Acquire),"queue_depth":queue,"queue_capacity":self.inner.options.queue_capacity,"batch_size":self.inner.options.batch_size,"batch_wait_seconds":self.inner.options.batch_wait.as_secs_f64(),"writer_alive":stats.alive.load(Ordering::Acquire)})
    }
    pub fn close(&self, timeout: Duration) -> bool {
        if let Ok(mut state) = self.inner.state.lock() {
            state.closed = true;
            state.sender.take();
        } else {
            return false;
        }
        let deadline = Instant::now() + timeout.min(Duration::from_secs(86400));
        loop {
            if !self.inner.stats.alive.load(Ordering::Acquire) {
                if let Ok(mut state) = self.inner.state.lock() {
                    if let Some(handle) = state.thread.take() {
                        return handle.join().is_ok();
                    }
                    return true;
                }
                return false;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        if let Ok(state) = self.state.get_mut() {
            state.sender.take();
        }
    }
}
fn save_batch(connection: &mut Connection, batch: &[Prepared]) -> Result<Vec<bool>> {
    let tx = connection.transaction()?;
    let mut results = Vec::with_capacity(batch.len());
    for request in batch {
        let r = &request.record;
        tx.prepare_cached("INSERT INTO anchors VALUES(?1,?2,?3,?4,?4) ON CONFLICT(anchor_id) DO UPDATE SET anchor_name=excluded.anchor_name,web_rid=excluded.web_rid,first_seen=MIN(anchors.first_seen,excluded.first_seen),last_seen=MAX(anchors.last_seen,excluded.last_seen)")?.execute(params![r.anchor_id,r.anchor_name,r.web_rid,r.received_at])?;
        tx.prepare_cached("INSERT INTO sessions VALUES(?1,?2,?3,?4,?5,?6,?6,?7) ON CONFLICT(anchor_id,session_id) DO UPDATE SET anchor_name=excluded.anchor_name,web_rid=excluded.web_rid,title=excluded.title,first_seen=MIN(sessions.first_seen,excluded.first_seen),last_seen=MAX(sessions.last_seen,excluded.last_seen),started_at=COALESCE(sessions.started_at,excluded.started_at)")?.execute(params![r.anchor_id,r.session_id,r.anchor_name,r.web_rid,request.title,r.received_at,request.started_at])?;
        if activity_archive::merge_existing(&tx, r, &request.payload)? {
            results.push(false);
            continue;
        }
        let inserted=tx.prepare_cached("INSERT INTO messages(anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,content,message_time,received_at,time_source,message_id,message_time_epoch,received_at_epoch,payload_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15) ON CONFLICT(anchor_id,session_id,message_id) WHERE message_id<>'' DO NOTHING")?.execute(params![r.anchor_id,r.anchor_name,r.web_rid,r.session_id,r.sender_id,r.sender_name,r.message_type,r.content,r.message_time,r.received_at,r.time_source,r.message_id,request.sent,request.received,request.payload])?==1;
        if inserted {
            activity_archive::register(&tx, r, &request.payload, tx.last_insert_rowid())?;
        }
        results.push(inserted);
    }
    tx.commit()?;
    Ok(results)
}
fn writer(receiver: Receiver<Prepared>, path: PathBuf, options: ArchiveOptions, stats: Arc<Stats>) {
    struct Alive(Arc<Stats>);
    impl Drop for Alive {
        fn drop(&mut self) {
            self.0.alive.store(false, Ordering::Release);
        }
    }
    let _alive = Alive(stats.clone());
    let mut connection = None;
    while let Ok(first) = receiver.recv() {
        let mut batch = vec![first];
        let deadline = Instant::now() + options.batch_wait;
        while batch.len() < options.batch_size {
            match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(request) => batch.push(request),
                Err(_) => break,
            }
        }
        let outcome = (|| {
            if connection.is_none() {
                connection = Some(connect(&path)?);
            }
            save_batch(connection.as_mut().unwrap(), &batch)
        })();
        match &outcome {
            Ok(values) => {
                let n = values.iter().filter(|v| **v).count() as u64;
                stats.committed.fetch_add(n, Ordering::Relaxed);
                stats
                    .duplicates
                    .fetch_add(batch.len() as u64 - n, Ordering::Relaxed);
                stats.batches.fetch_add(1, Ordering::Relaxed);
                stats
                    .largest
                    .fetch_max(batch.len() as u64, Ordering::Relaxed);
            }
            Err(_) => {
                stats.failed.fetch_add(1, Ordering::Relaxed);
            }
        }
        let error = outcome.as_ref().err().map(ToString::to_string);
        for (index, request) in batch.into_iter().enumerate() {
            stats.pending.fetch_sub(1, Ordering::AcqRel);
            let result = match &outcome {
                Ok(values) => Ok(values[index]),
                Err(_) => Err(error.clone().unwrap()),
            };
            let _ = request.reply.send(result);
        }
    }
}

pub struct ArchiveSnapshot {
    pub(crate) connection: Connection,
}
/// Destructive selections always name at least one real anchor. Optional
/// month and session selections intersect; an empty list is never "all".
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveDeleteFilter {
    pub anchor_ids: Vec<String>,
    pub month: Option<String>,
    pub session_ids: Option<Vec<String>>,
}
impl ArchiveDeleteFilter {
    fn filters(&self) -> Result<(String, Vec<rusqlite::types::Value>)> {
        if self.anchor_ids.is_empty() || self.anchor_ids.len() > 500 {
            return Err(ArchiveError::Invalid(
                "请选择 1 至 500 个历史主播，不能无范围删除".into(),
            ));
        }
        let (mut condition, mut values) =
            filters(Some(&self.anchor_ids), self.month.as_deref(), None)?;
        if let Some(sessions) = &self.session_ids {
            if sessions.is_empty() || sessions.len() > 500 {
                return Err(ArchiveError::Invalid("请选择 1 至 500 个直播场次".into()));
            }
            for id in sessions {
                identity(id)?;
            }
            condition.push_str(&format!(
                " AND session_id IN ({})",
                vec!["?"; sessions.len()].join(",")
            ));
            values.extend(sessions.iter().cloned().map(Into::into));
        }
        Ok((condition, values))
    }
}
fn deletion_preview(
    connection: &Connection,
    selection: &ArchiveDeleteFilter,
    condition: &str,
    values: &[rusqlite::types::Value],
) -> Result<Value> {
    let records: i64 = connection.query_row(
        &format!("SELECT COUNT(*) FROM messages WHERE {condition}"),
        params_from_iter(values.iter()),
        |r| r.get(0),
    )?;
    let sessions: i64 = connection.query_row(&format!("SELECT COUNT(*) FROM (SELECT anchor_id,session_id FROM messages WHERE {condition} GROUP BY anchor_id,session_id)"), params_from_iter(values.iter()), |r| r.get(0))?;
    Ok(
        json!({"records_count":records,"sessions_count":sessions,"anchor_ids":selection.anchor_ids,"month":selection.month,"session_ids":selection.session_ids}),
    )
}
impl ArchiveSnapshot {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::from_secs(10))?;
        connection.execute_batch("PRAGMA query_only=ON; BEGIN;")?;
        schema_version(&connection)?;
        connection.query_row("SELECT COUNT(*) FROM anchors", [], |r| r.get::<_, i64>(0))?;
        Ok(Self { connection })
    }
    pub fn list_anchors(&self, ids: Option<&[String]>) -> Result<Vec<Anchor>> {
        let (filter, parameters) = filters(ids, None, None)?;
        let mut stmt=self.connection.prepare(&format!("SELECT anchor_id,anchor_name,web_rid FROM anchors WHERE ({filter}) AND EXISTS(SELECT 1 FROM messages m WHERE m.anchor_id=anchors.anchor_id) ORDER BY anchor_name,anchor_id"))?;
        let rows = stmt.query_map(params_from_iter(parameters), |r| {
            Ok(Anchor {
                anchor_id: r.get(0)?,
                anchor_name: r.get(1)?,
                web_rid: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn count_missing_time(
        &self,
        ids: Option<&[String]>,
        month: Option<&str>,
        types: Option<&[String]>,
    ) -> Result<u64> {
        let (filter, parameters) = filters(ids, month, types)?;
        Ok(self.connection.query_row(
            &format!(
                "SELECT COUNT(*) FROM messages WHERE message_time_epoch IS NULL AND ({filter})"
            ),
            params_from_iter(parameters),
            |r| r.get::<_, i64>(0),
        )? as u64)
    }
    pub fn visit_messages(
        &self,
        ids: Option<&[String]>,
        month: &str,
        types: Option<&[String]>,
        mut visit: impl FnMut(StoredMessage) -> Result<()>,
    ) -> Result<()> {
        let (query, parameters) = message_query(ids, month, types)?;
        let mut stmt = self.connection.prepare(&query)?;
        let mut rows = stmt.query(params_from_iter(parameters))?;
        while let Some(row) = rows.next()? {
            visit(stored_row(row)?)?;
        }
        Ok(())
    }
}
impl Drop for ArchiveSnapshot {
    fn drop(&mut self) {
        let _ = self.connection.execute_batch("ROLLBACK;");
    }
}
pub(crate) fn message_query(
    ids: Option<&[String]>,
    month: &str,
    types: Option<&[String]>,
) -> Result<(String, Vec<rusqlite::types::Value>)> {
    let (filter, parameters) = filters(ids, Some(month), types)?;
    Ok((format!("SELECT id,anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,content,message_time,received_at,time_source,message_id,message_time_epoch,received_at_epoch,payload_json FROM messages WHERE {filter} ORDER BY COALESCE(message_time_epoch,received_at_epoch),id"),parameters))
}
fn filters(
    ids: Option<&[String]>,
    month: Option<&str>,
    types: Option<&[String]>,
) -> Result<(String, Vec<rusqlite::types::Value>)> {
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    if let Some(ids) = ids {
        for id in ids {
            identity(id)?;
        }
        if ids.is_empty() {
            clauses.push("0".into());
        } else {
            clauses.push(format!("anchor_id IN ({})", vec!["?"; ids.len()].join(",")));
            values.extend(ids.iter().cloned().map(Into::into));
        }
    }
    if let Some(month) = month {
        let (a, b) = month_bounds(month)?;
        clauses.push("COALESCE(message_time_epoch,received_at_epoch)>=? AND COALESCE(message_time_epoch,received_at_epoch)<?".into());
        values.extend([a.into(), b.into()]);
    }
    if let Some(types) = types {
        if types.iter().any(|t| !CONTENT_TYPES.contains(&t.as_str())) {
            return Err(ArchiveError::Invalid("只能导出聊天与互动消息类型".into()));
        }
        if types.is_empty() {
            clauses.push("0".into());
        } else {
            clauses.push(format!(
                "message_type IN ({})",
                vec!["?"; types.len()].join(",")
            ));
            values.extend(types.iter().cloned().map(Into::into));
        }
    }
    Ok((
        if clauses.is_empty() {
            "1".into()
        } else {
            clauses.join(" AND ")
        },
        values,
    ))
}
