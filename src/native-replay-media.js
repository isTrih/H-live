import {invoke as tauriInvoke} from "@tauri-apps/api/core";
import {listen as tauriListen,emitTo as tauriEmitTo} from "@tauri-apps/api/event";
import {normalizePlaybackAudio} from "./playback-preferences.js";

let nativeLifecycle=Promise.resolve();
let replayRevision=0,replayStateSequence=0;
const serialize=operation=>{const task=nativeLifecycle.catch(()=>{}).then(operation);nativeLifecycle=task.catch(()=>{});return task;};
const clamp=(value,min,max)=>Math.min(max,Math.max(min,value));
const safeError=error=>String(error?.message||error||"请重试。").replace(/https?:\/\/\S+/g,"<媒体地址>").replace(/Bearer\s+\S+/gi,"Bearer <已隐藏>").slice(0,500);

export function createNativeReplayMedia({element,initialAudio,getClockMetadata=()=>null,onTime=()=>{},onPlayback=()=>{},onReady=()=>{},onError=()=>{},onExpired=()=>{},onFullscreen=()=>{},onVolume=()=>{},onLoading=()=>{},invoke=tauriInvoke,listen=tauriListen,emitTo=tauriEmitTo,clock=()=>performance.now()}) {
  const doc=element.ownerDocument||globalThis.document,view=doc?.defaultView||globalThis.window;
  const video=element.querySelector("video"),region=()=>element.querySelector(".timeline-video")||element;
  let destroyed=false,source="",generation=0,ready=false,failed=false,opening=Promise.resolve();
  const audio=normalizePlaybackAudio(initialAudio);
  let state={time_pos:0,duration:0,paused:true,volume:audio.volume,muted:audio.muted,fullscreen:false,vo_configured:false},sampledAt=clock(),rate=1,lastAudible=audio.lastAudibleVolume,nativeVolume=audio.volume,startupTimer=null;
  let polling=null,pollingNow=false,pollErrors=0,lastInteraction=clock(),lastPlayback=null,lastVolume="",lastFullscreen=false;
  let boundsFrame=null,lastBounds="",boundsPending=false,boundsAgain=false,dirty=true,lastHTML="",observer=null,resizeObserver=null,unlisten=null;
  let commandChain=Promise.resolve(),commandSerial=0,pendingCommands=0,audioRevision=0,pendingAudioCommands=0;
  let browserFullscreenTask=null,browserFullscreenPending=false;
  let overlayReady=false;
  let motionRevision=0,pendingSeek=null,seekTimer=null;
  const latestCommands=new Map(),cleanups=[];
  const usable=()=>!destroyed&&!failed&&Boolean(source);
  const position=()=>clamp(state.time_pos+(usable()&&ready&&!state.paused?(clock()-sampledAt)/1000*rate:0),0,state.duration||Infinity);
  const bind=(target,name,handler,options)=>{target?.addEventListener?.(name,handler,options);cleanups.push(()=>target?.removeEventListener?.(name,handler,options));};
  function showControls(){if(destroyed)return;lastInteraction=clock();element.classList?.add("controls-visible");}
  function updateSourceVisibility(){element.classList?.toggle("native-replay-host",usable()&&overlayReady);}
  function clearNativeFullscreen(notify=true){
    const active=Boolean(state.fullscreen||lastFullscreen||element.classList?.contains("native-replay-fullscreen"));
    state.fullscreen=false;lastFullscreen=false;element.classList?.remove("native-replay-fullscreen");
    if(active&&notify)onFullscreen(false);
  }
  function clearPendingSeek(){clearTimeout(seekTimer);seekTimer=null;pendingSeek=null;}
  for(const name of ["pointermove","pointerdown","focusin","keydown"])bind(element,name,showControls);

  function mediaBounds(){
    const rect=region().getBoundingClientRect(),viewportWidth=view?.innerWidth||Infinity,viewportHeight=view?.innerHeight||Infinity;
    const modal=state.fullscreen?null:element.closest?.(".modal-body")?.getBoundingClientRect?.();
    const left=Math.max(rect.left,0,modal?.left??-Infinity),top=Math.max(rect.top,0,modal?.top??-Infinity);
    const right=Math.min(rect.right,viewportWidth,modal?.right??Infinity),bottom=Math.min(rect.bottom,viewportHeight,modal?.bottom??Infinity);
    const invisible=doc?.hidden||element.isConnected===false||element.closest?.("[hidden]")||right<=left||bottom<=top;
    const clip=invisible?{x:0,y:0,width:0,height:0}:{x:Math.max(0,Math.round(left-rect.left)),y:Math.max(0,Math.round(top-rect.top)),width:Math.max(0,Math.round(right-left)),height:Math.max(0,Math.round(bottom-top))};
    return {x:Math.round(rect.left),y:Math.round(rect.top),width:invisible?0:Math.max(0,Math.round(rect.width)),height:invisible?0:Math.max(0,Math.round(rect.height)),clip};
  }
  async function updateBounds(){
    if(!usable())return;
    if(boundsPending){boundsAgain=true;return;}
    const bounds=mediaBounds(),key=JSON.stringify(bounds);if(key===lastBounds)return;
    const id=generation;boundsPending=true;
    try{await opening;if(!usable()||id!==generation)return;await invoke("native_replay_bounds",{bounds});if(id===generation)lastBounds=key;}
    catch{/* A following status poll reports a closed native player. */}
    finally{boundsPending=false;if(boundsAgain){boundsAgain=false;scheduleBounds();}}
  }
  function scheduleBounds(){
    if(destroyed||boundsFrame!==null)return;
    if(view?.requestAnimationFrame)boundsFrame=view.requestAnimationFrame(()=>{boundsFrame=null;void updateBounds();});
    else void updateBounds();
  }
  bind(view,"resize",scheduleBounds);bind(view,"scroll",scheduleBounds,{capture:true,passive:true});bind(doc,"visibilitychange",scheduleBounds);
  bind(doc,"fullscreenchange",()=>{if(!destroyed&&!source)onFullscreen(doc.fullscreenElement===element);});
  const Resize=view?.ResizeObserver||globalThis.ResizeObserver;
  if(Resize){resizeObserver=new Resize(scheduleBounds);resizeObserver.observe(region());}
  const Mutation=view?.MutationObserver||globalThis.MutationObserver;
  function changesStructure(record){
    const target=record.target?.nodeType===3?record.target.parentElement:record.target;
    if(!target)return true;
    const attribute=record.attributeName;
    // Reflected attributes can generate records even when the value is equal.
    // The 50ms playhead paint must not invalidate every activity tooltip.
    if(attribute&&typeof target.getAttribute==="function"&&record.oldValue===target.getAttribute(attribute))return false;
    // Outer classes, the playhead and comments have their own compact payload.
    if(target===element&&attribute==="class")return false;
    if(attribute==="style"&&target.matches?.(".timeline-cursor"))return false;
    const danmaku=target.closest?.(".timeline-danmaku");
    if(danmaku&&(target!==danmaku||record.type==="childList"))return false;
    if(!attribute&&target.closest?.(".timeline-position,.timeline-video-time"))return false;
    if(record.type==="childList"){
      const added=[...(record.addedNodes||[])],removed=[...(record.removedNodes||[])];
      // Repeated textContent assignments replace text-node identity, but do not
      // change the child overlay's markup. Preserve actual text/icon changes.
      if(added.length<=1&&removed.length<=1&&added.every(node=>node.nodeType===3)&&removed.every(node=>node.nodeType===3)
        &&(added[0]?.textContent||"")===(removed[0]?.textContent||""))return false;
    }
    return true;
  }
  if(Mutation){observer=new Mutation(records=>{if(records.some(changesStructure))dirty=true;});observer.observe(element,{childList:true,subtree:true,attributes:true,attributeOldValue:true,attributeFilter:["class","hidden","disabled","aria-label","aria-pressed","style"]});}

  function structure(){
    const clone=element.cloneNode(true);
    for(const media of clone.querySelectorAll("video"))media.remove();
    const layer=clone.querySelector(".timeline-danmaku");if(layer)layer.innerHTML="";
    for(const label of clone.querySelectorAll(".timeline-position,.timeline-video-time"))label.textContent="";
    for(const cursor of clone.querySelectorAll(".timeline-cursor"))cursor.style.left="0%";
    for(const input of clone.querySelectorAll("input")){if(input.id.startsWith("timeline-clip-")){const live=element.querySelector(`#${input.id}`);input.setAttribute("value",live?.value||"");}else input.removeAttribute("value");if(input.id==="timeline-media-seek"||input.id==="timeline-volume")input.removeAttribute("aria-valuetext");}
    return clone.innerHTML;
  }
  async function publish(force=false){
    // Transfer visible ownership before sending the child's first loading UI.
    if(!usable()||!overlayReady)return;
    if(clock()-lastInteraction>2600&&element.classList?.contains("controls-visible"))element.classList.remove("controls-visible");
    const className=element.className.replace(/\b(?:timeline-player|native-replay-host)\b/g,"").trim();
    const time=position();
    const payload={time,duration:state.duration,worldClock:getClockMetadata(time,state.duration),paused:state.paused,playbackRate:rate,sequence:++replayStateSequence,danmakuRevision:motionRevision,volume:state.volume,muted:state.muted,fullscreen:state.fullscreen,className,
      danmaku:[...(element.querySelectorAll?.(".timeline-danmaku-item")||[])].map((item,index)=>({key:item.dataset.danmakuKey||`${item.dataset.lane||0}:${item.textContent}:${index}`,html:item.innerHTML,left:item.style.left||"100%",transform:item.style.transform||"",static:item.classList.contains("static"),lane:item.dataset.lane||"0"}))};
    if(force||dirty){const html=structure();if(force||html!==lastHTML)payload.html=html;lastHTML=html;dirty=false;}
    try{await emitTo("replay-overlay","native-replay-state",payload);}catch{/* The child announces readiness after its document loads. */}
  }
  function acceptStatus(response,id=generation,canConfirmSeek=false,acceptAudio=true){
    if(!usable()||id!==generation)return;
    const next=response?.status&&typeof response.status==="object"?response.status:response;
    if(!next||typeof next!=="object")return;
    if(next.error){failMedia(`原生录播播放失败：${safeError(next.error)}`,id);return;}
    const wasReady=ready,previousTime=position();
    let mediaTime=Number.isFinite(next.time_pos)?Math.max(0,next.time_pos):previousTime;
    if(pendingSeek){
      const elapsed=Math.max(0,(clock()-pendingSeek.at)/1000),advance=pendingSeek.paused?0:elapsed*rate;
      const distance=Math.min(Math.abs(mediaTime-pendingSeek.target),Math.abs(mediaTime-pendingSeek.target-advance));
      const oldDistance=Math.abs(mediaTime-pendingSeek.from-advance),jump=Math.abs(pendingSeek.target-pendingSeek.from);
      const tolerance=Math.max(.1,Math.min(.5,jump*.2));
      if(canConfirmSeek&&distance<=tolerance&&(distance+.015<oldDistance||Math.abs(mediaTime-pendingSeek.target)<.03||jump<.25&&elapsed>.5))clearPendingSeek();
      else mediaTime=previousTime;
    }
    if(acceptAudio&&Number.isFinite(next.volume))nativeVolume=clamp(next.volume,0,1);
    state={...state,...next,time_pos:mediaTime,duration:Number.isFinite(next.duration)&&next.duration>0?next.duration:state.duration,volume:acceptAudio?(nativeVolume>0?nativeVolume:lastAudible):state.volume,paused:typeof next.paused==="boolean"?next.paused:state.paused,muted:acceptAudio?(nativeVolume===0||(typeof next.muted==="boolean"?next.muted:state.muted)):state.muted};
    sampledAt=clock();if(state.volume>0)lastAudible=state.volume;
    if(Boolean(state.fullscreen)!==lastFullscreen){lastFullscreen=Boolean(state.fullscreen);element.classList.toggle("native-replay-fullscreen",lastFullscreen);onFullscreen(lastFullscreen);scheduleBounds();dirty=true;}
    const volumeKey=`${state.volume}:${state.muted}`;if(volumeKey!==lastVolume){lastVolume=volumeKey;onVolume({volume:state.volume,muted:state.muted});}
    if(!ready&&next.frame_ready===true){ready=true;clearTimeout(startupTimer);startupTimer=null;onReady({duration:state.duration,hwdec:state.hwdec,video_codec:state.video_codec});}
    if(wasReady&&lastPlayback!==!state.paused){lastPlayback=!state.paused;onPlayback(!state.paused);}
    onTime(position());
  }
  function failMedia(message,id=generation){
    if(destroyed||failed||id!==generation)return;
    failed=true;ready=false;clearTimeout(startupTimer);startupTimer=null;clearInterval(polling);polling=null;
    clearPendingSeek();
    overlayReady=false;updateSourceVisibility();
    clearNativeFullscreen();
    void serialize(()=>invoke("native_replay_close")).catch(()=>{});
    const expired=message.match(/\b(403|410)\b/);
    if(expired)onExpired({status:Number(expired[1]),message:"录播链接已失效，请重试。"});else onError(message);
  }
  async function poll(){
    if(!usable()||pollingNow||pendingCommands)return;
    const id=generation,revision=commandSerial;pollingNow=true;
    try{const response=await invoke("native_replay_status");if(id===generation&&revision===commandSerial){pollErrors=0;acceptStatus(response,id,true);}}
    catch(error){if(usable()&&id===generation&&++pollErrors>=3)failMedia(`原生录播连接失败：${safeError(error)}`,id);}
    finally{pollingNow=false;}
  }
  function queueAction(action,value){
    if(!usable())return Promise.resolve(false);
    const id=generation,serial=++commandSerial,key=["play","pause"].includes(action)?"playback":action,seekTarget=action==="seek"?pendingSeek:null,wantedAudioRevision=audioRevision,isAudio=action==="volume"||action==="mute";
    latestCommands.set(key,serial);pendingCommands++;
    if(isAudio)pendingAudioCommands++;
    const task=commandChain.catch(()=>{}).then(async()=>{
      await opening;
      if(!usable()||id!==generation||latestCommands.get(key)!==serial)return false;
      const response=await invoke("native_replay_action",{action,value});
      if(usable()&&id===generation&&latestCommands.get(key)===serial)acceptStatus(response,id,false,wantedAudioRevision===audioRevision&&pendingAudioCommands===(isAudio?1:0));
      return true;
    }).catch(error=>{if(usable()&&id===generation){if(action==="seek"&&pendingSeek===seekTarget)failMedia(`录播定位失败：${safeError(error)}`,id);else onError(`原生播放操作失败：${safeError(error)}`);}return false;}).finally(()=>{pendingCommands--;if(isAudio)pendingAudioCommands--;});
    commandChain=task;return task;
  }
  function optimistic(values,beforePublish){state={...state,time_pos:position(),...values};sampledAt=clock();beforePublish?.();showControls();void publish();}
  async function setSource(replay,{startTime=0}={}){
    const id=++generation;clearInterval(polling);polling=null;clearTimeout(startupTimer);startupTimer=null;ready=false;failed=false;source="";lastHTML="";dirty=true;lastBounds="";lastPlayback=null;pollErrors=0;
    clearPendingSeek();motionRevision=++replayRevision;
    overlayReady=false;updateSourceVisibility();
    clearNativeFullscreen();
    state={...state,time_pos:Math.max(0,Number(startTime)||0),duration:Number.isFinite(replay?.duration_ms)?Math.max(0,replay.duration_ms/1000):0,paused:true,vo_configured:false,frame_ready:false};sampledAt=clock();
    let url=null;
    if(replay?.play_url){try{url=new URL(replay.play_url);if(url.protocol!=="https:"||url.username||url.password)throw Error();}catch{failed=true;onError("录播地址无效。");}}
    opening=serialize(async()=>{
      if(destroyed||id!==generation)return;
      try{
        await invoke("native_replay_close");
        if(destroyed||id!==generation||!url||failed)return;
        source=url.href;showControls();onLoading("buffering");
        startupTimer=setTimeout(()=>failMedia("原生录播首帧加载超时，请重试。",id),45000);startupTimer?.unref?.();
        const openingAudioRevision=audioRevision;
        const response=await invoke("native_replay_open",{url:source,bounds:mediaBounds(),startTime:state.time_pos,paused:true,volume:state.volume,muted:state.muted});
        if(destroyed||id!==generation||failed)return;
        await subscription;
        acceptStatus(response,id,false,openingAudioRevision===audioRevision&&pendingAudioCommands===0);if(!usable()||id!==generation)return;updateSourceVisibility();lastBounds=JSON.stringify(mediaBounds());void publish(true);
        polling=setInterval(()=>{void poll();void publish();},250);polling?.unref?.();
      }catch(error){failMedia(`原生录播启动失败：${safeError(error)}`,id);}
    });
    await opening;
  }
  function targetFor(payload){
    const candidates=[...(element.querySelectorAll?.("[id],[data-action],[data-clip-edge]")||[])];
    if(payload.id)return candidates.find(item=>item.id===payload.id)||null;
    if(payload.dataset&&Object.keys(payload.dataset).length)return candidates.find(item=>Object.entries(payload.dataset).every(([key,value])=>String(item.dataset[key]??"")===String(value)))||null;
    return null;
  }
  function overlayAction(event){
    if(destroyed)return;
    const payload=event?.payload||event;if(!payload||typeof payload!=="object")return;
    if(payload.kind==="ready"){if(!usable())return;overlayReady=true;updateSourceVisibility();dirty=true;lastHTML="";void publish(true);return;}
    showControls();
    const target=targetFor(payload),EventType=view?.Event||globalThis.Event;
    if(["input","change"].includes(payload.kind)&&target){target.value=String(payload.value??"");target.dispatchEvent(new EventType(payload.kind,{bubbles:true,cancelable:true}));}
    else if(payload.kind==="click"&&target){const Mouse=view?.MouseEvent||globalThis.MouseEvent;target.dispatchEvent(new Mouse("click",{bubbles:true,cancelable:true}));}
    else if(payload.kind==="keydown"&&target){const Key=view?.KeyboardEvent||globalThis.KeyboardEvent;target.dispatchEvent(new Key("keydown",{bubbles:true,cancelable:true,key:payload.key,shiftKey:Boolean(payload.shiftKey)}));}
    else if(["pointerdown","pointermove","pointerup","pointercancel"].includes(payload.kind)){
      const rect=element.getBoundingClientRect(),Pointer=view?.PointerEvent||globalThis.PointerEvent;
      if(!Pointer||!Number.isFinite(payload.x)||!Number.isFinite(payload.y))return;
      const pointer=new Pointer(payload.kind,{bubbles:true,cancelable:true,clientX:rect.left+payload.x,clientY:rect.top+payload.y,pointerId:payload.pointerId||1,button:0,buttons:["pointerup","pointercancel"].includes(payload.kind)?0:1,isPrimary:true,pointerType:"mouse"});
      if(payload.kind==="pointerdown")(target||element.querySelector(".timeline-integrated-progress .timeline-chart"))?.dispatchEvent(pointer);else view?.dispatchEvent(pointer);
    }
    dirty=true;if(payload.kind!=="pointermove")void publish();
  }
  const subscription=Promise.resolve().then(()=>listen("native-replay-action",overlayAction)).then(remove=>{if(destroyed)remove();else unlisten=remove;}).catch(()=>{});
  function seek(time){
    if(!usable()||!Number.isFinite(time)||time<0)return false;
    const value=clamp(time,0,state.duration||Infinity),from=position(),id=generation;clearPendingSeek();
    pendingSeek={target:value,from,at:clock(),paused:state.paused};const target=pendingSeek;motionRevision=++replayRevision;
    seekTimer=setTimeout(()=>{if(pendingSeek===target)failMedia("录播定位超时，请重试。",id);},8000);seekTimer?.unref?.();
    optimistic({time_pos:value},()=>onTime(value));void queueAction("seek",value);return true;
  }
  async function play(){if(!usable())return false;optimistic({paused:false});return queueAction("play",0);}
  function pause(){if(!usable())return;optimistic({paused:true});void queueAction("pause",0);}
  function speed(value){if(!Number.isFinite(value)||value<.25||value>16)return;if(value===rate)return;rate=value;void queueAction("speed",value);}
  function volume(value){if(!destroyed&&Number.isFinite(value)){audioRevision++;const amount=clamp(value,0,1);nativeVolume=amount;if(amount>0)lastAudible=amount;optimistic({volume:amount,muted:amount===0});onVolume({volume:amount,muted:amount===0});void queueAction("volume",amount);void queueAction("mute",amount===0?1:0);}return state.volume;}
  function muted(value){audioRevision++;const mute=Boolean(value);if(!mute&&(nativeVolume===0||state.volume===0)){nativeVolume=lastAudible;state.volume=lastAudible;void queueAction("volume",lastAudible);}optimistic({muted:mute});onVolume({volume:state.volume,muted:mute});void queueAction("mute",mute?1:0);}
  async function fullscreen(){
    if(destroyed)throw Error("播放器已关闭。");
    if(!source){
      if(browserFullscreenTask)return browserFullscreenTask;
      const exiting=doc.fullscreenElement===element;
      if(!exiting&&!element.requestFullscreen)throw Error("当前窗口不支持全屏。");
      browserFullscreenPending=true;showControls();
      try{browserFullscreenTask=Promise.resolve(exiting?doc.exitFullscreen():element.requestFullscreen());await browserFullscreenTask;}
      finally{browserFullscreenPending=false;browserFullscreenTask=null;if(!destroyed)onFullscreen(doc.fullscreenElement===element);}
      return true;
    }
    if(!usable())throw Error("播放器未打开。");
    const next=!state.fullscreen;optimistic({fullscreen:next});lastFullscreen=next;element.classList.toggle("native-replay-fullscreen",next);onFullscreen(next);scheduleBounds();return queueAction("fullscreen",next?1:0);
  }
  function destroy(){if(destroyed)return;destroyed=true;generation++;source="";overlayReady=false;clearPendingSeek();updateSourceVisibility();clearNativeFullscreen(false);clearInterval(polling);clearTimeout(startupTimer);if(boundsFrame!==null)view?.cancelAnimationFrame?.(boundsFrame);observer?.disconnect();resizeObserver?.disconnect();unlisten?.();for(const remove of cleanups)remove();if(doc.fullscreenElement===element)void Promise.resolve(doc.exitFullscreen()).catch(()=>{});void serialize(()=>invoke("native_replay_close")).catch(()=>{});}
  return {element,video,setSource,seek,play,pause,speed,volume,muted,fullscreen,destroy,refreshOverlay:()=>{dirty=true;void publish(true);},get currentTime(){return position();},get duration(){return state.duration;},get paused(){return state.paused;},get hasMedia(){return usable();},get ready(){return ready;},get currentVolume(){return state.volume;},get currentMuted(){return state.muted;},get isFullscreen(){return !destroyed&&Boolean(state.fullscreen||browserFullscreenPending||doc.fullscreenElement===element);},get pendingFullscreen(){return !destroyed&&browserFullscreenPending;}};
}
