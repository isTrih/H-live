import test from "node:test";
import assert from "node:assert/strict";
import {createNativeReplayMedia} from "./native-replay-media.js";

const flush=()=>new Promise(resolve=>setImmediate(resolve));

// Reaches the production MutationObserver -> publish -> structure path. The
// dense static markup models activity intervals/tooltips, rather than timing a
// browser's unrelated layout or an IPC mock's wall-clock scheduling.
async function fixture(t) {
  t.mock.timers.enable({apis:["setTimeout","setInterval"]});
  let now=0,listener,observe,clones=0,notice="拖动进度条选择片段";
  const states=[],calls=[],handlers=new Map(),classes=new Set(["timeline-player","controls-visible"]);
  const attrs=new Map([["aria-label","暂停回放"],["disabled",null],["hidden",null]]);
  const target={getAttribute:name=>attrs.get(name)??null,closest:()=>null,matches:()=>false};
  const rect={left:0,top:0,right:640,bottom:360,width:640,height:360};
  const dense=Array.from({length:1000},(_,i)=>`<button data-action="timeline-activity" data-activity="${i}"><span role="tooltip">福袋区间 ${i} · 奖品及参与条件</span></button>`).join("");
  const html=()=>`<div class="timeline-video"><div class="timeline-danmaku"></div></div><div class="timeline-controls"><button data-action="timeline-play" aria-label="${attrs.get("aria-label")}">播放</button><span class="timeline-clip-message">${notice}</span>${dense}</div>`;
  const view={innerWidth:1000,innerHeight:800,addEventListener(){},removeEventListener(){},MutationObserver:class{constructor(callback){observe=callback;}observe(){}disconnect(){}}};
  const doc={defaultView:view,addEventListener(){},removeEventListener(){}};
  const element={ownerDocument:doc,isConnected:true,get className(){return [...classes].join(" ");},getAttribute:name=>name==="class"?[...classes].join(" "):null,
    classList:{add:key=>classes.add(key),remove:key=>classes.delete(key),contains:key=>classes.has(key),toggle(key,force){if(force)classes.add(key);else classes.delete(key);}},
    closest:()=>null,matches:()=>false,getBoundingClientRect:()=>rect,querySelector:selector=>selector===".timeline-video"?{getBoundingClientRect:()=>rect}:null,
    querySelectorAll:()=>[],cloneNode:()=>{clones++;return {innerHTML:html(),querySelectorAll:()=>[],querySelector:()=>null};},addEventListener:(name,handler)=>handlers.set(name,handler),removeEventListener:name=>handlers.delete(name)};
  const player=createNativeReplayMedia({element,clock:()=>now,
    onTime:()=>observe?.(["aria-label","disabled","hidden"].map(attributeName=>({type:"attributes",target,attributeName,oldValue:target.getAttribute(attributeName)}))),
    listen:async(_name,callback)=>{listener=callback;return()=>{};},emitTo:async(_target,_name,payload)=>states.push(payload),
    invoke:async(method)=>{calls.push(method);return {time_pos:5+now/1000,duration:100,paused:false,volume:1,muted:false,frame_ready:true};}});
  t.after(()=>player.destroy());
  await player.setSource({play_url:"https://media.example/replay.m3u8",duration_ms:100000});
  listener({payload:{kind:"ready"}});await flush();
  return {player,states,calls,target,attrs,observe,element,handlers,setNotice:value=>{notice=value;},clones:()=>clones,async tick(){now+=250;t.mock.timers.tick(250);await flush();}};
}

test("unchanged playback attribute writes do not repeatedly clone dense native-overlay HTML",async t=>{
  const f=await fixture(t),initial=f.clones();
  for(let i=0;i<40;i++)await f.tick();
  const sent=f.states.filter(payload=>typeof payload.html==="string");
  assert.equal(sent.length,1,"unchanged activity markup is already sent only once");
  assert.ok(f.calls.filter(method=>method==="native_replay_status").length>=35,"the real status/publish loop continued running");
  assert.equal(f.clones()-initial,0,`unchanged controls cloned dense player ${f.clones()-initial} times in 10 seconds`);
});

test("a real control-label change still refreshes native-overlay structure",async t=>{
  const f=await fixture(t);
  f.attrs.set("aria-label","播放回放");
  f.observe([{type:"attributes",target:f.target,attributeName:"aria-label",oldValue:"暂停回放"}]);
  await f.tick();
  assert.match(f.states.filter(payload=>payload.html).at(-1).html,/aria-label="播放回放"/);
});

const text=value=>({nodeType:3,textContent:value});
const replacement=(target,before,after)=>({type:"childList",target,removedNodes:[text(before)],addedNodes:[text(after)]});

test("clock, moving comments and equal notice text do not invalidate overlay structure",async t=>{
  const f=await fixture(t),initial=f.clones();
  const clock={closest:selector=>selector===".timeline-position,.timeline-video-time"?clock:null};
  const danmaku={closest:selector=>selector===".timeline-danmaku"?danmaku:null};
  const comment={closest:selector=>selector===".timeline-danmaku"?danmaku:null,getAttribute:()=>"left:50%"};
  const cursor={closest:()=>null,matches:selector=>selector===".timeline-cursor",getAttribute:()=>"left:25%"};
  for(let frame=0;frame<40;frame++){
    f.observe([
      replacement(clock,`00:00:${frame}`,`00:00:${frame+1}`),
      {type:"childList",target:danmaku,removedNodes:[],addedNodes:[{nodeType:1}]},
      {type:"attributes",target:comment,attributeName:"style",oldValue:"left:51%"},
      {type:"attributes",target:cursor,attributeName:"style",oldValue:"left:24%"},
      replacement(f.target,"拖动进度条选择片段","拖动进度条选择片段"),
    ]);
    await f.tick();
  }
  assert.equal(f.clones(),initial,"separately synchronized motion and unchanged text must not deep-clone activities");
});

test("controls visibility is synchronized immediately without recreating activity markup",async t=>{
  const f=await fixture(t),initial=f.clones();
  for(let i=0;i<12;i++)await f.tick();
  assert.doesNotMatch(f.states.at(-1).className,/controls-visible/);
  f.handlers.get("pointermove")();await f.tick();
  assert.match(f.states.at(-1).className,/controls-visible/);
  assert.equal(f.clones(),initial);
});

test("changed notices, activities and explicit refresh still update real overlay markup",async t=>{
  const f=await fixture(t),initial=f.clones();
  f.setNotice("正在创建下载任务…");
  f.observe([replacement(f.target,"拖动进度条选择片段","正在创建下载任务…")]);
  await f.tick();
  assert.equal(f.clones(),initial+1);
  assert.match(f.states.filter(payload=>payload.html).at(-1).html,/正在创建下载任务/);
  f.setNotice("切片已保存");
  f.observe([{type:"childList",target:f.target,removedNodes:[],addedNodes:[{nodeType:1}]}]);
  await f.tick();
  assert.match(f.states.filter(payload=>payload.html).at(-1).html,/切片已保存/);
  const beforeRefresh=f.clones();
  f.player.refreshOverlay();await flush();
  assert.equal(f.clones(),beforeRefresh+1,"an explicit renderer refresh must publish even if its HTML string is unchanged");
});
