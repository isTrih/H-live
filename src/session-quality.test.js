import test from "node:test";
import assert from "node:assert/strict";
import {createSessionQuality,qualityRate,qualityTime,onlineNumber,speakingOnlineRate} from "./session-quality.js";

const flush=()=>new Promise(resolve=>setImmediate(resolve));
function fixture(bridge){
  const root={innerHTML:"",querySelector:()=>null};
  const modal={classList:{add(){}}};
  globalThis.document={querySelector:s=>s==="#session-quality"?root:modal,activeElement:null};
  const state={modal:{kind:"detail",rid:"111"},preview:true};
  const notices=[];
  const controller=createSessionQuality({state,bridge,invoke:()=>{},toast:(...a)=>notices.push(a)});
  return {controller,state,root,notices};
}
const data=(name="2026-10-01")=>({total:1,next_offset:null,sessions:[{session_id:"1001",started_at:name,coverage:"partial",speakers:8,valid_speakers:4,analysis_state:"ready",valid_speaker_rate:.5,monitor_seconds:120}],rules:{window_seconds:60,self_count:3}});
const anchor={rid:"111",anchor_id:"11",name:"主播",session_id:"1001"};

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
test("month switch ignores previous in-flight response and resets page",async()=>{
 let resolveOld;const requests=[];
 const f=fixture(async(_m,p)=>{requests.push(p);if(requests.length===1)return new Promise(r=>resolveOld=r);return data("2026-09-01");});
 f.controller.mount(anchor);
 await f.controller.change({id:"quality-month",value:"2026-09",dataset:{}});
 resolveOld(data("STALE"));await flush();
 assert.match(f.root.innerHTML,/2026-09-01/);assert.doesNotMatch(f.root.innerHTML,/STALE/);
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
