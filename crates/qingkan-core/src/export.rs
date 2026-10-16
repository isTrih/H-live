//! One read snapshot, bounded-memory writers, atomic publication of complete exports.
use crate::archive::{
    self, ArchiveError, ArchiveOptions, ArchiveSnapshot, ArchiveStore, StoredMessage,
};
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Timelike};
use rusqlite::params_from_iter;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use unicode_casefold::UnicodeCaseFold;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

pub const HEADERS: [&str; 7] = [
    "主播",
    "直播间场次",
    "发消息人ID",
    "发消息人名字",
    "消息类型",
    "消息内容",
    "发消息时间",
];
pub const EXCEL_MAX_ROWS: u32 = 1_048_576;
const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("导出已取消，原目标文件未改变。")]
    Cancelled,
    #[error("{0}")]
    Data(String),
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
}
impl From<rusqlite::Error> for ExportError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Archive(value.into())
    }
}
pub type Result<T> = std::result::Result<T, ExportError>;
#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);
impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(ExportError::Cancelled)
        } else {
            Ok(())
        }
    }
}
#[derive(Clone)]
pub struct ExportOptions {
    pub anchor_ids: Vec<String>,
    pub month: String,
    pub output_path: PathBuf,
    pub format: String,
    pub types: Vec<String>,
}
#[derive(Debug, Serialize, Default)]
pub struct ExportReport {
    pub row_count: u64,
    pub path: String,
    pub format: String,
    pub month: String,
    pub missing_sender_count: u64,
    pub missing_time_count: u64,
    pub sheet_count: u64,
    pub formula_protected_count: u64,
    pub skipped_missing_session_count: u64,
    pub skipped_invalid_time_count: u64,
}
pub fn export_month(
    data_dir: &Path,
    options: ExportOptions,
    cancel: &CancelToken,
) -> Result<ExportReport> {
    // Opening ensures a new empty archive can still export a header-only file.
    validate(&options)?;
    cancel.check()?;
    let stop = cancel.clone();
    let store = ArchiveStore::open_cancellable(data_dir, ArchiveOptions::default(), move || {
        stop.is_cancelled()
    })
    .map_err(|error| match error {
        ArchiveError::MigrationCancelled => ExportError::Cancelled,
        other => ExportError::Archive(other),
    })?;
    let snapshot = store.snapshot()?;
    export_snapshot(&snapshot, options, cancel, EXCEL_MAX_ROWS)
}
fn validate(options: &ExportOptions) -> Result<()> {
    archive::month_bounds(&options.month)?;
    if options.anchor_ids.is_empty() {
        return Err(ArchiveError::Invalid("请至少选择一个主播。".into()).into());
    }
    for id in &options.anchor_ids {
        archive::identity(id)?;
    }
    if options
        .types
        .iter()
        .any(|t| !archive::CONTENT_TYPES.contains(&t.as_str()))
    {
        return Err(ArchiveError::Invalid("只能导出聊天与互动消息类型".into()).into());
    }
    if options.format != "csv" && options.format != "xlsx" {
        return Err(ArchiveError::Invalid("导出格式须为 xlsx 或 csv。".into()).into());
    }
    if let Some(extension) = options.output_path.extension() {
        if extension.to_string_lossy().to_lowercase() != options.format {
            return Err(ArchiveError::Invalid("文件扩展名须与导出格式一致。".into()).into());
        }
    }
    if !options
        .output_path
        .parent()
        .unwrap_or(Path::new("."))
        .is_dir()
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "导出目录不存在，请选择一个已有文件夹。",
        )
        .into());
    }
    Ok(())
}
fn export_snapshot(
    snapshot: &ArchiveSnapshot,
    mut options: ExportOptions,
    cancel: &CancelToken,
    max_rows: u32,
) -> Result<ExportReport> {
    validate(&options)?;
    cancel.check()?;
    let mut seen = HashSet::new();
    options.anchor_ids.retain(|id| seen.insert(id.clone()));
    let mut report = ExportReport {
        path: options.output_path.to_string_lossy().into_owned(),
        format: options.format.clone(),
        month: options.month.clone(),
        ..Default::default()
    };
    let parent = options.output_path.parent().unwrap_or(Path::new("."));
    let prefix = format!(
        ".{}.",
        options
            .output_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    );
    let mut temporary = tempfile::Builder::new()
        .prefix(&prefix)
        .suffix(".tmp")
        .tempfile_in(parent)?;
    if options.format == "csv" {
        write_csv(
            temporary.as_file_mut(),
            snapshot,
            &options,
            &mut report,
            cancel,
        )?;
    } else {
        write_xlsx(
            temporary.as_file_mut(),
            snapshot,
            &options,
            &mut report,
            cancel,
            max_rows,
        )?;
    }
    temporary.as_file_mut().flush()?;
    temporary.as_file().sync_all()?;
    cancel.check()?;
    // persist uses MoveFileExW(REPLACE_EXISTING) on Windows; errors retain the old destination.
    temporary
        .persist(&options.output_path)
        .map_err(|error| error.error)?;
    Ok(report)
}
fn timestamp(value: &str) -> Option<DateTime<FixedOffset>> {
    if value.is_empty() {
        return None;
    }
    let value = value.replacen(' ', "T", 1);
    DateTime::parse_from_rfc3339(&value)
        .ok()
        .map(|v| v.with_timezone(&archive::shanghai()))
        .or_else(|| {
            NaiveDateTime::parse_from_str(&value, "%Y-%m-%dT%H:%M:%S%.f")
                .ok()
                .and_then(|v| archive::shanghai().from_local_datetime(&v).single())
        })
}
fn display_time(value: DateTime<FixedOffset>) -> String {
    let base = value.format("%Y-%m-%d %H:%M:%S").to_string();
    let micro = value.nanosecond() / 1000;
    format!(
        "{base}{}+08:00",
        if micro == 0 {
            String::new()
        } else {
            format!(".{micro:06}")
        }
    )
}
fn label(kind: &str) -> Option<&'static str> {
    match kind {
        "chat" => Some("弹幕"),
        "gift" => Some("礼物"),
        "like" => Some("点赞"),
        "member" => Some("进场"),
        "social" => Some("关注/分享"),
        "fansclub" => Some("粉丝团"),
        "emoji" => Some("表情弹幕"),
        "activity" => Some("红包/福袋"),
        _ => None,
    }
}
fn record_row(
    record: StoredMessage,
    month: &str,
    types: &[String],
    report: &mut ExportReport,
) -> Option<[String; 7]> {
    let r = record.record;
    if !types.contains(&r.message_type) {
        return None;
    }
    let kind = label(&r.message_type)?;
    if r.session_id.is_empty() {
        report.skipped_missing_session_count += 1;
        return None;
    }
    let sent = timestamp(&r.message_time);
    let effective = sent.or_else(|| timestamp(&r.received_at));
    let effective = match effective {
        Some(value) => value,
        None => {
            report.skipped_invalid_time_count += 1;
            return None;
        }
    };
    if effective.format("%Y-%m").to_string() != month {
        return None;
    }
    report.row_count += 1;
    report.missing_sender_count += u64::from(r.sender_id.is_empty());
    report.missing_time_count += u64::from(sent.is_none());
    Some([
        r.anchor_name,
        r.session_id,
        r.sender_id,
        r.sender_name,
        kind.into(),
        r.content,
        sent.map(display_time).unwrap_or_default(),
    ])
}
fn each_row(
    snapshot: &ArchiveSnapshot,
    ids: &[String],
    options: &ExportOptions,
    report: &mut ExportReport,
    cancel: &CancelToken,
    mut write: impl FnMut([String; 7], &mut ExportReport) -> Result<()>,
) -> Result<()> {
    let (query, parameters) =
        archive::message_query(Some(ids), &options.month, Some(&options.types))?;
    let mut statement = snapshot.connection.prepare(&query)?;
    let mut rows = statement.query(params_from_iter(parameters))?;
    while let Some(row) = rows.next()? {
        cancel.check()?;
        if let Some(values) = record_row(
            archive::stored_row(row)?,
            &options.month,
            &options.types,
            report,
        ) {
            write(values, report)?;
        }
    }
    Ok(())
}
fn csv_value(value: &str, report: &mut ExportReport) -> String {
    let stripped =
        value.trim_start_matches(|c: char| c.is_whitespace() || c <= '\u{1f}' || c == '\u{feff}');
    if value.starts_with(['\t', '\r', '\n']) || stripped.starts_with(['=', '+', '-', '@']) {
        report.formula_protected_count += 1;
        format!("'{value}")
    } else {
        value.into()
    }
}
fn csv_line(
    writer: &mut impl Write,
    values: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<()> {
    for (index, value) in values.into_iter().enumerate() {
        if index != 0 {
            writer.write_all(b",")?;
        }
        let value = value.as_ref();
        if value.contains([',', '"', '\r', '\n']) {
            writer.write_all(b"\"")?;
            writer.write_all(value.replace('"', "\"\"").as_bytes())?;
            writer.write_all(b"\"")?;
        } else {
            writer.write_all(value.as_bytes())?;
        }
    }
    writer.write_all(b"\r\n")?;
    Ok(())
}
fn write_csv(
    file: &mut File,
    snapshot: &ArchiveSnapshot,
    options: &ExportOptions,
    report: &mut ExportReport,
    cancel: &CancelToken,
) -> Result<()> {
    let mut writer = BufWriter::with_capacity(64 * 1024, file);
    writer.write_all(&[0xef, 0xbb, 0xbf])?;
    csv_line(&mut writer, HEADERS)?;
    each_row(
        snapshot,
        &options.anchor_ids,
        options,
        report,
        cancel,
        |row, report| {
            csv_line(
                &mut writer,
                row.iter()
                    .map(|value| csv_value(value, report))
                    .collect::<Vec<_>>(),
            )
        },
    )?;
    writer.flush()?;
    Ok(())
}
fn escape_literal(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut result = String::with_capacity(value.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'_'
            && index + 7 <= bytes.len()
            && bytes[index + 1] == b'x'
            && bytes[index + 2..index + 6]
                .iter()
                .all(u8::is_ascii_hexdigit)
            && bytes[index + 6] == b'_'
        {
            result.push_str("_x005F_");
            index += 1;
        } else {
            let ch = value[index..].chars().next().unwrap();
            result.push(ch);
            index += ch.len_utf8();
        }
    }
    result
}
fn xml_escape(value: &str, attribute: bool) -> String {
    let mut text = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => text.push_str("&amp;"),
            '<' => text.push_str("&lt;"),
            '>' => text.push_str("&gt;"),
            '"' if attribute => text.push_str("&quot;"),
            '\r' => text.push_str("&#13;"),
            '\n' if attribute => text.push_str("&#10;"),
            '\t' if attribute => text.push_str("&#9;"),
            _ => text.push(c),
        }
    }
    text
}
fn xml_value(value: &str, source_row: u64, column: usize) -> Result<String> {
    if value.chars().any(
        |c| matches!(c,'\u{0}'..='\u{8}'|'\u{b}'|'\u{c}'|'\u{e}'..='\u{1f}'|'\u{fffe}'|'\u{ffff}'),
    ) {
        return Err(ExportError::Data(format!(
            "第 {source_row} 条消息的“{}”含 XLSX 无法原样保存的控制字符；请改用 CSV。",
            HEADERS.get(column).unwrap_or(&"分析字段")
        )));
    }
    if value.encode_utf16().count() > 32_767 {
        return Err(ExportError::Data(format!(
            "第 {source_row} 条消息的“{}”超过 Excel 单元格 32767 字符限制；请改用 CSV。",
            HEADERS.get(column).unwrap_or(&"分析字段")
        )));
    }
    Ok(xml_escape(&escape_literal(value), false))
}
fn xml_row(
    writer: &mut impl Write,
    values: impl IntoIterator<Item = impl AsRef<str>>,
    row: u32,
    source: u64,
    header: bool,
) -> Result<()> {
    // One bounded row buffer avoids feeding tiny fragments through deflate.
    let mut encoded = Vec::with_capacity(1024);
    write!(encoded, "<row r=\"{row}\">")?;
    for (column, value) in values.into_iter().enumerate() {
        let text = xml_value(value.as_ref(), source, column)?;
        write!(encoded,"<c r=\"{}{row}\" s=\"{}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{text}</t></is></c>",(b'A'+column as u8)as char,if header{1}else{2})?;
    }
    encoded.write_all(b"</row>")?;
    writer.write_all(&encoded)?;
    Ok(())
}
fn trim_name(value: &str, max: usize) -> String {
    let mut used = 0;
    value
        .chars()
        .take_while(|c| {
            used += c.len_utf16();
            used <= max
        })
        .collect::<String>()
        .trim_matches(|c: char| c.is_whitespace() || c == '\'')
        .to_owned()
}
#[derive(Default)]
struct SheetNames {
    used: HashSet<String>,
}
impl SheetNames {
    fn claim(&mut self, name: &str, id: &str, part: usize) -> String {
        let sanitized:String=name.chars().map(|c|if matches!(c,'\\'|'/'|'?'|'*'|':'|'['|']'|'\u{0}'..='\u{1f}'|'\u{7f}'..='\u{9f}'|'\u{fffe}'|'\u{ffff}'){'_'}else{c}).collect();
        let mut base = trim_name(&sanitized, 31);
        if base.is_empty() {
            base = trim_name(&format!("主播 {id}"), 31);
        }
        let mut number = part;
        loop {
            let suffix = if number == 1 {
                String::new()
            } else {
                format!(" ({number})")
            };
            let candidate = format!("{}{}", trim_name(&base, 31 - suffix.len()), suffix);
            let key = candidate
                .chars()
                .flat_map(char::to_lowercase)
                .case_fold()
                .collect::<String>();
            if key != "history" && self.used.insert(key) {
                return candidate;
            }
            number = std::cmp::max(2, number + 1);
        }
    }
}
fn zip_options() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(6))
        .large_file(true)
}
fn start_sheet(zip: &mut ZipWriter<&mut File>, number: usize) -> Result<()> {
    zip.start_file(format!("xl/worksheets/sheet{number}.xml"), zip_options())?;
    write!(zip,"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><worksheet xmlns=\"{NS}\"><sheetViews><sheetView workbookViewId=\"0\"><pane ySplit=\"1\" topLeftCell=\"A2\" activePane=\"bottomLeft\" state=\"frozen\"/><selection pane=\"bottomLeft\" activeCell=\"A2\" sqref=\"A2\"/></sheetView></sheetViews><sheetFormatPr defaultRowHeight=\"18\"/><cols><col min=\"1\" max=\"1\" width=\"24\" customWidth=\"1\"/><col min=\"2\" max=\"3\" width=\"25\" customWidth=\"1\"/><col min=\"4\" max=\"4\" width=\"24\" customWidth=\"1\"/><col min=\"5\" max=\"5\" width=\"15\" customWidth=\"1\"/><col min=\"6\" max=\"6\" width=\"72\" customWidth=\"1\"/><col min=\"7\" max=\"7\" width=\"30\" customWidth=\"1\"/></cols><sheetData>")?;
    xml_row(zip, HEADERS, 1, 0, true)
}
fn end_sheet(zip: &mut ZipWriter<&mut File>, rows: u32) -> Result<()> {
    write!(
        zip,
        "</sheetData><autoFilter ref=\"A1:G{rows}\"/></worksheet>"
    )?;
    Ok(())
}
fn write_xlsx(
    file: &mut File,
    snapshot: &ArchiveSnapshot,
    options: &ExportOptions,
    report: &mut ExportReport,
    cancel: &CancelToken,
    max_rows: u32,
) -> Result<()> {
    let names: HashMap<_, _> = snapshot
        .list_anchors(Some(&options.anchor_ids))?
        .into_iter()
        .map(|a| (a.anchor_id, a.anchor_name))
        .collect();
    let mut allocator = SheetNames::default();
    let first_names: HashMap<_, _> = options
        .anchor_ids
        .iter()
        .map(|id| {
            (
                id.clone(),
                allocator.claim(names.get(id).map(String::as_str).unwrap_or(""), id, 1),
            )
        })
        .collect();
    let mut sheet_names = Vec::new();
    let mut zip = ZipWriter::new(file);
    let mut source = 0;
    for id in &options.anchor_ids {
        cancel.check()?;
        let mut part = 1;
        let mut sheet_row = 1;
        sheet_names.push(first_names[id].clone());
        start_sheet(&mut zip, sheet_names.len())?;
        each_row(
            snapshot,
            std::slice::from_ref(id),
            options,
            report,
            cancel,
            |row, _| {
                if sheet_row >= max_rows {
                    end_sheet(&mut zip, sheet_row)?;
                    part += 1;
                    sheet_names.push(allocator.claim(&first_names[id], id, part));
                    start_sheet(&mut zip, sheet_names.len())?;
                    sheet_row = 1;
                }
                source += 1;
                sheet_row += 1;
                xml_row(&mut zip, row, sheet_row, source, false)
            },
        )?;
        end_sheet(&mut zip, sheet_row)?;
    }
    write_parts(&mut zip, &sheet_names)?;
    zip.finish()?;
    report.sheet_count = sheet_names.len() as u64;
    Ok(())
}
fn zip_text(zip: &mut ZipWriter<&mut File>, name: &str, text: &str) -> Result<()> {
    zip.start_file(name, zip_options())?;
    zip.write_all(text.as_bytes())?;
    Ok(())
}
fn write_parts(zip: &mut ZipWriter<&mut File>, names: &[String]) -> Result<()> {
    let sheets = names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            format!(
                "<sheet name=\"{}\" sheetId=\"{}\" r:id=\"rId{}\"/>",
                xml_escape(&escape_literal(n), true),
                i + 1,
                i + 1
            )
        })
        .collect::<String>();
    zip_text(zip,"xl/workbook.xml",&format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><workbook xmlns=\"{NS}\" xmlns:r=\"{REL}\"><sheets>{sheets}</sheets></workbook>"))?;
    let relations=(1..=names.len()).map(|i|format!("<Relationship Id=\"rId{i}\" Type=\"{REL}/worksheet\" Target=\"worksheets/sheet{i}.xml\"/>")).collect::<String>();
    zip_text(zip,"xl/_rels/workbook.xml.rels",&format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"{PKG}\">{relations}<Relationship Id=\"rIdStyles\" Type=\"{REL}/styles\" Target=\"styles.xml\"/></Relationships>"))?;
    zip_text(zip,"_rels/.rels",&format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"{PKG}\"><Relationship Id=\"rId1\" Type=\"{REL}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>"))?;
    let overrides=(1..=names.len()).map(|i|format!("<Override PartName=\"/xl/worksheets/sheet{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>")).collect::<String>();
    zip_text(zip,"[Content_Types].xml",&format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>{overrides}</Types>"))?;
    zip_text(
        zip,
        "xl/styles.xml",
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="{NS}"><fonts count="2"><font><sz val="11"/><name val="Microsoft YaHei"/></font><font><b/><sz val="11"/><name val="Microsoft YaHei"/></font></fonts><fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFDCE6F1"/><bgColor indexed="64"/></patternFill></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="3"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="49" fontId="1" fillId="2" borderId="0" xfId="0" applyAlignment="1"><alignment vertical="top" wrapText="1"/></xf><xf numFmtId="49" fontId="0" fillId="0" borderId="0" xfId="0" applyAlignment="1"><alignment vertical="top" wrapText="1"/></xf></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#
        ),
    )
}

/// Separate session-analysis export; never changes the seven raw-message columns.
pub fn export_quality(
    data_dir: &Path,
    anchor_id: &str,
    month: &str,
    output: &Path,
    format: &str,
    cancel: &CancelToken,
) -> Result<serde_json::Value> {
    archive::identity(anchor_id)?;
    if !month.is_empty() {
        archive::month_bounds(month)?;
    }
    validate(&ExportOptions {
        anchor_ids: vec![anchor_id.into()],
        month: if month.is_empty() {
            "2000-01".into()
        } else {
            month.into()
        },
        output_path: output.into(),
        format: format.into(),
        types: vec![],
    })?;
    cancel.check()?;
    let stop = cancel.clone();
    let store = ArchiveStore::open_cancellable(data_dir, ArchiveOptions::default(), move || {
        stop.is_cancelled()
    })?;
    let snapshot = store.snapshot()?;
    store.close(std::time::Duration::from_secs(1));
    let mut tmp = tempfile::Builder::new()
        .prefix(".qingkan-analysis-")
        .suffix(".tmp")
        .tempfile_in(output.parent().unwrap_or(Path::new(".")))?;
    let headers = [
        "主播",
        "直播间场次",
        "直播日期",
        "累计监控秒数",
        "监控覆盖",
        "聊天消息数",
        "发言人数",
        "有效发言人数",
        "有效发言占比",
        "有效消息数",
        "待判定消息数",
        "记录已清理",
        "规则版本",
        "分析状态",
        "最高在线人数",
        "分钟平均在线人数",
        "有效发言/平均在线",
        "在线采样分钟数",
    ];
    let mut count = 0u64;
    if format == "csv" {
        let mut writer = BufWriter::new(tmp.as_file_mut());
        writer.write_all(&[0xef, 0xbb, 0xbf])?;
        csv_line(&mut writer, headers)?;
        let mut report = ExportReport::default();
        quality_rows(&snapshot, anchor_id, month, cancel, |values| {
            csv_line(
                &mut writer,
                values
                    .iter()
                    .map(|s| csv_value(s, &mut report))
                    .collect::<Vec<_>>(),
            )?;
            count += 1;
            Ok(())
        })?;
        writer.flush()?;
    } else {
        let mut zip = ZipWriter::new(tmp.as_file_mut());
        zip.start_file("xl/worksheets/sheet1.xml", zip_options())?;
        write!(zip,"<?xml version=\"1.0\" encoding=\"UTF-8\"?><worksheet xmlns=\"{NS}\"><sheetViews><sheetView workbookViewId=\"0\"><pane ySplit=\"1\" topLeftCell=\"A2\" state=\"frozen\"/></sheetView></sheetViews><cols><col min=\"1\" max=\"18\" width=\"22\" customWidth=\"1\"/></cols><sheetData>")?;
        xml_row(&mut zip, headers, 1, 0, true)?;
        quality_rows(&snapshot, anchor_id, month, cancel, |values| {
            if count + 1 >= EXCEL_MAX_ROWS as u64 {
                return Err(ExportError::Data(
                    "场次数超过单工作表上限，请分月份或改用CSV".into(),
                ));
            }
            xml_row(&mut zip, values, count as u32 + 2, count + 1, false)?;
            count += 1;
            Ok(())
        })?;
        write!(
            zip,
            "</sheetData><autoFilter ref=\"A1:R{}\"/></worksheet>",
            count + 1
        )?;
        write_parts(&mut zip, &["场次分析".into()])?;
        zip.finish()?;
    }
    tmp.as_file_mut().flush()?;
    tmp.as_file().sync_all()?;
    cancel.check()?;
    tmp.persist(output).map_err(|v| v.error)?;
    Ok(
        serde_json::json!({"row_count":count,"path":output.to_string_lossy(),"format":format,"month":month,"kind":"session_analysis","rule_version":archive::quality::RULE_VERSION,"sheet_count":if format=="xlsx"{1}else{0}}),
    )
}
fn quality_rows(
    snapshot: &ArchiveSnapshot,
    a: &str,
    month: &str,
    cancel: &CancelToken,
    mut write: impl FnMut(Vec<String>) -> Result<()>,
) -> Result<()> {
    let mut query=snapshot.connection.prepare("SELECT session_id FROM quality_registry WHERE anchor_id=? AND (?='' OR substr(COALESCE(NULLIF(started_at,''),first_seen),1,7)=?) ORDER BY first_seen DESC,session_id DESC")?;
    let mut rows = query.query(rusqlite::params![a, month, month])?;
    while let Some(row) = rows.next()? {
        cancel.check()?;
        let s: String = row.get(0)?;
        let v = archive::quality::summary(&snapshot.connection, a, &s)?;
        let str_value = |key: &str| v[key].as_str().unwrap_or("").to_owned();
        let n = |key: &str| v[key].as_i64().unwrap_or(0).to_string();
        let date = v["started_at"]
            .as_str()
            .or(v["first_seen"].as_str())
            .unwrap_or("")
            .chars()
            .take(10)
            .collect();
        write(vec![
            str_value("anchor_name"),
            s,
            date,
            format!("{:.3}", v["monitor_seconds"].as_f64().unwrap_or(0.0)),
            if v["coverage"] == "partial" {
                "部分监控".into()
            } else {
                "覆盖未知".into()
            },
            n("chat_messages"),
            n("speakers"),
            n("valid_speakers"),
            v["valid_speaker_rate"]
                .as_f64()
                .map(|n| format!("{:.2}%", n * 100.0))
                .unwrap_or_default(),
            n("valid_messages"),
            n("pending"),
            if v["records_cleared"] == true {
                "是".into()
            } else {
                "否".into()
            },
            str_value("rule_version"),
            match v["analysis_state"].as_str() {
                Some("ready") => "已完成",
                Some("paused") => "已暂停",
                Some("needs_review") => "需确认",
                _ => "统计中",
            }
            .into(),
            v["peak_viewers"]
                .as_i64()
                .map(|n| n.to_string())
                .unwrap_or_default(),
            v["minute_avg_viewers"]
                .as_f64()
                .map(|n| format!("{n:.2}"))
                .unwrap_or_default(),
            v["valid_speakers_to_online_rate"]
                .as_f64()
                .map(|n| format!("{:.2}%", n * 100.))
                .unwrap_or_default(),
            n("audience_sample_minutes"),
        ])?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
