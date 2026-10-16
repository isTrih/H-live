"""Elapsed monitoring contracts with fake clocks and no network access."""
from pathlib import Path
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python" / "collector"))
from monitor import MonitorManager
from monitor_duration import DurationLedger


def until(predicate, timeout=2):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(.005)
    raise AssertionError("condition timed out")


class Clock:
    def __init__(self):
        self.tick, self.wall = 100.0, 1800000100.0

    def advance(self, seconds):
        self.tick += seconds
        self.wall += seconds


class MemoryArchive:
    def __init__(self):
        self.saved = {}

    def load_monitor_duration(self, anchor, session):
        return self.saved.get((anchor, session), 0)

    def save_monitor_duration(self, anchor, session, seconds):
        self.saved[(anchor, session)] = max(seconds, self.saved.get((anchor, session), 0))


class DurationLedgerTests(unittest.TestCase):
    def ledger(self, archive=None, checkpoint_seconds=60):
        clock = Clock()
        ledger = DurationLedger(archive or MemoryArchive(), clock=lambda: clock.tick,
                                wall_clock=lambda: clock.wall, checkpoint_seconds=checkpoint_seconds)
        self.addCleanup(lambda: ledger.close(2))
        return ledger, clock

    def test_overlapping_entries_count_the_union_not_each_web_room(self):
        ledger, clock = self.ledger()
        first, second = object(), object()
        ledger.bind(first, "11", "22")
        ledger.resume(first)
        clock.advance(10)
        ledger.bind(second, "11", "22")
        ledger.resume(second, initial_seconds=1000)
        clock.advance(5)
        ledger.pause(first)
        clock.advance(7)
        ledger.pause(second)
        self.assertEqual(ledger.snapshot(second)["duration_seconds"], 22)
        self.assertTrue(ledger.flush())
        self.assertEqual(ledger.archive.saved[("11", "22")], 22)
        clock.advance(300)
        ledger.resume(second)
        clock.advance(3)
        self.assertEqual(ledger.snapshot(second)["duration_seconds"], 25)

    def test_close_saves_the_tail_after_an_inflight_older_checkpoint(self):
        entered, release = threading.Event(), threading.Event()
        class BlockedArchive(MemoryArchive):
            def save_monitor_duration(self, anchor, session, seconds):
                if not entered.is_set():
                    entered.set()
                    release.wait(2)
                super().save_monitor_duration(anchor, session, seconds)
        archive = BlockedArchive()
        ledger, clock = self.ledger(archive, checkpoint_seconds=5)
        token = object()
        ledger.bind(token, "11", "22")
        ledger.resume(token)
        clock.advance(10)
        # Pausing a second token wakes the normal checkpoint thread.
        ledger.pause(object())
        self.assertTrue(entered.wait(1))
        clock.advance(1)
        result = []
        closing = threading.Thread(target=lambda: result.append(ledger.close(2)))
        closing.start()
        until(lambda: not ledger.snapshot(token)["duration_running"])
        release.set()
        closing.join(2)
        self.assertEqual(result, [True])
        self.assertEqual(archive.saved[("11", "22")], 11)

    def test_pause_wakes_do_not_checkpoint_other_healthy_sessions_before_the_deadline(self):
        class CountingArchive(MemoryArchive):
            def __init__(self):
                super().__init__()
                self.writes = []
            def save_monitor_duration(self, anchor, session, seconds):
                self.writes.append((anchor, session))
                super().save_monitor_duration(anchor, session, seconds)
        archive = CountingArchive()
        ledger, clock = self.ledger(archive, checkpoint_seconds=5)
        healthy = [object() for _ in range(250)]
        retrying = [object() for _ in range(50)]
        for number, token in enumerate(healthy + retrying):
            ledger.bind(token, str(number + 1), "22")
            ledger.resume(token)
        for number, token in enumerate(retrying):
            clock.advance(.05)
            ledger.pause(token)
            until(lambda: (str(number + 251), "22") in archive.saved)
        self.assertEqual(len(archive.writes), 50)
        self.assertFalse(any((str(number + 1), "22") in archive.saved for number in range(250)))
        clock.advance(3)
        ledger.pause(object())
        until(lambda: len(archive.saved) == 300)
        self.assertEqual(len(archive.writes), 300)

    def test_failed_checkpoint_is_reported_and_close_does_not_claim_success(self):
        class BrokenArchive(MemoryArchive):
            def save_monitor_duration(self, *args):
                raise OSError("fixture disk unavailable")
        warnings = []
        clock = Clock()
        ledger = DurationLedger(BrokenArchive(), lambda: warnings.append(True), clock=lambda: clock.tick)
        token = object()
        ledger.bind(token, "11", "22")
        ledger.resume(token)
        clock.advance(10)
        self.assertFalse(ledger.close(2))
        self.assertTrue(warnings)
        self.assertEqual(ledger.snapshot(token)["duration_seconds"], 10)

    def test_restart_restores_only_the_checkpoint_without_backfilling_a_stale_open_interval(self):
        archive = MemoryArchive()
        ledger, clock = self.ledger(archive)
        token = object()
        ledger.bind(token, "11", "22")
        ledger.resume(token)
        clock.advance(7)
        self.assertTrue(ledger.flush())
        clock.advance(2)
        # Reopening a copy of the last committed checkpoint models abrupt exit;
        # the previous process's uncommitted two seconds cannot be reconstructed.
        committed = MemoryArchive()
        committed.saved.update(archive.saved)
        reopened, later_clock = self.ledger(committed)
        later_clock.advance(10000)
        reopened.bind(token, "11", "22")
        reopened.resume(token)
        self.assertEqual(reopened.snapshot(token)["duration_seconds"], 7)


class Fetcher:
    def __init__(self, rid, on_message, cookie, on_status):
        self.on_status = on_status
        self.closed = threading.Event()

    def start(self):
        self.on_status("connected", "合成连接")
        self.closed.wait()

    def stop(self):
        self.closed.set()


class MonitorDurationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.clock = Clock()
        self.created = []
        self.session = "22"
        self.addCleanup(patch.stopall)
        patch("monitor_duration.monotonic", side_effect=lambda: self.clock.tick).start()
        patch("monitor.time.time", side_effect=lambda: self.clock.wall).start()

    def manager(self, info=None):
        def factory(rid, **kwargs):
            fetcher = Fetcher(rid, **kwargs)
            self.created.append(fetcher)
            return fetcher
        manager = MonitorManager(data_dir=self.directory.name, fetcher_factory=factory,
            room_info_fetcher=info or (lambda rid, cookie="": {"is_live": True, "anchor_id": "11",
                                                               "room_id": self.session, "started_at": 1800000000}),
            retry_seconds=.01, offline_seconds=.01, connect_interval_seconds=0)
        self.addCleanup(lambda: manager.shutdown(2))
        return manager

    def connected(self, manager, count=1, rid="123"):
        until(lambda: len(self.created) >= count and manager.snapshot()[rid]["duration_running"])

    def test_stop_restart_and_process_restart_continue_same_session_without_downtime(self):
        manager = self.manager()
        manager.start("123", "主播")
        self.connected(manager)
        self.assertEqual(manager.snapshot()["123"]["duration_seconds"], 0)
        self.clock.advance(12.5)
        manager.stop("123")
        self.assertEqual(manager.snapshot()["123"]["duration_seconds"], 12.5)
        self.clock.advance(1000)
        manager.start("123", "主播")
        self.connected(manager, 2)
        self.assertEqual(manager.snapshot()["123"]["duration_seconds"], 12.5)
        self.clock.advance(7.5)
        self.assertTrue(manager.shutdown(2))
        self.assertEqual(manager.archive.load_monitor_duration("11", "22"), 20)
        self.clock.advance(5000)
        reopened = self.manager()
        reopened.start("123", "主播")
        self.connected(reopened, 3)
        self.assertEqual(reopened.snapshot()["123"]["duration_seconds"], 20)
        self.clock.advance(5)
        self.assertEqual(reopened.snapshot()["123"]["duration_seconds"], 25)

    def test_reconnect_pauses_until_connected_and_old_status_cannot_pause_the_new_socket(self):
        waiting, release = threading.Event(), threading.Event()
        calls = []
        def info(rid, cookie=""):
            calls.append(rid)
            if len(calls) == 2:
                waiting.set()
                release.wait(2)
            return {"is_live": True, "anchor_id": "11", "room_id": "22"}
        manager = self.manager(info)
        self.addCleanup(release.set)
        manager.start("123", "主播")
        self.connected(manager)
        old = self.created[0]
        self.clock.advance(10)
        old.closed.set()
        self.assertTrue(waiting.wait(1))
        self.assertFalse(manager.snapshot()["123"]["duration_running"])
        self.clock.advance(400)
        self.assertEqual(manager.snapshot()["123"]["duration_seconds"], 10)
        release.set()
        self.connected(manager, 2)
        old.on_status("error", "迟到状态")
        self.clock.advance(5)
        self.assertTrue(manager.snapshot()["123"]["duration_running"])
        self.assertEqual(manager.snapshot()["123"]["duration_seconds"], 15)

    def test_new_real_session_starts_zero_and_never_backfills_broadcast_time(self):
        manager = self.manager()
        manager.start("123", "主播")
        self.connected(manager)
        self.clock.advance(10)
        manager.stop("123")
        self.session = "33"
        self.clock.advance(100)
        manager.start("123", "主播")
        self.connected(manager, 2)
        row = manager.snapshot()["123"]
        self.assertEqual(row["session_id"], "33")
        self.assertEqual(row["duration_seconds"], 0)
        self.assertEqual(row["broadcast_started_at"], 1800000000)

    def test_initial_mid_broadcast_task_start_is_used_but_offline_wait_is_excluded(self):
        calls = []
        def info(rid, cookie=""):
            calls.append(rid)
            self.clock.advance(10)
            return {"is_live": rid == "123" or calls.count(rid) > 1,
                    "anchor_id": "11" if rid == "123" else "12", "room_id": "22"}
        manager = self.manager(info)
        manager.start("123", "中途加入")
        self.connected(manager)
        self.assertEqual(manager.snapshot()["123"]["duration_seconds"], 10)
        manager.stop("123")
        manager.start("124", "等待开播")
        self.connected(manager, 2, "124")
        row = manager.snapshot()["124"]
        self.assertEqual(row["duration_seconds"], 0)
        self.assertEqual(row["duration_source"], "monitored")

    def test_two_web_entries_for_one_broadcast_do_not_double_count(self):
        manager = self.manager()
        manager.start("123", "第一个入口")
        self.connected(manager)
        self.clock.advance(10)
        manager.start("124", "第二个入口")
        self.connected(manager, 2, "124")
        self.clock.advance(5)
        manager.stop("123")
        self.clock.advance(7)
        manager.stop("124")
        self.assertEqual(manager.snapshot()["124"]["duration_seconds"], 22)
        self.assertTrue(manager.shutdown(2))


if __name__ == "__main__":
    unittest.main()
