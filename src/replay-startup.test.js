import test from "node:test";
import assert from "node:assert/strict";
import {createSessionTimeline} from "./session-timeline.js";
import {createPlaybackPreferences} from "./playback-preferences.js";
import {createBookmarkDocument} from "./test-support/bookmark-dom.js";

const flush=()=>new Promise(resolve=>setImmediate(resolve));
const anchor={rid:"111",anchor_id:"11",session_id:"21",state:"connected",active:true,is_live:true};
const replay={play_url:"https://media.xingtu.cn/replay.m3u8",duration_ms:120000};
const archive={anchor_id:"11",session_id:"21",start_epoch:1792735200,duration_seconds:120,messages:[],buckets:[],activities:[]};

function fixture(t,{source=replay,autoReady=true,deferReplay=false}={}){
  const previous=globalThis.document,document=createBookmarkDocument();globalThis.document=document;
  const pending=new Map(),requests=[],calls=[],media=[];
  const controller=createSessionTimeline({state:{modal:{kind:"detail",rid:"111"}},audioPreferences:createPlaybackPreferences({getStorage:()=>null}),
    bridge:(method,params)=>{
      calls.push([method,params]);
      if(method==="session_replay"&&!deferReplay)return Promise.resolve({enabled:true,available:true,replay:typeof source==="function"?source(params):source});
      return new Promise((resolve,reject)=>{pending.set(method,resolve);requests.push({method,params,resolve,reject});});
    },
    createMedia:options=>{
      let audio={...options.initialAudio};
      const player={element:options.element,hasMedia:false,isFullscreen:false,currentTime:0,duration:120,paused:true,
        sources:[],plays:0,options,
        async setSource(value){this.sources.push(value);this.hasMedia=Boolean(value);if(value&&autoReady)options.onReady({duration:120});},
        seek(value){this.currentTime=value;options.onTime(value);return true;},
        speed(){},pause(){this.paused=true;options.onPlayback(false);},
        async play(){this.plays++;this.paused=false;options.onPlayback(true);return true;},
        volume(value){audio={volume:value,muted:value===0};options.onVolume(audio);},
        muted(value){audio={...audio,muted:value};options.onVolume(audio);},destroy(){},
      };media.push(player);return player;
    },
  });
  t.after(()=>{controller.unmount();globalThis.document=previous;});
  return {controller,document,pending,requests,calls,media};
}

test("replay source and playback start while the archive, model settings and bookmarks are still loading",async t=>{
  const f=fixture(t);f.controller.mount(anchor);await flush();await flush();
  assert.equal(f.media.length,1);
  assert.equal(f.media[0].sources.length,1);
  assert.equal(f.media[0].sources[0].play_url,replay.play_url);
  assert.equal(f.media[0].hasMedia,true);
  assert.ok(f.media[0].plays>0);
  assert.equal(f.pending.has("session_timeline"),true);
  assert.equal(f.pending.has("highlight_settings"),true);
  assert.equal(f.pending.has("mark_list"),true);
  assert.match(f.document.querySelector(".timeline-heading-date").textContent,/场次时间待确认/);
});

test("late archive and settings do not reload or unpause an already playable recording",async t=>{
  const f=fixture(t);f.controller.mount(anchor);await flush();await flush();
  const player=f.media[0],node=player.element;
  player.currentTime=31;player.pause();
  f.controller.input({id:"timeline-volume",value:"0.24"});
  const plays=player.plays;
  f.pending.get("session_timeline")(archive);
  f.pending.get("highlight_settings")({});
  f.pending.get("mark_list")({marks:[],total:0});
  await flush();await flush();
  assert.equal(f.media.length,1);
  assert.equal(player.element,node);
  assert.equal(node.isConnected,true);
  assert.equal(player.sources.length,1);
  assert.equal(player.currentTime,31);
  assert.equal(player.paused,true);
  assert.equal(player.plays,plays);
});

const fastReplay={...replay,time_mapping_pending:true,time_mapping:[]};
const timing=url=>({play_url:url,time_mapping_pending:false,playlist_duration_ms:120000,
  time_mapping:[{offset_seconds:0,duration:120,program_date_time:1792735200,time_source:"pdt"}]});

test("timing loads after the first decoded frame and never blocks media-seconds playback",async t=>{
  const f=fixture(t,{source:fastReplay,autoReady:false});f.controller.mount(anchor);await flush();await flush();
  const player=f.media[0];
  assert.equal(player.sources.length,1);
  assert.equal(f.calls.filter(([method])=>method==="session_replay_timing").length,0);
  assert.equal(player.options.getClockMetadata(0,120).mode,"elapsed");
  player.options.onReady({duration:120});await flush();
  const request=f.requests.find(request=>request.method==="session_replay_timing");
  assert.ok(request);assert.deepEqual(request.params,{session_id:"21",play_url:replay.play_url});
  assert.equal(player.sources.length,1,"timing must not call setSource again");
  f.controller.input({id:"timeline-media-seek",value:"31"});
  assert.equal(player.currentTime,31,"manual media-second seeks remain available");
  assert.match(f.document.querySelector(".timeline-timing-status").textContent,/正在对齐时间轴/);
  const plays=player.plays;player.pause();f.controller.input({id:"timeline-volume",value:"0.24"});
  request.resolve(timing(replay.play_url));await flush();await flush();
  assert.equal(player.options.getClockMetadata(31,120).mode,"pdt");
  assert.equal(player.sources.length,1);assert.equal(player.currentTime,31);
  assert.equal(player.paused,true);assert.equal(player.plays,plays);
  assert.equal(f.document.querySelector("#timeline-volume").value,"0.24");
});

test("timing failures keep video usable and only retry timing rather than the media source",async t=>{
  const f=fixture(t,{source:fastReplay});f.controller.mount(anchor);await flush();await flush();
  const player=f.media[0],request=f.requests.find(request=>request.method==="session_replay_timing");
  assert.ok(request);request.reject(Error("timing network unavailable"));await flush();
  assert.equal(player.hasMedia,true);assert.equal(player.sources.length,1);
  assert.match(f.document.querySelector(".timeline-timing-status").textContent,/时间轴暂未对齐/);
  assert.equal(f.document.querySelector('.timeline-error'),null,"a timing failure must not become a playback failure");
  const retry=f.controller.action({dataset:{action:"timeline-timing-refresh"}});await flush();
  const retried=f.requests.filter(request=>request.method==="session_replay_timing").at(-1);
  assert.notEqual(retried,request);retried.resolve(timing(replay.play_url));await retry;await flush();
  assert.equal(player.sources.length,1);assert.equal(player.options.getClockMetadata(0,120).mode,"pdt");
});

test("late timing from another session cannot overwrite the selected source",async t=>{
  const f=fixture(t,{source:params=>({...fastReplay,play_url:`https://media.xingtu.cn/${params.session_id}.m3u8`})});
  f.controller.mount(anchor);await flush();await flush();
  const first=f.requests.find(request=>request.method==="session_replay_timing");assert.ok(first);
  f.controller.selectSession("22");await flush();await flush();
  const second=f.requests.filter(request=>request.method==="session_replay_timing").at(-1);
  assert.equal(second.params.session_id,"22");
  first.resolve(timing(first.params.play_url));await flush();
  assert.equal(f.media[0].options.getClockMetadata(0,120).mode,"elapsed");
  second.resolve(timing(second.params.play_url));await flush();
  assert.equal(f.media[0].options.getClockMetadata(0,120).mode,"pdt");
  assert.equal(f.media[0].sources.filter(Boolean).length,2);
});

test("a mismatched playback URL is rejected without stopping video",async t=>{
  const f=fixture(t,{source:fastReplay});f.controller.mount(anchor);await flush();await flush();
  const request=f.requests.find(request=>request.method==="session_replay_timing");assert.ok(request);
  request.resolve(timing("https://media.xingtu.cn/other.m3u8"));await flush();
  assert.equal(f.media[0].hasMedia,true);assert.equal(f.media[0].options.getClockMetadata(0,120).mode,"elapsed");
  assert.match(f.document.querySelector(".timeline-timing-status").textContent,/时间轴暂未对齐/);
});

test("fullscreen applies timing in place while preserving the video and active clipping controls",async t=>{
  const f=fixture(t,{source:fastReplay});f.controller.mount(anchor);await flush();await flush();
  const player=f.media[0],request=f.requests.find(request=>request.method==="session_replay_timing");assert.ok(request);
  player.isFullscreen=true;player.options.onFullscreen(true);player.currentTime=31;
  const plays=player.plays,node=player.element,seek=node.querySelector('#timeline-media-seek'),cursor=node.querySelector('.timeline-cursor'),handle=node.querySelector('[data-clip-edge="start"]');
  handle.focus();request.resolve(timing(replay.play_url));await flush();
  assert.equal(player.options.getClockMetadata(31,120).mode,"pdt","long fullscreen playback must not wait for exit to align clocks and comments");
  assert.equal(player.element,node);assert.equal(node.isConnected,true);
  assert.equal(node.querySelector('#timeline-media-seek'),seek);assert.equal(node.querySelector('.timeline-cursor'),cursor);
  assert.equal(node.querySelector('[data-clip-edge="start"]'),handle);assert.equal(f.document.activeElement,handle);
  assert.match(node.querySelector('.timeline-media-axis').textContent,/14:00:00/);
  player.isFullscreen=false;player.options.onFullscreen(false);await flush();
  assert.equal(player.options.getClockMetadata(31,120).mode,"pdt");
  assert.equal(player.currentTime,31);assert.equal(player.plays,plays);assert.equal(player.sources.length,1);
});

test("the slow-loading retry hint covers lookup time before a replay URL exists",async t=>{
  const previousNow=Date.now;let now=1000;Date.now=()=>now;t.after(()=>{Date.now=previousNow;});
  t.mock.timers.enable({apis:["setInterval"]});
  const f=fixture(t,{deferReplay:true});f.controller.mount(anchor);await flush();
  assert.match(f.document.querySelector('.timeline-video-center').textContent,/正在获取录播/);
  const retry=f.document.querySelector('.timeline-loading-retry');assert.equal(retry.hidden,true);
  now+=16000;t.mock.timers.tick(50);
  assert.match(f.document.querySelector('.timeline-video-center').textContent,/录播加载较慢/);
  assert.equal(retry.hidden,false);
  assert.equal(f.media[0].sources.length,0);
});

test("a playback refresh using the same URL rejects timing from the previous request",async t=>{
  const f=fixture(t,{source:fastReplay});f.controller.mount(anchor);await flush();await flush();
  const first=f.requests.find(request=>request.method==="session_replay_timing");
  f.controller.refreshReplay();await flush();await flush();
  const second=f.requests.filter(request=>request.method==="session_replay_timing").at(-1);
  assert.notEqual(first,second);assert.equal(first.params.play_url,second.params.play_url);
  first.resolve(timing(replay.play_url));await flush();
  assert.equal(f.media[0].options.getClockMetadata(0,120).mode,"elapsed");
  second.resolve(timing(replay.play_url));await flush();
  assert.equal(f.media[0].options.getClockMetadata(0,120).mode,"pdt");
  assert.equal(f.media[0].sources.length,2);
});

test("pending timing hides guessed danmaku and activities, then aligns them without interrupting clip drafts",async t=>{
  t.mock.timers.enable({apis:["setInterval"]});
  const f=fixture(t,{source:fastReplay});f.controller.mount({...anchor,is_live:false});await flush();await flush();
  const player=f.media[0],start=1792735200;
  f.pending.get("session_timeline")({...archive,bucket_seconds:60,buckets:[{offset_seconds:0,count:1}],chat_count:1,
    messages:[{id:"at-zero",message_type:"chat",offset_seconds:0,time_source:"server",content:"不要误放开场弹幕",sender_name:"观众"},{id:"a",message_type:"chat",offset_seconds:30,time_source:"server",content:"这里是弹幕",sender_name:"观众"}],
    activities:[{id:"activity",activity_id:"round-1",kind:"lottery",offset_seconds:30,time_source:"server",epoch:start+30,draw_at:start+60,details:{prize_name:"周边"}}]});
  await flush();
  assert.equal(f.document.querySelector('.timeline-danmaku').textContent,"","archive arrival must not briefly show comments at zero while timing is pending");
  f.controller.input({id:"timeline-media-seek",value:"31"});
  assert.equal(f.document.querySelector('.timeline-danmaku').textContent,"");
  assert.equal(player.element.querySelector('.timeline-activity-track'),null);
  assert.equal(f.document.querySelector('[data-action="timeline-session-download"]').disabled,false);
  await f.controller.action({dataset:{action:"timeline-clip-toggle"}});
  const draft=f.document.querySelector('#timeline-clip-start');draft.focus();draft.value="00:00:";
  f.controller.input(draft);draft.setSelectionRange(5,6);
  const request=f.requests.find(request=>request.method==="session_replay_timing");request.resolve(timing(replay.play_url));await flush();
  assert.equal(f.media.length,1);assert.equal(player.sources.length,1);assert.equal(player.currentTime,31);
  assert.match(f.document.querySelector('.timeline-danmaku').textContent,/这里是弹幕/);
  assert.ok(player.element.querySelector('.timeline-activity-track'));
  assert.equal(f.document.querySelector('#timeline-clip-start').value,"00:00:");
  assert.equal(f.document.activeElement.id,'timeline-clip-start');
  assert.equal(f.document.activeElement.selectionStart,5);assert.equal(f.document.activeElement.selectionEnd,6);
});

test("closing the detail view rejects a delayed timing response",async t=>{
  const f=fixture(t,{source:fastReplay});f.controller.mount(anchor);await flush();await flush();
  const request=f.requests.find(request=>request.method==="session_replay_timing");
  f.controller.unmount();f.document.querySelector('#session-timeline').innerHTML='closed';
  request.resolve(timing(replay.play_url));await flush();
  assert.equal(f.document.querySelector('#session-timeline').innerHTML,'closed');
});
