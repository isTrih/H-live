"""Rust write adapter; read-only snapshot compatibility for diagnostic callers."""
from contextlib import contextmanager
from datetime import datetime, timedelta, timezone
import json
import math
from pathlib import Path
import re
import sqlite3
from app_storage import app_data_dir
from qingkan_native import ArchiveStore as _NativeArchive, ArchiveDeleteCancelled
CONTENT_TYPES = frozenset({"chat", "gift", "like", "member", "social", "fansclub", "emoji", "activity"})
SCHEMA_VERSION = 3
SHANGHAI = timezone(timedelta(hours=8), "Asia/Shanghai")

def server_time(value):
    """Return a normalized server timestamp, never substitute the receiving time."""
    try:
        seconds = float(value)
        if isinstance(value, bool) or not math.isfinite(seconds) or seconds <= 0:
            return "", None
        stamp = datetime.fromtimestamp(seconds, SHANGHAI)
        return stamp.isoformat(timespec="milliseconds"), seconds
    except (TypeError, ValueError, OverflowError, OSError):
        return "", None


def month_bounds(month):
    if not isinstance(month, str) or not re.fullmatch(r"[0-9]{4}-(?:0[1-9]|1[0-2])", month):
        raise ValueError("月份必须为 YYYY-MM，例如 2026-10")
    year, number = map(int, month.split("-"))
    try:
        start = datetime(year, number, 1, tzinfo=SHANGHAI)
        end = datetime(year + (number == 12), number % 12 + 1, 1, tzinfo=SHANGHAI)
        return start.timestamp(), end.timestamp()
    except (ValueError, OverflowError, OSError) as exc:
        raise ValueError("月份超出支持范围") from exc


def _identity(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9]{1,64}", value) or int(value) == 0:
        raise ValueError("缺少有效的真实主播 ID 或直播场次 ID")
    return value


def _filters(anchor_ids=None, month=None, types=None):
    clauses, parameters = [], []
    if anchor_ids is not None:
        if isinstance(anchor_ids, str):
            raise ValueError("主播 ID 应为列表")
        ids = list(dict.fromkeys(_identity(item) for item in anchor_ids))
        if not ids:
            clauses.append("0")
        else:
            clauses.append(f"anchor_id IN ({','.join('?' for _ in ids)})")
            parameters.extend(ids)
    if month is not None:
        begin, end = month_bounds(month)
        clauses.append("COALESCE(message_time_epoch, received_at_epoch) >= ? AND COALESCE(message_time_epoch, received_at_epoch) < ?")
        parameters.extend((begin, end))
    if types is not None:
        if isinstance(types, str):
            raise ValueError("消息类型应为列表")
        chosen = sorted(set(types))
        if not set(chosen).issubset(CONTENT_TYPES):
            raise ValueError("只能导出聊天与互动消息类型")
        if not chosen:
            clauses.append("0")
        else:
            clauses.append(f"message_type IN ({','.join('?' for _ in chosen)})")
            parameters.extend(chosen)
    return " AND ".join(clauses) or "1", parameters


class ArchiveSnapshot:
    def __init__(self, connection):
        self.connection = connection

    def list_anchors(self, anchor_ids=None):
        condition, parameters = _filters(anchor_ids)
        return [dict(row) for row in self.connection.execute(f"""
            SELECT a.anchor_id, a.anchor_name, a.web_rid FROM anchors a
            WHERE ({condition}) AND EXISTS(SELECT 1 FROM messages m WHERE m.anchor_id=a.anchor_id)
            ORDER BY a.anchor_name, a.anchor_id
        """, parameters)]

    def iter_messages(self, anchor_ids, month, types=None):
        # Missing server times belong to the receipt month, but message_time
        # remains empty and time_source remains 'missing' in exported rows.
        month_bounds(month)
        condition, parameters = _filters(anchor_ids, month, types)
        cursor = self.connection.execute(
            f"SELECT * FROM messages WHERE {condition} ORDER BY COALESCE(message_time_epoch, received_at_epoch), id",
            parameters,
        )
        for row in cursor:
            yield dict(row)

    def count_missing_time(self, anchor_ids=None, month=None, types=None):
        condition, parameters = _filters(anchor_ids, month, types)
        return self.connection.execute(
            f"SELECT COUNT(*) FROM messages WHERE message_time_epoch IS NULL AND ({condition})", parameters
        ).fetchone()[0]


class ArchiveStore:
    def __init__(self, data_dir=None, queue_capacity=4096, batch_size=256, batch_wait_seconds=.004):
        self.data_dir = Path(data_dir) if data_dir is not None else app_data_dir()
        self.db_path = self.data_dir / "messages.sqlite3"
        self.path = self.db_path
        self._native = _NativeArchive(str(self.data_dir), queue_capacity, batch_size, batch_wait_seconds)

    def record_message(self, record, title="", started_at=None):
        return self._native.record_message(json.dumps(record, ensure_ascii=False, default=str), str(title), started_at or None)

    def metrics(self):
        return json.loads(self._native.metrics())

    def recent_messages(self, params=None):
        return json.loads(self._native.recent_messages(json.dumps(params or {}, ensure_ascii=False)))

    def list_anchors(self):
        return json.loads(self._native.list_anchors())

    def quality(self, action, params=None, cancel_token=None):
        return json.loads(self._native.quality(action, json.dumps(params or {}, ensure_ascii=False), cancel_token))

    def register_session(self, anchor_id, session_id, anchor_name, web_rid, title="", started_at=None):
        return self.quality("register", {"anchor_id": anchor_id, "session_id": session_id,
                            "anchor_name": anchor_name, "web_rid": web_rid,
                            "title": title, "started_at": started_at})

    def list_sessions(self, selection, limit=200, offset=0):
        return json.loads(self._native.list_sessions(json.dumps(selection), limit, offset))

    def preview_delete(self, selection):
        return json.loads(self._native.preview_delete(json.dumps(selection)))

    def execute_delete(self, selection, cancel_token=None):
        return json.loads(self._native.execute_delete(json.dumps(selection), cancel_token))

    def count_missing_time(self, anchor_ids=None, month=None, types=None):
        return self._native.count_missing_time(anchor_ids, month, types)

    def load_monitor_duration(self, anchor_id, session_id):
        return self._native.load_monitor_duration(_identity(anchor_id), _identity(session_id))

    def save_monitor_duration(self, anchor_id, session_id, seconds):
        if isinstance(seconds, bool) or not isinstance(seconds, (int, float)):
            raise ValueError("累计监控时长必须为非负有限秒数")
        try:
            seconds = float(seconds)
        except OverflowError as exc:
            raise ValueError("累计监控时长必须为非负有限秒数") from exc
        if not math.isfinite(seconds) or seconds < 0:
            raise ValueError("累计监控时长必须为非负有限秒数")
        self._native.save_monitor_duration(_identity(anchor_id), _identity(session_id), seconds)

    @contextmanager
    def export_snapshot(self):
        # Compatibility reads only; production month exports never pass rows
        # through this Python connection or allocate a Python row per message.
        connection = sqlite3.connect(self.db_path.resolve().as_uri() + "?mode=ro", uri=True, timeout=10)
        connection.row_factory = sqlite3.Row
        try:
            connection.execute("PRAGMA query_only=ON")
            connection.execute("BEGIN")
            if not 0 <= connection.execute("PRAGMA user_version").fetchone()[0] <= SCHEMA_VERSION:
                raise RuntimeError("消息库版本较新，请使用更新版本的应用")
            connection.execute("SELECT COUNT(*) FROM anchors").fetchone()
            yield ArchiveSnapshot(connection)
        finally:
            connection.rollback()
            connection.close()

    def iter_messages(self, anchor_ids, month, types=None):
        with self.export_snapshot() as reader:
            yield from reader.iter_messages(anchor_ids, month, types)

    def close(self, timeout=15):
        return self._native.close(timeout)

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()
