"""Offline protocol/lifecycle regressions; no real Douyin account or network."""
import gzip
import os
import tempfile
import threading
import time
import unittest
from unittest.mock import Mock, patch
from urllib.parse import parse_qs, urlparse, quote

import requests

import liveMan as engine
from protobuf.douyin import (ChatMessage, Common, ControlMessage, EmojiChatMessage, FansclubMessage,
                             GiftMessage, GiftStruct, LikeMessage, MemberMessage, Message,
                             PushFrame, Response, RoomUserSeqMessage, SocialMessage, User)


def response(*, text='{"roomId":"123456"}', data=None, cookies=None):
    result = Mock()
    result.text = text
    result.cookies = cookies or {}
    result.json.return_value = data
    return result


def deliver(fetcher, method, payload):
    """Exercise the same compressed-frame interface used by live sockets."""
    body = Response(messages_list=[Message(method=method, payload=payload)])
    fetcher._wsOnMessage(Mock(), bytes(PushFrame(payload=gzip.compress(bytes(body)))))


class EngineTests(unittest.TestCase):
    def setUp(self):
        self.session = Mock()
        self.session.cookies = {}
        self.session.get.return_value = response()
        self.session_patch = patch.object(engine.requests, "Session", return_value=self.session)
        self.session_patch.start()
        self.addCleanup(self.session_patch.stop)
        self.statuses = []
        self.fetcher = engine.DouyinLiveWebFetcher(
            "123", cookie="test_session=synthetic", on_status=lambda state, detail: self.statuses.append((state, detail)))
        self.addCleanup(self.fetcher.stop)

    def states(self):
        return [state for state, _ in self.statuses]

    def test_stop_before_start_performs_no_network(self):
        self.fetcher.stop()
        self.fetcher.stop()
        self.fetcher.start()
        self.session.get.assert_not_called()
        self.assertEqual(["closed"], self.states())

    def test_stop_during_http_prevents_socket_connection(self):
        requested, release = threading.Event(), threading.Event()
        failures = []

        def blocked_request(*args, **kwargs):
            requested.set()
            self.assertTrue(release.wait(2))
            return response()

        def worker():
            try:
                self.fetcher.start()
            except Exception as error:
                failures.append(error)

        self.session.get.side_effect = blocked_request
        with patch.object(engine.websocket, "WebSocketApp") as factory:
            thread = threading.Thread(target=worker)
            thread.start()
            self.assertTrue(requested.wait(1))
            self.fetcher.stop()
            release.set()
            thread.join(2)
            self.assertFalse(thread.is_alive())
            factory.assert_not_called()
        self.assertEqual([], failures)
        self.assertEqual(["connecting", "closed"], self.states())
        self.session.close.assert_called()

    def test_stop_during_signature_prevents_socket_connection(self):
        def signature(*args, **kwargs):
            self.fetcher.stop()
            return "test-signature"

        with patch.object(engine, "generateSignature", side_effect=signature), \
                patch.object(engine.websocket, "WebSocketApp") as factory:
            self.fetcher.start()
            factory.assert_not_called()
        self.assertNotIn("connected", self.states())

    def test_http_error_is_raised_with_status_and_session_closed(self):
        result = response()
        http_response = requests.Response()
        http_response.status_code = 403
        result.raise_for_status.side_effect = requests.HTTPError(response=http_response)
        self.session.get.return_value = result
        with self.assertRaisesRegex(engine.LiveMonitorError, "HTTP 403"):
            self.fetcher.start()
        self.assertEqual(["connecting", "error", "closed"], self.states())
        self.session.close.assert_called()

    def test_timeout_is_bounded_and_reported(self):
        self.session.get.side_effect = requests.Timeout()
        with self.assertRaisesRegex(engine.LiveMonitorError, "超时"):
            self.fetcher.start()
        self.assertEqual(engine.HTTP_TIMEOUT, self.session.get.call_args.kwargs["timeout"])

    def test_cookie_is_used_for_room_lookup(self):
        self.assertEqual("123456", self.fetcher.room_id)
        self.assertEqual("test_session=synthetic", self.session.get.call_args.kwargs["headers"]["Cookie"])
        self.assertEqual(1, self.session.get.call_count)

    def test_escaped_room_id_is_supported(self):
        self.session.get.return_value = response(text=r'{\"roomId\":\"987654\"}')
        self.assertEqual("987654", self.fetcher.room_id)

    def test_missing_room_id_is_an_error(self):
        self.session.get.return_value = response(text="verification required")
        with self.assertRaisesRegex(engine.LiveMonitorError, "roomId"):
            _ = self.fetcher.room_id

    def test_visitor_cookie_uses_valid_semicolon_separator(self):
        self.fetcher.cookie = ""
        self.session.get.side_effect = [response(cookies={"ttwid": "synthetic"}), response()]
        self.assertEqual("123456", self.fetcher.room_id)
        cookie = self.session.get.call_args.kwargs["headers"]["Cookie"]
        self.assertTrue(cookie.startswith("ttwid=synthetic; msToken="))
        self.assertNotIn("&msToken", cookie)

    def test_missing_visitor_credential_is_an_error(self):
        self.fetcher.cookie = ""
        with self.assertRaisesRegex(engine.LiveMonitorError, "访客凭据"):
            _ = self.fetcher.ttwid

    def test_websocket_callback_error_is_not_swallowed(self):
        ws = Mock()

        def run(**kwargs):
            self.fetcher._wsOnError(ws, ConnectionRefusedError("synthetic sensitive data"))
            return True

        ws.run_forever.side_effect = run
        with patch.object(engine, "generateSignature", return_value="synthetic"), \
                patch.object(engine.websocket, "WebSocketApp", return_value=ws):
            with self.assertRaisesRegex(engine.LiveMonitorError, "ConnectionRefusedError") as raised:
                self.fetcher.start()
        self.assertNotIn("sensitive", str(raised.exception))
        self.assertEqual(["connecting", "error", "closed"], self.states())

    def test_closed_before_handshake_is_not_success(self):
        ws = Mock()
        ws.run_forever.return_value = False
        with patch.object(engine, "generateSignature", return_value="synthetic"), \
                patch.object(engine.websocket, "WebSocketApp", return_value=ws):
            with self.assertRaisesRegex(engine.LiveMonitorError, "握手成功前"):
                self.fetcher.start()
        self.assertNotIn("connected", self.states())

    def test_live_timestamp_and_per_socket_timeout(self):
        captured = {}
        ws = Mock()

        def factory(url, **kwargs):
            captured["url"] = url
            captured.update(kwargs)
            return ws

        def run(**kwargs):
            captured["headers"] = captured["header"]()
            self.fetcher.stop()
            captured["on_open"](ws)
            return False

        ws.run_forever.side_effect = run
        with patch.object(engine.time, "time", return_value=1800000000.123), \
                patch.object(engine, "generateSignature", return_value="a+b="), \
                patch.object(engine.websocket, "WebSocketApp", side_effect=factory):
            self.fetcher.start()
        params = parse_qs(urlparse(captured["url"]).query)
        self.assertIn("1800000000123", params["cursor"][0])
        self.assertIn("first_req_ms:1800000000123", params["internal_ext"][0])
        self.assertEqual(["a+b="], params["signature"])
        self.assertEqual("test_session=synthetic", captured["headers"]["cookie"])
        ws.sock.settimeout.assert_called_once_with(10)
        self.assertNotIn("connected", self.states())

    def test_heartbeat_uses_shared_daemon_pool_and_unregisters_on_stop(self):
        sent = threading.Event()
        self.fetcher.ws = Mock()
        self.fetcher.ws.send.side_effect = lambda *args: sent.set()
        self.fetcher._wsOnOpen(self.fetcher.ws)
        self.assertTrue(sent.wait(1))
        heartbeat_threads = [thread for thread in threading.enumerate() if thread.name.startswith("heartbeat-")]
        self.assertLessEqual(len(heartbeat_threads), 9)
        self.assertTrue(all(thread.daemon for thread in heartbeat_threads))
        self.assertTrue(engine._HEARTBEATS.contains(self.fetcher))
        self.fetcher.stop()
        self.assertFalse(engine._HEARTBEATS.contains(self.fetcher))
        calls = self.fetcher.ws.send.call_count
        self.fetcher._sendHeartbeatOnce()
        self.assertEqual(calls, self.fetcher.ws.send.call_count)
        self.fetcher.ws.sock.abort.assert_called()
        self.fetcher.ws.close.assert_called_with(timeout=0)

    def test_independent_instances_keep_separate_cancellation_and_messages(self):
        first_callback, second_callback = Mock(), Mock()
        first = self.fetcher
        first.on_message = first_callback
        second_session = Mock()
        with patch.object(engine.requests, "Session", return_value=second_session):
            second = engine.DouyinLiveWebFetcher("456", on_message=second_callback)
        self.addCleanup(second.stop)
        first.stop()
        self.assertFalse(second._stop_event.is_set())
        self.assertIsNot(first.session, second.session)
        deliver(second, "WebcastChatMessage", bytes(ChatMessage(user=User(nick_name="测试用户"), content="第二个直播间", event_time=123)))
        first_callback.assert_not_called()
        second_callback.assert_called_once()
        msg_type, payload = second_callback.call_args.args
        self.assertEqual("chat", msg_type)
        self.assertEqual("第二个直播间", payload["content"])

    def test_ack_and_chat_message_callback_contract(self):
        callback, legacy_callback = Mock(), Mock()
        self.fetcher.on_message = callback
        self.fetcher.on_chat_message = legacy_callback
        chat = ChatMessage(user=User(nick_name="测试"), content="你好", event_time=123)
        body = Response(messages_list=[Message(method="WebcastChatMessage", payload=bytes(chat))],
                        need_ack=True, internal_ext="ack-token")
        frame = PushFrame(log_id=42, payload=gzip.compress(bytes(body)))
        ws = Mock()
        self.fetcher._wsOnMessage(ws, bytes(frame))
        msg_type, payload = callback.call_args.args
        self.assertEqual("chat", msg_type)
        self.assertEqual("你好", payload["content"])
        self.assertEqual("测试", payload["user_name"])
        self.assertIn("time_str", payload)
        self.assertIn("text", payload)
        legacy_callback.assert_called_once_with("测试", "你好", 123)
        ack = PushFrame().parse(ws.send.call_args.args[0])
        self.assertEqual(42, ack.log_id)
        self.assertEqual("ack", ack.payload_type)
        self.assertEqual(b"ack-token", ack.payload)

    def test_content_fields_preserve_long_ids_and_actual_session(self):
        callback = Mock()
        self.fetcher.on_message = callback
        self.fetcher._DouyinLiveWebFetcher__room_id = "7696406817731447603"
        common = Common(msg_id=18446744073709551614, room_id=7696455970398309120, create_time=1800000000)
        chat = ChatMessage(common=common, user=User(id=18446744073709551613, nick_name="显示名"),
                           content="纯内容", event_time=1800000000123)
        deliver(self.fetcher, "WebcastChatMessage", bytes(chat))
        payload = callback.call_args.args[1]
        self.assertEqual("18446744073709551613", payload["sender_id"])
        self.assertEqual("18446744073709551614", payload["message_id"])
        self.assertEqual("7696455970398309120", payload["room_id"])
        self.assertEqual("显示名", payload["sender_name"])
        self.assertEqual("纯内容", payload["content"])
        self.assertEqual(1800000000, payload["event_time"])

    def test_missing_sender_or_send_time_is_not_fabricated(self):
        callback = Mock()
        self.fetcher.on_message = callback
        self.fetcher._DouyinLiveWebFetcher__room_id = "7696406817731447603"
        deliver(self.fetcher, "WebcastChatMessage", bytes(ChatMessage(user=User(nick_name="只有昵称"), content="你好")))
        payload = callback.call_args.args[1]
        self.assertEqual("", payload["sender_id"])
        self.assertEqual("", payload["message_id"])
        self.assertEqual("7696406817731447603", payload["room_id"])
        self.assertEqual(0, payload["event_time"])
        self.assertEqual("", payload["time_str"])
        self.fetcher._DouyinLiveWebFetcher__room_id = None
        deliver(self.fetcher, "WebcastChatMessage", bytes(ChatMessage(content="未知场次")))
        self.assertEqual("", callback.call_args.args[1]["room_id"])
        self.assertNotEqual(self.fetcher.live_id, callback.call_args.args[1]["room_id"])

    def test_all_supported_content_types_have_uniform_source_fields(self):
        callback = Mock()
        self.fetcher.on_message = callback
        user = User(id=18446744073709551613, nick_name="显示名")
        common = Common(msg_id=18446744073709551614, room_id=7696455970398309120, create_time=1800000000123)
        cases = [
            ("chat", "WebcastChatMessage", ChatMessage(common=common, user=user, content="聊天")),
            ("gift", "WebcastGiftMessage", GiftMessage(common=common, user=user, gift=GiftStruct(name="礼物"), repeat_count=1)),
            ("like", "WebcastLikeMessage", LikeMessage(common=common, user=user, count=3)),
            ("member", "WebcastMemberMessage", MemberMessage(common=common, user=user)),
            ("social", "WebcastSocialMessage", SocialMessage(common=common, user=user, action=999)),
            ("fansclub", "WebcastFansclubMessage", FansclubMessage(common_info=common, user=user, content="加入粉丝团")),
            ("emoji", "WebcastEmojiChatMessage", EmojiChatMessage(common=common, user=user, default_content="表情")),
        ]
        for kind, method, msg in cases:
            with self.subTest(kind=kind):
                deliver(self.fetcher, method, bytes(msg))
                actual_kind, payload = callback.call_args.args
                self.assertEqual(kind, actual_kind)
                self.assertEqual("18446744073709551613", payload["sender_id"])
                self.assertEqual("18446744073709551614", payload["message_id"])
                self.assertEqual("7696455970398309120", payload["room_id"])
                self.assertEqual(1800000000, payload["event_time"])
                self.assertEqual("显示名", payload["sender_name"])
                self.assertIsInstance(payload["content"], str)
                self.assertFalse(payload["content"].startswith("["))
                self.assertNotIn("【", payload["content"])
        social = next(call.args[1] for call in callback.call_args_list if call.args[0] == "social")
        self.assertNotIn("关注了主播", social["content"])

    def test_stats_room_rank_adaptation_payloads_are_not_parsed_or_emitted(self):
        callback = Mock()
        self.fetcher.on_message = callback
        # Invalid protobuf bodies prove these methods are discarded before parse.
        methods = ("WebcastRoomStatsMessage", "WebcastRoomRankMessage",
                   "WebcastRoomMessage", "WebcastRoomStreamAdaptationMessage")
        body = Response(messages_list=[Message(method=method, payload=b"\xff") for method in methods], need_ack=True)
        ws = Mock()
        with patch("builtins.print") as output:
            self.fetcher._wsOnMessage(ws, bytes(PushFrame(payload=gzip.compress(bytes(body)))))
        callback.assert_not_called()
        output.assert_not_called()
        ws.send.assert_called_once()  # Transport ACK still works.

    def test_current_viewers_are_separate_from_cumulative_counts_and_missing(self):
        callback = self.fetcher.on_message = Mock()
        message = RoomUserSeqMessage(common=Common(room_id=222), total=42, total_user=99999,
                                     total_pv_for_anchor="88888", popularity=77777)
        deliver(self.fetcher, "WebcastRoomUserSeqMessage", bytes(message))
        callback.assert_called_once_with("stats", {"viewer_count": 42, "room_id": "222"})
        callback.reset_mock()
        deliver(self.fetcher, "WebcastRoomUserSeqMessage", bytes(RoomUserSeqMessage(total_user=99999)))
        callback.assert_not_called()
        deliver(self.fetcher, "WebcastRoomUserSeqMessage", b"\x18\x00")  # explicit field 3, zero
        self.assertEqual(0, callback.call_args.args[1]["viewer_count"])

    def test_end_control_keeps_room_identity_and_stops_later_messages(self):
        callback = Mock()
        self.fetcher.on_message = callback
        common = Common(room_id=7696455970398309120, create_time=1800000000, msg_id=1234)
        control = ControlMessage(common=common, status=3)
        body = Response(messages_list=[
            Message(method="WebcastControlMessage", payload=bytes(control)),
            Message(method="WebcastChatMessage", payload=bytes(ChatMessage(content="should not follow end"))),
        ])
        self.fetcher._wsOnMessage(Mock(), bytes(PushFrame(payload=gzip.compress(bytes(body)))))
        callback.assert_called_once()
        kind, payload = callback.call_args.args
        self.assertEqual("control", kind)
        self.assertEqual("7696455970398309120", payload["room_id"])
        self.assertEqual(1800000000, payload["event_time"])
        self.assertTrue(self.fetcher._stop_event.is_set())

    def test_end_control_stops_socket_even_when_consumer_raises(self):
        self.fetcher.on_message = Mock(side_effect=RuntimeError("synthetic consumer error"))
        self.fetcher.ws = Mock()
        deliver(self.fetcher, "WebcastControlMessage", bytes(ControlMessage(status=3)))
        self.assertTrue(self.fetcher._stop_event.is_set())
        self.fetcher.ws.close.assert_called_with(timeout=0)
        self.session.close.assert_called()

    def test_timestamp_normalization_preserves_absence_and_rejects_invalid_values(self):
        for value in (None, "", True, -1, "invalid", float("nan"), 1800000000000000):
            with self.subTest(value=value):
                self.assertEqual(0, engine._epoch_seconds(value))
        self.assertEqual(1800000000, engine._epoch_seconds(1800000000))
        self.assertEqual(1800000000, engine._epoch_seconds("1800000000123"))


class MetadataTests(unittest.TestCase):
    def setUp(self):
        self.session = Mock()
        self.session.cookies = {}
        self.patcher = patch.object(engine.requests, "Session", return_value=self.session)
        self.patcher.start()
        self.addCleanup(self.patcher.stop)

    def test_live_metadata_has_identity_current_count_but_no_video_fields(self):
        self.session.get.return_value = response(data={"status_code": 0, "data": {
            "user": {"nickname": "主播", "id_str": "18446744073709551613"}, "data": [{"status": 2, "title": "直播",
            "id_str": "7696455970398309120", "start_time": 1800000000123,
            "user_count_str": "100", "stream_url": {"secret": "not returned"}}]}})
        result = engine.fetch_room_info("123", "synthetic_cookie=example")
        self.assertEqual({"nickname": "主播", "is_live": True, "title": "直播", "anchor_id": "18446744073709551613",
                          "room_id": "7696455970398309120", "started_at": 1800000000,
                          "viewer_count": 100}, result)
        self.assertEqual("synthetic_cookie=example", self.session.get.call_args.kwargs["headers"]["Cookie"])
        self.session.close.assert_called_once()

    def test_metadata_unknown_ids_and_room_creation_time_do_not_invent_session(self):
        self.session.get.return_value = response(data={"data": {"user": {"nickname": "主播"},
            "data": [{"status": 2, "create_time": 1800000000, "owner": {"create_time": 1800000000}}]}})
        result = engine.fetch_room_info("123", "synthetic=example")
        self.assertEqual("", result["anchor_id"])
        self.assertEqual("", result["room_id"])
        self.assertEqual(0, result["started_at"])
        self.assertNotIn("user_count", result)

    def test_metadata_current_viewers_never_fall_back_to_total_user(self):
        for room, expected in (({"status": 2, "user_count": "42", "total_user": 999}, 42),
                               ({"status": 2, "online_count": 0}, 0),
                               ({"status": 2, "user_count_str": "100"}, 100),
                               ({"status": 2, "user_count_str": "1.2万"}, None),
                               ({"status": 2, "total_user": 999, "user_count": "1.2万"}, None),
                               ({"status": 4, "user_count": 42}, None)):
            self.session.get.return_value = response(data={"data": {"data": [room]}})
            self.assertEqual(expected, engine.fetch_room_info("123", "synthetic=example")["viewer_count"])

    def test_metadata_owner_identity_and_integer_room_id(self):
        self.session.get.return_value = response(data={"data": {"data": [{"status": 4,
            "id": 7696455970398309120, "owner": {"id": 18446744073709551613, "nickname": "主播"}}]}})
        result = engine.fetch_room_info("123", "synthetic=example")
        self.assertEqual("18446744073709551613", result["anchor_id"])
        self.assertEqual("7696455970398309120", result["room_id"])
        self.assertFalse(result["is_live"])

    def test_identity_does_not_accept_float_or_nickname(self):
        self.assertEqual("", engine._numeric_id(float(18446744073709551613)))
        self.assertEqual("", engine._numeric_id("主播昵称"))
        self.assertEqual("", engine._numeric_id(0))
        self.assertEqual("18446744073709551613", engine._numeric_id(18446744073709551613))

    def test_explicit_offline_status(self):
        self.session.get.return_value = response(data={"data": {"data": [{"status": 4}]}})
        self.assertFalse(engine.fetch_room_info("123", "synthetic=example")["is_live"])

    def test_boolean_and_float_status_are_unknown_not_offline(self):
        for status in (True, False, 1.0, 2.0, 3.0, 4.0):
            with self.subTest(status=status):
                self.session.get.return_value = response(data={"data": {"data": [{"status": status}]}})
                with self.assertRaisesRegex(engine.LiveMonitorError, "未知直播状态"):
                    engine.fetch_room_info("123", "synthetic=example")
        self.assertEqual(6, self.session.close.call_count)

    def test_live_status_without_stream_url_remains_live(self):
        for status in (2, "2"):
            with self.subTest(status=status):
                self.session.get.return_value = response(data={"data": {"data": [{"status": status}]}})
                self.assertTrue(engine.fetch_room_info("123", "synthetic=example")["is_live"])

    def test_empty_unknown_or_invalid_response_is_not_offline(self):
        for data in ({}, {"data": {}}, {"data": {"data": []}},
                     {"data": {"data": [{"status": 999}]}}, {"status_code": 401}, []):
            with self.subTest(data=data):
                self.session.get.return_value = response(data=data)
                with self.assertRaises(engine.LiveMonitorError):
                    engine.fetch_room_info("123", "synthetic=example")
        self.assertEqual(6, self.session.close.call_count)

    def test_non_json_response_is_clear_error(self):
        self.session.get.return_value = response()
        self.session.get.return_value.json.side_effect = ValueError("not json")
        with self.assertRaisesRegex(engine.LiveMonitorError, "JSON"):
            engine.fetch_room_info("123", "synthetic=example")
        self.session.close.assert_called_once()

    def test_room_api_rejection_is_classified_without_claiming_offline(self):
        self.session.get.return_value = response(data={"status_code": 401,
            "status_msg": "synthetic-secret-must-not-be-echoed"})
        with self.assertRaises(engine.LiveRoomUnavailableError) as raised:
            engine.fetch_room_info("123", "synthetic=example")
        self.assertEqual("room_unavailable", raised.exception.reason)
        self.assertNotIn("synthetic-secret", str(raised.exception))
        self.assertIn("可能", str(raised.exception))

    def test_transport_and_schema_failures_are_not_room_unavailable(self):
        cases = [requests.exceptions.SSLError("synthetic-secret"),
                 requests.exceptions.ConnectTimeout("synthetic-secret")]
        for failure in cases:
            self.session.get.side_effect = failure
            with self.subTest(kind=type(failure).__name__), self.assertRaises(engine.LiveMonitorError) as raised:
                engine.fetch_room_info("123", "synthetic=example")
            self.assertNotIsInstance(raised.exception, engine.LiveRoomUnavailableError)
            self.assertNotIn("synthetic-secret", str(raised.exception))
        self.session.get.side_effect = None
        self.session.get.return_value = response(data=[])
        with self.assertRaises(engine.LiveMonitorError) as raised:
            engine.fetch_room_info("123", "synthetic=example")
        self.assertNotIsInstance(raised.exception, engine.LiveRoomUnavailableError)

    def test_invalid_id_does_not_issue_request(self):
        with self.assertRaises(engine.LiveMonitorError):
            engine.fetch_room_info("123&extra=1")
        self.session.get.assert_not_called()


class AuthenticatedProxyTests(unittest.TestCase):
    USER = "synthetic@operator"
    PASSWORD = "synthetic:/%?#password"

    def route(self, protocol):
        return f"{protocol}://{quote(self.USER, safe='')}:{quote(self.PASSWORD, safe='')}@8.8.8.8:8080"

    def test_http_and_socks_keep_encoded_credentials_in_session_and_decode_once_for_ws(self):
        for protocol in ("http", "socks5h"):
            with self.subTest(protocol=protocol):
                route = self.route(protocol)
                session = Mock()
                self.assertEqual(route, engine._configure_session(session, route))
                self.assertEqual({"http": route, "https": route}, session.proxies)
                self.assertIs(session.trust_env, False)
                self.assertIs(session.verify, True)
                options = engine._websocket_proxy_options(route)
                self.assertEqual(protocol, options["proxy_type"])
                self.assertEqual((self.USER, self.PASSWORD), options["http_proxy_auth"])
                self.assertEqual((self.USER, self.PASSWORD), requests.utils.get_auth_from_url(route))
                self.assertEqual("8.8.8.8", options["http_proxy_host"])
                self.assertEqual(["never-bypass.invalid"], options["http_no_proxy"])

    def test_socks_metadata_and_socket_share_route_without_proxy_auth_in_target_headers(self):
        route = self.route("socks5h")
        metadata_session, socket_session = Mock(), Mock()
        metadata_session.cookies = socket_session.cookies = {}
        metadata_session.get.return_value = response(data={"data": {"data": [{"status": 2}]}})
        socket_session.get.return_value = response()
        with patch.object(engine.requests, "Session", side_effect=[metadata_session, socket_session]):
            engine.fetch_room_info("123", cookie="synthetic_cookie=fixture", proxy=route)
            fetcher = engine.DouyinLiveWebFetcher("123", cookie="synthetic_cookie=fixture", proxy=route)
        socket_app = Mock()
        captured_headers = []
        def run(**kwargs):
            captured_headers.append(factory.call_args.kwargs["header"]())
            fetcher._wsOnOpen(socket_app)
            fetcher.stop()
            return False
        socket_app.run_forever.side_effect = run
        with patch.object(engine, "generateSignature", return_value="fixture"), \
                patch.object(engine.websocket, "WebSocketApp", return_value=socket_app) as factory:
            fetcher.start()
        for session in (metadata_session, socket_session):
            self.assertEqual(route, session.proxies["https"])
            for call in session.get.call_args_list:
                self.assertNotIn("Proxy-Authorization", call.kwargs["headers"])
                self.assertNotIn(self.PASSWORD, repr(call.kwargs["headers"]))
        options = socket_app.run_forever.call_args.kwargs
        self.assertEqual("socks5h", options["proxy_type"])
        self.assertEqual((self.USER, self.PASSWORD), options["http_proxy_auth"])
        self.assertEqual(engine.ssl.CERT_REQUIRED, options["sslopt"]["cert_reqs"])
        self.assertIs(options["sslopt"]["check_hostname"], True)
        self.assertNotIn("Proxy-Authorization", captured_headers[0])
        self.assertNotIn(self.PASSWORD, repr(captured_headers[0]))

    def test_websocket_library_uses_socks_remote_dns_with_credentials(self):
        import websocket._http as transport
        route = self.route("socks5h")
        options = transport.proxy_info(**engine._websocket_proxy_options(route), http_proxy_timeout=5)
        socket_options = Mock(sslopt={"cert_reqs": engine.ssl.CERT_REQUIRED, "check_hostname": True})
        proxy = Mock()
        with patch.object(transport.Proxy, "create", return_value=proxy) as create, \
                patch.object(transport, "_ssl_socket", return_value=Mock()) as tls:
            transport._start_proxied_socket("wss://webcast100-ws-web-lq.douyin.com/test", socket_options, options)
        self.assertIs(create.call_args.kwargs["rdns"], True)
        self.assertEqual(self.USER, create.call_args.kwargs["username"])
        self.assertEqual(self.PASSWORD, create.call_args.kwargs["password"])
        proxy.connect.assert_called_once_with("webcast100-ws-web-lq.douyin.com", 443, timeout=5)
        self.assertEqual(engine.ssl.CERT_REQUIRED, tls.call_args.args[1]["cert_reqs"])

    def test_invalid_routes_never_echo_userinfo(self):
        invalid = ["socks5://synthetic-user:synthetic-secret@8.8.8.8:80",
                   "http://synthetic-secret@8.8.8.8:80",
                   "http://synthetic-user:synthetic-secret@8.8.8.8:65536",
                   "http://synthetic-user:synthetic-secret%0a@8.8.8.8:80",
                   "http://synthetic-user:synthetic-secret%xx@8.8.8.8:80",
                   "http://synthetic-user:synthetic-secret@[broken:80",
                   "http://synthetic-user:synthetic-secret@8.8.8.8:80/path"]
        for route in invalid:
            with self.subTest(route_kind=invalid.index(route)), self.assertRaises(engine.LiveMonitorError) as raised:
                engine._proxy_url(route)
            self.assertNotIn("synthetic-user", str(raised.exception))
            self.assertNotIn("synthetic-secret", str(raised.exception))

    def test_errors_and_callback_logs_do_not_expose_proxy_credentials(self):
        session = Mock()
        session.get.side_effect = requests.exceptions.ProxyError(self.route("http"))
        with self.assertRaises(engine.LiveMonitorError) as raised:
            engine._http_get(session, "https://live.douyin.com/")
        self.assertNotIn("synthetic", str(raised.exception))
        with patch.object(engine.requests, "Session", return_value=session):
            fetcher = engine.DouyinLiveWebFetcher("123", proxy=self.route("http"),
                on_status=Mock(side_effect=ValueError(self.route("http"))))
        with self.assertLogs(engine.LOGGER, level="WARNING") as logs:
            fetcher._notify_status("error", "safe")
        self.assertNotIn("synthetic", "\n".join(logs.output))
        self.assertNotIn("Traceback", "\n".join(logs.output))
        self.assertNotIn("synthetic", str(fetcher._record_error(RuntimeError(self.route("http")))))


class SignatureTests(unittest.TestCase):
    def test_parallel_signatures_never_exceed_two_v8_contexts(self):
        lock = threading.Lock()
        entered = threading.Event()
        release = threading.Event()
        live, peak, errors = [0], [0], []
        class Context:
            def __init__(self):
                with lock:
                    live[0] += 1
                    peak[0] = max(peak[0], live[0])
                    if live[0] == 2:
                        entered.set()
            def eval(self, *args, **kwargs):
                release.wait(2)
            def call(self, *args, **kwargs):
                return "signature"
            def close(self):
                with lock:
                    live[0] -= 1
        def sign():
            try:
                engine.generateSignature("wss://example.test/?room_id=1")
            except Exception as error:
                errors.append(error)
        with patch.object(engine, "MiniRacer", Context):
            threads = [threading.Thread(target=sign) for _ in range(12)]
            for thread in threads:
                thread.start()
            self.assertTrue(entered.wait(1))
            self.assertEqual(2, peak[0])
            release.set()
            for thread in threads:
                thread.join(2)
                self.assertFalse(thread.is_alive())
        self.assertEqual([], errors)
        self.assertEqual(2, peak[0])
        self.assertEqual(0, live[0])

    def test_script_resolves_outside_working_directory_and_engine_closes(self):
        ctx = Mock()
        ctx.call.return_value = "signature"
        old_cwd = os.getcwd()
        with tempfile.TemporaryDirectory() as directory:
            try:
                os.chdir(directory)
                with patch.object(engine, "MiniRacer", return_value=ctx):
                    self.assertEqual("signature", engine.generateSignature("wss://example.test/?room_id=1"))
            finally:
                os.chdir(old_cwd)
        ctx.eval.assert_called_once()
        ctx.close.assert_called_once()


    def test_eval_and_signature_failures_release_engine(self):
        for operation in ("eval", "call"):
            with self.subTest(operation=operation):
                ctx = Mock()
                getattr(ctx, operation).side_effect = RuntimeError("synthetic failure")
                with patch.object(engine, "MiniRacer", return_value=ctx):
                    with self.assertRaises(engine.LiveMonitorError):
                        engine.generateSignature("wss://example.test/?room_id=1")
                ctx.close.assert_called_once()

    def test_empty_signature_is_failure(self):
        ctx = Mock()
        ctx.call.return_value = ""
        with patch.object(engine, "MiniRacer", return_value=ctx):
            with self.assertRaisesRegex(engine.LiveMonitorError, "签名为空"):
                engine.generateSignature("wss://example.test/?room_id=1")
        ctx.close.assert_called_once()


class SharedHeartbeatCapacityTests(unittest.TestCase):
    def test_three_hundred_rooms_do_not_spawn_three_hundred_heartbeat_threads(self):
        class Room:
            def __init__(self):
                self._stop_event = threading.Event()
                self._heartbeat_stop = threading.Event()
                self.sent = 0
            def _sendHeartbeatOnce(self):
                self.sent += 1
        rooms = [Room() for _ in range(300)]
        try:
            for room in rooms:
                engine._HEARTBEATS.add(room)
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline and not all(room.sent for room in rooms):
                time.sleep(0.01)
            self.assertTrue(all(room.sent for room in rooms))
            threads = [thread for thread in threading.enumerate() if thread.name.startswith("heartbeat-")]
            self.assertLessEqual(len(threads), 9)
            self.assertTrue(all(thread.daemon for thread in threads))
        finally:
            for room in rooms:
                engine._HEARTBEATS.remove(room)


if __name__ == "__main__":
    unittest.main()
