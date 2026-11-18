// The DOM video survives timeline redraws. Media URLs and credentials are never persisted.
import {createReplayLoader} from "./replay-loader.js";
import {normalizePlaybackAudio} from "./playback-preferences.js";

export function createReplayMedia({element, initialAudio, onTime=()=>{}, onPlayback=()=>{}, onReady=()=>{}, onError=()=>{}, onExpired=()=>{}, onFullscreen=()=>{}, onVolume=()=>{}, onLoading=()=>{}, fetchMedia, loadHls=()=>import("hls.js")}) {
  const video=element.querySelector("video");
  const audio=normalizePlaybackAudio(initialAudio);video.volume=audio.volume;video.muted=audio.muted;
  let hls=null, generation=0, destroyed=false, source="", ready=false, metadataReady=false, pendingTime=null, mediaRecovery=false, startupFailed=false;
  const listeners=[];
  let controlsTimer=null;
  let startupTimer=null, startupStarted=0, startupProgress=0, loadingStage="", lastStartupError="";
  let fullscreenPending=false, fullscreenTask=null, lastFullscreen=false, lastVolumeState="", lastAudibleVolume=audio.lastAudibleVolume;
  const listen=(target,name,handler)=>{target?.addEventListener?.(name,handler);listeners.push(()=>target?.removeEventListener?.(name,handler));};
  function showControls(){
    element.classList?.add("controls-visible");clearTimeout(controlsTimer);
    controlsTimer=setTimeout(()=>element.classList?.remove("controls-visible"),2600);controlsTimer?.unref?.();
  }
  for(const event of ["pointermove","pointerdown","focusin","keydown"])listen(element,event,showControls);
  const usable=()=>!destroyed&&!startupFailed&&Boolean(source);
  const position=()=>Number.isFinite(video.currentTime)?video.currentTime:0;
  function stopStartupWatch(){clearTimeout(startupTimer);startupTimer=null;}
  function loading(stage){if(usable()&&stage!==loadingStage){loadingStage=stage;onLoading(stage);}}
  function failMedia(message){
    if(!usable())return;
    startupFailed=true;stopStartupWatch();
    hls?.stopLoad?.();video.pause();onError(message);
  }
  function watchStartup(id){
    stopStartupWatch();
    if(!usable()||id!==generation||ready)return;
    const now=Date.now(),remaining=Math.min(45000-(now-startupProgress),90000-(now-startupStarted));
    if(remaining<=0){failMedia(`录播首帧加载超时，请重试。${lastStartupError?` ${lastStartupError}`:""}`);return;}
    startupTimer=setTimeout(()=>watchStartup(id),remaining);startupTimer?.unref?.();
  }
  function noteProgress(id=generation){
    if(!usable()||id!==generation||ready)return;
    startupProgress=Date.now();watchStartup(id);
  }
  function applyPendingSeek() {
    if(!usable()||!metadataReady||pendingTime===null)return;
    // MSE can announce metadata before it has a usable duration. Clamping to
    // that temporary zero would discard a marker click as a seek to the start.
    if(Number.isFinite(video.duration)&&video.duration<=0)return;
    const target=Number.isFinite(video.duration)?Math.min(pendingTime,video.duration):pendingTime;
    try{
      video.currentTime=target;
      // Some engines temporarily ignore a seek without throwing while the
      // source buffer initializes. Keep it until a later media event can retry.
      if(Math.abs(position()-target)<.05)pendingTime=null;
    }catch{}
  }
  function seek(time) {
    if(!usable()||!Number.isFinite(time)||time<0)return false;
    pendingTime=time;
    if(!ready)loading("seeking");
    applyPendingSeek();
    return true;
  }
  listen(video,"loadedmetadata",()=>{if(!usable())return;metadataReady=true;noteProgress();applyPendingSeek();});
  for(const event of ["durationchange","loadeddata","canplay","progress"])listen(video,event,applyPendingSeek);
  for(const event of ["loadeddata","canplay","seeked"])listen(video,event,()=>{
    if(!usable()||ready)return;
    noteProgress();applyPendingSeek();
    if(pendingTime!==null||video.seeking||(Number.isFinite(video.readyState)&&video.readyState<2))return;
    ready=true;stopStartupWatch();onReady({duration:video.duration});
  });
  listen(video,"progress",()=>noteProgress());
  const publishTime=()=>{if(usable()&&pendingTime===null)onTime(position());};
  listen(video,"timeupdate",publishTime);
  listen(video,"seeking",publishTime);
  listen(video,"play",()=>{if(usable())onPlayback(true);});
  listen(video,"pause",()=>{if(usable()&&ready)onPlayback(false);});
  listen(video,"ended",()=>{if(usable())onPlayback(false);});
  listen(video,"error",()=>{
    const reasons={2:"录播网络请求失败（MEDIA_ERR_NETWORK），请重试。",3:"当前设备无法解码此录播（MEDIA_ERR_DECODE）。",4:"当前设备不支持此录播格式或编码（MEDIA_ERR_SRC_NOT_SUPPORTED）。"};
    failMedia(reasons[video.error?.code]||"录播播放失败，请重试。");
  });
  function publishFullscreen(force=false){
    if(destroyed)return;
    const active=element.ownerDocument.fullscreenElement===element;
    if(fullscreenPending&&!active&&!force)return;
    if(force||active!==lastFullscreen){lastFullscreen=active;onFullscreen(active);}
  }
  listen(element.ownerDocument,"fullscreenchange",()=>publishFullscreen());
  const currentVolume=()=>Number.isFinite(video.volume)?Math.max(0,Math.min(1,video.volume)):1;
  function publishVolume(){
    if(destroyed)return;
    const value=currentVolume(),isMuted=Boolean(video.muted),key=`${value}:${isMuted}`;
    if(value>0)lastAudibleVolume=value;
    if(key!==lastVolumeState){lastVolumeState=key;onVolume({volume:value,muted:isMuted});}
  }
  listen(video,"volumechange",publishVolume);
  function clear() {
    stopStartupWatch();source="";ready=false;metadataReady=false;pendingTime=null;startupFailed=false;loadingStage="";lastStartupError="";
    if(hls){hls.destroy();hls=null;}
    video.pause();video.removeAttribute("src");video.load();
  }
  async function setSource(replay,{startTime=0}={}) {
    const id=++generation;clear();mediaRecovery=false;
    if(destroyed||!replay?.play_url)return;
    showControls();
    let url;try{url=new URL(replay.play_url);}catch{onError("录播地址无效。");return;}
    if(url.protocol!=="https:"||url.username||url.password){onError("录播地址无效。");return;}
    source=url.href;
    const requestedStart=Number.isFinite(startTime)&&startTime>=0?startTime:0;
    const duration=Number.isFinite(replay.duration_ms)&&replay.duration_ms>0?replay.duration_ms/1000:Infinity;
    const startPosition=Math.min(requestedStart,duration);
    pendingTime=startPosition;video.preload="auto";
    startupStarted=startupProgress=Date.now();watchStartup(id);
    const isHls=replay.media_kind==="hls"||/\.m3u8(?:$|\?)/i.test(source);
    loading(isHls?"manifest":"buffering");
    if(!isHls){video.src=source;video.load();return;}
    try {
      const imported=await loadHls();if(!usable()||id!==generation)return;
      const Hls=imported.default||imported;
      if(!Hls.isSupported()){
        if(video.canPlayType("application/vnd.apple.mpegurl")){loading(startPosition>0?"seeking":"buffering");video.src=source;video.load();return;}
        failMedia("当前系统不支持此录播格式。");return;
      }
      hls=new Hls({enableWorker:true,startPosition,maxBufferLength:30,maxMaxBufferLength:60,backBufferLength:20,...(typeof fetchMedia==="function"?{loader:createReplayLoader(fetchMedia)}:{})});
      const instance=hls;
      for(const name of ["MANIFEST_LOADED","MANIFEST_PARSED","LEVEL_LOADED","FRAG_LOADED","BUFFER_APPENDED"]){
        if(!Hls.Events[name])continue;
        instance.on(Hls.Events[name],()=>{if(!usable()||id!==generation)return;noteProgress(id);if(!ready)loading(startPosition>0?"seeking":"buffering");});
      }
      instance.on(Hls.Events.ERROR,(_event,data)=>{
        if(!usable()||id!==generation)return;
        const rawStatus=data.response?.code||data.response?.status;
        const status=Number.isInteger(rawStatus)&&rawStatus>=100&&rawStatus<=599?rawStatus:null;
        const details={
          manifestLoadError:"录播索引获取失败",manifestLoadTimeOut:"录播索引请求超时",manifestParsingError:"录播索引解析失败",
          manifestIncompatibleCodecsError:"当前设备不支持此录播编码",levelLoadError:"录播播放列表获取失败",levelLoadTimeOut:"录播播放列表请求超时",levelParsingError:"录播播放列表解析失败",
          fragLoadError:"录播片段获取失败",fragLoadTimeOut:"录播片段请求超时",fragParsingError:"录播片段解析失败",fragDecryptError:"录播片段解密失败",
          bufferAddCodecError:"当前设备无法创建此录播的解码器",bufferIncompatibleCodecsError:"当前设备不支持此录播编码",bufferAppendError:"录播片段写入失败",bufferAppendingError:"录播片段写入失败",
        };
        const knownDetail=typeof data.details==="string"&&Object.hasOwn(details,data.details)?data.details:"";
        const message=`${knownDetail?`${details[knownDetail]}（${knownDetail}）`:data.type===Hls.ErrorTypes.NETWORK_ERROR?"无法获取录播，请检查网络后重试":"录播解码失败，请重试"}${status?`（HTTP ${status}）`:""}。`;
        if(!ready)lastStartupError=message;
        if(status===403||status===410){startupFailed=true;stopStartupWatch();instance.stopLoad?.();onExpired({status,message:`录播链接已失效（HTTP ${status}），请重试。`});return;}
        if(!data.fatal)return;
        if(["manifestIncompatibleCodecsError","bufferAddCodecError","bufferIncompatibleCodecsError"].includes(knownDetail)){failMedia(message);return;}
        if(data.type===Hls.ErrorTypes.MEDIA_ERROR&&!mediaRecovery){mediaRecovery=true;instance.recoverMediaError();return;}
        failMedia(message);
      });
      instance.attachMedia(video);instance.loadSource(source);
    }catch{if(!destroyed&&id===generation)failMedia("无法加载录播播放器，请重试。");}
  }
  async function play(){if(!usable())return false;try{await video.play();return true;}catch{if(!destroyed)onPlayback(false);return false;}}
  function pause(){video.pause();}
  function speed(value){if(Number.isFinite(value)&&value>=.25&&value<=16)video.playbackRate=value;}
  function volume(value){
    if(!destroyed&&Number.isFinite(value)){
      video.volume=Math.max(0,Math.min(1,value));
      video.muted=video.volume===0;
      publishVolume();
    }
    return currentVolume();
  }
  function muted(value){
    if(destroyed)return;
    if(!value&&currentVolume()===0)video.volume=lastAudibleVolume;
    video.muted=Boolean(value);publishVolume();
  }
  function fullscreen(){
    if(destroyed)return Promise.reject(Error("播放器已关闭。"));
    if(fullscreenTask)return fullscreenTask;
    const doc=element.ownerDocument;
    const exiting=doc.fullscreenElement===element;
    if(!exiting&&!element.requestFullscreen)return Promise.reject(Error("当前窗口不支持全屏。"));
    // Set synchronously, before requestFullscreen: a polling redraw must keep
    // the element attached while the browser is deciding the fullscreen request.
    fullscreenPending=true;
    showControls();
    let request;
    try{request=exiting?doc.exitFullscreen():element.requestFullscreen();}
    catch(error){fullscreenPending=false;publishFullscreen(true);return Promise.reject(error);}
    fullscreenTask=Promise.resolve(request).finally(()=>{
      fullscreenPending=false;fullscreenTask=null;
      if(destroyed){if(doc.fullscreenElement===element)void doc.exitFullscreen().catch(()=>{});return;}
      publishFullscreen(doc.fullscreenElement!==element);
    });
    return fullscreenTask;
  }
  function destroy(){if(destroyed)return;destroyed=true;generation++;clearTimeout(controlsTimer);for(const remove of listeners)remove();if(element.ownerDocument.fullscreenElement===element)void element.ownerDocument.exitFullscreen().catch(()=>{});clear();}
  return {element,video,setSource,seek,play,pause,speed,volume,muted,fullscreen,destroy,get currentVolume(){return currentVolume();},get currentMuted(){return Boolean(video.muted);},get pendingFullscreen(){return !destroyed&&fullscreenPending;},get hasMedia(){return usable();},get ready(){return ready;},get currentTime(){return position();},get duration(){return Number.isFinite(video.duration)?video.duration:0;},get paused(){return video.paused;},get isFullscreen(){return !destroyed&&(fullscreenPending||element.ownerDocument.fullscreenElement===element);}};
}
