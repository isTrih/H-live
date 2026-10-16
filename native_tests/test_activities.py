"""Native activity FFI tests against synthetic tagged protobuf fixtures only."""
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'python' / 'collector'))
sys.path.insert(0, str(Path(__file__).resolve().parent))
import qingkan_native
from activity_fixtures import lottery, frame, number, text, blob


class ActivityNativeTests(unittest.TestCase):
    def test_compressed_activity_frame_preserves_ack_and_details(self):
        ack, events = qingkan_native.decode_frame(frame([('WebcastLotteryEventNewMessage', lottery())]), '1')
        self.assertIn(b'synthetic-ack', ack)
        event, = json.loads(events)
        self.assertEqual(event['type'], 'activity')
        p = event['payload']
        self.assertEqual(p['activity_id'], '18446744073709551614')
        self.assertEqual(p['message_id'], '18446744073709551615')
        self.assertEqual(p['room_id'], '200111')
        self.assertEqual(p['sender_id'], '')
        self.assertEqual(p['sender_name'], '')
        self.assertEqual(p['event_time'], 1792022400)
        self.assertEqual(p['activity_details']['conditions'][0]['content'], '合成口令')
        self.assertIn('合成奖品', p['content'])

    def test_unknown_methods_stay_ignored_and_bad_activity_does_not_drop_chat(self):
        ack, output = qingkan_native.decode_frame(frame([
            ('WebcastLotteryEventNewMessage', b'\xff'),
            ('WebcastUnverifiedRedPacketMessage', b''),
            ('WebcastChatMessage', text(3, '合成普通弹幕'))]), '123')
        self.assertIsNotNone(ack)
        self.assertEqual([event['type'] for event in json.loads(output)], ['chat'])

    def test_reward_recipient_details_are_not_collected(self):
        raw = number(2, 1) + text(4, 'private-winner') * 257 + text(5, 'private-result') + text(5000, 'private-openid')
        p = json.loads(qingkan_native.decode_message('WebcastLuckyBoxRewardMessage', raw, '123'))['payload']
        self.assertEqual(p['activity_stage'], 'reward')
        self.assertNotIn('private-', json.dumps(p))
        self.assertEqual(p['event_time'], 0)


if __name__ == '__main__':
    unittest.main()
