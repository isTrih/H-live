from datetime import datetime
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import threading
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python" / "collector"))
from archive_store import ArchiveStore, SHANGHAI, month_bounds, server_time


def message(**overrides):
    row = {"anchor_id": "9007199254740993123", "anchor_name": "主播一", "web_rid": "111",
           "session_id": "7696456021958609673", "sender_id": "9007199254740993999", "sender_name": "用户一",
           "message_type": "chat", "content": "纯消息内容", "message_time": "2026-10-01T00:00:00.000+08:00",
           "received_at": "2026-10-01T00:00:02.000+08:00", "time_source": "server", "message_id": "", "payload": {}}
    row.update(overrides)
    return row


class ArchiveTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.store = ArchiveStore(self.temporary.name)

    def tearDown(self):
        self.store.close()
        self.temporary.cleanup()

    def rows(self, month="2026-10", **kwargs):
        return list(self.store.iter_messages([message()["anchor_id"]], month, **kwargs))

    def test_long_ids_and_pure_content_are_preserved_as_text(self):
        row = message()
        self.assertTrue(self.store.record_message(row, title="开播主题"))
        saved = self.rows()[0]
        for key in ("anchor_id", "session_id", "sender_id", "content"):
            self.assertEqual(saved[key], row[key])
            self.assertIsInstance(saved[key], str)
        self.assertEqual(self.store.list_anchors(), [{"anchor_id": row["anchor_id"], "anchor_name": row["anchor_name"], "web_rid": "111"}])

    def test_room_entry_and_nickname_changes_keep_the_same_anchor_and_session(self):
        self.store.record_message(message(message_id="1"))
        self.store.record_message(message(web_rid="222", anchor_name="改名主播", message_id="2"))
        with self.store.export_snapshot() as reader:
            self.assertEqual(reader.connection.execute("SELECT COUNT(*) FROM sessions").fetchone()[0], 1)
            self.assertEqual(reader.connection.execute("SELECT started_at FROM sessions").fetchone()[0], None)
            self.assertEqual(reader.list_anchors()[0]["web_rid"], "222")
        self.assertEqual(len(self.rows()), 2)

    def test_new_real_room_id_creates_new_session_even_on_same_day(self):
        self.store.record_message(message(message_id="same-message-id"))
        self.store.record_message(message(session_id="7696456021958609674", message_id="same-message-id"))
        with self.store.export_snapshot() as reader:
            self.assertEqual(reader.connection.execute("SELECT COUNT(*) FROM sessions").fetchone()[0], 2)
        self.assertEqual(len(self.rows()), 2)

    def test_known_message_id_deduplicates_but_missing_id_does_not_guess(self):
        self.assertTrue(self.store.record_message(message(message_id="123")))
        self.assertFalse(self.store.record_message(message(message_id="123")))
        self.assertTrue(self.store.record_message(message()))
        self.assertTrue(self.store.record_message(message()))
        self.assertEqual(len(self.rows()), 3)

    def test_month_boundaries_use_server_time_and_missing_time_uses_receipt_month(self):
        self.store.record_message(message(message_time="2026-09-30T23:59:59.999+08:00", message_id="1"))
        self.store.record_message(message(message_id="2"))
        self.store.record_message(message(message_time="2026-11-01T00:00:00.000+08:00", message_id="3"))
        self.store.record_message(message(message_time="", time_source="missing", message_id="4"))
        self.store.record_message(message(message_time="", time_source="missing", received_at="2026-11-01T00:00:00.000+08:00", message_id="5"))
        october = self.rows()
        self.assertEqual([row["message_id"] for row in october], ["2", "4"])
        self.assertEqual(october[1]["message_time"], "")
        self.assertEqual(october[1]["time_source"], "missing")
        self.assertEqual(self.store.count_missing_time(month="2026-10"), 1)
        self.assertEqual(self.store.count_missing_time(), 2)

    def test_equivalent_utc_server_timestamp_uses_shanghai_month(self):
        self.store.record_message(message(message_time="2026-09-30T16:00:00+00:00"))
        self.assertEqual(len(self.rows()), 1)
        self.assertEqual(len(self.rows("2026-09")), 0)

    def test_read_snapshot_is_consistent_while_writer_commits(self):
        self.store.record_message(message(message_id="1"))
        with self.store.export_snapshot() as reader:
            first = list(reader.iter_messages(None, "2026-10"))
            self.store.record_message(message(message_id="2"))
            second = list(reader.iter_messages(None, "2026-10"))
            self.assertEqual(first, second)
            with self.assertRaises(sqlite3.OperationalError):
                reader.connection.execute("DELETE FROM messages")
        self.assertEqual(len(self.rows()), 2)

    def test_stats_and_missing_real_identity_cannot_enter_content_archive(self):
        for row in (message(message_type="stats"), message(anchor_id=""), message(session_id=""), message(session_id="nickname")):
            with self.assertRaises(ValueError):
                self.store.record_message(row)
        self.assertEqual(self.rows(), [])

    def test_old_jsonl_is_preserved_but_never_implicitly_imported(self):
        folder = Path(self.temporary.name) / "messages" / "111"
        folder.mkdir(parents=True)
        old = folder / "2026-10-01.jsonl"
        content = json.dumps({"text": "旧版无场次无sender消息"}, ensure_ascii=False)
        old.write_text(content, encoding="utf-8")
        with ArchiveStore(self.temporary.name) as new_store:
            self.assertEqual(list(new_store.iter_messages(None, "2026-10")), [])
        self.assertEqual(old.read_text(encoding="utf-8"), content)

    def test_invalid_months_and_types_are_rejected(self):
        for month in ("2026-1", "2026-13", "2026-00", "0000-01", "2026-10 OR 1=1", "../2026-10"):
            with self.subTest(month=month), self.assertRaises(ValueError):
                list(self.store.iter_messages(None, month))
        with self.assertRaises(ValueError):
            list(self.store.iter_messages(None, "2026-10", ["stats"]))
        self.assertEqual(list(self.store.iter_messages([], "2026-10")), [])

    def test_server_time_missing_or_invalid_never_substitutes_local_clock(self):
        for value in (None, 0, "", "not-a-time", float("nan"), float("inf"), 10**30, True):
            self.assertEqual(server_time(value), ("", None))
        moment = datetime(2026, 10, 1, tzinfo=SHANGHAI)
        iso, epoch = server_time(moment.timestamp())
        self.assertEqual(iso, "2026-10-01T00:00:00.000+08:00")
        self.assertEqual(epoch, moment.timestamp())

    def test_sessions_use_only_explicit_server_started_at(self):
        self.store.record_message(message(message_id="1"), started_at=None)
        self.store.record_message(message(message_id="2"), started_at="2026-09-30T22:00:00+08:00")
        self.store.record_message(message(message_id="3"), started_at=None)
        with self.store.export_snapshot() as reader:
            row = reader.connection.execute("SELECT * FROM sessions").fetchone()
            self.assertEqual(row["started_at"], "2026-09-30T22:00:00+08:00")
            self.assertEqual(row["first_seen"], message()["received_at"])

    def test_concurrent_producers_commit_batches_and_close_flushes_exactly(self):
        gate = threading.Barrier(21)
        failures = []
        def producer(index):
            gate.wait()
            try:
                for sequence in range(20):
                    self.store.record_message(message(message_id=f"{index}-{sequence}"))
            except Exception as exc:
                failures.append(exc)
        threads = [threading.Thread(target=producer, args=(index,)) for index in range(20)]
        for thread in threads:
            thread.start()
        gate.wait()
        for thread in threads:
            thread.join(10)
            self.assertFalse(thread.is_alive())
        self.assertEqual(failures, [])
        self.assertTrue(self.store.close())
        self.assertEqual(len(self.rows()), 400)
        metrics = self.store.metrics()
        self.assertGreater(metrics["largest_batch"], 1)
        self.assertEqual(metrics["committed_messages"], 400)
        self.assertEqual(metrics["queue_depth"], 0)
        self.assertFalse(metrics["writer_alive"])
        with self.assertRaises(RuntimeError):
            self.store.record_message(message())



    def test_close_under_database_lock_waits_for_accepted_write_and_rejects_later_work(self):
        blocker = sqlite3.connect(self.store.db_path)
        results = []
        producer = threading.Thread(target=lambda: results.append(self.store.record_message(message(message_id="accepted"))))
        try:
            blocker.execute("BEGIN IMMEDIATE")
            producer.start()
            deadline = time.monotonic() + 2
            while not self.store.metrics()["pending_requests"] and time.monotonic() < deadline:
                time.sleep(0.005)
            self.assertEqual(self.store.metrics()["pending_requests"], 1)
            self.assertFalse(self.store.close(timeout=0.02))
            self.assertEqual(results, [])
            with self.assertRaisesRegex(RuntimeError, "消息库已关闭"):
                self.store.record_message(message(message_id="too-late"))
        finally:
            blocker.rollback()
            blocker.close()
            producer.join(5)
        self.assertFalse(producer.is_alive())
        self.assertTrue(self.store.close(timeout=2))
        self.assertEqual(results, [True])
        self.assertEqual([row["message_id"] for row in self.rows()], ["accepted"])


if __name__ == "__main__":
    unittest.main()
