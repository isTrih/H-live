import json
from pathlib import Path
import sys
import threading
import time
import unittest
from unittest.mock import MagicMock, patch

import requests
import socks
from urllib3.exceptions import MaxRetryError, NewConnectionError, ProxyError as Urllib3ProxyError

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python" / "collector"))
from proxy_pool import API_URL, HEALTH_URL, ProxyFailureRegistry, ProxyPool, ProxyPoolError, fetch_candidates, normalize_endpoint, validate_proxy


def wait_until(predicate, timeout=3):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.005)
    raise AssertionError("condition did not become true")


def healthy(endpoint):
    return {"ok": True, "status_code": 200, "latency_ms": 12.3}


class ProxyPoolTests(unittest.TestCase):
    def setUp(self):
        self.pools = []

    def tearDown(self):
        for pool in self.pools:
            pool.shutdown(3)

    def pool(self, **options):
        options.setdefault("fetcher", lambda count: ["8.8.8.8:80", "1.1.1.1:8080", "9.9.9.9:3128"])
        options.setdefault("validator", healthy)
        pool = ProxyPool(**options)
        self.pools.append(pool)
        return pool

    def test_init_has_no_network_or_threads_until_demand(self):
        fetcher = MagicMock(return_value=[])
        pool = self.pool(fetcher=fetcher)
        self.assertEqual(pool.snapshot()["fetch_count"], 0)
        self.assertEqual(pool.snapshot()["total"], 0)
        fetcher.assert_not_called()
        self.assertIsNone(pool._thread)

    def test_nested_transport_auth_failures_have_fixed_redacted_health_error(self):
        connect_error = OSError('Tunnel connection failed: 407 Proxy Authentication Required')
        http_error = requests.exceptions.ProxyError(MaxRetryError(
            None, 'https://synthetic-user:synthetic-password@live.douyin.com/',
            Urllib3ProxyError('Unable to connect to proxy', connect_error)))
        socks_error = NewConnectionError(None, 'Failed to establish a new connection: SOCKS5 authentication failed')
        socks_error.__cause__ = socks.SOCKS5AuthError('SOCKS5 authentication failed')
        wrapped_socks = requests.exceptions.ConnectionError(MaxRetryError(
            None, 'https://synthetic-user:synthetic-password@live.douyin.com/', socks_error))
        for error in (http_error, wrapped_socks):
            with self.subTest(transport=type(error).__name__):
                def reject(endpoint, error=error):
                    raise error
                pool = self.pool(fetcher=lambda count: ['8.8.8.8:80'], validator=reject, refresh_max_requests=1)
                pool.refresh()
                wait_until(lambda: not pool.snapshot()['refresh']['active'])
                report = pool.snapshot()
                self.assertEqual(report['last_error'], '代理认证失败，请检查账号、密码或供应商访问权限')
                self.assertNotIn('synthetic-user', json.dumps(report))
                self.assertNotIn('synthetic-password', json.dumps(report))
                self.assertEqual(report['failed'], 1)

    def test_non_auth_transport_errors_are_not_misclassified_by_407_in_url_or_message(self):
        errors = [
            requests.exceptions.ProxyError(MaxRetryError(None, 'https://user:407@8.8.8.8:407/',
                Urllib3ProxyError('Unable to connect to proxy', OSError('Tunnel connection failed: 502 Bad Gateway')))),
            OSError('Tunnel connection failed: 4070 unrelated code'),
            socks.SOCKS5Error('0x01: General SOCKS server failure'),
        ]
        for error in errors:
            with self.subTest(error=type(error).__name__):
                def reject(endpoint, error=error):
                    raise error
                pool = self.pool(fetcher=lambda count: ['8.8.8.8:80'], validator=reject, refresh_max_requests=1)
                pool.refresh()
                wait_until(lambda: not pool.snapshot()['refresh']['active'])
                self.assertNotIn('代理认证失败', pool.snapshot()['last_error'])

    def test_http_407_health_response_has_fixed_auth_message(self):
        pool = self.pool(fetcher=lambda count: ['8.8.8.8:80'],
                         validator=lambda endpoint: {'ok': False, 'status_code': 407}, refresh_max_requests=1)
        pool.refresh()
        wait_until(lambda: not pool.snapshot()['refresh']['active'])
        self.assertEqual(pool.snapshot()['last_error'], '代理认证失败，请检查账号、密码或供应商访问权限')

    def test_public_numeric_addresses_only_and_valid_ports(self):
        self.assertEqual(normalize_endpoint("8.8.8.8:0080"), "8.8.8.8:80")
        self.assertEqual(normalize_endpoint("[2606:4700:4700::1111]:443"), "[2606:4700:4700::1111]:443")
        for endpoint in ("127.0.0.1:80", "192.168.1.1:80", "10.0.0.1:80", "169.254.169.254:80", "100.64.0.1:80",
                         "224.0.0.1:80", "0.0.0.0:80", "[::1]:80", "localhost:80", "user:pass@8.8.8.8:80",
                         "http://8.8.8.8:80", "8.8.8.8:0", "8.8.8.8:65536", "8.8.8.8:８０", "8.8.8.8:80/path", "8.8.8.8:80\n"):
            with self.subTest(endpoint=endpoint), self.assertRaises(ValueError):
                normalize_endpoint(endpoint)

    def test_concurrent_rooms_receive_distinct_stable_leases(self):
        pool = self.pool()
        leases = {}
        threads = [threading.Thread(target=lambda rid=rid: leases.setdefault(rid, pool.acquire(rid, threading.Event())))
                   for rid in ("111", "222", "333")]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(3)
            self.assertFalse(thread.is_alive())
        self.assertEqual(len({lease.endpoint for lease in leases.values()}), 3)
        self.assertIs(pool.acquire("111", threading.Event()), leases["111"])
        self.assertEqual(pool.snapshot()["assigned"], 3)
        self.assertEqual(pool.snapshot()["fetch_count"], 1)

    def test_different_ports_on_same_ip_cannot_be_leased_to_two_rooms(self):
        pool = self.pool(fetcher=lambda count: ["8.8.8.8:80", "8.8.8.8:8080", "1.1.1.1:80"])
        first = pool.acquire("111", threading.Event())
        second = pool.acquire("222", threading.Event())
        self.assertNotEqual(first.endpoint.split(":")[0], second.endpoint.split(":")[0])
        self.assertEqual(pool.snapshot()["checks_total"], 2)

    def test_invalidate_cools_bad_endpoint_and_stale_release_cannot_release_new_lease(self):
        pool = self.pool()
        old = pool.acquire("111", threading.Event())
        self.assertTrue(pool.invalidate("111", old))
        new = pool.acquire("111", threading.Event())
        self.assertNotEqual(old.endpoint, new.endpoint)
        self.assertFalse(pool.release("111", old))
        self.assertIs(pool.get("111"), new)
        self.assertEqual(pool.snapshot()["failed"], 1)
        self.assertTrue(pool.release("111", new))

    def test_ttl_expires_unused_candidates_but_does_not_interrupt_active_lease(self):
        pool = self.pool(ttl_seconds=0.02)
        lease = pool.acquire("111", threading.Event())
        time.sleep(0.04)
        self.assertIs(pool.acquire("111", threading.Event()), lease)
        self.assertEqual(pool.snapshot()["available"], 0)
        self.assertTrue(pool.release("111", lease))
        self.assertEqual(pool.snapshot()["total"], 0)

    def test_waiting_acquire_is_cancelled_without_fallback_or_fetch_loop(self):
        pool = self.pool(fetcher=lambda count: [])
        stop, results = threading.Event(), []
        thread = threading.Thread(target=lambda: results.append(pool.acquire("111", stop)))
        thread.start()
        wait_until(lambda: pool.snapshot()["fetch_count"] == 1)
        time.sleep(0.2)
        self.assertEqual(pool.snapshot()["fetch_count"], 1)
        begin = time.monotonic()
        stop.set()
        thread.join(0.25)
        self.assertFalse(thread.is_alive())
        self.assertLess(time.monotonic() - begin, 0.25)
        self.assertEqual(results, [None])

    def test_probes_never_exceed_twelve_and_shutdown_is_bounded(self):
        release = threading.Event()
        active, peak = 0, 0
        lock = threading.Lock()
        def blocking_probe(endpoint):
            nonlocal active, peak
            with lock:
                active += 1
                peak = max(peak, active)
            release.wait(3)
            with lock:
                active -= 1
            return healthy(endpoint)
        pool = self.pool(fetcher=lambda count: [f"8.8.4.{index}:80" for index in range(1, 21)], validator=blocking_probe)
        pool.refresh()
        try:
            wait_until(lambda: pool.snapshot()["checking"] == 12)
            self.assertEqual(pool.snapshot()["pending"], 8)
            begin = time.monotonic()
            self.assertFalse(pool.shutdown(0.02))
            self.assertLess(time.monotonic() - begin, 0.25)
            self.assertEqual(peak, 12)
        finally:
            release.set()
        self.assertTrue(pool.shutdown(3))
        self.assertEqual(pool.snapshot()["available"], 0)
        self.assertIsNone(pool.acquire("111", threading.Event()))

    def test_restricted_or_bad_candidates_never_become_assignable(self):
        pool = self.pool(fetcher=lambda count: ["8.8.8.8:80", "127.0.0.1:80", "not-address"],
                         validator=lambda endpoint: {"ok": True, "status_code": 403, "latency_ms": 5})
        pool.refresh()
        wait_until(lambda: pool.snapshot()["checks_total"] == 1)
        state = pool.snapshot()
        self.assertEqual(state["available"], 0)
        self.assertEqual(state["failed"], 1)
        self.assertIn("403", state["last_error"])

    def test_fetch_rate_has_minimum_one_second_and_error_text_is_safe(self):
        calls = []
        def fetcher(count):
            calls.append(time.monotonic())
            if len(calls) == 1:
                return ["8.8.8.8:80"]
            raise RuntimeError("private-request-cookie")
        pool = self.pool(fetcher=fetcher, request_interval_seconds=0.01)
        pool.refresh()
        wait_until(lambda: pool.snapshot()["available"] == 1)
        pool.refresh()
        wait_until(lambda: len(calls) == 2)
        self.assertGreaterEqual(calls[1] - calls[0], 0.99)
        self.assertNotIn("private-request-cookie", json.dumps(pool.snapshot()))
        self.assertIn("RuntimeError", pool.snapshot()["fetch_error"])

    def test_repeated_refresh_uses_the_fixed_source_without_switching_fetchers(self):
        with patch("proxy_pool.fetch_candidates", side_effect=[["8.8.8.8:80"], ["1.1.1.1:80"]]) as fetcher:
            pool = ProxyPool(validator=healthy)
            self.pools.append(pool)
            pool.refresh()
            wait_until(lambda: pool.snapshot()["available"] == 1)
            pool.refresh()
            wait_until(lambda: pool.snapshot()["available"] == 2)
            self.assertEqual([call.args for call in fetcher.call_args_list], [(20,), (20,)])
            self.assertTrue(all(not call.kwargs for call in fetcher.call_args_list))

    def test_manual_refresh_checks_fifty_new_candidates_and_waits_for_last_probe(self):
        requests, checked = [], []
        final_probe = threading.Event()
        def fetcher(count):
            offset = sum(requests)
            requests.append(count)
            return [f"8.8.4.{index}:80" for index in range(offset + 1, offset + count + 1)]
        def validator(endpoint):
            checked.append(endpoint)
            if endpoint == "8.8.4.50:80":
                final_probe.wait(5)
            return healthy(endpoint)
        pool = self.pool(fetcher=fetcher, validator=validator)
        self.assertTrue(pool.refresh())
        self.assertTrue(pool.snapshot()["refresh"]["active"])
        self.assertFalse(pool.refresh(), "repeated clicks must not queue another batch")
        try:
            wait_until(lambda: pool.snapshot()["refresh"]["checked"] == 49, timeout=4)
            progress = pool.snapshot()["refresh"]
            self.assertTrue(progress["active"])
            self.assertEqual(progress["state"], "running")
            self.assertEqual(progress["scheduled"], 50)
            self.assertEqual(requests, [20, 20, 10])
        finally:
            final_probe.set()
        wait_until(lambda: not pool.snapshot()["refresh"]["active"])
        state = pool.snapshot()
        self.assertEqual(state["refresh"]["state"], "completed")
        self.assertEqual(state["refresh"]["checked"], 50)
        self.assertEqual(state["refresh"]["passed"], 50)
        self.assertEqual(state["refresh"]["source_requests"], 3)
        self.assertEqual(len(set(checked)), 50)
        self.assertEqual(state["available"], 50)
        time.sleep(0.12)
        self.assertEqual(requests, [20, 20, 10])

    def test_failed_candidates_stay_excluded_after_detail_cache_eviction(self):
        checked = []
        old_failure = "8.12.0.1:80"
        pool = self.pool(fetcher=lambda count: [old_failure, "9.9.9.9:80"],
                         validator=lambda endpoint: checked.append(endpoint) or healthy(endpoint),
                         refresh_max_requests=1)
        with pool._condition:
            for index in range(1, 2051):
                pool._failure_locked(f"8.12.{index // 256}.{index % 256}:80", "failed")
            self.assertNotIn(old_failure, pool._failed)
        pool.refresh()
        wait_until(lambda: not pool.snapshot()["refresh"]["active"])
        self.assertEqual(checked, ["9.9.9.9:80"])
        self.assertEqual(pool.snapshot()["refresh"]["skipped_failed"], 1)
        self.assertEqual(pool.snapshot()["failed"], 2050)

    def test_shared_failure_registry_skips_prior_failure_in_new_pool_automatic_demand(self):
        registry = ProxyFailureRegistry()
        checked = []
        first = self.pool(fetcher=lambda count: ["8.8.8.8:80"],
                          validator=lambda endpoint: {"ok": False, "status_code": 403},
                          failure_registry=registry, refresh_max_requests=1)
        first.refresh()
        wait_until(lambda: not first.snapshot()["refresh"]["active"])
        first.shutdown()
        second = self.pool(fetcher=lambda count: ["8.8.8.8:80", "1.1.1.1:80"],
                           validator=lambda endpoint: checked.append(endpoint) or healthy(endpoint),
                           failure_registry=registry)
        lease = second.acquire("111", threading.Event())
        self.assertEqual(lease.endpoint, "1.1.1.1:80")
        self.assertEqual(checked, ["1.1.1.1:80"])
        self.assertEqual(second.snapshot()["failed"], 1)

    def test_next_manual_batch_skips_failed_and_existing_candidates(self):
        checked, requests = [], []
        def fetcher(count):
            requests.append(count)
            return ["8.8.8.8:80", "1.1.1.1:80"] if len(requests) == 1 else ["8.8.8.8:80", "1.1.1.1:80", "9.9.9.9:80"]
        def validator(endpoint):
            checked.append(endpoint)
            return {"ok": False, "status_code": 403} if endpoint == "8.8.8.8:80" else healthy(endpoint)
        pool = self.pool(fetcher=fetcher, validator=validator, refresh_max_requests=1)
        pool.refresh()
        wait_until(lambda: not pool.snapshot()["refresh"]["active"])
        self.assertTrue(pool.refresh())
        wait_until(lambda: not pool.snapshot()["refresh"]["active"])
        progress = pool.snapshot()["refresh"]
        self.assertEqual(checked, ["8.8.8.8:80", "1.1.1.1:80", "9.9.9.9:80"])
        self.assertEqual(progress["checked"], 1)
        self.assertEqual(progress["skipped_failed"], 1)
        self.assertEqual(progress["skipped_existing"], 1)

    def test_manual_refresh_source_shortage_finishes_at_request_budget(self):
        fetcher = MagicMock(return_value=[])
        pool = self.pool(fetcher=fetcher, refresh_max_requests=2)
        pool.refresh()
        wait_until(lambda: not pool.snapshot()["refresh"]["active"], timeout=3)
        progress = pool.snapshot()["refresh"]
        self.assertEqual(progress["state"], "partial")
        self.assertEqual(progress["source_requests"], 2)
        self.assertEqual(progress["checked"], 0)
        self.assertIn("不足", progress["detail"])
        time.sleep(0.12)
        self.assertEqual(fetcher.call_count, 2)

    def test_manual_refresh_source_deadline_does_not_wait_for_backoff(self):
        pool = self.pool(fetcher=lambda count: [], refresh_timeout_seconds=0.12)
        pool.refresh()
        wait_until(lambda: not pool.snapshot()["refresh"]["active"], timeout=1)
        progress = pool.snapshot()["refresh"]
        self.assertEqual(progress["state"], "partial")
        self.assertEqual(progress["source_requests"], 1)
        self.assertIn("时间上限", progress["detail"])

    def test_shutdown_wakes_waiting_rooms_without_assignment(self):
        pool = self.pool(fetcher=lambda count: [])
        results = []
        thread = threading.Thread(target=lambda: results.append(pool.acquire("111", threading.Event())))
        thread.start()
        wait_until(lambda: pool.snapshot()["waiting"] == 1)
        self.assertTrue(pool.shutdown(1))
        thread.join(0.25)
        self.assertEqual(results, [None])

    def test_fetch_uses_fixed_api_count_limit_no_environment_or_credentials(self):
        session, response = MagicMock(), MagicMock()
        session.__enter__.return_value = session
        session.get.return_value.__enter__.return_value = response
        response.status_code = 200
        response.iter_content.return_value = [b'{"code":200,"data":{"proxies":["8.8.8.8:80"]}}']
        with patch("proxy_pool.requests.Session", return_value=session):
            self.assertEqual(fetch_candidates(300), ["8.8.8.8:80"])
        args, options = session.get.call_args
        self.assertEqual(args, (API_URL,))
        self.assertEqual(options["params"], {"protocol": "https", "count": 20})
        self.assertIs(session.trust_env, False)
        self.assertIs(options["verify"], True)
        self.assertIs(options["allow_redirects"], False)
        self.assertNotIn("cookies", options)

    def test_https_probe_keeps_certificate_checks_and_explicit_proxy(self):
        session, response = MagicMock(), MagicMock()
        session.__enter__.return_value = session
        session.get.return_value.__enter__.return_value = response
        response.status_code = 200
        with patch("proxy_pool.requests.Session", return_value=session):
            result = validate_proxy("8.8.8.8:80")
        args, options = session.get.call_args
        self.assertTrue(result["ok"])
        self.assertEqual(result["status_code"], 200)
        self.assertEqual(args, (HEALTH_URL,))
        self.assertEqual(options["proxies"], {"http": "http://8.8.8.8:80", "https": "http://8.8.8.8:80"})
        self.assertEqual(options["timeout"], (3, 5))
        self.assertIs(options["verify"], True)
        self.assertIs(session.trust_env, False)
        self.assertNotIn("cookies", options)

    def test_redirect_is_not_a_successful_homepage_or_assignable_lease(self):
        session, response = MagicMock(), MagicMock()
        session.__enter__.return_value = session
        session.get.return_value.__enter__.return_value = response
        response.status_code = 302
        with patch("proxy_pool.requests.Session", return_value=session):
            result = validate_proxy("8.8.8.8:80")
        self.assertFalse(result["ok"])
        self.assertFalse(session.get.call_args.kwargs["allow_redirects"])
        pool = self.pool(validator=lambda _: {"ok": True, "status_code": 302})
        self.assertFalse(pool._check("8.8.8.8:80")[0])


if __name__ == "__main__":
    unittest.main()
