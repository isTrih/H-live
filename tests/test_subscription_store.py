"""Synthetic subscription lifecycle and encrypted-storage regression checks."""
import copy
from datetime import datetime, timezone
import json
from pathlib import Path
import sys
import tempfile
import threading
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python" / "collector"))
from app_storage import load_config, save_config
from proxy_import import make_record
from subscription_store import ProxySubscriptions, SubscriptionError, merged_candidates, validate_subscriptions


def record(ip="8.8.8.8", password="private-fixture"):
    return make_record({"ip": ip, "port": 8080, "username": "fixture", "password": password})


def until(predicate):
    deadline = time.monotonic() + 3
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(.005)
    raise AssertionError("subscription did not reach expected state")


class SubscriptionLifecycleTests(unittest.TestCase):
    def setUp(self):
        self.config = {"imported_proxies": [], "proxy_subscriptions": []}
        self.lock = threading.RLock()
        self.applied = []

    def read(self):
        with self.lock:
            return copy.deepcopy(self.config)

    def update(self, callback):
        with self.lock:
            candidate = copy.deepcopy(self.config)
            callback(candidate)
            candidate["proxy_subscriptions"] = validate_subscriptions(candidate["proxy_subscriptions"])
            self.config = candidate

    def manager(self, fetcher):
        manager = ProxySubscriptions(self.read, self.update,
                                     lambda: self.applied.append(merged_candidates(self.read())), fetcher)
        self.addCleanup(manager.shutdown)
        return manager

    def test_async_fetch_and_snapshot_never_expose_tokens_or_credentials(self):
        def fetch(url, stop_event=None):
            self.assertIn("token=subscription-secret", url)
            return {"records": [record()], "rejected": 2}
        manager = self.manager(fetch)
        added = manager.add("https://example.com/list?token=subscription-secret", 60)
        until(lambda: manager.snapshot()[0]["state"] == "ready")
        self.assertEqual(manager.snapshot()[0]["count"], 1)
        self.assertEqual(manager.snapshot()[0]["label"], "example.com")
        self.assertEqual(manager.snapshot()[0]["rejected"], 2)
        self.assertEqual(added["id"], self.read()["proxy_subscriptions"][0]["id"])
        exposed = json.dumps(manager.snapshot())
        for secret in ("subscription-secret", "private-fixture", "fixture", "https://"):
            self.assertNotIn(secret, exposed)
        self.assertEqual(len(self.applied), 1)

    def test_failed_refresh_preserves_last_successful_candidates(self):
        calls = []
        def fetch(url, stop_event=None):
            calls.append(True)
            if len(calls) > 1:
                raise RuntimeError("secret-transport-token")
            return {"records": [record()]}
        manager = self.manager(fetch)
        key = manager.add("https://example.com/list")["id"]
        until(lambda: manager.snapshot()[0]["state"] == "ready")
        original = self.read()
        self.assertTrue(manager.refresh(key))
        until(lambda: manager.snapshot()[0]["state"] == "error")
        self.assertEqual(self.read(), original)
        self.assertNotIn("secret-transport-token", json.dumps(manager.snapshot()))
        self.assertEqual(len(self.applied), 1)

    def test_remove_during_fetch_cannot_restore_subscription_or_routes(self):
        started, release = threading.Event(), threading.Event()
        def fetch(url, stop_event=None):
            started.set()
            release.wait(2)
            return {"records": [record()]}
        manager = self.manager(fetch)
        key = manager.add("https://example.com/list")["id"]
        self.assertTrue(started.wait(1))
        self.assertEqual(manager.remove(key), {"removed": 1})
        release.set()
        until(lambda: not manager.busy)
        self.assertTrue(manager.shutdown())
        self.assertEqual(self.read()["proxy_subscriptions"], [])
        self.assertEqual(self.applied, [[]])

    def test_removed_queued_subscription_is_not_fetched_from_an_old_tick_snapshot(self):
        self.config["proxy_subscriptions"] = [
            {"id": key * 32, "url": f"https://example.com/{key}", "records": [],
             "updated_at": "", "enabled": True, "interval_minutes": 60}
            for key in ("a", "b")]
        started, release = threading.Event(), threading.Event()
        calls = []
        def fetch(url, stop_event=None):
            calls.append(url)
            if url.endswith("/a"):
                started.set()
                release.wait(2)
            return {"records": []}
        manager = self.manager(fetch)
        self.addCleanup(release.set)
        self.assertTrue(started.wait(1))
        manager.remove("b" * 32)
        release.set()
        until(lambda: manager.snapshot()[0]["state"] == "ready")
        self.assertTrue(manager.shutdown())
        self.assertEqual(calls, ["https://example.com/a"])

    def test_shutdown_cancels_fetch_and_cannot_commit_late_response(self):
        started = threading.Event()
        def fetch(url, stop_event=None):
            started.set()
            stop_event.wait(2)
            return {"records": [record()]}
        manager = self.manager(fetch)
        manager.add("https://example.com/list")
        self.assertTrue(started.wait(1))
        self.assertTrue(manager.shutdown())
        self.assertEqual(self.read()["proxy_subscriptions"][0]["records"], [])
        self.assertEqual(self.applied, [])

    def test_manual_records_win_and_subscription_removal_keeps_manual(self):
        manual = record(password="manual-password")
        self.config["imported_proxies"] = [manual]
        manager = self.manager(lambda *args, **kwargs: {"records": [record(), record("1.1.1.1")]})
        key = manager.add("https://example.com/list")["id"]
        until(lambda: manager.snapshot()[0]["state"] == "ready")
        merged = merged_candidates(self.read())
        self.assertEqual(len(merged), 2)
        self.assertEqual(merged[0], manual)
        manager.remove(key)
        self.assertEqual(merged_candidates(self.read()), [manual])

    def test_saved_overdue_subscription_refreshes_without_manual_request(self):
        self.config["proxy_subscriptions"] = [{"id": "a" * 32, "url": "https://example.com/list",
                "records": [], "updated_at": "2020-01-01T00:00:00+00:00", "enabled": True, "interval_minutes": 60}]
        manager = self.manager(lambda *args, **kwargs: {"records": [record()]})
        until(lambda: manager.snapshot()[0]["state"] == "ready")
        self.assertEqual(manager.snapshot()[0]["count"], 1)

    def test_duplicates_bad_urls_and_intervals_rejected_without_fetch(self):
        count = []
        manager = self.manager(lambda *args, **kwargs: count.append(True) or {"records": []})
        for url in ("http://example.com", "https://user:secret@example.com", "https://example.com/#secret", "https://example.com/\n"):
            with self.assertRaises(SubscriptionError):
                manager.add(url)
        with self.assertRaises(SubscriptionError):
            manager.add("https://example.com/list", 0)
        self.assertEqual(count, [])
        manager.add("https://example.com/list")
        with self.assertRaises(SubscriptionError):
            manager.add("https://example.com/list")

    def test_subscription_url_and_auth_are_encrypted_at_rest(self):
        subscription = {"id": "a" * 32, "url": "https://example.com/?token=subscription-secret",
                        "records": [record()], "updated_at": "", "enabled": True, "interval_minutes": 60}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            save_config({"proxy_subscriptions": [subscription]}, path)
            encoded = (path / "config.json").read_bytes()
            for secret in (b"subscription-secret", b"private-fixture", b"example.com", b"8.8.8.8"):
                self.assertNotIn(secret, encoded)
            decoded = load_config(path, path / "no-legacy")
            self.assertEqual(decoded["proxy_subscriptions"], [subscription])


if __name__ == "__main__":
    unittest.main()
