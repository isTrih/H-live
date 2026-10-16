from collections import Counter
import json
from pathlib import Path
import sys
import threading
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python" / "collector"))
from proxy_pool import ProxyPool


ENDPOINTS = ["8.8.8.8:80", "1.1.1.1:80", "9.9.9.9:80", "8.8.4.4:80"]


def record(endpoint, password="fixture-secret"):
    return {"endpoint": endpoint, "protocol": "http", "username": "fixture-user", "password": password}


def healthy(endpoint, **kwargs):
    return {"ok": True, "status_code": 200, "latency_ms": 1}


def wait_until(predicate, timeout=3):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.005)
    raise AssertionError("condition did not become true")


class SharedExitRoutingTests(unittest.TestCase):
    def setUp(self):
        self.pools = []

    def tearDown(self):
        for pool in self.pools:
            pool.shutdown(3)

    def pool(self, **kwargs):
        kwargs.setdefault("imported_candidates", [record(ep) for ep in ENDPOINTS])
        kwargs.setdefault("shared", True)
        kwargs.setdefault("include_direct", True)
        kwargs.setdefault("validator", healthy)
        kwargs.setdefault("route_request_interval_seconds", 0)
        pool = ProxyPool(**kwargs)
        self.pools.append(pool)
        return pool

    def test_fifty_concurrent_tasks_balance_four_proxies_and_direct_after_warmup(self):
        gate = threading.Event()
        pool = self.pool(validator=lambda ep: (gate.wait(2), healthy(ep))[1])
        leases, lock = {}, threading.Lock()
        def acquire(index):
            lease = pool.acquire(str(index), threading.Event())
            with lock:
                leases[str(index)] = lease
        threads = [threading.Thread(target=acquire, args=(index,)) for index in range(50)]
        for thread in threads:
            thread.start()
        wait_until(lambda: pool.snapshot()["checking"] == 4)
        self.assertEqual(leases, {}, "direct must not absorb the tasks while the initial batch is checking")
        gate.set()
        for thread in threads:
            thread.join(3)
            self.assertFalse(thread.is_alive())
        self.assertEqual(Counter(item.endpoint for item in leases.values()), dict.fromkeys(ENDPOINTS + ["direct"], 10))
        self.assertEqual(len({id(item) for item in leases.values()}), 50)
        self.assertEqual(pool.snapshot()["available"], 4)
        self.assertEqual(pool.snapshot()["assigned"], 50)
        for rid, lease in leases.items():
            self.assertIs(pool.acquire(rid, threading.Event()), lease)
            if lease.kind == "direct":
                self.assertEqual(lease.proxy_url, "")
            else:
                peers = [other for other in leases.values() if other.endpoint == lease.endpoint]
                self.assertTrue(all(other.control is lease.control for other in peers))
            self.assertTrue(pool.release(rid, lease))
        self.assertEqual(pool.snapshot()["assigned"], 0)
        self.assertTrue(all(route["assigned_tasks"] == 0 for route in pool.snapshot()["routes"]))
        serialized = json.dumps(pool.snapshot()) + repr(list(leases.values()))
        self.assertNotIn("fixture-secret", serialized)
        self.assertNotIn("fixture-user", serialized)

    def test_prepared_pool_does_not_wait_again_and_stale_tokens_cannot_release_new_lease(self):
        pool = self.pool()
        pool.refresh()
        wait_until(lambda: not pool.snapshot()["refresh"]["active"])
        begin = time.monotonic()
        old = pool.acquire("a", threading.Event())
        self.assertLess(time.monotonic() - begin, 0.2)
        pool.release("a", old)
        new = pool.acquire("a", threading.Event())
        self.assertIsNot(new, old)
        self.assertFalse(pool.release("a", old))
        self.assertIs(pool.get("a"), new)

    def test_warmup_is_bounded_and_cancel_does_not_leave_a_lease(self):
        gate, stop = threading.Event(), threading.Event()
        pool = self.pool(validator=lambda ep: (gate.wait(2), healthy(ep))[1], warmup_seconds=0.12)
        values = []
        thread = threading.Thread(target=lambda: values.append(pool.acquire("cancel", stop)))
        thread.start()
        wait_until(lambda: pool.snapshot()["checking"] == 4)
        stop.set()
        thread.join(0.2)
        self.assertEqual(values, [None])
        self.assertIsNone(pool.get("cancel"))
        try:
            begin = time.monotonic()
            lease = pool.acquire("fallback", threading.Event())
            self.assertEqual(lease.kind, "direct")
            self.assertLess(time.monotonic() - begin, 0.25)
        finally:
            gate.set()

    def test_target_refusal_cools_only_that_exit_without_removing_any_lease(self):
        pool = self.pool(target_cooldown_seconds=0.12)
        leases = [pool.acquire(str(index), threading.Event()) for index in range(10)]
        first = leases[0]
        self.assertTrue(pool.report_failure("0", first, "target_http", 444))
        self.assertEqual(pool.snapshot()["assigned"], 10)
        self.assertEqual(pool.snapshot()["failed"], 0)
        self.assertGreater(first.control.snapshot()["cooldown_seconds"], 0)
        self.assertEqual(leases[1].control.snapshot()["cooldown_seconds"], 0)
        self.assertNotEqual(pool.acquire("new", threading.Event()).route_id, first.route_id)
        self.assertIs(pool.get("0"), first)
        self.assertTrue(pool.report_failure("0", first, "target_http", 500))

    def test_auth_rejects_only_failed_task_and_preserves_other_existing_socket_leases(self):
        pool = self.pool(imported_candidates=[record(ENDPOINTS[0])], include_direct=False)
        first = pool.acquire("a", threading.Event())
        second = pool.acquire("b", threading.Event())
        self.assertFalse(pool.report_failure("a", first, "proxy_auth"))
        self.assertIsNone(pool.get("a"))
        self.assertIs(pool.get("b"), second)
        self.assertEqual(pool.snapshot()["available"], 0)
        self.assertEqual(pool.snapshot()["routes"][0]["state"], "suspended")
        self.assertEqual(second.control.snapshot()["closed"], False)

    def test_transport_requires_distinct_tasks_and_schedules_one_recheck(self):
        calls = []
        pool = self.pool(imported_candidates=[record(ENDPOINTS[0])], include_direct=False,
                         validator=lambda ep: calls.append(ep) or healthy(ep), transport_cooldown_seconds=0.1,
                         transport_window_seconds=1)
        leases = {rid: pool.acquire(rid, threading.Event()) for rid in ("a", "b", "c")}
        for _ in range(4):
            pool.report_failure("a", leases["a"], "transport")
        self.assertEqual(len(calls), 1)
        pool.report_failure("b", leases["b"], "transport")
        pool.report_failure("c", leases["c"], "transport")
        wait_until(lambda: len(calls) == 2)
        for rid, lease in leases.items():
            self.assertTrue(pool.report_failure(rid, lease, "transport"))
            self.assertIs(pool.get(rid), lease)
        time.sleep(0.04)
        self.assertEqual(len(calls), 2)
        self.assertEqual(pool.snapshot()["failed"], 0)

    def test_subscription_remove_drains_existing_lease_without_new_assignments(self):
        pool = self.pool(imported_candidates=[record(ENDPOINTS[0])], include_direct=False)
        first = pool.acquire("a", threading.Event())
        pool.update_imported_candidates([])
        routes = pool.snapshot()["routes"]
        self.assertEqual(routes[0]["state"], "draining")
        self.assertEqual(routes[0]["assigned_tasks"], 1)
        self.assertIs(pool.get("a"), first)
        self.assertEqual(pool.snapshot()["available"], 0)
        self.assertTrue(pool.release("a", first))
        self.assertEqual(pool.snapshot()["routes"], [])

    def test_removed_or_replaced_route_releases_only_on_next_connection_validation(self):
        for replacement in ([], [record(ENDPOINTS[0], "updated-secret")]):
            with self.subTest(replacement=bool(replacement)):
                pool = self.pool(imported_candidates=[record(ENDPOINTS[0])], include_direct=False)
                first = pool.acquire("a", threading.Event())
                second = pool.acquire("b", threading.Event())
                pool.update_imported_candidates(replacement)
                self.assertIs(pool.get("a"), first)
                self.assertIs(pool.get("b"), second)
                self.assertFalse(pool.revalidate("a", first))
                self.assertIsNone(pool.get("a"))
                self.assertIs(pool.get("b"), second)
                if replacement:
                    wait_until(lambda: pool.snapshot()["checks_total"] == 2)
                    new = pool.acquire("a", threading.Event())
                    self.assertNotEqual(first.generation, new.generation)
                    self.assertTrue(pool.revalidate("a", new))
                    self.assertFalse(pool.revalidate("a", first))
                    self.assertIs(pool.get("a"), new)

    def test_retired_route_transport_failure_does_not_hold_removed_endpoint(self):
        pool = self.pool(imported_candidates=[record(ENDPOINTS[0])])
        old = pool.acquire("a", threading.Event())
        pool.update_imported_candidates([])
        self.assertFalse(pool.report_failure("a", old, "transport"))
        self.assertEqual(pool.acquire("a", threading.Event()).kind, "direct")

    def test_subscription_candidate_work_is_bounded_and_refills_released_capacity(self):
        gate = threading.Event()
        pool = self.pool(imported_candidates=[], max_candidates=20,
                         validator=lambda ep: (gate.wait(2), healthy(ep))[1])
        candidates = [record(f"8.8.4.{index}:80") for index in range(1, 41)]
        pool.update_imported_candidates(candidates)
        wait_until(lambda: pool.snapshot()["checking"] == 12)
        snapshot = pool.snapshot()
        self.assertLessEqual(snapshot["checking"] + snapshot["pending"], 20)
        gate.set()
        wait_until(lambda: pool.snapshot()["checking"] == 0 and pool.snapshot()["pending"] == 0)
        before = pool.snapshot()
        self.assertEqual(before["checks_total"], 20)
        self.assertEqual(before["available"], 20)
        removed = before["routes"][0]["endpoint"]
        pool.update_imported_candidates([item for item in candidates if item["endpoint"] != removed])
        wait_until(lambda: pool.snapshot()["checks_total"] == 21)
        self.assertEqual(pool.snapshot()["available"], 20)
        self.assertNotIn(removed, {row["endpoint"] for row in pool.snapshot()["routes"]})

    def test_draining_active_exit_occupies_capacity_until_its_last_lease_releases(self):
        candidates = [record(f"8.8.4.{index}:80") for index in range(1, 21)]
        pool = self.pool(imported_candidates=[], max_candidates=20, include_direct=False)
        pool.update_imported_candidates(candidates)
        wait_until(lambda: pool.snapshot()["available"] == 20)
        leases = {str(index): pool.acquire(str(index), threading.Event()) for index in range(20)}
        removed = leases["0"].endpoint
        replacement = record("1.1.1.1:80")
        pool.update_imported_candidates([item for item in candidates if item["endpoint"] != removed] + [replacement])
        self.assertEqual(pool.snapshot()["checks_total"], 20)
        self.assertEqual(pool.snapshot()["pending"], 0)
        self.assertIs(pool.get("0"), leases["0"])
        pool.release("0", leases["0"])
        wait_until(lambda: pool.snapshot()["checks_total"] == 21)
        self.assertEqual(pool.snapshot()["available"], 20)
        self.assertIn(replacement["endpoint"], {row["endpoint"] for row in pool.snapshot()["routes"]})

    def test_changed_credentials_validate_new_generation_keep_old_lease_and_control(self):
        urls = []
        with patch("proxy_pool.validate_proxy", side_effect=lambda ep, proxy_url: urls.append(proxy_url) or healthy(ep)):
            pool = self.pool(imported_candidates=[record(ENDPOINTS[0], "old-secret")], include_direct=False, validator=None)
            old = pool.acquire("a", threading.Event())
            pool.update_imported_candidates([record(ENDPOINTS[0], "new-secret")])
            wait_until(lambda: pool.snapshot()["checks_total"] == 2)
            new = pool.acquire("b", threading.Event())
        self.assertIn("old-secret", old.proxy_url)
        self.assertIn("new-secret", new.proxy_url)
        self.assertEqual(old.route_id, new.route_id)
        self.assertIs(old.control, new.control)
        self.assertNotEqual(old.generation, new.generation)
        routes = pool.snapshot()["routes"]
        self.assertEqual(len(routes), 1)
        self.assertEqual(routes[0]["assigned_tasks"], 2)
        self.assertEqual(len(urls), 2)
        self.assertIs(pool.get("a"), old)
        # A late failure using old credentials must not block the replacement.
        pool.report_failure("a", old, "proxy_auth")
        self.assertEqual(pool.snapshot()["failed"], 0)
        self.assertEqual(pool.snapshot()["available"], 1)

    def test_ttl_recheck_preserves_live_tokens_and_uses_same_exit_http_limit(self):
        calls = []
        pool = self.pool(imported_candidates=[record(ENDPOINTS[0])], include_direct=False,
                         ttl_seconds=0.08, validator=lambda ep: calls.append(ep) or healthy(ep))
        lease = pool.acquire("a", threading.Event())
        permits = [lease.control.acquire_request() for _ in range(2)]
        time.sleep(0.1)
        pool.snapshot()
        wait_until(lambda: pool.snapshot()["checking"] == 1)
        self.assertEqual(len(calls), 1, "a health probe must also wait behind the two active HTTP requests")
        self.assertIs(pool.get("a"), lease)
        for release in permits:
            release()
        wait_until(lambda: len(calls) == 2)
        self.assertIs(pool.get("a"), lease)
        self.assertIs(pool.acquire("b", threading.Event()).control, lease.control)

    def test_expired_subscription_entry_never_gets_new_lease(self):
        pool = self.pool(imported_candidates=[record(ENDPOINTS[0])])
        old = pool.acquire("a", threading.Event())
        expired = {**record(ENDPOINTS[0]), "expires_at": "2000-01-01T00:00:00+00:00"}
        pool.update_imported_candidates([expired])
        self.assertIs(pool.get("a"), old)
        self.assertEqual(pool.acquire("b", threading.Event()).kind, "direct")
        self.assertEqual(pool.snapshot()["available"], 0)

    def test_metadata_and_future_expiry_changes_do_not_resurrect_failed_route(self):
        calls = []
        initial = {**record(ENDPOINTS[0]), "expires_at": "2090-01-01T00:00:00+00:00"}
        pool = self.pool(imported_candidates=[initial],
                         validator=lambda ep: calls.append(ep) or {"ok": False, "status_code": 407})
        pool.refresh()
        wait_until(lambda: not pool.snapshot()["refresh"]["active"])
        pool.update_imported_candidates([{**initial, "city": "fixture-city", "country_code": "ZZ",
                                          "expires_at": "2091-01-01T00:00:00+00:00"}])
        time.sleep(0.05)
        self.assertEqual(len(calls), 1)
        self.assertEqual(pool.snapshot()["failed"], 1)
        self.assertEqual(pool.snapshot()["pending"], 0)
        self.assertEqual(pool.snapshot()["available"], 0)

    def test_expired_to_valid_and_explicit_readdition_are_checked_again(self):
        calls = []
        endpoint = ENDPOINTS[0]
        pool = self.pool(imported_candidates=[record(endpoint)],
                         validator=lambda ep: calls.append(ep) or healthy(ep))
        first = pool.acquire("a", threading.Event())
        expired = {**record(endpoint), "expires_at": "2000-01-01T00:00:00+00:00"}
        pool.update_imported_candidates([expired])
        pool.update_imported_candidates([record(endpoint)])
        wait_until(lambda: pool.snapshot()["checks_total"] == 2)
        self.assertIs(pool.get("a"), first)
        pool.update_imported_candidates([])
        pool.update_imported_candidates([record(endpoint)])
        wait_until(lambda: pool.snapshot()["checks_total"] == 3)
        self.assertEqual(len(calls), 3)
        self.assertIs(pool.get("a"), first)

    def test_failed_health_recheck_keeps_existing_rooms_but_suspends_new_assignments(self):
        calls = []
        def check(ep):
            calls.append(ep)
            return healthy(ep) if len(calls) == 1 else {"ok": False, "status_code": 502}
        pool = self.pool(imported_candidates=[record(ENDPOINTS[0])], include_direct=False,
                         validator=check, transport_cooldown_seconds=0.02)
        leases = {rid: pool.acquire(rid, threading.Event()) for rid in ("a", "b", "c")}
        for rid, lease in leases.items():
            pool.report_failure(rid, lease, "transport")
        wait_until(lambda: pool.snapshot()["checks_total"] == 2)
        self.assertEqual(pool.snapshot()["available"], 0)
        self.assertEqual(pool.snapshot()["assigned"], 3)
        self.assertEqual(pool.snapshot()["routes"][0]["state"], "suspended")
        for rid, lease in leases.items():
            self.assertIs(pool.get(rid), lease)
        time.sleep(0.05)
        self.assertEqual(len(calls), 2)

    def test_confirmed_bad_exit_releases_only_next_failed_task_and_rebalances(self):
        for include_direct in (False, True):
            with self.subTest(include_direct=include_direct):
                calls = Counter()
                bad, alternate = ENDPOINTS[:2]
                def check(ep):
                    calls[ep] += 1
                    return ({"ok": False, "status_code": 502}
                            if ep == bad and calls[ep] > 1 else healthy(ep))
                pool = self.pool(imported_candidates=[record(bad)], include_direct=include_direct,
                                 validator=check, transport_cooldown_seconds=0.01)
                leases = {str(index): pool.acquire(str(index), threading.Event()) for index in range(6)}
                grouped = [(rid, lease) for rid, lease in leases.items() if lease.endpoint == bad]
                for rid, lease in grouped[:3]:
                    self.assertTrue(pool.report_failure(rid, lease, "transport"))
                wait_until(lambda: pool.snapshot()["checks_total"] == 2)
                pool.update_imported_candidates([record(bad), record(alternate)])
                wait_until(lambda: pool.snapshot()["checks_total"] == 3)

                failed_rid, failed_lease = grouped[0]
                self.assertFalse(pool.report_failure(failed_rid, failed_lease, "transport"))
                self.assertIsNone(pool.get(failed_rid))
                for rid, lease in leases.items():
                    if rid != failed_rid:
                        self.assertIs(pool.get(rid), lease, "healthy existing sockets must retain their lease")
                replacement = pool.acquire(failed_rid, threading.Event())
                self.assertNotEqual(replacement.endpoint, bad)
                if not include_direct:
                    self.assertEqual(replacement.kind, "proxy")
                    self.assertEqual(replacement.endpoint, alternate)
                self.assertFalse(pool.release(failed_rid, failed_lease))
                self.assertIs(pool.get(failed_rid), replacement)
                self.assertEqual(calls[bad], 2, "a suspended route must not start another probe for this failure")

    def test_confirmed_bad_exit_without_direct_or_other_proxy_waits_cancellably(self):
        calls = []
        def check(ep):
            calls.append(ep)
            return healthy(ep) if len(calls) == 1 else {"ok": False, "status_code": 502}
        pool = self.pool(imported_candidates=[record(ENDPOINTS[0])], include_direct=False,
                         validator=check, transport_cooldown_seconds=0.01)
        leases = {rid: pool.acquire(rid, threading.Event()) for rid in ("a", "b", "c")}
        for rid, lease in leases.items():
            pool.report_failure(rid, lease, "transport")
        wait_until(lambda: pool.snapshot()["checks_total"] == 2)
        self.assertFalse(pool.report_failure("a", leases["a"], "transport"))
        stop, results = threading.Event(), []
        thread = threading.Thread(target=lambda: results.append(pool.acquire("a", stop)))
        thread.start()
        wait_until(lambda: pool.snapshot()["waiting"] == 1)
        self.assertEqual(results, [])
        self.assertIs(pool.get("b"), leases["b"])
        stop.set()
        thread.join(0.25)
        self.assertEqual(results, [None])
        self.assertIsNone(pool.get("a"))

    def test_old_probe_result_cannot_overwrite_updated_credentials(self):
        started, finish = threading.Event(), threading.Event()
        urls = []
        def check(endpoint, proxy_url):
            urls.append(proxy_url)
            if "old-secret" in proxy_url:
                started.set()
                finish.wait(2)
                return {"ok": False, "status_code": 407}
            return healthy(endpoint)
        with patch("proxy_pool.validate_proxy", side_effect=check):
            pool = self.pool(imported_candidates=[record(ENDPOINTS[0], "old-secret")], validator=None)
            pool.refresh()
            self.assertTrue(started.wait(1))
            pool.update_imported_candidates([record(ENDPOINTS[0], "new-secret")])
            finish.set()
            wait_until(lambda: pool.snapshot()["checks_total"] == 2)
            lease = pool.acquire("a", threading.Event())
        self.assertIn("new-secret", lease.proxy_url)
        self.assertEqual(pool.snapshot()["failed"], 0)
        self.assertEqual(len(urls), 2)

    def test_shutdown_ignores_inflight_probe_and_wakes_exit_waiters(self):
        finish = threading.Event()
        pool = self.pool(imported_candidates=[record(ENDPOINTS[0])], warmup_seconds=0,
                         validator=lambda ep: (finish.wait(2), healthy(ep))[1])
        pool.refresh()
        wait_until(lambda: pool.snapshot()["checking"] == 1)
        direct = pool.acquire("a", threading.Event())
        direct.control.cooldown(10)
        values = []
        thread = threading.Thread(target=lambda: values.append(direct.control.acquire_request()))
        thread.start()
        self.assertFalse(pool.shutdown(0))
        thread.join(0.2)
        finish.set()
        self.assertTrue(pool.shutdown(2))
        self.assertEqual(values, [None])
        self.assertEqual(pool.snapshot()["assigned"], 0)
        self.assertEqual(pool.snapshot()["available"], 0)
        self.assertEqual(pool.snapshot()["routes"], [])
        self.assertFalse(pool.update_imported_candidates([record(ENDPOINTS[1])]))


if __name__ == "__main__":
    unittest.main()
