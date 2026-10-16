"""Real archive + monitor workers, fake room HTTP and WSS (no network)."""
import sys,tempfile,threading,time,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'python/collector'))
import test_monitor as fixtures
from monitor import MonitorManager
from liveMan import LiveMonitorError

class ViewerRefreshTests(unittest.TestCase):
 def setUp(self):
  self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
  self.created={};self.phase='initial';self.lock=threading.Lock();self.inflight=0;self.maximum=0;self.calls=0
  self.block=threading.Event();self.entered=threading.Event();self.block.set()
  def factory(rid,**kwargs):
   f=fixtures.FakeFetcher(rid,**kwargs);self.created[rid]=f;return f
  def info(rid,cookie='',*,traffic_control=None,stop_event=None):
   with self.lock:
    phase=self.phase
    if phase!='initial':self.inflight+=1;self.maximum=max(self.maximum,self.inflight);self.calls+=1
   try:
    if phase!='initial':
     self.assertIs(traffic_control,self.manager._direct_control);self.entered.set();self.block.wait(3);stop_event.wait(.005)
    if phase=='error':raise LiveMonitorError('synthetic',kind='target_http',http_status=444)
    return {**fixtures.live_info(rid),'viewer_count':10 if phase=='initial' else 20,'room_id':'999' if phase=='wrong' else '200'+rid}
   finally:
    if phase!='initial':
     with self.lock:self.inflight-=1
  self.manager=MonitorManager(data_dir=self.temp.name,fetcher_factory=factory,room_info_fetcher=info,connect_interval_seconds=0)
  self.addCleanup(self.manager.shutdown,5)
  self.addCleanup(self.block.set)
 def start(self,count=1):
  for i in range(count):self.manager.start(str(111+i),'合成主播')
  fixtures.wait_until(lambda:len(self.created)==count and all(r['state']=='connected' for r in self.manager.snapshot().values()),timeout=10)
 def done(self):fixtures.wait_until(lambda:not self.manager.viewer_refresh_status()['active'],timeout=10)
 def test_fifty_rooms_use_four_refresh_threads_keep_existing_exit_and_archive_separate_stats(self):
  self.start(50);self.phase='refresh';result=self.manager.refresh_viewers();self.assertTrue(result['accepted'])
  self.assertFalse(self.manager.refresh_viewers()['accepted']);self.done()
  result=self.manager.viewer_refresh_status();self.assertEqual(50,result['updated']);self.assertEqual(50,self.calls);self.assertLessEqual(self.maximum,4)
  self.assertTrue(all(row['viewer_count']==20 for row in self.manager.snapshot().values()))
  detail=self.manager.archive.quality('detail',{'anchor_id':'100111','session_id':'200111'})['overview']
  self.assertEqual(20,detail['peak_viewers']);self.assertEqual(0,detail['chat_messages']);self.assertGreater(detail['minute_avg_viewers'],0)
 def test_http_refresh_never_overwrites_newer_wss_sample(self):
  self.start();self.phase='refresh';self.block.clear();self.manager.refresh_viewers();self.assertTrue(self.entered.wait(2))
  self.created['111'].on_message('stats',{'viewer_count':99,'room_id':'200111'});self.block.set();self.done()
  self.assertEqual(99,self.manager.snapshot()['111']['viewer_count']);self.assertEqual(1,self.manager.viewer_refresh_status()['skipped'])
 def test_changed_session_or_444_does_not_overwrite_or_stop_live_socket(self):
  self.start();self.phase='wrong';self.manager.refresh_viewers();self.done()
  row=self.manager.snapshot()['111'];self.assertEqual('200111',row['session_id']);self.assertEqual(10,row['viewer_count']);self.assertEqual('connected',row['state'])
  self.assertEqual(1,self.manager.viewer_refresh_status()['failed'])
  self.phase='error';self.manager._viewer_refresh_at=-float('inf');self.manager.refresh_viewers();self.done()
  self.assertGreater(self.manager._direct_control.snapshot()['cooldown_seconds'],0);self.assertEqual('connected',self.manager.snapshot()['111']['state'])
 def test_stopping_during_refresh_discards_late_response(self):
  self.start();self.phase='refresh';self.block.clear();self.manager.refresh_viewers();self.assertTrue(self.entered.wait(2))
  self.manager.stop('111');self.block.set();self.done();self.assertEqual(10,self.manager.snapshot()['111']['viewer_count'])
  self.assertEqual(1,self.manager.viewer_refresh_status()['skipped'])

if __name__=='__main__':unittest.main()
