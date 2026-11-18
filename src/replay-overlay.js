// This transparent child WebView carries DOM controls, never video pixels.
import {listen,emitTo} from "@tauri-apps/api/event";
import {createActivityTooltip} from "./activity-tooltip.js";
import {danmakuProgress,syncDanmakuMotion,isCurrentReplaySnapshot} from "./replay-danmaku-motion.js";
import {playbackClock,clockEpoch} from "./replay-clock.js";
import {setDisplayTimezone} from "./time-display.js";
import "./style.css";
import "./fonts.css";
import "./session-timeline.css";
import "./session-bookmarks.css";
import "./replay-player.css";
import "./selection-policy.css";
import "./replay-overlay.css";

const host=document.querySelector("#native-overlay");
const tooltip=createActivityTooltip();
let state={paused:true,time:0,duration:0},receivedAt=performance.now(),lastHtml="",comments=[],clockText="",clockMetadataKey="",clockRevision=0;
let dragging=false,controlsTimer=null;
let overlayWidth=innerWidth,measureAll=true;
window.addEventListener("resize",()=>{measureAll=true;});
document.fonts?.addEventListener?.("loadingdone",()=>{measureAll=true;});
const send=payload=>void emitTo("main","native-replay-action",payload).catch(()=>{});
function controlIdentity(target){
  const control=target?.closest?.("[data-action],[data-clip-edge],input,select");
  return control?{id:control.id||undefined,dataset:{...control.dataset}}:{};
}
function showControls(){
  host.classList.add("controls-visible");clearTimeout(controlsTimer);
  controlsTimer=setTimeout(()=>host.classList.remove("controls-visible"),2600);
}
const registered=listen("native-replay-state",event=>{
  const value=event.payload;if(!value||typeof value!=="object")return;
  if(!isCurrentReplaySnapshot(value,state))return;
  const previous=state;
  state={...state,...value};receivedAt=performance.now();
  if(Object.hasOwn(value,"worldClock")){
    const key=JSON.stringify(value.worldClock);
    if(key!==clockMetadataKey){clockMetadataKey=key;clockRevision++;if(value.worldClock?.timezone){try{setDisplayTimezone(value.worldClock.timezone);}catch{}}}
  }
  if(typeof value.html==="string"&&value.html!==lastHtml){
    const focus=host.querySelector(":focus"),identity=focus?.id;
    lastHtml=value.html;host.innerHTML=value.html;clockText="";measureAll=true;
    if(identity)host.querySelector(`#${CSS.escape(identity)}`)?.focus({preventScroll:true});
    tooltip.refresh();
  }
  if(typeof value.className==="string"){
    const visible=host.classList.contains("controls-visible");
    host.className=`timeline-player ${value.className}`;
    host.classList.toggle("controls-visible",visible||state.paused);
  }
  const volume=host.querySelector("#timeline-volume");
  if(volume&&document.activeElement!==volume)volume.value=String(state.muted?0:state.volume??1);
  host.classList.toggle("is-paused",Boolean(state.paused));
  if(previous.paused!==state.paused)showControls();
  if(Array.isArray(value.danmaku)){
    const layer=host.querySelector(".timeline-danmaku");
    if(layer){
      const old=new Map(comments.map(item=>[item.key,item]));
      comments=value.danmaku.slice(0,16).map((item,index)=>{
        const key=String(item.key??index),previous=old.get(key);let node=previous?.node;old.delete(key);
        if(!node){node=document.createElement("span");node.className="timeline-danmaku-item";}
        const html=String(item.html||"");if(!previous||previous.html!==html)node.innerHTML=html;
        if(node.parentElement!==layer)layer.append(node);
        node.classList.toggle("static",Boolean(item.static));node.dataset.lane=String(item.lane??index%6);
        node.style.left=item.static?"":"100%";if(item.static)node.style.transform="";
        return {key,node,html,width:previous?.width||0,measure:!previous||previous.html!==html||previous.static!==Boolean(item.static),static:Boolean(item.static),motion:syncDanmakuMotion(previous?.motion,item,{now:receivedAt,paused:state.paused,rate:state.playbackRate,revision:state.danmakuRevision})};
      });
      for(const entry of old.values())entry.node.remove();
    }
  }
});
function animate(now){
  if(measureAll||comments.some(comment=>comment.measure)){
    overlayWidth=host.querySelector(".timeline-danmaku")?.clientWidth||innerWidth;
    for(const comment of comments)if(measureAll||comment.measure){comment.width=comment.node.offsetWidth;comment.measure=false;}
    measureAll=false;
  }
  const elapsed=state.paused?0:Math.min(1,(now-receivedAt)/1000);
  const time=Math.max(0,Math.min(state.duration||Infinity,(state.time||0)+elapsed*(state.playbackRate||1)));
  const key=`${clockRevision}:${state.duration}:${Math.floor(time)}:${Math.floor(clockEpoch(state.worldClock,time)??-1)}`;
  if(key!==clockText){
    clockText=key;const labels=playbackClock(state.worldClock,time,state.duration||0);
    for(const node of host.querySelectorAll(".timeline-position,.timeline-video-time"))node.textContent=labels.position;
    for(const node of host.querySelectorAll(".timeline-duration"))node.textContent=labels.end;
    for(const node of host.querySelectorAll(".timeline-elapsed"))node.textContent=labels.duration;
    for(const node of host.querySelectorAll(".timeline-clock-note")){node.hidden=!labels.note;node.textContent=labels.note;}
  }
  const cursor=host.querySelector(".timeline-cursor");if(cursor)cursor.style.left=`${state.duration>0?time/state.duration*100:0}%`;
  const seek=host.querySelector("#timeline-media-seek");if(seek&&!dragging){seek.value=String(time);seek.max=String(state.duration||1);}
  for(const comment of comments)if(!comment.static){
    const progress=danmakuProgress(comment.motion,now);
    comment.node.style.transform=`translate3d(${-progress*(overlayWidth+comment.width)}px,0,0)`;
  }
  requestAnimationFrame(animate);
}
requestAnimationFrame(animate);
host.addEventListener("click",event=>{const identity=controlIdentity(event.target);if(identity.id||identity.dataset?.action){event.preventDefault();send({kind:"click",...identity});}});
for(const kind of ["input","change"])host.addEventListener(kind,event=>send({kind,...controlIdentity(event.target),value:event.target.value}));
host.addEventListener("pointerdown",event=>{
  showControls();
  const chart=event.target.closest?.(".timeline-chart");
  if(!chart?.classList.contains("is-selecting-clip")&&!event.target.closest?.("[data-clip-edge]"))return;
  if(event.target.closest?.('[data-action="timeline-activity"],[data-action="timeline-highlight"],[data-action="timeline-bookmark"]'))return;
  dragging=true;event.preventDefault();
  send({kind:"pointerdown",...controlIdentity(event.target),x:event.clientX,y:event.clientY,pointerId:event.pointerId,button:event.button});
});
for(const kind of ["pointermove","pointerup","pointercancel"])window.addEventListener(kind,event=>{
  if(!dragging)return;event.preventDefault();
  send({kind,x:event.clientX,y:event.clientY,pointerId:event.pointerId,button:event.button});
  if(kind!=="pointermove")dragging=false;
},{passive:false});
host.addEventListener("keydown",event=>{
  if(event.key==="Escape"){event.preventDefault();send({kind:"click",dataset:{action:"timeline-fullscreen"}});return;}
  if(!event.target.closest?.("[data-clip-edge]"))return;
  event.preventDefault();send({kind:"keydown",...controlIdentity(event.target),key:event.key,shiftKey:event.shiftKey});
});
for(const kind of ["pointermove","focusin","keydown"])host.addEventListener(kind,showControls);
void registered.then(()=>emitTo("main","native-replay-action",{kind:"ready"}));
