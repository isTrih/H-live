"""Activity archive contract through the Python/native application boundary."""
import csv
from contextlib import closing
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python" / "collector"))
from archive_store import ArchiveStore, CONTENT_TYPES
from message_export import export_month, MESSAGE_TYPE_LABELS


def activity(message_id="activity", **changes):
    row = dict(anchor_id="11", anchor_name="主播", web_rid="111", session_id="101",
               sender_id="", sender_name="", message_type="activity", content="福袋：礼物 3 份",
               message_time="2026-10-01T00:00:00+08:00", received_at="2026-10-01T00:00:01+08:00",
               time_source="server", message_id=message_id,
               payload={"activity_kind": "lottery", "activity_id": "001234567890123456789", "prize_count": 3})
    row.update(changes)
    return row


class ActivityArchiveTests(unittest.TestCase):
    def test_activity_is_a_content_type_and_default_exports_keep_unknown_sender_empty(self):
        self.assertIn("activity", CONTENT_TYPES)
        self.assertEqual(MESSAGE_TYPE_LABELS["activity"], "红包/福袋")
        with tempfile.TemporaryDirectory() as directory:
            with ArchiveStore(directory) as store:
                self.assertTrue(store.record_message(activity()))
                self.assertFalse(store.record_message(activity()))
                rows = list(store.iter_messages(["11"], "2026-10", ["activity"]))
                self.assertEqual(len(rows), 1)
                self.assertEqual(json.loads(rows[0]["payload_json"]), activity()["payload"])
                for suffix in ("csv", "xlsx"):
                    path = Path(directory) / f"activities.{suffix}"
                    report = export_month(directory, ["11"], "2026-10", path)
                    self.assertEqual((report["row_count"], report["missing_sender_count"]), (1, 1))
                    if suffix == "csv":
                        with path.open(encoding="utf-8-sig", newline="") as stream:
                            values = list(csv.reader(stream))[1]
                    else:
                        with ZipFile(path) as z:
                            doc = ET.fromstring(z.read("xl/worksheets/sheet1.xml"))
                        ns = {"s": "http://schemas.openxmlformats.org/spreadsheetml/2006/main"}
                        cells = doc.findall("s:sheetData/s:row", ns)[1]
                        values = [cell.find("s:is/s:t", ns).text or "" for cell in cells]
                    self.assertEqual(values[2:6], ["", "", "红包/福袋", "福袋：礼物 3 份"])

    def test_activity_month_filters_and_deletion_keep_other_month(self):
        with tempfile.TemporaryDirectory() as directory:
            with ArchiveStore(directory) as store:
                store.record_message(activity("oct", message_time="2026-09-30T16:00:00Z"))
                store.record_message(activity("nov", message_time="", time_source="missing",
                                              received_at="2026-11-01T00:00:00+08:00"))
                self.assertEqual([r["message_id"] for r in store.iter_messages(["11"], "2026-10")], ["oct"])
                selection = dict(anchor_ids=["11"], month="2026-10", session_ids=["101"])
                self.assertEqual(store.preview_delete(selection)["records_count"], 1)
                self.assertEqual(store.execute_delete(selection)["deleted_records"], 1)
                remaining = list(store.iter_messages(["11"], "2026-11"))
                self.assertEqual([r["message_id"] for r in remaining], ["nov"])
                self.assertEqual(remaining[0]["message_time"], "")
                with closing(sqlite3.connect(store.path)) as c:
                    self.assertEqual(c.execute("PRAGMA user_version").fetchone()[0], 3)
                    self.assertEqual(c.execute("PRAGMA foreign_key_check").fetchall(), [])


if __name__ == "__main__":
    unittest.main()
