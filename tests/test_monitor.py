import json
from datetime import datetime
from pathlib import Path
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python" / "collector"))

from monitor import MonitorManager
from archive_store import SHANGHAI


def live_info(rid, cookie=""):
    return {"is_live": True, "title": "直播", "anchor_id": "100" + rid, "room_id": "200" + rid, "started_at": 0}


def wait_until(predicate, timeout=2):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.005)
    raise AssertionError("condition was not reached before timeout")


class FakeFetcher:
    def __init__(self, rid, on_message, cookie, on_status, disconnect=False):
        self.rid = rid
        self.on_message = on_message
        self.on_status = on_status
        self.disconnected = threading.Event()
        self.entered = threading.Event()
        self.disconnect = disconnect

    def start(self):
        self.entered.set()
        if not self.disconnected.is_set():
            self.on_status("connected", "测试连接")
        if not self.disconnect:
            self.disconnected.wait()

    def stop(self):
        self.disconnected.set()

    def emit(self, text, **extra):
        self.on_message("chat", {"text": text, "content": text, "sender_name": "用户", "sender_id": "888",
                                 "room_id": "200" + self.rid, "event_time": int(time.time()), **extra})


class MonitorTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.events = []
        self.created = []
        self.creation_lock = threading.Lock()
        self.manager = None

    def tearDown(self):
        if self.manager is not None:
            self.manager.shutdown(2)
        self.temporary.cleanup()

    def factory(self, rid, **kwargs):
        fetcher = FakeFetcher(rid, **kwargs)
        with self.creation_lock:
            self.created.append(fetcher)
        return fetcher

    def make_manager(self, **overrides):
        args = dict(on_event=self.events.append, data_dir=self.temporary.name,
                    fetcher_factory=self.factory,
                    room_info_fetcher=live_info,
                    retry_seconds=0.01, offline_seconds=0.01, connect_interval_seconds=0)
        args.update(overrides)
        self.manager = MonitorManager(**args)
        return self.manager

    def records(self, rid):
        with self.manager.archive.export_snapshot() as reader:
            result = [dict(row) for row in reader.connection.execute("SELECT * FROM messages WHERE web_rid=? ORDER BY id", (rid,))]
        return [{**row, "text": row["content"], "rid": row["web_rid"], "name": row["anchor_name"]} for row in result]

    def test_three_rooms_archive_concurrently_without_cross_room_messages(self):
        manager = self.make_manager()
        for rid in ("111", "222", "333"):
            self.assertTrue(manager.start(rid, f"主播{rid}"))
        wait_until(lambda: len(self.created) == 3 and all(f.entered.is_set() for f in self.created))
        def send(fetcher):
            for index in range(100):
                fetcher.emit(f"{fetcher.rid}:中文消息{index}")
        threads = [threading.Thread(target=send, args=(fetcher,)) for fetcher in self.created]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(3)
            self.assertFalse(thread.is_alive())
        for rid in ("111", "222", "333"):
            records = self.records(rid)
            self.assertEqual(len(records), 100)
            self.assertEqual([r["text"] for r in records], [f"{rid}:中文消息{i}" for i in range(100)])
            self.assertTrue(all(r["rid"] == rid and r["name"] == f"主播{rid}" for r in records))
            self.assertEqual(manager.snapshot()[rid]["message_count"], 100)

    def test_archive_is_complete_before_ui_callback_and_ui_failure_is_isolated(self):
        received = []
        def broken_ui(event):
            if event["kind"] == "message":
                received.append(len(self.records(event["rid"])))
            raise RuntimeError("simulated full UI queue")
        manager = self.make_manager(on_event=broken_ui)
        manager.start("111", "主播")
        wait_until(lambda: bool(self.created))
        self.created[0].emit("第一条")
        self.created[0].emit("第二条")
        self.assertEqual(received, [1, 2])
        self.assertEqual(len(self.records("111")), 2)
        self.assertTrue(manager.snapshot()["111"]["active"])

    def test_duplicate_start_is_rejected_and_stop_blocks_old_callbacks(self):
        manager = self.make_manager()
        self.assertTrue(manager.start("111", "旧主播"))
        self.assertFalse(manager.start("111", "重复"))
        wait_until(lambda: bool(self.created))
        old = self.created[0]
        old.emit("收到的消息")
        self.assertTrue(manager.stop("111"))
        event_count = len(self.events)
        old.emit("应忽略的消息")
        old.on_status("connected", "迟到的连接事件")
        self.assertEqual(len(self.events), event_count)
        self.assertEqual(len(self.records("111")), 1)
        self.assertEqual(manager.snapshot()["111"]["state"], "stopped")
        self.assertFalse(manager.snapshot()["111"]["active"])
        self.assertFalse(manager.stop("111"))

    def test_worker_thread_start_failure_is_inactive_and_allows_retry(self):
        manager = self.make_manager()
        with patch.object(threading.Thread, "start", side_effect=RuntimeError("cannot start new thread")):
            self.assertFalse(manager.start("111", "主播", cookie="private-cookie"))
        state = manager.snapshot()["111"]
        self.assertEqual(state["state"], "error")
        self.assertFalse(state["active"])
        self.assertNotIn("private-cookie", json.dumps(self.events))
        self.assertTrue(manager.start("111", "主播"))
        wait_until(lambda: len(self.created) == 1 and self.created[0].entered.is_set())
        self.created[0].emit("恢复后的消息")
        self.assertEqual(len(self.records("111")), 1)
        self.assertTrue(manager.shutdown(2))

    def test_broadcast_start_uses_server_epoch_and_unknown_stays_null(self):
        def info(rid, cookie=""):
            return {**live_info(rid, cookie), "started_at": 1800000000 if rid == "111" else 0}
        manager = self.make_manager(room_info_fetcher=info)
        manager.start("111", "known")
        manager.start("222", "unknown")
        wait_until(lambda: len(self.created) == 2 and all(value.entered.is_set() for value in self.created))
        rows = manager.snapshot()
        self.assertEqual(1800000000, rows["111"]["broadcast_started_at"])
        self.assertIsNone(rows["222"]["broadcast_started_at"])
        next(value for value in self.created if value.rid == "111").emit("archive actual start")
        with manager.archive.export_snapshot() as reader:
            row = reader.connection.execute("SELECT started_at FROM sessions WHERE session_id=?", ("200111",)).fetchone()
            self.assertEqual("2027-01-15T16:00:00.000+08:00", row["started_at"])

    def test_restart_uses_new_generation_and_ignores_old_worker(self):
        manager = self.make_manager()
        manager.start("111", "旧名称")
        wait_until(lambda: bool(self.created))
        old = self.created[0]
        manager.stop("111")
        manager.start("111", "新名称")
        wait_until(lambda: len(self.created) == 2 and self.created[1].entered.is_set())
        old.emit("旧消息")
        old.on_status("error", "旧错误")
        self.created[1].emit("新消息")
        records = self.records("111")
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["name"], "新名称")
        self.assertEqual(manager.snapshot()["111"]["state"], "connected")

    def test_offline_room_waits_then_connects_and_stop_cancels_polling(self):
        attempts = []
        def info(rid, cookie=""):
            attempts.append(rid)
            return {**live_info(rid), "is_live": len(attempts) >= 3}
        manager = self.make_manager(room_info_fetcher=info)
        manager.start("111", "主播")
        wait_until(lambda: bool(self.created))
        self.assertEqual(len(attempts), 3)
        self.assertTrue(any(e.get("state") == "offline" for e in self.events))
        manager.stop("111")
        time.sleep(0.04)
        self.assertEqual(len(attempts), 3)

    def test_room_refusal_after_live_is_suspected_until_offline_is_confirmed(self):
        from liveMan import LiveRoomUnavailableError
        calls = []
        checking, release = threading.Event(), threading.Event()
        def info(rid, cookie=""):
            calls.append(rid)
            if len(calls) == 1:
                return live_info(rid)
            if len(calls) == 2:
                raise LiveRoomUnavailableError()
            checking.set()
            release.wait(2)
            return {**live_info(rid), "is_live": False}
        manager = self.make_manager(room_info_fetcher=info)
        manager.start("111", "主播")
        wait_until(lambda: bool(self.created) and self.created[0].entered.is_set())
        before = manager.snapshot()["111"]
        self.created[0].disconnected.set()
        try:
            self.assertTrue(checking.wait(1))
            pending = manager.snapshot()["111"]
            self.assertEqual("confirming_offline", pending["state"])
            self.assertTrue(pending["is_live"])
            for key in ("session_id", "duration_started_at", "live_checked_at"):
                self.assertEqual(before[key], pending[key])
        finally:
            release.set()
        wait_until(lambda: manager.snapshot()["111"]["state"] == "offline")
        self.assertFalse(manager.snapshot()["111"]["is_live"])

    def test_transient_room_refusal_reconnects_to_same_session(self):
        from liveMan import LiveRoomUnavailableError
        calls = []
        def info(rid, cookie=""):
            calls.append(rid)
            if len(calls) == 2:
                raise LiveRoomUnavailableError()
            return live_info(rid)
        manager = self.make_manager(room_info_fetcher=info)
        manager.start("111", "主播")
        wait_until(lambda: bool(self.created) and self.created[0].entered.is_set())
        before = manager.snapshot()["111"]
        self.created[0].emit("断线前消息", message_id="1")
        self.created[0].disconnected.set()
        wait_until(lambda: len(self.created) == 2 and self.created[1].entered.is_set())
        self.created[1].emit("恢复后消息", message_id="2")
        after = manager.snapshot()["111"]
        self.assertEqual(before["session_id"], after["session_id"])
        self.assertGreaterEqual(after["duration_seconds"], before["duration_seconds"])
        self.assertTrue(after["duration_running"])
        self.assertEqual(2, len(self.records("111")))
        self.assertTrue(any(e.get("state") == "confirming_offline" for e in self.events))
        self.assertFalse(any(e.get("state") == "offline" for e in self.events))

    def test_persistent_refusal_does_not_confirm_offline(self):
        from liveMan import LiveRoomUnavailableError
        calls = []
        def info(rid, cookie=""):
            calls.append(rid)
            if len(calls) > 1:
                raise LiveRoomUnavailableError()
            return live_info(rid)
        manager = self.make_manager(room_info_fetcher=info)
        manager.start("111", "主播")
        wait_until(lambda: bool(self.created) and self.created[0].entered.is_set())
        self.created[0].disconnected.set()
        wait_until(lambda: len(calls) >= 4 and manager.snapshot()["111"]["state"] == "retrying")
        self.assertTrue(manager.snapshot()["111"]["is_live"])
        self.assertFalse(any(e.get("state") == "offline" for e in self.events))
        self.assertIn("尚不能确认", manager.snapshot()["111"]["detail"])

    def test_initial_room_refusal_is_not_treated_as_suspected_offline(self):
        from liveMan import LiveRoomUnavailableError
        def info(rid, cookie=""):
            raise LiveRoomUnavailableError()
        manager = self.make_manager(room_info_fetcher=info)
        manager.start("111", "主播")
        wait_until(lambda: manager.snapshot()["111"]["state"] == "retrying")
        self.assertIsNone(manager.snapshot()["111"]["is_live"])
        self.assertFalse(any(e.get("state") == "confirming_offline" for e in self.events))

    def test_disconnect_reconnects_and_old_attempt_callbacks_are_ignored(self):
        def factory(rid, **kwargs):
            fetcher = FakeFetcher(rid, **kwargs, disconnect=not self.created)
            self.created.append(fetcher)
            return fetcher
        manager = self.make_manager(fetcher_factory=factory)
        manager.start("111", "主播")
        wait_until(lambda: len(self.created) == 2 and self.created[1].entered.is_set())
        self.created[0].emit("断开连接的迟到消息")
        self.created[0].on_status("error", "旧连接错误")
        self.created[1].emit("重连后消息")
        self.assertEqual([r["text"] for r in self.records("111")], ["重连后消息"])
        self.assertTrue(any(e.get("state") == "retrying" for e in self.events))
        self.assertEqual(manager.snapshot()["111"]["state"], "connected")

    def test_one_failed_room_does_not_block_another_and_secrets_are_not_reported(self):
        def info(rid, cookie=""):
            if rid == "111":
                raise RuntimeError("cookie=PRIVATE_SECRET")
            return live_info(rid)
        manager = self.make_manager(room_info_fetcher=info)
        manager.start("111", "失败房间")
        manager.start("222", "正常房间")
        wait_until(lambda: bool(self.created) and self.created[0].entered.is_set())
        self.created[0].emit("正常消息")
        self.assertEqual(len(self.records("222")), 1)
        self.assertTrue(any(e.get("state") == "retrying" and e["rid"] == "111" for e in self.events))
        self.assertNotIn("PRIVATE_SECRET", str(self.events))

    def test_archive_failure_is_explicit_and_stops_only_that_room(self):
        manager = self.make_manager()
        manager.start("111", "主播")
        wait_until(lambda: bool(self.created))
        with patch.object(manager.archive, "record_message", side_effect=PermissionError("private path")):
            self.created[0].emit("无法写入")
        self.assertEqual(manager.snapshot()["111"]["state"], "error")
        self.assertFalse(manager.snapshot()["111"]["active"])
        self.assertIn("数据库写入失败", manager.snapshot()["111"]["detail"])
        self.assertFalse(any(e["kind"] == "message" for e in self.events))
        time.sleep(0.04)
        self.assertEqual(len(self.created), 1)

    def test_stop_during_room_lookup_prevents_connection(self):
        entered, release = threading.Event(), threading.Event()
        def info(rid, cookie=""):
            entered.set()
            release.wait(2)
            return {"is_live": True}
        manager = self.make_manager(room_info_fetcher=info)
        manager.start("111", "主播")
        self.assertTrue(entered.wait(1))
        manager.stop("111")
        release.set()
        self.assertTrue(manager.shutdown(1))
        self.assertEqual(self.created, [])

    def test_shutdown_timeout_and_no_restart_after_shutdown(self):
        entered, release = threading.Event(), threading.Event()
        def info(rid, cookie=""):
            entered.set()
            release.wait(2)
            return {"is_live": True}
        manager = self.make_manager(room_info_fetcher=info)
        manager.start("111", "主播")
        self.assertTrue(entered.wait(1))
        try:
            begin = time.monotonic()
            self.assertFalse(manager.shutdown(0.02))
            self.assertLess(time.monotonic() - begin, 0.2)
            self.assertFalse(manager.start("222", "新主播"))
        finally:
            release.set()
        self.assertTrue(manager.shutdown(1))

    def test_invalid_room_cannot_create_archive_path(self):
        manager = self.make_manager()
        with self.assertRaises(ValueError):
            manager.start("../../outside", "恶意输入")
        self.assertEqual(manager.snapshot(), {})

    def test_room_metadata_resolves_placeholders_but_preserves_user_aliases(self):
        manager = self.make_manager(room_info_fetcher=lambda rid, cookie="": {
            **live_info(rid), "nickname": f"真实主播{rid}", "title": "真实标题",
        })
        manager.start("111", "主播 111")
        manager.start("222", "我设置的别名")
        wait_until(lambda: len(self.created) == 2 and all(f.entered.is_set() for f in self.created))
        self.assertEqual(manager.snapshot()["111"]["name"], "真实主播111")
        self.assertEqual(manager.snapshot()["222"]["name"], "我设置的别名")
        first = next(f for f in self.created if f.rid == "111")
        first.emit("已识别名称")
        self.assertEqual(self.records("111")[0]["name"], "真实主播111")
        first.on_message("stats", {"text": "当前观看20", "current": 20})
        self.assertNotIn("user_count", manager.snapshot()["111"])
        self.assertEqual(len(self.records("111")), 1)

    def test_midnight_does_not_create_new_session(self):
        manager = self.make_manager()
        manager.start("111", "主播")
        wait_until(lambda: bool(self.created))
        times = [datetime(2026, 10, 14, 23, 59, 59, tzinfo=SHANGHAI),
                 datetime(2026, 10, 15, 0, 0, 1, tzinfo=SHANGHAI)]
        with patch("monitor.datetime") as clock:
            clock.now.side_effect = times
            self.created[0].emit("午夜前", event_time=times[0].timestamp())
            self.created[0].emit("午夜后", event_time=times[1].timestamp())
        before, after = self.records("111")
        self.assertEqual(before["text"], "午夜前")
        self.assertEqual(after["text"], "午夜后")
        self.assertTrue(after["message_time"].startswith("2026-10-15T00:00:01"))
        self.assertEqual(before["session_id"], after["session_id"])
        self.assertFalse((Path(self.temporary.name) / "messages").exists())

    def test_stop_waits_for_accepted_archive_and_does_not_block_other_rooms(self):
        manager = self.make_manager()
        manager.start("111", "主播1")
        manager.start("222", "主播2")
        wait_until(lambda: len(self.created) == 2)
        first = next(f for f in self.created if f.rid == "111")
        second = next(f for f in self.created if f.rid == "222")
        writing, release, stopped = threading.Event(), threading.Event(), threading.Event()
        original_write = manager.archive.record_message
        def blocked_write(record, **kwargs):
            if record["web_rid"] == "111":
                writing.set()
                if not release.wait(2):
                    raise TimeoutError("test release was not signaled")
            return original_write(record, **kwargs)

        def stop_first():
            manager.stop("111")
            stopped.set()

        with patch.object(manager.archive, "record_message", blocked_write):
            sender = threading.Thread(target=first.emit, args=("在途消息",))
            stopper = threading.Thread(target=stop_first)
            sender.start()
            self.assertTrue(writing.wait(1))
            stopper.start()
            try:
                self.assertFalse(stopped.wait(0.03))
                second.emit("其他房间保持响应")
                self.assertEqual(manager.snapshot()["222"]["message_count"], 1)
            finally:
                release.set()
                sender.join(2)
                stopper.join(2)
        self.assertFalse(sender.is_alive())
        self.assertFalse(stopper.is_alive())
        self.assertTrue(stopped.is_set())
        self.assertEqual(manager.snapshot()["111"]["message_count"], 1)
        self.assertEqual(len(self.records("111")), 1)
        first.emit("停止后的消息")
        self.assertEqual(len(self.records("111")), 1)

    def test_short_lived_connections_back_off_instead_of_resetting_on_handshake(self):
        def factory(rid, **kwargs):
            fetcher = FakeFetcher(rid, **kwargs, disconnect=len(self.created) < 3)
            self.created.append(fetcher)
            return fetcher
        manager = self.make_manager(fetcher_factory=factory)
        manager.start("111", "主播")
        wait_until(lambda: len(self.created) == 4 and self.created[-1].entered.is_set())
        details = [event["detail"] for event in self.events if event.get("state") == "retrying"]
        self.assertEqual(len(details), 3)
        for detail, delay in zip(details, (0.01, 0.02, 0.04)):
            self.assertIn(f"{delay:g} 秒后重试", detail)

    def test_transport_error_remains_active_with_explicit_retry_status(self):
        manager = self.make_manager()
        manager.start("111", "主播")
        wait_until(lambda: bool(self.created))
        self.created[0].on_status("error", "服务器断开连接")
        state = manager.snapshot()["111"]
        self.assertEqual(state["state"], "retrying")
        self.assertTrue(state["active"])
        self.assertIn("准备重新连接", state["detail"])

    def test_individual_room_credentials_are_passed_to_lookup_and_fetcher(self):
        lookup_cookies, socket_cookies = {}, {}
        def info(rid, cookie=""):
            lookup_cookies[rid] = cookie
            return live_info(rid)
        def factory(rid, **kwargs):
            socket_cookies[rid] = kwargs["cookie"]
            return self.factory(rid, **kwargs)
        manager = self.make_manager(fetcher_factory=factory, room_info_fetcher=info)
        manager.start("111", "主播1", "sessionid=FAKE_ACCOUNT_A")
        manager.start("222", "主播2", "sessionid=FAKE_ACCOUNT_B")
        wait_until(lambda: len(self.created) == 2)
        expected = {"111": "sessionid=FAKE_ACCOUNT_A", "222": "sessionid=FAKE_ACCOUNT_B"}
        self.assertEqual(lookup_cookies, expected)
        self.assertEqual(socket_cookies, expected)
        self.assertNotIn("FAKE_ACCOUNT", str(self.events))
        self.assertNotIn("FAKE_ACCOUNT", str(manager.snapshot()))

    def test_stats_are_discarded_and_control_goes_only_to_run_log(self):
        manager = self.make_manager()
        manager.start("111", "主播")
        wait_until(lambda: bool(self.created))
        previous = len(self.events)
        for kind in ("stats", "roomstats", "viewer", "adaptation"):
            self.created[0].on_message(kind, {"text": "观众统计不可记录", "current": 123})
        self.assertEqual(len(self.events), previous)
        self.created[0].on_message("control", {"text": "直播结束"})
        self.assertEqual(self.events[-1]["kind"], "log")
        self.assertEqual(manager.snapshot()["111"]["message_count"], 0)
        self.assertEqual(self.records("111"), [])
        wait_until(lambda: manager.metrics()["log_queue_depth"] == 0)
        wait_until(lambda: any("直播结束" in path.read_text(encoding="utf-8") for path in (Path(self.temporary.name) / "logs").glob("*.jsonl")))
        logs = "".join(path.read_text(encoding="utf-8") for path in (Path(self.temporary.name) / "logs").glob("*.jsonl"))
        self.assertIn("直播结束", logs)
        self.assertNotIn("观众统计不可记录", logs)

    def test_current_viewer_updates_match_room_and_stay_out_of_chat_archive(self):
        manager = self.make_manager(room_info_fetcher=lambda rid, cookie="": {**live_info(rid, cookie), "viewer_count": 21})
        manager.start("111", "主播")
        wait_until(lambda: len(self.created) == 1 and self.created[0].entered.is_set())
        self.assertEqual(21, manager.snapshot()["111"]["viewer_count"])
        previous = len(self.events)
        self.created[0].on_message("stats", {"viewer_count": 42, "room_id": "200111"})
        self.assertEqual(42, manager.snapshot()["111"]["viewer_count"])
        self.created[0].on_message("stats", {"viewer_count": 99, "room_id": "999"})
        self.assertEqual(42, manager.snapshot()["111"]["viewer_count"])
        self.created[0].on_message("stats", {"viewer_count": 0, "room_id": "200111"})
        self.assertEqual(0, manager.snapshot()["111"]["viewer_count"])
        self.assertEqual(previous, len(self.events))
        self.assertEqual([], self.records("111"))
        self.assertEqual(0, manager.snapshot()["111"]["message_count"])
        overview=manager.archive.quality('detail',{'anchor_id':'100111','session_id':'200111'})['overview']
        self.assertEqual(42,overview['peak_viewers'])
        self.assertEqual(1,overview['audience_sample_minutes'])
        manager.stop("111")
        self.created[0].on_message("stats", {"viewer_count": 55, "room_id": "200111"})
        self.assertEqual(0, manager.snapshot()["111"]["viewer_count"])

    def test_log_failure_warns_once_but_content_continues(self):
        manager = self.make_manager()
        (Path(self.temporary.name) / "logs").write_text("occupied", encoding="utf-8")
        manager.start("111", "主播")
        wait_until(lambda: bool(self.created))
        self.created[0].emit("内容继续保存")
        self.assertEqual(len(self.records("111")), 1)
        wait_until(lambda: any(event.get("msg_type") == "warning" for event in self.events))
        warnings = [event for event in self.events if event.get("msg_type") == "warning"]
        self.assertEqual(len(warnings), 1)
        self.assertTrue(manager.snapshot()["111"]["active"])

    def test_missing_real_identity_pauses_without_inventing_session(self):
        manager = self.make_manager(room_info_fetcher=lambda rid, cookie="": {"is_live": True})
        manager.start("111", "主播")
        wait_until(lambda: manager.snapshot()["111"]["state"] == "error")
        self.assertFalse(manager.snapshot()["111"]["active"])
        self.assertEqual(self.created, [])
        self.assertEqual(self.records("111"), [])

    def test_session_mismatch_pauses_and_does_not_misfile_message(self):
        manager = self.make_manager()
        manager.start("111", "主播")
        wait_until(lambda: bool(self.created))
        self.created[0].emit("不能错归场次", room_id="999999")
        self.assertEqual(manager.snapshot()["111"]["state"], "error")
        self.assertEqual(self.records("111"), [])

    def test_server_message_ids_deduplicate_across_manual_restart(self):
        manager = self.make_manager()
        manager.start("111", "主播")
        wait_until(lambda: bool(self.created))
        self.created[0].emit("同一条消息", message_id="1234567890123456789")
        manager.stop("111")
        manager.start("111", "主播")
        wait_until(lambda: len(self.created) == 2)
        self.created[1].emit("同一条消息", message_id="1234567890123456789")
        self.created[1].emit("没有消息ID仍保留", event_time=0, sender_id="")
        rows = self.records("111")
        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[0]["session_id"], rows[1]["session_id"])
        self.assertEqual(rows[1]["message_time"], "")
        self.assertEqual(rows[1]["time_source"], "missing")
        self.assertEqual(rows[1]["sender_id"], "")
        self.assertEqual(manager.snapshot()["111"]["message_count"], 1)

    def test_metadata_live_status_is_independent_from_connection_and_preserved_on_stop(self):
        manager = self.make_manager(room_info_fetcher=lambda rid, cookie="": {**live_info(rid), "is_live": False})
        manager.start("111", "主播")
        wait_until(lambda: manager.snapshot()["111"]["state"] == "offline")
        before = manager.snapshot()["111"]
        self.assertFalse(before["is_live"])
        self.assertIsNotNone(datetime.fromisoformat(before["live_checked_at"]).tzinfo)
        manager.stop("111")
        after = manager.snapshot()["111"]
        self.assertEqual(after["is_live"], before["is_live"])
        self.assertEqual(after["live_checked_at"], before["live_checked_at"])

    def test_connection_limit_covers_metadata_until_handshake_and_stop_cancels_queue(self):
        entered = []
        released = threading.Event()
        lock = threading.Lock()
        def info(rid, cookie=""):
            with lock:
                entered.append(rid)
            released.wait(2)
            return live_info(rid)
        manager = self.make_manager(room_info_fetcher=info, max_connecting=2)
        for index in range(6):
            manager.start(str(111 + index), "主播")
        try:
            wait_until(lambda: len(entered) == 2)
            time.sleep(0.02)
            self.assertEqual(len(entered), 2)
            self.assertEqual(manager.metrics()["connecting_inflight"], 2)
            self.assertIsNone(manager.snapshot()["116"]["is_live"])
            manager.stop("116")
        finally:
            released.set()
        wait_until(lambda: len(self.created) == 5)
        self.assertNotIn("116", entered)
        wait_until(lambda: manager.metrics()["connecting_inflight"] == 0)

    def test_rename_updates_subsequent_archive_and_event_without_reconnect(self):
        manager = self.make_manager()
        manager.start("111", "原备注")
        wait_until(lambda: bool(self.created))
        self.assertTrue(manager.rename("111", "新备注"))
        self.created[0].emit("更名后的内容")
        self.assertEqual(self.records("111")[0]["anchor_name"], "新备注")
        self.assertEqual(manager.snapshot()["111"]["name"], "新备注")
        self.assertEqual(len(self.created), 1)


if __name__ == "__main__":
    unittest.main()
