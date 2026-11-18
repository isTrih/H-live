//! Relative download templates. User text is data, never a filesystem path or format program.
use crate::{
    runtime::{Runtime, Shared},
    storage::{self, Result},
};
use chrono::{DateTime, Datelike};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

const DEFAULT_FILE: &str = "{session_id}-{start_ms}-{job_id8}.mp4";
const VARIABLES: &[(&str, &str, &str)] = &[
    ("anchor", "主播名称", "小天"),
    ("anchor_id", "主播 ID", "123456"),
    ("session_id", "场次 ID", "7698991795467832114"),
    ("title", "切片标题", "精彩互动"),
    ("date", "场次日期", "2026-10-25"),
    ("year", "年", "2026"),
    ("month", "月", "10"),
    ("day", "日", "25"),
    ("download_date", "下载任务日期", "2026-10-25"),
    ("start", "片段起点", "00-02-00"),
    ("end", "片段终点", "00-02-30"),
    ("start_ms", "起点毫秒", "120000"),
    ("end_ms", "终点毫秒", "150000"),
    ("duration", "片段秒数", "30"),
    ("job_id", "任务 ID", "0123456789abcdef0123456789abcdef"),
    ("job_id8", "任务短 ID", "01234567"),
    ("source", "来源", "打点"),
    ("bookmark_id", "打点 ID", "0123456789abcdef0123456789abcdef"),
];

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NamingSettings {
    pub root_directory: String,
    pub directory_template: String,
    pub filename_template: String,
}
impl NamingSettings {
    fn defaults(directory: &Path) -> Self {
        Self {
            root_directory: directory.join("clips").to_string_lossy().into_owned(),
            directory_template: String::new(),
            filename_template: DEFAULT_FILE.into(),
        }
    }
    pub(crate) fn load(shared: &Shared) -> Result<Self> {
        let state = shared.state.lock().map_err(|_| "下载设置不可用")?;
        match state.config.value.get("download_naming") {
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|_| "下载命名设置无效，请重新保存".into()),
            None => Ok(Self::defaults(&shared.directory)),
        }
    }
    fn validate(&self, directory: &Path) -> Result<()> {
        root(&self.root_directory, directory, false)?;
        render(self, &NamingContext::example())?;
        Ok(())
    }
}

/// All dates are company-local dates. Values never acquire path separator semantics.
pub(crate) struct NamingContext {
    pub anchor_id: String,
    pub anchor_name: String,
    pub session_id: String,
    pub session_date: String,
    pub title: String,
    pub source: String,
    pub bookmark_id: Option<String>,
    pub start: f64,
    pub end: f64,
    pub job_id: String,
    pub created_at: String,
}
impl NamingContext {
    fn example() -> Self {
        Self {
            anchor_id: "123456".into(),
            anchor_name: "小天".into(),
            session_id: "7698991795467832114".into(),
            session_date: "2026-10-25".into(),
            title: "精彩互动".into(),
            source: "bookmark".into(),
            bookmark_id: Some("0123456789abcdef0123456789abcdef".into()),
            start: 120.,
            end: 150.,
            job_id: "0123456789abcdef0123456789abcdef".into(),
            created_at: "2026-10-25T12:00:00+08:00".into(),
        }
    }
    pub(crate) fn for_clip(
        shared: &Shared,
        p: &Value,
        session: &str,
        title: &str,
        start: f64,
        end: f64,
        job_id: &str,
        created_at: &str,
    ) -> Result<Self> {
        let source = p["source"].as_str().unwrap_or("manual");
        if !matches!(source, "manual" | "highlight" | "bookmark" | "full_session") {
            return Err("切片来源无效".into());
        }
        let bookmark_id = p
            .get("bookmark_id")
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_str()
                    .filter(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                    .map(str::to_owned)
                    .ok_or_else(|| "打点 ID 无效".to_owned())
            })
            .transpose()?;
        let anchor_id = match p.get("anchor_id").filter(|v| !v.is_null()) {
            Some(value) => value
                .as_str()
                .filter(|s| storage::id(s))
                .ok_or("主播 ID 无效")?
                .to_owned(),
            None => String::new(),
        };
        let state = shared.state.lock().map_err(|_| "主播状态不可用")?;
        let room = state
            .rooms
            .values()
            .map(|r| &r.value)
            .find(|r| r["session_id"] == session);
        let fallback_id = room.and_then(|r| r["anchor_id"].as_str()).unwrap_or("");
        let anchor_id = if anchor_id.is_empty() {
            fallback_id.into()
        } else {
            anchor_id
        };
        let fallback_name = room
            .and_then(|r| r["name"].as_str())
            .or_else(|| p["anchor_name"].as_str())
            .unwrap_or("主播")
            .to_owned();
        drop(state);
        let mut anchor_name = fallback_name;
        let mut session_date = p["session_date"]
            .as_str()
            .and_then(|raw| date(raw).ok())
            .unwrap_or(date(created_at)?);
        if !anchor_id.is_empty() {
            let selection = qingkan_core::archive::ArchiveDeleteFilter {
                anchor_ids: vec![anchor_id.clone()],
                month: None,
                session_ids: Some(vec![session.into()]),
            };
            if let Ok(history) = shared.archive.list_sessions(&selection, 1, 0) {
                if let Some(row) = history["sessions"].as_array().and_then(|rows| rows.first()) {
                    if let Some(name) = row["anchor_name"].as_str().filter(|s| !s.is_empty()) {
                        anchor_name = name.into();
                    }
                    if let Some(time) = row["started_at"]
                        .as_str()
                        .or_else(|| row["first_seen"].as_str())
                    {
                        if let Ok(value) = date(time) {
                            session_date = value;
                        }
                    }
                }
            }
        }
        Ok(Self {
            anchor_id,
            anchor_name,
            session_id: session.into(),
            session_date,
            title: title.into(),
            source: source.into(),
            bookmark_id,
            start,
            end,
            job_id: job_id.into(),
            created_at: created_at.into(),
        })
    }
    fn values(&self) -> Result<BTreeMap<&'static str, String>> {
        let day = chrono::NaiveDate::parse_from_str(&self.session_date, "%Y-%m-%d")
            .map_err(|_| "场次日期无效")?;
        let millis = |seconds: f64| format!("{:.0}", (seconds * 1000.).round());
        Ok(BTreeMap::from([
            ("anchor", self.anchor_name.clone()),
            ("anchor_id", self.anchor_id.clone()),
            ("session_id", self.session_id.clone()),
            ("title", self.title.clone()),
            ("date", self.session_date.clone()),
            ("year", format!("{:04}", day.year())),
            ("month", format!("{:02}", day.month())),
            ("day", format!("{:02}", day.day())),
            ("download_date", date(&self.created_at)?),
            ("start", clock(self.start)),
            ("end", clock(self.end)),
            ("start_ms", millis(self.start)),
            ("end_ms", millis(self.end)),
            (
                "duration",
                format!("{:.3}", self.end - self.start)
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .into(),
            ),
            ("job_id", self.job_id.clone()),
            ("job_id8", self.job_id.chars().take(8).collect()),
            (
                "source",
                match self.source.as_str() {
                    "full_session" => "整场",
                    "bookmark" => "打点",
                    "highlight" => "亮点",
                    _ => "手动",
                }
                .into(),
            ),
            (
                "bookmark_id",
                self.bookmark_id.clone().unwrap_or_else(|| "无打点".into()),
            ),
        ]))
    }
}
fn date(raw: &str) -> Result<String> {
    if let Ok(day) = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return Ok(day.format("%Y-%m-%d").to_string());
    }
    DateTime::parse_from_rfc3339(raw)
        .map(|time| {
            time.with_timezone(&qingkan_core::archive::shanghai())
                .format("%Y-%m-%d")
                .to_string()
        })
        .map_err(|_| "日期无效".into())
}
fn clock(seconds: f64) -> String {
    let n = seconds.floor().max(0.) as u64;
    format!("{:02}-{:02}-{:02}", n / 3600, n / 60 % 60, n % 60)
}
fn invalid(c: char) -> bool {
    c.is_control() || "<>:\"/\\|?*".contains(c)
}
fn sanitize(raw: &str) -> String {
    let mut output = String::new();
    let mut length = 0;
    for c in raw.chars() {
        if length + c.len_utf16() > 80 {
            break;
        }
        output.push(if invalid(c) { '_' } else { c });
        length += c.len_utf16();
    }
    let trimmed = output.trim().trim_end_matches(['.', ' ']);
    if trimmed.is_empty() || matches!(trimmed, "." | "..") {
        "未命名".into()
    } else {
        trimmed.into()
    }
}
fn reserved(raw: &str) -> bool {
    let stem = raw
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|n| {
                matches!(
                    n,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}
pub(crate) fn validate_explicit_filename(raw: &str) -> Result<()> {
    if raw.is_empty()
        || raw.chars().any(invalid)
        || reserved(raw)
        || raw.ends_with(['.', ' '])
        || raw.encode_utf16().count() > 220
    {
        return Err("切片文件名包含不支持的字符或保留名称".into());
    }
    Ok(())
}
fn component(template: &str, values: &BTreeMap<&str, String>) -> Result<String> {
    let mut output = String::new();
    let mut remaining = template;
    while let Some(start) = remaining.find('{') {
        let prefix = &remaining[..start];
        if prefix.contains('}') || prefix.chars().any(invalid) {
            return Err("命名规则包含无效字符".into());
        }
        output.push_str(prefix);
        let close = remaining[start + 1..]
            .find('}')
            .ok_or("命名变量缺少右花括号")?
            + start
            + 1;
        let key = &remaining[start + 1..close];
        output.push_str(&sanitize(
            values
                .get(key)
                .ok_or_else(|| format!("未知命名变量：{{{key}}}"))?,
        ));
        remaining = &remaining[close + 1..];
    }
    if remaining.contains('}') || remaining.chars().any(invalid) {
        return Err("命名规则包含无效字符".into());
    }
    output.push_str(remaining);
    let value = output.trim().trim_end_matches(['.', ' ']);
    if value.is_empty() || matches!(value, "." | "..") {
        return Err("目录或文件名不能为空或为 ..".into());
    }
    let value = if reserved(value) {
        format!("_{value}")
    } else {
        value.into()
    };
    if value.encode_utf16().count() > 200 {
        return Err("目录或文件名过长，请缩短命名规则".into());
    }
    Ok(value)
}
fn render(settings: &NamingSettings, context: &NamingContext) -> Result<PathBuf> {
    if settings.directory_template.len() > 1024 || settings.filename_template.len() > 512 {
        return Err("命名规则过长".into());
    }
    let directory = settings.directory_template.replace('\\', "/");
    if directory.starts_with('/') || directory.ends_with('/') || directory.contains("//") {
        return Err("目录规则须为相对目录，使用 / 分隔".into());
    }
    let values = context.values()?;
    let mut relative = PathBuf::new();
    if !directory.is_empty() {
        if directory.split('/').count() > 8 {
            return Err("最多支持 8 层子目录".into());
        }
        for part in directory.split('/') {
            relative.push(component(part, &values)?);
        }
    }
    let mut filename = component(&settings.filename_template, &values)?;
    if !filename.to_ascii_lowercase().ends_with(".mp4") {
        filename.push_str(".mp4");
    }
    relative.push(filename);
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("命名规则必须位于下载目录内".into());
    }
    Ok(relative)
}
fn root(raw: &str, directory: &Path, create_default: bool) -> Result<PathBuf> {
    let path = Path::new(raw);
    if !path.is_absolute() || raw.chars().any(char::is_control) || raw.len() > 2048 {
        return Err("请选择完整的下载目录".into());
    }
    if !path.exists() && path == directory.join("clips") {
        if !create_default {
            return Ok(path.into());
        }
        std::fs::create_dir_all(path).map_err(|_| "无法创建默认下载目录")?;
    }
    let root = path
        .canonicalize()
        .map_err(|_| "下载目录不存在，请重新选择")?;
    if !root.is_dir() {
        return Err("下载位置必须是文件夹".into());
    }
    Ok(root)
}
/// Creates each template directory inside the canonical root, refusing symlink escapes.
pub(crate) fn target(
    settings: &NamingSettings,
    context: &NamingContext,
    directory: &Path,
    occupied: impl Fn(&Path) -> Result<bool>,
) -> Result<PathBuf> {
    let relative = render(settings, context)?;
    let base = root(&settings.root_directory, directory, true)?;
    let mut parent = base.clone();
    for part in relative.parent().into_iter().flat_map(Path::components) {
        let Component::Normal(name) = part else {
            return Err("子目录规则无效".into());
        };
        parent.push(name);
        match std::fs::symlink_metadata(&parent) {
            Ok(meta) if linked_directory(&meta) || !meta.is_dir() => {
                return Err("下载子目录不能是链接或文件".into())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&parent).map_err(|_| "无法创建下载子目录")?;
            }
            Err(_) => return Err("无法访问下载子目录".into()),
        }
        parent = parent.canonicalize().map_err(|_| "下载子目录不可用")?;
        if !parent.starts_with(&base) {
            return Err("下载子目录超出选定目录".into());
        }
    }
    let filename = relative
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("下载文件名无效")?;
    let stem = filename.get(..filename.len() - 4).ok_or("下载文件名无效")?;
    for suffix in 1..=10_000 {
        let path = parent.join(if suffix == 1 {
            filename.into()
        } else {
            format!("{stem} ({suffix}).mp4")
        });
        if std::fs::symlink_metadata(&path).is_err() && !occupied(&path)? {
            return Ok(path);
        }
    }
    Err("同名下载过多，请调整命名规则".into())
}
fn linked_directory(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
pub(crate) fn same_path(a: &str, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.eq_ignore_ascii_case(&b.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        Path::new(a) == b
    }
}
impl Runtime {
    pub(crate) fn download_naming_request(&self, method: &str, p: Value) -> Result<Value> {
        let mut settings = NamingSettings::load(&self.shared)?;
        if method != "download_naming_settings" {
            for (key, value) in [
                ("root_directory", &mut settings.root_directory),
                ("directory_template", &mut settings.directory_template),
                ("filename_template", &mut settings.filename_template),
            ] {
                if let Some(raw) = p.get(key) {
                    *value = raw.as_str().ok_or("命名设置必须为文本")?.trim().into();
                }
            }
        }
        if method != "download_naming_settings" {
            settings.validate(&self.shared.directory)?;
        }
        let mut context = NamingContext::example();
        if let Some(c) = p.get("context") {
            if !c.is_object() {
                return Err("预览内容无效".into());
            }
            for (key, value) in [
                ("anchor", &mut context.anchor_name),
                ("anchor_id", &mut context.anchor_id),
                ("session_id", &mut context.session_id),
                ("title", &mut context.title),
                ("source", &mut context.source),
            ] {
                if let Some(raw) = c.get(key) {
                    *value = raw
                        .as_str()
                        .filter(|s| s.len() <= 500)
                        .ok_or("预览内容过长或格式无效")?
                        .into();
                }
            }
            if let Some(raw) = c.get("session_date") {
                context.session_date = date(raw.as_str().ok_or("场次日期无效")?)?;
            }
            if let Some(raw) = c.get("start_seconds") {
                context.start = raw
                    .as_f64()
                    .filter(|n| n.is_finite() && *n >= 0.)
                    .ok_or("预览起点无效")?;
            }
            if let Some(raw) = c.get("end_seconds") {
                context.end = raw
                    .as_f64()
                    .filter(|n| n.is_finite() && *n > context.start)
                    .ok_or("预览终点无效")?;
            }
            if let Some(raw) = c.get("bookmark_id") {
                context.bookmark_id = Some(
                    raw.as_str()
                        .filter(|s| s.len() <= 100)
                        .ok_or("预览打点 ID 无效")?
                        .into(),
                );
            }
        }
        if context.end <= context.start {
            return Err("预览终点须晚于起点".into());
        }
        let relative = render(&settings, &context)?;
        let preview = json!({"relative_path":relative.to_string_lossy(), "output_path":Path::new(&settings.root_directory).join(&relative).to_string_lossy()});
        if method == "download_naming_preview" {
            return Ok(preview);
        }
        if method == "download_naming_save" {
            let mut state = self.shared.state.lock().map_err(|_| "下载设置不可用")?;
            let mut config = state.config.value.clone();
            config["download_naming"] =
                serde_json::to_value(&settings).map_err(|_| "下载设置格式无效")?;
            state.config.replace(config)?;
        }
        Ok(
            json!({"settings":settings,"defaults":NamingSettings::defaults(&self.shared.directory),
            "root_available":root(&settings.root_directory,&self.shared.directory,false).is_ok(),
            "variables":VARIABLES.iter().map(|(key,label,example)|json!({"key":key,"label":label,"example":example})).collect::<Vec<_>>(),"preview":preview}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn variable_values_cannot_create_directories_and_windows_devices_are_safe() {
        let mut settings = NamingSettings::defaults(Path::new("unused"));
        settings.directory_template = "{date}/{anchor}".into();
        settings.filename_template = "{title}_{start}-{end}".into();
        let mut context = NamingContext::example();
        context.anchor_name = "../../主播\\外部".into();
        context.title = "CON".into();
        let output = render(&settings, &context).unwrap();
        assert_eq!(output.components().count(), 3);
        assert!(output.to_string_lossy().contains(".._.._主播_外部"));
        settings.filename_template = "{title}.mp4".into();
        assert_eq!(
            render(&settings, &context).unwrap().file_name().unwrap(),
            "_CON.mp4"
        );
        for template in [
            "../{date}",
            "/absolute",
            "C:/drive",
            "{unknown}",
            "{date",
            "folder//nested",
        ] {
            settings.directory_template = template.into();
            assert!(render(&settings, &context).is_err(), "{template}");
        }
    }
    #[test]
    fn default_names_are_compatible_and_nested_outputs_never_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = NamingSettings::defaults(dir.path());
        let context = NamingContext::example();
        assert_eq!(
            render(&settings, &context).unwrap().to_str().unwrap(),
            "7698991795467832114-120000-01234567.mp4"
        );
        settings.directory_template = "{date}/{anchor}".into();
        settings.filename_template = "{title}.mp4".into();
        let first = target(&settings, &context, dir.path(), |_| Ok(false)).unwrap();
        std::fs::write(&first, "keep").unwrap();
        let next = target(&settings, &context, dir.path(), |p| {
            Ok(p.file_name().unwrap() == "精彩互动 (2).mp4")
        })
        .unwrap();
        assert_eq!(next.file_name().unwrap(), "精彩互动 (3).mp4");
        assert_eq!(std::fs::read_to_string(first).unwrap(), "keep");
    }
    #[cfg(unix)]
    #[test]
    fn refuses_a_template_directory_link_outside_root() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let mut settings = NamingSettings::defaults(dir.path());
        std::fs::create_dir(dir.path().join("clips")).unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("clips/link")).unwrap();
        settings.directory_template = "link".into();
        assert!(
            target(&settings, &NamingContext::example(), dir.path(), |_| Ok(
                false
            ))
            .is_err()
        );
    }
}
