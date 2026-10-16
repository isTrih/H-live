"""Headless, local JSON-lines service for the desktop shell (no Tk imports).

stdout is reserved for RPC responses. Live events are polled from a bounded ring;
network callbacks never write to the IPC pipe or save settings synchronously.
"""
from collections import deque
import copy
import json
import os
from pathlib import Path
import queue
import re
import sys
import threading
import time
import uuid

from app_storage import ConfigError, app_data_dir, load_config, parse_room_id, save_config
from monitor import MonitorManager
from qingkan_native import CancelToken, __version__ as NATIVE_VERSION
from proxy_import import MAX_IMPORTED_PROXIES, ProxyImportError, parse_proxy_text, public_record
from subscription_store import ProxySubscriptions, SubscriptionError, merged_candidates

VERSION = "0.2.4"
MAX_ANCHORS = 500
EVENT_CAPACITY = 2000
MAX_LINE_BYTES = 1024 * 1024


class ServiceError(ValueError):
    """A deliberately safe, user-readable error."""


def _safe_error(error):
    if isinstance(error, (ServiceError, ConfigError, SubscriptionError)):
        return str(error)
    return f"操作失败（{type(error).__name__}），请检查数据目录、参数或运行日志"


def _room_list(value):
    if not isinstance(value, list) or len(value) > MAX_ANCHORS:
        raise ServiceError("rids 必须是最多 500 个直播间 ID 的列表")
    result = []
    for item in value:
        try:
            rid = parse_room_id(item)
        except ValueError:
            raise ServiceError("包含无法识别的直播间 ID") from None
        if rid not in result:
            result.append(rid)
    return result


def _parse_values(value):
    if not isinstance(value, str) or len(value) > 200_000:
        raise ServiceError("请提供不超过 200,000 字符的直播间链接或 ID")
    value = re.sub(r"\[[^\]]*\]\((https://[^\s)]+)\)", r"\1", value)
    value = re.sub(r"(https?://)", r"\n\1", value.strip())
    accepted, invalid = [], []
    for token in re.split(r"[\s,，;；]+", value):
        token = token.strip("[]()<>")
        if not token:
            continue
        try:
            rid = parse_room_id(token)
        except ValueError:
            # Do not echo arbitrary pasted values: they may be credentials.
            invalid.append("无法识别的直播间链接或 ID")
            continue
        if rid not in accepted:
            accepted.append(rid)
    if len(accepted) > MAX_ANCHORS:
        raise ServiceError("最多可配置 500 个监控任务")
    return accepted, invalid


def _rss_mb():
    if os.name != "nt":
        return None
    try:
        import ctypes
        from ctypes import wintypes
        class Counters(ctypes.Structure):
            _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD),
                        ("PeakWorkingSetSize", ctypes.c_size_t), ("WorkingSetSize", ctypes.c_size_t),
                        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t), ("QuotaPagedPoolUsage", ctypes.c_size_t),
                        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t), ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
                        ("PagefileUsage", ctypes.c_size_t), ("PeakPagefileUsage", ctypes.c_size_t)]
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.GetCurrentProcess.restype = wintypes.HANDLE
        psapi = ctypes.WinDLL("psapi", use_last_error=True)
        psapi.GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(Counters), wintypes.DWORD]
        psapi.GetProcessMemoryInfo.restype = wintypes.BOOL
        counters = Counters()
        counters.cb = ctypes.sizeof(counters)
        if psapi.GetProcessMemoryInfo(kernel.GetCurrentProcess(), ctypes.byref(counters), counters.cb):
            return round(counters.WorkingSetSize / 1024 / 1024, 1)
    except Exception:
        pass
    return None


class BackendService:
    def __init__(self, data_dir=None, manager_factory=MonitorManager, export_function=None, proxy_pool_factory=None,
                 subscription_fetcher=None):
        if NATIVE_VERSION != VERSION:
            raise RuntimeError("原生处理模块版本不一致，请重新安装应用")
        self.data_dir = Path(data_dir) if data_dir is not None else app_data_dir()
        self._lock = threading.RLock()
        self._persist_lock = threading.Lock()
        self._route_sync_lock = threading.RLock()
        self._archive_action_lock = threading.RLock()
        self._archive_delete_in_progress = False
        self._delete_jobs = {}
        self._shutdown_lock = threading.Lock()
        self._events = deque(maxlen=EVENT_CAPACITY)
        self._seq = 0
        self._rates = deque(maxlen=20)
        self._pending_metadata = {}
        self._closed = threading.Event()
        self._shutdown_result = None
        self._jobs = {}
        self._operations = queue.Queue(maxsize=64)
        self._export_function = export_function
        from proxy_pool import ProxyFailureRegistry
        self._proxy_failure_registry = ProxyFailureRegistry()
        self._imported_proxy_failure_registry = ProxyFailureRegistry()
        self._proxy_pool_factory = proxy_pool_factory or self._build_proxy_pool
        self.config = load_config(data_dir=self.data_dir)
        metadata = self.config.get("anchor_metadata", {})
        self._metadata_seen = dict(metadata) if isinstance(metadata, dict) else {}
        if not self.config.get("first_run_complete"):
            self.config["first_run_complete"] = True
            save_config(self.config, self.data_dir)
        if len(self.config.get("anchors", {})) > MAX_ANCHORS:
            raise ServiceError("已有配置超过 500 个任务，请先缩减主播列表")
        enabled = self.config.get("proxy_pool", {}).get("enabled", True)
        self._proxy_pool = self._proxy_pool_factory() if enabled else None
        # A manually warmed pool is not a route until the user enables it.
        self._prepared_proxy_pool = None
        self.manager = manager_factory(on_event=self._receive_event, data_dir=self.data_dir, proxy_pool=self._proxy_pool)
        from quality_worker import QualityWorker
        self._quality = QualityWorker(self.manager.archive)
        self.subscriptions = ProxySubscriptions(self._subscription_config, self._update_config,
                                               self._sync_subscription_routes, subscription_fetcher)
        self._operator = threading.Thread(target=self._operation_loop, name="desktop-operations", daemon=True)
        self._housekeeper = threading.Thread(target=self._metadata_loop, name="desktop-metadata", daemon=True)
        self._resume_intent = None
        self._resume_queued = False
        self._operator.start()
        self._housekeeper.start()
        self._queue_update_resume()

    def _queue_update_resume(self):
        from update_resume import read_intent, resume_rids, acknowledge
        try:
            intent = read_intent(self.data_dir)
            rids = resume_rids(intent, VERSION, self.config.get("anchors", {}))
            if intent and VERSION in (intent["from_version"], intent["target_version"]):
                if rids:
                    with self._lock:
                        if self._resume_queued:
                            return
                        self._resume_intent = intent
                        self._resume_queued = True
                    try:
                        self._enqueue("resume", rids)
                    except Exception:
                        with self._lock:
                            self._resume_queued = False
                        raise
                else:
                    acknowledge(self.data_dir, intent["nonce"])
        except Exception:
            self._receive_event({"kind":"log", "msg_type":"warning", "detail":"更新恢复记录读取失败，已保留文件；请手动恢复需要的监控任务"})

    def _prepare_update(self, params):
        from update_resume import save_intent, valid_version
        target = params.get("target_version")
        if not valid_version(target):
            raise ServiceError("更新版本无效，请重新检查更新")
        with self._operations.mutex:
            if self._operations.unfinished_tasks:
                return {"ready":False,"reason":"pending","detail":"监控操作仍在执行，请稍后重试"}
        with self._lock:
            if self._archive_delete_in_progress or any(j.get("status") in ("queued","running") or (j.get("thread") is not None and j["thread"].is_alive()) for j in self._jobs.values()):
                return {"ready":False,"reason":"exporting","detail":"导出或记录清理仍在执行，请完成后更新"}
            configured = set(self.config.get("anchors", {}))
        snapshot = self.manager.snapshot()
        rids = [rid for rid,row in snapshot.items() if rid in configured and row.get("active")]
        try:
            save_intent(self.data_dir, VERSION, target, rids)
        except Exception:
            raise ServiceError("无法保存监控恢复列表，监控尚未暂停，请检查数据目录后重试") from None
        try:
            deadline = time.monotonic() + 15
            for rid in rids:
                self.manager.stop(rid)
            while not self.manager.can_configure_proxy() and time.monotonic() < deadline:
                time.sleep(.05)
            self._flush_metadata()
            result = self._update_readiness()
            while not result.get("ready") and result.get("reason") == "pending" and time.monotonic() < deadline:
                time.sleep(.05)
                self._flush_metadata()
                result = self._update_readiness()
            if result.get("ready"):
                return {**result,"paused_tasks":len(rids),"resume_after_update":True}
        except Exception:
            result = {"ready":False,"reason":"pending","detail":"暂停或落盘尚未完成，已取消本次更新"}
        self._queue_update_resume()
        return result

    def _build_proxy_pool(self, source=None, records=None, include_direct=None):
        from proxy_pool import ProxyPool
        with self._lock:
            source = source or self.config.get("proxy_pool", {}).get("source", "public")
            config = copy.deepcopy(self.config)
            if records is not None:
                config["imported_proxies"] = records
            records = merged_candidates(config)
            if include_direct is None:
                include_direct = config.get("proxy_pool", {}).get("include_direct", True)
        if source == "imported":
            return ProxyPool(imported_candidates=records, failure_registry=self._imported_proxy_failure_registry,
                             shared=True, include_direct=include_direct)
        return ProxyPool(failure_registry=self._proxy_failure_registry, shared=True, include_direct=include_direct)

    def _subscription_config(self):
        with self._lock:
            return copy.deepcopy({"imported_proxies": self.config.get("imported_proxies", []),
                                  "proxy_subscriptions": self.config.get("proxy_subscriptions", [])})

    def _sync_subscription_routes(self):
        with self._route_sync_lock:
            records = merged_candidates(self._subscription_config())
            with self._lock:
                pools = [pool for pool in (self._proxy_pool, self._prepared_proxy_pool)
                         if pool is not None and getattr(pool, "source", None) == "imported"]
            for pool in pools:
                pool.update_imported_candidates(records)

    def _require_proxy_idle(self):
        can_configure = getattr(self.manager, "can_configure_proxy", None)
        if self._operations.unfinished_tasks or not callable(can_configure) or not can_configure():
            raise ServiceError("请先停止全部监控，并等待所有连接和操作退出后再修改当前代理来源")

    def _list_imported_proxies(self):
        with self._lock:
            records = list(self.config.get("imported_proxies", []))
            source = self.config.get("proxy_pool", {}).get("source", "public")
            pool = self._proxy_pool or self._prepared_proxy_pool
        rows = []
        for record in records:
            row = public_record(record)
            if source == "imported" and pool is not None:
                status = getattr(pool, "candidate_status", None)
                if callable(status):
                    row["status"] = status(record["endpoint"])
            elif self._imported_proxy_failure_registry.contains(record["endpoint"]):
                row["status"] = "failed"
            rows.append(row)
        return {"items": rows, "total": len(rows), "source": source}

    def _replace_imported_proxies(self, records, reset_failures=()):
        with self._route_sync_lock:
            self._replace_imported_proxies_locked(records, reset_failures)

    def _replace_imported_proxies_locked(self, records, reset_failures):
        with self._lock:
            selected = self.config.get("proxy_pool", {}).get("source", "public") == "imported"
            enabled = self._proxy_pool is not None
        if selected and enabled:
            self._require_proxy_idle()
        new_pool = self._build_proxy_pool("imported", records) if selected and enabled else None
        try:
            self._update_config(lambda config: config.update(imported_proxies=records))
        except Exception:
            if new_pool is not None:
                new_pool.shutdown(timeout=0)
            raise
        if selected:
            if enabled:
                self.manager.configure_proxy(new_pool)
            with self._lock:
                prepared, self._prepared_proxy_pool = self._prepared_proxy_pool, None
                self._proxy_pool = new_pool
            if prepared is not None:
                prepared.shutdown(timeout=0)
        # Persistence must succeed and old pools must be closed before clearing:
        # a finishing check using old credentials must not restore the failure.
        for endpoint in reset_failures:
            self._imported_proxy_failure_registry.discard(endpoint)
        self._sync_subscription_routes()

    def _receive_event(self, event):
        if self._closed.is_set():
            return
        safe = dict(event)
        # Defensive: credentials never belong in IPC events, even for an
        # alternate manager implementation. The actual manager emits none.
        for key in ("cookie", "anchor_cookies", "cookie_encrypted", "anchor_cookies_encrypted",
                    "imported_proxies", "imported_proxies_encrypted", "proxy_subscriptions", "proxy_subscriptions_encrypted",
                    "proxy_url", "username", "password"):
            safe.pop(key, None)
        with self._lock:
            self._seq += 1
            safe["seq"] = self._seq
            self._events.append(safe)
            bucket = int(time.monotonic() * 10)
            if self._rates and self._rates[-1][0] == bucket:
                self._rates[-1] = (bucket, self._rates[-1][1] + 1)
            else:
                self._rates.append((bucket, 1))
            rid, anchor_id = safe.get("rid"), safe.get("anchor_id")
            if rid and isinstance(anchor_id, str) and re.fullmatch(r"[0-9]{1,64}", anchor_id) and int(anchor_id):
                metadata = {"anchor_id": anchor_id, "name": str(safe.get("name") or rid),
                            "session_id": str(safe.get("session_id") or "")}
                if self._metadata_seen.get(rid) != metadata:
                    self._metadata_seen[rid] = metadata
                    self._pending_metadata[rid] = metadata

    def _update_config(self, update):
        with self._persist_lock:
            with self._lock:
                candidate = copy.deepcopy(self.config)
            update(candidate)
            save_config(candidate, self.data_dir)
            with self._lock:
                self.config = candidate

    def _flush_metadata(self):
        with self._lock:
            pending, self._pending_metadata = self._pending_metadata, {}
        if not pending:
            return
        def update(config):
            metadata = config.setdefault("anchor_metadata", {})
            if not isinstance(metadata, dict):
                metadata = config["anchor_metadata"] = {}
            for rid, info in pending.items():
                if rid in config.get("anchors", {}):
                    metadata[rid] = info
                    if config["anchors"][rid] in (rid, f"主播 {rid}", f"主播{rid}"):
                        config["anchors"][rid] = info["name"]
        try:
            self._update_config(update)
        except Exception:
            with self._lock:
                for rid, info in pending.items():
                    self._pending_metadata.setdefault(rid, info)
            raise

    def _metadata_loop(self):
        while not self._closed.wait(1):
            try:
                self._flush_metadata()
            except Exception as error:
                self._receive_event({"kind": "log", "msg_type": "warning", "detail": _safe_error(error)})

    def _operation_loop(self):
        while not self._closed.is_set():
            try:
                operation, rids = self._operations.get(timeout=0.2)
            except queue.Empty:
                continue
            try:
                for rid in rids:
                    if self._closed.is_set():
                        break
                    if operation in ("start", "resume"):
                        with self._lock:
                            name = self.config.get("anchors", {}).get(rid)
                            cookie = self.config.get("anchor_cookies", {}).get(rid) or self.config.get("cookie", "")
                        if name is not None:
                            self.manager.start(rid, name, cookie=cookie)
                    else:
                        self.manager.stop(rid)
                if operation == "resume" and not self._closed.is_set() and self._resume_intent is not None:
                    from update_resume import acknowledge
                    acknowledge(self.data_dir, self._resume_intent["nonce"])
                    self._resume_intent = None
            except Exception as error:
                self._receive_event({"kind": "log", "msg_type": "error", "detail": _safe_error(error)})
            finally:
                if operation == "resume":
                    with self._lock:
                        self._resume_queued = False
                self._operations.task_done()

    def _enqueue(self, operation, rids):
        if self._closed.is_set():
            raise ServiceError("服务正在关闭")
        try:
            self._operations.put_nowait((operation, rids))
        except queue.Full:
            raise ServiceError("操作队列繁忙，请稍后重试") from None

    def _anchors(self, snapshot=None):
        snapshot = self.manager.snapshot() if snapshot is None else snapshot
        with self._lock:
            configured = dict(self.config.get("anchors", {}))
            cookies = set(self.config.get("anchor_cookies", {}))
            global_cookie = bool(self.config.get("cookie"))
            metadata = self.config.get("anchor_metadata", {})
            metadata = dict(metadata) if isinstance(metadata, dict) else {}
            pc_unsupported = set(self.config.get("pc_unsupported_rids", []))
        result = []
        for rid, name in configured.items():
            row = snapshot.get(rid, {})
            saved = metadata.get(rid, {})
            saved = saved if isinstance(saved, dict) else {}
            result.append({"rid": rid, "name": name, "anchor_id": row.get("anchor_id") or saved.get("anchor_id", ""),
                           "session_id": row.get("session_id") or saved.get("session_id", ""),
                           "state": row.get("state", "stopped"), "detail": row.get("detail", "尚未开始"),
                           "pc_unsupported": rid in pc_unsupported,
                           "pc_support_source": "manual" if rid in pc_unsupported else "",
                           "is_live": row.get("is_live"), "live_checked_at": row.get("live_checked_at", ""),
                           "last_message_at": row.get("last_message_at", ""),
                           "broadcast_started_at": row.get("broadcast_started_at"),
                           "duration_started_at": row.get("duration_started_at"),
                           "duration_source": row.get("duration_source", ""),
                           "duration_seconds": row.get("duration_seconds"),
                           "duration_running": row.get("duration_running", False),
                           "duration_updated_at": row.get("duration_updated_at"),
                           "viewer_count": row.get("viewer_count"),
                           "viewer_updated_at": row.get("viewer_updated_at", ""),
                           "proxy": row.get("proxy", ""),
                           "route_id": row.get("route_id", ""), "route_kind": row.get("route_kind", "direct"),
                           "proxy_status": row.get("proxy_status", "waiting" if self._proxy_pool is not None else "direct"),
                           "message_count": row.get("message_count", 0), "active": bool(row.get("active")),
                           "cookie_configured": rid in cookies or global_cookie,
                           "own_cookie_configured": rid in cookies})
        return result

    def _settings(self):
        with self._lock:
            return {"cookie_configured": bool(self.config.get("cookie")), "max_tasks": MAX_ANCHORS,
                    "data_dir": str(self.data_dir.resolve()),
                    "proxy_source": self.config.get("proxy_pool", {}).get("source", "public"),
                    "imported_proxy_count": len(self.config.get("imported_proxies", [])),
                    "proxy_include_direct": self.config.get("proxy_pool", {}).get("include_direct", True),
                    "proxy_enabled": self._proxy_pool is not None,
                    "proxy": {"mode": "pool" if self._proxy_pool is not None else "direct",
                              "configured": True, "supported": True}}

    def _proxy_snapshot(self):
        with self._lock:
            pool = self._proxy_pool or self._prepared_proxy_pool
            enabled = self._proxy_pool is not None
            source = self.config.get("proxy_pool", {}).get("source", "public")
        if pool is None:
            result = {"available": 0, "assigned": 0, "checking": 0, "failed": 0,
                      "total": 0, "last_error": "", "leases": [], "routes": []}
        else:
            result = pool.snapshot()
        if not enabled:
            # Candidate probes do not own the direct monitoring connections.
            # Keep their counters, but display load from the running manager.
            snapshot = getattr(self.manager, "proxy_snapshot", None)
            direct = snapshot() if callable(snapshot) else {}
            result["routes"] = direct.get("routes", []) + [
                route for route in result.get("routes", []) if route.get("kind") != "direct"]
            result["leases"] = direct.get("leases", [])
        return {**result, "enabled": enabled, "source": source, "subscriptions": self.subscriptions.snapshot()}

    def _metrics(self, snapshot):
        now = time.monotonic()
        with self._lock:
            rate = sum(count for bucket, count in self._rates if bucket > int(now * 10) - 10)
        values = {"total_messages": sum(item.get("message_count", 0) for item in snapshot.values()),
                  "active": sum(bool(item.get("active")) for item in snapshot.values()),
                  "connected": sum(item.get("state") == "connected" for item in snapshot.values()),
                  "events_per_second": rate, "process_rss_mb": _rss_mb(),
                  "thread_count": threading.active_count(), "operation_queue": self._operations.qsize()}
        metric_function = getattr(self.manager, "metrics", None)
        if callable(metric_function):
            try:
                values.update(metric_function())
            except Exception:
                pass
        archive = values.get("archive")
        if isinstance(archive, dict):
            values["archive_queue"] = archive.get("queue_depth", 0)
        values["proxy"] = {key: value for key, value in self._proxy_snapshot().items() if key != "leases"}
        refresh_status=getattr(self.manager,"viewer_refresh_status",None)
        values["viewer_refresh"]=refresh_status() if callable(refresh_status) else {}
        return values

    def _export_jobs(self):
        with self._lock:
            return [{key: copy.deepcopy(value) for key, value in job.items() if key not in ("thread", "cancel")}
                    for job in self._jobs.values()]

    def _update_readiness(self):
        """Read-only snapshot; the shell must gate RPC and recheck before shutdown."""
        def pending(detail="后台仍有待完成的操作或写入，请稍后重试"):
            return {"ready": False, "reason": "pending", "detail": detail}

        # unfinished_tasks includes the item already removed by the operator,
        # until its task_done(). Queue emptiness alone would miss a blocked start.
        with self._operations.mutex:
            if self._operations.unfinished_tasks:
                return pending("监控操作仍在排队或执行，请等待完成后再更新")
        try:
            can_configure = getattr(self.manager, "can_configure_proxy", None)
            if not callable(can_configure):
                return pending("暂时无法确认后台状态，请稍后重试")
            if not can_configure():
                return {"ready": False, "reason": "monitoring",
                        "detail": "请先停止全部监控，并等待连接退出后再更新"}
            with self._lock:
                if self._archive_delete_in_progress or any(job.get("thread") is not None and job["thread"].is_alive() for job in self._delete_jobs.values()):
                    return pending("消息删除仍在执行，请等待完成后再更新")
                for job in self._jobs.values():
                    thread = job.get("thread")
                    if job.get("status") in ("running", "queued") or (thread is not None and thread.is_alive()):
                        return {"ready": False, "reason": "exporting",
                                "detail": "导出仍在运行或收尾，请等待完成后再更新"}
                if self._pending_metadata:
                    return pending()
            if self._persist_lock.locked():
                return pending()
            if self.subscriptions.busy:
                return pending("代理订阅正在刷新，请稍后再更新")
            archive_metrics = self.manager.archive.metrics()
            if not isinstance(archive_metrics, dict):
                return pending("暂时无法确认归档状态，请稍后重试")
            for key in ("queue_depth", "pending_requests"):
                value = archive_metrics.get(key)
                if isinstance(value, bool) or not isinstance(value, int) or value != 0:
                    return pending("归档仍在写入或状态待确认，请稍后重试")
        except Exception:
            # No raw manager, filesystem or transport exceptions cross IPC.
            return pending("暂时无法确认后台状态，请稍后重试")
        return {"ready": True, "reason": "ready", "detail": "监控、导出和待写入操作均已结束，可以更新"}

    def dispatch(self, method, params=None):
        # Pool replacement and subscription reconciliation share one ordering.
        # Reading a snapshot before taking this lock could revive removed routes.
        if isinstance(method, str) and method in ("configure_proxy", "import_proxies", "remove_imported_proxies"):
            with self._route_sync_lock:
                return self._dispatch(method, params)
        if isinstance(method, str) and method in ("start", "stop", "remove", "export", "execute_archive_delete", "quality_rebuild", "quality_pause", "quality_configure", "quality_commands", "quality_export", "prepare_update", "cancel_update"):
            with self._archive_action_lock:
                if method in ("start", "cancel_update", "export", "execute_archive_delete", "quality_rebuild", "quality_pause", "quality_configure", "quality_commands", "quality_export") and self._archive_delete_in_progress:
                    raise ServiceError("消息删除仍在执行，请等待完成后再开始监控、导出或再次删除")
                return self._dispatch(method, params)
        return self._dispatch(method, params)

    def _dispatch(self, method, params=None):
        if not isinstance(method, str) or not isinstance(params if params is not None else {}, dict):
            raise ServiceError("请求须包含 method 字符串及 params 对象")
        params = params or {}
        if self._closed.is_set() and method != "shutdown":
            raise ServiceError("服务已关闭")
        if method == "update_readiness":
            return self._update_readiness()
        if method == "prepare_update":
            return self._prepare_update(params)
        if method == "cancel_update":
            self._queue_update_resume()
            return {"recovery_queued":True}
        if method == "init":
            snapshot = self.manager.snapshot()
            with self._lock:
                cursor = self._seq
            return {"version": VERSION, "native_version": NATIVE_VERSION, "processing": "rust", "anchors": self._anchors(snapshot),
                    "archive_anchors": self.manager.archive.list_anchors(), "settings": self._settings(),
                    "metrics": self._metrics(snapshot), "cursor": cursor}
        if method == "poll":
            after, limit = params.get("after", 0), params.get("limit", 300)
            if isinstance(after, bool) or not isinstance(after, int) or after < 0:
                raise ServiceError("after 须为非负整数游标")
            if isinstance(limit, bool) or not isinstance(limit, int) or not 1 <= limit <= 300:
                raise ServiceError("limit 须为 1 至 300")
            with self._lock:
                oldest = self._events[0]["seq"] if self._events else self._seq + 1
                events = [dict(event) for event in self._events if event["seq"] > after][:limit]
                cursor = events[-1]["seq"] if events else self._seq
                dropped = max(0, oldest - after - 1)
            snapshot = self.manager.snapshot()
            return {"cursor": cursor, "events": events, "anchors": self._anchors(snapshot),
                    "metrics": self._metrics(snapshot), "export_jobs": self._export_jobs(), "dropped": dropped}
        if method == "archive_anchors":
            return self.manager.archive.list_anchors()
        if method == "recent_messages":
            return self.manager.archive.recent_messages(params)
        if method == "refresh_viewers":
            return self.manager.refresh_viewers()
        if method == "viewer_refresh_status":
            return self.manager.viewer_refresh_status()
        if method in ("quality_sessions", "quality_detail", "quality_settings", "quality_rebuild", "quality_pause", "quality_configure", "quality_commands"):
            result = self.manager.archive.quality(method.removeprefix("quality_"), params)
            result["worker"] = self._quality.status()
            return result
        if method == "quality_export":
            return self._start_export_locked({**params, "anchor_ids": [params.get("anchor_id")]}, analysis=True)
        if method in ("archive_sessions", "preview_archive_delete", "execute_archive_delete"):
            allowed = {"anchor_ids", "month", "session_ids"}
            if method == "archive_sessions":
                allowed.update(("limit", "offset"))
            if set(params) - allowed:
                raise ServiceError("归档范围包含未知参数，请重新选择范围")
            selection = self._archive_selection(params)
            if method == "archive_sessions":
                limit, offset = params.get("limit", 200), params.get("offset", 0)
                if type(limit) is not int or not 1 <= limit <= 500 or type(offset) is not int or not 0 <= offset <= 1_000_000:
                    raise ServiceError("场次分页参数无效")
                return self.manager.archive.list_sessions(selection, limit, offset)
            reason = self._archive_delete_blocked(selection["anchor_ids"])
            if method == "preview_archive_delete":
                return {**self.manager.archive.preview_delete(selection), "ready": not reason, "blocked_reason": reason}
            if reason:
                raise ServiceError(reason)
            return self._start_archive_delete(selection)
        if method == "archive_delete_status":
            with self._lock:
                job = self._delete_jobs.get(params.get("job_id"))
                if job is None:
                    raise ServiceError("找不到此消息删除任务")
                return {key: copy.deepcopy(value) for key, value in job.items() if key not in ("thread", "cancel")}
        if method == "proxy_status":
            return self._proxy_snapshot()
        if method == "list_proxy_subscriptions":
            return {"items": self.subscriptions.snapshot()}
        if method == "add_proxy_subscription":
            return self.subscriptions.add(params.get("url"), params.get("interval_minutes", 60))
        if method == "refresh_proxy_subscription":
            return {"accepted": self.subscriptions.refresh(params.get("id"))}
        if method == "remove_proxy_subscription":
            return self.subscriptions.remove(params.get("id"))
        if method in ("preview_proxies", "import_proxies"):
            with self._lock:
                existing = list(self.config.get("imported_proxies", []))
            try:
                records, preview = parse_proxy_text(params.get("text"), params.get("default_protocol", "http"), existing)
            except ProxyImportError as error:
                raise ServiceError(str(error)) from None
            if method == "preview_proxies":
                return preview
            if len(existing) + preview['added'] > MAX_IMPORTED_PROXIES:
                raise ServiceError("最多保存 5000 条导入代理，请先删除不需要的地址")
            if records:
                previous = {record['id']: record for record in existing}
                replacements = {record['id']: record for record in records}
                merged = [replacements.get(record['id'], record) for record in existing]
                merged.extend(record for record in records if record['id'] not in previous)
                changed_routes = [record['endpoint'] for record in records
                                  if record['id'] in previous and any(
                                      record[key] != previous[record['id']][key]
                                      for key in ('protocol', 'username', 'password'))]
                self._replace_imported_proxies(merged, reset_failures=changed_routes)
            return preview
        if method == "list_imported_proxies":
            return self._list_imported_proxies()
        if method == "remove_imported_proxies":
            ids = params.get("ids")
            if not isinstance(ids, list) or len(ids) > MAX_IMPORTED_PROXIES or any(not isinstance(item, str) or not re.fullmatch(r"[0-9a-f]{24}", item) for item in ids):
                raise ServiceError("ids 须为已导入代理标识的列表")
            selected_ids = set(ids)
            with self._lock:
                existing = list(self.config.get("imported_proxies", []))
            records = [record for record in existing if record["id"] not in selected_ids]
            removed = len(existing) - len(records)
            if removed:
                self._replace_imported_proxies(records)
            return {"removed": removed}
        if method == "refresh_proxies":
            with self._lock:
                pool = self._proxy_pool
                if pool is None:
                    if self._prepared_proxy_pool is None:
                        self._prepared_proxy_pool = self._proxy_pool_factory()
                    pool = self._prepared_proxy_pool
                accepted = pool.refresh()
            return {"accepted": bool(accepted), "refresh": pool.snapshot().get("refresh", {})}
        if method == "configure_proxy":
            enabled = params.get("enabled")
            if not isinstance(enabled, bool):
                raise ServiceError("enabled 必须为布尔值")
            with self._lock:
                old_source = self.config.get("proxy_pool", {}).get("source", "public")
                old_direct = self.config.get("proxy_pool", {}).get("include_direct", True)
            source = params.get("source", old_source)
            include_direct = params.get("include_direct", old_direct)
            if type(include_direct) is not bool:
                raise ServiceError("include_direct 须为布尔值")
            if source not in ("public", "imported"):
                raise ServiceError("source 须为 public 或 imported")
            if enabled == (self._proxy_pool is not None) and source == old_source and include_direct == old_direct:
                return {"enabled": enabled, "source": source, "restart_required": False}
            self._require_proxy_idle()
            new_pool = None
            if enabled:
                with self._lock:
                    if source != old_source or include_direct != old_direct:
                        new_pool = self._build_proxy_pool(source, include_direct=include_direct)
                    elif self._prepared_proxy_pool is None:
                        self._prepared_proxy_pool = self._proxy_pool_factory()
                    if new_pool is None:
                        new_pool = self._prepared_proxy_pool
            try:
                self._update_config(lambda config: config.setdefault("proxy_pool", {}).update(enabled=enabled, source=source,
                                                                                           include_direct=include_direct, schema_version=1))
            except Exception:
                if new_pool is not None and new_pool is not self._prepared_proxy_pool:
                    new_pool.shutdown(timeout=0)
                raise
            self.manager.configure_proxy(new_pool)
            with self._lock:
                prepared = self._prepared_proxy_pool
                self._proxy_pool = new_pool
                if new_pool is not None or source != old_source or include_direct != old_direct:
                    self._prepared_proxy_pool = None
            if prepared is not None and prepared is not new_pool and (source != old_source or include_direct != old_direct):
                prepared.shutdown(timeout=0)
            self._sync_subscription_routes()
            return {"enabled": enabled, "source": source, "restart_required": True}
        if method == "add_anchors":
            rids, invalid = _parse_values(params.get("values"))
            with self._lock:
                added = [rid for rid in rids if rid not in self.config.get("anchors", {})]
                if len(self.config.get("anchors", {})) + len(added) > MAX_ANCHORS:
                    raise ServiceError("最多可配置 500 个监控任务")
            if added:
                self._update_config(lambda config: config.setdefault("anchors", {}).update({rid: f"主播 {rid}" for rid in added}))
            return {"added": added, "invalid": invalid}
        if method in ("start", "stop", "remove"):
            rids = _room_list(params.get("rids"))
            with self._lock:
                if any(rid not in self.config.get("anchors", {}) for rid in rids):
                    raise ServiceError("部分直播间不在当前列表中")
            # Reserve an operation slot before committing removal. A removed
            # room cannot be left running because a queue was already full.
            self._enqueue("stop" if method == "remove" else method, rids)
            if method == "remove":
                def remove(config):
                    for rid in rids:
                        config["anchors"].pop(rid, None)
                        config.setdefault("anchor_cookies", {}).pop(rid, None)
                        if rid in config.get("pc_unsupported_rids", []):
                            config["pc_unsupported_rids"].remove(rid)
                        metadata = config.get("anchor_metadata", {})
                        if isinstance(metadata, dict):
                            metadata.pop(rid, None)
                self._update_config(remove)
                with self._lock:
                    for rid in rids:
                        self._metadata_seen.pop(rid, None)
                        self._pending_metadata.pop(rid, None)
            return {"accepted": len(rids)}
        if method == "rename":
            rids = _room_list([params.get("rid")])
            name = params.get("name")
            if not isinstance(name, str) or not name.strip() or len(name) > 200:
                raise ServiceError("名称须为 1 至 200 个字符")
            rid, name = rids[0], name.strip()
            with self._lock:
                if rid not in self.config.get("anchors", {}):
                    raise ServiceError("直播间不在列表中")
            self._update_config(lambda config: config["anchors"].update({rid: name}))
            rename = getattr(self.manager, "rename", None)
            if callable(rename):
                rename(rid, name)
            return {"renamed": rid}
        if method == "set_pc_unsupported":
            rid = _room_list([params.get("rid")])[0]
            unsupported = params.get("unsupported")
            if type(unsupported) is not bool:
                raise ServiceError("unsupported 须为布尔值")
            with self._lock:
                if rid not in self.config.get("anchors", {}):
                    raise ServiceError("直播间不在列表中")
            def update_support(config):
                if rid not in config.get("anchors", {}):
                    raise ServiceError("直播间不在列表中")
                marked = set(config.get("pc_unsupported_rids", []))
                if unsupported:
                    marked.add(rid)
                else:
                    marked.discard(rid)
                config["pc_unsupported_rids"] = sorted(marked)
            self._update_config(update_support)
            return {"rid": rid, "pc_unsupported": unsupported, "pc_support_source": "manual" if unsupported else ""}
        if method in ("set_cookie", "clear_cookie"):
            rid = params.get("rid")
            if rid is not None:
                rid = _room_list([rid])[0]
                with self._lock:
                    if rid not in self.config.get("anchors", {}):
                        raise ServiceError("直播间不在列表中")
            cookie = "" if method == "clear_cookie" else params.get("cookie")
            if not isinstance(cookie, str) or len(cookie) > 65536 or "\r" in cookie or "\n" in cookie:
                raise ServiceError("Cookie 须为不超过 65536 字符的单行文本")
            cookie = cookie.strip()
            def update_cookie(config):
                if rid is None:
                    config["cookie"] = cookie
                elif cookie:
                    config.setdefault("anchor_cookies", {})[rid] = cookie
                else:
                    config.setdefault("anchor_cookies", {}).pop(rid, None)
            self._update_config(update_cookie)
            return {"cookie_configured": bool(cookie)}
        if method == "export":
            return self._start_export(params)
        if method == "cancel_export":
            with self._lock:
                job = self._jobs.get(params.get("job_id"))
                if job is None:
                    raise ServiceError("找不到此导出任务")
                job["cancel"].set()
            return {"cancel_requested": True}
        if method == "shutdown":
            return self.shutdown()
        raise ServiceError("未知 RPC 方法")

    @staticmethod
    def _archive_selection(params):
        def identifiers(value, label):
            if not isinstance(value, list) or not 1 <= len(value) <= 500:
                raise ServiceError(f"请选择 1 至 500 个{label}，不能无范围删除")
            if any(not isinstance(item, str) or not re.fullmatch(r"[0-9]{1,64}", item) or not int(item) for item in value):
                raise ServiceError(f"{label} ID 无效")
            return list(dict.fromkeys(value))
        anchors = identifiers(params.get("anchor_ids"), "历史主播")
        month = params.get("month")
        if month is not None:
            from archive_store import month_bounds
            try:
                month_bounds(month)
            except ValueError:
                raise ServiceError("月份须为有效的 YYYY-MM") from None
        sessions = params.get("session_ids")
        if sessions is not None:
            sessions = identifiers(sessions, "直播场次")
        return {"anchor_ids": anchors, "month": month, "session_ids": sessions}

    def _archive_delete_blocked(self, anchor_ids, ignore_current=False):
        with self._operations.mutex:
            if self._operations.unfinished_tasks:
                return "监控操作仍在排队或执行，请等待完成后再删除"
        can_delete = getattr(self.manager, "can_delete_archive", None)
        if not callable(can_delete) or not can_delete(anchor_ids):
            return "请先停止所选主播及尚未识别身份的监控，并等待连接完全退出后再删除"
        with self._lock:
            if self._archive_delete_in_progress and not ignore_current:
                return "消息删除仍在执行，请等待完成"
            for job in self._jobs.values():
                thread = job.get("thread")
                busy = job.get("status") in ("running", "queued") or (thread is not None and thread.is_alive())
                if busy and (not job.get("anchor_ids") or set(anchor_ids).intersection(job["anchor_ids"])):
                    return "所选主播的导出仍在运行或收尾，请等待完成后再删除"
        return ""

    def _start_archive_delete(self, selection):
        from archive_store import ArchiveDeleteCancelled
        with self._lock:
            if len(self._delete_jobs) >= 50:
                for old_id, old_job in list(self._delete_jobs.items()):
                    thread = old_job.get("thread")
                    if old_job["status"] != "running" and (thread is None or not thread.is_alive()):
                        self._delete_jobs.pop(old_id)
                        break
            job_id = uuid.uuid4().hex
            job = {"job_id": job_id, "status": "running", "report": None, "error": None, "cancel": CancelToken()}
            self._delete_jobs[job_id] = job
            self._archive_delete_in_progress = True
        def run():
            try:
                with self._archive_action_lock:
                    if self._closed.is_set() or job["cancel"].is_set():
                        raise ArchiveDeleteCancelled("消息删除已取消，未提交的更改已回滚")
                    reason = self._archive_delete_blocked(selection["anchor_ids"], ignore_current=True)
                    if reason:
                        raise ServiceError(reason)
                report = self.manager.archive.execute_delete(selection, job["cancel"])
                with self._lock:
                    job.update(status="completed", report=report)
            except ArchiveDeleteCancelled:
                with self._lock:
                    job.update(status="cancelled", error="消息删除已取消，未提交的更改已回滚")
            except Exception as error:
                with self._lock:
                    job.update(status="error", error=_safe_error(error))
            finally:
                with self._lock:
                    self._archive_delete_in_progress = False
        thread = threading.Thread(target=run, name=f"archive-delete-{job_id[:8]}", daemon=True)
        job["thread"] = thread
        try:
            thread.start()
        except RuntimeError:
            with self._lock:
                self._archive_delete_in_progress = False
                job.update(status="error", error="无法启动消息删除任务，请稍后重试", thread=None)
            raise ServiceError("无法启动消息删除任务，请稍后重试") from None
        return {"job_id": job_id, "accepted": True}

    def _start_export(self, params):
        with self._archive_action_lock:
            if self._archive_delete_in_progress:
                raise ServiceError("消息删除仍在执行，请等待完成后再导出")
            return self._start_export_locked(params)

    def _start_export_locked(self, params, analysis=False):
        from message_export import export_month, normalize_month, MESSAGE_TYPE_LABELS
        anchor_ids, types = params.get("anchor_ids"), params.get("types")
        if not isinstance(anchor_ids, list) or not anchor_ids or len(anchor_ids) > MAX_ANCHORS:
            raise ServiceError("请选择最多 500 个历史主播")
        if any(not isinstance(value, str) or not re.fullmatch(r"[0-9]{1,64}", value) or not int(value) for value in anchor_ids):
            raise ServiceError("历史主播 ID 无效")
        try:
            month = "" if analysis and not params.get("month") else normalize_month(params.get("month"))
        except ValueError:
            raise ServiceError("月份须为有效的 YYYY-MM") from None
        if types is None:
            types = list(MESSAGE_TYPE_LABELS)
        if not isinstance(types, list) or not types or any(value not in MESSAGE_TYPE_LABELS for value in types):
            raise ServiceError("请选择支持的内容消息类型")
        format_name, output_path = params.get("format", "xlsx"), params.get("output_path")
        if format_name not in ("xlsx", "csv") or not isinstance(output_path, str) or not output_path:
            raise ServiceError("导出需要 xlsx/csv 格式及目标文件路径")
        with self._lock:
            if sum(job["status"] == "running" for job in self._jobs.values()) >= 2:
                raise ServiceError("最多同时运行两个导出任务")
            if len(self._jobs) >= 100:
                for old_id in list(self._jobs):
                    if self._jobs[old_id]["status"] != "running":
                        self._jobs.pop(old_id)
                        break
            job_id = uuid.uuid4().hex
            job = {"id": job_id, "job_id": job_id, "status": "running", "report": None, "error": None,
                   "cancel": CancelToken(), "anchor_ids": list(dict.fromkeys(anchor_ids))}
            self._jobs[job_id] = job
        if analysis:
            from quality_worker import export_session_quality
            exporter = export_session_quality
            job["kind"] = "session_analysis"
        else:
            exporter = self._export_function or export_month
        def run():
            from message_export import ExportCancelled, ExportDataError
            try:
                report = exporter(self.data_dir, anchor_ids, month, output_path, format=format_name,
                                  types=types, cancel_event=job["cancel"])
                with self._lock:
                    job.update(status="completed", report=report)
            except ExportCancelled:
                with self._lock:
                    job["status"] = "cancelled"
            except Exception as error:
                with self._lock:
                    job.update(status="error", error=str(error) if isinstance(error, ExportDataError) else _safe_error(error))
        thread = threading.Thread(target=run, name=f"export-{job_id[:8]}", daemon=True)
        job["thread"] = thread
        thread.start()
        return {"job_id": job_id, "id": job_id}

    def shutdown(self):
        with self._shutdown_lock:
            with self._archive_action_lock:
                self._closed.set()
            return self._shutdown_locked()

    def _shutdown_locked(self):
        if self._shutdown_result is not None:
            return self._shutdown_result
        self._closed.set()
        subscriptions_stopped = self.subscriptions.shutdown(timeout=15)
        with self._lock:
            jobs = list(self._jobs.values())
            deletions = list(self._delete_jobs.values())
        for job in jobs + deletions:
            job["cancel"].set()
        delete_deadline = time.monotonic() + 5
        for job in deletions:
            thread = job.get("thread")
            if thread is not None:
                thread.join(max(0, delete_deadline - time.monotonic()))
        self._operator.join(timeout=2)
        quality_stopped = self._quality.close(timeout=5)
        all_stopped = self.manager.shutdown(timeout=15)
        with self._lock:
            prepared, self._prepared_proxy_pool = self._prepared_proxy_pool, None
        if prepared is not None:
            all_stopped = prepared.shutdown(timeout=5) and all_stopped
        deadline = time.monotonic() + 3
        for job in jobs:
            thread = job.get("thread")
            if thread is not None:
                thread.join(max(0, deadline - time.monotonic()))
        self._housekeeper.join(timeout=1)
        try:
            self._flush_metadata()
        except Exception as error:
            print(_safe_error(error), file=sys.stderr)
            all_stopped = False
        all_stopped = (all_stopped and quality_stopped and subscriptions_stopped and not self._operator.is_alive() and not self._housekeeper.is_alive()
                       and not any(job.get("thread") is not None and job["thread"].is_alive() for job in jobs))
        all_stopped = all_stopped and not any(job.get("thread") is not None and job["thread"].is_alive() for job in deletions)
        self._shutdown_result = {"closed": True, "all_workers_stopped": bool(all_stopped)}
        return self._shutdown_result


def run_service(input_stream=None, output_stream=None, factory=BackendService):
    input_stream = input_stream or sys.stdin
    output_stream = output_stream or sys.stdout
    service = None
    startup_error = None
    try:
        try:
            service = factory()
        except Exception as error:
            startup_error = _safe_error(error)
        while True:
            line = input_stream.readline(MAX_LINE_BYTES + 1)
            if not line:
                break
            request_id, method = None, None
            try:
                if len(line) > MAX_LINE_BYTES:
                    while line and not line.endswith("\n"):
                        line = input_stream.readline(MAX_LINE_BYTES + 1)
                    raise ServiceError("请求过大")
                request = json.loads(line)
                if not isinstance(request, dict):
                    raise ServiceError("请求必须是 JSON 对象")
                request_id, method = request.get("id"), request.get("method")
                if request_id is not None and (isinstance(request_id, bool) or not isinstance(request_id, (str, int))):
                    raise ServiceError("id 须为字符串或整数")
                if startup_error:
                    raise ServiceError(startup_error)
                result = service.dispatch(method, request.get("params", {}))
                response = {"id": request_id, "ok": True, "result": result}
            except (json.JSONDecodeError, UnicodeError):
                response = {"id": request_id, "ok": False, "error": "请求不是有效 UTF-8 JSON"}
            except Exception as error:
                response = {"id": request_id, "ok": False, "error": _safe_error(error)}
            output_stream.write(json.dumps(response, ensure_ascii=False, separators=(",", ":")) + "\n")
            output_stream.flush()
            if method == "shutdown":
                break
    finally:
        if service is not None:
            service.shutdown()


def main():
    from contextlib import redirect_stdout
    for stream in (sys.stdin, sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8", errors="replace" if stream is sys.stderr else "strict")
    protocol_output = sys.stdout
    with redirect_stdout(sys.stderr):
        run_service(output_stream=protocol_output)


if __name__ == "__main__":
    main()
