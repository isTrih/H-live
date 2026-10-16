"""Native archive + real manager + RPC boundaries, with no account or network."""
from contextlib import closing
import json
import queue
from pathlib import Path
import sqlite3
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

from app_storage import load_config, save_config
from backend_service import BackendService, ServiceError, run_service
from monitor import MonitorManager


def until(predicate, timeout=3):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(.005)
    raise AssertionError("condition not reached")


def record(anchor="11", session="101", ident="1", sent="2026-10-01T00:00:00+08:00"):
    return {"anchor_id": anchor, "anchor_name": "合成主播", "web_rid": "111",
            "session_id": session, "sender_id": "12345678901234567890", "sender_name": "测试用户",
            "message_type": "chat", "content": "合成内容", "message_time": sent,
            "received_at": "2026-10-31T23:59:59.999+08:00", "time_source": "server" if sent else "missing",
            "message_id": ident, "payload": {}}


class OfflineFetcher:
    def __init__(self, rid, on_message, cookie, on_status):
        self.rid, self.on_status = rid, on_status
        self.closed = threading.Event()
        self.exit_gate = threading.Event()
        self.exit_gate.set()

    def start(self):
        self.on_status("connected", "synthetic")
        self.closed.wait(5)
        self.exit_gate.wait(5)

    def stop(self):
        self.closed.set()


class ArchiveDeleteTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name)
        self.network = patch("requests.Session", side_effect=AssertionError("network forbidden"))
        self.network_mock = self.network.start()
        self.addCleanup(self.network.stop)
        config = load_config(data_dir=self.path)
        config.update(first_run_complete=True, anchors={"111": "主播A", "222": "主播B"})
        config["proxy_pool"]["enabled"] = False
        save_config(config, self.path)
        self.fetchers = {}
        self.info_gate = threading.Event()
        self.info_gate.set()
        def info(rid, cookie=""):
            self.info_gate.wait(5)
            return {"is_live": True, "anchor_id": "11" if rid == "111" else "22", "room_id": "101"}
        def factory(*args, **kwargs):
            fetcher = OfflineFetcher(*args, **kwargs)
            self.fetchers[fetcher.rid] = fetcher
            return fetcher
        def manager_factory(**kwargs):
            return MonitorManager(**kwargs, room_info_fetcher=info, fetcher_factory=factory,
                                  connect_interval_seconds=0)
        self.service = BackendService(self.path, manager_factory=manager_factory)
        self.addCleanup(self.cleanup_service)
        for row in (record(), record(ident="2", sent="2026-11-01T00:00:00+08:00"),
                    record(session="102", ident="3", sent=""), record(anchor="22", ident="4")):
            self.assertTrue(self.service.manager.archive.record_message(row))

    def cleanup_service(self):
        self.info_gate.set()
        for fetcher in self.fetchers.values():
            fetcher.exit_gate.set()
        self.service.shutdown()
        self.network_mock.assert_not_called()

    def rpc(self, method, **params):
        return self.service.dispatch(method, params)

    def delete(self, **params):
        accepted = self.rpc("execute_archive_delete", **params)
        self.assertTrue(accepted["accepted"])
        until(lambda: self.rpc("archive_delete_status", job_id=accepted["job_id"])["status"] != "running")
        status = self.rpc("archive_delete_status", job_id=accepted["job_id"])
        self.assertEqual(status["status"], "completed", status)
        return status["report"]

    def test_bridge_preview_intersection_and_cleanup_do_not_touch_other_data(self):
        exported = self.path / "already-exported.csv"
        exported.write_text("existing export", encoding="utf-8")
        scope = {"anchor_ids": ["11"], "month": "2026-10", "session_ids": ["101"]}
        preview = self.rpc("preview_archive_delete", **scope)
        self.assertEqual((preview["records_count"], preview["sessions_count"], preview["ready"]), (1, 1, True))
        self.assertEqual(self.delete(**scope),
                         {"deleted_records": 1, "deleted_sessions": 0, "remaining_records": 3})
        self.assertEqual(self.rpc("preview_archive_delete", anchor_ids=["22"])["records_count"], 1)
        self.assertEqual(self.rpc("preview_archive_delete", anchor_ids=["11"], month="2026-11")["records_count"], 1)
        self.assertEqual(exported.read_text(encoding="utf-8"), "existing export")
        self.assertEqual(set(self.service.config["anchors"]), {"111", "222"})
        self.assertEqual(self.delete(anchor_ids=["11"])["deleted_sessions"], 2)
        self.assertEqual([row["anchor_id"] for row in self.rpc("archive_anchors")], ["22"])
        with closing(sqlite3.connect(self.path / "messages.sqlite3")) as connection:
            self.assertEqual(connection.execute("PRAGMA foreign_key_check").fetchall(), [])
        self.assertTrue(self.service.manager.archive.record_message(record()))

    def test_rpc_never_accepts_implicit_all_or_misspelled_filter(self):
        for scope in ({}, {"anchor_ids": []}, {"anchor_ids": "11"}, {"anchor_ids": ["0"]},
                      {"anchor_ids": [True]}, {"anchor_ids": ["11"], "month": "2026-13"},
                      {"anchor_ids": ["11"], "session_ids": []}, {"anchor_ids": ["11"], "session_id": "101"}):
            for method in ("preview_archive_delete", "execute_archive_delete"):
                with self.subTest(scope=scope, method=method), self.assertRaises(ServiceError):
                    self.service.dispatch(method, scope)
        self.assertEqual(self.rpc("preview_archive_delete", anchor_ids=["11", "22"])["records_count"], 4)

    def test_sessions_use_month_filter_and_explicit_pagination(self):
        one = self.rpc("archive_sessions", anchor_ids=["11"], month="2026-10", limit=1)
        two = self.rpc("archive_sessions", anchor_ids=["11"], month="2026-10", limit=1, offset=one["next_offset"])
        self.assertEqual(one["total"], 2)
        self.assertIsNone(two["next_offset"])
        self.assertNotEqual(one["sessions"][0]["session_id"], two["sessions"][0]["session_id"])
        self.assertIsNone(one["sessions"][0]["started_at"])
        with self.assertRaises(ServiceError):
            self.rpc("archive_sessions", anchor_ids=["11"], limit=True)

    def test_execution_rechecks_monitor_started_after_preview(self):
        self.assertTrue(self.rpc("preview_archive_delete", anchor_ids=["11"])["ready"])
        self.rpc("start", rids=["111"])
        until(lambda: self.service.manager.snapshot().get("111", {}).get("state") == "connected")
        with self.assertRaisesRegex(ServiceError, "停止"):
            self.rpc("execute_archive_delete", anchor_ids=["11"])
        self.assertEqual(self.rpc("preview_archive_delete", anchor_ids=["11"])["records_count"], 3)
        # Known unrelated live tasks do not block deleting this other anchor.
        self.assertEqual(self.delete(anchor_ids=["22"])["deleted_records"], 1)

    def test_unknown_identity_and_stopped_but_lingering_thread_block(self):
        self.info_gate.clear()
        self.rpc("start", rids=["222"])
        until(lambda: "222" in self.service.manager.snapshot())
        self.assertFalse(self.rpc("preview_archive_delete", anchor_ids=["11"])["ready"])
        self.info_gate.set()
        until(lambda: self.service.manager.snapshot()["222"]["state"] == "connected")
        self.assertTrue(self.rpc("preview_archive_delete", anchor_ids=["11"])["ready"])
        self.fetchers["222"].exit_gate.clear()
        self.rpc("stop", rids=["222"])
        until(lambda: not self.service.manager.snapshot()["222"]["active"] and not self.service._operations.unfinished_tasks)
        self.assertFalse(self.rpc("preview_archive_delete", anchor_ids=["22"])["ready"])
        self.fetchers["222"].exit_gate.set()
        until(lambda: self.service.manager.can_delete_archive(["22"]))
        self.assertTrue(self.rpc("preview_archive_delete", anchor_ids=["22"])["ready"])

    def test_related_export_and_completed_but_alive_export_thread_block(self):
        end = threading.Event()
        thread = threading.Thread(target=lambda: end.wait(5))
        thread.start()
        self.addCleanup(thread.join)
        self.addCleanup(end.set)
        self.service._jobs["synthetic"] = {"id": "synthetic", "status": "completed", "anchor_ids": ["11"],
                                             "thread": thread, "cancel": end}
        self.assertFalse(self.rpc("preview_archive_delete", anchor_ids=["11"])["ready"])
        with self.assertRaisesRegex(ServiceError, "导出"):
            self.rpc("execute_archive_delete", anchor_ids=["11"])
        self.assertEqual(self.delete(anchor_ids=["22"])["deleted_records"], 1)
        end.set(); thread.join()
        self.assertTrue(self.rpc("preview_archive_delete", anchor_ids=["11"])["ready"])

    def test_async_delete_returns_fast_and_rejects_new_start_export_until_done(self):
        entered, release = threading.Event(), threading.Event()
        self.addCleanup(release.set)
        original = self.service.manager.archive.execute_delete
        def held(selection, cancel_token=None):
            entered.set()
            if not release.wait(3):
                raise RuntimeError("test timeout")
            return original(selection, cancel_token)
        with patch.object(self.service.manager.archive, "execute_delete", side_effect=held):
            before = time.monotonic()
            accepted = self.rpc("execute_archive_delete", anchor_ids=["11"])
            self.assertLess(time.monotonic() - before, .3)
            self.assertTrue(entered.wait(1))
            for method, params in (("start", {"rids": ["111"]}),
                                   ("export", {"anchor_ids": ["11"], "month": "2026-10", "output_path": str(self.path / "after.csv")}),
                                   ("execute_archive_delete", {"anchor_ids": ["22"]})):
                before = time.monotonic()
                with self.assertRaisesRegex(ServiceError, "删除仍在执行"):
                    self.service.dispatch(method, params)
                self.assertLess(time.monotonic() - before, .3)
            self.rpc("poll")
            self.assertFalse(self.rpc("update_readiness")["ready"])
            self.assertEqual(self.rpc("archive_delete_status", job_id=accepted["job_id"])["status"], "running")
            release.set()
            until(lambda: self.rpc("archive_delete_status", job_id=accepted["job_id"])["status"] == "completed")
        self.assertEqual(self.rpc("preview_archive_delete", anchor_ids=["11"])["records_count"], 0)

    def test_queued_operation_and_queued_export_block_and_failure_releases_gate(self):
        # unfinished_tasks counts an operator's in-progress call, not just its queue.
        entered, release = threading.Event(), threading.Event()
        self.addCleanup(release.set)
        def held_start(*args, **kwargs):
            entered.set(); release.wait(3)
        with patch.object(self.service.manager, "start", side_effect=held_start):
            self.rpc("start", rids=["111"])
            self.assertTrue(entered.wait(1))
            with self.assertRaisesRegex(ServiceError, "排队或执行"):
                self.rpc("execute_archive_delete", anchor_ids=["11"])
            release.set()
            until(lambda: not self.service._operations.unfinished_tasks)
        self.service._jobs["queued"] = {"status": "queued", "anchor_ids": ["11"], "cancel": threading.Event()}
        with self.assertRaisesRegex(ServiceError, "导出"):
            self.rpc("execute_archive_delete", anchor_ids=["11"])
        self.service._jobs.clear()
        with patch.object(self.service.manager.archive, "execute_delete", side_effect=RuntimeError("synthetic")):
            accepted = self.rpc("execute_archive_delete", anchor_ids=["11"])
            until(lambda: self.rpc("archive_delete_status", job_id=accepted["job_id"])["status"] == "error")
        self.assertFalse(self.service._archive_delete_in_progress)
        self.assertTrue(self.rpc("preview_archive_delete", anchor_ids=["11"])["ready"])

    def test_manual_pc_marker_is_persistent_independent_and_removed_with_anchor(self):
        result = self.rpc("set_pc_unsupported", rid="111", unsupported=True)
        self.assertEqual(result["pc_support_source"], "manual")
        row = next(row for row in self.rpc("init")["anchors"] if row["rid"] == "111")
        self.assertTrue(row["pc_unsupported"])
        self.assertIsNone(row["is_live"])
        self.assertEqual(row["state"], "stopped")
        self.assertEqual(load_config(data_dir=self.path)["pc_unsupported_rids"], ["111"])
        with self.assertRaises(ServiceError):
            self.rpc("set_pc_unsupported", rid="999", unsupported=True)
        with self.assertRaises(ServiceError):
            self.rpc("set_pc_unsupported", rid="111", unsupported=1)
        self.rpc("remove", rids=["111"])
        until(lambda: not self.service._operations.unfinished_tasks)
        self.rpc("add_anchors", values="111")
        row = next(row for row in self.rpc("init")["anchors"] if row["rid"] == "111")
        self.assertFalse(row["pc_unsupported"])

    def test_serial_json_lines_loop_answers_execute_poll_status_and_shutdown_cancels(self):
        incoming, outgoing = queue.Queue(), queue.Queue()
        entered = threading.Event()
        original = self.service.manager.archive.execute_delete
        class Input:
            def readline(self, size=-1):
                return incoming.get(timeout=5)
        class Output:
            def write(self, text):
                outgoing.put(json.loads(text))
            def flush(self):
                pass
        def held(selection, cancel_token=None):
            entered.set()
            deadline = time.monotonic() + 5
            while not cancel_token.is_set() and time.monotonic() < deadline:
                time.sleep(.005)
            # Exercise the native cancellation bridge before any deletion.
            return original(selection, cancel_token)
        request_id = 0
        def request(method, params=None):
            nonlocal request_id
            request_id += 1
            incoming.put(json.dumps({"id": request_id, "method": method, "params": params or {}}) + "\n")
            response = outgoing.get(timeout=1)
            self.assertEqual(response["id"], request_id)
            return response
        with patch.object(self.service.manager.archive, "execute_delete", side_effect=held):
            loop = threading.Thread(target=run_service, args=(Input(), Output(), lambda: self.service), daemon=True)
            loop.start()
            accepted = request("execute_archive_delete", {"anchor_ids": ["11"]})
            self.assertTrue(accepted["ok"])
            job_id = accepted["result"]["job_id"]
            self.assertTrue(entered.wait(1))
            self.assertTrue(request("poll")["ok"])
            self.assertEqual(request("archive_delete_status", {"job_id": job_id})["result"]["status"], "running")
            self.assertFalse(request("start", {"rids": ["111"]})["ok"])
            self.assertFalse(request("update_readiness")["result"]["ready"])
            stopped = request("shutdown")
            self.assertTrue(stopped["result"]["all_workers_stopped"])
            loop.join(2)
            self.assertFalse(loop.is_alive())
        self.assertEqual(self.service._delete_jobs[job_id]["status"], "cancelled")
        with closing(sqlite3.connect(self.path / "messages.sqlite3")) as connection:
            self.assertEqual(connection.execute("SELECT COUNT(*) FROM messages").fetchone()[0], 4)

    def test_delete_thread_start_failure_does_not_leave_service_locked(self):
        with patch("threading.Thread.start", side_effect=RuntimeError("synthetic resource shortage")):
            with self.assertRaisesRegex(ServiceError, "无法启动"):
                self.rpc("execute_archive_delete", anchor_ids=["11"])
        self.assertFalse(self.service._archive_delete_in_progress)
        self.assertTrue(self.rpc("preview_archive_delete", anchor_ids=["11"])["ready"])

    def test_shutdown_can_cancel_job_before_worker_acquires_action_lock(self):
        real_thread = threading.Thread
        scheduled, release = threading.Event(), threading.Event()
        self.addCleanup(release.set)
        def delayed_thread(*args, **kwargs):
            if kwargs.get("name", "").startswith("archive-delete-"):
                target = kwargs["target"]
                def delayed():
                    scheduled.set()
                    release.wait(3)
                    target()
                kwargs["target"] = delayed
            return real_thread(*args, **kwargs)
        with patch("backend_service.threading.Thread", side_effect=delayed_thread):
            accepted = self.rpc("execute_archive_delete", anchor_ids=["11"])
        self.assertTrue(scheduled.wait(1))
        result = []
        closing_thread = real_thread(target=lambda: result.append(self.service.dispatch("shutdown", {})))
        closing_thread.start()
        self.assertTrue(self.service._closed.wait(1))
        release.set()
        closing_thread.join(2)
        self.assertFalse(closing_thread.is_alive(), "shutdown must release the action lock before joining the worker")
        self.assertTrue(result[0]["all_workers_stopped"])
        self.assertEqual(self.service._delete_jobs[accepted["job_id"]]["status"], "cancelled")
        with closing(sqlite3.connect(self.path / "messages.sqlite3")) as connection:
            self.assertEqual(connection.execute("SELECT COUNT(*) FROM messages").fetchone()[0], 4)


if __name__ == "__main__":
    unittest.main()
