"""Independent room monitors with session-aware content and separate run logs."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
import json
import math
import inspect
from pathlib import Path
import queue
import threading
import time
from concurrent.futures import ThreadPoolExecutor,as_completed
from typing import Callable

from app_storage import app_data_dir, parse_room_id
from archive_store import ArchiveStore, CONTENT_TYPES, SHANGHAI, _identity, server_time
from route_control import RouteControl
from monitor_duration import DurationLedger


@dataclass(eq=False)
class _RoomWorker:
    rid: str
    name: str
    cookie: str = field(repr=False)
    stop_event: threading.Event = field(default_factory=threading.Event)
    write_lock: threading.RLock = field(default_factory=threading.RLock)
    thread: threading.Thread | None = None
    fetcher: object | None = None
    attempt: object | None = None
    state: str = "connecting"
    detail: str = "准备监控"
    message_count: int = 0
    last_message_at: str = ""
    title: str = ""
    anchor_id: str = ""
    session_id: str = ""
    started_at: str = ""
    broadcast_started_at: int | None = None
    task_started_at: int = 0
    duration_started_at: int | None = None
    duration_source: str = ""
    duration_session_id: str = ""
    duration_seconds: float = 0
    duration_running: bool = False
    duration_updated_at: float | None = None
    duration_initial_from_task: bool = False
    observed_offline: bool = False
    viewer_count: int | None = None
    viewer_updated_at: str = ""
    viewer_revision: int = 0
    is_live: bool | None = None
    live_checked_at: str = ""
    proxy_pool: object | None = field(default=None, repr=False)
    proxy_lease: object | None = field(default=None, repr=False)
    traffic_control: object | None = field(default=None, repr=False)
    proxy: str = ""
    proxy_status: str = "direct"
    route_id: str = ""
    route_kind: str = ""
    predecessors: tuple = field(default=(), repr=False)


class MonitorManager:
    """One bounded-memory worker per room; UI callbacks must return promptly.

    Archival is synchronous on the receiving thread and precedes the optional
    UI callback. A full UI queue therefore cannot cause archive loss. Different
    rooms write independently, and obsolete workers cannot publish events.
    """

    def __init__(
        self,
        on_event: Callable[[dict], None] | None = None,
        data_dir: str | Path | None = None,
        fetcher_factory=None,
        room_info_fetcher=None,
        retry_seconds: float = 5,
        offline_seconds: float = 30,
        max_connecting: int = 8,
        connect_interval_seconds: float = 0.05,
        proxy_pool=None,
    ):
        if retry_seconds <= 0 or offline_seconds <= 0:
            raise ValueError("重试间隔必须大于零")
        self.on_event = on_event
        self.data_dir = Path(data_dir) if data_dir is not None else app_data_dir()
        self.fetcher_factory = fetcher_factory
        self.room_info_fetcher = room_info_fetcher
        self.retry_seconds = float(retry_seconds)
        self.offline_seconds = float(offline_seconds)
        if not 1 <= max_connecting <= 8 or connect_interval_seconds < 0:
            raise ValueError("同时建立连接数须为 1–8，启动间隔不能为负")
        self._connection_gate = threading.BoundedSemaphore(max_connecting)
        self._connect_interval = float(connect_interval_seconds)
        self._connect_schedule_lock = threading.Lock()
        self._next_connect_at = 0.0
        self._connecting_pending = 0
        self._connecting_inflight = 0
        self._max_connecting = max_connecting
        self._lock = threading.RLock()
        self._rooms: dict[str, _RoomWorker] = {}
        self._threads: set[threading.Thread] = set()
        self._closed = False
        self._proxy_pool = proxy_pool
        self._direct_control = RouteControl()
        self.archive = ArchiveStore(self.data_dir)
        self._log_queue = queue.Queue(maxsize=8192)
        self._log_warning_sent = False
        self._logs_closed = False
        self._log_thread = threading.Thread(target=self._log_writer, name="monitor-run-log-writer", daemon=True)
        self._log_thread.start()
        self._duration_warning_sent = False
        self._audience_warning_sent = False
        self._viewer_refresh = {"id":"0","active":False,"total":0,"checked":0,"updated":0,"failed":0,"skipped":0}
        self._viewer_refresh_at = -float('inf')
        self._durations = DurationLedger(self.archive, self._warn_duration_failure, wall_clock=lambda: time.time())

    def _warn_duration_failure(self):
        with self._lock:
            if not self._duration_warning_sent:
                self._duration_warning_sent = True
                event = {"kind": "log", "rid": "", "name": "", "msg_type": "warning",
                         "detail": "累计监控时长保存失败；本次内存计时继续，请检查磁盘权限或空间，退出前需保存成功"}
                self._log_event(event)
                self._notify(event)

    def _capture_duration(self, worker):
        value = self._durations.snapshot(worker)
        if value is not None:
            for key, item in value.items():
                setattr(worker, key, item)

    def _pause_duration(self, worker, attempt=None):
        with worker.write_lock:
            with self._lock:
                if attempt is not None and (not self._active(worker) or worker.attempt is not attempt):
                    return
                self._durations.pause(worker)
                self._capture_duration(worker)

    def _active(self, worker: _RoomWorker) -> bool:
        return self._rooms.get(worker.rid) is worker and not worker.stop_event.is_set()

    def _notify(self, event: dict) -> None:
        # Called under the RLock so an old callback cannot arrive after a new
        # generation's start/stop event. GUI clients only enqueue here.
        if self.on_event is not None:
            try:
                self.on_event(event)
            except Exception:
                # A failed UI callback must not interrupt archival or sockets.
                pass

    def _status(self, worker: _RoomWorker, state: str, detail: str = "", attempt=None) -> None:
        with self._lock:
            if not self._active(worker) or (attempt is not None and worker.attempt is not attempt):
                return
            worker.state, worker.detail = state, str(detail)
            event = {"kind": "status", "rid": worker.rid, "name": worker.name,
                     "state": state, "detail": worker.detail, "anchor_id": worker.anchor_id,
                     "session_id": worker.session_id}
            self._log_event(event)
            self._notify(event)

    def _log_event(self, event):
        """Runtime diagnostics are never stored in the content database."""
        now = datetime.now(SHANGHAI)
        event.setdefault("timestamp", now.isoformat(timespec="milliseconds"))
        event.setdefault("time_str", now.strftime("%H:%M:%S"))
        if self._logs_closed:
            return
        try:
            self._log_queue.put_nowait(dict(event))
        except queue.Full:
            self._warn_log_failure("运行日志队列已满，部分日志未保存；聊天归档不受影响")

    def _warn_log_failure(self, detail):
        with self._lock:
            if not self._log_warning_sent:
                self._log_warning_sent = True
                now = datetime.now(SHANGHAI)
                self._notify({"kind": "log", "rid": "", "name": "", "msg_type": "warning", "detail": detail,
                              "timestamp": now.isoformat(timespec="milliseconds"), "time_str": now.strftime("%H:%M:%S")})

    def _log_writer(self):
        stream, current_day = None, None
        try:
            while True:
                event = self._log_queue.get()
                if event is None:
                    self._log_queue.task_done()
                    break
                batch = [event]
                ending = False
                while len(batch) < 128:
                    try:
                        item = self._log_queue.get_nowait()
                    except queue.Empty:
                        break
                    if item is None:
                        self._log_queue.task_done()
                        ending = True
                        break
                    batch.append(item)
                try:
                    for item in batch:
                        day = item["timestamp"][:10]
                        if stream is None or day != current_day:
                            if stream is not None:
                                stream.close()
                            folder = self.data_dir / "logs"
                            folder.mkdir(parents=True, exist_ok=True)
                            stream = (folder / f"{day}.jsonl").open("a", encoding="utf-8", newline="")
                            current_day = day
                        stream.write(json.dumps(item, ensure_ascii=False, default=str) + "\n")
                    stream.flush()
                except Exception:
                    if stream is not None:
                        try:
                            stream.close()
                        except Exception:
                            pass
                        stream = None
                    self._warn_log_failure("运行日志写入失败；聊天归档继续，请检查日志目录权限或磁盘空间")
                finally:
                    for _ in batch:
                        self._log_queue.task_done()
                if ending:
                    break
        finally:
            if stream is not None:
                stream.close()

    def _connection_slot(self, worker):
        with self._lock:
            self._connecting_pending += 1
        acquired = False
        route_release = None
        waiting_state = "confirming_offline" if worker.state == "confirming_offline" else "connecting"
        try:
            if worker.traffic_control is not None:
                cooldown = worker.traffic_control.snapshot()["cooldown_seconds"]
                detail = f"出口冷却中，约 {math.ceil(cooldown)} 秒后重试" if cooldown > 0 else "等待出口建连名额"
                self._status(worker, waiting_state, detail)
                route_release = worker.traffic_control.acquire_connect(worker.stop_event)
                if route_release is None:
                    return None
            self._status(worker, waiting_state, "等待全局建连名额并查询开播状态")
            while not worker.stop_event.is_set():
                if self._connection_gate.acquire(timeout=0.1):
                    acquired = True
                    break
        except Exception:
            if route_release is not None:
                route_release()
            raise
        finally:
            with self._lock:
                self._connecting_pending -= 1
        if not acquired:
            if route_release is not None:
                route_release()
            return None
        with self._lock:
            self._connecting_inflight += 1
        released = threading.Event()
        release_lock = threading.Lock()
        def release():
            with release_lock:
                if not released.is_set():
                    released.set()
                    self._connection_gate.release()
                    if route_release is not None:
                        route_release()
                    with self._lock:
                        self._connecting_inflight -= 1
        with self._connect_schedule_lock:
            scheduled = max(time.monotonic(), self._next_connect_at)
            self._next_connect_at = scheduled + self._connect_interval
        if worker.stop_event.wait(max(0, scheduled - time.monotonic())):
            release()
            return None
        return release

    def _fail(self, worker, detail):
        with worker.write_lock:
            with self._lock:
                if not self._active(worker):
                    return
                worker.stop_event.set()
                self._durations.pause(worker)
                self._capture_duration(worker)
                worker.attempt = None
                worker.state, worker.detail = "error", detail
                event = {"kind": "status", "rid": worker.rid, "name": worker.name,
                         "anchor_id": worker.anchor_id, "session_id": worker.session_id,
                         "state": "error", "detail": detail}
                self._log_event(event)
                self._notify(event)
                fetcher = worker.fetcher
        self._stop_fetcher(fetcher)

    def start(self, rid: str, name: str, cookie: str = "") -> bool:
        rid = parse_room_id(rid)
        with self._lock:
            if self._closed:
                return False
            previous = self._rooms.get(rid)
            if previous is not None and self._active(previous):
                return False
            worker = _RoomWorker(rid, str(name).strip() or rid, cookie)
            worker.task_started_at = int(time.time())
            worker.proxy_pool = self._proxy_pool
            worker.traffic_control = self._direct_control if worker.proxy_pool is None else None
            if worker.proxy_pool is None:
                worker.route_id, worker.route_kind = "direct", "direct"
            worker.proxy_status = "waiting" if worker.proxy_pool is not None else "direct"
            if previous is not None:
                worker.write_lock = previous.write_lock
            # An intermediate cancelled generation may exit before its own
            # predecessor. Wait for every old generation, not just the latest.
            worker.predecessors = tuple(thread for thread in self._threads if thread.name == f"room-{rid}")
            self._rooms[rid] = worker
            thread = threading.Thread(target=self._run, args=(worker,), name=f"room-{rid}", daemon=True)
            worker.thread = thread
            self._threads.add(thread)
            self._status(worker, "connecting", "正在查询开播状态")
            try:
                thread.start()
            except RuntimeError:
                self._threads.discard(thread)
                worker.stop_event.set()
                worker.cookie = ""
                worker.state, worker.detail = "error", "系统无法创建监控线程，请减少同时监控数量后重新开始"
                event = {"kind": "status", "rid": rid, "name": worker.name,
                         "state": worker.state, "detail": worker.detail, "anchor_id": "", "session_id": ""}
                self._log_event(event)
                self._notify(event)
                return False
            return True

    def rename(self, rid: str, name: str) -> bool:
        rid = parse_room_id(rid)
        with self._lock:
            worker = self._rooms.get(rid)
            if worker is None:
                return False
            worker.name = str(name).strip() or rid
            return True

    def can_configure_proxy(self):
        with self._lock:
            return not any(self._active(worker) for worker in self._rooms.values()) and not self._threads

    def can_delete_archive(self, anchor_ids):
        """Keep selected and unidentified producers out of destructive writes."""
        selected = set(anchor_ids)
        with self._lock:
            pending_rids = {thread.name.removeprefix("room-") for thread in self._threads}
            for rid, worker in self._rooms.items():
                if not self._active(worker) and rid not in pending_rids:
                    continue
                # A previous generation can still be resolving its identity.
                if any(thread in self._threads for thread in worker.predecessors):
                    return False
                if not worker.anchor_id or worker.anchor_id in selected:
                    return False
                pending_rids.discard(rid)
            return not pending_rids

    def configure_proxy(self, pool):
        with self._lock:
            if not self.can_configure_proxy():
                raise RuntimeError("请先停止全部监控并等待连接退出后再切换代理模式")
            previous = self._proxy_pool
            self._proxy_pool = pool
            for worker in self._rooms.values():
                worker.proxy = ""
                worker.proxy_status = "waiting" if pool is not None else "direct"
                worker.route_id, worker.route_kind = ("", "") if pool is not None else ("direct", "direct")
        if previous is not None and previous is not pool:
            previous.shutdown(timeout=0)

    def proxy_snapshot(self):
        with self._lock:
            pool = self._proxy_pool
            direct_rooms = [worker.rid for worker in self._rooms.values()
                            if worker.proxy_pool is None and self._active(worker)]
        if pool is None:
            counters = self._direct_control.snapshot()
            route = {"kind": "direct", "route_id": "direct", "endpoint": "direct",
                     "state": "cooldown" if counters["cooldown_seconds"] > 0 else "ready",
                     "assigned_tasks": len(direct_rooms), "connecting": counters["connecting"],
                     "request_inflight": counters["request_inflight"],
                     "cooldown_seconds": counters["cooldown_seconds"]}
            leases = [{"rid": rid, "proxy": "direct", "kind": "direct", "route_id": "direct", "status": "assigned"}
                      for rid in sorted(direct_rooms)]
            return {"enabled": False, "available": 0, "assigned": 0, "checking": 0,
                    "failed": 0, "total": 0, "last_error": "", "leases": leases, "routes": [route]}
        return {**pool.snapshot(), "enabled": True}

    def refresh_proxies(self):
        with self._lock:
            pool = self._proxy_pool
        if pool is not None:
            pool.refresh()
        return pool is not None

    def metrics(self):
        with self._lock:
            values = list(self._rooms.values())
            result = {"active": sum(self._active(worker) for worker in values),
                      "connected": sum(self._active(worker) and worker.state == "connected" for worker in values),
                      "connecting_pending": self._connecting_pending, "connecting_inflight": self._connecting_inflight,
                      "max_connecting": self._max_connecting, "connect_interval_seconds": self._connect_interval,
                      "retrying": sum(self._active(worker) and worker.state == "retrying" for worker in values),
                      "log_queue_depth": self._log_queue.qsize(), "log_queue_capacity": self._log_queue.maxsize}
        result["archive"] = self.archive.metrics()
        result["proxy"] = {key: value for key, value in self.proxy_snapshot().items() if key != "leases"}
        return result

    def _stop_fetcher(self, fetcher) -> None:
        if fetcher is not None:
            try:
                fetcher.stop()
            except Exception:
                pass

    def stop(self, rid: str) -> bool:
        rid = parse_room_id(rid)
        with self._lock:
            worker = self._rooms.get(rid)
        if worker is None:
            return False
        # Use the same lock order as _message: finish the accepted record and
        # its counter before publishing stopped. Never wait for disk I/O while
        # holding the manager lock, which would stall every other room.
        with worker.write_lock:
            with self._lock:
                if not self._active(worker):
                    return False
                worker.stop_event.set()
                worker.attempt = None
                worker.state, worker.detail = "stopped", "监控已停止"
                self._durations.pause(worker)
                self._capture_duration(worker)
                fetcher = worker.fetcher
                event = {"kind": "status", "rid": rid, "name": worker.name,
                         "state": "stopped", "detail": worker.detail, "anchor_id": worker.anchor_id,
                         "session_id": worker.session_id}
                self._log_event(event)
                self._notify(event)
        self._stop_fetcher(fetcher)
        return True

    def stop_all(self) -> None:
        with self._lock:
            rids = list(self._rooms)
        for rid in rids:
            self.stop(rid)

    def shutdown(self, timeout: float = 10) -> bool:
        """Prevent new work, stop workers and wait at most timeout for threads."""
        deadline = time.monotonic() + max(0, timeout)
        with self._lock:
            self._closed = True
        self.stop_all()
        with self._lock:
            threads = list(self._threads)
        for thread in threads:
            if thread is not threading.current_thread():
                thread.join(max(0, deadline - time.monotonic()))
        durations_closed = self._durations.close(timeout=max(0, deadline - time.monotonic()))
        archive_closed = self.archive.close(timeout=max(0, deadline - time.monotonic()))
        with self._lock:
            stop_logs = not self._logs_closed
            if stop_logs:
                self._logs_closed = True
        if stop_logs:
            self._log_queue.put(None)
        if self._log_thread is not threading.current_thread():
            self._log_thread.join(max(0, deadline - time.monotonic()))
        pool_closed = self._proxy_pool is None or self._proxy_pool.shutdown(timeout=max(0, deadline - time.monotonic()))
        return (durations_closed and archive_closed and pool_closed and not self._log_thread.is_alive()
                and not any(thread.is_alive() for thread in threads if thread is not threading.current_thread()))

    def snapshot(self) -> dict[str, dict]:
        with self._lock:
            for worker in self._rooms.values():
                self._capture_duration(worker)
            return {
                rid: {"rid": rid, "name": worker.name, "state": worker.state,
                      "detail": worker.detail, "message_count": worker.message_count,
                      "last_message_at": worker.last_message_at, "active": self._active(worker),
                      "title": worker.title, "anchor_id": worker.anchor_id, "session_id": worker.session_id,
                      "is_live": worker.is_live, "live_checked_at": worker.live_checked_at,
                      "broadcast_started_at": worker.broadcast_started_at,
                      "duration_started_at": worker.duration_started_at,
                      "duration_source": worker.duration_source,
                      "duration_seconds": worker.duration_seconds,
                      "duration_running": worker.duration_running,
                      "duration_updated_at": worker.duration_updated_at,
                      "viewer_count": worker.viewer_count,
                      "viewer_updated_at": worker.viewer_updated_at,
                      "proxy": worker.proxy, "proxy_status": worker.proxy_status,
                      "route_id": worker.route_id, "route_kind": worker.route_kind}
                for rid, worker in self._rooms.items()
            }

    def viewer_refresh_status(self):
        with self._lock:
            return dict(self._viewer_refresh)

    def refresh_viewers(self):
        with self._lock:
            if self._closed:
                raise ValueError("采集服务正在退出")
            if self._viewer_refresh['active'] or time.monotonic()-self._viewer_refresh_at<10:
                return {"accepted":False,**self._viewer_refresh}
            workers=[w for w in self._rooms.values() if self._active(w) and w.is_live is True and w.attempt is not None]
            self._viewer_refresh_at=time.monotonic()
            self._viewer_refresh={"id":str(time.monotonic_ns()),"active":bool(workers),"total":len(workers),"checked":0,"updated":0,"failed":0,"skipped":0}
            if workers:
                thread=threading.Thread(target=self._refresh_viewer_batch,args=(workers,),name='viewer-refresh',daemon=True)
                self._threads.add(thread);thread.start()
            return {"accepted":True,**self._viewer_refresh}

    def _refresh_viewer_batch(self,workers):
        try:
            with ThreadPoolExecutor(max_workers=4,thread_name_prefix='viewer-refresh-room') as executor:
                futures=[executor.submit(self._refresh_one_viewer,w) for w in workers]
                for future in as_completed(futures):
                    result=future.result()
                    with self._lock:
                        self._viewer_refresh['checked']+=1
                        self._viewer_refresh[result]+=1
        finally:
            with self._lock:
                self._viewer_refresh['active']=False
                self._threads.discard(threading.current_thread())

    def _refresh_one_viewer(self,worker):
        with self._lock:
            if not self._active(worker) or worker.is_live is not True or worker.attempt is None:return 'skipped'
            attempt,session,anchor,revision=worker.attempt,worker.session_id,worker.anchor_id,worker.viewer_revision
            lease,control=worker.proxy_lease,worker.traffic_control
            if worker.proxy_pool is not None and lease is None:return 'skipped'
        try:
            callback=self.room_info_fetcher
            if callback is None:
                from liveMan import fetch_room_info
                callback=fetch_room_info
            route={'proxy':lease} if worker.proxy_pool is not None else {}
            supported=inspect.signature(callback).parameters
            options={k:v for k,v in {'traffic_control':control,'stop_event':worker.stop_event}.items() if k in supported}
            info=callback(worker.rid,cookie=worker.cookie,**route,**options)
            # A refresh cannot change a live connection's real session, mark it
            # offline, or overwrite a newer WSS observation during this request.
            if not isinstance(info,dict) or info.get('is_live') is not True or str(info.get('room_id'))!=session or str(info.get('anchor_id'))!=anchor:return 'failed'
            if type(info.get('viewer_count')) is not int or not 0<=info['viewer_count']<=9_007_199_254_740_991:return 'failed'
            return 'updated' if self._apply_viewer_count(worker,info.get('viewer_count'),attempt,revision=revision) else 'skipped'
        except Exception as error:
            if worker.stop_event.is_set():return 'skipped'
            if getattr(error,'http_status',0) in (444,429):
                if control is not None:control.cooldown(60)
            return 'failed'

    def _apply_viewer_count(self,worker,count,attempt=None,revision=None):
        if type(count) is not int or not 0<=count<=9_007_199_254_740_991:return False
        with worker.write_lock:
            with self._lock:
                if not self._active(worker) or worker.is_live is not True or (attempt is not None and worker.attempt is not attempt) or (revision is not None and worker.viewer_revision!=revision):return False
                worker.viewer_count=count;worker.viewer_revision+=1
                worker.viewer_updated_at=datetime.now(SHANGHAI).isoformat(timespec='microseconds')
                params={'anchor_id':worker.anchor_id,'session_id':worker.session_id,'viewer_count':count,'observed_at':worker.viewer_updated_at}
            try:self.archive.quality('audience_record',params)
            except Exception:
                with self._lock:
                    if not self._audience_warning_sent:
                        self._audience_warning_sent=True
                        event={'kind':'log','rid':'','name':'','msg_type':'warning','detail':'场次在线采样保存失败；当前人数仍更新，缺少的采样不会补造，请检查磁盘空间或权限'}
                        self._log_event(event);self._notify(event)
            return True

    def _message(self, worker: _RoomWorker, msg_type, payload, attempt=None) -> None:
        # Gift delivery is not reliable for the supported visitor monitoring
        # mode. Keep archive compatibility, but never create new gift records.
        if msg_type == "gift":
            return
        if msg_type == "stats":
            if not isinstance(payload, dict):
                return
            count = payload.get("viewer_count")
            if type(count) is not int or not 0 <= count <= 9_007_199_254_740_991:
                return
            if str(payload.get("room_id") or worker.session_id)==worker.session_id:
                self._apply_viewer_count(worker,count,attempt)
            return  # Separate aggregate telemetry, never a chat/content event.
        if msg_type not in CONTENT_TYPES and msg_type != "control":
            return
        with worker.write_lock:
            with self._lock:
                if not self._active(worker) or (attempt is not None and worker.attempt is not attempt):
                    return
            now = datetime.now(SHANGHAI)
            payload = dict(payload) if isinstance(payload, dict) else {"text": str(payload)}
            received_at = now.isoformat(timespec="milliseconds")
            if msg_type == "control":
                try:
                    logged_session = _identity(str(payload.get("room_id") or ""))
                except ValueError:
                    logged_session = worker.session_id
                with self._lock:
                    event = {"kind": "log", "rid": worker.rid, "name": worker.name,
                             "anchor_id": worker.anchor_id, "session_id": logged_session,
                             "msg_type": "control", "text": str(payload.get("content") or payload.get("text", "")),
                             "detail": str(payload.get("content") or payload.get("text", "")), "timestamp": received_at}
                    if logged_session != worker.session_id:
                        event["registered_session_id"] = worker.session_id
                    self._log_event(event)
                    self._notify(event)
                return
            if str(payload.get("room_id") or worker.session_id) != worker.session_id:
                self._fail(worker, "收到的真实场次 ID 与当前连接不一致，已暂停此房间；请重新开始以核验场次")
                return
            message_time, message_epoch = server_time(payload.get("event_time"))
            def identifier(value):
                return "" if value in (None, "", 0, "0") else str(value)
            normalized = {"anchor_id": worker.anchor_id, "anchor_name": worker.name,
                          "web_rid": worker.rid, "session_id": worker.session_id,
                          "sender_id": identifier(payload.get("sender_id")),
                          "sender_name": str(payload.get("sender_name") or ""),
                          "message_type": str(msg_type), "content": str(payload.get("content") or ""),
                          "message_time": message_time, "received_at": received_at,
                          "time_source": "server" if message_epoch is not None else "missing",
                          "message_id": identifier(payload.get("message_id")), "payload": payload}
            event = {**normalized, "kind": "message", "rid": worker.rid, "name": worker.name,
                     "msg_type": str(msg_type), "time_str": message_time[11:19] if message_time else "",
                     "text": str(payload.get("text") or normalized["content"]), "timestamp": received_at}
            try:
                inserted = self.archive.record_message(normalized, title=worker.title, started_at=worker.started_at)
            except Exception:
                self._fail(worker, "消息数据库写入失败，已停止此房间；请检查数据目录权限及磁盘空间后重新开始")
                return
            if not inserted:
                return
            with self._lock:
                if self._active(worker) and (attempt is None or worker.attempt is attempt):
                    worker.message_count += 1
                    worker.last_message_at = event["timestamp"]
                    self._notify(event)

    def _run(self, worker: _RoomWorker) -> None:
        failures = 0
        room_unavailable_count = 0
        try:
            if worker.proxy_pool is not None and worker.predecessors:
                self._status(worker, "waiting_proxy", "等待上一连接退出并释放出口名额")
                for predecessor in worker.predecessors:
                    while predecessor.is_alive() and not worker.stop_event.is_set():
                        predecessor.join(0.1)
            factory = self.fetcher_factory
            info_fetcher = self.room_info_fetcher
            if factory is None or info_fetcher is None:
                from liveMan import DouyinLiveWebFetcher, fetch_room_info
                factory = factory or DouyinLiveWebFetcher
                info_fetcher = info_fetcher or fetch_room_info
            # Optional engine controls remain compatible with existing test or
            # integration factories that implement the older callback contract.
            def optional_kwargs(callback, values):
                try:
                    parameters = inspect.signature(callback).parameters
                except (TypeError, ValueError):
                    return {}
                return {name: value for name, value in values.items() if name in parameters}
            while not worker.stop_event.is_set():
                fetcher = None
                release_slot = None
                attempt = object()
                connected_at = [None]
                suspect_offline = False
                room_refused = False
                failure_kind, failure_status = None, 0
                try:
                    if (worker.proxy_pool is not None and worker.proxy_lease is not None
                            and getattr(worker.proxy_pool, "shared", False)):
                        revalidate = getattr(worker.proxy_pool, "revalidate", None)
                        # A subscription can retire a route while its sockets
                        # remain healthy. Switch only before the next attempt,
                        # including offline polling and normal socket closes.
                        if callable(revalidate) and not revalidate(worker.rid, worker.proxy_lease):
                            worker.proxy_lease = None
                            with self._lock:
                                worker.proxy, worker.proxy_status = "", "waiting"
                                worker.route_id, worker.route_kind = "", ""
                                worker.traffic_control = None
                    if worker.proxy_pool is not None and worker.proxy_lease is None:
                        detail = ("等待可用出口；仅按已启用的出口策略分配" if getattr(worker.proxy_pool, "shared", False)
                                  else "等待可用的独立代理；不会自动改用直连")
                        self._status(worker, "waiting_proxy", detail)
                        lease = worker.proxy_pool.acquire(worker.rid, worker.stop_event)
                        worker.proxy_lease = lease
                        if lease is None:
                            break
                        with self._lock:
                            worker.proxy = lease.endpoint
                            worker.proxy_status = "direct" if getattr(lease, "kind", "proxy") == "direct" else "assigned"
                            worker.route_id = getattr(lease, "route_id", lease.endpoint)
                            worker.route_kind = getattr(lease, "kind", "proxy")
                            worker.traffic_control = getattr(lease, "control", None)
                    if worker.stop_event.is_set():
                        break
                    route = {"proxy": worker.proxy_lease} if worker.proxy_pool is not None else {}
                    if room_unavailable_count and worker.is_live is True:
                        self._status(worker, "confirming_offline" if room_unavailable_count <= 2 else "retrying",
                                     "疑似下播，正在确认" if room_unavailable_count <= 2 else "直播状态仍无法确认，正在重试")
                    else:
                        self._status(worker, "connecting", "等待连接名额并查询开播状态")
                    release_slot = self._connection_slot(worker)
                    if release_slot is None:
                        break
                    info = info_fetcher(worker.rid, cookie=worker.cookie, **route,
                                        **optional_kwargs(info_fetcher, {"traffic_control": worker.traffic_control,
                                                                         "stop_event": worker.stop_event}))
                    if not isinstance(info, dict) or "is_live" not in info:
                        raise RuntimeError("开播状态响应无效")
                    room_unavailable_count = 0
                    with self._lock:
                        if not self._active(worker):
                            break
                        worker.title = str(info.get("title", ""))
                        worker.is_live = bool(info["is_live"])
                        if not worker.is_live:
                            worker.observed_offline = True
                        start_iso, start_epoch = server_time(info.get("started_at")) if worker.is_live else ("", None)
                        worker.started_at = start_iso
                        worker.broadcast_started_at = int(start_epoch) if start_epoch is not None else None
                        count = info.get("viewer_count")
                        worker.viewer_count = count if worker.is_live and type(count) is int and 0 <= count <= 9007199254740991 else None
                        worker.viewer_updated_at=""
                        worker.viewer_revision+=1
                        worker.live_checked_at = datetime.now(SHANGHAI).isoformat(timespec="milliseconds")
                        nickname = info.get("nickname")
                        if (isinstance(nickname, str) and nickname.strip()
                                and worker.name in {worker.rid, f"主播 {worker.rid}", f"主播{worker.rid}"}):
                            worker.name = nickname.strip()
                    if not info["is_live"]:
                        release_slot()
                        failures = 0
                        self._status(worker, "offline", f"尚未开播，{self.offline_seconds:g} 秒后再检查")
                        worker.stop_event.wait(self.offline_seconds)
                        continue

                    try:
                        anchor_id = _identity(info.get("anchor_id"))
                        session_id = _identity(info.get("room_id"))
                    except ValueError:
                        self._fail(worker, "无法确认真实主播 ID 或直播场次 ID，已暂停；请稍后重新开始，不会创建推测的场次")
                        break

                    with worker.write_lock:
                        with self._lock:
                            if not self._active(worker):
                                break
                            first_observation = not worker.duration_session_id and not worker.observed_offline
                        try:
                            self.archive.register_session(anchor_id, session_id, worker.name, worker.rid,
                                                          worker.title, worker.started_at or None)
                            self._durations.bind(worker, anchor_id, session_id)
                        except Exception:
                            self._fail(worker, "累计监控时长读取失败，已暂停；请检查数据目录权限或磁盘空间后重新开始")
                            break
                        with self._lock:
                            worker.attempt = attempt
                            worker.anchor_id, worker.session_id = anchor_id, session_id
                            worker.duration_initial_from_task = first_observation
                            worker.duration_session_id = session_id
                            worker.duration_source = "monitored"
                            self._capture_duration(worker)

                    self._apply_viewer_count(worker,worker.viewer_count,attempt)
                    def on_status(state, detail="", _attempt=attempt, _connected_at=connected_at, _release=release_slot):
                        if isinstance(state, dict):
                            detail, state = state.get("detail", ""), state.get("state", "connecting")
                        if state == "connected":
                            _connected_at[0] = time.monotonic()
                            _release()
                            with worker.write_lock:
                                with self._lock:
                                    if not self._active(worker) or worker.attempt is not _attempt:
                                        return
                                    initial_seconds = (max(0, time.time() - worker.task_started_at)
                                                       if worker.duration_initial_from_task else 0)
                                    self._durations.resume(worker, initial_seconds)
                                    self._capture_duration(worker)
                                    worker.duration_started_at = time.time() - worker.duration_seconds
                        if state == "error":
                            self._pause_duration(worker, _attempt)
                            # Transport failures are retried. Reserve the error
                            # state for an inactive room requiring intervention.
                            state = "retrying"
                            detail = f"{detail or '连接异常'}，正在准备重新连接"
                        if state in {"connecting", "connected", "offline", "retrying", "error"}:
                            self._status(worker, state, detail, _attempt)

                    fetcher = factory(worker.rid, on_message=lambda kind, data, _attempt=attempt: self._message(worker, kind, data, _attempt),
                                      cookie=worker.cookie, on_status=on_status, **route,
                                      **optional_kwargs(factory, {"traffic_control": worker.traffic_control}))
                    with self._lock:
                        if not self._active(worker):
                            break
                        worker.fetcher = fetcher
                    self._status(worker, "connecting", "正在连接直播间消息服务")
                    fetcher.start()
                    reason = "连接已断开"
                except Exception as exc:
                    if worker.stop_event.is_set() or getattr(exc, "kind", None) == "cancelled":
                        break
                    failure_kind = getattr(exc, "kind", "transport")
                    if failure_kind not in {"target_http", "proxy_auth", "transport"}:
                        failure_kind = "transport"
                    failure_status = getattr(exc, "http_status", 0)
                    if type(failure_status) is not int:
                        failure_status = 0
                    # Engine errors may contain request data; show only a safe
                    # type here, while LiveMonitorError has curated user text.
                    if type(exc).__name__ == "LiveRoomUnavailableError":
                        room_refused = True
                        reason = "直播间信息暂不可用，尚不能确认是否下播"
                        room_unavailable_count += 1
                        suspect_offline = worker.is_live is True and room_unavailable_count <= 2
                    elif type(exc).__name__ == "LiveMonitorError":
                        room_unavailable_count = 0
                        reason = str(exc)
                    else:
                        room_unavailable_count = 0
                        reason = f"连接失败（{type(exc).__name__}）"
                finally:
                    self._pause_duration(worker)
                    if release_slot is not None:
                        release_slot()
                    with worker.write_lock:
                        with self._lock:
                            if worker.attempt is attempt:
                                worker.attempt = None
                            if worker.fetcher is fetcher:
                                worker.fetcher = None
                    self._stop_fetcher(fetcher)
                if worker.stop_event.is_set():
                    break
                if suspect_offline:
                    # A room-level refusal after a confirmed live session is a
                    # hint, never evidence of an ended broadcast or bad proxy.
                    # Keep its identity, timer and lease through two rechecks.
                    delay = min(5, self.retry_seconds)
                    self._status(worker, "confirming_offline", f"疑似下播，正在确认，{delay:g} 秒后复查")
                    worker.stop_event.wait(delay)
                    continue
                if not room_refused and worker.proxy_pool is not None and worker.proxy_lease is not None:
                    keep_lease = True
                    if getattr(worker.proxy_pool, "shared", False):
                        # A normal room/socket close is not evidence that its
                        # shared route failed. Other rooms keep their sockets.
                        if failure_kind is not None:
                            keep_lease = worker.proxy_pool.report_failure(worker.rid, worker.proxy_lease,
                                                                         failure_kind, status_code=failure_status)
                    else:
                        worker.proxy_pool.invalidate(worker.rid, lease=worker.proxy_lease)
                        keep_lease = False
                    if not keep_lease:
                        worker.proxy_lease = None
                        with self._lock:
                            worker.proxy, worker.proxy_status = "", "waiting"
                            worker.route_id, worker.route_kind = "", ""
                elif (worker.proxy_pool is None and failure_kind == "target_http"
                      and failure_status in {444, 429} and worker.traffic_control is not None):
                    worker.traffic_control.cooldown(60)
                # A handshake immediately followed by a disconnect is still a
                # failed attempt. Reset backoff only after a stable connection.
                if connected_at[0] is not None and time.monotonic() - connected_at[0] >= 60:
                    failures = 0
                failures = min(failures + 1, 4)
                delay = self.retry_seconds * (2 ** max(0, failures - 1))
                self._status(worker, "retrying", f"{reason}，{delay:g} 秒后重试")
                worker.stop_event.wait(delay)
        except Exception as exc:
            self._fail(worker, f"监控无法启动（{type(exc).__name__}），请重新安装应用或检查依赖")
        finally:
            self._pause_duration(worker)
            self._durations.unbind(worker)
            if worker.proxy_pool is not None and worker.proxy_lease is not None:
                worker.proxy_pool.release(worker.rid, lease=worker.proxy_lease)
                worker.proxy_lease = None
            with self._lock:
                worker.proxy = ""
                worker.proxy_status = "released" if worker.proxy_pool is not None else "direct"
                worker.route_id, worker.route_kind = ("", "") if worker.proxy_pool is not None else ("direct", "direct")
                self._threads.discard(threading.current_thread())
            worker.cookie = ""
