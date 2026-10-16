"""Persistent checkpoint boundary; elapsed intervals are owned by DurationLedger."""
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python" / "collector"))
from archive_store import ArchiveStore


class MonitorDurationStorageTests(unittest.TestCase):
    def test_checkpoint_is_monotonic_and_survives_reopen_without_creating_archive_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            with ArchiveStore(directory) as store:
                self.assertEqual(store.load_monitor_duration("11", "101"), 0)
                self.assertIsNone(store.save_monitor_duration("11", "101", 12.25))
                store.save_monitor_duration("11", "101", 4.0)
                self.assertEqual(store.load_monitor_duration("11", "101"), 12.25)
                self.assertEqual(store.list_anchors(), [])
                self.assertEqual(store.load_monitor_duration("11", "102"), 0)
            with ArchiveStore(directory) as reopened:
                self.assertEqual(reopened.load_monitor_duration("11", "101"), 12.25)
                reopened.execute_delete(dict(anchor_ids=["11"]))
                self.assertEqual(reopened.load_monitor_duration("11", "101"), 12.25)

    def test_invalid_inputs_leave_checkpoint_unchanged(self):
        with tempfile.TemporaryDirectory() as directory:
            with ArchiveStore(directory) as store:
                store.save_monitor_duration("11", "101", 3)
                for seconds in (True, "4", None, -1, float("inf"), float("nan"), 10**1000):
                    with self.subTest(value=type(seconds).__name__), self.assertRaises(ValueError):
                        store.save_monitor_duration("11", "101", seconds)
                for anchor, session in (("", "101"), ("11", "0"), ("nickname", "101")):
                    with self.assertRaises(ValueError):
                        store.save_monitor_duration(anchor, session, 4)
                    with self.assertRaises(ValueError):
                        store.load_monitor_duration(anchor, session)
                self.assertEqual(store.load_monitor_duration("11", "101"), 3)


if __name__ == "__main__":
    unittest.main()
