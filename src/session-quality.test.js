import test from "node:test";
import assert from "node:assert/strict";
import {createSessionQuality,qualityRate,qualityTime,onlineNumber,speakingOnlineRate} from "./session-quality.js";

const flush=()=>new Promise(resolve=>setImmediate(resolve));
function fixture(bridge,onSession=()=>{},onContext=()=>{}){
  const root={innerHTML:"",querySelector:()=>null};
  const modal={classList:{add(){}}};
  globalThis.document={querySelector:s=>s==="#session-quality"?root:modal,activeElement:null};
  const state={modal:{kind:"detail",rid:"111"},preview:true};
  const notices=[];
  const controller=createSessionQuality({state,bridge,invoke:()=>{},toast:(...a)=>notices.push(a),onSession,onContext});
  return {controller,state,root,notices};
}
const data=(name="2026-10-01")=>({total:1,next_offset:null,sessions:[{session_id:"1001",started_at:name,coverage:"partial",speakers:8,valid_speakers:4,analysis_state:"ready",valid_speaker_rate:.5,monitor_seconds:120}],rules:{window_seconds:60,self_count:3}});
const anchor={rid:"111",anchor_id:"11",name:"主播",session_id:"1001"};
const detail=(sessionId,content="")=>({overview:{session_id:sessionId,analysis_state:"ready"},reasons:{},messages:content?[{content,reason:"normal"}]:[],commands:[],total:content?1:0});

test("offline last-known sessions load immediately and are labelled recent rather than live",async()=>{
 const f=fixture(async(method,p)=>method==="quality_detail"?detail(p.session_id):data());
 f.controller.mount({...anchor,is_live:false,active:true,state:"waiting_live"});await flush();
 assert.match(f.root.innerHTML,/最近场 · 00:02:00/);assert.doesNotMatch(f.root.innerHTML,/当前场|开播中/);
 f.controller.tick({...anchor,is_live:true,active:true,state:"connected"});
 await f.controller.action({dataset:{action:"quality-refresh"}});
 assert.match(f.root.innerHTML,/当前场 · /);assert.match(f.root.innerHTML,/开播中/);
});

test("restoring an offline identity and session together loads history on that first tick",async()=>{
 const requests=[],selected=[];
 const f=fixture(async(method,p)=>{requests.push([method,p]);return method==="quality_detail"?detail(p.session_id,"恢复的聊天"):data();},id=>selected.push(id));
 f.controller.mount({...anchor,anchor_id:"",session_id:"",is_live:null});await flush();
 f.controller.tick({...anchor,is_live:false});await flush();
 assert.deepEqual(requests.filter(([m])=>m==="quality_sessions").map(([,p])=>[p.anchor_id,p.current_session_id]),[["11","1001"]]);
 assert.deepEqual(selected,["1001"]);assert.match(f.root.innerHTML,/恢复的聊天/);
});

for(const pendingStage of ["quality_sessions","quality_detail"]){
 test(`identity replacement rejects a late ${pendingStage} response and resets history selection`,async()=>{
  let finishOld,delayOld=false;const requests=[],selected=[];
  const f=fixture(async(method,p)=>{
   requests.push([method,p]);
   if(p.anchor_id==="11"&&delayOld&&method===pendingStage)return new Promise(resolve=>finishOld=resolve);
   if(method==="quality_detail")return detail(p.session_id,p.anchor_id==="22"?"新主播聊天":"原主播聊天");
   return {...data(p.anchor_id==="22"?"2026-10-22":"2026-10-01"),sessions:[{...data().sessions[0],session_id:p.anchor_id==="22"?"2001":"1001"},{...data().sessions[0],session_id:"older"}]};
  },id=>selected.push(id));
  f.controller.mount(anchor);await flush();delayOld=true;
  const pending=f.controller.action({dataset:{action:"quality-session",session:"older"}});await flush();
  f.controller.tick({...anchor,anchor_id:"22",session_id:"2001",is_live:false});await flush();
  assert.ok(requests.some(([method,p])=>method==="quality_sessions"&&p.anchor_id==="22"));
  finishOld(pendingStage==="quality_detail"?detail("older","迟到的原主播聊天"):data("1999-01-01"));await pending;await flush();
  assert.equal(selected.at(-1),"2001");assert.match(f.root.innerHTML,/新主播聊天/);assert.doesNotMatch(f.root.innerHTML,/迟到的原主播聊天|99\/01\/01/);
  assert.match(f.root.innerHTML,/id="quality-episode-22-2001" class="quality-episode selected"/);
 });
}

test("online metrics keep missing values unknown and ratios may exceed 100 percent",()=>{
 assert.equal(onlineNumber(null),"—");assert.equal(onlineNumber(0),"0");assert.equal(onlineNumber(123.45),"123.5");assert.equal(onlineNumber(.001),"<0.1");
 assert.equal(speakingOnlineRate({analysis_state:"ready",valid_speakers_to_online_rate:1.5}),"150.0%");
 assert.equal(speakingOnlineRate({analysis_state:"ready",speakers_to_online_rate:1.5,valid_speakers_to_online_rate:.2}),"20.0%");
 assert.equal(speakingOnlineRate({analysis_state:"analyzing",valid_speakers_to_online_rate:.5}),"—");
});

test("quality UI keeps incomplete rates unknown and durations above 24h",()=>{
 assert.equal(qualityRate({analysis_state:"analyzing",valid_speaker_rate:.5}),"—");
 assert.equal(qualityRate({analysis_state:"ready",valid_speaker_rate:.5}),"50.0%");
 assert.equal(qualityTime(90061),"25:01:01");assert.equal(qualityTime(null),"—");
});

test("speaker share and online ratio have distinct labels and values",async()=>{
 const result=data();result.sessions[0].valid_speakers_to_online_rate=.2;
 const f=fixture(async()=>result);f.controller.mount(anchor);await flush();
 assert.match(f.root.innerHTML,/有效发言占比/);assert.match(f.root.innerHTML,/有效发言人数 ÷ 总发言人数/);
 assert.match(f.root.innerHTML,/50\.0%/);assert.match(f.root.innerHTML,/20\.0%/);
});

test("speaker tooltip separates total and effective people and keeps missing counts unknown",async()=>{
 const result=data();
 result.total=3;
 result.sessions.push({...result.sessions[0],session_id:"missing",speakers:null,valid_speakers:undefined});
 result.sessions.push({...result.sessions[0],session_id:"zero",speakers:0,valid_speakers:0,analysis_state:"analyzing"});
 const f=fixture(async method=>method==="quality_detail"?detail("1001"):result);
 f.controller.mount(anchor);await flush();
 assert.match(f.root.innerHTML,/发言人数<b>8<\/b>/);
 assert.match(f.root.innerHTML,/有效发言人数<b>4<\/b>/);
 assert.match(f.root.innerHTML,/去除只有无效发言的账号；至少一条有效发言，按账号 ID 去重/);
 assert.match(f.root.innerHTML,/发言人数<b>—<\/b>/);
 assert.match(f.root.innerHTML,/有效发言人数<b>—<\/b>/);
 assert.match(f.root.innerHTML,/发言人数<b>0<\/b>/);
 assert.match(f.root.innerHTML,/有效发言人数<b>0<\/b>/);
 assert.match(f.root.innerHTML,/统计中/);
 assert.doesNotMatch(f.root.innerHTML,/有效 \/ 发言/);
});

test("context publishes the selected overview, clears it during switches and rejects stale history",async()=>{
 const current={...data().sessions[0],started_at:"2026-10-01T12:00:00+08:00"};
 const history={...current,session_id:"older",started_at:"2026-09-01T12:00:00+08:00",speakers:20,valid_speakers:3};
 const contexts=[];
 let finishHistory,delayHistory=true;
 const f=fixture(async(method,p)=>{
  if(method==="quality_sessions")return {...data(),total:2,sessions:[current,history]};
  if(p.session_id==="older"&&delayHistory)return new Promise(resolve=>finishHistory=resolve);
  return {...detail(p.session_id),overview:p.session_id==="older"?history:current};
 },()=>{},(...context)=>contexts.push(context));
 f.controller.mount(anchor);
 assert.deepEqual(contexts.at(-1),["",null]);
 await flush();
 assert.equal(contexts.at(-1)[1],current);
 assert.match(contexts.at(-1)[0],/26\/10\/01/);
 const pending=f.controller.action({dataset:{action:"quality-session",session:"older"}});
 assert.deepEqual(contexts.at(-1),["",null]);
 await flush();
 await f.controller.action({dataset:{action:"quality-session",session:"1001"}});
 assert.equal(contexts.at(-1)[1],current);
 finishHistory({...detail("older"),overview:history});await pending;
 assert.equal(contexts.at(-1)[1],current);
 assert.equal(contexts.some(([,overview])=>overview===history),false);
 delayHistory=false;
 await f.controller.action({dataset:{action:"quality-session",session:"older"}});
 assert.equal(contexts.at(-1)[1],history);
 assert.match(contexts.at(-1)[0],/26\/09\/01/);
});
test("sessions use episode cards with hover statistics and selection never expands a table",async()=>{
 const selected=[];const f=fixture(async()=>data());f.controller.mount(anchor);await flush();
 assert.match(f.root.innerHTML,/class="quality-episode selected"/);assert.match(f.root.innerHTML,/role="tooltip"/);assert.match(f.root.innerHTML,/aria-describedby="session-tip-0"/);assert.doesNotMatch(f.root.innerHTML,/<table/);
 await f.controller.action({dataset:{action:"quality-toggle-sidebar"}});
 assert.match(f.root.innerHTML,/aria-label="展开场次列表"/);assert.match(f.root.innerHTML,/id="quality-sidebar-content" hidden/);
 await f.controller.action({dataset:{action:"quality-session",session:"1001"}});assert.match(f.root.innerHTML,/aria-pressed="true"/);assert.doesNotMatch(f.root.innerHTML,/<table/);
});
test("new broadcasts move the current card and statistics together while history stays selected",async()=>{
 let latest="1001";const selected=[];
 const f=fixture(async method=>method==="quality_detail"?{overview:{session_id:latest,analysis_state:"ready"},reasons:{},messages:[],commands:[],total:0}:{...data(),sessions:[{...data().sessions[0],session_id:latest},{...data().sessions[0],session_id:"old"}]},id=>selected.push(id));
 f.controller.mount(anchor);await flush();latest="1002";f.controller.tick({...anchor,session_id:latest});await flush();
 assert.deepEqual(selected,["1001","1002"]);assert.match(f.root.innerHTML,/id="quality-episode-11-1002" class="quality-episode selected"/);
 await f.controller.action({dataset:{action:"quality-session",session:"old"}});latest="1003";f.controller.tick({...anchor,session_id:latest});await flush();assert.equal(selected.at(-1),"old");assert.match(f.root.innerHTML,/id="quality-episode-11-old" class="quality-episode selected"/);
});
test("episode focus and its accessible tooltip survive refresh",async()=>{
 const f=fixture(async()=>data());let restored=false;
 const focus={id:"quality-episode-11-1001",tagName:"BUTTON",focus(){restored=true;}};
 f.root.contains=el=>el===focus;f.root.querySelector=selector=>selector===`#${focus.id}`?focus:null;
 f.controller.mount(anchor);await flush();globalThis.document.activeElement=focus;
 await f.controller.action({dataset:{action:"quality-refresh"}});assert.equal(restored,true);assert.match(f.root.innerHTML,/aria-describedby="session-tip-0"/);
});
test("month switch ignores previous in-flight response and resets page",async()=>{
 let resolveOld;const requests=[];
 const f=fixture(async(_m,p)=>{requests.push(p);if(requests.length===1)return new Promise(r=>resolveOld=r);return data("2026-09-01");});
 f.controller.mount(anchor);
 await f.controller.change({id:"quality-month",value:"2026-09",dataset:{}});
 resolveOld(data("STALE"));await flush();
 assert.match(f.root.innerHTML,/26\/09\/01/);assert.doesNotMatch(f.root.innerHTML,/STALE/);
 assert.equal(requests[1].offset,0);assert.equal(requests[1].month,"2026-09");
});
test("closing modal prevents a late response from repainting it",async()=>{
 let finish;const f=fixture(()=>new Promise(r=>finish=r));f.controller.mount(anchor);
 f.state.modal=null;f.root.innerHTML="closed";finish(data());await flush();assert.equal(f.root.innerHTML,"closed");
});
test("samples escape content and failed command save retains the draft",async()=>{
 const f=fixture(async(method)=>{
  if(method==="quality_commands")throw Error("保存失败");
  if(method==="quality_detail")return {overview:{session_id:"1001",analysis_state:"ready"},reasons:{normal:1},messages:[{content:'<script>alert(1)</script>',sender_name:'<b>x</b>',reason:"normal"}],commands:[],total:1};
  return data();
 });
 f.controller.mount(anchor);await flush();
 await f.controller.action({dataset:{action:"quality-session",session:"1001"}});
 assert.match(f.root.innerHTML,/&lt;script&gt;/);assert.doesNotMatch(f.root.innerHTML,/<script>/);
 f.controller.input({dataset:{},id:"quality-commands",value:"确认的口令"});
 await f.controller.action({dataset:{action:"quality-save-commands"}});
 await f.controller.action({dataset:{action:"quality-refresh"}});
 assert.match(f.root.innerHTML,/确认的口令/);assert.equal(f.notices[0][0],"保存失败");
});
