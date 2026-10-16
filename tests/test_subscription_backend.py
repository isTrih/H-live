"""Real backend and DPAPI with temporary data, synthetic sources, zero network."""
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python" / "collector"))
from app_storage import load_config
from backend_service import BackendService
from proxy_import import make_record


def until(condition, timeout=3):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if condition():
            return
        time.sleep(.005)
    raise AssertionError("condition timed out")


def record(ip="8.8.8.8", password="synthetic-proxy-secret"):
    return make_record({"ip": ip, "port": 8080, "username": "synthetic-account", "password": password})


@unittest.skipUnless(os.name == "nt", "DPAPI integration requires Windows")
class SubscriptionBackendTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        guard = patch("requests.Session", side_effect=AssertionError("network forbidden in subscription integration"))
        session = guard.start()
        self.addCleanup(guard.stop)
        self.addCleanup(session.assert_not_called)
        health = patch("proxy_pool.validate_proxy", return_value={"ok": True, "status_code": 200})
        health.start()
        self.addCleanup(health.stop)

    def service(self, fetcher):
        result = BackendService(self.directory, subscription_fetcher=fetcher)
        self.addCleanup(result.shutdown)
        return result

    def test_real_backend_encrypts_subscription_and_never_returns_url_tokens(self):
        service = self.service(lambda *args, **kwargs: {"records": [record()]})
        url = "https://example.com/private/list?token=synthetic-subscription-token"
        service.dispatch("add_proxy_subscription", {"url": url})
        until(lambda: service.dispatch("list_proxy_subscriptions")["items"][0]["state"] == "ready")
        encoded = (self.directory / "config.json").read_bytes()
        for secret in (b"synthetic-subscription-token", b"synthetic-proxy-secret", b"synthetic-account", b"example.com"):
            self.assertNotIn(secret, encoded)
        self.assertIn(b"proxy_subscriptions_encrypted", encoded)
        loaded = load_config(self.directory, self.directory / "no-legacy.json")
        self.assertEqual(loaded["proxy_subscriptions"][0]["url"], url)
        self.assertEqual(loaded["proxy_subscriptions"][0]["records"], [record()])
        for method in ("init", "poll", "proxy_status", "list_proxy_subscriptions", "list_imported_proxies"):
            output = json.dumps(service.dispatch(method))
            for secret in ("synthetic-subscription-token", "synthetic-proxy-secret", "synthetic-account", "/private/list"):
                self.assertNotIn(secret, output)

    def test_subscription_changes_preserve_manual_record_and_live_lease(self):
        manual = record(password="manual-route-secret")
        service = self.service(lambda *args, **kwargs: {"records": [record(), record("1.1.1.1")]})
        service.dispatch("import_proxies", {"text": json.dumps([manual])})
        service.dispatch("configure_proxy", {"enabled": True, "source": "imported", "include_direct": False})
        pool = service._proxy_pool
        with patch("proxy_pool.validate_proxy", return_value={"ok": True, "status_code": 200}):
            service.dispatch("refresh_proxies")
            until(lambda: pool.snapshot()["available"] == 1)
        lease = pool.acquire("retained-room", threading.Event())
        self.addCleanup(lambda: pool.release("retained-room", lease))
        key = service.dispatch("add_proxy_subscription", {"url": "https://example.com/list"})["id"]
        until(lambda: service.dispatch("list_proxy_subscriptions")["items"][0]["state"] == "ready")
        self.assertIs(service._proxy_pool, pool)
        self.assertIs(pool.get("retained-room"), lease)
        self.assertEqual(pool._imported[manual["endpoint"]], manual)
        self.assertEqual(len(pool._imported), 2)
        service.dispatch("remove_proxy_subscription", {"id": key})
        self.assertEqual(list(pool._imported.values()), [manual])
        self.assertIs(pool.get("retained-room"), lease)

    def test_delete_while_fetching_and_shutdown_cannot_commit_late_candidates(self):
        started, release = threading.Event(), threading.Event()
        def fetch(url, stop_event=None):
            started.set()
            release.wait(2)
            return {"records": [record()]}
        service = self.service(fetch)
        self.addCleanup(release.set)
        service.dispatch("configure_proxy", {"enabled": True, "source": "imported", "include_direct": False})
        key = service.dispatch("add_proxy_subscription", {"url": "https://example.com/list"})["id"]
        self.assertTrue(started.wait(1))
        self.assertEqual(service.dispatch("remove_proxy_subscription", {"id": key}), {"removed": 1})
        release.set()
        self.assertTrue(service.shutdown()["all_workers_stopped"])
        self.assertEqual(load_config(self.directory, self.directory / "no-legacy.json")["proxy_subscriptions"], [])
        self.assertEqual(service._proxy_pool._imported, {})

    def test_include_direct_change_discards_prepared_pool_instead_of_reusing_old_policy(self):
        service = self.service(lambda *args, **kwargs: {"records": []})
        service.dispatch("configure_proxy", {"enabled": False, "source": "imported", "include_direct": True})
        service.dispatch("refresh_proxies")
        prepared = service._prepared_proxy_pool
        self.assertTrue(prepared.include_direct)
        service.dispatch("configure_proxy", {"enabled": True, "source": "imported", "include_direct": False})
        self.assertIsNot(service._proxy_pool, prepared)
        self.assertFalse(service._proxy_pool.include_direct)
        self.assertTrue(prepared.snapshot()["closed"])
        self.assertIsNone(service._prepared_proxy_pool)

    def test_stale_refresh_sync_cannot_restore_routes_after_delete(self):
        service = self.service(lambda *args, **kwargs: {"records": [record()]})
        service.dispatch("configure_proxy", {"enabled": True, "source": "imported", "include_direct": False})
        pool = service._proxy_pool
        entered, release, removed = threading.Event(), threading.Event(), threading.Event()
        errors = []
        original = pool.update_imported_candidates
        def delayed(records):
            if records and not entered.is_set():
                entered.set()
                release.wait(3)
            return original(records)
        def remove(key):
            try:
                service.dispatch("remove_proxy_subscription", {"id": key})
            except Exception as error:
                errors.append(type(error).__name__)
            finally:
                removed.set()
        self.addCleanup(release.set)
        with patch.object(pool, "update_imported_candidates", side_effect=delayed):
            key = service.dispatch("add_proxy_subscription", {"url": "https://example.com/list"})["id"]
            self.assertTrue(entered.wait(1))
            thread = threading.Thread(target=remove, args=(key,))
            thread.start()
            # A safe implementation may serialize deletion behind the old
            # update. Give the stale implementation time to overtake it.
            removed.wait(.1)
            release.set()
            thread.join(3)
            self.assertFalse(thread.is_alive())
        self.assertEqual(errors, [])
        self.assertTrue(service.subscriptions.shutdown())
        self.assertEqual(service.dispatch("list_proxy_subscriptions")["items"], [])
        self.assertEqual(pool._imported, {}, "A deleted subscription must not be restored by an earlier sync")


if __name__ == "__main__":
    unittest.main()
