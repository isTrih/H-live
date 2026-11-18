import test from "node:test";
import assert from "node:assert/strict";
import {normalizeClipRange,clipAroundTime,parseClipTime} from "./clip-selection.js";
import {createSessionTimeline} from "./session-timeline.js";

test("dragged ranges clamp to media duration, support reverse drags and reject empty selections",()=>{
  assert.deepEqual(normalizeClipRange(80,20,100),{start:20,end:80});
  assert.deepEqual(normalizeClipRange(-10,150,100),{start:0,end:100});
  assert.equal(normalizeClipRange(10,10,100),null);assert.equal(normalizeClipRange(null,10,100),null);
  assert.deepEqual(clipAroundTime(5,100),{start:0,end:35});
  assert.deepEqual(clipAroundTime(95,100),{start:80,end:100});
  assert.equal(clipAroundTime(null,100),null);assert.equal(clipAroundTime(120,100),null);
  assert.equal(parseClipTime("01:02:03.5"),3723.5);assert.equal(parseClipTime("1:99"),null);assert.equal(parseClipTime(""),null);
});

const flush=()=>new Promise(resolve=>setImmediate(resolve));
function fixture(t,{ready=true,map=[]}={}) {
  const previousDocument=globalThis.document,calls=[],requests=[],notices=[];
  const element={querySelector:()=>null,querySelectorAll:()=>[],addEventListener(){},replaceWith(){},classList:{toggle(){}},parentElement:null};
  const root={innerHTML:"",contains:()=>false,querySelector:selector=>selector===".timeline-player"?element:null,querySelectorAll:()=>[]};
  globalThis.document={querySelector:selector=>selector==="#session-timeline"?root:null,activeElement:null};
  const state={modal:{kind:"detail",rid:"1"}};
  let media;
  const controller=createSessionTimeline({state,toast:(...args)=>notices.push(args),onCreateClip:async request=>{requests.push(request);return {job_id:"job-a"};},
    createMedia:options=>media={element,hasMedia:false,duration:100,currentTime:5,paused:true,setSource:async source=>{media.hasMedia=Boolean(source);if(source&&ready)options.onReady();},play:async()=>{},pause(){},speed(){},muted(){},seek(){},destroy(){}},
    bridge:async(method,params)=>{calls.push([method,params]);
      if(method==="session_replay")return {enabled:true,available:true,replay:{play_url:"https://media.example/recording.m3u8",duration_ms:100000,time_mapping:map}};
      if(method==="highlight_settings")return {base_url:"https://models.example/v1",key_configured:true,model:"chat"};
      if(method==="highlight_analyze")return {job_id:"analysis"};
      if(method==="highlight_status")return {status:"completed",result:{highlights:[{offset_seconds:5,title:"观众追问"}]}};
      return {session_id:"session-a",start_epoch:1000,duration_seconds:100,messages:[],buckets:[],activities:[],total_messages:1};
    }});
  t.after(()=>{controller.unmount();globalThis.document=previousDocument;});
  controller.mount({rid:"1",anchor_id:"anchor-a",session_id:"session-a",is_live:false});
  return {controller,root,calls,requests,notices,media:()=>media};
}
test("a playable highlight creates a padded media range without starting another AI request",async t=>{
  const f=fixture(t);await flush();await f.controller.action({dataset:{action:"timeline-analyze"}});
  await f.controller.action({dataset:{action:"timeline-highlight-clip",highlight:"0"}});
  assert.deepEqual(f.requests,[{source:"highlight",session_id:"session-a",anchor_id:"anchor-a",start_seconds:0,end_seconds:35,title:"观众追问"}]);
  assert.equal(f.calls.filter(([method])=>method==="highlight_analyze").length,1);
  assert.match(f.root.innerHTML,/timeline-highlight-entry/);
});
test("loading media cannot export a fake clip",async t=>{
  const f=fixture(t,{ready:false});await flush();
  await f.controller.action({dataset:{action:"timeline-clip-toggle"}});await f.controller.action({dataset:{action:"timeline-clip-download"}});
  assert.equal(f.requests.length,0);assert.ok(f.notices.some(([message])=>/没有可播放录像/.test(message)));
});
test("an unmapped highlight does not silently snap across a wall-clock gap",async t=>{
  const f=fixture(t,{map:[{offset_seconds:0,duration:20,program_date_time:1200}]});await flush();
  await f.controller.action({dataset:{action:"timeline-analyze"}});await f.controller.action({dataset:{action:"timeline-highlight-clip",highlight:"0"}});
  assert.equal(f.requests.length,0);assert.ok(f.notices.some(([message])=>/没有可对齐/.test(message)));
});
test("manual endpoints use media seconds and reject inverted or out-of-duration input",async t=>{
  const f=fixture(t);await flush();await f.controller.action({dataset:{action:"timeline-clip-toggle"}});
  f.controller.input({id:"timeline-clip-start",value:"00:00:10"});f.controller.input({id:"timeline-clip-end",value:"00:00:25"});
  await f.controller.action({dataset:{action:"timeline-clip-download"}});
  assert.equal(f.requests[0].start_seconds,10);assert.equal(f.requests[0].end_seconds,25);
  f.controller.input({id:"timeline-clip-end",value:"00:00:05"});await f.controller.action({dataset:{action:"timeline-clip-download"}});assert.equal(f.requests.length,1);
  f.controller.input({id:"timeline-clip-end",value:"00:02:00"});await f.controller.action({dataset:{action:"timeline-clip-download"}});
  assert.equal(f.requests.length,1);
});
