"""SQLite message archive with real anchor/session identities and export snapshots."""

from contextlib import contextmanager
from datetime import datetime, timedelta, timezone
import json
import math
from pathlib import Path
import re
import queue
import sqlite3
import threading
import time
from dataclasses import dataclass, field

from app_storage import app_data_dir

CONTENT_TYPES = frozenset({"chat", "gift", "like", "member", "social", "fansclub", "emoji"})
SHANGHAI = timezone(timedelta(hours=8), "Asia/Shanghai")
_MESSAGE_FIELDS = ("anchor_id", "anchor_name", "web_rid", "session_id", "sender_id", "sender_name",
                   "message_type", "content", "message_time", "received_at", "time_source", "message_id")
_INSERT_MESSAGE = f"""INSERT INTO messages ({','.join(_MESSAGE_FIELDS)},message_time_epoch,received_at_epoch,payload_json)
    VALUES({','.join('?' for _ in range(len(_MESSAGE_FIELDS) + 3))})
    ON CONFLICT(anchor_id,session_id,message_id) WHERE message_id <> '' DO NOTHING"""


@dataclass
class _WriteRequest:
    record: dict
    values: list
    title: str
    started_at: str | None
    done: threading.Event = field(default_factory=threading.Event)
    inserted: bool = False
    error: Exception | None = None


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
    """Messages only; old JSONL files are intentionally not imported or deleted."""

    def __init__(self, data_dir=None, queue_capacity=4096, batch_size=256, batch_wait_seconds=0.004):
        if queue_capacity < 1 or batch_size < 1 or batch_wait_seconds < 0:
            raise ValueError("归档队列及批次参数无效")
        self.data_dir = Path(data_dir) if data_dir is not None else app_data_dir()
        self.data_dir.mkdir(parents=True, exist_ok=True)
        self.db_path = self.data_dir / "messages.sqlite3"
        self.path = self.db_path
        self._lock = threading.RLock()
        self._writer = None
        self._queue = queue.Queue(maxsize=queue_capacity)
        self._batch_size = batch_size
        self._batch_wait_seconds = batch_wait_seconds
        self._closed = False
        self._writer_stats = {"committed_messages": 0, "duplicates": 0, "batches": 0, "queue_high_water": 0,
                              "largest_batch": 0, "failed_batches": 0}
        connection = self._connect()
        try:
            version = connection.execute("PRAGMA user_version").fetchone()[0]
            if version > 1:
                raise RuntimeError("消息库版本较新，请使用更新版本的应用")
            connection.execute("PRAGMA journal_mode=WAL")
            connection.executescript("""
                CREATE TABLE IF NOT EXISTS anchors (
                    anchor_id TEXT PRIMARY KEY NOT NULL, anchor_name TEXT NOT NULL,
                    web_rid TEXT NOT NULL, first_seen TEXT NOT NULL, last_seen TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS sessions (
                    anchor_id TEXT NOT NULL, session_id TEXT NOT NULL,
                    anchor_name TEXT NOT NULL, web_rid TEXT NOT NULL, title TEXT NOT NULL,
                    first_seen TEXT NOT NULL, last_seen TEXT NOT NULL, started_at TEXT,
                    PRIMARY KEY(anchor_id, session_id)
                );
                CREATE TABLE IF NOT EXISTS messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    anchor_id TEXT NOT NULL, anchor_name TEXT NOT NULL, web_rid TEXT NOT NULL,
                    session_id TEXT NOT NULL, sender_id TEXT NOT NULL, sender_name TEXT NOT NULL,
                    message_type TEXT NOT NULL, content TEXT NOT NULL,
                    message_time TEXT NOT NULL, received_at TEXT NOT NULL,
                    time_source TEXT NOT NULL CHECK(time_source IN ('server','missing')),
                    message_id TEXT NOT NULL, message_time_epoch REAL, received_at_epoch REAL NOT NULL,
                    payload_json TEXT NOT NULL,
                    CHECK(anchor_id <> '' AND session_id <> ''),
                    CHECK(message_type IN ('chat','gift','like','member','social','fansclub','emoji')),
                    FOREIGN KEY(anchor_id,session_id) REFERENCES sessions(anchor_id,session_id)
                );
                CREATE UNIQUE INDEX IF NOT EXISTS messages_server_identity
                    ON messages(anchor_id,session_id,message_id) WHERE message_id <> '';
                CREATE INDEX IF NOT EXISTS messages_anchor_month
                    ON messages(anchor_id,COALESCE(message_time_epoch,received_at_epoch));
                PRAGMA user_version=1;
            """)
        finally:
            connection.close()

    def _connect(self):
        connection = sqlite3.connect(str(self.db_path), timeout=10, check_same_thread=False)
        connection.row_factory = sqlite3.Row
        connection.execute("PRAGMA foreign_keys=ON")
        connection.execute("PRAGMA synchronous=NORMAL")
        return connection

    def _start_writer(self):
        if self._writer is None:
            writer = threading.Thread(target=self._writer_loop, name="message-archive-writer", daemon=True)
            # If thread creation fails, a later room must be able to retry;
            # retaining an unstarted thread would leave queued requests waiting.
            writer.start()
            self._writer = writer

    @staticmethod
    def _save_batch_sessions(connection, batch):
        anchors, sessions = {}, {}
        for request in batch:
            record = request.record
            seen = record["received_at"]
            anchor = [record["anchor_id"], record["anchor_name"], record["web_rid"], seen, seen]
            previous = anchors.get(record["anchor_id"])
            if previous:
                anchor[3], anchor[4] = min(previous[3], seen), max(previous[4], seen)
            anchors[record["anchor_id"]] = anchor
            key = record["anchor_id"], record["session_id"]
            session = [*key, record["anchor_name"], record["web_rid"], str(request.title), seen, seen, request.started_at or None]
            previous = sessions.get(key)
            if previous:
                session[5], session[6] = min(previous[5], seen), max(previous[6], seen)
                session[7] = previous[7] or session[7]
            sessions[key] = session
        connection.executemany("""INSERT INTO anchors VALUES(?,?,?,?,?)
            ON CONFLICT(anchor_id) DO UPDATE SET anchor_name=excluded.anchor_name,web_rid=excluded.web_rid,
            first_seen=MIN(anchors.first_seen,excluded.first_seen),last_seen=MAX(anchors.last_seen,excluded.last_seen)""", anchors.values())
        connection.executemany("""INSERT INTO sessions VALUES(?,?,?,?,?,?,?,?)
            ON CONFLICT(anchor_id,session_id) DO UPDATE SET anchor_name=excluded.anchor_name,web_rid=excluded.web_rid,
            title=excluded.title,first_seen=MIN(sessions.first_seen,excluded.first_seen),last_seen=MAX(sessions.last_seen,excluded.last_seen),
            started_at=COALESCE(sessions.started_at,excluded.started_at)""", sessions.values())

    def _writer_loop(self):
        connection = None
        try:
            while True:
                first = self._queue.get()
                if first is None:
                    self._queue.task_done()
                    break
                batch, ending = [first], False
                deadline = time.monotonic() + self._batch_wait_seconds
                while len(batch) < self._batch_size:
                    try:
                        request = self._queue.get(timeout=max(0, deadline - time.monotonic()))
                    except queue.Empty:
                        break
                    if request is None:
                        self._queue.task_done()
                        ending = True
                        break
                    batch.append(request)
                try:
                    if connection is None:
                        connection = self._connect()
                    with connection:
                        self._save_batch_sessions(connection, batch)
                        for request in batch:
                            request.inserted = connection.execute(_INSERT_MESSAGE, request.values).rowcount == 1
                    inserted = sum(request.inserted for request in batch)
                    self._writer_stats["committed_messages"] += inserted
                    self._writer_stats["duplicates"] += len(batch) - inserted
                    self._writer_stats["batches"] += 1
                    self._writer_stats["largest_batch"] = max(self._writer_stats["largest_batch"], len(batch))
                except Exception as exc:
                    self._writer_stats["failed_batches"] += 1
                    for request in batch:
                        request.error = exc
                finally:
                    # No caller observes success until the complete transaction
                    # has committed. Failure rolls the entire batch back.
                    for request in batch:
                        request.done.set()
                        self._queue.task_done()
                if ending:
                    break
        finally:
            if connection is not None:
                connection.close()

    @staticmethod
    def _save_session(connection, record, title, started_at):
        seen = record["received_at"]
        connection.execute("""INSERT INTO anchors VALUES(?,?,?,?,?)
            ON CONFLICT(anchor_id) DO UPDATE SET anchor_name=excluded.anchor_name,
            web_rid=excluded.web_rid,first_seen=MIN(anchors.first_seen,excluded.first_seen),
            last_seen=MAX(anchors.last_seen,excluded.last_seen)""",
            (record["anchor_id"], record["anchor_name"], record["web_rid"], seen, seen))
        connection.execute("""INSERT INTO sessions VALUES(?,?,?,?,?,?,?,?)
            ON CONFLICT(anchor_id,session_id) DO UPDATE SET anchor_name=excluded.anchor_name,
            web_rid=excluded.web_rid,title=excluded.title,
            first_seen=MIN(sessions.first_seen,excluded.first_seen),last_seen=MAX(sessions.last_seen,excluded.last_seen),
            started_at=COALESCE(sessions.started_at,excluded.started_at)""",
            (record["anchor_id"], record["session_id"], record["anchor_name"], record["web_rid"],
             str(title), seen, seen, started_at or None))

    def record_message(self, record, title="", started_at=None):
        """Commit one normalized message. False means a known message ID duplicate."""
        record = dict(record)
        _identity(record["anchor_id"])
        _identity(record["session_id"])
        if record["message_type"] not in CONTENT_TYPES:
            raise ValueError("观众统计与运行日志不能写入内容消息库")
        if record["time_source"] not in {"server", "missing"}:
            raise ValueError("无效时间来源")
        received = datetime.fromisoformat(record["received_at"])
        if received.tzinfo is None:
            raise ValueError("接收时间必须包含时区")
        record["received_at"] = received.astimezone(SHANGHAI).isoformat(timespec="milliseconds")
        if record["message_time"]:
            sent = datetime.fromisoformat(record["message_time"])
            if sent.tzinfo is None or record["time_source"] != "server":
                raise ValueError("服务端发送时间必须包含时区")
            message_epoch = sent.timestamp()
            record["message_time"] = sent.astimezone(SHANGHAI).isoformat(timespec="milliseconds")
        else:
            if record["time_source"] != "missing":
                raise ValueError("缺失发送时间必须明确标识")
            message_epoch = None
        values = ["" if record.get(key) is None else str(record.get(key, "")) for key in _MESSAGE_FIELDS]
        values.extend((message_epoch, received.timestamp(), json.dumps(record.get("payload", {}), ensure_ascii=False, default=str)))
        with self._lock:
            if self._closed:
                raise RuntimeError("消息库已关闭")
            self._start_writer()
            request = _WriteRequest(record, values, title, started_at)
            self._queue.put(request)
            self._writer_stats["queue_high_water"] = max(self._writer_stats["queue_high_water"], self._queue.qsize())
        request.done.wait()
        if request.error is not None:
            raise request.error
        return request.inserted

    @contextmanager
    def export_snapshot(self):
        connection = sqlite3.connect(self.db_path.resolve().as_uri() + "?mode=ro", uri=True, timeout=10)
        connection.row_factory = sqlite3.Row
        try:
            connection.execute("PRAGMA query_only=ON")
            connection.execute("BEGIN")
            connection.execute("SELECT COUNT(*) FROM anchors").fetchone()
            yield ArchiveSnapshot(connection)
        finally:
            connection.rollback()
            connection.close()

    def list_anchors(self):
        with self.export_snapshot() as reader:
            return reader.list_anchors()

    def iter_messages(self, anchor_ids, month, types=None):
        with self.export_snapshot() as reader:
            yield from reader.iter_messages(anchor_ids, month, types)

    def count_missing_time(self, anchor_ids=None, month=None, types=None):
        with self.export_snapshot() as reader:
            return reader.count_missing_time(anchor_ids, month, types)

    def metrics(self):
        return {**self._writer_stats, "mode": "bounded_batch_writer", "queue_depth": self._queue.qsize(),
                "pending_requests": self._queue.unfinished_tasks,
                "queue_capacity": self._queue.maxsize, "batch_size": self._batch_size,
                "batch_wait_seconds": self._batch_wait_seconds,
                "writer_alive": self._writer is not None and self._writer.is_alive()}

    def close(self, timeout=15):
        with self._lock:
            if not self._closed:
                self._closed = True
                if self._writer is not None:
                    self._queue.put(None)
            writer = self._writer
        if writer is not None and writer is not threading.current_thread():
            writer.join(timeout)
        return writer is None or not writer.is_alive()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()
