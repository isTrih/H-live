import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python" / "collector"))

import app_storage
from app_storage import ConfigError, app_data_dir, load_config, parse_room_id, save_config


class StorageTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.data = self.root / "app"
        self.legacy = self.root / "legacy.json"

    def tearDown(self):
        self.temporary.cleanup()

    def test_room_ids_and_supported_links(self):
        inputs = {
            " 100000000000 ": "100000000000",
            "https://live.douyin.com/100000001000": "100000001000",
            "https://live.douyin.com/100000002000/?foo=bar": "100000002000",
            "https://www.douyin.com/follow/live/100000000000?anchor_id=ignored": "100000000000",
            "https://douyin.com/follow/live/100000001000": "100000001000",
        }
        for value, expected in inputs.items():
            with self.subTest(value=value):
                self.assertEqual(parse_room_id(value), expected)

    def test_reject_untrusted_hosts_paths_ports_and_non_ascii_ids(self):
        for value in ("", "../123", "..\\123", "１２３", "١٢٣", "12 34", "1" * 26,
                      "http://live.douyin.com/123", "https://evil.example/123",
                      "https://live.douyin.com.evil.example/123", "https://live.douyin.com@evil.example/123",
                      "https://evil@live.douyin.com/123", "https://live.douyin.com:443/123",
                      "https://live.douyin.com/123/456", "https://live.douyin.com/%31%32%33",
                      "https://www.douyin.com/123", "https://www.douyin.com/follow/live/../123"):
            with self.subTest(value=value):
                with self.assertRaises(ValueError):
                    parse_room_id(value)

    def test_data_directory_override(self):
        with patch.dict(os.environ, {"DOUYIN_MONITOR_HOME": str(self.data)}):
            self.assertEqual(app_data_dir(), self.data.resolve())

    def test_first_run_empty_config(self):
        self.assertEqual(load_config(self.data, self.legacy), {"anchors": {}, "cookie": "", "anchor_cookies": {}, "anchor_metadata": {}, "proxy_pool": app_storage.default_proxy_pool(), "imported_proxies": [], "proxy_subscriptions": []})

    def test_roundtrip_unicode_and_custom_first_run_flag(self):
        config = {"anchors": {"111": "中文主播"}, "cookie": "", "first_run_complete": True}
        save_config(config, self.data)
        loaded = load_config(self.data, self.legacy)
        self.assertEqual(loaded["anchors"], config["anchors"])
        self.assertTrue(loaded["first_run_complete"])
        self.assertEqual(loaded["cookie"], "")
        self.assertNotIn("cookie", json.loads((self.data / "config.json").read_text(encoding="utf-8")))
        self.assertEqual(config["cookie"], "")

    @unittest.skipUnless(os.name == "nt", "DPAPI requires Windows")
    def test_cookie_is_dpapi_encrypted_and_roundtrips(self):
        secret = "sessionid=DO_NOT_STORE_AS_PLAIN; ttwid=中文令牌"
        per_room = {"111": "sessionid=ROOM_ONE_SECRET", "222": "sessionid=ROOM_TWO_SECRET"}
        save_config({"anchors": {"111": "主播"}, "cookie": secret, "anchor_cookies": per_room}, self.data)
        raw = (self.data / "config.json").read_text(encoding="utf-8")
        self.assertNotIn(secret, raw)
        self.assertNotIn("DO_NOT_STORE_AS_PLAIN", raw)
        self.assertIn("cookie_encrypted", raw)
        self.assertNotIn("ROOM_ONE_SECRET", raw)
        self.assertNotIn("ROOM_TWO_SECRET", raw)
        self.assertNotIn('"anchor_cookies":', raw)
        loaded = load_config(self.data, self.legacy)
        self.assertEqual(loaded["cookie"], secret)
        self.assertEqual(loaded["anchor_cookies"], per_room)

    def test_dpapi_failure_never_writes_plaintext_or_overwrites_good_config(self):
        save_config({"anchors": {"111": "旧主播"}}, self.data)
        previous = (self.data / "config.json").read_bytes()
        with patch.object(app_storage, "_crypt_cookie", side_effect=ConfigError("加密失败")):
            with self.assertRaises(ConfigError):
                save_config({"anchors": {"222": "新主播"}, "cookie": "PRIVATE_SECRET"}, self.data)
        self.assertEqual((self.data / "config.json").read_bytes(), previous)
        self.assertEqual(list(self.data.glob("*.tmp")), [])

    def test_atomic_replace_failure_preserves_previous_configuration(self):
        save_config({"anchors": {"111": "旧主播"}}, self.data)
        previous = (self.data / "config.json").read_bytes()
        with patch.object(app_storage.os, "replace", side_effect=PermissionError("denied")):
            with self.assertRaises(ConfigError):
                save_config({"anchors": {"222": "新主播"}}, self.data)
        self.assertEqual((self.data / "config.json").read_bytes(), previous)
        self.assertEqual(list(self.data.glob("*.tmp")), [])

    def test_adjacent_legacy_file_migrates_once_and_is_preserved(self):
        legacy = {"anchors": {"111": "原主播"}, "cookie": "", "extra_setting": 7}
        content = json.dumps(legacy, ensure_ascii=False)
        self.legacy.write_text(content, encoding="utf-8")
        loaded = load_config(self.data, self.legacy)
        self.assertEqual(loaded["anchors"], legacy["anchors"])
        self.assertEqual(loaded["extra_setting"], 7)
        self.assertEqual(self.legacy.read_text(encoding="utf-8"), content)
        save_config({"anchors": {}, "first_run_complete": True}, self.data)
        self.assertEqual(load_config(self.data, self.legacy)["anchors"], {})

    def test_invalid_saved_config_is_not_overwritten(self):
        self.data.mkdir()
        target = self.data / "config.json"
        target.write_text("{broken", encoding="utf-8")
        with self.assertRaises(ConfigError):
            load_config(self.data, self.legacy)
        self.assertEqual(target.read_text(encoding="utf-8"), "{broken")

    def test_bad_ids_and_multiline_cookie_are_rejected(self):
        for config in ({"anchors": {"../../outside": "x"}}, {"anchors": []},
                       {"cookie": "key=value\r\nInjected: yes"}, {"cookie": None},
                       {"anchor_cookies": {"../../bad": "secret"}},
                       {"anchor_cookies": {"111": "key=value\r\nInjected: yes"}}):
            with self.subTest(config=config):
                with self.assertRaises(ConfigError):
                    save_config(config, self.data)
        self.assertFalse((self.data / "config.json").exists())

    def test_corrupt_encrypted_cookie_has_safe_error(self):
        self.data.mkdir()
        (self.data / "config.json").write_text(
            '{"anchors":{},"cookie_encrypted":"PRIVATE_SECRET!"}', encoding="utf-8")
        with self.assertRaises(ConfigError) as caught:
            load_config(self.data, self.legacy)
        self.assertNotIn("PRIVATE_SECRET", str(caught.exception))

    def test_anchor_identity_mapping_survives_entry_changes_and_roundtrips(self):
        metadata = {"111": {"anchor_id": "9007199254740993123", "anchor_name": "真实主播"},
                    "222": {"anchor_id": "9007199254740993123", "anchor_name": "真实主播"}}
        save_config({"anchors": {"111": "旧入口", "222": "新入口"}, "anchor_metadata": metadata}, self.data)
        self.assertEqual(load_config(self.data, self.legacy)["anchor_metadata"], metadata)
        with self.assertRaises(ConfigError):
            save_config({"anchor_metadata": {"111": {"anchor_id": "nickname"}}}, self.data)

    def test_proxy_pool_defaults_enabled_preserves_disable_and_rejects_other_sources(self):
        self.assertTrue(load_config(self.data, self.legacy)["proxy_pool"]["enabled"])
        save_config({"proxy_pool": {"provider": "scdn", "enabled": False, "api_url": ""}}, self.data)
        self.assertTrue(load_config(self.data, self.legacy)["proxy_pool"]["enabled"])
        save_config({"proxy_pool": {**app_storage.default_proxy_pool(), "enabled": False}}, self.data)
        self.assertEqual(load_config(self.data, self.legacy)["proxy_pool"], {**app_storage.default_proxy_pool(), "enabled": False})
        save_config({"proxy_pool": {"provider": "scdn", "enabled": True, "api_url": ""}}, self.data)
        self.assertEqual(load_config(self.data, self.legacy)["proxy_pool"], app_storage.default_proxy_pool())
        for proxy in ({"enabled": 1}, {"provider": "other"}, {"api_url": "http://127.0.0.1/api"},
                      {"schema_version": 2}, {"protocol": "http"}):
            with self.assertRaises(ConfigError):
                save_config({"proxy_pool": proxy}, self.data)


if __name__ == "__main__":
    unittest.main()
