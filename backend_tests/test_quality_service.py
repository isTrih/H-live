"""Real Rust archive and asynchronous analysis, fake network manager."""
import csv
from pathlib import Path
import tempfile
import unittest

from test_backend import FakeManager, wait_for
from backend_service import BackendService, ServiceError
from archive_store import ArchiveStore
from app_storage import save_config


class QualityManager(FakeManager):
    def __init__(self,**kwargs):
        super().__init__(**kwargs)
        self.archive=ArchiveStore(kwargs['data_dir'])
    def shutdown(self,timeout=15):
        return super().shutdown(timeout) and self.archive.close()


class QualityServiceTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
        self.path=Path(self.tmp.name)
        save_config({'anchors':{},'proxy_pool':{'enabled':False},'first_run_complete':True},self.path)
        self.service=BackendService(self.path,manager_factory=QualityManager)
        self.addCleanup(self.service.shutdown)
    def test_zero_sessions_real_worker_commands_rebuild_and_separate_export(self):
        service=self.service;archive=service.manager.archive
        archive.register_session('11','101','测试主播','111',started_at='2026-10-01T12:00:00+08:00')
        archive.register_session('11','102','测试主播','111')
        for i in range(4):
            archive.record_message({'anchor_id':'11','session_id':'101','anchor_name':'测试主播','web_rid':'111',
                'sender_id':str(i),'sender_name':'同名','message_type':'chat','content':'谢谢' if i<3 else '测试口令',
                'message_id':str(i),'message_time':'2026-10-01T12:00:00+08:00','received_at':'2026-10-01T12:00:01+08:00','time_source':'server'})
        def detail():return service.dispatch('quality_detail',{'anchor_id':'11','session_id':'101'})
        wait_for(lambda:detail()['overview']['analyzed_chat']==4,timeout=5)
        self.assertEqual(4,detail()['overview']['valid_speakers'])
        service.dispatch('quality_commands',{'anchor_id':'11','session_id':'101','commands':['测试口令']})
        wait_for(lambda:detail()['overview']['analysis_state']=='ready',timeout=5)
        self.assertEqual(3,detail()['overview']['valid_speakers'])
        listed=service.dispatch('quality_sessions',{'anchor_id':'11','current_session_id':'101'})
        self.assertEqual(2,listed['total']);self.assertEqual('101',listed['sessions'][0]['session_id'])
        self.assertTrue(listed['worker']['available'])
        target=self.path/'analysis.csv'
        job=service.dispatch('quality_export',{'anchor_id':'11','format':'csv','output_path':str(target)})['job_id']
        wait_for(lambda:service._jobs[job]['status']!='running',timeout=5)
        self.assertEqual('completed',service._jobs[job]['status'])
        with target.open(encoding='utf-8-sig',newline='') as f:rows=list(csv.reader(f))
        self.assertEqual(3,len(rows));self.assertEqual(18,len(rows[0]))
        self.assertEqual('最高在线人数',rows[0][14])
        self.assertEqual('有效发言占比',rows[0][8])
        self.assertEqual('75.00%',next(row for row in rows[1:] if row[1]=='101')[8])
        self.assertEqual('有效发言/平均在线',rows[0][16])
        self.assertEqual(4,archive.quality('detail',{'anchor_id':'11','session_id':'101'})['overview']['chat_messages'])
    def test_rpc_rejects_invalid_rules_and_hides_internal_step_register(self):
        for method,params in [('quality_configure',{'anchor_id':'11','rules':{'window_seconds':1}}),('quality_step',{}),('quality_register',{}),('quality_sessions',{'anchor_id':'bad'}),('quality_sessions',{'anchor_id':'11','limit':1000})]:
            with self.assertRaises((ValueError,ServiceError)):self.service.dispatch(method,params)
        self.service._archive_delete_in_progress=True
        with self.assertRaises(ServiceError):self.service.dispatch('quality_rebuild',{'anchor_id':'11'})
        self.service._archive_delete_in_progress=False
