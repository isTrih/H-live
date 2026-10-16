"""Filtered archive view through the production backend RPC boundary."""
import unittest
import test_quality_service as fixtures


class RecentServiceTests(fixtures.QualityServiceTests):
    # Reuse fixture setup only, avoid inheriting unrelated test cases.
    test_zero_sessions_real_worker_commands_rebuild_and_separate_export = None
    test_rpc_rejects_invalid_rules_and_hides_internal_step_register = None

    def test_recent_rpc_keeps_types_filters_pause_cursor_and_delete_consistent(self):
        archive=self.service.manager.archive
        for i,kind in enumerate(['gift','activity','emoji','fansclub','social']):
            archive.record_message({'anchor_id':'11','session_id':'101','anchor_name':'主播','web_rid':'111',
                'sender_id':'22','sender_name':'观众','message_type':kind,'content':kind,'message_id':str(i),
                'message_time':'2026-10-16T12:00:00+08:00','received_at':'2026-10-16T12:00:01+08:00',
                'time_source':'server','payload':{'activity_kind':'lottery'}})
        for kind in ['gift','activity','emoji','fansclub','social']:
            result=self.service.dispatch('recent_messages',{'type':kind,'web_rid':'111'})
            self.assertEqual([kind],[r['message_type'] for r in result['messages']])
            self.assertEqual('5',result['high_watermark'])
            self.assertEqual([],self.service.dispatch('recent_messages',{'type':kind,'web_rid':'222'})['messages'])
        self.assertEqual([],self.service.dispatch('recent_messages',{'before_id':'0'})['messages'])
        archive.record_message({'anchor_id':'11','session_id':'101','anchor_name':'主播','web_rid':'111',
            'sender_id':'22','sender_name':'观众','message_type':'member','content':'进入直播间','message_id':'entry',
            'received_at':'2026-10-16T12:01:01+08:00','time_source':'missing'})
        self.assertEqual(4,len(self.service.dispatch('recent_messages',{})['messages']))
        self.assertEqual('6',self.service.dispatch('recent_messages',{})['high_watermark'])
        self.assertEqual(1,len(archive.recent_messages({'type':'member'})['messages']))
        for params in [{'type':'stats'},{'web_rid':'bad'},{'limit':151},{'before_id':12}]:
            with self.assertRaises(ValueError):self.service.dispatch('recent_messages',params)
        archive.execute_delete({'anchor_ids':['11']})
        self.assertEqual([],self.service.dispatch('recent_messages',{'type':'gift'})['messages'])


if __name__=='__main__':unittest.main()
