import test from 'node:test';
import assert from 'node:assert/strict';
import {bookmarkPosition,bookmarkClip,createSessionBookmarks} from './session-bookmarks.js';
import {replayTimeMap} from './replay-sync.js';
import {createSessionTimeline} from './session-timeline.js';

const flush=()=>new Promise(resolve=>setImmediate(resolve));
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};};
const context=()=>({anchorId:'11',sessionId:'21',live:true,startEpoch:1000,duration:300,mediaDuration:300,ready:true,toMedia:offset=>offset});
const mark=(id='a',epoch=1100,extra={})=>({id,anchor_id:'11',session_id:'21',marked_at_epoch:epoch,title:`精彩-${id}`,note:'',...extra});
const button=(action,id)=>({dataset:{action,...(id?{mark:id}:{})}});

function fixture({bridge,onCreateClip,initial=context()}={}){
  let current=initial,changes=0;
  const calls=[],notices=[],seek=[],clips=[];
  const controller=createSessionBookmarks({
    bridge:async(method,params)=>{calls.push([method,params]);return bridge?bridge(method,params):{marks:[],total:0};},
    getContext:()=>current,changed:()=>changes++,toast:(...args)=>notices.push(args),onSeek:epoch=>seek.push(epoch),
    onSelectClip:clip=>clips.push(clip),onCreateClip,
  });
  controller.select();
  return {controller,calls,notices,seek,clips,setContext:value=>{current=value;controller.select();},changes:()=>changes};
}

test('bookmarks map absolute live time through PDT spans and calibration without filling a recording gap',()=>{
  const replay={time_mapping:[{offset_seconds:0,duration:30,program_date_time:1020},{offset_seconds:30,duration:50,program_date_time:1100}]};
  const map=replayTimeMap(replay,1000);
  const ctx={...context(),mediaDuration:80,toMedia:offset=>map.toMedia(offset)};
  assert.equal(bookmarkPosition(mark('first',1040),ctx,true),20);
  assert.equal(bookmarkPosition(mark('later',1110),ctx,true),40);
  assert.equal(bookmarkPosition(mark('gap',1070),ctx,true),null);
  assert.equal(bookmarkPosition(mark('before',1019),ctx,true),null);
  assert.equal(bookmarkPosition(mark('end',1150),ctx,true),80);
  assert.equal(bookmarkPosition(mark('after',1150.1),ctx,true),null);
  assert.equal(bookmarkPosition(mark('gap',1070),ctx),70,'archive comments keep their own wall-clock position');
  const corrected=replayTimeMap(replay,1000,5);
  assert.equal(bookmarkPosition(mark('corrected',1040),{...ctx,toMedia:offset=>corrected.toMedia(offset)},true),15);
  assert.equal(bookmarkPosition(mark('unknown',NaN),ctx,true),null);
  assert.equal(bookmarkPosition(mark('unknown',1040),{...ctx,startEpoch:null},true),null);
});

test('selected clips retain boundary context, reject gaps, and stay within the downloader 15 minute limit',()=>{
  const ctx={...context(),mediaDuration:2000,duration:2000};
  assert.deepEqual(bookmarkClip(mark('start',1005),ctx),{start:0,end:35,mark:mark('start',1005)});
  assert.deepEqual(bookmarkClip(mark('end',2995),ctx),{start:1980,end:2000,mark:mark('end',2995)});
  assert.equal(bookmarkClip(mark(),{...ctx,ready:false}),null);
  assert.equal(bookmarkClip(mark(),{...ctx,toMedia:()=>null}),null);
  assert.equal(bookmarkClip(mark(),ctx,-1,30),null);
  assert.equal(bookmarkClip(mark(),ctx,0,0),null);
  assert.equal(bookmarkClip(mark('long',2000),ctx,600,600),null,'a valid center must not submit a 20 minute job that the native queue rejects');
});

test('late list responses cannot replace a newly selected session or leak another anchor',async()=>{
  const old=deferred();
  const f=fixture({bridge:(_method,params)=>params.session_id==='21'?old.promise:{marks:[mark('new',1200,{session_id:'22'}),mark('wrong-owner',1201,{anchor_id:'99',session_id:'22'})],total:1}});
  const loading=f.controller.load();
  f.setContext({...context(),sessionId:'22'});
  await f.controller.load();
  old.resolve({marks:[mark('old')],total:1});await loading;
  assert.match(f.controller.markup(),/精彩-new/);
  assert.doesNotMatch(f.controller.markup(),/精彩-old|wrong-owner/);
  await f.controller.action(button('timeline-bookmark','new'));
  assert.deepEqual(f.seek,[1200]);
});

test('live create suppresses double clicks and retries an unconfirmed result with the same request key',async()=>{
  const first=deferred();let attempts=0,records=[];
  const f=fixture({bridge:(method)=>{
    if(method==='mark_list')return {marks:records,total:records.length};
    if(method==='mark_create'){attempts++;if(attempts===1)return first.promise;records=[mark('saved')];return {mark:records[0],created:false};}
  }});
  await f.controller.load();f.controller.input({id:'bookmark-title',value:'精彩转折'});
  const creating=f.controller.action(button('timeline-mark-create'));
  await f.controller.action(button('timeline-mark-create'));
  assert.equal(f.calls.filter(([method])=>method==='mark_create').length,1);
  first.reject(Error('连接中断，提交结果未知'));await creating;
  assert.match(f.controller.markup(),/精彩转折/);
  await f.controller.action(button('timeline-mark-create'));
  const creates=f.calls.filter(([method])=>method==='mark_create').map(([,params])=>params);
  assert.equal(creates.length,2);assert.equal(creates[0].request_key,creates[1].request_key);
  assert.equal(creates[0].title,'精彩转折');assert.equal(creates[0].marked_at_epoch,undefined);
  assert.match(f.controller.markup(),/精彩-saved/);
});

test('creating while an earlier list is pending still displays the newly saved mark',async()=>{
  const first=deferred();let reads=0,records=[];
  const f=fixture({bridge:method=>{
    if(method==='mark_list')return ++reads===1?first.promise:{marks:records,total:records.length};
    if(method==='mark_create'){records=[mark('just-created')];return {mark:records[0],created:true};}
  }});
  const oldLoad=f.controller.load();
  const create=f.controller.action(button('timeline-mark-create'));
  await flush();first.resolve({marks:[],total:0});
  await Promise.all([oldLoad,create]);await flush();
  assert.match(f.controller.markup(),/精彩-just-created/,'an old list must not remove a mark that was saved after it started');
});

test('a completed create does not erase a new title typed while that request was pending',async()=>{
  const pending=deferred();
  const f=fixture({bridge:method=>method==='mark_create'?pending.promise:{marks:[],total:0}});
  f.controller.input({id:'bookmark-title',value:'正在保存的标题'});
  const creating=f.controller.action(button('timeline-mark-create'));
  f.controller.input({id:'bookmark-title',value:'下一条标题草稿'});
  pending.resolve({mark:mark(),created:true});await creating;
  assert.match(f.controller.markup(),/value="下一条标题草稿"/);
});

test('refresh preserves edited notes and failed saves keep the draft with escaped markup',async()=>{
  const f=fixture({bridge:method=>{
    if(method==='mark_update')throw Error('磁盘不可写');
    return {marks:[mark('a',1100,{note:'服务器旧备注'})],total:1};
  }});
  await f.controller.load();await f.controller.action(button('timeline-mark-edit','a'));
  f.controller.input({id:'bookmark-edit-title',value:'未提交标题'});
  f.controller.input({id:'bookmark-edit-note',value:'第一行\n<script>保留草稿</script>'});
  await f.controller.load();
  assert.match(f.controller.markup(),/value="未提交标题"/);
  assert.match(f.controller.markup(),/第一行\n&lt;script&gt;保留草稿&lt;\/script&gt;/);
  await f.controller.action(button('timeline-mark-save'));
  assert.match(f.controller.markup(),/磁盘不可写/);
  assert.match(f.controller.markup(),/第一行\n&lt;script&gt;保留草稿&lt;\/script&gt;/);
  assert.doesNotMatch(f.controller.markup(),/<script>/);
});

test('partial batch failure deselects confirmed jobs and reuses the unconfirmed job key on retry',async()=>{
  const requests=[];let failed=false;
  const f=fixture({bridge:()=>({marks:[mark('a',1100),mark('b',1200)],total:2}),onCreateClip:async params=>{
    requests.push({...params});
    if(params.bookmark_id==='b'&&!failed){failed=true;throw Error('第二条的提交结果未知');}
    return {id:`job-${params.bookmark_id}`};
  }});
  await f.controller.load();f.controller.change({id:'bookmark-select-all',checked:true});
  await f.controller.action(button('timeline-mark-download'));
  assert.match(f.controller.markup(),/第二条的提交结果未知/);
  assert.match(f.controller.markup(),/下载精选（1）/);
  await f.controller.action(button('timeline-mark-download'));
  assert.deepEqual(requests.map(request=>request.bookmark_id),['a','b','b']);
  assert.equal(requests[1].request_key,requests[2].request_key);
  assert.notEqual(requests[0].request_key,requests[1].request_key);
  assert.equal(requests[1].source,'bookmark');assert.equal(requests[1].session_id,'21');
  assert.doesNotMatch(f.controller.markup(),/下载精选（[12]）/);
});

test('switching sessions during a batch stops scheduling remaining old-session jobs',async()=>{
  const first=deferred(),requests=[];
  const f=fixture({bridge:()=>({marks:[mark('a'),mark('b')],total:2}),onCreateClip:params=>{requests.push({...params});return first.promise;}});
  await f.controller.load();f.controller.change({id:'bookmark-select-all',checked:true});
  const downloading=f.controller.action(button('timeline-mark-download'));
  f.setContext({...context(),sessionId:'22'});
  first.resolve({id:'job-a'});await downloading;
  assert.equal(requests.length,1);
  assert.equal(requests[0].session_id,'21');
  assert.doesNotMatch(f.controller.markup(),/精彩-a|精彩-b|保存中/);
});

test('timeline markers omit replay gaps and an unmounted controller ignores pending reads',async()=>{
  const pending=deferred();
  const f=fixture({bridge:()=>pending.promise,initial:{...context(),toMedia:offset=>offset===100?null:offset}});
  const reading=f.controller.load();f.controller.unmount();pending.resolve({marks:[mark('late')],total:1});await reading;
  assert.equal(f.controller.markup(),'');assert.equal(f.changes(),0);
  const mapped=fixture({bridge:()=>({marks:[mark('gap',1100),mark('good',1200)],total:2}),initial:{...context(),toMedia:offset=>offset===100?null:offset}});
  await mapped.controller.load();
  assert.doesNotMatch(mapped.controller.markers(true),/data-mark="gap"/);
  assert.match(mapped.controller.markers(true),/data-mark="good"/);
  assert.match(mapped.controller.markers(false),/data-mark="gap"/);
});

test('typing a newer note while save is pending retains that newer draft after the earlier save completes',async()=>{
  const pending=deferred();
  const f=fixture({bridge:method=>method==='mark_update'?pending.promise:{marks:[mark('a')],total:1}});
  await f.controller.load();await f.controller.action(button('timeline-mark-edit','a'));
  f.controller.input({id:'bookmark-edit-note',value:'已提交的第一版'});
  const saving=f.controller.action(button('timeline-mark-save'));
  f.controller.input({id:'bookmark-edit-note',value:'还没提交的第二版'});
  pending.resolve({mark:mark('a',1100,{note:'已提交的第一版'})});await saving;
  assert.match(f.controller.markup(),/还没提交的第二版/);
  assert.match(f.controller.markup(),/id="bookmark-edit-note"/);
});

test('empty clip duration inputs cannot silently become a zero second before range',async()=>{
  const requests=[];
  const f=fixture({bridge:()=>({marks:[mark('a')],total:1}),onCreateClip:async p=>{requests.push(p);return {id:'a'};}});
  await f.controller.load();f.controller.change({id:'bookmark-select-all',checked:true});
  f.controller.input({id:'bookmark-before',value:''});
  await f.controller.action(button('timeline-mark-download'));
  assert.equal(requests.length,0);
  assert.match(f.controller.markup(),/请确认精选打点/);
});

test('a pending create from a previous session cannot reset or repaint the new session draft',async()=>{
  const pending=deferred();
  const f=fixture({bridge:method=>method==='mark_create'?pending.promise:{marks:[],total:0}});
  f.controller.input({id:'bookmark-title',value:'旧场次标题'});
  const creating=f.controller.action(button('timeline-mark-create'));
  f.setContext({...context(),sessionId:'22'});
  f.controller.input({id:'bookmark-title',value:'新场次标题'});
  const before=f.changes();pending.resolve({mark:mark('old'),created:true});await creating;
  assert.match(f.controller.markup(),/value="新场次标题"/);
  assert.equal(f.changes(),before);
  assert.equal(f.notices.length,0);
});

function timelineFixture(t,resolveMarks){
  const previous=globalThis.document;
  const seek={value:null,setAttribute(){}},position={textContent:''};
  const root={innerHTML:'',contains:()=>false,querySelector:selector=>selector==='#timeline-seek'?seek:selector==='.timeline-position'?position:null};
  globalThis.document={querySelector:selector=>selector==='#session-timeline'?root:null,activeElement:null};
  const calls=[],state={modal:{kind:'detail',rid:'123'}};
  const timeline=createSessionTimeline({state,bridge:async(method,params)=>{
    calls.push([method,params]);
    if(method==='mark_list')return resolveMarks(params);
    if(method==='session_replay')return {enabled:false,available:false};
    if(method==='session_timeline')return {anchor_id:'11',session_id:params.session_id,start_epoch:1000,duration_seconds:300,messages:[],buckets:[],activities:[]};
    return {};
  }});
  t.after(()=>{timeline.unmount();globalThis.document=previous;});
  return {timeline,root,seek,position,calls};
}

test('timeline integration seeks an archive-only bookmark by absolute epoch and disables live creation on a history episode',async t=>{
  const f=timelineFixture(t,params=>({marks:[mark('integrated',1150,{session_id:params.session_id})],total:1}));
  f.timeline.mount({rid:'123',anchor_id:'11',session_id:'21',active:true,is_live:true,state:'connected'});await flush();
  assert.match(f.root.innerHTML,/data-action="timeline-mark-create"/);
  assert.match(f.root.innerHTML,/class="timeline-bookmark"[^>]*data-mark="integrated"/);
  await f.timeline.action(button('timeline-bookmark','integrated'));
  assert.equal(f.seek.value,150);
  f.timeline.selectSession('22');await flush();
  assert.doesNotMatch(f.root.innerHTML,/data-action="timeline-mark-create"/);
  await f.timeline.action(button('timeline-mark-create'));
  assert.equal(f.calls.filter(([method])=>method==='mark_create').length,0);
});

test('timeline integration rejects a late bookmark list after switching episodes',async t=>{
  const old=deferred();
  const f=timelineFixture(t,params=>params.session_id==='21'?old.promise:{marks:[mark('new-episode',1120,{session_id:'22'})],total:1});
  f.timeline.mount({rid:'123',anchor_id:'11',session_id:'21',active:true,is_live:true,state:'connected'});await flush();
  f.timeline.selectSession('22');await flush();
  old.resolve({marks:[mark('old-episode')],total:1});await flush();
  assert.match(f.root.innerHTML,/精彩-new-episode/);
  assert.doesNotMatch(f.root.innerHTML,/精彩-old-episode/);
  await f.timeline.action(button('timeline-bookmark','new-episode'));assert.equal(f.seek.value,120);
});
