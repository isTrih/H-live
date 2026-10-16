"""Offline synthetic protobuf -> production fetcher callback -> manager -> DB."""
import json
from pathlib import Path
import sys
import tempfile
import threading
import unittest
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'python' / 'collector'))
sys.path.insert(0, str(ROOT / 'native_tests'))
from activity_fixtures import lottery, frame, blob, number, text
from liveMan import DouyinLiveWebFetcher
from monitor import MonitorManager


class OfflineFetcher:
    def __init__(self, rid, on_message, on_status, **kwargs):
        self.on_message, self.on_status = on_message, on_status
        self.ready, self.stopped = threading.Event(), threading.Event()
    def start(self):
        self.on_status('connected', '')
        self.ready.set()
        self.stopped.wait(5)
    def stop(self):
        self.stopped.set()


class ActivityPipelineTests(unittest.TestCase):
    def setUp(self):
        self.guard = patch('requests.Session.request', side_effect=AssertionError('network forbidden'))
        self.guard.start()
        self.temp = tempfile.TemporaryDirectory()
        self.created, self.events = [], []
        self.made = threading.Event()
        def factory(*args, **kwargs):
            result = OfflineFetcher(*args, **kwargs)
            self.created.append(result); self.made.set()
            return result
        self.manager = MonitorManager(data_dir=self.temp.name, on_event=self.events.append,
            fetcher_factory=factory, room_info_fetcher=lambda *args, **kwargs: {
                'is_live': True, 'anchor_id': '100111', 'room_id': '200111', 'title': '合成', 'started_at': 0},
            connect_interval_seconds=0)
        self.manager.start('111', '合成主播')
        self.assertTrue(self.made.wait(2)); self.assertTrue(self.created[0].ready.wait(2))
        self.engine = DouyinLiveWebFetcher('111', cookie='', on_message=self.created[0].on_message)
        self.engine.ws = Mock()
        self.engine._DouyinLiveWebFetcher__room_id = '200111'

    def tearDown(self):
        self.engine.stop()
        self.manager.shutdown(2)
        self.temp.cleanup()
        self.guard.stop()

    def rows(self):
        with self.manager.archive.export_snapshot() as reader:
            return [dict(row) for row in reader.connection.execute('SELECT * FROM messages')]

    def emit(self, room=200111, message=18446744073709551615):
        self.engine._wsOnMessage(self.engine.ws, frame([('WebcastLotteryEventNewMessage', lottery(room, message))]))

    def test_activity_archive_ui_and_dedupe_keep_structured_details(self):
        self.emit(); self.emit()
        row, = self.rows()
        self.assertEqual(row['message_type'], 'activity')
        self.assertEqual(row['sender_id'], '')
        self.assertEqual(row['sender_name'], '')
        self.assertEqual(row['session_id'], '200111')
        self.assertEqual(row['message_id'], '18446744073709551615')
        payload = json.loads(row['payload_json'])
        self.assertEqual(payload['activity_details']['conditions'][0]['content'], '合成口令')
        messages = [event for event in self.events if event['kind'] == 'message']
        self.assertEqual(len(messages), 1)
        self.assertEqual(messages[0]['payload'], payload)
        self.assertEqual(self.manager.snapshot()['111']['message_count'], 1)
        self.assertEqual(self.engine.ws.send.call_count, 2)

    def test_activity_respects_real_session_guard_and_stopped_callback_barrier(self):
        self.emit(room=999)
        self.assertEqual(self.rows(), [])
        self.assertEqual(self.manager.snapshot()['111']['state'], 'error')
        self.emit(message=2)
        self.assertEqual(self.rows(), [])

    def test_distinct_notifications_for_same_draw_are_one_archive_and_ui_message(self):
        for message in (100, 101, 102):
            self.emit(message=message)
        row, = self.rows()
        self.assertEqual(row['message_id'], '100')
        self.assertEqual(self.manager.snapshot()['111']['message_count'], 1)
        self.assertEqual(len([e for e in self.events if e['kind'] == 'message']), 1)
        self.assertEqual(len(self.manager.archive.recent_messages({'type': 'activity'})['messages']), 1)
        self.assertEqual(self.engine.ws.send.call_count, 3)

    def test_retired_gifts_are_not_recorded_counted_or_shown_with_live_messages(self):
        # Even if a gift packet arrives, visitor monitoring no longer records it.
        for message, name in [(501, '小心心'), (502, '人气票')]:
            common = number(2, message) + number(3, 200111) + number(4, 1792022400)
            user = number(1, 12345) + text(3, '合成测试观众')
            gift = blob(1, common) + number(6, 1) + blob(7, user) + blob(15, text(16, name))
            self.engine._wsOnMessage(self.engine.ws, frame([('WebcastGiftMessage', gift)]))
        self.emit(message=601)
        self.emit(message=602)
        gifts = [row for row in self.rows() if row['message_type'] == 'gift']
        self.assertEqual(gifts, [])
        recent = self.manager.archive.recent_messages({'type': 'gift', 'web_rid': '111'})['messages']
        self.assertEqual(recent, [])
        self.assertEqual(len([e for e in self.events if e['kind'] == 'message' and e['message_type'] == 'gift']), 0)
        self.assertEqual(self.manager.snapshot()['111']['message_count'], 1)

    def test_synthetic_sync_and_preview_reach_archive_and_filtered_realtime_query(self):
        # Public fixtures are deterministic and generated without network data.
        room='100000012000'
        # A separate manager avoids relying on private task restart timing.
        self.manager.shutdown(2)
        self.made.clear()
        def factory(*args,**kwargs):
            result=OfflineFetcher(*args,**kwargs);self.created.append(result);self.made.set();return result
        self.manager=MonitorManager(data_dir=self.temp.name,on_event=self.events.append,fetcher_factory=factory,
            room_info_fetcher=lambda *args,**kwargs:{'is_live':True,'anchor_id':'100111','room_id':room,'title':'合成','started_at':0},connect_interval_seconds=0)
        self.manager.start('111','合成主播');self.assertTrue(self.made.wait(2));self.assertTrue(self.created[-1].ready.wait(2))
        self.engine.on_message=self.created[-1].on_message
        self.engine._DouyinLiveWebFetcher__room_id=room
        fixtures=ROOT/'crates/qingkan-core/tests/fixtures'
        for method,name in [('WebcastRoomDataSyncMessage','lottery_sync_synthetic.bin'),('WebcastPreviewCjRpMessage','red_packet_preview_synthetic.bin')]:
            self.engine._wsOnMessage(self.engine.ws,frame([(method,(fixtures/name).read_bytes())]))
        recent=self.manager.archive.recent_messages({'type':'activity','web_rid':'111'})
        self.assertEqual({'lottery','red_packet'},{r['payload']['activity_kind'] for r in recent['messages']})
        self.assertTrue(all(r['session_id']==room for r in recent['messages']))
        self.assertTrue(all(r['sender_id']=='' for r in recent['messages']))


if __name__ == '__main__':
    unittest.main()
