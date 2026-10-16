"""Offline sidecar protocol, responsiveness and credential-boundary tests."""
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python" / "collector"))
from backend_service import BackendService, EVENT_CAPACITY, ServiceError, run_service
from app_storage import load_config, save_config
from message_export import ExportCancelled, ExportDataError
from proxy_pool import ProxyPool


def wait_for(condition, timeout=2):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if condition():
            return
        time.sleep(0.01)
    raise AssertionError("condition not reached")


class FakeArchive:
    def list_anchors(self):
        return [{"anchor_id": "100000003000", "anchor_name": "archived", "web_rid": "100000000000"}]

    def metrics(self):
        return {"queue_depth": 0, "pending_requests": 0}


class FakeManager:
    def __init__(self, on_event=None, data_dir=None, proxy_pool=None):
        self.on_event = on_event
        self.archive = FakeArchive()
        self.rows = {}
        self.started = []
        self.stopped = []
        self.start_gate = None
        self.closed = False
        self.proxy_pool = proxy_pool

    def can_configure_proxy(self):
        return not any(row.get("active") for row in self.rows.values())

    def configure_proxy(self, pool):
        if self.proxy_pool is not None:
            self.proxy_pool.shutdown(timeout=0)
        self.proxy_pool = pool

    def start(self, rid, name, cookie=""):
        if self.start_gate is not None:
            self.start_gate.wait(2)
        self.started.append((rid, name, cookie))
        self.rows[rid] = {"state": "connecting", "active": True, "message_count": 0,
                          "is_live": None, "live_checked_at": ""}
        return True

    def stop(self, rid):
        self.stopped.append(rid)
        if rid in self.rows:
            self.rows[rid].update(state="stopped", active=False)
        return True

    def rename(self, rid, name):
        if rid in self.rows:
            self.rows[rid]["name"] = name

    def snapshot(self):
        return {rid: dict(value) for rid, value in self.rows.items()}

    def shutdown(self, timeout=15):
        if self.start_gate:
            self.start_gate.set()
        self.closed = True
        if self.proxy_pool is not None:
            self.proxy_pool.shutdown(timeout=0)
        return True


class BackendTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        # These are fixtures for task, capacity and per-room credential tests,
        # not product defaults for a newly installed application.
        save_config({"anchors": {"100000000000": "测试主播一", "100000001000": "测试主播二",
                                 "100000002000": "测试主播三"},
                     "first_run_complete": True}, self.directory.name)
        self.service = BackendService(self.directory.name, manager_factory=FakeManager)
        self.addCleanup(self.service.shutdown)

    def test_init_loads_saved_anchor_fixtures_and_never_starts_network(self):
        result = self.service.dispatch("init")
        self.assertEqual("0.2.4", result["version"])
        self.assertEqual("0.2.4", result["native_version"])
        self.assertEqual("rust", result["processing"])
        self.assertEqual(3, len(result["anchors"]))
        self.assertEqual([], self.service.manager.started)
        self.assertTrue(all(row["is_live"] is None and row["state"] == "stopped" for row in result["anchors"]))
        self.assertFalse(result["settings"]["cookie_configured"])
        self.assertTrue(result["settings"]["proxy"]["supported"])
        self.assertTrue(result["settings"]["proxy_enabled"])
        self.assertEqual(0, result["metrics"]["proxy"]["fetch_count"])

    def test_update_readiness_idle_is_read_only(self):
        result = self.service.dispatch("update_readiness")
        self.assertEqual({"ready": True, "reason": "ready", "detail": "监控、导出和待写入操作均已结束，可以更新"}, result)
        self.assertEqual([], self.service.manager.started)
        self.assertEqual([], self.service.manager.stopped)
        self.assertFalse(self.service._closed.is_set())
        self.assertEqual(0, self.service.dispatch("proxy_status")["fetch_count"])

    def test_update_readiness_blocks_dequeued_operation_then_active_monitor(self):
        gate = self.service.manager.start_gate = threading.Event()
        self.addCleanup(gate.set)
        self.service.dispatch("start", {"rids": ["100000000000"]})
        wait_for(lambda: self.service._operations.qsize() == 0)
        self.assertEqual([], self.service.manager.started)
        result = self.service.dispatch("update_readiness")
        self.assertFalse(result["ready"])
        self.assertEqual("pending", result["reason"])
        gate.set()
        wait_for(lambda: self.service._operations.unfinished_tasks == 0)
        self.assertEqual("monitoring", self.service.dispatch("update_readiness")["reason"])
        self.service.dispatch("stop", {"rids": ["100000000000"]})
        wait_for(lambda: self.service._operations.unfinished_tasks == 0)
        self.assertTrue(self.service.dispatch("update_readiness")["ready"])

    def test_update_readiness_blocks_residual_monitor_thread(self):
        # No active snapshot row, but a previous worker has not finished cleanup.
        with patch.object(self.service.manager, "can_configure_proxy", return_value=False):
            self.assertEqual("monitoring", self.service.dispatch("update_readiness")["reason"])
        self.assertEqual({}, self.service.manager.snapshot())

    def test_update_readiness_blocks_export_status_and_live_finalizer(self):
        finished = threading.Event()
        thread = threading.Thread(target=finished.wait, daemon=True)
        thread.start()
        self.addCleanup(thread.join, 2)
        self.addCleanup(finished.set)
        job = {"status": "completed", "thread": thread, "cancel": threading.Event()}
        self.service._jobs["fixture"] = job
        self.assertEqual("exporting", self.service.dispatch("update_readiness")["reason"])
        self.assertFalse(job["cancel"].is_set())
        finished.set()
        thread.join(1)
        for status in ("running", "queued"):
            job["status"] = status
            self.assertEqual("exporting", self.service.dispatch("update_readiness")["reason"])
        job["status"] = "completed"
        self.assertTrue(self.service.dispatch("update_readiness")["ready"])

    def test_update_readiness_blocks_inflight_archive_and_unconfirmed_metrics(self):
        for metrics in ({"queue_depth": 0, "pending_requests": 1},
                        {"queue_depth": 1, "pending_requests": 1},
                        {"queue_depth": 0}, None):
            with self.subTest(metrics=metrics), patch.object(self.service.manager.archive, "metrics", return_value=metrics):
                result = self.service.dispatch("update_readiness")
                self.assertFalse(result["ready"])
                self.assertEqual("pending", result["reason"])
        with patch.object(self.service.manager.archive, "metrics", side_effect=RuntimeError("synthetic-secret")):
            self.assertNotIn("synthetic-secret", json.dumps(self.service.dispatch("update_readiness")))
        self.assertTrue(self.service.dispatch("update_readiness")["ready"])

    def test_update_readiness_waits_for_config_write(self):
        with self.service._persist_lock:
            self.assertEqual("pending", self.service.dispatch("update_readiness")["reason"])
        self.assertTrue(self.service.dispatch("update_readiness")["ready"])

    def test_shutdown_does_not_claim_live_export_finalizer_has_stopped(self):
        finished = threading.Event()
        thread = threading.Thread(target=finished.wait, daemon=True)
        thread.start()
        self.addCleanup(thread.join, 2)
        self.addCleanup(finished.set)
        self.service._jobs["fixture"] = {"status": "completed", "thread": thread, "cancel": threading.Event()}
        result = self.service.shutdown()
        self.assertTrue(thread.is_alive())
        self.assertTrue(result["closed"])
        self.assertFalse(result["all_workers_stopped"])

    def test_ring_is_bounded_and_cursor_preserves_pagination(self):
        for number in range(EVENT_CAPACITY + 100):
            self.service._receive_event({"kind": "message", "rid": "1", "content": str(number)})
        result = self.service.dispatch("poll", {"after": 0, "limit": 300})
        self.assertEqual(2000, len(self.service._events))
        self.assertEqual(100, result["dropped"])
        self.assertEqual(101, result["events"][0]["seq"])
        self.assertEqual(400, result["cursor"])
        following = self.service.dispatch("poll", {"after": result["cursor"], "limit": 300})
        self.assertEqual(401, following["events"][0]["seq"])
        self.assertEqual(0, following["dropped"])

    def test_poll_limits_are_enforced(self):
        for params in ({"limit": 301}, {"limit": 0}, {"limit": True}, {"after": -1}, {"after": "1"}):
            with self.subTest(params=params), self.assertRaises(ServiceError):
                self.service.dispatch("poll", params)

    def test_start_and_poll_remain_fast_while_operation_is_blocked(self):
        gate = self.service.manager.start_gate = threading.Event()
        self.addCleanup(gate.set)
        started = time.monotonic()
        self.service.dispatch("start", {"rids": ["100000000000"]})
        self.service.dispatch("poll", {"after": 0})
        self.assertLess(time.monotonic() - started, 0.2)
        self.assertEqual([], self.service.manager.started)
        gate.set()
        wait_for(lambda: bool(self.service.manager.started))

    def test_batch_input_handles_markdown_concatenated_urls_and_invalid_items(self):
        result = self.service.dispatch("add_anchors", {"values": "[room](https://live.douyin.com/123)https://www.douyin.com/follow/live/456?anchor_id=999\n789\nnot-a-room"})
        self.assertEqual(["123", "456", "789"], result["added"])
        self.assertEqual(1, len(result["invalid"]))
        self.assertNotIn("not-a-room", json.dumps(result))

    def test_task_capacity_is_enforced_without_partial_add(self):
        with self.assertRaises(ServiceError):
            self.service.dispatch("add_anchors", {"values": "\n".join(str(number) for number in range(1, 499))})
        self.assertEqual(3, len(self.service.dispatch("init")["anchors"]))

    def test_cookie_is_write_only_and_room_override_is_used(self):
        def fake_crypt(data, decrypt=False):
            return data[::-1]
        with patch("app_storage._crypt_cookie", side_effect=fake_crypt):
            self.service.dispatch("set_cookie", {"cookie": "global=synthetic-secret"})
            self.service.dispatch("set_cookie", {"rid": "100000000000", "cookie": "room=synthetic-secret"})
            for method in ("init", "poll"):
                serialized = json.dumps(self.service.dispatch(method))
                self.assertNotIn("synthetic-secret", serialized)
                self.assertNotIn("cookie_encrypted", serialized)
            self.service.dispatch("start", {"rids": ["100000000000", "100000001000"]})
            wait_for(lambda: len(self.service.manager.started) == 2)
            self.assertEqual("room=synthetic-secret", self.service.manager.started[0][2])
            self.assertEqual("global=synthetic-secret", self.service.manager.started[1][2])
            self.service.dispatch("clear_cookie", {"rid": "100000000000"})
            self.assertFalse(self.service._anchors()[0]["own_cookie_configured"])
            self.assertTrue(self.service._anchors()[0]["cookie_configured"])
            self.service.dispatch("clear_cookie")
        disk = Path(self.directory.name, "config.json").read_text(encoding="utf-8")
        self.assertNotIn("synthetic-secret", disk)

    def test_invalid_cookie_errors_do_not_echo_credentials(self):
        with self.assertRaises(ServiceError) as result:
            self.service.dispatch("set_cookie", {"cookie": "secret=value\r\nforged: header"})
        self.assertNotIn("secret=value", str(result.exception))

    def test_remove_preserves_historical_anchor_choices(self):
        self.service.dispatch("remove", {"rids": ["100000000000"]})
        wait_for(lambda: "100000000000" in self.service.manager.stopped)
        self.assertEqual(2, len(self.service.dispatch("init")["anchors"]))
        self.assertEqual("100000003000", self.service.dispatch("archive_anchors")[0]["anchor_id"])

    def test_connection_state_does_not_fabricate_broadcast_state(self):
        rid = "100000000000"
        self.service.manager.rows[rid] = {"state": "retrying", "is_live": True, "live_checked_at": "2026-10-14T20:00:00+08:00"}
        row = self.service.dispatch("poll")["anchors"][0]
        self.assertTrue(row["is_live"])
        self.assertEqual("retrying", row["state"])
        self.assertEqual("2026-10-14T20:00:00+08:00", row["live_checked_at"])

    def test_dashboard_receives_last_message_and_archive_queue(self):
        self.service.manager.rows["100000000000"] = {"last_message_at": "2026-10-14T20:01:00+08:00",
                                                       "broadcast_started_at": 1800000000, "viewer_count": 42,
                                                       "duration_started_at": 1800000100, "duration_source": "monitored",
                                                       "duration_seconds": 3720.5, "duration_running": True,
                                                       "duration_updated_at": 1800003820.5}
        self.service.manager.metrics = lambda: {"archive": {"queue_depth": 12}}
        result = self.service.dispatch("poll")
        self.assertEqual("2026-10-14T20:01:00+08:00", result["anchors"][0]["last_message_at"])
        self.assertEqual(12, result["metrics"]["archive_queue"])
        self.assertEqual(1800000000, result["anchors"][0]["broadcast_started_at"])
        self.assertIsNone(result["anchors"][1]["broadcast_started_at"])
        self.assertEqual(42, result["anchors"][0]["viewer_count"])
        self.assertIsNone(result["anchors"][1]["viewer_count"])
        self.assertEqual(1800000100, result["anchors"][0]["duration_started_at"])
        self.assertEqual("monitored", result["anchors"][0]["duration_source"])
        self.assertIsNone(result["anchors"][1]["duration_started_at"])
        self.assertEqual(3720.5, result["anchors"][0]["duration_seconds"])
        self.assertIs(result["anchors"][0]["duration_running"], True)
        self.assertEqual(1800003820.5, result["anchors"][0]["duration_updated_at"])
        self.assertIsNone(result["anchors"][1]["duration_seconds"])
        self.assertIs(result["anchors"][1]["duration_running"], False)

    def test_proxy_status_refresh_and_stopped_only_configuration(self):
        result = self.service.dispatch("proxy_status")
        self.assertTrue(result["enabled"])
        self.assertEqual([], result["leases"])
        self.assertEqual(0, result["fetch_count"])
        with patch.object(self.service._proxy_pool, "refresh") as refresh:
            self.assertTrue(self.service.dispatch("refresh_proxies")["accepted"])
            refresh.assert_called_once()
        with patch.object(self.service._proxy_pool, "refresh", return_value=False):
            self.assertFalse(self.service.dispatch("refresh_proxies")["accepted"])
        self.service.dispatch("start", {"rids": ["100000000000"]})
        wait_for(lambda: bool(self.service.manager.started))
        with self.assertRaisesRegex(ServiceError, "停止全部"):
            self.service.dispatch("configure_proxy", {"enabled": False})
        self.service.dispatch("stop", {"rids": ["100000000000"]})
        wait_for(lambda: self.service._operations.unfinished_tasks == 0)
        result = self.service.dispatch("configure_proxy", {"enabled": False})
        self.assertFalse(result["enabled"])
        self.assertTrue(result["restart_required"])
        self.assertFalse(self.service.dispatch("init")["settings"]["proxy_enabled"])
        self.assertFalse(self.service.dispatch("poll")["metrics"]["proxy"]["enabled"])
        self.assertTrue(self.service.dispatch("configure_proxy", {"enabled": True})["enabled"])
        self.assertEqual(0, self.service.dispatch("proxy_status")["fetch_count"])

    def test_direct_mode_can_warm_pool_without_rerouting_and_enable_reuses_candidates(self):
        self.service.dispatch("configure_proxy", {"enabled": False})
        created, requests = [], []
        def fetch(count):
            requests.append(count)
            return [f"8.8.{len(requests)}.{index}:8080" for index in range(1, count + 1)]
        def factory():
            pool = ProxyPool(fetcher=fetch,
                             validator=lambda endpoint: {"ok": True, "status_code": 200, "latency_ms": 1})
            created.append(pool)
            return pool
        self.service._proxy_pool_factory = factory
        self.assertIsNone(self.service._prepared_proxy_pool)
        self.service.dispatch("start", {"rids": ["100000000000"]})
        wait_for(lambda: bool(self.service.manager.started))
        self.assertTrue(self.service.dispatch("refresh_proxies")["accepted"])
        wait_for(lambda: (status := self.service.dispatch("proxy_status"))["available"] == 50 and not status["refresh"]["active"], timeout=5)
        self.assertIsNone(self.service.manager.proxy_pool)
        self.assertFalse(self.service.dispatch("proxy_status")["enabled"])
        self.assertFalse(self.service.dispatch("init")["settings"]["proxy_enabled"])
        self.assertEqual(50, self.service.dispatch("poll")["metrics"]["proxy"]["available"])
        with self.assertRaisesRegex(ServiceError, "停止全部"):
            self.service.dispatch("configure_proxy", {"enabled": True})
        self.service.dispatch("stop", {"rids": ["100000000000"]})
        wait_for(lambda: self.service._operations.unfinished_tasks == 0)
        self.service.dispatch("configure_proxy", {"enabled": True})
        self.assertIs(self.service.manager.proxy_pool, created[0])
        self.assertIsNone(self.service._prepared_proxy_pool)
        self.assertEqual(1, len(created))
        self.assertEqual(50, self.service.dispatch("proxy_status")["available"])
        self.assertEqual(50, sum(requests))
        self.assertEqual(50, self.service.dispatch("proxy_status")["refresh"]["checked"])
        self.assertTrue(self.service.dispatch("refresh_proxies")["accepted"])
        wait_for(lambda: self.service.dispatch("proxy_status")["available"] == 100, timeout=5)
        self.assertEqual(100, sum(requests))

    def test_direct_exit_load_remains_visible_while_proxy_candidates_are_prepared(self):
        self.service.dispatch("configure_proxy", {"enabled": False})
        direct = {"kind": "direct", "route_id": "direct", "endpoint": "direct",
                  "assigned_tasks": 2, "connecting": 1, "request_inflight": 1,
                  "state": "cooldown", "cooldown_seconds": 12}
        leases = [{"rid": "100", "proxy": "direct"}, {"rid": "101", "proxy": "direct"}]
        self.service.manager.proxy_snapshot = lambda: {"routes": [direct], "leases": leases}
        before = self.service.dispatch("proxy_status")
        self.assertEqual(before["routes"], [direct])
        self.assertEqual(before["leases"], leases)
        prepared = ProxyPool(imported_candidates=[], shared=True, include_direct=True)
        self.service._prepared_proxy_pool = prepared
        with patch.object(prepared, "snapshot", return_value={"available": 3, "routes": [
                {"kind": "direct", "assigned_tasks": 0}, {"kind": "proxy", "endpoint": "8.8.8.8:80"}], "leases": []}):
            warmed = self.service.dispatch("proxy_status")
        self.assertEqual(warmed["routes"], [direct, {"kind": "proxy", "endpoint": "8.8.8.8:80"}])
        self.assertEqual(warmed["leases"], leases)
        self.assertEqual(warmed["available"], 3)
        self.assertFalse(warmed["enabled"])

    def test_shutdown_cleans_a_warmed_but_disabled_pool(self):
        self.service.dispatch("configure_proxy", {"enabled": False})
        self.service._proxy_pool_factory = lambda: ProxyPool(
            fetcher=lambda count: [], validator=lambda endpoint: {})
        self.service.dispatch("refresh_proxies")
        pool = self.service._prepared_proxy_pool
        self.assertIsNone(self.service.manager.proxy_pool)
        self.assertTrue(self.service.shutdown()["all_workers_stopped"])
        self.assertTrue(pool._closed)
        self.assertFalse(pool._thread.is_alive())
        self.assertIsNone(self.service._prepared_proxy_pool)

    def test_failed_addresses_stay_excluded_when_mode_switch_creates_a_new_pool(self):
        calls, checked = [], []
        bad = "9.9.9.9:8080"
        def fetch(count):
            calls.append(count)
            prefix = [bad] if count > 1 else []
            return prefix + [f"8.7.{len(calls)}.{index}:8080" for index in range(1, count - len(prefix) + 1)]
        def validate(endpoint):
            checked.append(endpoint)
            return {"ok": endpoint != bad, "status_code": 200 if endpoint != bad else 403, "latency_ms": 1}
        self.service._proxy_pool._fetcher = fetch
        self.service._proxy_pool._validator = validate
        default_factory = self.service._proxy_pool_factory
        def factory():
            pool = default_factory()  # Retains the service's real shared registry.
            pool._fetcher, pool._validator = fetch, validate
            return pool
        self.service._proxy_pool_factory = factory
        self.service.dispatch("refresh_proxies")
        wait_for(lambda: not self.service.dispatch("proxy_status")["refresh"]["active"], timeout=5)
        self.assertEqual(1, checked.count(bad))
        self.service.dispatch("configure_proxy", {"enabled": False})
        self.service.dispatch("refresh_proxies")
        wait_for(lambda: not self.service.dispatch("proxy_status")["refresh"]["active"], timeout=5)
        status = self.service.dispatch("proxy_status")
        self.assertFalse(status["enabled"])
        self.assertEqual(50, status["refresh"]["scheduled"])
        self.assertEqual(50, status["available"])
        self.assertGreaterEqual(status["refresh"]["skipped_failed"], 1)
        self.assertEqual(1, checked.count(bad))

    def test_proxy_configuration_rejects_queued_start_and_invalid_enabled(self):
        gate = self.service.manager.start_gate = threading.Event()
        self.addCleanup(gate.set)
        self.service.dispatch("start", {"rids": ["100000000000"]})
        with self.assertRaisesRegex(ServiceError, "停止全部"):
            self.service.dispatch("configure_proxy", {"enabled": False})
        with self.assertRaisesRegex(ServiceError, "布尔值"):
            self.service.dispatch("configure_proxy", {"enabled": "false"})
        gate.set()

    def test_readded_room_can_persist_the_same_identity_again(self):
        event = {"kind": "status", "rid": "100000000000", "name": "fixture", "anchor_id": "100000003000", "session_id": "7696456021958609673"}
        self.service._receive_event(event)
        self.service._flush_metadata()
        self.service.dispatch("remove", {"rids": [event["rid"]]})
        self.service.dispatch("add_anchors", {"values": event["rid"]})
        self.service._receive_event(event)
        self.service._flush_metadata()
        self.assertEqual(event["anchor_id"], self.service.config["anchor_metadata"][event["rid"]]["anchor_id"])

    def test_remove_and_immediate_readd_start_keep_operation_order(self):
        rid = "100000000000"
        self.service.dispatch("start", {"rids": [rid]})
        wait_for(lambda: len(self.service.manager.started) == 1)
        entered, release = threading.Event(), threading.Event()
        self.addCleanup(release.set)
        original_stop = self.service.manager.stop
        def blocked_stop(value):
            entered.set()
            release.wait(2)
            return original_stop(value)
        self.service.manager.stop = blocked_stop
        self.service.dispatch("remove", {"rids": [rid]})
        self.assertTrue(entered.wait(1))
        self.assertNotIn(rid, [row["rid"] for row in self.service.dispatch("poll")["anchors"]])
        self.service.dispatch("add_anchors", {"values": rid})
        self.service.dispatch("start", {"rids": [rid]})
        self.assertEqual(1, len(self.service.manager.started))
        release.set()
        wait_for(lambda: len(self.service.manager.started) == 2)
        row = next(row for row in self.service.dispatch("poll")["anchors"] if row["rid"] == rid)
        self.assertTrue(row["active"])
        self.assertEqual("connecting", row["state"])

    def test_metadata_is_saved_off_callback_and_does_not_repeat_unchanged(self):
        event = {"kind": "status", "rid": "100000000000", "name": "fixture", "anchor_id": "100000003000", "session_id": "7696456021958609673"}
        with patch.object(self.service, "_update_config", wraps=self.service._update_config) as save:
            self.service._receive_event(event)
            save.assert_not_called()
            self.service._flush_metadata()
            save.assert_called_once()
            self.service._receive_event(event)
            self.service._flush_metadata()
            save.assert_called_once()
        disk = json.loads(Path(self.directory.name, "config.json").read_text(encoding="utf-8"))
        self.assertEqual("100000003000", disk["anchor_metadata"]["100000000000"]["anchor_id"])

    def test_export_is_background_and_can_be_cancelled(self):
        entered = threading.Event()
        def exporter(*args, cancel_event, **kwargs):
            entered.set()
            wait_for(cancel_event.is_set)
            raise ExportCancelled()
        self.service._export_function = exporter
        result = self.service.dispatch("export", {"anchor_ids": ["100000003000"], "month": "2026-10",
            "types": ["chat"], "format": "xlsx", "output_path": str(Path(self.directory.name, "test.xlsx"))})
        self.assertTrue(entered.wait(1))
        self.assertEqual("running", self.service.dispatch("poll")["export_jobs"][0]["status"])
        self.service.dispatch("cancel_export", {"job_id": result["job_id"]})
        wait_for(lambda: self.service._export_jobs()[0]["status"] == "cancelled")

    def test_export_data_error_keeps_actionable_message_but_other_errors_are_sanitized(self):
        for failure, expected in ((ExportDataError("第 2 条消息的“消息内容”超过 Excel 单元格 32767 字符限制；请改用 CSV。"), "请改用 CSV"),
                                  (RuntimeError("cookie=synthetic-secret"), "RuntimeError")):
            def exporter(*args, **kwargs):
                raise failure
            self.service._export_function = exporter
            job_id = self.service.dispatch("export", {"anchor_ids": ["100000003000"], "month": "2026-10",
                "types": ["chat"], "format": "xlsx", "output_path": str(Path(self.directory.name, "test.xlsx"))})["job_id"]
            wait_for(lambda: self.service._jobs[job_id]["status"] == "error")
            result = next(job for job in self.service.dispatch("poll")["export_jobs"] if job["id"] == job_id)
            self.assertIn(expected, result["error"])
            self.assertNotIn("synthetic-secret", result["error"])

    def test_protocol_recovers_after_invalid_json_and_shuts_down(self):
        source = io.StringIO('not-json\n{"id":1,"method":"unknown"}\n{"id":2,"method":"init"}\n{"id":3,"method":"shutdown"}\n')
        output = io.StringIO()
        run_service(source, output, factory=lambda: self.service)
        rows = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual([False, False, True, True], [row["ok"] for row in rows])
        self.assertEqual(3, rows[-1]["id"])
        self.assertTrue(rows[-1]["result"]["closed"])
        self.assertTrue(self.service.manager.closed)


class ColdStartTests(unittest.TestCase):
    def test_new_install_has_no_anchors_and_stays_empty_after_restart(self):
        with tempfile.TemporaryDirectory() as directory:
            # A fresh installation has no adjacent legacy configuration. Make
            # that boundary explicit instead of consulting this checkout.
            missing_legacy = Path(directory, "no-legacy-config.json")
            with patch("backend_service.load_config", side_effect=lambda data_dir: load_config(data_dir, missing_legacy)):
                for _ in range(2):
                    service = BackendService(directory, manager_factory=FakeManager)
                    try:
                        result = service.dispatch("init")
                        self.assertEqual([], result["anchors"])
                        self.assertEqual([], service.manager.started)
                        self.assertEqual(0, result["metrics"]["proxy"]["fetch_count"])
                        self.assertEqual({}, load_config(directory, missing_legacy)["anchors"])
                    finally:
                        self.assertTrue(service.shutdown()["all_workers_stopped"])

    def test_saved_anchors_are_preserved_with_or_without_first_run_flag(self):
        anchors = {"100000000000": "已保存的自定义昵称", "100000001000": "另一个已保存主播", "222": "自选主播"}
        for flag in (None, False, True):
            with self.subTest(first_run_complete=flag), tempfile.TemporaryDirectory() as directory:
                config = {"anchors": anchors}
                if flag is not None:
                    config["first_run_complete"] = flag
                save_config(config, directory)
                service = BackendService(directory, manager_factory=FakeManager)
                try:
                    rows = service.dispatch("init")["anchors"]
                    self.assertEqual(anchors, {row["rid"]: row["name"] for row in rows})
                    self.assertEqual(anchors, load_config(directory)["anchors"])
                    self.assertEqual([], service.manager.started)
                finally:
                    self.assertTrue(service.shutdown()["all_workers_stopped"])

    def test_valid_legacy_configs_keep_saved_anchors_or_an_intentionally_empty_list(self):
        for anchors in ({}, {"100000002000": "原来保存的主播", "333": "自定义主播"}):
            with self.subTest(anchors=anchors), tempfile.TemporaryDirectory() as directory:
                legacy = Path(directory, "legacy.json")
                original = json.dumps({"anchors": anchors, "extra_setting": "preserved"}, ensure_ascii=False).encode("utf-8")
                legacy.write_bytes(original)
                data = Path(directory, "new-app")
                with patch("backend_service.load_config", side_effect=lambda data_dir: load_config(data_dir, legacy)):
                    service = BackendService(data, manager_factory=FakeManager)
                    try:
                        rows = service.dispatch("init")["anchors"]
                        self.assertEqual(anchors, {row["rid"]: row["name"] for row in rows})
                        migrated = load_config(data, legacy)
                        self.assertEqual(anchors, migrated["anchors"])
                        self.assertEqual("preserved", migrated["extra_setting"])
                        self.assertEqual(original, legacy.read_bytes())
                        self.assertEqual([], service.manager.started)
                    finally:
                        self.assertTrue(service.shutdown()["all_workers_stopped"])


class ProcessSmokeTests(unittest.TestCase):
    def test_real_sidecar_uses_only_json_stdout_and_exits_without_network(self):
        root = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory() as directory:
            environment = dict(os.environ, DOUYIN_MONITOR_HOME=directory, PYTHONIOENCODING="utf-8")
            requests = [{"id": 1, "method": "init"}, {"id": 2, "method": "poll", "params": {"after": 0}},
                        {"id": 3, "method": "update_readiness"}, {"id": 4, "method": "shutdown"}]
            process = subprocess.run([sys.executable, str(root / "python" / "collector" / "backend_service.py")],
                input="".join(json.dumps(row) + "\n" for row in requests), text=True, encoding="utf-8",
                capture_output=True, timeout=20, env=environment, cwd=directory)
            self.assertEqual(0, process.returncode, process.stderr)
            rows = [json.loads(line) for line in process.stdout.splitlines()]
            self.assertEqual(4, len(rows))
            self.assertTrue(all(row["ok"] for row in rows))
            self.assertEqual([], rows[0]["result"]["anchors"])
            self.assertEqual([], rows[1]["result"]["anchors"])
            self.assertEqual(0, rows[1]["result"]["metrics"]["active"])
            self.assertEqual("ready", rows[2]["result"]["reason"])
            self.assertTrue(rows[2]["result"]["ready"])
            self.assertTrue(rows[3]["result"]["all_workers_stopped"])
            self.assertTrue(Path(directory, "messages.sqlite3").exists())


if __name__ == "__main__":
    unittest.main()
