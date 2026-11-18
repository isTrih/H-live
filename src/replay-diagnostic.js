// Explicit desktop diagnostic entry. Never loaded by the normal application.
import {invoke} from "@tauri-apps/api/core";
import {listen} from "@tauri-apps/api/event";
import {createReplayMedia} from "./replay-media.js";
import {createNativeReplayMedia} from "./native-replay-media.js";
import {version} from "../package.json";
import timelineStyles from "./session-timeline.css?inline";
import playerStyles from "./replay-player.css?inline";

const report={version,engine:navigator.userAgent,hevc_mse:globalThis.MediaSource?.isTypeSupported('video/mp4; codecs="hvc1.1.6.L93.B0"')||false,h264_mse:globalThis.MediaSource?.isTypeSupported('video/mp4; codecs="avc1.64001f"')||false,stages:[],ipc:[],first_frame:false};
let finished=false,player,danmakuTimer;
const stamp=performance.now();
async function finish(error=""){
  if(finished)return;finished=true;clearTimeout(timeout);clearInterval(danmakuTimer);player?.destroy();
  report.elapsed_ms=Math.round(performance.now()-stamp);if(error)report.error=error;
  await invoke("replay_diagnostic_finish",{report});
}
const timeout=setTimeout(()=>void finish("diagnostic_timeout"),180000);
window.addEventListener("DOMContentLoaded",async()=>{
  try{
    document.body.innerHTML='<main><h1>播放诊断</h1><div id="player"><video playsinline muted></video></div></main>';
    const config=await invoke("replay_diagnostic_config");
    const element=document.querySelector("#player"),video=element.querySelector("video");
    video.style.cssText="width:640px;height:360px;background:#000";
    const record=value=>{report.ipc.push({shape:value?.constructor?.name||typeof value,bytes:value?.byteLength??value?.length??null});return value;};
    if(config.mode==="h264-probe"){
      const bytes=record(await invoke("replay_diagnostic_mp4"));
      const url=URL.createObjectURL(new Blob([bytes],{type:"video/mp4"}));
      video.addEventListener("loadeddata",()=>{report.first_frame=true;report.width=video.videoWidth;report.height=video.videoHeight;URL.revokeObjectURL(url);void finish();},{once:true});
      video.addEventListener("error",()=>void finish(`media_error_${video.error?.code||0}`),{once:true});
      video.src=url;video.preload="auto";video.load();void video.play().catch(()=>{});return;
    }
    player=createReplayMedia({element,
      fetchMedia:async args=>record(await invoke("replay_media_fetch",args)),
      onLoading:stage=>report.stages.push({stage,ms:Math.round(performance.now()-stamp)}),
      onReady:()=>{report.first_frame=true;report.width=video.videoWidth;report.height=video.videoHeight;void finish();},
      onError:error=>void finish(error),onExpired:()=>void finish("expired_http"),
    });
    const response=await invoke("backend_request",{method:"session_replay",params:{session_id:"7691695639532702518"}});
    if(!response?.replay)throw Error("replay_unavailable");
    if(config.mode==="native"){
      player.destroy();player=null;
      await listen("native-replay-action",event=>{
        if(event.payload?.kind==="ready")report.overlay_ready=true;
        else if(event.payload?.kind)report.overlay_control_event=event.payload.kind;
      });
      await listen("replay-diagnostic-overlay",event=>{report.overlay_dom=event.payload;});
      const style=document.createElement("style");style.textContent=timelineStyles+playerStyles+'body{margin:0;background:#e8eae7;font-family:sans-serif}h1{margin:0;height:72px;font-size:20px;padding:12px;box-sizing:border-box}.timeline-player{position:relative;width:640px;height:360px}.timeline-video{position:relative;height:360px}.native-replay-fullscreen{position:fixed!important;inset:0!important;width:100vw!important;height:100vh!important}.native-replay-fullscreen .timeline-video{height:100%!important}';document.head.append(style);
      element.className="timeline-player has-recording controls-visible";
      element.innerHTML='<div class="timeline-video"><video muted></video><div class="timeline-danmaku"><span class="timeline-danmaku-item" data-lane="1" style="left:65%"><b>诊断观众</b><span>GPU画面与DOM弹幕叠层</span></span></div></div><div class="timeline-controls"><button data-action="timeline-play" aria-label="播放暂停">▶</button><span class="timeline-time"><b class="timeline-position">00:00:00</b>/<span class="timeline-duration">00:00:00</span></span><input id="timeline-media-seek" type="range" min="0" max="7200" step="0.1"><input id="timeline-volume" type="range" min="0" max="1" step=".01" value=".4"><button data-action="timeline-mute">静音</button><button data-action="timeline-fullscreen">全屏</button></div>';
      let resolveReady;const firstFrame=new Promise(resolve=>{resolveReady=resolve;});
      // Use the production 12-second playback clock; a fixed CSS left value
      // would feed the child a stale position on every status refresh.
      let danmakuKey="";
      function paintDanmaku(time){
        const layer=element.querySelector(".timeline-danmaku");
        const messages=Array.from({length:4},(_,index)=>Math.floor(time/4)-3+index).filter(index=>index>=0&&time-index*4<=12).map(index=>({id:`probe-${index}`,lane:index%6,progress:(time-index*4)/12,sender_name:`观众${index%4+1}`,content:"一起回看精彩互动"}));
        const key=messages.map(item=>item.id).join("|");
        if(key!==danmakuKey){danmakuKey=key;layer.innerHTML=messages.map(item=>`<span class="timeline-danmaku-item" data-lane="${item.lane}" data-danmaku-key="${item.id}"><b>${item.sender_name}</b><span>${item.content}</span></span>`).join("");}
        layer.querySelectorAll(".timeline-danmaku-item").forEach((node,index)=>{const progress=messages[index].progress;node.style.left=`${(1-progress)*100}%`;node.style.transform=`translateX(-${progress*100}%)`;});
      }
      player=createNativeReplayMedia({element,initialAudio:{volume:1,muted:true},onReady:()=>resolveReady(),onError:error=>void finish(error),onVolume:()=>{},onTime:paintDanmaku});
      danmakuTimer=setInterval(()=>paintDanmaku(player.currentTime),50);
      element.addEventListener("click",event=>{
        const action=event.target.closest("[data-action]")?.dataset.action;
        if(action==="timeline-play"){if(player.paused)void player.play();else player.pause();}
        if(action==="timeline-fullscreen")void player.fullscreen();
        if(action==="timeline-mute")player.muted(!player.currentMuted);
      });
      element.addEventListener("input",event=>{if(event.target.id==="timeline-volume")player.volume(Number(event.target.value));else if(event.target.id==="timeline-media-seek")player.seek(Number(event.target.value));});
      await player.setSource(response.replay,{startTime:0});await firstFrame;
      report.adapter_ready=player.ready;
      await player.play();await new Promise(resolve=>setTimeout(resolve,1200));
      report.native=await invoke("native_replay_status");
      report.screenshot_bytes=await invoke("replay_diagnostic_capture");report.first_frame=report.screenshot_bytes>0;
      // Allow the real child document's ready handshake and first DOM paint.
      const overlayDeadline=performance.now()+5000;
      while(!report.overlay_ready&&performance.now()<overlayDeadline)await new Promise(resolve=>setTimeout(resolve,100));
      if(!report.overlay_ready)throw Error("overlay_ready_timeout");
      report.source_hidden=getComputedStyle(element).visibility==="hidden";
      await new Promise(resolve=>setTimeout(resolve,400));
      const beforeMuted=player.currentMuted;
      await invoke("replay_diagnostic_overlay");
      const controlDeadline=performance.now()+2000;
      while((!report.overlay_dom||!report.overlay_control_event)&&performance.now()<controlDeadline)await new Promise(resolve=>setTimeout(resolve,100));
      report.overlay_control_roundtrip=report.overlay_control_event==="click"&&(await invoke("native_replay_status")).muted!==beforeMuted;
      report.window_screenshot_bytes=await invoke("replay_diagnostic_window");
      player.pause();await new Promise(resolve=>setTimeout(resolve,150));report.pause_ok=(await invoke("native_replay_status")).paused===true;
      player.seek(35);await new Promise(resolve=>setTimeout(resolve,900));report.seek_ok=Math.abs((await invoke("native_replay_status")).time_pos-35)<1;
      player.volume(.4);await new Promise(resolve=>setTimeout(resolve,150));report.volume_ok=Math.abs((await invoke("native_replay_status")).volume-.4)<.01;
      player.muted(true);await new Promise(resolve=>setTimeout(resolve,150));report.mute_ok=(await invoke("native_replay_status")).muted===true;
      if(!player.isFullscreen)await player.fullscreen();await new Promise(resolve=>setTimeout(resolve,150));report.fullscreen_ok=(await invoke("native_replay_status")).fullscreen===true;
      if(player.isFullscreen)await player.fullscreen();await new Promise(resolve=>setTimeout(resolve,100));
      await invoke("native_replay_bounds",{bounds:{x:0,y:72,width:0,height:0}});report.hide_ok=true;
      await invoke("native_replay_bounds",{bounds:{x:0,y:72,width:640,height:360,clip:{x:0,y:40,width:640,height:320}}});report.clip_ok=true;
      await invoke("native_replay_close");report.closed_ok=(await invoke("native_replay_status")).active===false;
      void finish();return;
    }
    await player.setSource(response.replay,{startTime:0});
  }catch(error){
    const allowed=String(error?.message||error||"").replace(/https?:\/\/\S+/g,"<url>").replace(/[\r\n]/g," ");
    report.setup_error=allowed.slice(0,200);void finish("diagnostic_setup_failed");
  }
});
