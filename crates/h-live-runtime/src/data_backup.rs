//! Portable data packages. Credentials stay local; restore runs only after the host drops Runtime.
use crate::{
    runtime::{Runtime, VERSION},
    storage::{self, Result},
};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{Read, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
    time::Duration,
};
use tempfile::{NamedTempFile, TempDir};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};
#[path = "data_backup_merge.rs"]
mod merge;

const FORMAT: &str = "qingkan-live-backup";
const MAX_TOTAL: u64 = 32 * 1024 * 1024 * 1024;
const MAX_JSON: u64 = 16 * 1024 * 1024;
const MAX_MANIFEST: u64 = 1024 * 1024;
const FILES: [&str; 7] = [
    "messages.sqlite3",
    "clips.sqlite3",
    "config.json",
    "session-marks.json",
    "polling-habits.json",
    "preferences.json",
    "dataset.json",
];
const MANAGED: [&str; 11] = [
    "messages.sqlite3",
    "messages.sqlite3-wal",
    "messages.sqlite3-shm",
    "clips.sqlite3",
    "clips.sqlite3-wal",
    "clips.sqlite3-shm",
    "config.json",
    "session-marks.json",
    "polling-habits.json",
    "clips.json",
    "update-resume.json",
];
const CONFIG_KEYS: [&str; 9] = [
    "anchors",
    "anchor_metadata",
    "anchor_tags",
    "pc_unsupported_rids",
    "debug_logging",
    "startup_monitoring",
    "highlight_provider",
    "download_naming",
    "time_display",
];
const JOURNAL: &str = ".data-restore-journal.json";

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    bytes: u64,
    sha256: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    version: u32,
    app_version: String,
    created_at: String,
    files: Vec<Entry>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    format: u32,
    phase: String,
    backup: String,
    originals: Vec<Entry>,
}

pub struct DataBackup;
pub struct PreparedBackup {
    directory: TempDir,
    manifest: Manifest,
    summary: Value,
    token: String,
    used: AtomicBool,
}

impl Runtime {
    pub fn flush_for_backup(&self) -> Result<()> {
        let _gate = self.action.lock().map_err(|_| "后台状态不可用")?;
        self.ensure_open()?;
        self.shared.flush_metadata()?;
        self.shared.polling.flush()?;
        self.shared
            .state
            .lock()
            .map_err(|_| "后台状态不可用")?
            .config
            .save()
    }
}

fn error(text: &str) -> String {
    text.into()
}
fn regular(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| error("备份文件不存在或不可读"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(error("备份不允许链接或特殊文件"));
    }
    Ok(())
}
fn safe_directory(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        if let Ok(metadata) = fs::symlink_metadata(ancestor) {
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(error("备份目录不能包含链接或普通文件"));
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err(error("备份目录不能包含重解析链接"));
                }
            }
        }
    }
    Ok(())
}
fn read_json(path: &Path, maximum: u64) -> Result<Value> {
    regular(path)?;
    if fs::metadata(path)
        .map_err(|_| error("无法读取备份文件"))?
        .len()
        > maximum
    {
        return Err(error("JSON 文件超过备份限制"));
    }
    let bytes = fs::read(path).map_err(|_| error("无法读取备份文件"))?;
    serde_json::from_slice(bytes.strip_prefix(&[239, 187, 191]).unwrap_or(&bytes))
        .map_err(|_| error("备份 JSON 格式无效"))
}
fn write_json(path: &Path, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| error("备份 JSON 无法保存"))?;
    if bytes.len() as u64 > MAX_JSON {
        return Err(error("JSON 文件超过备份限制"));
    }
    storage::atomic(path, &bytes)
}
fn digest(path: &Path) -> Result<(u64, String)> {
    regular(path)?;
    let mut input = File::open(path).map_err(|_| error("无法读取备份文件"))?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|_| error("读取备份文件失败"))?;
        if count == 0 {
            break;
        }
        size = size.checked_add(count as u64).ok_or("备份文件超出限制")?;
        if size > MAX_TOTAL {
            return Err(error("备份文件超出 32 GiB 限制"));
        }
        hasher.update(&buffer[..count]);
    }
    Ok((size, format!("{:x}", hasher.finalize())))
}
fn copy_atomic(source: &Path, target: &Path) -> Result<()> {
    regular(source)?;
    if target.exists() {
        regular(target)?;
    }
    let parent = target.parent().ok_or("备份目标无效")?;
    safe_directory(parent)?;
    let mut temp = NamedTempFile::new_in(parent).map_err(|_| error("无法创建备份临时文件"))?;
    std::io::copy(
        &mut File::open(source).map_err(|_| error("无法读取备份文件"))?,
        temp.as_file_mut(),
    )
    .map_err(|_| error("复制备份失败，可能磁盘空间不足"))?;
    temp.as_file()
        .sync_all()
        .map_err(|_| error("同步备份文件失败"))?;
    temp.persist(target)
        .map_err(|_| error("提交备份文件失败"))?;
    Ok(())
}
fn readonly(path: &Path) -> Result<Connection> {
    regular(path)?;
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| error("备份数据库无法打开"))?;
    conn.busy_timeout(Duration::from_secs(10))
        .map_err(|_| error("备份数据库不可用"))?;
    conn.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")
        .map_err(|_| error("备份数据库不可用"))?;
    Ok(conn)
}
fn snapshot(source: &Path, output: &Path) -> Result<()> {
    regular(source)?;
    let conn = Connection::open_with_flags(
        source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| error("备份数据库无法打开"))?;
    conn.busy_timeout(Duration::from_secs(10))
        .map_err(|_| error("备份数据库不可用"))?;
    // A separate connection is required: VACUUM cannot run inside ArchiveSnapshot's BEGIN.
    conn.execute("VACUUM INTO ?1", [output.to_string_lossy().as_ref()])
        .map_err(|_| error("数据库快照失败，请稍后重试并检查磁盘空间"))?;
    File::options()
        .write(true)
        .open(output)
        .and_then(|f| f.sync_all())
        .map_err(|_| error("数据库快照同步失败"))?;
    Ok(())
}
fn text(value: &Value, maximum: usize) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.chars().count() <= maximum && !s.chars().any(|c| c == '\0'))
}
fn timezone(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.is_ascii()
        && s.split('/').all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_+-".contains(&b))
                && p.bytes().any(|b| b.is_ascii_alphabetic())
        })
}

fn portable_config(raw: &Value) -> Result<Value> {
    if !raw.is_object() {
        return Err(error("备份设置格式无效"));
    }
    let mut config = json!({"anchors":{},"anchor_metadata":{},"anchor_tags":{},"pc_unsupported_rids":[],"debug_logging":false,"startup_monitoring":true,"time_display":{"timezone":"Asia/Shanghai"}});
    for name in CONFIG_KEYS {
        if let Some(value) = raw.get(name) {
            config[name] = value.clone();
        }
    }
    config["anchor_tags"] = crate::anchor_tags::canonical(&config["anchor_tags"])?;
    // Only templates travel. Absolute download paths are specific to this machine.
    if let Some(naming) = config.get_mut("download_naming") {
        let directory = naming["directory_template"]
            .as_str()
            .ok_or("下载目录规则无效")?
            .to_owned();
        let filename = naming["filename_template"]
            .as_str()
            .ok_or("下载文件规则无效")?
            .to_owned();
        *naming = json!({"directory_template":directory,"filename_template":filename});
    }
    validate_config(&config)?;
    Ok(config)
}
fn validate_config(config: &Value) -> Result<()> {
    let object = config.as_object().ok_or("备份设置格式无效")?;
    if object.keys().any(|k| !CONFIG_KEYS.contains(&k.as_str())) {
        return Err(error("备份包含不允许迁移的设置"));
    }
    let anchors = config["anchors"]
        .as_object()
        .filter(|a| a.len() <= 500)
        .ok_or("备份主播列表无效")?;
    if anchors
        .iter()
        .any(|(id, name)| !storage::id(id) || !text(name, 200))
    {
        return Err(error("备份主播列表无效"));
    }
    let metadata = config["anchor_metadata"]
        .as_object()
        .filter(|a| a.len() <= 500)
        .ok_or("备份主播身份无效")?;
    for (id, row) in metadata {
        if !storage::id(id)
            || !row.is_object()
            || row
                .as_object()
                .unwrap()
                .keys()
                .any(|k| !matches!(k.as_str(), "anchor_id" | "session_id"))
            || ["anchor_id", "session_id"].iter().any(|key| {
                row.get(key)
                    .is_some_and(|v| !v.as_str().is_some_and(|s| s.is_empty() || storage::id(s)))
            })
        {
            return Err(error("备份主播身份无效"));
        }
    }
    crate::anchor_tags::validate(&config["anchor_tags"])?;
    if config["pc_unsupported_rids"]
        .as_array()
        .is_none_or(|a| a.len() > 500 || a.iter().any(|v| !v.as_str().is_some_and(storage::id)))
        || !config["debug_logging"].is_boolean()
        || !config["startup_monitoring"].is_boolean()
    {
        return Err(error("备份显示设置无效"));
    }
    if !config["time_display"]["timezone"]
        .as_str()
        .is_some_and(timezone)
        || config["time_display"]
            .as_object()
            .is_none_or(|o| o.len() != 1)
    {
        return Err(error("备份时区无效"));
    }
    if let Some(provider) = config.get("highlight_provider") {
        if provider.as_object().is_none_or(|o| {
            o.len() > 2
                || o.keys()
                    .any(|k| !matches!(k.as_str(), "base_url" | "model"))
        }) || !provider["model"]
            .as_str()
            .is_some_and(|s| s.len() <= 256 && !s.chars().any(char::is_control))
        {
            return Err(error("备份模型设置无效"));
        }
        let url = url::Url::parse(provider["base_url"].as_str().ok_or("备份 API 地址无效")?)
            .map_err(|_| error("备份 API 地址无效"))?;
        if !matches!(url.scheme(), "https" | "http")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(error("备份 API 地址不可含凭据"));
        }
    }
    if let Some(naming) = config.get("download_naming") {
        if naming.as_object().is_none_or(|o| {
            o.len() != 2
                || o.keys()
                    .any(|k| !matches!(k.as_str(), "directory_template" | "filename_template"))
        }) {
            return Err(error("备份下载规则无效"));
        }
        for key in ["directory_template", "filename_template"] {
            let s = naming[key]
                .as_str()
                .filter(|s| s.len() <= 2048 && !s.chars().any(char::is_control))
                .ok_or("备份下载规则无效")?;
            if key == "filename_template" && (s.is_empty() || s.contains(['/', '\\']))
                || s.starts_with(['/', '\\'])
                || s.contains(':')
                || s.split(['/', '\\']).any(|p| matches!(p, "." | ".."))
            {
                return Err(error("备份下载规则不能包含绝对路径或跳转"));
            }
            let mut braces = 0;
            for c in s.chars() {
                if c == '{' {
                    braces += 1;
                }
                if c == '}' {
                    braces -= 1;
                }
                if !(0..=1).contains(&braces) {
                    return Err(error("备份下载规则括号无效"));
                }
            }
            if braces != 0 {
                return Err(error("备份下载规则括号无效"));
            }
        }
    }
    Ok(())
}
fn validate_preferences(preferences: &Value) -> Result<()> {
    let object = preferences
        .as_object()
        .filter(|o| o.len() <= 10_000)
        .ok_or("备份界面偏好无效")?;
    if serde_json::to_vec(preferences)
        .map_err(|_| error("备份界面偏好无效"))?
        .len() as u64
        > MAX_JSON
    {
        return Err(error("备份界面偏好超出限制"));
    }
    for (key, value) in object {
        let value = value.as_str().ok_or("备份界面偏好须为字符串")?;
        let valid = match key.as_str() {
            "livescope.appearance.font" => matches!(value, "douyin" | "chill"),
            "livescope.updates.automatic" => matches!(value, "true" | "false"),
            _ => {
                key.strip_prefix("livescope.replay-offset.")
                    .and_then(|s| s.split_once(':'))
                    .is_some_and(|(a, s)| storage::id(a) && storage::id(s))
                    && value
                        .parse::<f64>()
                        .is_ok_and(|n| n.is_finite() && n.abs() <= 86400.)
            }
        };
        if !valid {
            return Err(error("备份包含不允许恢复的界面偏好"));
        }
    }
    Ok(())
}
fn validate_marks(value: &Value) -> Result<usize> {
    if value["format"] != 1 || value.as_object().is_none_or(|o| o.len() != 2) {
        return Err(error("打点备份格式无效"));
    }
    let marks = value["marks"]
        .as_array()
        .filter(|a| a.len() <= 10_000)
        .ok_or("打点备份超出限制")?;
    let mut ids = BTreeSet::new();
    for value in marks {
        let mark: crate::session_marks::SessionMark =
            serde_json::from_value(value.clone()).map_err(|_| error("打点备份记录无效"))?;
        if mark.id.len() != 32
            || !mark.id.bytes().all(|b| b.is_ascii_hexdigit())
            || !ids.insert(mark.id.clone())
            || ![&mark.anchor_id, &mark.session_id, &mark.room_id]
                .iter()
                .all(|s| storage::id(s))
            || !mark.marked_at_epoch.is_finite()
            || mark.marked_at_epoch <= 0.
            || mark.title.chars().count() > 80
            || mark.note.chars().count() > 2000
            || mark.anchor_name.chars().count() > 120
            || mark
                .broadcast_started_at
                .is_some_and(|t| !t.is_finite() || t <= 0.)
            || value.get("request_key").is_some_and(|v| {
                !v.as_str().is_some_and(|s| {
                    !s.is_empty()
                        && s.len() <= 80
                        && s.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
                })
            })
        {
            return Err(error("打点备份记录无效"));
        }
        for field in ["marked_at", "created_at", "updated_at"] {
            if !value[field]
                .as_str()
                .is_some_and(|s| chrono::DateTime::parse_from_rfc3339(s).is_ok())
            {
                return Err(error("打点备份时间无效"));
            }
        }
    }
    Ok(marks.len())
}
fn validate_habits(value: &Value) -> Result<()> {
    if value["version"] != 1 || value.as_object().is_none_or(|o| o.len() != 3) {
        return Err(error("开播偏好备份无效"));
    }
    let aliases = value["aliases"]
        .as_object()
        .filter(|o| o.len() <= 10_000)
        .ok_or("开播偏好备份无效")?;
    if aliases
        .iter()
        .any(|(id, v)| !storage::id(id) || !v.as_str().is_some_and(storage::id))
    {
        return Err(error("开播偏好身份无效"));
    }
    for (id, slots) in value["anchors"]
        .as_object()
        .filter(|o| o.len() <= 10_000)
        .ok_or("开播偏好备份无效")?
    {
        if !storage::id(id)
            || slots.as_object().is_none_or(|o| {
                o.len() > 48
                    || o.iter().any(|(k, v)| {
                        k.parse::<u8>().map_or(true, |n| n >= 48) || v.as_i64().is_none()
                    })
            })
        {
            return Err(error("开播偏好时间无效"));
        }
    }
    Ok(())
}

fn columns(conn: &Connection, name: &str, required: &[&str]) -> Result<()> {
    let kind: String = conn
        .query_row(
            "SELECT type FROM sqlite_schema WHERE name=?1",
            [name],
            |r| r.get(0),
        )
        .map_err(|_| error("备份缺少核心数据表"))?;
    if kind != "table" {
        return Err(error("备份核心数据表无效"));
    }
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info({name})"))
        .map_err(|_| error("备份数据库结构无效"))?;
    let actual = statement
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(|_| error("备份数据库结构无效"))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(|_| error("备份数据库结构无效"))?;
    if required.iter().any(|name| !actual.contains(*name)) {
        return Err(error("备份数据库核心列缺失"));
    }
    Ok(())
}

#[derive(Clone, PartialEq)]
struct TableShape {
    name: String,
    columns: Vec<(String, String, i64, i64)>,
}
#[derive(Clone)]
struct DatabaseShape {
    tables: Vec<TableShape>,
    indexes: BTreeSet<String>,
}
fn database_shape(connection: &Connection) -> Result<DatabaseShape> {
    let names = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
        .map_err(|_| error("数据库表结构不可读"))?
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|_| error("数据库表结构不可读"))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| error("数据库表结构不可读"))?;
    let mut tables = Vec::new();
    for name in names {
        if matches!(name.as_str(), "sqlite_stat1" | "sqlite_stat4") {
            continue;
        }
        let mut statement = connection
            .prepare("SELECT name,type,\"notnull\",pk FROM pragma_table_xinfo(?1) ORDER BY cid")
            .map_err(|_| error("数据库列结构不可读"))?;
        let columns = statement
            .query_map([&name], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .map_err(|_| error("数据库列结构不可读"))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| error("数据库列结构不可读"))?;
        tables.push(TableShape { name, columns });
    }
    let indexes = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type='index'")
        .map_err(|_| error("数据库索引不可读"))?
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|_| error("数据库索引不可读"))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(|_| error("数据库索引不可读"))?;
    Ok(DatabaseShape { tables, indexes })
}
fn expected_shape(messages: bool) -> Result<&'static DatabaseShape> {
    static MESSAGES: OnceLock<Result<DatabaseShape>> = OnceLock::new();
    static CLIPS: OnceLock<Result<DatabaseShape>> = OnceLock::new();
    let cell = if messages { &MESSAGES } else { &CLIPS };
    cell.get_or_init(|| {
        let temp = tempfile::tempdir().map_err(|_| error("无法创建备份校验环境"))?;
        let name = if messages {
            let archive =
                qingkan_core::archive::ArchiveStore::open(temp.path(), Default::default())
                    .map_err(|_| error("无法建立消息备份结构标准"))?;
            if !archive.close(Duration::from_secs(5)) {
                return Err(error("消息备份结构校验未完成"));
            }
            "messages.sqlite3"
        } else {
            drop(crate::clip_history::ClipHistory::open(temp.path())?);
            "clips.sqlite3"
        };
        database_shape(&readonly(&temp.path().join(name))?)
    })
    .as_ref()
    .map_err(Clone::clone)
}
fn check_complete_schema(connection: &Connection, messages: bool) -> Result<()> {
    let actual = database_shape(connection)?;
    let expected = expected_shape(messages)?;
    if actual.tables != expected.tables || actual.indexes != expected.indexes {
        return Err(error("备份数据库表、列或索引与当前应用不兼容"));
    }
    Ok(())
}
fn validate_db(path: &Path, messages: bool) -> Result<Value> {
    let conn = readonly(path)?;
    let check: String = conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(|_| error("备份数据库校验失败"))?;
    if check != "ok" {
        return Err(error("备份数据库已损坏"));
    }
    if conn
        .prepare("PRAGMA foreign_key_check")
        .and_then(|mut s| s.exists([]))
        .map_err(|_| error("备份数据库关联校验失败"))?
    {
        return Err(error("备份数据库关联记录损坏"));
    }
    let unsafe_schema: i64 = conn.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE type IN ('trigger','view') OR lower(sql) LIKE '%create virtual table%'",[],|r|r.get(0)).map_err(|_|error("备份数据库结构无效"))?;
    if unsafe_schema != 0 {
        return Err(error("备份数据库包含不支持的动态结构"));
    }
    check_complete_schema(&conn, messages)?;
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|_| error("备份数据库版本不可读"))?;
    if messages {
        if version != qingkan_core::archive::SCHEMA_VERSION {
            return Err(error("消息备份版本不兼容，请使用匹配版本的应用"));
        }
        for (table, required) in [
            ("anchors", &["anchor_id", "anchor_name", "web_rid"][..]),
            (
                "sessions",
                &["anchor_id", "session_id", "first_seen", "last_seen"][..],
            ),
            (
                "messages",
                &[
                    "id",
                    "anchor_id",
                    "session_id",
                    "message_type",
                    "message_time_epoch",
                    "received_at_epoch",
                    "payload_json",
                ][..],
            ),
            (
                "session_monitor_durations",
                &["anchor_id", "session_id", "accumulated_seconds"][..],
            ),
            (
                "quality_registry",
                &["anchor_id", "session_id", "generation"][..],
            ),
            ("quality_settings", &["anchor_id", "config"][..]),
            (
                "quality_manual",
                &["anchor_id", "session_id", "content"][..],
            ),
            (
                "quality_results",
                &["message_id", "generation", "verdict"][..],
            ),
        ] {
            columns(&conn, table, required)?;
        }
        let count = |table: &str| {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .map_err(|_| error("备份记录统计失败"))
        };
        let sessions:i64=conn.query_row("SELECT COUNT(*) FROM(SELECT anchor_id,session_id FROM sessions UNION SELECT anchor_id,session_id FROM quality_registry)",[],|r|r.get(0)).map_err(|_|error("备份场次统计失败"))?;
        Ok(json!({"messages":count("messages")?,"anchors":count("anchors")?,"sessions":sessions}))
    } else {
        if version != 0 {
            return Err(error("下载备份版本不兼容"));
        }
        columns(
            &conn,
            "clip_jobs",
            &[
                "id",
                "status",
                "created_day",
                "created_epoch",
                "path_key",
                "request_key",
                "search_text",
                "payload",
            ],
        )?;
        columns(&conn, "clip_history_meta", &["key", "value"])?;
        let oversized: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM clip_jobs WHERE length(CAST(payload AS BLOB))>?1 OR length(CAST(id AS BLOB))!=32)",[MAX_JSON as i64],|r|r.get(0)).map_err(|_|error("下载备份记录不可读"))?;
        if oversized {
            return Err(error("下载备份记录超出限制"));
        }
        let mut statement = conn
            .prepare("SELECT id,payload FROM clip_jobs")
            .map_err(|_| error("下载备份不可读"))?;
        let mut rows = statement.query([]).map_err(|_| error("下载备份不可读"))?;
        let mut count = 0;
        while let Some(row) = rows.next().map_err(|_| error("下载备份不可读"))? {
            let id: String = row.get(0).map_err(|_| error("下载备份记录无效"))?;
            let raw: String = row.get(1).map_err(|_| error("下载备份记录无效"))?;
            if raw.len() as u64 > MAX_JSON {
                return Err(error("下载备份记录超出限制"));
            }
            let job: crate::clips::ClipJob =
                serde_json::from_str(&raw).map_err(|_| error("下载备份记录无效"))?;
            if job.id != id
                || id.len() != 32
                || !id.bytes().all(|b| b.is_ascii_hexdigit())
                || !storage::id(&job.session_id)
                || !job.requested_start_seconds.is_finite()
                || !job.requested_end_seconds.is_finite()
                || job.requested_start_seconds < 0.
                || !(job.requested_end_seconds > job.requested_start_seconds
                    || job.mode == "full_session"
                        && job.requested_start_seconds == 0.
                        && job.requested_end_seconds == 0.)
                || chrono::DateTime::parse_from_rfc3339(&job.created_at).is_err()
            {
                return Err(error("下载备份记录无效"));
            }
            count += 1;
        }
        Ok(json!({"downloads":count}))
    }
}

fn dataset_summary(directory: &Path, manifest: &Manifest) -> Result<Value> {
    let mut counts = validate_db(&directory.join("messages.sqlite3"), true)?;
    counts["downloads"] =
        validate_db(&directory.join("clips.sqlite3"), false)?["downloads"].clone();
    let config = read_json(&directory.join("config.json"), MAX_JSON)?;
    validate_config(&config)?;
    counts["monitored_anchors"] = json!(config["anchors"].as_object().unwrap().len());
    counts["marks"] = json!(validate_marks(&read_json(
        &directory.join("session-marks.json"),
        MAX_JSON
    )?)?);
    validate_habits(&read_json(
        &directory.join("polling-habits.json"),
        MAX_JSON,
    )?)?;
    validate_preferences(&read_json(&directory.join("preferences.json"), MAX_JSON)?)?;
    let dataset = read_json(&directory.join("dataset.json"), MAX_JSON)?;
    if dataset != json!({"format":1,"credentials":"excluded","includes_media":false}) {
        return Err(error("数据包迁移策略不受支持"));
    }
    Ok(
        json!({"format":1,"app_version":manifest.app_version,"created_at":manifest.created_at,"counts":counts,"includes_media":false,"credentials":"excluded","import_mode":"merge","warnings":["增量导入会保留本地数据，同一记录不会重复添加。","API 密钥、代理凭据与登录信息保留当前设备设置。","备份不包含已下载 MP4、缓存、日志与应用程序。","仅新导入的下载记录会使用当前设备的数据目录。"]}),
    )
}
fn entries(directory: &Path) -> Result<Vec<Entry>> {
    FILES
        .iter()
        .map(|path| {
            let (bytes, sha256) = digest(&directory.join(path))?;
            Ok(Entry {
                path: (*path).into(),
                bytes,
                sha256,
            })
        })
        .collect()
}
fn validate_manifest(manifest: &Manifest) -> Result<()> {
    if manifest.format != FORMAT
        || manifest.version != 1
        || manifest.files.len() != FILES.len()
        || manifest.app_version.len() > 80
        || chrono::DateTime::parse_from_rfc3339(&manifest.created_at).is_err()
    {
        return Err(error("备份格式或版本不受支持"));
    }
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    for entry in &manifest.files {
        if !FILES.contains(&entry.path.as_str())
            || !names.insert(entry.path.as_str())
            || entry.sha256.len() != 64
            || !entry
                .sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || entry.bytes > MAX_TOTAL
            || entry.path.ends_with(".json") && entry.bytes > MAX_JSON
        {
            return Err(error("备份清单文件无效"));
        }
        total = total.checked_add(entry.bytes).ok_or("备份大小超出限制")?;
        if total > MAX_TOTAL {
            return Err(error("备份展开后超过 32 GiB 限制"));
        }
    }
    Ok(())
}
fn verify_entries(directory: &Path, manifest: &Manifest) -> Result<()> {
    validate_manifest(manifest)?;
    for entry in &manifest.files {
        let (bytes, hash) = digest(&directory.join(&entry.path))?;
        if bytes != entry.bytes || hash != entry.sha256 {
            return Err(error("备份文件摘要不匹配，未修改当前数据"));
        }
    }
    Ok(())
}

impl DataBackup {
    pub fn export(directory: &Path, output: &Path, preferences: Value) -> Result<Value> {
        Self::export_selected(directory, output, preferences, None)
    }
    pub fn anchors(directory: &Path) -> Result<Value> {
        merge::anchors(directory)
    }
    pub fn export_selected(
        directory: &Path,
        output: &Path,
        preferences: Value,
        anchor_ids: Option<Vec<String>>,
    ) -> Result<Value> {
        safe_directory(directory)?;
        validate_preferences(&preferences)?;
        let parent = output
            .parent()
            .filter(|p| p.is_dir())
            .ok_or("请选择已有的备份保存目录")?;
        safe_directory(parent)?;
        if output.exists() {
            return Err(error("备份文件已存在，请选择新文件名"));
        }
        let staged = tempfile::Builder::new()
            .prefix(".backup-export-")
            .tempdir_in(directory)
            .map_err(|_| error("无法创建备份暂存目录"))?;
        snapshot(
            &directory.join("messages.sqlite3"),
            &staged.path().join("messages.sqlite3"),
        )?;
        snapshot(
            &directory.join("clips.sqlite3"),
            &staged.path().join("clips.sqlite3"),
        )?;
        validate_db(&staged.path().join("clips.sqlite3"), false)?;
        rebase_clips(&staged.path().join("clips.sqlite3"), Path::new(""))?;
        let config = portable_config(&read_json(&directory.join("config.json"), MAX_JSON)?)?;
        write_json(&staged.path().join("config.json"), &config)?;
        for (name, default) in [
            ("session-marks.json", json!({"format":1,"marks":[]})),
            (
                "polling-habits.json",
                json!({"version":1,"aliases":{},"anchors":{}}),
            ),
        ] {
            let source = directory.join(name);
            if source.exists() {
                copy_atomic(&source, &staged.path().join(name))?;
            } else {
                write_json(&staged.path().join(name), &default)?;
            }
        }
        write_json(&staged.path().join("preferences.json"), &preferences)?;
        write_json(
            &staged.path().join("dataset.json"),
            &json!({"format":1,"credentials":"excluded","includes_media":false}),
        )?;
        if let Some(ids) = anchor_ids {
            merge::filter(staged.path(), ids)?;
        }
        let manifest = Manifest {
            format: FORMAT.into(),
            version: 1,
            app_version: VERSION.into(),
            created_at: storage::now(),
            files: entries(staged.path())?,
        };
        validate_manifest(&manifest)?;
        let mut summary = dataset_summary(staged.path(), &manifest)?;
        let mut temporary = NamedTempFile::new_in(parent).map_err(|_| error("无法创建备份文件"))?;
        {
            let mut writer = ZipWriter::new(temporary.as_file_mut());
            let options = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .compression_level(Some(3))
                .large_file(true)
                .unix_permissions(0o600);
            writer
                .start_file("manifest.json", options)
                .map_err(|_| error("写入备份清单失败"))?;
            writer
                .write_all(&serde_json::to_vec(&manifest).map_err(|_| error("备份清单无效"))?)
                .map_err(|_| error("写入备份清单失败"))?;
            for name in FILES {
                writer
                    .start_file(name, options)
                    .map_err(|_| error("创建备份条目失败"))?;
                std::io::copy(
                    &mut File::open(staged.path().join(name))
                        .map_err(|_| error("读取备份数据失败"))?,
                    &mut writer,
                )
                .map_err(|_| error("写入备份包失败，可能磁盘空间不足"))?;
            }
            writer.finish().map_err(|_| error("完成备份包失败"))?;
        }
        let bytes = temporary
            .as_file()
            .metadata()
            .map_err(|_| error("读取备份包大小失败"))?
            .len();
        if bytes > MAX_TOTAL {
            return Err(error("备份压缩包超过 32 GiB 限制"));
        }
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| error("同步备份包失败"))?;
        temporary
            .persist_noclobber(output)
            .map_err(|_| error("保存备份包失败，未覆盖已有文件"))?;
        summary["bytes"] = json!(bytes);
        summary["path"] = json!(output.to_string_lossy());
        Ok(summary)
    }

    pub fn prepare(input: &Path, workspace: &Path) -> Result<PreparedBackup> {
        regular(input)?;
        if fs::metadata(input)
            .map_err(|_| error("备份包不可读"))?
            .len()
            > MAX_TOTAL
        {
            return Err(error("备份压缩包超过 32 GiB 限制"));
        }
        safe_directory(workspace)?;
        fs::create_dir_all(workspace).map_err(|_| error("无法创建导入暂存目录"))?;
        let staged = tempfile::Builder::new()
            .prefix(".backup-import-")
            .tempdir_in(workspace)
            .map_err(|_| error("无法创建导入暂存目录"))?;
        let mut archive = ZipArchive::new(File::open(input).map_err(|_| error("备份包不可读"))?)
            .map_err(|_| error("备份包 ZIP 格式无效"))?;
        if archive.len() != FILES.len() + 1 {
            return Err(error("备份包条目数量无效"));
        }
        let mut names = BTreeSet::new();
        let mut total = 0u64;
        for i in 0..archive.len() {
            let file = archive
                .by_index(i)
                .map_err(|_| error("备份 ZIP 条目不可读"))?;
            let name = file.name();
            let kind = file.unix_mode().unwrap_or(0) & 0o170000;
            if !(name == "manifest.json" || FILES.contains(&name))
                || !names.insert(name.to_owned())
                || file.is_dir()
                || !matches!(kind, 0 | 0o100000)
                || file.size() > MAX_TOTAL
                || name.ends_with(".json")
                    && file.size()
                        > if name == "manifest.json" {
                            MAX_MANIFEST
                        } else {
                            MAX_JSON
                        }
            {
                return Err(error("备份包含重复、越界、链接或不允许的条目"));
            }
            total = total.checked_add(file.size()).ok_or("备份展开大小超限")?;
            if total > MAX_TOTAL + MAX_MANIFEST {
                return Err(error("备份展开后超过 32 GiB 限制"));
            }
        }
        let manifest: Manifest = {
            let file = archive
                .by_name("manifest.json")
                .map_err(|_| error("备份缺少清单"))?;
            let mut bytes = Vec::new();
            file.take(MAX_MANIFEST + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| error("读取备份清单失败"))?;
            if bytes.len() as u64 > MAX_MANIFEST {
                return Err(error("备份清单过大"));
            }
            serde_json::from_slice(&bytes).map_err(|_| error("备份清单格式无效"))?
        };
        validate_manifest(&manifest)?;
        for entry in &manifest.files {
            let mut file = archive
                .by_name(&entry.path)
                .map_err(|_| error("备份缺少数据文件"))?;
            if file.size() != entry.bytes {
                return Err(error("备份文件长度与清单不匹配"));
            }
            let mut output = File::options()
                .create_new(true)
                .write(true)
                .open(staged.path().join(&entry.path))
                .map_err(|_| error("无法写入导入暂存数据"))?;
            let mut hasher = Sha256::new();
            let mut count = 0u64;
            let mut buffer = [0u8; 65536];
            loop {
                let n = file
                    .read(&mut buffer)
                    .map_err(|_| error("备份 CRC 或压缩数据校验失败"))?;
                if n == 0 {
                    break;
                }
                count = count.checked_add(n as u64).ok_or("解压长度超限")?;
                if count > entry.bytes {
                    return Err(error("备份实际解压长度超过清单限制"));
                }
                hasher.update(&buffer[..n]);
                output
                    .write_all(&buffer[..n])
                    .map_err(|_| error("导入暂存空间不足"))?;
            }
            if count != entry.bytes || format!("{:x}", hasher.finalize()) != entry.sha256 {
                return Err(error("备份摘要校验失败，未修改当前数据"));
            }
            output.sync_all().map_err(|_| error("导入暂存同步失败"))?;
        }
        let summary = dataset_summary(staged.path(), &manifest)?;
        Ok(PreparedBackup {
            directory: staged,
            manifest,
            summary,
            token: format!("{:032x}", rand::random::<u128>()),
            used: AtomicBool::new(false),
        })
    }
}

fn merge_config(imported: &Value, local: &Value, directory: &Path) -> Result<Value> {
    validate_config(imported)?;
    if !local.is_object() {
        return Err(error("当前设置文件无效，不能安全保留本机凭据"));
    }
    let mut output = local.clone();
    merge::config(&mut output, imported)?;
    if output.get("download_naming").is_none() {
        let templates = imported.get("download_naming");
        output["download_naming"] = json!({"root_directory":directory.join("clips").to_string_lossy(),"directory_template":templates.and_then(|v|v["directory_template"].as_str()).unwrap_or(""),"filename_template":templates.and_then(|v|v["filename_template"].as_str()).unwrap_or("{session_id}-{start_ms}-{job_id8}.mp4")});
    }
    output["schema_version"] = json!(2);
    Ok(output)
}
fn rebase_clips(path: &Path, directory: &Path) -> Result<()> {
    let mut conn = Connection::open(path).map_err(|_| error("无法重定位下载记录"))?;
    conn.execute_batch("PRAGMA journal_mode=DELETE;")
        .map_err(|_| error("无法重定位下载记录"))?;
    let tx = conn
        .transaction()
        .map_err(|_| error("无法重定位下载记录"))?;
    {
        let mut statement = tx
            .prepare("SELECT payload FROM clip_jobs ORDER BY id")
            .map_err(|_| error("下载记录不可读"))?;
        let mut rows = statement.query([]).map_err(|_| error("下载记录不可读"))?;
        while let Some(row) = rows.next().map_err(|_| error("下载记录不可读"))? {
            let raw: String = row.get(0).map_err(|_| error("下载记录不可读"))?;
            let mut job: crate::clips::ClipJob =
                serde_json::from_str(&raw).map_err(|_| error("下载记录无效"))?;
            job.output_path = directory
                .join("clips")
                .join(format!("imported-{}.mp4", job.id))
                .to_string_lossy()
                .into_owned();
            job.source_identity = None;
            job.source_fingerprint = None;
            if matches!(job.status.as_str(), "queued" | "running") {
                job.status = "failed".into();
                job.phase = "interrupted".into();
                job.error = Some("从备份恢复，下载未完成，可重试".into());
            }
            let payload = serde_json::to_string(&job).map_err(|_| error("下载记录无效"))?;
            tx.execute(
                "UPDATE clip_jobs SET status=?1,path_key=?2,search_text=?3,payload=?4 WHERE id=?5",
                rusqlite::params![
                    job.status,
                    crate::clip_history::path_key(&job.output_path),
                    format!(
                        "{}\n{}\n{}\n{}",
                        job.title, job.anchor_name, job.session_id, job.output_path
                    )
                    .to_lowercase(),
                    payload,
                    job.id
                ],
            )
            .map_err(|_| error("下载记录重定位失败"))?;
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO clip_history_meta(key,value) VALUES('legacy_json_v1',?1)",
        [storage::now()],
    )
    .map_err(|_| error("下载迁移状态不可写"))?;
    tx.commit().map_err(|_| error("下载记录重定位失败"))?;
    Ok(())
}
fn remove_file(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            regular(path)?;
            fs::remove_file(path).map_err(|_| error("无法替换当前数据文件"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(error("当前数据文件不可访问")),
    }
}
fn save_journal(directory: &Path, journal: &Journal) -> Result<()> {
    storage::atomic(
        &directory.join(JOURNAL),
        &serde_json::to_vec(journal).map_err(|_| error("恢复事务无效"))?,
    )
}

impl PreparedBackup {
    pub fn summary(&self) -> Value {
        self.summary.clone()
    }
    pub fn token(&self) -> String {
        self.token.clone()
    }
    pub fn restore(&self, directory: &Path) -> Result<Value> {
        self.restore_inner(directory, None)
    }
    fn restore_inner(&self, directory: &Path, fail_after: Option<usize>) -> Result<Value> {
        if self.used.swap(true, Ordering::AcqRel) {
            return Err(error("此备份预览已失效，请重新选择备份"));
        }
        safe_directory(directory)?;
        fs::create_dir_all(directory).map_err(|_| error("数据目录不可写"))?;
        recover_restore(directory)?;
        verify_entries(self.directory.path(), &self.manifest)?;
        let preferences = read_json(&self.directory.path().join("preferences.json"), MAX_JSON)?;
        validate_preferences(&preferences)?;
        let local = if directory.join("config.json").exists() {
            read_json(&directory.join("config.json"), MAX_JSON)?
        } else {
            json!({})
        };
        let staging = tempfile::Builder::new()
            .prefix(".restore-stage-")
            .tempdir_in(directory)
            .map_err(|_| error("无法创建恢复暂存目录"))?;
        // Opening a read-only WAL database may create empty sidecars. They are not original
        // data and must not accidentally become part of the recovery snapshot.
        let absent_sidecars = [
            "messages.sqlite3-wal",
            "messages.sqlite3-shm",
            "clips.sqlite3-wal",
            "clips.sqlite3-shm",
        ]
        .into_iter()
        .filter_map(|name| match directory.join(name).try_exists() {
            Ok(false) => Some(Ok(name)),
            Ok(true) => None,
            Err(_) => Some(Err(error("当前数据库侧文件不可访问"))),
        })
        .collect::<Result<Vec<_>>>()?;
        let staged = merge::stage(directory, self.directory.path(), staging.path(), &local);
        for name in absent_sidecars {
            remove_file(&directory.join(name))?;
        }
        let added = staged?;
        let backup = format!(
            "before-import-{}-{:08x}",
            chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
            rand::random::<u32>()
        );
        let backups = directory.join("backups");
        safe_directory(&backups)?;
        fs::create_dir_all(&backups).map_err(|_| error("无法创建导入前备份"))?;
        let previous = backups.join(&backup);
        fs::create_dir(&previous).map_err(|_| error("无法创建导入前备份"))?;
        let mut originals = Vec::new();
        for name in MANAGED {
            let original = directory.join(name);
            if original
                .try_exists()
                .map_err(|_| error("当前数据文件不可访问"))?
            {
                copy_atomic(&original, &previous.join(name))?;
                let (bytes, sha256) = digest(&previous.join(name))?;
                originals.push(Entry {
                    path: name.into(),
                    bytes,
                    sha256,
                });
            }
        }
        let mut journal = Journal {
            format: 1,
            phase: "applying".into(),
            backup,
            originals,
        };
        save_journal(directory, &journal)?;
        let applied = (|| -> Result<()> {
            for (index, name) in MANAGED.iter().enumerate() {
                let incoming = staging.path().join(name);
                if incoming.exists() {
                    copy_atomic(&incoming, &directory.join(name))?;
                } else {
                    remove_file(&directory.join(name))?;
                }
                if fail_after == Some(index + 1) {
                    return Err(error("模拟恢复中断"));
                }
            }
            journal.phase = "committed".into();
            save_journal(directory, &journal)?;
            Ok(())
        })();
        if let Err(failure) = applied {
            return match recover_restore(directory) {
                Ok(_) => Err(format!("{failure}；当前数据已回滚，导入前备份已保留")),
                Err(rollback) => Err(format!(
                    "{failure}；自动回滚未完成：{rollback}，请重启应用继续恢复"
                )),
            };
        }
        let cleanup_pending = remove_file(&directory.join(JOURNAL)).is_err();
        let mut result = self.summary();
        result["restored"] = json!(true);
        result["import_mode"] = json!("merge");
        result["added"] = added;
        result["before_import_path"] = json!(previous.to_string_lossy());
        result["rollback_path"] = json!(previous.to_string_lossy());
        result["preferences"] = preferences;
        if cleanup_pending {
            result["journal_cleanup_pending"] = json!(true);
            if let Some(warnings) = result["warnings"].as_array_mut() {
                warnings.push(json!("数据已恢复，事务标记将在下次启动时清理。"));
            }
        }
        Ok(result)
    }
}

/// Run before opening any Runtime or SQLite connection. An incomplete import always rolls back.
pub fn recover_restore(directory: &Path) -> Result<Value> {
    safe_directory(directory)?;
    let path = directory.join(JOURNAL);
    if !path.try_exists().map_err(|_| error("恢复事务不可访问"))? {
        return Ok(json!({"recovered":false}));
    }
    let journal: Journal = serde_json::from_value(read_json(&path, MAX_MANIFEST)?)
        .map_err(|_| error("恢复事务损坏，原数据已保留"))?;
    if journal.format != 1
        || !matches!(journal.phase.as_str(), "applying" | "committed")
        || !journal.backup.starts_with("before-import-")
        || journal.backup.len() > 100
        || !journal
            .backup
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || journal.originals.len() > MANAGED.len()
    {
        return Err(error("恢复事务格式不受支持"));
    }
    if journal.phase == "committed" {
        let cleanup_pending = remove_file(&path).is_err();
        return Ok(
            json!({"recovered":false,"committed":true,"journal_cleanup_pending":cleanup_pending}),
        );
    }
    let previous = directory.join("backups").join(&journal.backup);
    safe_directory(&previous)?;
    let mut originals = BTreeSet::new();
    for entry in &journal.originals {
        if !MANAGED.contains(&entry.path.as_str()) || !originals.insert(entry.path.as_str()) {
            return Err(error("恢复事务包含无效文件"));
        }
        let (bytes, hash) = digest(&previous.join(&entry.path))?;
        if bytes != entry.bytes || hash != entry.sha256 {
            return Err(error("导入前备份校验失败，停止自动恢复"));
        }
    }
    for name in MANAGED {
        if originals.contains(name) {
            copy_atomic(&previous.join(name), &directory.join(name))?;
        } else {
            remove_file(&directory.join(name))?;
        }
    }
    remove_file(&path)?;
    Ok(json!({"recovered":true,"rollback_path":previous.to_string_lossy()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qingkan_core::archive::MessageRecord;

    fn initialized() -> (TempDir, Runtime) {
        let directory = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(directory.path().into()).unwrap();
        runtime.flush_for_backup().unwrap();
        (directory, runtime)
    }
    fn record(runtime: &Runtime, id: &str) {
        runtime
            .shared
            .archive
            .record_message(
                MessageRecord {
                    anchor_id: "11".into(),
                    anchor_name: "备份主播".into(),
                    web_rid: "1".into(),
                    session_id: "21".into(),
                    sender_id: "31".into(),
                    sender_name: "测试观众".into(),
                    message_type: "chat".into(),
                    content: "WAL 中已提交的聊天".into(),
                    message_time: storage::now(),
                    received_at: storage::now(),
                    time_source: "server".into(),
                    message_id: id.into(),
                    payload: json!({"kind":"test"}),
                },
                "备份测试".into(),
                None,
            )
            .unwrap();
    }
    fn add_clip(directory: &Path) {
        let job:crate::clips::ClipJob=serde_json::from_value(json!({"id":"0123456789abcdef0123456789abcdef","session_id":"21","title":"精彩对局","status":"completed","phase":"completed","progress":100.,"downloaded_bytes":1024,"total_bytes":1024,"output_path":"Z:/old-machine/private/video.mp4","error":null,"requested_start_seconds":10.,"requested_end_seconds":40.,"actual_start_seconds":9.,"actual_end_seconds":40.,"attempt":1,"created_at":storage::now(),"updated_at":storage::now(),"anchor_id":"11","anchor_name":"备份主播","source":"manual","source_identity":"machine-specific-source","source_fingerprint":"old-fingerprint"})).unwrap();
        crate::clip_history::ClipHistory::open(directory)
            .unwrap()
            .put(&job)
            .unwrap();
    }
    fn package(directory: &Path, prefs: Value) -> std::path::PathBuf {
        let output = directory.join(format!("test-{:08x}.hbackup", rand::random::<u32>()));
        DataBackup::export(directory, &output, prefs).unwrap();
        output
    }
    fn zip_entries(path: &Path) -> Vec<(String, Vec<u8>)> {
        let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
        (0..archive.len())
            .map(|index| {
                let mut entry = archive.by_index(index).unwrap();
                let name = entry.name().to_owned();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                (name, bytes)
            })
            .collect()
    }
    fn write_zip(path: &Path, entries: &[(String, Vec<u8>)]) {
        let mut writer = ZipWriter::new(File::create(path).unwrap());
        for (name, bytes) in entries {
            writer
                .start_file(
                    name,
                    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
                )
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn live_wal_snapshot_and_roundtrip_preserve_records_preferences_and_portable_history() {
        let (source, runtime) = initialized();
        runtime
            .request("add_anchors", json!({"values":"1"}))
            .unwrap();
        let keeper = Connection::open(source.path().join("messages.sqlite3")).unwrap();
        keeper
            .execute_batch("PRAGMA wal_autocheckpoint=0; BEGIN;")
            .unwrap();
        let _: i64 = keeper
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        record(&runtime, "wal-message");
        add_clip(source.path());
        assert!(
            fs::metadata(source.path().join("messages.sqlite3-wal"))
                .unwrap()
                .len()
                > 0
        );
        runtime.flush_for_backup().unwrap();
        let preferences = json!({"livescope.appearance.font":"chill","livescope.updates.automatic":"false","livescope.replay-offset.11:21":"-12.5"});
        let output = package(source.path(), preferences.clone());
        let prepared = DataBackup::prepare(&output, &source.path().join("inspect")).unwrap();
        assert_eq!(prepared.summary()["counts"]["messages"], 1);
        assert_eq!(prepared.summary()["counts"]["downloads"], 1);
        assert_eq!(prepared.summary()["counts"]["monitored_anchors"], 1);
        assert!(prepared.summary().get("preferences").is_none());
        let copied = readonly(&prepared.directory.path().join("messages.sqlite3")).unwrap();
        assert_eq!(
            copied
                .query_row("SELECT content FROM messages", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "WAL 中已提交的聊天"
        );
        drop(copied);
        let portable = readonly(&prepared.directory.path().join("clips.sqlite3")).unwrap();
        let clip: String = portable
            .query_row("SELECT payload FROM clip_jobs", [], |r| r.get(0))
            .unwrap();
        assert!(!clip.contains("old-machine") && !clip.contains("machine-specific-source"));
        drop(portable);
        let target = tempfile::tempdir().unwrap();
        fs::write(target.path().join("clips.json"), b"old legacy history").unwrap();
        let restored = prepared.restore(target.path()).unwrap();
        assert_eq!(restored["preferences"], preferences);
        assert_eq!(restored["restored"], true);
        assert!(target.path().join("clips.json").exists());
        assert!(Path::new(restored["before_import_path"].as_str().unwrap())
            .join("clips.json")
            .exists());
        let reopened = Runtime::open(target.path().into()).unwrap();
        assert_eq!(
            reopened
                .request("recent_messages", json!({"type":"chat"}))
                .unwrap()["messages"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let job = reopened.request("clip_jobs", json!({})).unwrap()["jobs"][0].clone();
        assert!(job["output_path"]
            .as_str()
            .unwrap()
            .starts_with(target.path().to_str().unwrap()));
        reopened.shutdown().unwrap();
        keeper.execute_batch("ROLLBACK;").unwrap();
        drop(keeper);
        runtime.shutdown().unwrap();
    }

    #[test]
    fn fresh_install_empty_data_exports_and_restores_without_a_preexisting_config_or_marks() {
        let source = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(source.path().into()).unwrap();
        assert!(!source.path().join("config.json").exists());
        runtime.flush_for_backup().unwrap();
        let output = package(source.path(), json!({}));
        let prepared = DataBackup::prepare(&output, &source.path().join("inspect")).unwrap();
        assert_eq!(prepared.summary()["counts"]["messages"], 0);
        assert_eq!(prepared.summary()["counts"]["marks"], 0);
        let target = tempfile::tempdir().unwrap();
        prepared.restore(target.path()).unwrap();
        let reopened = Runtime::open(target.path().into()).unwrap();
        assert_eq!(
            reopened.request("mark_list", json!({})).unwrap()["total"],
            0
        );
        reopened.shutdown().unwrap();
        runtime.shutdown().unwrap();
    }

    #[test]
    fn portable_settings_drop_all_unknown_secrets_and_preserve_current_local_credentials_on_restore(
    ) {
        let raw = json!({"anchors":{"1":"主播"},"anchor_metadata":{},"cookie_encrypted":"old-login","anchor_cookies_encrypted":"old-anchor-login","highlight_api_key_encrypted":"old-ai","community_api_key_encrypted":"old-community","imported_proxies":[{"password":"old-proxy-password"}],"proxy_subscriptions":[{"url":"https://host/?token=secret"}],"future_secret":"must-never-export","download_naming":{"root_directory":"X:/private","directory_template":"{anchor}/{date}","filename_template":"{title}.mp4"},"highlight_provider":{"base_url":"https://imported.example/v1","model":"m"}});
        let portable = portable_config(&raw).unwrap();
        let exported = portable.to_string();
        for secret in [
            "old-login",
            "old-anchor-login",
            "old-ai",
            "old-community",
            "old-proxy-password",
            "secret",
            "X:/private",
        ] {
            assert!(!exported.contains(secret));
        }
        let local = json!({"anchors":{"9":"本机旧主播"},"highlight_api_key_encrypted":"local-ai","highlight_provider":{"base_url":"https://local.example/v1","model":"local-model"},"community_api_key_encrypted":"local-community","cookie_encrypted":"local-login","proxy_pool":{"enabled":true},"imported_proxies_encrypted":"local-proxy","future_machine_key":"local-only"});
        let merged = merge_config(&portable, &local, Path::new("C:/new-data")).unwrap();
        assert_eq!(merged["highlight_api_key_encrypted"], "local-ai");
        assert_eq!(
            merged["highlight_provider"], local["highlight_provider"],
            "imported origin must never receive the existing local key"
        );
        assert_eq!(merged["imported_proxies_encrypted"], "local-proxy");
        assert_eq!(merged["future_machine_key"], "local-only");
        assert_eq!(merged["anchors"], json!({"1":"主播","9":"本机旧主播"}));
        assert_ne!(merged["download_naming"]["root_directory"], "X:/private");
    }

    #[test]
    fn every_restore_write_boundary_rolls_back_existing_and_previously_absent_files() {
        let (source, runtime) = initialized();
        record(&runtime, "source");
        let output = package(source.path(), json!({}));
        for failure in 1..=MANAGED.len() {
            let (target, target_runtime) = initialized();
            record(&target_runtime, "local-original");
            target_runtime.shutdown().unwrap();
            drop(target_runtime);
            let original_config = b"{\"anchors\":{},\"private_machine_token\":\"keep-me\"}";
            fs::write(target.path().join("config.json"), original_config).unwrap();
            fs::write(
                target.path().join("clips.json"),
                b"original legacy clip bytes",
            )
            .unwrap();
            let before: Vec<_> = MANAGED
                .iter()
                .map(|name| ((*name).to_owned(), fs::read(target.path().join(name)).ok()))
                .collect();
            let prepared = DataBackup::prepare(&output, &source.path().join("inspect")).unwrap();
            let failed = prepared
                .restore_inner(target.path(), Some(failure))
                .unwrap_err();
            assert!(
                failed.contains("已回滚"),
                "failure boundary {failure}: {failed}"
            );
            for (name, bytes) in &before {
                assert_eq!(
                    &fs::read(target.path().join(name)).ok(),
                    bytes,
                    "boundary {failure}: {name}"
                );
            }
            assert_eq!(recover_restore(target.path()).unwrap()["recovered"], false);
            assert_eq!(recover_restore(target.path()).unwrap()["recovered"], false);
        }
        runtime.shutdown().unwrap();
    }

    #[test]
    fn crash_journal_recovers_idempotently_and_committed_journal_does_not_undo_new_data() {
        let target = tempfile::tempdir().unwrap();
        let previous = target.path().join("backups/before-import-test");
        fs::create_dir_all(&previous).unwrap();
        fs::write(previous.join("config.json"), b"original config bytes").unwrap();
        let (bytes, sha256) = digest(&previous.join("config.json")).unwrap();
        let mut journal = Journal {
            format: 1,
            phase: "applying".into(),
            backup: "before-import-test".into(),
            originals: vec![Entry {
                path: "config.json".into(),
                bytes,
                sha256,
            }],
        };
        save_journal(target.path(), &journal).unwrap();
        fs::write(target.path().join("config.json"), b"partial new config").unwrap();
        fs::write(target.path().join("clips.sqlite3"), b"new absent-before DB").unwrap();
        assert_eq!(recover_restore(target.path()).unwrap()["recovered"], true);
        assert_eq!(
            fs::read(target.path().join("config.json")).unwrap(),
            b"original config bytes"
        );
        assert!(!target.path().join("clips.sqlite3").exists());
        assert_eq!(recover_restore(target.path()).unwrap()["recovered"], false);
        journal.phase = "committed".into();
        save_journal(target.path(), &journal).unwrap();
        fs::write(target.path().join("config.json"), b"complete new config").unwrap();
        assert_eq!(recover_restore(target.path()).unwrap()["committed"], true);
        assert_eq!(
            fs::read(target.path().join("config.json")).unwrap(),
            b"complete new config"
        );
    }

    #[test]
    fn import_rejects_unsafe_zip_entries_bad_hashes_oversize_manifest_and_invalid_preferences() {
        let (source, runtime) = initialized();
        let output = package(source.path(), json!({}));
        let original = zip_entries(&output);
        for bad_name in [
            "../config.json",
            "/config.json",
            "C:/config.json",
            "unexpected.json",
        ] {
            let mut entries = original.clone();
            entries
                .iter_mut()
                .find(|(name, _)| name == "config.json")
                .unwrap()
                .0 = bad_name.into();
            let file = source.path().join("bad-name.hbackup");
            write_zip(&file, &entries);
            assert!(DataBackup::prepare(&file, source.path()).is_err());
        }
        let mut corrupt = original.clone();
        corrupt
            .iter_mut()
            .find(|(name, _)| name == "config.json")
            .unwrap()
            .1 = b"{}".to_vec();
        let file = source.path().join("bad-hash.hbackup");
        write_zip(&file, &corrupt);
        assert!(DataBackup::prepare(&file, source.path()).is_err());
        let mut oversized = original.clone();
        let manifest = &mut oversized
            .iter_mut()
            .find(|(name, _)| name == "manifest.json")
            .unwrap()
            .1;
        let mut value: Value = serde_json::from_slice(manifest).unwrap();
        value["files"][0]["bytes"] = json!(MAX_TOTAL + 1);
        *manifest = serde_json::to_vec(&value).unwrap();
        let file = source.path().join("oversized.hbackup");
        write_zip(&file, &oversized);
        assert!(DataBackup::prepare(&file, source.path()).is_err());
        for prefs in [
            json!({"other-secret":"no"}),
            json!({"livescope.appearance.font":"evil"}),
            json!({"livescope.replay-offset.11:21":"NaN"}),
            json!({"livescope.replay-offset.00:21":"2"}),
            json!({"livescope.replay-offset.11:21":"86401"}),
        ] {
            assert!(DataBackup::export(
                source.path(),
                &source.path().join("invalid-prefs.hbackup"),
                prefs
            )
            .is_err());
        }
        runtime.shutdown().unwrap();
    }

    #[test]
    fn missing_core_columns_new_versions_and_dynamic_sqlite_schema_are_rejected() {
        let (source, runtime) = initialized();
        let output = package(source.path(), json!({}));
        for operation in [
            "PRAGMA user_version=999",
            "DROP TABLE quality_meta",
            "CREATE TRIGGER bad_backup AFTER INSERT ON anchors BEGIN DELETE FROM messages; END",
        ] {
            let prepared = DataBackup::prepare(&output, &source.path().join("inspect")).unwrap();
            let conn =
                Connection::open(prepared.directory.path().join("messages.sqlite3")).unwrap();
            conn.execute_batch(operation).unwrap();
            drop(conn);
            assert!(
                validate_db(&prepared.directory.path().join("messages.sqlite3"), true).is_err(),
                "{operation}"
            );
        }
        runtime.shutdown().unwrap();
    }

    #[test]
    fn duplicate_zip_names_and_unix_symlinks_are_rejected_before_extraction() {
        let (source, runtime) = initialized();
        let output = package(source.path(), json!({}));
        let mut entries = zip_entries(&output);
        entries
            .iter_mut()
            .find(|(name, _)| name == "dataset.json")
            .unwrap()
            .0 = "confix.json".into();
        let duplicate = source.path().join("duplicate.hbackup");
        write_zip(&duplicate, &entries);
        let mut bytes = fs::read(&duplicate).unwrap();
        for index in 0..bytes.len().saturating_sub(10) {
            if &bytes[index..index + 11] == b"confix.json" {
                bytes[index..index + 11].copy_from_slice(b"config.json");
            }
        }
        fs::write(&duplicate, bytes).unwrap();
        assert!(DataBackup::prepare(&duplicate, source.path()).is_err());

        let symlink = source.path().join("symlink.hbackup");
        fs::copy(&output, &symlink).unwrap();
        let mut bytes = fs::read(&symlink).unwrap();
        let mut changed = false;
        for index in 0..bytes.len().saturating_sub(57) {
            if &bytes[index..index + 4] == b"PK\x01\x02" {
                let length = u16::from_le_bytes([bytes[index + 28], bytes[index + 29]]) as usize;
                if length == 11 && &bytes[index + 46..index + 57] == b"config.json" {
                    bytes[index + 5] = 3;
                    bytes[index + 38..index + 42]
                        .copy_from_slice(&((0o120777u32) << 16).to_le_bytes());
                    changed = true;
                    break;
                }
            }
        }
        assert!(changed);
        fs::write(&symlink, bytes).unwrap();
        assert!(DataBackup::prepare(&symlink, source.path()).is_err());
        runtime.shutdown().unwrap();
    }
}
