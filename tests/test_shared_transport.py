"""Offline shared-exit contracts: real admission/manager, synthetic transports."""
import json
from pathlib import Path
import sys
import tempfile
import threading
import time
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import requests
from urllib3.exceptions import MaxRetryError, ProxyError as UrllibProxyError

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python" / "collector"))
import liveMan as engine
from monitor import MonitorManager
from proxy_import import make_record
from proxy_pool import ProxyPool
from route_control import RouteControl


def until(predicate, timeout=3):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if predicate():
            return
        time.sleep(.005)
    raise AssertionError("condition timed out")


def response(data=None):
    value = Mock(status_code=200, text='{"roomId":"222"}')
    value.cookies = {"ttwid": "synthetic-visitor"}
    value.json.return_value = data
    return value


class EngineExitTests(unittest.TestCase):
    def test_all_four_http_requests_share_control_but_wss_heartbeat_and_ack_do_not(self):
        control = Mock()
        control.acquire_request.return_value = release = Mock()
        lease = SimpleNamespace(proxy_url="http://8.8.8.8:80", kind="proxy", control=control)
        first, second = Mock(), Mock()
        first.cookies = second.cookies = {}
        first.get.side_effect = [response(), response({"data": {"data": [
            {"status": 2, "id_str": "222", "owner": {"id_str": "111"}}]}})]
        second.get.side_effect = [response(), response()]
        stopped = threading.Event()
        with patch.object(engine.requests, "Session", side_effect=[first, second]):
            engine.fetch_room_info("123", proxy=lease, stop_event=stopped)
            fetcher = engine.DouyinLiveWebFetcher("123", proxy=lease)
        self.addCleanup(fetcher.stop)
        socket = Mock()
        socket.run_forever.return_value = False
        with patch.object(engine, "generateSignature", return_value="synthetic"), \
                patch.object(engine.websocket, "WebSocketApp", return_value=socket):
            fetcher._connectWebSocket()
        self.assertEqual(control.acquire_request.call_count, 4)
        self.assertEqual(release.call_count, 4)
        fetcher._sendHeartbeatOnce()
        with patch.object(engine, "decode_frame", return_value=(b"ack", "[]")):
            fetcher._wsOnMessage(socket, b"frame")
        self.assertEqual(control.acquire_request.call_count, 4)
        self.assertEqual(socket.run_forever.call_args.kwargs["http_proxy_host"], "8.8.8.8")
        self.assertEqual(first.proxies, second.proxies)
        self.assertIsNot(first, second)
        self.assertEqual(control.acquire_request.call_args_list[0].args, (stopped,))

    def test_stop_during_metadata_homepage_does_not_issue_enter_request(self):
        stop = threading.Event()
        control = Mock()
        control.acquire_request.return_value = release = Mock()
        session = Mock()
        def get(*args, **kwargs):
            stop.set()
            return response()
        session.get.side_effect = get
        with patch.object(engine.requests, "Session", return_value=session):
            with self.assertRaises(engine.LiveMonitorError) as caught:
                engine.fetch_room_info("123", traffic_control=control, stop_event=stop)
        self.assertEqual(caught.exception.kind, "cancelled")
        self.assertEqual(session.get.call_count, 1)
        release.assert_called_once()
        session.close.assert_called_once()

    def test_waiting_request_cancels_without_network_or_leaking_permit(self):
        control = RouteControl(request_limit=1, request_interval_seconds=0)
        holding = control.acquire_request()
        stop, done = threading.Event(), threading.Event()
        session, errors = Mock(), []
        def run():
            try:
                engine._http_get(session, "https://live.douyin.com/", traffic_control=control, stop_event=stop)
            except engine.LiveMonitorError as error:
                errors.append(error.kind)
            finally:
                done.set()
        thread = threading.Thread(target=run)
        thread.start()
        stop.set()
        self.assertTrue(done.wait(.5))
        thread.join()
        session.get.assert_not_called()
        self.assertEqual(errors, ["cancelled"])
        self.assertEqual(control.snapshot()["request_inflight"], 1)
        holding()
        self.assertEqual(control.snapshot()["request_inflight"], 0)

    def test_http_errors_expose_only_safe_kind_status_and_stage(self):
        for stage in ("visitor", "room_info", "room_page"):
            session = Mock()
            status = requests.Response()
            status.status_code = 444
            session.get.return_value.raise_for_status.side_effect = requests.HTTPError("synthetic-secret", response=status)
            with self.assertRaises(engine.LiveMonitorError) as caught:
                engine._http_get(session, "https://live.douyin.com/", stage=stage)
            error = caught.exception
            self.assertEqual((error.kind, error.http_status, error.stage), ("target_http", 444, stage))
            self.assertNotIn("synthetic-secret", str(error))
        inner = OSError("Tunnel connection failed: 407 Proxy Authentication Required")
        session.get.side_effect = requests.exceptions.ProxyError(MaxRetryError(None, "https://live.douyin.com/",
            UrllibProxyError("Unable to connect to proxy", inner)))
        with self.assertRaises(engine.LiveMonitorError) as caught:
            engine._http_get(session, "https://live.douyin.com/", stage="room_info")
        self.assertEqual(caught.exception.kind, "proxy_auth")

    def test_websocket_rejection_and_auth_have_distinct_safe_kinds(self):
        from python_socks import ProxyError
        cases = [
            (engine.websocket.WebSocketBadStatusException("synthetic-secret", status_code=444), "target_http", 444),
            (engine.websocket.WebSocketProxyException("failed CONNECT via proxy status: 407"), "proxy_auth", 0),
            (ProxyError("Username and password authentication failure"), "proxy_auth", 0),
            (engine.websocket.WebSocketProxyException("synthetic-secret:407 connection refused"), "transport", 0),
        ]
        for source, kind, code in cases:
            fetcher = engine.DouyinLiveWebFetcher("123")
            try:
                error = fetcher._record_error(source)
                self.assertEqual((error.kind, error.http_status, error.stage), (kind, code, "websocket_handshake"))
                self.assertNotIn("synthetic-secret", str(error))
            finally:
                fetcher.stop()

    def test_cancel_signature_wait_never_constructs_v8_or_steals_a_slot(self):
        slots = threading.BoundedSemaphore(1)
        slots.acquire()
        stop, errors = threading.Event(), []
        def run():
            try:
                engine.generateSignature("wss://example.invalid/", stop_event=stop)
            except engine.LiveMonitorError as error:
                errors.append(error.kind)
        with patch.object(engine, "_SIGNATURE_SLOTS", slots), patch.object(engine, "_generate_signature") as sign:
            thread = threading.Thread(target=run)
            thread.start()
            stop.set()
            thread.join(.5)
            self.assertFalse(thread.is_alive())
            self.assertEqual(errors, ["cancelled"])
            sign.assert_not_called()
            self.assertFalse(slots.acquire(blocking=False))
        slots.release()

    def test_explicit_direct_lease_ignores_environment_and_keeps_its_control(self):
        control = RouteControl()
        lease = SimpleNamespace(kind="direct", proxy_url="", endpoint="direct", control=control)
        fetcher = engine.DouyinLiveWebFetcher("123", proxy=lease)
        try:
            self.assertEqual(fetcher.proxy_url, "")
            self.assertEqual(fetcher.session.proxies, {})
            self.assertFalse(fetcher.session.trust_env)
            self.assertIs(fetcher.traffic_control, control)
            self.assertEqual(engine._websocket_proxy_options(lease)["http_no_proxy"], ["*"])
        finally:
            fetcher.stop()


class SyntheticFetcher:
    def __init__(self, rid, on_message, cookie, on_status, proxy=None, traffic_control=None):
        self.rid, self.on_message, self.on_status = rid, on_message, on_status
        self.cookie, self.proxy, self.control = cookie, proxy, traffic_control
        self.closed = threading.Event()
        self.failure = None

    def start(self):
        self.on_status("connected", "合成连接")
        self.closed.wait()
        if self.failure is not None:
            raise self.failure

    def stop(self):
        self.closed.set()

    def emit(self, content):
        self.on_message("chat", {"content": content, "sender_id": "9007199254740993", "sender_name": "合成用户",
                                 "room_id": "2" + self.rid, "event_time": int(time.time())})


class SharedManagerTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)

    def test_fifty_rooms_share_four_exits_global_eight_is_only_a_handshake_limit(self):
        records = [make_record({"ip": ip, "port": 80}) for ip in ("8.8.8.8", "1.1.1.1", "9.9.9.9")]
        pool = ProxyPool(imported_candidates=records, validator=lambda _: {"ok": True, "status_code": 200},
                         shared=True, include_direct=True, warmup_seconds=0, target_cooldown_seconds=.3,
                         route_request_interval_seconds=0)
        self.addCleanup(pool.shutdown)
        pool.refresh()
        until(lambda: pool.snapshot()["available"] == 3)
        lock, metadata_gate = threading.Lock(), threading.Event()
        created, entered, routes = {}, [], {}
        def info(rid, cookie="", proxy=None, traffic_control=None, stop_event=None):
            with lock:
                entered.append(rid)
                routes[rid] = proxy
            metadata_gate.wait(3)
            return {"is_live": True, "anchor_id": "1" + rid, "room_id": "2" + rid}
        def factory(rid, on_message, cookie, on_status, proxy=None, traffic_control=None):
            fetcher = SyntheticFetcher(rid, on_message, cookie, on_status, proxy, traffic_control)
            with lock:
                created.setdefault(rid, []).append(fetcher)
            return fetcher
        manager = MonitorManager(data_dir=self.directory.name, proxy_pool=pool, room_info_fetcher=info,
                                 fetcher_factory=factory, connect_interval_seconds=0, retry_seconds=.01)
        self.addCleanup(lambda: manager.shutdown(3))
        self.addCleanup(metadata_gate.set)
        for n in range(50):
            manager.start(str(100 + n), "合成主播", cookie=f"fixture-cookie-{n}")
        until(lambda: len(entered) == 8)
        self.assertEqual(manager.metrics()["connecting_inflight"], 8)
        time.sleep(.03)
        self.assertEqual(len(entered), 8)
        metadata_gate.set()
        until(lambda: len(manager.snapshot()) == 50 and all(row["state"] == "connected" for row in manager.snapshot().values()))
        self.assertEqual(manager.metrics()["connecting_inflight"], 0)
        self.assertEqual(pool.snapshot()["assigned"], 50)
        distribution = {}
        for rid, lease in routes.items():
            distribution[lease.route_id] = distribution.get(lease.route_id, 0) + 1
            self.assertIs(created[rid][0].proxy, lease)
            self.assertIs(created[rid][0].control, lease.control)
            self.assertEqual(created[rid][0].cookie, f"fixture-cookie-{int(rid) - 100}")
            self.assertEqual(manager.snapshot()[rid]["route_id"], lease.route_id)
        self.assertEqual(len(distribution), 4)
        self.assertLessEqual(max(distribution.values()) - min(distribution.values()), 1)
        bad = "100"
        lease = routes[bad]
        peers = [rid for rid, item in routes.items() if item.route_id == lease.route_id and rid != bad]
        created[bad][0].failure = engine.LiveMonitorError("抖音请求失败：HTTP 444", kind="target_http",
                                                       http_status=444, stage="websocket_handshake")
        created[bad][0].closed.set()
        until(lambda: lease.control.snapshot()["cooldown_seconds"] > 0)
        self.assertIs(pool.get(bad), lease)
        for rid in peers:
            self.assertFalse(created[rid][0].closed.is_set())
            created[rid][0].emit("其他房间继续收到内容")
            self.assertEqual(manager.snapshot()[rid]["message_count"], 1)
        until(lambda: len(created[bad]) == 2)
        self.assertIs(created[bad][1].proxy, lease)
        self.assertEqual(pool.snapshot()["failed"], 0)
        self.assertTrue(manager.shutdown(3))
        self.assertEqual(pool.snapshot()["assigned"], 0)
        for item in routes.values():
            self.assertEqual(item.control.snapshot()["connecting"], 0)

    def test_direct_http_444_cools_new_work_and_stop_cancels_wait(self):
        calls = []
        def info(rid, cookie="", traffic_control=None, stop_event=None):
            calls.append((rid, traffic_control, stop_event))
            raise engine.LiveMonitorError("HTTP 444", kind="target_http", http_status=444, stage="room_info")
        manager = MonitorManager(data_dir=self.directory.name, room_info_fetcher=info,
                                 fetcher_factory=SyntheticFetcher, retry_seconds=.005, connect_interval_seconds=0)
        self.addCleanup(lambda: manager.shutdown(2))
        manager.start("100", "合成主播")
        until(lambda: manager._direct_control.snapshot()["cooldown_seconds"] > 0)
        manager.start("101", "另一个房间")
        time.sleep(.025)
        self.assertEqual(len(calls), 1)
        self.assertIs(calls[0][1], manager._direct_control)
        status = manager.proxy_snapshot()
        self.assertEqual(status["assigned"], 0)
        self.assertEqual(status["routes"][0]["assigned_tasks"], 2)
        self.assertEqual(status["routes"][0]["state"], "cooldown")
        self.assertEqual([item["rid"] for item in status["leases"]], ["100", "101"])
        self.assertTrue(all(item["proxy"] == "direct" for item in status["leases"]))
        started = time.monotonic()
        self.assertTrue(manager.shutdown(1))
        self.assertLess(time.monotonic() - started, .6)
        self.assertEqual(manager._direct_control.snapshot()["connecting"], 0)
        self.assertEqual(manager.metrics()["connecting_inflight"], 0)
        self.assertEqual(manager.proxy_snapshot()["routes"][0]["assigned_tasks"], 0)

    def test_normal_shared_websocket_close_reuses_lease_without_failure_report(self):
        pool = ProxyPool(imported_candidates=[], shared=True, include_direct=True, warmup_seconds=0)
        self.addCleanup(pool.shutdown)
        created = []
        def info(rid, cookie="", proxy=None):
            return {"is_live": True, "anchor_id": "111", "room_id": "2123"}
        def factory(rid, on_message, cookie, on_status, proxy=None, traffic_control=None):
            result = SyntheticFetcher(rid, on_message, cookie, on_status, proxy, traffic_control)
            created.append(result)
            return result
        manager = MonitorManager(data_dir=self.directory.name, proxy_pool=pool, room_info_fetcher=info,
                                 fetcher_factory=factory, retry_seconds=.005, connect_interval_seconds=0)
        self.addCleanup(lambda: manager.shutdown(2))
        with patch.object(pool, "report_failure", wraps=pool.report_failure) as report:
            manager.start("123", "合成主播")
            until(lambda: len(created) == 1)
            first_lease = created[0].proxy
            created[0].closed.set()
            until(lambda: len(created) == 2)
            self.assertIs(created[1].proxy, first_lease)
            report.assert_not_called()

    def test_normal_close_replaces_retired_route_without_stopping_its_live_peer(self):
        original = make_record({"ip": "8.8.8.8", "port": 80})
        replacement = make_record({"ip": "1.1.1.1", "port": 80})
        pool = ProxyPool(imported_candidates=[original], shared=True, include_direct=False,
                         validator=lambda _: {"ok": True, "status_code": 200},
                         warmup_seconds=0, route_request_interval_seconds=0)
        self.addCleanup(pool.shutdown)
        pool.refresh()
        until(lambda: pool.snapshot()["available"] == 1)
        created = {"123": [], "124": []}

        def info(rid, cookie="", proxy=None):
            return {"is_live": True, "anchor_id": "1" + rid, "room_id": "2" + rid}

        def factory(rid, on_message, cookie, on_status, proxy=None, traffic_control=None):
            result = SyntheticFetcher(rid, on_message, cookie, on_status, proxy, traffic_control)
            created[rid].append(result)
            return result

        manager = MonitorManager(data_dir=self.directory.name, proxy_pool=pool, room_info_fetcher=info,
                                 fetcher_factory=factory, retry_seconds=.005, connect_interval_seconds=0)
        self.addCleanup(lambda: manager.shutdown(2))
        manager.start("123", "正在重连的房间")
        manager.start("124", "保持在线的房间")
        until(lambda: all(row["state"] == "connected" for row in manager.snapshot().values()))
        old_lease, live_peer = created["123"][0].proxy, created["124"][0]
        pool.update_imported_candidates([replacement])
        until(lambda: pool.snapshot()["available"] == 1)
        self.assertIs(pool.get("123"), old_lease)
        self.assertIs(pool.get("124"), live_peer.proxy)
        self.assertFalse(live_peer.closed.is_set())

        created["123"][0].closed.set()
        until(lambda: len(created["123"]) == 2)
        renewed = created["123"][1].proxy
        self.assertEqual(renewed.endpoint, replacement["endpoint"])
        self.assertIsNot(renewed, old_lease)
        self.assertIs(pool.get("124"), live_peer.proxy)
        self.assertFalse(live_peer.closed.is_set())
        live_peer.emit("旧出口上的健康连接继续收消息")
        self.assertEqual(manager.snapshot()["124"]["message_count"], 1)
        self.assertTrue(manager.shutdown(2))
        self.assertEqual(pool.snapshot()["assigned"], 0)

    def test_offline_poll_uses_updated_credentials_on_its_next_attempt(self):
        original = make_record({"ip": "8.8.8.8", "port": 80,
                                "username": "fixture", "password": "old-fixture"})
        replacement = make_record({"ip": "8.8.8.8", "port": 80,
                                   "username": "fixture", "password": "new-fixture"})
        pool = ProxyPool(imported_candidates=[original], shared=True, include_direct=False,
                         validator=lambda _: {"ok": True, "status_code": 200},
                         warmup_seconds=0, route_request_interval_seconds=0)
        self.addCleanup(pool.shutdown)
        pool.refresh()
        until(lambda: pool.snapshot()["available"] == 1)
        observed, resume, routes = threading.Event(), threading.Event(), []

        def info(rid, cookie="", proxy=None):
            routes.append(proxy)
            if len(routes) == 1:
                observed.set()
                resume.wait(2)
            return {"is_live": False}

        factory = Mock(side_effect=AssertionError("离线房间不得建立 WSS"))
        manager = MonitorManager(data_dir=self.directory.name, proxy_pool=pool, room_info_fetcher=info,
                                 fetcher_factory=factory, offline_seconds=.01, connect_interval_seconds=0)
        self.addCleanup(lambda: manager.shutdown(2))
        self.addCleanup(resume.set)
        manager.start("123", "未开播房间")
        self.assertTrue(observed.wait(1))
        old_lease = routes[0]
        pool.update_imported_candidates([replacement])
        until(lambda: pool.snapshot()["available"] == 1)
        self.assertIs(pool.get("123"), old_lease)
        resume.set()
        until(lambda: len(routes) >= 2)
        self.assertIsNot(routes[1], old_lease)
        self.assertEqual(routes[1].endpoint, old_lease.endpoint)
        self.assertEqual(routes[1].proxy_url, "http://fixture:new-fixture@8.8.8.8:80")
        self.assertEqual(manager.snapshot()["123"]["state"], "offline")
        factory.assert_not_called()
        self.assertTrue(manager.shutdown(2))
        self.assertEqual(pool.snapshot()["assigned"], 0)
        self.assertEqual(routes[1].control.snapshot()["connecting"], 0)

    def test_cancel_while_waiting_global_gate_returns_held_exit_permit(self):
        entered, resume = threading.Event(), threading.Event()
        def info(rid, cookie=""):
            entered.set()
            resume.wait(2)
            return {"is_live": False}
        manager = MonitorManager(data_dir=self.directory.name, room_info_fetcher=info,
                                 fetcher_factory=SyntheticFetcher, max_connecting=1,
                                 connect_interval_seconds=0)
        self.addCleanup(lambda: manager.shutdown(2))
        self.addCleanup(resume.set)
        manager.start("100", "占用全局名额")
        self.assertTrue(entered.wait(1))
        manager.start("101", "排队房间")
        until(lambda: manager._direct_control.snapshot()["connecting"] == 2)
        self.assertEqual(manager.metrics()["connecting_inflight"], 1)
        manager.stop("101")
        until(lambda: manager._direct_control.snapshot()["connecting"] == 1)
        self.assertEqual(manager.metrics()["connecting_pending"], 0)
        resume.set()
        until(lambda: manager._direct_control.snapshot()["connecting"] == 0)


if __name__ == "__main__":
    unittest.main()
