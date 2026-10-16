"""Offline differential contract against the frozen 0.1.1 Python decoder.

Run after building qingkan_native and putting its .pyd on PYTHONPATH. Fixtures
are synthetic protobufs; this test never contacts Douyin or opens user data.
"""
import gzip
import importlib.util
import json
from contextlib import nullcontext
from pathlib import Path
import random
import sys
import unittest
from unittest.mock import Mock

import qingkan_native as native

ROOT = Path(__file__).resolve().parents[1]
BASELINE = ROOT / "legacy" / "python-reference"
sys.path.insert(0, str(BASELINE))
spec = importlib.util.spec_from_file_location("baseline_protocol_engine", BASELINE / "liveMan.py")
legacy = importlib.util.module_from_spec(spec)
try:
    spec.loader.exec_module(legacy)
finally:
    sys.path.remove(str(BASELINE))
p = legacy  # legacy imports the exact baseline protobuf definitions.

METHODS = {
    "WebcastChatMessage": "_parseChatMsg", "WebcastGiftMessage": "_parseGiftMsg",
    "WebcastLikeMessage": "_parseLikeMsg", "WebcastMemberMessage": "_parseMemberMsg",
    "WebcastSocialMessage": "_parseSocialMsg", "WebcastFansclubMessage": "_parseFansclubMsg",
    "WebcastEmojiChatMessage": "_parseEmojiChatMsg", "WebcastControlMessage": "_parseControlMsg",
    "WebcastRoomUserSeqMessage": "_parseCurrentViewers",
}


def reference(method, payload, room="7696406817731447603"):
    events = []
    fetcher = legacy.DouyinLiveWebFetcher("123", on_message=lambda kind, data: events.append({"type": kind, "payload": data}))
    fetcher._DouyinLiveWebFetcher__room_id = room
    try:
        if method in METHODS:
            getattr(fetcher, METHODS[method])(payload)
        return events[0] if events else None
    finally:
        fetcher.stop()


def reference_frame(data, room="7696406817731447603"):
    events = []
    fetcher = legacy.DouyinLiveWebFetcher("123", on_message=lambda kind, payload: events.append({"type": kind, "payload": payload}))
    fetcher._DouyinLiveWebFetcher__room_id = room
    socket = Mock()
    try:
        fetcher._wsOnMessage(socket, data)
        return (socket.send.call_args.args[0] if socket.send.called else None, events)
    finally:
        fetcher.stop()


def frame(messages, *, ack=True, log_id=2**64 - 1, ext="synthetic-ack-😀"):
    response = p.Response(messages_list=[p.Message(method=method, payload=payload) for method, payload in messages],
                          need_ack=ack, internal_ext=ext)
    return bytes(p.PushFrame(log_id=log_id, payload=gzip.compress(bytes(response), mtime=0)))


class ProtocolDifferentialTests(unittest.TestCase):
    def compare(self, method, message, room="7696406817731447603"):
        payload = message if isinstance(message, bytes) else bytes(message)
        expected = reference(method, payload, room)
        actual = json.loads(native.decode_message(method, payload, room))
        self.assertEqual(expected, actual)
        return actual

    def test_all_content_fields_match_baseline_including_max_u64(self):
        common = p.Common(msg_id=2**64 - 1, room_id=2**64 - 2, create_time=1800000000123)
        user = p.User(id=2**64 - 3, nick_name='显示名😀\\"\n')
        cases = [
            ("WebcastChatMessage", p.ChatMessage(common=common, user=user, content="文字😀\n第二行")),
            ("WebcastGiftMessage", p.GiftMessage(common=common, user=user, gift=p.GiftStruct(name="花"), combo_count=2**64 - 1)),
            ("WebcastLikeMessage", p.LikeMessage(common=common, user=user, count=2**64 - 1)),
            ("WebcastMemberMessage", p.MemberMessage(common=common, user=user)),
            ("WebcastSocialMessage", p.SocialMessage(common=common, user=user, action=2**64 - 1)),
            ("WebcastFansclubMessage", p.FansclubMessage(common_info=common, user=user, content="加入粉丝团")),
            ("WebcastEmojiChatMessage", p.EmojiChatMessage(common=common, user=user, default_content="[表情]")),
        ]
        for method, message in cases:
            with self.subTest(method=method):
                event = self.compare(method, message)
                self.assertEqual(str(2**64 - 3), event["payload"]["sender_id"])
                self.assertEqual(str(2**64 - 1), event["payload"]["message_id"])

    def test_missing_fields_and_fallback_room_are_not_invented(self):
        for method in METHODS:
            for room in ("", "0", " 000123 ", "nickname", "9" * 32):
                with self.subTest(method=method, room=room):
                    self.compare(method, b"", room)

    def test_time_seconds_milliseconds_missing_and_priority(self):
        for stamp in (0, 1, 123, 1800000000, 1800000000123, 99999999999,
                      100000000000, 32536850399, 32536850400,
                      99999999999999, 100000000000000, 2**64 - 1):
            for common_stamp in (0, 1800000123456):
                common = p.Common(create_time=common_stamp)
                with self.subTest(stamp=stamp, common_stamp=common_stamp):
                    self.compare("WebcastChatMessage", p.ChatMessage(common=common, event_time=stamp))
                    self.compare("WebcastGiftMessage", p.GiftMessage(common=common, send_time=stamp))

    def test_gift_count_priority_and_social_action_description(self):
        for combo, repeat, group in ((3, 2, 1), (0, 2, 1), (0, 0, 1), (0, 0, 0)):
            self.compare("WebcastGiftMessage", p.GiftMessage(combo_count=combo, repeat_count=repeat, group_count=group))
        for description in ("", "关注了主播", "分享直播", "未知操作"):
            self.compare("WebcastSocialMessage", p.SocialMessage(action=999, common=p.Common(describe=description)))

    def test_current_viewers_presence_exactness_and_no_cumulative_fallback(self):
        for count in (-1, 0, 37, 2**53 - 1, 2**53, 2**63 - 1):
            value = bytes(p.RoomUserSeqMessage(total=count, online_user_for_anchor="123"))
            if count == 0:
                value += b"\x18\x00"  # Explicit scalar zero, unlike omitted proto3 default.
            self.compare("WebcastRoomUserSeqMessage", value)
        for count in ("", " 00037 ", "0", "1.2万", "-1", "37.0", "9007199254740992"):
            self.compare("WebcastRoomUserSeqMessage", p.RoomUserSeqMessage(online_user_for_anchor=count, total_user=123456))

    def test_unsupported_room_statistics_and_unknown_methods_ignore_bad_payload(self):
        for method in ("WebcastRoomStatsMessage", "WebcastRoomRankMessage", "WebcastRoomMessage",
                       "WebcastRoomStreamAdaptationMessage", "WebcastUnknownMessage"):
            self.compare(method, b"\xff")

    def test_control_is_only_emitted_for_three(self):
        for status in (-1, 0, 1, 2, 3, 4):
            self.compare("WebcastControlMessage", p.ControlMessage(status=status, common=p.Common(create_time=1800000000)))

    def test_heartbeat_and_ack_are_byte_exact(self):
        self.assertIsInstance(native.hb_frame(), bytes)
        self.assertEqual(bytes(p.PushFrame(payload_type="hb")), native.hb_frame())
        for log_id in (0, 1, 2**64 - 1):
            for ext in ("", "synthetic-token", "确认😀"):
                for ack in (False, True):
                    data = frame([], ack=ack, log_id=log_id, ext=ext)
                    expected_ack, expected_events = reference_frame(data)
                    actual_ack, events = native.decode_frame(data)
                    self.assertEqual(expected_ack, actual_ack)
                    self.assertEqual(expected_events, json.loads(events))

    def test_frame_isolation_control_barrier_and_event_order(self):
        cases = [
            [("WebcastChatMessage", b"\x1a\x02\xff"),
             ("WebcastLikeMessage", bytes(p.LikeMessage(count=5))),
             ("future", b"\xff"),
             ("WebcastChatMessage", bytes(p.ChatMessage(content="完整消息")))],
            [("WebcastMemberMessage", b""),
             ("WebcastControlMessage", bytes(p.ControlMessage(status=3))),
             ("WebcastChatMessage", bytes(p.ChatMessage(content="停止后不派发")))],
        ]
        for messages in cases:
            data = frame(messages)
            with self.assertLogs(legacy.LOGGER, level="WARNING") if messages[0][0] == "WebcastChatMessage" else nullcontext():
                expected_ack, expected_events = reference_frame(data)
            ack, events = native.decode_frame(data, "7696406817731447603")
            self.assertEqual(expected_ack, ack)
            self.assertEqual(expected_events, json.loads(events))

    def test_unknown_wire_fields_are_ignored(self):
        for suffix in (b"\xa0\x06\x01", b"\xa2\x06\x03abc", b"\xa5\x06\x00\x00\x00\x00"):
            self.compare("WebcastChatMessage", bytes(p.ChatMessage(content="已知内容")) + suffix)

    def test_deterministic_mixed_frames_match_baseline(self):
        rng = random.Random(20261015)
        methods = ["WebcastChatMessage", "WebcastLikeMessage", "WebcastGiftMessage",
                   "WebcastMemberMessage", "WebcastSocialMessage", "WebcastFansclubMessage", "WebcastEmojiChatMessage"]
        for batch in range(20):
            messages = []
            for _ in range(25):
                user = p.User(id=rng.randrange(2**64), nick_name=rng.choice(("", "张三", "😀")))
                common = p.Common(msg_id=rng.randrange(2**64), room_id=rng.choice((0, 7696406817731447603)),
                                  create_time=rng.choice((0, 1800000000, 1800000000123)))
                index = rng.randrange(len(methods))
                message = [lambda: p.ChatMessage(user=user, common=common, content=f"测试 {batch}"),
                    lambda: p.LikeMessage(user=user, common=common, count=rng.randrange(100)),
                    lambda: p.GiftMessage(user=user, common=common, gift=p.GiftStruct(name="礼物"), repeat_count=rng.randrange(4)),
                    lambda: p.MemberMessage(user=user, common=common),
                    lambda: p.SocialMessage(user=user, common=common, action=rng.randrange(10)),
                    lambda: p.FansclubMessage(user=user, common_info=common, content="粉丝团"),
                    lambda: p.EmojiChatMessage(user=user, common=common, default_content="表情")][index]()
                messages.append((methods[index], bytes(message)))
            data = frame(messages)
            expected_ack, expected_events = reference_frame(data)
            ack, events = native.decode_frame(data, "7696406817731447603")
            self.assertEqual(expected_ack, ack)
            self.assertEqual(expected_events, json.loads(events))

    def test_concatenated_gzip_members_match_baseline(self):
        raw = bytes(p.Response(need_ack=True, internal_ext="ack"))
        data = bytes(p.PushFrame(payload=gzip.compress(raw[:2]) + gzip.compress(raw[2:])))
        expected_ack, expected_events = reference_frame(data)
        ack, events = native.decode_frame(data)
        self.assertEqual(expected_ack, ack)
        self.assertEqual(expected_events, json.loads(events))

    def test_empty_gzip_and_zero_padding_match_baseline(self):
        raw = bytes(p.Response(need_ack=True, internal_ext="ack"))
        for data in (b"", bytes(p.PushFrame(payload=b"")),
                     bytes(p.PushFrame(payload=gzip.compress(raw) + b"\0\0")),
                     bytes(p.PushFrame(payload=gzip.compress(raw[:2]) + b"\0\0" + gzip.compress(raw[2:])))):
            expected_ack, expected_events = reference_frame(data)
            ack, events = native.decode_frame(data)
            self.assertEqual(expected_ack, ack)
            self.assertEqual(expected_events, json.loads(events))

    def test_duplicate_nested_fields_replace_instead_of_merge(self):
        self.compare("WebcastChatMessage", bytes(p.ChatMessage(common=p.Common(msg_id=123)))
                     + bytes(p.ChatMessage(common=p.Common(room_id=456))))
        self.compare("WebcastChatMessage", bytes(p.ChatMessage(user=p.User(id=123)))
                     + bytes(p.ChatMessage(user=p.User(nick_name="名字"))))
        self.compare("WebcastGiftMessage", bytes(p.GiftMessage(gift=p.GiftStruct(name="先前名称")))
                     + bytes(p.GiftMessage(gift=p.GiftStruct(id=123))))

    def test_invalid_outer_data_has_safe_errors(self):
        for data in (b"private-invalid-frame", bytes(p.PushFrame(payload=b"private-gzip"))):
            with self.assertRaises(ValueError) as raised:
                native.decode_frame(data)
            self.assertNotIn("private", str(raised.exception))
        with self.assertRaises(ValueError):
            native.decode_message("WebcastChatMessage", b"\x1a\x02\xff")


if __name__ == "__main__":
    unittest.main()
