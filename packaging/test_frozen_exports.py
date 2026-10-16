"""Exercise both export formats through the frozen JSON-RPC sidecar, offline."""
from pathlib import Path
import csv
import hashlib
import json
import os
import queue
import sqlite3
import subprocess
import sys
import threading
import tempfile
import time
import xml.etree.ElementTree as ET
import zipfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python" / "collector"))
from archive_store import ArchiveStore


def main():
    executable = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / 'dist/DouyinLiveMonitor/engine/DouyinMonitorEngine.exe'
    expected_version = json.loads((ROOT / 'package.json').read_text(encoding='utf-8'))['version']
    directory = ROOT / f'build/frozen-export-check-{expected_version}'
    directory.mkdir(parents=True, exist_ok=True)
    data = Path(tempfile.mkdtemp(prefix='data-',dir=directory))
    store = ArchiveStore(data)
    content = '=SUM(1,2) 中文😀'
    activity_content = '福袋：活动 001234567890123456789；奖品 3 份；条件：关注主播；开奖时间未知'
    long_id = '98765432109876543210987654321'
    base = dict(anchor_id='107', anchor_name='离线导出测试', web_rid='107', session_id='7600000000000000001',
                sender_id=long_id, sender_name='测试用户', message_type='chat', content=content,
                message_time='2026-01-01T00:00:00+08:00', received_at='2026-01-01T00:00:02+08:00',
                time_source='server', message_id='frozen-export-1')
    store.record_message(base)
    store.record_message({**base, 'sender_id': '', 'sender_name': '', 'message_time': '', 'time_source': 'missing', 'message_id': 'frozen-export-2'})
    store.record_message({**base, 'anchor_id': '108', 'anchor_name': '另一位主播', 'web_rid': '108',
                          'sender_name': '另一观众', 'content': '第二位消息', 'message_id': 'frozen-export-3'})
    store.record_message({**base, 'anchor_id': '109', 'anchor_name': '当月无消息', 'web_rid': '109',
                          'message_time': '2025-12-01T00:00:00+08:00', 'message_id': 'frozen-export-4'})
    store.record_message({**base, 'anchor_id': '108', 'anchor_name': '另一位主播', 'web_rid': '108',
                          'sender_id': '', 'sender_name': '', 'message_type': 'activity',
                          'content': activity_content, 'message_time': '2026-01-01T00:00:01+08:00',
                          'message_id': 'frozen-export-activity-1',
                          'payload': {'activity_kind': 'lottery', 'activity_id': '001234567890123456789',
                                      'prize_count': 3, 'condition': '关注主播'}})
    store.register_session("107","7600000000000000002","离线导出测试","107",started_at="2026-01-02T00:00:00+08:00")
    store.register_session("107","7600000000000000001","离线导出测试","107",started_at="2026-01-01T00:00:00+08:00")
    for at,count in [('2026-01-01T12:00:00+08:00',10),('2026-01-01T12:00:20+08:00',30),('2026-01-01T12:01:00+08:00',40)]:
        store.quality('audience_record',{'anchor_id':'107','session_id':'7600000000000000001','viewer_count':count,'observed_at':at})
    known_lottery={**base,'anchor_id':'110','anchor_name':'合并福袋测试','web_rid':'110',
                   'message_type':'activity','message_id':'lottery-first','sender_id':'','sender_name':'',
                   'payload':{'activity_kind':'lottery','activity_details':{'draw_at':1767197100,'winner_count':2}}}
    assert store.record_message(known_lottery)
    assert not store.record_message({**known_lottery,'message_id':'lottery-repeat'})
    # Represents a pre-retirement archive, not a new collector event.
    store.record_message({**base,'anchor_id':'111','anchor_name':'旧礼物归档','web_rid':'111',
                          'message_type':'gift','message_id':'legacy-gift','content':'历史礼物：小心心 x1'})
    store.close()
    # An older archive has distinct server notifications for the same draw.
    # Let the actual frozen executable perform first-open consolidation.
    with sqlite3.connect(data/'messages.sqlite3') as db:
        db.execute('DROP INDEX messages_lottery_draw')
        db.execute("""INSERT INTO messages(anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,content,message_time,received_at,time_source,message_id,message_time_epoch,received_at_epoch,payload_json)
          SELECT anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,content,message_time,received_at,time_source,'lottery-old-duplicate',message_time_epoch,received_at_epoch,json_set(payload_json,'$.activity_details.prize_name','离线合并奖品') FROM messages WHERE anchor_id='110'""")
    responses = queue.Queue()
    environment = {**os.environ, 'DOUYIN_MONITOR_HOME': str(data), 'PYTHONIOENCODING': 'utf-8'}
    report = {'ok': False, 'data_directory':str(data), 'artifacts': [], 'source': 'Synthetic offline fixture; no live traffic'}
    bundled_native = executable.parent / '_internal/qingkan_native.pyd'
    native_digest = hashlib.sha256(bundled_native.read_bytes()).hexdigest()
    assert native_digest == hashlib.sha256((ROOT / 'python/collector/qingkan_native.pyd').read_bytes()).hexdigest()
    report['native_extension_sha256'] = native_digest
    with (directory / 'stderr.log').open('w', encoding='utf-8') as errors:
        child = subprocess.Popen([str(executable)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors,
                                 text=True, encoding='utf-8', env=environment, creationflags=subprocess.CREATE_NO_WINDOW)
        def receive():
            for line in child.stdout:
                responses.put(line)
        threading.Thread(target=receive, daemon=True).start()
        serial = 0
        def rpc(method, params=None):
            nonlocal serial
            serial += 1
            child.stdin.write(json.dumps({'id': serial, 'method': method, 'params': params or {}}, ensure_ascii=False) + '\n')
            child.stdin.flush()
            result = json.loads(responses.get(timeout=40))
            assert result['id'] == serial and result['ok'], result
            return result['result']
        def exported(parameters,method="export"):
            job = rpc(method, parameters)['job_id']
            deadline = time.monotonic() + 20
            while True:
                state = next(item for item in rpc('poll', {'after': 0})['export_jobs'] if item['id'] == job)
                if state['status'] != 'running':
                    break
                if time.monotonic() > deadline:
                    raise TimeoutError('Frozen export did not complete')
                time.sleep(0.05)
            assert state['status'] == 'completed', state
            return state['report']
        try:
            initialized = rpc('init')
            assert initialized['version'] == expected_version
            assert initialized['native_version'] == expected_version
            assert initialized['processing'] == 'rust'
            report['native_version'] = initialized['native_version']
            report['processing'] = initialized['processing']
            refreshed=rpc('refresh_viewers')
            assert refreshed['accepted'] and refreshed['total']==0 and not refreshed['active']
            assert rpc('viewer_refresh_status')['id']==refreshed['id']
            report['viewer_refresh_empty_batch_rpc']=True
            recent=rpc('recent_messages',{'type':'activity','web_rid':'108'})
            assert len(recent['messages'])==1 and recent['messages'][0]['payload']['activity_kind']=='lottery'
            assert recent['messages'][0]['content']==activity_content
            assert rpc('recent_messages',{'type':'activity','before_id':'0'})['messages']==[]
            assert isinstance(recent['high_watermark'],str)
            report['filtered_recent_archive_and_pause_cursor']=True
            lottery_recent=rpc('recent_messages',{'type':'activity','web_rid':'110'})['messages']
            assert len(lottery_recent)==1
            assert '离线合并奖品' in lottery_recent[0]['content']
            with sqlite3.connect(data/'messages.sqlite3') as db:
                first_id,merged_payload=db.execute("SELECT message_id,payload_json FROM messages WHERE anchor_id='110'").fetchone()
            assert first_id=='lottery-first'
            assert json.loads(merged_payload)['activity_details']['prize_name']=='离线合并奖品'
            assert json.loads(merged_payload)['activity_details']['winner_count']==2
            for extension in ('csv','xlsx'):
                lottery_export=exported({'anchor_ids':['110'],'month':'2026-01','format':extension,'output_path':str(directory/f'lottery-merged.{extension}')})
                assert lottery_export['row_count']==1
            report['lottery_same_draw_dedupe_historical_merge_and_single_export']=True
            assert rpc('recent_messages',{'web_rid':'111'})['messages']==[]
            legacy_gifts=rpc('recent_messages',{'type':'gift','web_rid':'111'})['messages']
            assert len(legacy_gifts)==1 and legacy_gifts[0]['message_type']=='gift'
            for extension in ('csv','xlsx'):
                legacy_export=exported({'anchor_ids':['111'],'month':'2026-01','format':extension,'output_path':str(directory/f'legacy-gifts.{extension}')})
                assert legacy_export['row_count']==1
            report['retired_gifts_hidden_from_live_but_history_exportable']=True
            rpc('add_anchors', {'values': '107'})
            anchor = next(row for row in rpc('init')['anchors'] if row['rid'] == '107')
            assert 'broadcast_started_at' in anchor and anchor['broadcast_started_at'] is None
            assert 'viewer_count' in anchor and anchor['viewer_count'] is None
            assert 'duration_started_at' in anchor and anchor['duration_started_at'] is None
            assert anchor['duration_source'] == ''
            assert 'duration_seconds' in anchor and anchor['duration_seconds'] in (None, 0)
            assert anchor['duration_running'] is False and 'duration_updated_at' in anchor
            report['current_live_telemetry_fields'] = True
            report['live_telemetry_schema'] = {key: anchor[key] for key in ('broadcast_started_at', 'viewer_count', 'duration_started_at', 'duration_source', 'duration_seconds', 'duration_running', 'duration_updated_at')}
            proxy_state = rpc('proxy_status')
            assert isinstance(proxy_state['enabled'], bool)
            assert proxy_state['refresh']['target'] == 50
            report['proxy_refresh_target'] = proxy_state['refresh']['target']
            assert rpc('configure_proxy', {'enabled': False})['enabled'] is False
            assert rpc('proxy_status')['enabled'] is False
            assert rpc('configure_proxy', {'enabled': True})['enabled'] is True
            assert rpc('proxy_status')['enabled'] is True
            report['proxy_module_and_mode_switch'] = True
            assert any(anchor['anchor_id'] == '107' for anchor in rpc('archive_anchors'))
            for extension in ('xlsx', 'csv'):
                target = directory / ('messages.' + extension)
                result = exported({'anchor_ids': ['107', '108', '109'], 'month': '2026-01', 'format': extension, 'output_path': str(target)})
                assert result['row_count'] == 4 and result['missing_sender_count'] == 2 and result['missing_time_count'] == 1, result
                if extension == 'csv':
                    with target.open(encoding='utf-8-sig', newline='') as stream:
                        rows = list(csv.reader(stream))
                    assert rows[0] == ['主播', '直播间场次', '发消息人ID', '发消息人名字', '消息类型', '消息内容', '发消息时间']
                    assert rows[1][2] == long_id and rows[1][5] == "'" + content, rows
                    assert rows[1][3] == '测试用户'
                    missing = next(row for row in rows[1:] if row[2] == '' and row[6] == '')
                    assert missing[3] == '' and missing[6] == ''
                    assert any(row[0] == '另一位主播' and row[3] == '另一观众' for row in rows[1:])
                    activity_row = next(row for row in rows[1:] if row[4] == '红包/福袋')
                    assert activity_row[2:4] == ['', ''] and activity_row[5] == activity_content
                else:
                    with zipfile.ZipFile(target) as archive:
                        xml = ET.fromstring(archive.read('xl/worksheets/sheet1.xml'))
                        workbook = ET.fromstring(archive.read('xl/workbook.xml'))
                        second = ET.fromstring(archive.read('xl/worksheets/sheet2.xml'))
                        empty = ET.fromstring(archive.read('xl/worksheets/sheet3.xml'))
                    ns = {'m': 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'}
                    assert [sheet.attrib['name'] for sheet in workbook.find('m:sheets', ns)] == ['离线导出测试', '另一位主播', '当月无消息']
                    assert result['sheet_count'] == 3
                    cells = {cell.attrib['r']: cell for cell in xml.findall('.//m:c', ns)}
                    assert cells['C2'].attrib['t'] == 'inlineStr'
                    assert cells['C2'].find('m:is/m:t', ns).text == long_id
                    assert cells['D2'].find('m:is/m:t', ns).text == '测试用户'
                    assert cells['F2'].find('m:is/m:t', ns).text == content
                    assert cells['G3'].find('m:is/m:t', ns).text is None
                    assert xml.find('m:autoFilter', ns).attrib['ref'] == 'A1:G3'
                    assert second.find('.//m:c[@r="A2"]/m:is/m:t', ns).text == '另一位主播'
                    assert second.find('.//m:c[@r="E3"]/m:is/m:t', ns).text == '红包/福袋'
                    assert second.find('.//m:c[@r="F3"]/m:is/m:t', ns).text == activity_content
                    assert second.find('.//m:c[@r="C3"]/m:is/m:t', ns).text is None
                    assert second.find('.//m:c[@r="D3"]/m:is/m:t', ns).text is None
                    assert len(empty.findall('m:sheetData/m:row', ns)) == 1
                    assert not xml.findall('.//m:f', ns)
                    report['per_anchor_worksheets_and_sender_name'] = True
                report['artifacts'].append(result)
            for extension in ('csv', 'xlsx'):
                target = directory / ('activities.' + extension)
                result = exported({'anchor_ids': ['108'], 'month': '2026-01', 'types': ['activity'],
                                   'format': extension, 'output_path': str(target)})
                assert result['row_count'] == 1 and result['missing_sender_count'] == 1
                if extension == 'csv':
                    with target.open(encoding='utf-8-sig', newline='') as stream:
                        rows = list(csv.reader(stream))
                    assert len(rows) == 2 and rows[1][2:6] == ['', '', '红包/福袋', activity_content]
                else:
                    with zipfile.ZipFile(target) as archive:
                        workbook = ET.fromstring(archive.read('xl/workbook.xml'))
                        xml = ET.fromstring(archive.read('xl/worksheets/sheet1.xml'))
                    assert [sheet.attrib['name'] for sheet in workbook.find('m:sheets', ns)] == ['另一位主播']
                    assert xml.find('.//m:c[@r="E2"]/m:is/m:t', ns).text == '红包/福袋'
                    assert xml.find('.//m:c[@r="F2"]/m:is/m:t', ns).text == activity_content
                    assert len(xml.findall('m:sheetData/m:row', ns)) == 2
                report['artifacts'].append(result)
            report['activity_default_and_filtered_exports'] = True
            deadline=time.monotonic()+10
            while True:
                overview=rpc('quality_detail',{'anchor_id':'107','session_id':'7600000000000000001'})['overview']
                if overview['analysis_state']=='ready':break
                assert time.monotonic()<deadline
                time.sleep(.05)
            assert overview['speakers']==1 and overview['valid_speakers']==1 and overview['unknown_sender_messages']==1
            assert overview['peak_viewers']==40 and overview['minute_avg_viewers']==30 and overview['audience_sample_minutes']==2
            assert abs(overview['speakers_to_online_rate']-1/30)<1e-9
            assert abs(overview['valid_speakers_to_online_rate']-1/30)<1e-9
            report['online_session_peak_minute_mean_and_ratio']=True
            listed=rpc('quality_sessions',{'anchor_id':'107','month':'2026-01','limit':20})
            assert listed['total']==2 and listed['worker']['available']
            rpc('quality_commands',{'anchor_id':'107','session_id':'7600000000000000001','commands':[content]})
            while True:
                overview=rpc('quality_detail',{'anchor_id':'107','session_id':'7600000000000000001'})['overview']
                if overview['analysis_state']=='ready':break
                assert time.monotonic()<deadline
                time.sleep(.05)
            assert overview['valid_speakers']==0 and overview['chat_messages']==2
            for extension in ('csv','xlsx'):
                target=directory/('analysis.'+extension)
                result=exported({'anchor_id':'107','month':'2026-01','format':extension,'output_path':str(target)},'quality_export')
                assert result['row_count']==2 and result['kind']=='session_analysis'
                if extension=='csv':
                    with target.open(encoding='utf-8-sig',newline='') as stream:rows=list(csv.reader(stream))
                    assert len(rows)==3 and len(rows[0])==18 and rows[0][7]=='有效发言人数'
                    assert rows[0][14]=='最高在线人数'
                    assert rows[0][8]=='有效发言占比'
                    assert rows[0][16]=='有效发言/平均在线'
                    captured=next(row for row in rows[1:] if row[1]=='7600000000000000001')
                    assert captured[14:18]==['40','30.00','0.00%','2']  # confirmed command excludes this account's only text
                else:
                    with zipfile.ZipFile(target) as archive:
                        sheet=ET.fromstring(archive.read('xl/worksheets/sheet1.xml'))
                    assert len(sheet.findall('m:sheetData/m:row',ns))==3
                    assert sheet.find('.//m:c[@r="H1"]/m:is/m:t',ns).text=='有效发言人数'
                    assert sheet.find('.//m:c[@r="I1"]/m:is/m:t',ns).text=='有效发言占比'
                report['artifacts'].append(result)
            report['quality_zero_session_rebuild_and_exports']=True
            parts=list(map(int,expected_version.split('.')));parts[2]+=1
            ready=rpc('prepare_update',{'target_version':'.'.join(map(str,parts))})
            assert ready['ready'] and ready['paused_tasks']==0
            intent=json.loads((data/'update-resume.json').read_text(encoding='utf-8'))
            assert intent['rids']==[] and intent['from_version']==expected_version
            assert set(intent)=={'format','from_version','target_version','rids','nonce'}
            report['frozen_update_prepare_intent']=True

            assert rpc('shutdown')['all_workers_stopped'] is True
            assert child.wait(timeout=10) == 0
            report.update(ok=True, child_exited=True)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
            (directory / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
