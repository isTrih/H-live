"""Offline route/lease integration tests; synthetic endpoints are never dialed."""
import os
from pathlib import Path
import ssl
import sys
import tempfile
import threading
import time
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python" / "collector"))
import liveMan as engine
from monitor import MonitorManager
from websocket._http import proxy_info
from websocket._url import get_proxy_info


def until(condition, timeout=2):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if condition():
            return
        time.sleep(.005)
    raise AssertionError("condition timed out")


def response(data=None):
    value = Mock(status_code=200, text='{"roomId":"222"}')
    value.cookies = {"ttwid": "synthetic"}
    value.json.return_value = data
    return value


class RoutingTests(unittest.TestCase):
    def test_metadata_visitor_room_and_websocket_keep_one_route_and_verified_tls(self):
        lease = SimpleNamespace(proxy_url="http://8.8.8.8:8080")
        metadata_session, socket_session = Mock(), Mock()
        metadata_session.cookies = socket_session.cookies = {}
        metadata_session.get.side_effect = [response(), response({"data": {"data": [
            {"status": 2, "id_str": "222", "owner": {"id_str": "111"}}]}})]
        socket_session.get.side_effect = [response(), response()]
        with patch.object(engine.requests, "Session", side_effect=[metadata_session, socket_session]):
            engine.fetch_room_info("123", proxy=lease)
            fetcher = engine.DouyinLiveWebFetcher("123", proxy=lease)
        socket = Mock()
        def run(**kwargs):
            fetcher._wsOnOpen(socket)
            fetcher.stop()
            return False
        socket.run_forever.side_effect = run
        with patch.object(engine, "generateSignature", return_value="synthetic"), \
                patch.object(engine.websocket, "WebSocketApp", return_value=socket):
            fetcher.start()
        for session in (metadata_session, socket_session):
            self.assertFalse(session.trust_env)
            self.assertTrue(session.verify)
            self.assertEqual({"http": lease.proxy_url, "https": lease.proxy_url}, session.proxies)
            self.assertEqual(2, session.get.call_count)
            for call in session.get.call_args_list:
                self.assertTrue(call.args[0].startswith("https://"))
                self.assertFalse(call.kwargs["allow_redirects"])
        kwargs = socket.run_forever.call_args.kwargs
        self.assertEqual("8.8.8.8", kwargs["http_proxy_host"])
        self.assertEqual(8080, kwargs["http_proxy_port"])
        self.assertEqual(ssl.CERT_REQUIRED, kwargs["sslopt"]["cert_reqs"])
        self.assertTrue(kwargs["sslopt"]["check_hostname"])

    def test_environment_cannot_override_proxy_or_direct_mode(self):
        environment = {"HTTP_PROXY": "http://wrong.invalid:90", "HTTPS_PROXY": "http://wrong.invalid:91",
                       "http_proxy": "http://wrong.invalid:92", "https_proxy": "http://wrong.invalid:93",
                       "NO_PROXY": "*", "no_proxy": "*"}
        with patch.dict(os.environ, environment):
            for route, expected in (("http://8.8.8.8:8080", ("8.8.8.8", 8080, None)), (None, (None, 0, None))):
                options = proxy_info(**engine._websocket_proxy_options(route))
                self.assertEqual(expected, get_proxy_info("webcast.douyin.com", True,
                    options.proxy_host, options.proxy_port, options.auth, options.no_proxy, options.proxy_protocol))
                session = Mock()
                engine._configure_session(session, route)
                self.assertFalse(session.trust_env)
                if route is None:
                    self.assertEqual({}, session.proxies)

    def test_redirects_plain_http_and_incomplete_proxy_credentials_are_rejected(self):
        session = Mock()
        session.get.return_value = response()
        session.get.return_value.status_code = 302
        with self.assertRaisesRegex(engine.LiveMonitorError, "重定向"):
            engine._http_get(session, "https://live.douyin.com/")
        session.get.reset_mock()
        with self.assertRaisesRegex(engine.LiveMonitorError, "未加密"):
            engine._http_get(session, "http://live.douyin.com/")
        session.get.assert_not_called()
        with self.assertRaises(engine.LiveMonitorError) as error:
            engine._proxy_url("http://secret@8.8.8.8:8080")
        self.assertNotIn("secret", str(error.exception))


class LeasePool:
    def __init__(self, empty=False):
        self.empty = empty
        self.calls, self.releases, self.invalidations = [], [], []
        self.assigned = {}
        self.lock = threading.Lock()

    def acquire(self, rid, stop_event):
        if self.empty:
            stop_event.wait(3)
            return None
        with self.lock:
            index = len(self.calls) + 1
            lease = SimpleNamespace(endpoint=f"8.8.8.{index}:8080", proxy_url=f"http://8.8.8.{index}:8080")
            self.calls.append((rid, lease))
            self.assigned[rid] = lease
            return lease

    def release(self, rid, lease=None):
        with self.lock:
            if self.assigned.get(rid) is lease:
                self.releases.append((rid, lease))
                del self.assigned[rid]

    def invalidate(self, rid, lease=None):
        self.invalidations.append((rid, lease))
        self.release(rid, lease)

    def snapshot(self):
        return {"assigned": len(self.assigned), "leases": []}

    def shutdown(self, timeout=5):
        return True


class Fetcher:
    def __init__(self, rid, on_message, cookie, on_status, proxy):
        self.proxy = proxy
        self.stop_event = threading.Event()
        self.on_status = on_status
    def start(self):
        self.on_status("connected", "fixture")
        self.stop_event.wait(3)
    def stop(self):
        self.stop_event.set()


class LeaseIntegrationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.created = []
        self.routes = []
    def make(self, pool, metadata=None, factory=None):
        def lookup(rid, cookie="", proxy=None):
            self.routes.append((rid, proxy))
            return {"is_live": True, "anchor_id": "111" + rid, "room_id": "222" + rid}
        def build(rid, **kwargs):
            value = Fetcher(rid, **kwargs)
            self.created.append(value)
            return value
        manager = MonitorManager(data_dir=self.directory.name, proxy_pool=pool,
            room_info_fetcher=metadata or lookup, fetcher_factory=factory or build,
            retry_seconds=.01, offline_seconds=.01, connect_interval_seconds=0)
        self.addCleanup(lambda: manager.shutdown(3))
        return manager

    def test_rooms_have_independent_leases_and_same_metadata_socket_route(self):
        pool = LeasePool()
        manager = self.make(pool)
        manager.start("1", "one")
        manager.start("2", "two")
        until(lambda: len(self.created) == 2)
        self.assertIsNot(self.created[0].proxy, self.created[1].proxy)
        self.assertEqual({id(route) for _, route in self.routes}, {id(value.proxy) for value in self.created})
        self.assertEqual(2, len({row["proxy"] for row in manager.snapshot().values()}))
        manager.stop_all()
        until(lambda: len(pool.releases) == 2)

    def test_empty_pool_waits_without_direct_requests_and_stop_cancels(self):
        pool = LeasePool(empty=True)
        manager = self.make(pool)
        manager.start("1", "one")
        until(lambda: manager.snapshot()["1"]["state"] == "waiting_proxy")
        self.assertEqual([], self.routes)
        self.assertEqual([], self.created)
        manager.stop("1")
        until(manager.can_configure_proxy)

    def test_offline_polling_keeps_lease_and_failure_rotates_before_lookup(self):
        pool = LeasePool()
        calls = []
        def lookup(rid, cookie, proxy):
            calls.append(proxy)
            if len(calls) <= 2:
                return {"is_live": False}
            if len(calls) == 3:
                raise engine.LiveMonitorError("模拟代理失败")
            return {"is_live": True, "anchor_id": "111", "room_id": "222"}
        manager = self.make(pool, lookup)
        manager.start("1", "one")
        until(lambda: len(self.created) == 1)
        self.assertIs(calls[0], calls[1])
        self.assertIs(calls[1], calls[2])
        self.assertIsNot(calls[2], calls[3])
        self.assertIs(self.created[0].proxy, calls[3])
        self.assertEqual(1, len(pool.invalidations))

    def test_stop_during_http_holds_lease_and_restart_waits_for_predecessor(self):
        pool = LeasePool()
        entered, release = threading.Event(), threading.Event()
        self.addCleanup(release.set)
        calls = []
        def lookup(rid, cookie, proxy):
            calls.append(proxy)
            if len(calls) == 1:
                entered.set()
                release.wait(2)
            return {"is_live": True, "anchor_id": "111", "room_id": "222"}
        manager = self.make(pool, lookup)
        manager.start("1", "one")
        self.assertTrue(entered.wait(1))
        manager.stop("1")
        self.assertEqual([], pool.releases)
        self.assertFalse(manager.can_configure_proxy())
        with self.assertRaisesRegex(RuntimeError, "停止全部"):
            manager.configure_proxy(None)
        manager.start("1", "one")
        time.sleep(.03)
        self.assertEqual(1, len(pool.calls))
        # Cancel the intermediate generation while the original HTTP call is
        # still blocked; the third generation must wait for the first too.
        intermediate = manager._rooms["1"].thread
        manager.stop("1")
        intermediate.join(.3)
        self.assertFalse(intermediate.is_alive())
        manager.start("1", "one")
        time.sleep(.03)
        self.assertEqual(1, len(pool.calls))
        release.set()
        until(lambda: len(self.created) == 1)
        self.assertEqual(2, len(pool.calls))
        self.assertEqual(1, len(pool.releases))
        self.assertIsNot(calls[0], calls[1])
        self.assertIs(pool.assigned["1"], calls[1])


if __name__ == "__main__":
    unittest.main()
