"""Stream a consistent month of structured message history to CSV or XLSX.

Only the normalized SQLite archive is read. Legacy JSONL files do not contain
enough provenance to reconstruct a reliable session or sender and are untouched.
"""
from __future__ import annotations

import csv
from datetime import datetime, timedelta, timezone
import os
from pathlib import Path
import re
import tempfile
from xml.sax.saxutils import escape, quoteattr
from zipfile import ZIP_DEFLATED, ZipFile


HEADERS = ("主播", "直播间场次", "发消息人ID", "发消息人名字", "消息类型", "消息内容", "发消息时间")
MESSAGE_TYPE_LABELS = {
    "chat": "弹幕", "gift": "礼物", "like": "点赞", "member": "进场",
    "social": "关注/分享", "fansclub": "粉丝团", "emoji": "表情弹幕",
}
SHANGHAI = timezone(timedelta(hours=8), name="Asia/Shanghai")
EXCEL_MAX_ROWS = 1_048_576  # includes the header on every worksheet
EXCEL_MAX_CELL_UNITS = 32_767
_LITERAL_OOXML_ESCAPE = re.compile(r"_x[0-9A-Fa-f]{4}_")
_ILLEGAL_XML = re.compile("[\\x00-\\x08\\x0b\\x0c\\x0e-\\x1f\\ud800-\\udfff\\ufffe\\uffff]")
_ILLEGAL_SHEET_NAME = re.compile(r"[\\/?*:\[\]\x00-\x1f\x7f-\x9f\ud800-\udfff\ufffe\uffff]")
_SHEET_NS = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
_REL_NS = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
_PACKAGE_REL_NS = "http://schemas.openxmlformats.org/package/2006/relationships"


class ExportCancelled(Exception):
    """The user cancelled; any pre-existing output file remains unchanged."""


class ExportDataError(ValueError):
    """A source value cannot be represented faithfully in the chosen format."""


def normalize_month(month: str) -> str:
    if not isinstance(month, str) or not re.fullmatch(r"[0-9]{4}-[0-9]{2}", month.strip()):
        raise ValueError("月份须为 YYYY-MM，例如 2026-10。")
    month = month.strip()
    try:
        datetime(int(month[:4]), int(month[5:]), 1)
    except ValueError as exc:
        raise ValueError("月份无效，请输入有效的 YYYY-MM。") from exc
    return month


def _selected_values(values, label):
    if values is None or isinstance(values, (str, bytes)):
        raise ValueError(f"{label}须为列表。")
    result = tuple(dict.fromkeys(str(value).strip() for value in values))
    if any(not value for value in result):
        raise ValueError(f"{label}不能包含空值。")
    return result


def _check_cancel(cancel_event):
    if cancel_event is not None and cancel_event.is_set():
        raise ExportCancelled("导出已取消，原目标文件未改变。")


def _timestamp(value):
    if not value:
        return None
    try:
        parsed = datetime.fromisoformat(str(value).replace("Z", "+00:00"))
        if parsed.tzinfo is None:
            parsed = parsed.replace(tzinfo=SHANGHAI)
        return parsed.astimezone(SHANGHAI)
    except (TypeError, ValueError, OverflowError):
        return None


def _month_of(timestamp):
    return f"{timestamp.year:04d}-{timestamp.month:02d}"


def _records(reader, anchor_ids, month, types, report, cancel_event):
    selected = set(anchor_ids)
    for record in reader.iter_messages(anchor_ids, month, types=types):
        _check_cancel(cancel_event)
        # Keep these checks here as well as in the indexed archive query. They
        # prevent unrelated record kinds or legacy entries entering the table.
        if str(record.get("anchor_id", "")) not in selected:
            continue
        kind = record.get("message_type", "")
        if kind not in types:
            continue
        session_id = str(record.get("session_id") or "")
        if not session_id:
            report["skipped_missing_session_count"] += 1
            continue
        message_time = _timestamp(record.get("message_time"))
        effective_time = message_time or _timestamp(record.get("received_at"))
        if effective_time is None:
            report["skipped_invalid_time_count"] += 1
            continue
        if _month_of(effective_time) != month:
            continue
        sender_id = str(record.get("sender_id") or "")
        report["missing_sender_count"] += int(not sender_id)
        report["missing_time_count"] += int(message_time is None)
        report["row_count"] += 1
        yield (
            str(record.get("anchor_name") or ""),
            session_id,
            sender_id,
            str(record.get("sender_name") or ""),
            MESSAGE_TYPE_LABELS[kind],
            str(record.get("content") or ""),
            message_time.isoformat(sep=" ") if message_time else "",
        )


def _csv_value(value, report):
    # Leading whitespace/control characters must not disguise a formula. Numeric
    # IDs remain their exact digit strings: never emit ="123..." formulas.
    stripped = re.sub(r"^[\s\x00-\x1f\ufeff]+", "", value)
    dangerous = value.startswith(("\t", "\r", "\n")) or stripped.startswith(("=", "+", "-", "@"))
    if dangerous:
        report["formula_protected_count"] += 1
        return "'" + value
    return value


def _write_csv(path, rows, report, cancel_event):
    with path.open("w", encoding="utf-8-sig", newline="") as stream:
        writer = csv.writer(stream, lineterminator="\r\n")
        writer.writerow(HEADERS)
        for row in rows:
            _check_cancel(cancel_event)
            writer.writerow([_csv_value(value, report) for value in row])
        stream.flush()
        os.fsync(stream.fileno())


def _xml_value(value, row_number, column):
    if _ILLEGAL_XML.search(value):
        raise ExportDataError(
            f"第 {row_number} 条消息的“{HEADERS[column]}”含 XLSX 无法原样保存的控制字符；请改用 CSV。"
        )
    if len(value.encode("utf-16-le")) // 2 > EXCEL_MAX_CELL_UNITS:
        raise ExportDataError(
            f"第 {row_number} 条消息的“{HEADERS[column]}”超过 Excel 单元格 32767 字符限制；请改用 CSV。"
        )
    # OOXML treats _xHHHH_ as an escape, even within inline strings. Escape its
    # literal leading underscore so a message containing such text is preserved.
    value = _LITERAL_OOXML_ESCAPE.sub(lambda match: "_x005F_" + match.group()[1:], value)
    return escape(value).replace("\r", "&#13;")


def _xml_row(values, sheet_row, source_row, header=False):
    cells = []
    for column, value in enumerate(values):
        cell = f"{chr(65 + column)}{sheet_row}"
        text = _xml_value(value, source_row, column)
        style = "1" if header else "2"
        cells.append(f'<c r="{cell}" s="{style}" t="inlineStr"><is><t xml:space="preserve">{text}</t></is></c>')
    return (f'<row r="{sheet_row}">' + "".join(cells) + "</row>").encode("utf-8")


def _sheet_start():
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<worksheet xmlns="{_SHEET_NS}">'
        '<sheetViews><sheetView workbookViewId="0">'
        '<pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/>'
        '<selection pane="bottomLeft" activeCell="A2" sqref="A2"/>'
        '</sheetView></sheetViews><sheetFormatPr defaultRowHeight="18"/>'
        '<cols><col min="1" max="1" width="24" customWidth="1"/>'
        '<col min="2" max="3" width="25" customWidth="1"/>'
        '<col min="4" max="4" width="24" customWidth="1"/>'
        '<col min="5" max="5" width="15" customWidth="1"/>'
        '<col min="6" max="6" width="72" customWidth="1"/>'
        '<col min="7" max="7" width="30" customWidth="1"/></cols><sheetData>'
    ).encode("utf-8")


_STYLES = f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="{_SHEET_NS}">
<fonts count="2"><font><sz val="11"/><name val="Microsoft YaHei"/></font><font><b/><sz val="11"/><name val="Microsoft YaHei"/></font></fonts>
<fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFDCE6F1"/><bgColor indexed="64"/></patternFill></fill></fills>
<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
<cellXfs count="3"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="49" fontId="1" fillId="2" borderId="0" xfId="0" applyAlignment="1"><alignment vertical="top" wrapText="1"/></xf><xf numFmtId="49" fontId="0" fillId="0" borderId="0" xfId="0" applyAlignment="1"><alignment vertical="top" wrapText="1"/></xf></cellXfs>
<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
</styleSheet>'''


def _sheet_name_attribute(name):
    # A literal _xNNNN_ in an Excel name needs the same OOXML escape protection
    # as cell text. XML attributes also need quotes and ampersands escaped.
    literal = _LITERAL_OOXML_ESCAPE.sub(lambda match: "_x005F_" + match.group()[1:], name)
    return quoteattr(literal)


def _write_workbook_parts(archive, sheet_names):
    sheet_count = len(sheet_names)
    sheets = "".join(f'<sheet name={_sheet_name_attribute(name)} sheetId="{number}" r:id="rId{number}"/>'
                     for number, name in enumerate(sheet_names, 1))
    archive.writestr("xl/workbook.xml", f'<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="{_SHEET_NS}" xmlns:r="{_REL_NS}"><sheets>{sheets}</sheets></workbook>')
    relations = "".join(f'<Relationship Id="rId{number}" Type="{_REL_NS}/worksheet" Target="worksheets/sheet{number}.xml"/>' for number in range(1, sheet_count + 1))
    relations += f'<Relationship Id="rIdStyles" Type="{_REL_NS}/styles" Target="styles.xml"/>'
    archive.writestr("xl/_rels/workbook.xml.rels", f'<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="{_PACKAGE_REL_NS}">{relations}</Relationships>')
    archive.writestr("_rels/.rels", f'<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="{_PACKAGE_REL_NS}"><Relationship Id="rId1" Type="{_REL_NS}/officeDocument" Target="xl/workbook.xml"/></Relationships>')
    overrides = ''.join(f'<Override PartName="/xl/worksheets/sheet{number}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>' for number in range(1, sheet_count + 1))
    archive.writestr("[Content_Types].xml", '<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>' + overrides + '</Types>')
    archive.writestr("xl/styles.xml", _STYLES)


def _trim_sheet_name(name, max_units=31):
    units = 0
    result = []
    for character in name:
        size = 2 if ord(character) > 0xFFFF else 1
        if units + size > max_units:
            break
        result.append(character)
        units += size
    # Truncation can expose an apostrophe at the end of an otherwise valid name.
    return re.sub(r"^[\s']+|[\s']+$", "", "".join(result))


class _SheetNames:
    def __init__(self):
        self.used = set()

    def claim(self, name, anchor_id, part=1):
        base = _ILLEGAL_SHEET_NAME.sub("_", str(name or ""))
        base = _trim_sheet_name(base) or _trim_sheet_name(f"主播 {anchor_id}")
        number = part
        while True:
            suffix = "" if number == 1 else f" ({number})"
            candidate = _trim_sheet_name(base, 31 - len(suffix)) + suffix
            key = candidate.casefold()
            if key != "history" and key not in self.used:
                self.used.add(key)
                return candidate
            number = max(2, number + 1)


def _write_xlsx(path, reader, anchor_ids, month, types, report, cancel_event):
    # Name lookup and every per-anchor query use the SAME read transaction.
    # Only metadata and one message lookahead are held in Python memory.
    names = {row["anchor_id"]: row["anchor_name"] for row in reader.list_anchors(anchor_ids)}
    allocator = _SheetNames()
    first_names = {anchor_id: allocator.claim(names.get(anchor_id), anchor_id) for anchor_id in anchor_ids}
    sheet_names = []
    source_row = 0
    with path.open("w+b") as stream:
        with ZipFile(stream, "w", compression=ZIP_DEFLATED, compresslevel=6, allowZip64=True) as archive:
            for anchor_id in anchor_ids:
                _check_cancel(cancel_event)
                pending = iter(_records(reader, [anchor_id], month, types, report, cancel_event))
                first_row = next(pending, None)
                part = 1
                # A selected anchor always gets a header, even with no rows.
                while part == 1 or first_row is not None:
                    _check_cancel(cancel_event)
                    name = first_names[anchor_id] if part == 1 else allocator.claim(first_names[anchor_id], anchor_id, part)
                    sheet_names.append(name)
                    sheet_row = 1
                    with archive.open(f"xl/worksheets/sheet{len(sheet_names)}.xml", "w", force_zip64=True) as sheet:
                        sheet.write(_sheet_start())
                        sheet.write(_xml_row(HEADERS, 1, 0, header=True))
                        while first_row is not None and sheet_row < EXCEL_MAX_ROWS:
                            _check_cancel(cancel_event)
                            source_row += 1
                            sheet_row += 1
                            sheet.write(_xml_row(first_row, sheet_row, source_row))
                            first_row = next(pending, None)
                        sheet.write(f'</sheetData><autoFilter ref="A1:G{sheet_row}"/></worksheet>'.encode("utf-8"))
                    part += 1
            _write_workbook_parts(archive, sheet_names)
        stream.flush()
        os.fsync(stream.fileno())
    report["sheet_count"] = len(sheet_names)


def export_month(data_dir, anchor_ids, month, output_path, format=None, types=None, cancel_event=None):
    """Export selected anchors' content messages using a single archive snapshot.

    ``month`` is YYYY-MM in Asia/Shanghai. Missing server timestamps use the
    reception month for inclusion, with a blank exported message-time cell.
    CSV uses UTF-8 BOM; import ID columns as text in Excel to retain every digit.
    XLSX groups by stable anchor ID, naming sheets from snapshot metadata.
    Cancellation or any error leaves an existing destination untouched.
    """
    month = normalize_month(month)
    anchor_ids = _selected_values(anchor_ids, "主播")
    if not anchor_ids:
        raise ValueError("请至少选择一个主播。")
    selected_types = tuple(MESSAGE_TYPE_LABELS) if types is None else _selected_values(types, "消息类型")
    if set(selected_types) - MESSAGE_TYPE_LABELS.keys():
        raise ValueError("仅可导出弹幕、礼物、点赞、进场、关注/分享、粉丝团和表情弹幕。")
    destination = Path(output_path).expanduser().absolute()
    output_format = str(format or destination.suffix.lstrip(".")).lower().lstrip(".")
    if output_format not in {"csv", "xlsx"}:
        raise ValueError("导出格式须为 xlsx 或 csv。")
    if destination.suffix and destination.suffix.lower() != "." + output_format:
        raise ValueError("文件扩展名须与导出格式一致。")
    if not destination.parent.is_dir():
        raise FileNotFoundError("导出目录不存在，请选择一个已有文件夹。")
    _check_cancel(cancel_event)
    report = {
        "row_count": 0, "path": str(destination), "format": output_format, "month": month,
        "missing_sender_count": 0, "missing_time_count": 0, "sheet_count": 0,
        "formula_protected_count": 0, "skipped_missing_session_count": 0,
        "skipped_invalid_time_count": 0,
    }
    temporary = None
    # Lazy import also keeps format validation usable when no archive is open.
    from archive_store import ArchiveStore
    store = ArchiveStore(data_dir)
    try:
        with tempfile.NamedTemporaryFile(prefix=f".{destination.name}.", suffix=".tmp", dir=destination.parent, delete=False) as handle:
            temporary = Path(handle.name)
        with store.export_snapshot() as reader:
            if output_format == "xlsx":
                _write_xlsx(temporary, reader, anchor_ids, month, selected_types, report, cancel_event)
            else:
                rows = _records(reader, anchor_ids, month, selected_types, report, cancel_event)
                _write_csv(temporary, rows, report, cancel_event)
        _check_cancel(cancel_event)
        os.replace(temporary, destination)
        return report
    finally:
        store.close()
        if temporary is not None:
            temporary.unlink(missing_ok=True)
