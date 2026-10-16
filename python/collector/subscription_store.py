"""Saved proxy subscriptions and a single cancellable refresh worker.

The backend owns encrypted persistence. This module never returns subscription
URLs, query tokens or proxy credentials to the UI.
"""
from __future__ import annotations

from datetime import datetime, timezone
import re
import threading
import time
import uuid

from proxy_import import MAX_IMPORTED_PROXIES, validate_records


class SubscriptionError(ValueError):
    pass


def url_label(url):
    from proxy_subscription import SubscriptionError as URLSubscriptionError, subscription_summary
    try:
        return subscription_summary(url)["label"]
    except URLSubscriptionError as error:
        raise SubscriptionError(str(error)) from None


def validate_subscriptions(values):
    if not isinstance(values, list) or len(values) > 20:
        raise SubscriptionError("最多保存 20 个代理订阅")
    result, seen = [], set()
    for item in values:
        if not isinstance(item, dict) or not re.fullmatch(r"[0-9a-f]{32}", str(item.get("id", ""))):
            raise SubscriptionError("代理订阅标识无效")
        if item["id"] in seen:
            raise SubscriptionError("代理订阅标识重复")
        seen.add(item["id"])
        url_label(item.get("url"))
        interval = item.get("interval_minutes", 60)
        if type(interval) is not int or not 5 <= interval <= 1440 or type(item.get("enabled", True)) is not bool:
            raise SubscriptionError("订阅刷新间隔须为 5–1440 分钟")
        records = validate_records(item.get("records", []))
        updated = item.get("updated_at", "")
        if not isinstance(updated, str) or len(updated) > 40:
            raise SubscriptionError("订阅更新时间无效")
        result.append({"id": item["id"], "url": item["url"], "enabled": item.get("enabled", True),
                       "interval_minutes": interval, "records": records, "updated_at": updated})
    return result


def merged_candidates(config):
    # Explicitly imported records win over public subscriptions on collisions.
    by_id = {record["id"]: record for record in config.get("imported_proxies", [])}
    for subscription in config.get("proxy_subscriptions", []):
        if subscription.get("enabled", True):
            for record in subscription.get("records", []):
                by_id.setdefault(record["id"], record)
    if len(by_id) > MAX_IMPORTED_PROXIES:
        raise SubscriptionError("已导入代理和订阅合计最多 5000 个地址")
    return list(by_id.values())


class ProxySubscriptions:
    def __init__(self, read_config, update_config, on_change, fetcher=None):
        self._read = read_config
        self._update = update_config
        self._changed = on_change
        self._fetcher = fetcher
        self._lock = threading.RLock()
        self._closed = threading.Event()
        self._wake = threading.Event()
        self._requested = set()
        self._runtime = {}
        self._next = {}
        self._thread = threading.Thread(target=self._run, name="proxy-subscriptions", daemon=True)
        self._thread.start()

    @property
    def busy(self):
        with self._lock:
            return any(value.get("state") in {"queued", "refreshing"} for value in self._runtime.values())

    def snapshot(self):
        config = self._read()
        with self._lock:
            return [{"id": item["id"], "label": url_label(item["url"]),
                     "interval_minutes": item["interval_minutes"], "enabled": item["enabled"],
                     "count": len(item["records"]), "updated_at": item["updated_at"],
                     **self._runtime.get(item["id"], {"state": "idle", "error": ""})}
                    for item in config.get("proxy_subscriptions", [])]

    def add(self, url, interval_minutes=60):
        url_label(url)
        item = {"id": uuid.uuid4().hex, "url": url, "interval_minutes": interval_minutes,
                "enabled": True, "records": [], "updated_at": ""}
        validate_subscriptions([item])
        def update(config):
            values = config.setdefault("proxy_subscriptions", [])
            if any(value["url"] == url for value in values):
                raise SubscriptionError("此订阅地址已添加")
            if len(values) >= 20:
                raise SubscriptionError("最多保存 20 个代理订阅")
            values.append(item)
        self._update(update)
        self.refresh(item["id"])
        return {"id": item["id"], "accepted": True}

    def refresh(self, subscription_id):
        if self._closed.is_set():
            raise SubscriptionError("服务正在关闭")
        if not any(item["id"] == subscription_id for item in self._read().get("proxy_subscriptions", [])):
            raise SubscriptionError("订阅不存在")
        with self._lock:
            if self._runtime.get(subscription_id, {}).get("state") == "refreshing" or subscription_id in self._requested:
                return False
            self._requested.add(subscription_id)
            self._runtime[subscription_id] = {"state": "queued", "error": ""}
        self._wake.set()
        return True

    def remove(self, subscription_id):
        removed = []
        def update(config):
            values = config.get("proxy_subscriptions", [])
            removed.extend(item for item in values if item["id"] == subscription_id)
            config["proxy_subscriptions"] = [item for item in values if item["id"] != subscription_id]
        self._update(update)
        with self._lock:
            self._requested.discard(subscription_id)
            self._runtime.pop(subscription_id, None)
            self._next.pop(subscription_id, None)
        self._changed()
        return {"removed": len(removed)}

    def _run(self):
        while not self._closed.is_set():
            try:
                self._tick()
            except Exception:
                # A persistence/callback problem cannot kill future refreshes.
                pass
            self._wake.wait(0.5)
            self._wake.clear()

    def _tick(self):
        for item in self._read().get("proxy_subscriptions", []):
            if self._closed.is_set():
                return
            # An earlier subscription may have spent seconds fetching. Do not
            # use that old tick snapshot to fetch a subsequently removed entry.
            current = next((entry for entry in self._read().get("proxy_subscriptions", [])
                            if entry["id"] == item["id"] and entry["url"] == item["url"]), None)
            if current is None or not current["enabled"]:
                continue
            item = current
            key = item["id"]
            with self._lock:
                if key not in self._next:
                    try:
                        age = max(0, time.time() - datetime.fromisoformat(item["updated_at"]).timestamp())
                    except (ValueError, TypeError):
                        age = item["interval_minutes"] * 60
                    self._next[key] = time.monotonic() + max(0, item["interval_minutes"] * 60 - age)
                if key not in self._requested and time.monotonic() < self._next[key]:
                    continue
                self._requested.discard(key)
                runtime = {"state": "refreshing", "error": ""}
                self._runtime[key] = runtime
            try:
                if self._fetcher is None:
                    from proxy_subscription import fetch_subscription
                    fetcher = fetch_subscription
                else:
                    fetcher = self._fetcher
                fetched = fetcher(item["url"], stop_event=self._closed)
                records = validate_records(fetched["records"])
                if self._closed.is_set():
                    return
                updated = datetime.now(timezone.utc).isoformat(timespec="seconds")
                applied = []
                def update(config):
                    if self._closed.is_set():
                        return
                    for current in config.get("proxy_subscriptions", []):
                        if current["id"] == key and current["url"] == item["url"]:
                            current.update(records=records, updated_at=updated)
                            merged_candidates(config)
                            applied.append(True)
                self._update(update)
                if applied:
                    self._changed()
                    with self._lock:
                        if self._runtime.get(key) is runtime:
                            self._runtime[key] = {"state": "ready", "error": "", "rejected": fetched.get("rejected", 0)}
            except Exception as error:
                from proxy_subscription import SubscriptionError as FetchSubscriptionError
                detail = str(error) if isinstance(error, FetchSubscriptionError) else "订阅刷新失败，请检查地址及网络"
                exists = any(current["id"] == key for current in self._read().get("proxy_subscriptions", []))
                with self._lock:
                    if exists and self._runtime.get(key) is runtime:
                        self._runtime[key] = {"state": "error", "error": detail + "；已保留上次候选"}
            finally:
                with self._lock:
                    if key in self._runtime:
                        self._next[key] = time.monotonic() + item["interval_minutes"] * 60

    def shutdown(self, timeout=15):
        self._closed.set()
        self._wake.set()
        if self._thread is not threading.current_thread():
            self._thread.join(timeout)
        return not self._thread.is_alive()
