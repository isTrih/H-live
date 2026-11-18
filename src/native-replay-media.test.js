import test from "node:test";
import assert from "node:assert/strict";
import {createNativeReplayMedia} from "./native-replay-media.js";
import {replayTimeMap} from "./replay-sync.js";
import {replayClockMetadata,playbackClock} from "./replay-clock.js";

const flush=()=>new Promise(resolve=>setImmediate(resolve));
function fixture(t,{initial={},initialAudio,open,bounds,modalBounds,seekReportsOld=false,getClockMetadata}={}){
  let now=0,listener,removed=0;
  const calls=[],states=[],events=[],errors=[],ready=[],fullscreenEvents=[],classes=new Set(["timeline-player"]),listeners=new Map();
  const rect=bounds||{left:10,top:20,right:650,bottom:380,width:640,height:360};
  const control={id:"",dataset:{action:"timeline-play"},dispatchEvent:event=>events.push(event)};
  const view={innerWidth:1000,innerHeight:800,Event:class{constructor(type,options){this.type=type;Object.assign(this,options);}},MouseEvent:class{constructor(type,options){this.type=type;Object.assign(this,options);}},PointerEvent:class{constructor(type,options){this.type=type;Object.assign(this,options);}},addEventListener(){},removeEventListener(){},dispatchEvent:event=>events.push(event)};
  const doc={defaultView:view,addEventListener(){},removeEventListener(){}};
  const element={ownerDocument:doc,isConnected:true,get className(){return [...classes].join(" ");},classList:{add:key=>classes.add(key),remove:key=>classes.delete(key),contains:key=>classes.has(key),toggle(key,force){if(force)classes.add(key);else classes.delete(key);}},closest:selector=>selector===".modal-body"&&modalBounds?{getBoundingClientRect:()=>modalBounds}:null,getBoundingClientRect:()=>rect,
    querySelector:selector=>selector===".timeline-video"?{getBoundingClientRect:()=>rect}:null,
    querySelectorAll:selector=>selector==="[id],[data-action],[data-clip-edge]"?[control]:[],
    cloneNode:()=>({innerHTML:'<div class="timeline-video"><div class="timeline-danmaku"><span>同一条弹幕</span></div></div><div class="timeline-controls"><b class="timeline-position">00:00:05</b><button data-action="timeline-play">播放</button></div>',querySelectorAll:()=>[],querySelector:()=>null}),
    addEventListener:(name,handler)=>listeners.set(name,handler),removeEventListener:name=>listeners.delete(name)};
  let nativeState={time_pos:5,duration:100,paused:true,volume:0,muted:false,vo_configured:true,frame_ready:true,hwdec:"d3d11va",...initial};
  const player=createNativeReplayMedia({element,initialAudio,getClockMetadata,clock:()=>now,onReady:value=>ready.push(value),onError:value=>errors.push(value),onFullscreen:value=>fullscreenEvents.push(value),
    listen:async(_name,handler)=>{listener=handler;return()=>removed++;},emitTo:async(_target,_name,state)=>states.push(state),
    invoke:async(method,params)=>{calls.push([method,params]);if(method==="native_replay_open")return open?open():nativeState;if(method==="native_replay_action"){if(params.action==="play")nativeState={...nativeState,paused:false};if(params.action==="seek"&&!seekReportsOld)nativeState={...nativeState,time_pos:params.value};if(params.action==="volume")nativeState={...nativeState,volume:params.value};if(params.action==="mute")nativeState={...nativeState,muted:Boolean(params.value)};return nativeState;}if(method==="native_replay_status")return nativeState;return {};}});
  t.after(()=>player.destroy());
  return {player,calls,states,events,errors,ready,fullscreenEvents,element,document:doc,listener:()=>listener,advance:ms=>{now+=ms;},removed:()=>removed,listeners,setNativeState:values=>{nativeState={...nativeState,...values};}};
}
test("native readiness requires a configured output and media time interpolates between low-rate status samples",async t=>{
  const f=fixture(t);await f.player.setSource({play_url:"https://media.example/replay.m3u8",duration_ms:100000});
  const opening=f.calls.find(([method])=>method==="native_replay_open")[1];assert.equal(opening.volume,1);assert.equal(opening.muted,false);
  assert.equal(f.ready.length,1);assert.equal(f.ready[0].hwdec,"d3d11va");assert.equal(f.player.currentMuted,true);assert.equal(f.player.currentVolume,1);
  await f.player.play();f.advance(600);assert.equal(f.player.currentTime,5.6);
  assert.equal(f.calls.filter(([method])=>method==="native_replay_status").length,0,"interpolation does not add IPC calls");
  f.player.seek(200);await flush();assert.equal(f.calls.find(([method,p])=>method==="native_replay_action"&&p.action==="seek")[1].value,100);
  f.player.muted(false);await flush();assert.ok(f.calls.some(([method,p])=>method==="native_replay_action"&&p.action==="volume"&&p.value===1));
});
test("native overlay receives compact raw-PDT clock metadata while seeks remain media seconds",async t=>{
  const start=Date.parse("2026-10-25T06:00:00Z")/1000;
  const map=replayTimeMap({time_mapping:Array.from({length:5447},(_,i)=>({offset_seconds:i*4,duration:4,program_date_time:start+i*4}))},start,7200);
  const f=fixture(t,{initial:{duration:21788},getClockMetadata:(time,duration)=>replayClockMetadata(map,time,duration)});
  await f.player.setSource({play_url:"https://media.example/replay.m3u8",duration_ms:21788000});
  f.listener()({payload:{kind:"ready"}});await flush();
  const metadata=f.states.at(-1).worldClock;
  assert.equal(metadata.spans.length,1);assert.ok(JSON.stringify(metadata).length<400);
  assert.equal(playbackClock(metadata,5,21788).position,"14:00:05");
  f.player.seek(35);await flush();
  assert.equal(f.calls.find(([method,p])=>method==="native_replay_action"&&p.action==="seek")[1].value,35);
  assert.equal(playbackClock(f.states.at(-1).worldClock,35,21788).position,"14:00:35");
});

test("native open receives saved audio before loading and zero-volume resumes its last audible level",async t=>{
  const f=fixture(t,{initialAudio:{volume:0,muted:true,lastAudibleVolume:.28},initial:{volume:0,muted:true}});
  await f.player.setSource({play_url:"https://media.example/replay.m3u8"});
  const params=f.calls.find(([method])=>method==="native_replay_open")[1];
  assert.equal(params.volume,0);assert.equal(params.muted,true);
  f.player.muted(false);await flush();
  assert.ok(f.calls.some(([method,p])=>method==="native_replay_action"&&p.action==="volume"&&p.value===.28));
  assert.equal(f.player.currentVolume,.28);assert.equal(f.player.currentMuted,false);
});

test("volume changed before native opening is carried into the first native command",async t=>{
  const f=fixture(t,{initial:{volume:.31,muted:false}});
  f.player.volume(.31);
  assert.equal(f.calls.length,0,"setting pre-open audio must not create a native surface");
  await f.player.setSource({play_url:"https://media.example/replay.m3u8"});
  const params=f.calls.find(([method])=>method==="native_replay_open")[1];
  assert.equal(params.volume,.31);assert.equal(params.muted,false);
});

test("a stale opening response cannot override audio selected while the native player opens",async t=>{
  let finish;
  const f=fixture(t,{initialAudio:{volume:.75,muted:false},initial:{volume:.25,muted:true},open:()=>new Promise(resolve=>finish=resolve)});
  const opening=f.player.setSource({play_url:"https://media.example/replay.m3u8"});await flush();
  f.player.volume(.25);f.player.muted(true);
  finish({time_pos:0,duration:100,paused:true,volume:.75,muted:false,frame_ready:false});
  await opening;
  assert.equal(f.player.currentVolume,.25);assert.equal(f.player.currentMuted,true);
  await flush();
  assert.equal(f.player.currentVolume,.25);assert.equal(f.player.currentMuted,true);
});
test("native status errors close the surface and do not expose signed media URLs",async t=>{
  const f=fixture(t,{initial:{vo_configured:false,frame_ready:false,error:"cannot load https://media.example/a?token=private"}});
  await f.player.setSource({play_url:"https://media.example/replay.m3u8"});await flush();
  assert.equal(f.player.hasMedia,false);assert.equal(f.ready.length,0);assert.equal(f.errors.length,1);assert.doesNotMatch(f.errors[0],/token=private|media\.example/);
  assert.ok(f.calls.filter(([method])=>method==="native_replay_close").length>=2);
});
test("no first frame reaches a bounded failure instead of staying in loading forever",async t=>{
  t.mock.timers.enable({apis:["setTimeout"]});
  const f=fixture(t,{initial:{vo_configured:true,frame_ready:false,width:0,height:0}});await f.player.setSource({play_url:"https://media.example/replay.m3u8"});
  assert.equal(f.ready.length,0);t.mock.timers.tick(45000);await flush();
  assert.match(f.errors[0],/首帧加载超时/);assert.equal(f.player.hasMedia,false);
});
test("video bounds preserve the video size while clipping to the visible modal and viewport intersection",async t=>{
  const f=fixture(t,{bounds:{left:-20,top:10,right:620,bottom:370,width:640,height:360},modalBounds:{left:0,top:40,right:600,bottom:300}});
  await f.player.setSource({play_url:"https://media.example/replay.m3u8"});
  const bounds=f.calls.find(([method])=>method==="native_replay_open")[1].bounds;
  assert.deepEqual(bounds,{x:-20,y:10,width:640,height:360,clip:{x:20,y:30,width:600,height:260}});
});
test("overlay actions use the real parent controls and destruction removes the subscription",async t=>{
  const f=fixture(t);await f.player.setSource({play_url:"https://media.example/replay.m3u8"});
  f.listener()({payload:{kind:"click",dataset:{action:"timeline-play"}}});assert.equal(f.events.at(-1).type,"click");
  f.listener()({payload:{kind:"pointercancel",x:20,y:30,pointerId:7}});assert.equal(f.events.at(-1).type,"pointercancel");assert.equal(f.events.at(-1).clientX,30);
  f.player.destroy();await flush();assert.equal(f.removed(),1);assert.equal(f.listeners.size,0);assert.equal(f.calls.at(-1)[0],"native_replay_close");
});

test("archive-only danmaku keeps browser fullscreen without opening a native video window",async t=>{
  const f=fixture(t);let entered=0,exited=0;
  f.element.requestFullscreen=async()=>{entered++;f.document.fullscreenElement=f.element;};
  f.document.exitFullscreen=async()=>{exited++;f.document.fullscreenElement=null;};
  await f.player.fullscreen();assert.equal(f.player.isFullscreen,true);assert.equal(entered,1);
  await f.player.fullscreen();assert.equal(f.player.isFullscreen,false);assert.equal(exited,1);
  assert.equal(f.calls.filter(([method])=>method==="native_replay_open"||method==="native_replay_action").length,0);
});

test("the native child owns one visible copy of time, danmaku and controls after its ready handshake",async t=>{
  const f=fixture(t);await f.player.setSource({play_url:"https://media.example/replay.m3u8"});
  assert.equal(f.element.classList.contains("native-replay-host"),false,"the source placeholder remains visible before the overlay is ready");
  assert.equal(f.states.length,0,"no cloned visuals can reach the child before ownership is transferred");
  f.listener()({payload:{kind:"ready"}});await flush();
  assert.equal(f.element.classList.contains("native-replay-host"),true,"hide source visuals once the child can display the cloned content");
  await f.player.play();f.player.pause();await flush();await f.player.fullscreen();await flush();
  for(const payload of f.states){
    assert.doesNotMatch(payload.className,/native-replay-host/,"a source-only hiding class must never hide the child overlay");
    if(payload.html){assert.equal((payload.html.match(/class="timeline-controls"/g)||[]).length,1);assert.equal((payload.html.match(/class="timeline-danmaku"/g)||[]).length,1);assert.equal((payload.html.match(/class="timeline-position"/g)||[]).length,1);}
  }
  assert.ok(f.states.some(payload=>payload.html));
  await f.player.setSource(null);assert.equal(f.element.classList.contains("native-replay-host"),false,"archive-only controls are visible again");
});

test("failed native startup and destruction always restore the source DOM",async t=>{
  const f=fixture(t,{open:async()=>{throw Error("GPU window unavailable");}});await f.player.setSource({play_url:"https://media.example/replay.m3u8"});
  f.listener()({payload:{kind:"ready"}});assert.equal(f.element.classList.contains("native-replay-host"),false);
  const ready=fixture(t);await ready.player.setSource({play_url:"https://media.example/replay.m3u8"});ready.listener()({payload:{kind:"ready"}});assert.equal(ready.element.classList.contains("native-replay-host"),true);
  ready.player.destroy();assert.equal(ready.element.classList.contains("native-replay-host"),false);
});

test("the buffering placeholder transfers before the first overlay payload even while native open is pending",async t=>{
  let finishOpen;const f=fixture(t,{open:()=>new Promise(resolve=>{finishOpen=resolve;})});
  t.after(()=>finishOpen?.({frame_ready:false,paused:true,duration:100}));
  const opening=f.player.setSource({play_url:"https://media.example/replay.m3u8"});await flush();
  assert.equal(f.element.classList.contains("native-replay-host"),false);
  assert.equal(f.states.length,0,"an unready overlay has received no source visuals");
  f.listener()({payload:{kind:"ready"}});await flush();
  assert.equal(f.element.classList.contains("native-replay-host"),true,"source visuals must be hidden before the child first displays buffering UI");
  assert.ok(f.states.some(payload=>payload.html));assert.doesNotMatch(f.states.at(-1).className,/native-replay-host/);
  finishOpen({frame_ready:false,paused:true,duration:100});await opening;
  assert.equal(f.element.classList.contains("native-replay-host"),true);
});

test("an open failure after the overlay handshake restores the loading and retry surface",async t=>{
  let failOpen;const f=fixture(t,{open:()=>new Promise((_resolve,reject)=>{failOpen=reject;})});
  t.after(()=>failOpen?.(Error("diagnostic cleanup")));
  const opening=f.player.setSource({play_url:"https://media.example/replay.m3u8"});await flush();
  f.listener()({payload:{kind:"ready"}});assert.equal(f.element.classList.contains("native-replay-host"),true);
  failOpen(Error("GPU window unavailable"));await opening;
  assert.equal(f.element.classList.contains("native-replay-host"),false);assert.equal(f.errors.length,1);
});

test("switching a fullscreen native replay to an archive-only session clears the fixed surface and restores browser fullscreen",async t=>{
  const f=fixture(t);await f.player.setSource({play_url:"https://media.example/replay.m3u8"});await f.player.fullscreen();
  assert.equal(f.player.isFullscreen,true);assert.equal(f.element.classList.contains("native-replay-fullscreen"),true);
  await f.player.setSource(null);
  assert.equal(f.player.isFullscreen,false);assert.equal(f.element.classList.contains("native-replay-fullscreen"),false);assert.equal(f.fullscreenEvents.at(-1),false);
  f.element.requestFullscreen=async()=>{f.document.fullscreenElement=f.element;};
  f.document.exitFullscreen=async()=>{f.document.fullscreenElement=null;};
  await f.player.fullscreen();assert.equal(f.document.fullscreenElement,f.element);assert.equal(f.player.isFullscreen,true);
});

test("a fullscreen playback failure clears native fullscreen before restoring the source error surface",async t=>{
  const f=fixture(t);await f.player.setSource({play_url:"https://media.example/replay.m3u8"});f.listener()({payload:{kind:"ready"}});await f.player.fullscreen();
  f.setNativeState({error:"GPU output lost"});await f.player.play();await flush();
  assert.equal(f.errors.length,1);assert.equal(f.player.isFullscreen,false);assert.equal(f.element.classList.contains("native-replay-fullscreen"),false);assert.equal(f.element.classList.contains("native-replay-host"),false);assert.equal(f.fullscreenEvents.at(-1),false);
});

test("setting an empty source preserves an archive-only browser fullscreen session",async t=>{
  const f=fixture(t);f.element.requestFullscreen=async()=>{f.document.fullscreenElement=f.element;};f.document.exitFullscreen=async()=>{f.document.fullscreenElement=null;};
  await f.player.fullscreen();const count=f.fullscreenEvents.length;
  await f.player.setSource(null);
  assert.equal(f.player.isFullscreen,true);assert.equal(f.document.fullscreenElement,f.element);assert.equal(f.fullscreenEvents.length,count);
});

test("old seek replies and status samples cannot pull the optimistic playhead back before the target settles",async t=>{
  t.mock.timers.enable({apis:["setTimeout","setInterval"]});const f=fixture(t,{seekReportsOld:true,initial:{paused:false}});
  await f.player.setSource({play_url:"https://media.example/replay.m3u8"});f.listener()({payload:{kind:"ready"}});const revision=f.states.at(-1).danmakuRevision;
  f.player.seek(35);await flush();assert.equal(f.player.currentTime,35);
  assert.ok(f.states.at(-1).danmakuRevision>revision);
  f.advance(300);f.setNativeState({time_pos:5.3});t.mock.timers.tick(250);await flush();assert.ok(Math.abs(f.player.currentTime-35.3)<.001);
  f.advance(100);f.setNativeState({time_pos:35.4});t.mock.timers.tick(250);await flush();assert.ok(Math.abs(f.player.currentTime-35.4)<.001);
  f.advance(100);f.setNativeState({time_pos:35.5});t.mock.timers.tick(250);await flush();assert.ok(Math.abs(f.player.currentTime-35.5)<.001);
  assert.equal(f.errors.length,0);
});

test("a newer short seek replaces the old target, and clearing the source cancels its settle deadline",async t=>{
  t.mock.timers.enable({apis:["setTimeout","setInterval"]});const f=fixture(t,{seekReportsOld:true});await f.player.setSource({play_url:"https://media.example/replay.m3u8"});f.listener()({payload:{kind:"ready"}});
  f.player.seek(7);await flush();const revision=f.states.at(-1).danmakuRevision;
  f.player.seek(5.5);await flush();assert.equal(f.player.currentTime,5.5);assert.ok(f.states.at(-1).danmakuRevision>revision);
  await f.player.setSource(null);t.mock.timers.tick(9000);await flush();assert.equal(f.errors.length,0);assert.equal(f.player.hasMedia,false);
});

test("an unconfirmed seek reaches a bounded retryable error instead of keeping an invented target forever",async t=>{
  t.mock.timers.enable({apis:["setTimeout","setInterval"]});const f=fixture(t,{seekReportsOld:true});await f.player.setSource({play_url:"https://media.example/replay.m3u8"});
  f.player.seek(35);await flush();t.mock.timers.tick(8000);await flush();assert.match(f.errors[0],/定位超时/);assert.equal(f.player.hasMedia,false);
});
