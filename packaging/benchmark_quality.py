"""Offline saturation test: 300 producers, durable ingestion and concurrent analysis."""
from concurrent.futures import ThreadPoolExecutor
import json
from contextlib import closing
from pathlib import Path
import sqlite3
import sys
import tempfile
import time
import threading

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'python/collector'))
from archive_store import ArchiveStore
from quality_worker import QualityWorker
from backend_service import _rss_mb


def main():
    version=json.loads((ROOT/'package.json').read_text(encoding='utf-8'))['version']
    timings=[];query_times=[];lock=threading.Lock()
    def percentile(values,fraction):return round(sorted(values)[min(len(values)-1,int(len(values)*fraction))]*1000,2)
    start_rss=_rss_mb()
    with tempfile.TemporaryDirectory(prefix='quality-load-',dir=ROOT/'build') as directory:
        store=ArchiveStore(directory);worker=QualityWorker(store)
        def produce(anchor):
            local=[]
            a=str(1000+anchor);session=str(8000+anchor)
            store.register_session(a,session,f'负载主播{anchor}',a)
            for i in range(90 if anchor else 3090):
                stamp='2026-10-01T12:00:00+08:00'
                message={'anchor_id':a,'session_id':session,'anchor_name':f'负载主播{anchor}','web_rid':a,
                         'sender_id':str(i%30+1),'sender_name':'同名观众','content':'集中重复口令' if i%2 else f'这是第{i}个问题？',
                         'message_type':'chat','message_id':f'{anchor}-{i}','message_time':stamp,'received_at':stamp,'time_source':'server'}
                begin=time.perf_counter();assert store.record_message(message);local.append(time.perf_counter()-begin)
            with lock:timings.extend(local)
        begin=time.perf_counter()
        with ThreadPoolExecutor(max_workers=300) as executor:
            futures=[executor.submit(produce,i) for i in range(300)]
            for future in futures:future.result()
        ingested=time.perf_counter()-begin
        deadline=time.monotonic()+180
        while True:
            t=time.perf_counter();store.quality('sessions',{'anchor_id':'1000','limit':20});query_times.append(time.perf_counter()-t)
            with closing(sqlite3.connect(store.db_path)) as db:
                raw=db.execute('SELECT COUNT(*) FROM messages').fetchone()[0]
                analyzed=db.execute('SELECT COALESCE(SUM(analyzed),0) FROM quality_stats').fetchone()[0]
            if analyzed==raw:break
            if time.monotonic()>deadline:raise TimeoutError(f'analysis backlog {analyzed}/{raw}')
            time.sleep(.1)
        elapsed=time.perf_counter()-begin
        with closing(sqlite3.connect(store.db_path)) as db:
            sessions=db.execute('SELECT COUNT(*) FROM quality_registry').fetchone()[0]
            classified=db.execute('SELECT COUNT(*) FROM quality_results').fetchone()[0]
            invalid=db.execute('SELECT COUNT(*) FROM quality_results WHERE verdict=0').fetchone()[0]
        assert raw==300*90+3000==classified and sessions==300 and invalid>0
        peak_rss=_rss_mb();assert worker.close();assert store.close()
    report={'ok':True,'version':version,'scope':'Synthetic offline saturation; no Douyin requests, WSS sockets, proxy availability or 300-room network guarantee.',
            'producers':300,'messages':raw,'sessions':sessions,'invalid_messages':invalid,'ingest_seconds':round(ingested,3),
            'ingest_messages_per_second':round(raw/ingested,1),'all_analysis_seconds':round(elapsed,3),'end_to_end_messages_per_second':round(raw/elapsed,1),
            'record_call_ms':{'p50':percentile(timings,.5),'p95':percentile(timings,.95),'p99':percentile(timings,.99)},
            'session_query_ms':{'samples':len(query_times),'p95':percentile(query_times,.95)},'rss_start_mb':start_rss,'rss_end_mb':peak_rss,
            'lost_messages':0,'worker_drained':True}
    target=ROOT/f'build/quality-load-{version}.json';target.write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8');print(json.dumps(report,ensure_ascii=False))

if __name__=='__main__':main()
