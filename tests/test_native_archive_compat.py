"""Observable compatibility of the Rust adapter with an existing 0.1.1 archive."""
from pathlib import Path
import importlib.util
import json
import sqlite3
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python" / "collector"))
from archive_store import ArchiveStore
from message_export import export_month, ExportCancelled
from qingkan_native import CancelToken


def legacy_store():
    spec = importlib.util.spec_from_file_location("archive_reference_for_compat", ROOT / "legacy" / "python-reference" / "archive_store.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module.ArchiveStore


def record(message_id):
    return dict(anchor_id="9007199254740993123", anchor_name="主播", web_rid="111", session_id="7696456021958609673",
                sender_id="00123456789012345678901234567890", sender_name="观众😀", message_type="chat", content="原文",
                message_time="2027-01-01T00:00:00.000+08:00", received_at="2027-01-01T00:00:01.000+08:00",
                time_source="server", message_id=message_id, payload={"original": "原文"})


class NativeArchiveCompatibilityTests(unittest.TestCase):
    def test_close_rejects_unrepresentable_wait_without_panicking(self):
        with tempfile.TemporaryDirectory() as directory:
            store = ArchiveStore(directory)
            try:
                for value in (float("nan"), -1, 1e300, 1e18):
                    with self.assertRaises(ValueError):
                        store.close(timeout=value)
            finally:
                store.close()

    def test_old_archive_migrates_preserving_data_but_old_app_cannot_downgrade(self):
        with tempfile.TemporaryDirectory() as directory:
            old = legacy_store()(directory)
            self.assertTrue(old.record_message(record("old"), title="标题"))
            old.close()
            new = ArchiveStore(directory)
            self.assertEqual(new.list_anchors()[0]["anchor_id"], record("old")["anchor_id"])
            self.assertFalse(new.record_message(record("old")))
            self.assertTrue(new.record_message(record("new")))
            new.close()
            with self.assertRaisesRegex(RuntimeError, "版本较新"):
                legacy_store()(directory)
            reopened = ArchiveStore(directory)
            try:
                rows = list(reopened.iter_messages(None, "2027-01"))
                self.assertEqual([row["message_id"] for row in rows], ["old", "new"])
                for row in rows:
                    self.assertEqual(row["sender_id"], record("old")["sender_id"])
                    self.assertEqual(json.loads(row["payload_json"]), {"original": "原文"})
                with sqlite3.connect(Path(directory) / "messages.sqlite3") as connection:
                    self.assertEqual(connection.execute("PRAGMA user_version").fetchone()[0], 3)
                    self.assertEqual(connection.execute("PRAGMA integrity_check").fetchone()[0], "ok")
                connection.close()
            finally:
                reopened.close()

    def test_native_cancellation_token_preserves_preexisting_output(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "cancel.xlsx"
            target.write_bytes(b"original")
            token = CancelToken()
            token.set()
            self.assertTrue(token.is_set())
            with self.assertRaises(ExportCancelled):
                export_month(directory, ["1"], "2027-01", target, cancel_event=token)
            self.assertEqual(target.read_bytes(), b"original")
            self.assertEqual(list(Path(directory).glob("*.tmp")), [])


if __name__ == "__main__":
    unittest.main()
