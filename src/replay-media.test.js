import test from "node:test";
import assert from "node:assert/strict";
import {createReplayMedia} from "./replay-media.js";
function fixture(callbacks={}){
  const video=new EventTarget();Object.assign(video,{currentTime:0,duration:120,paused:true,playbackRate:1,volume:1,muted:true,canPlayType:()=>"",load(){},removeAttribute(){this.src="";},pause(){this.paused=true;this.dispatchEvent(new Event("pause"));},async play(){this.paused=false;this.dispatchEvent(new Event("play"));}});
  const doc=new EventTarget();doc.fullscreenElement=null;doc.exitFullscreen=async()=>{doc.fullscreenElement=null;doc.dispatchEvent(new Event("fullscreenchange"));};
  const element={ownerDocument:doc,querySelector:()=>video,async requestFullscreen(){doc.fullscreenElement=element;doc.dispatchEvent(new Event("fullscreenchange"));}};
  const controller=createReplayMedia({element,...callbacks});return {video,doc,element,controller};
}
function hlsMock(){
  return class Hls{
    static Events={ERROR:"error",MANIFEST_LOADED:"manifest",MANIFEST_PARSED:"parsed",LEVEL_LOADED:"level",FRAG_LOADED:"fragment",BUFFER_APPENDED:"buffer"};
    static ErrorTypes={NETWORK_ERROR:"networkError",MEDIA_ERROR:"mediaError"};
    static isSupported(){return true;}
    constructor(config){this.config=config;this.handlers=new Map();this.stopped=false;this.recoveries=0;Hls.latest=this;}
    on(name,handler){this.handlers.set(name,handler);}
    emit(name,data={}){this.handlers.get(name)?.(name,data);}
    attachMedia(){}loadSource(){}stopLoad(){this.stopped=true;}recoverMediaError(){this.recoveries++;}destroy(){}
  };
}

test("browser media applies remembered audio before loading and keeps it across source changes",async()=>{
  const f=fixture({initialAudio:{volume:.37,muted:false,lastAudibleVolume:.37}});
  assert.equal(f.video.volume,.37);assert.equal(f.video.muted,false);
  await f.controller.setSource({media_kind:"mp4",play_url:"https://media.xingtu.cn/one.mp4"});
  f.controller.muted(true);
  await f.controller.setSource({media_kind:"mp4",play_url:"https://media.xingtu.cn/two.mp4"});
  assert.equal(f.video.volume,.37);assert.equal(f.video.muted,true);f.controller.destroy();
  const zero=fixture({initialAudio:{volume:0,muted:true,lastAudibleVolume:.37}});
  zero.controller.muted(false);
  assert.equal(zero.video.volume,.37);assert.equal(zero.video.muted,false);zero.controller.destroy();
});

test("initial HLS position is configured before fetching and metadata does not claim a playable first frame",async()=>{
  const Hls=hlsMock(),stages=[],frames=[];
  const f=fixture({loadHls:async()=>({default:Hls}),onLoading:value=>stages.push(value),onReady:value=>frames.push(value)});
  await f.controller.setSource({media_kind:"hls",play_url:"https://media.xingtu.cn/replay.m3u8"},{startTime:65});
  assert.equal(Hls.latest.config.startPosition,65);assert.equal(f.video.preload,"auto");
  Hls.latest.emit(Hls.Events.MANIFEST_LOADED);
  f.video.dispatchEvent(new Event("loadedmetadata"));
  assert.equal(f.video.currentTime,65);assert.equal(f.controller.ready,false);assert.equal(frames.length,0);
  f.video.dispatchEvent(new Event("loadeddata"));f.video.dispatchEvent(new Event("canplay"));
  assert.equal(f.controller.ready,true);assert.equal(frames.length,1);assert.deepEqual(stages,["manifest","seeking"]);
  f.controller.destroy();
});

test("startup with no progress stops HLS after 45 seconds and surfaces an actionable error",async t=>{
  t.mock.timers.enable({apis:["setTimeout","Date"],now:0});
  const Hls=hlsMock(),errors=[];
  const f=fixture({loadHls:async()=>({default:Hls}),onError:value=>errors.push(value)});
  await f.controller.setSource({media_kind:"hls",play_url:"https://media.xingtu.cn/replay.m3u8"});
  t.mock.timers.tick(44999);assert.equal(errors.length,0);
  t.mock.timers.tick(1);assert.equal(errors.length,1);assert.match(errors[0],/首帧加载超时/);assert.equal(Hls.latest.stopped,true);
  f.video.dispatchEvent(new Event("loadeddata"));assert.equal(f.controller.ready,false);f.controller.destroy();
});

test("a first frame decoded during the initial seek becomes ready once that seek completes",async()=>{
  const frames=[],f=fixture({onReady:value=>frames.push(value)});
  await f.controller.setSource({media_kind:"mp4",play_url:"https://media.xingtu.cn/replay.mp4"},{startTime:50});
  f.video.dispatchEvent(new Event("loadedmetadata"));
  f.video.readyState=3;f.video.seeking=true;f.video.dispatchEvent(new Event("loadeddata"));
  assert.equal(frames.length,0);
  f.video.seeking=false;f.video.dispatchEvent(new Event("seeked"));
  assert.equal(frames.length,1);assert.equal(f.controller.ready,true);f.controller.destroy();
});

test("genuine loading progress extends the idle deadline but cannot exceed the 90-second first-frame cap",async t=>{
  t.mock.timers.enable({apis:["setTimeout","Date"],now:0});
  const errors=[],f=fixture({onError:value=>errors.push(value)});
  await f.controller.setSource({media_kind:"mp4",play_url:"https://media.xingtu.cn/replay.mp4"});
  t.mock.timers.tick(40000);f.video.dispatchEvent(new Event("progress"));
  t.mock.timers.tick(40000);f.video.dispatchEvent(new Event("progress"));
  assert.equal(errors.length,0);
  t.mock.timers.tick(10000);assert.equal(errors.length,1);f.controller.destroy();
});

test("a decoded first frame and switching sessions both cancel their startup deadlines",async t=>{
  t.mock.timers.enable({apis:["setTimeout","Date"],now:0});
  const errors=[],f=fixture({onError:value=>errors.push(value)});
  await f.controller.setSource({media_kind:"mp4",play_url:"https://media.xingtu.cn/replay.mp4"});
  f.video.dispatchEvent(new Event("loadedmetadata"));f.video.dispatchEvent(new Event("canplay"));
  t.mock.timers.tick(90000);assert.equal(errors.length,0);
  await f.controller.setSource({media_kind:"mp4",play_url:"https://media.xingtu.cn/other.mp4"});
  await f.controller.setSource(null);t.mock.timers.tick(90000);assert.equal(errors.length,0);f.controller.destroy();
});

test("media errors provide safe format and network classifications without browser error messages",async()=>{
  const errors=[],f=fixture({onError:value=>errors.push(value)});
  for(const code of [2,3,4]){
    await f.controller.setSource({media_kind:"mp4",play_url:"https://media.xingtu.cn/replay.mp4"});
    f.video.error={code,message:"signed-url-secret"};f.video.dispatchEvent(new Event("error"));
  }
  assert.match(errors[0],/MEDIA_ERR_NETWORK/);assert.match(errors[1],/MEDIA_ERR_DECODE/);assert.match(errors[2],/MEDIA_ERR_SRC_NOT_SUPPORTED/);
  assert.ok(errors.every(value=>!value.includes("signed-url-secret")));f.controller.destroy();
});

test("HLS error diagnostics whitelist details and HTTP codes, stop unsupported codecs, and refresh expired links",async()=>{
  const Hls=hlsMock(),errors=[],expired=[];
  const f=fixture({loadHls:async()=>({default:Hls}),onError:value=>errors.push(value),onExpired:value=>expired.push(value)});
  const source={media_kind:"hls",play_url:"https://media.xingtu.cn/replay.m3u8"};
  await f.controller.setSource(source);
  Hls.latest.emit(Hls.Events.ERROR,{fatal:true,type:"networkError",details:"manifestLoadError",response:{code:500},reason:"signed-url-secret"});
  assert.match(errors[0],/manifestLoadError.*HTTP 500/);assert.equal(Hls.latest.stopped,true);
  await f.controller.setSource(source);
  Hls.latest.emit(Hls.Events.ERROR,{fatal:true,type:"mediaError",details:"bufferIncompatibleCodecsError",reason:"signed-url-secret"});
  assert.match(errors[1],/不支持.*bufferIncompatibleCodecsError/);assert.equal(Hls.latest.recoveries,0);
  await f.controller.setSource(source);
  Hls.latest.emit(Hls.Events.ERROR,{fatal:false,type:"networkError",details:"unknown-secret-detail",response:{code:403}});
  assert.deepEqual(expired,[{status:403,message:"录播链接已失效（HTTP 403），请重试。"}]);
  assert.ok(errors.every(value=>!value.includes("signed-url-secret")&&!value.includes("unknown-secret-detail")));f.controller.destroy();
});
test("recorded video supplies the clock; play, pause, seek and speed control the same media element",async()=>{
  const times=[],states=[];const f=fixture({onTime:value=>times.push(value),onPlayback:value=>states.push(value)});
  await f.controller.setSource({media_kind:"mp4",play_url:"https://media.xingtu.cn/replay.mp4"});
  f.controller.seek(35);f.video.dispatchEvent(new Event("loadedmetadata"));f.video.dispatchEvent(new Event("loadeddata"));assert.equal(f.video.currentTime,35);
  await f.controller.play();f.controller.speed(2);f.video.currentTime=40;f.video.dispatchEvent(new Event("timeupdate"));f.controller.pause();
  assert.deepEqual(times,[40]);assert.deepEqual(states,[true,false]);assert.equal(f.video.playbackRate,2);
  f.controller.destroy();assert.equal(f.controller.hasMedia,false);
});
test("fullscreen applies to the shared player container so overlays are inside it, then cleanup exits",async()=>{
  const transitions=[];const f=fixture({onFullscreen:value=>transitions.push(value)});
  await f.controller.fullscreen();assert.equal(f.doc.fullscreenElement,f.element);assert.equal(f.controller.isFullscreen,true);
  f.controller.destroy();assert.equal(f.doc.fullscreenElement,null);assert.deepEqual(transitions,[true]);
});

test("volume slider changes the video immediately, unmutes audible levels and restores the last audible level",()=>{
  const changes=[];const f=fixture({onVolume:value=>changes.push(value)});
  assert.equal(f.controller.volume(.4),.4);assert.equal(f.controller.currentMuted,false);
  f.controller.volume(0);assert.equal(f.controller.currentMuted,true);
  f.controller.muted(false);assert.equal(f.controller.currentVolume,.4);assert.equal(f.controller.currentMuted,false);
  f.controller.volume(2);assert.equal(f.controller.currentVolume,1);
  f.controller.volume(NaN);assert.equal(f.controller.currentVolume,1);
  f.video.volume=.2;f.video.muted=true;f.video.dispatchEvent(new Event("volumechange"));
  assert.deepEqual(changes.at(-1),{volume:.2,muted:true});
  f.controller.destroy();
});

test("fullscreen entry protects the pending container and deduplicates concurrent clicks",async()=>{
  const f=fixture();let finish,requests=0;
  f.element.requestFullscreen=()=>{requests++;return new Promise(resolve=>{finish=()=>{f.doc.fullscreenElement=f.element;f.doc.dispatchEvent(new Event("fullscreenchange"));resolve();};});};
  const entering=f.controller.fullscreen();
  assert.equal(f.controller.isFullscreen,true);assert.equal(f.controller.pendingFullscreen,true);
  assert.equal(f.controller.fullscreen(),entering);assert.equal(requests,1);
  finish();await entering;
  assert.equal(f.controller.pendingFullscreen,false);assert.equal(f.controller.isFullscreen,true);
  await f.controller.fullscreen();assert.equal(f.controller.isFullscreen,false);
  f.controller.destroy();
});

test("rejected fullscreen entry clears pending protection and notifies the page to resume painting",async()=>{
  const changes=[];const f=fixture({onFullscreen:value=>changes.push(value)});let reject;
  f.element.requestFullscreen=()=>new Promise((_,fail)=>{reject=fail;});
  const entering=f.controller.fullscreen();assert.equal(f.controller.isFullscreen,true);
  reject(Error("permission rejected"));await assert.rejects(entering,/permission rejected/);
  assert.equal(f.controller.isFullscreen,false);assert.equal(f.controller.pendingFullscreen,false);
  assert.deepEqual(changes,[false]);f.controller.destroy();
});

test("fullscreen completion after closing the player exits without reviving its callbacks",async()=>{
  const changes=[];const f=fixture({onFullscreen:value=>changes.push(value)});let finish;
  f.element.requestFullscreen=()=>new Promise(resolve=>{finish=()=>{f.doc.fullscreenElement=f.element;resolve();};});
  const entering=f.controller.fullscreen();f.controller.destroy();finish();await entering;
  assert.equal(f.doc.fullscreenElement,null);assert.equal(f.controller.isFullscreen,false);assert.deepEqual(changes,[]);
});

test("a marker seek survives a temporary zero MSE duration and applies when duration becomes available",async()=>{
  const f=fixture();
  await f.controller.setSource({media_kind:"mp4",play_url:"https://media.xingtu.cn/replay.mp4"});
  f.video.duration=0;
  f.video.dispatchEvent(new Event("loadedmetadata"));
  assert.equal(f.controller.seek(65),true);
  assert.equal(f.video.currentTime,0);
  f.video.duration=120;
  f.video.dispatchEvent(new Event("durationchange"));
  assert.equal(f.video.currentTime,65);
  f.controller.destroy();
});

test("a temporarily rejected media seek is retried without publishing the stale playhead",async()=>{
  const times=[];
  const f=fixture({onTime:value=>times.push(value)});
  let currentTime=0,readyToSeek=false;
  Object.defineProperty(f.video,"currentTime",{get:()=>currentTime,set:value=>{if(!readyToSeek)throw Error("source buffer not ready");currentTime=value;}});
  await f.controller.setSource({media_kind:"mp4",play_url:"https://media.xingtu.cn/replay.mp4"});
  f.video.dispatchEvent(new Event("loadedmetadata"));
  f.controller.seek(42);
  f.video.dispatchEvent(new Event("timeupdate"));
  assert.deepEqual(times,[]);
  readyToSeek=true;
  f.video.dispatchEvent(new Event("canplay"));
  assert.equal(f.video.currentTime,42);
  f.video.dispatchEvent(new Event("timeupdate"));
  assert.deepEqual(times,[42]);
  f.controller.destroy();
});

test("a silently ignored seek remains pending until a later buffer progress event",async()=>{
  const f=fixture();
  let currentTime=0,readyToSeek=false;
  Object.defineProperty(f.video,"currentTime",{get:()=>currentTime,set:value=>{if(readyToSeek)currentTime=value;}});
  await f.controller.setSource({media_kind:"mp4",play_url:"https://media.xingtu.cn/replay.mp4"});
  f.video.dispatchEvent(new Event("loadedmetadata"));
  f.controller.seek(75);
  assert.equal(f.video.currentTime,0);
  readyToSeek=true;
  f.video.dispatchEvent(new Event("progress"));
  assert.equal(f.video.currentTime,75);
  f.controller.destroy();
});
test("switching away while HLS loads cannot attach or fetch a stale session",async()=>{
  let finish;let created=0;const f=fixture({loadHls:()=>new Promise(resolve=>finish=resolve)});
  const loading=f.controller.setSource({media_kind:"hls",play_url:"https://media.xingtu.cn/replay.m3u8"});
  await f.controller.setSource(null);
  class Hls{constructor(){created++;}static isSupported(){return true;}}
  finish({default:Hls});await loading;assert.equal(created,0);assert.equal(f.controller.hasMedia,false);f.controller.destroy();
});
test("HLS uses the supported MSE engine even when WebView advertises native HLS",async()=>{
  let attached=false;
  class Hls{static Events={ERROR:"error"};static isSupported(){return true;}on(){}attachMedia(){attached=true;}loadSource(){}destroy(){}}
  const f=fixture({loadHls:async()=>({default:Hls})});f.video.canPlayType=()=>"maybe";
  await f.controller.setSource({media_kind:"hls",play_url:"https://media.xingtu.cn/replay.m3u8"});
  assert.equal(attached,true);assert.equal(f.video.src,"");f.controller.destroy();
});
test("non-HTTPS or credential-bearing media addresses are rejected before any fetch",async()=>{
  const errors=[];const f=fixture({onError:value=>errors.push(value)});
  for(const play_url of ["http://media.xingtu.cn/test.mp4","https://secret@media.xingtu.cn/test.mp4"]){await f.controller.setSource({play_url});assert.equal(f.controller.hasMedia,false);}
  assert.equal(errors.length,2);assert.ok(errors.every(error=>!error.includes("secret")));f.controller.destroy();
});
