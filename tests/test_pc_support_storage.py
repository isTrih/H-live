"""Manual browser-support marks stay optional and scoped to saved room IDs."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python" / "collector"))
from app_storage import ConfigError, load_config, save_config


class PcSupportStorageTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.data = self.root / "data"
        self.legacy = self.root / "legacy.json"

    def load(self):
        return load_config(self.data, self.legacy)

    def test_marks_are_deduplicated_and_limited_to_existing_anchors(self):
        config = {"anchors": {"111": "甲", "222": "乙"},
                  "pc_unsupported_rids": ["222", "999", "111", "222"],
                  "custom_setting": {"keep": True}}
        original = copy.deepcopy(config)
        save_config(config, self.data)
        loaded = self.load()
        self.assertEqual(["222", "111"], loaded["pc_unsupported_rids"])
        self.assertEqual(config["anchors"], loaded["anchors"])
        self.assertEqual({"keep": True}, loaded["custom_setting"])
        self.assertEqual(original, config)

    def test_absent_key_stays_absent_for_new_and_existing_configurations(self):
        fresh = self.load()
        self.assertEqual({}, fresh["anchors"])
        self.assertNotIn("pc_unsupported_rids", fresh)
        for anchors in ({}, {"100000000000": "已存主播", "100000001000": "已存主播二", "100000002000": "已存主播三"}):
            with self.subTest(anchors=anchors):
                save_config({"anchors": anchors, "first_run_complete": True}, self.data)
                loaded = self.load()
                self.assertEqual(anchors, loaded["anchors"])
                self.assertNotIn("pc_unsupported_rids", loaded)
                self.assertNotIn("pc_unsupported_rids", json.loads((self.data / "config.json").read_text(encoding="utf-8")))

    def test_removing_one_anchor_cleans_only_its_mark(self):
        save_config({"anchors": {"111": "甲", "222": "乙"},
                     "pc_unsupported_rids": ["111", "222"], "extra": 7}, self.data)
        updated = self.load()
        del updated["anchors"]["111"]
        save_config(updated, self.data)
        loaded = self.load()
        self.assertEqual({"222": "乙"}, loaded["anchors"])
        self.assertEqual(["222"], loaded["pc_unsupported_rids"])
        self.assertEqual(7, loaded["extra"])
        loaded["anchors"]["111"] = "重新添加"
        save_config(loaded, self.data)
        self.assertEqual(["222"], self.load()["pc_unsupported_rids"])

    def test_explicit_empty_list_roundtrips_as_cleared(self):
        save_config({"anchors": {"111": "甲"}, "pc_unsupported_rids": ["111"]}, self.data)
        loaded = self.load()
        loaded["pc_unsupported_rids"] = []
        save_config(loaded, self.data)
        self.assertEqual([], self.load()["pc_unsupported_rids"])
        self.assertEqual({"111": "甲"}, self.load()["anchors"])

    def test_invalid_marks_have_safe_errors_and_preserve_previous_file(self):
        save_config({"anchors": {"111": "甲"}, "pc_unsupported_rids": ["111"]}, self.data)
        target = self.data / "config.json"
        before = target.read_bytes()
        bad_values = (None, True, "111", {"111": True}, [111], [True], [None], [""],
                      ["１２３"], ["١٢٣"], ["1" * 26], [" 111 "], ["111\n"],
                      ["https://live.douyin.com/111"], ["cookie=SYNTHETIC_PRIVATE_VALUE"],
                      [{"password": "SYNTHETIC_PRIVATE_VALUE"}])
        for value in bad_values:
            with self.subTest(kind=type(value).__name__), self.assertRaises(ConfigError) as caught:
                save_config({"anchors": {"111": "甲"}, "pc_unsupported_rids": value}, self.data)
            self.assertEqual("非 PC 支持标记配置无效，应为数字直播间 ID 列表", str(caught.exception))
            self.assertEqual(before, target.read_bytes())

    def test_load_validates_marks_and_legacy_migration_preserves_original(self):
        self.legacy.write_text(json.dumps({"anchors": {"111": "甲"},
            "pc_unsupported_rids": ["111", "222", "111"], "extra": "kept"}), encoding="utf-8")
        original = self.legacy.read_bytes()
        loaded = self.load()
        self.assertEqual(["111"], loaded["pc_unsupported_rids"])
        self.assertEqual("kept", loaded["extra"])
        self.assertEqual(original, self.legacy.read_bytes())
        self.assertEqual(["111"], self.load()["pc_unsupported_rids"])
        target = self.data / "config.json"
        target.write_text(json.dumps({"anchors": {}, "pc_unsupported_rids": ["secret=PRIVATE"]}), encoding="utf-8")
        invalid_bytes = target.read_bytes()
        with self.assertRaises(ConfigError) as caught:
            self.load()
        self.assertNotIn("PRIVATE", str(caught.exception))
        self.assertEqual(invalid_bytes, target.read_bytes())


if __name__ == "__main__":
    unittest.main()
