import { esc, icon, num } from "./presentation.js";
import { live } from "./features/monitor/model.js";
import { timelineActivities, activityMarkerLayout, activityMarkerHtml } from "./timeline-activities.js";
import {createReplayMedia} from "./replay-media.js";
import {replayTimeMap} from "./replay-sync.js";
import {replayProgressData} from "./replay-progress.js";
import {normalizeClipRange,clipAroundTime,clipTime,parseClipTime} from "./clip-selection.js";
import {createSessionBookmarks} from "./session-bookmarks.js";
import {formatWorldTime,getDisplayTimezone} from "./time-display.js";
import {archiveClockMetadata,replayClockMetadata,playbackClock,clockAxis} from "./replay-clock.js";
import {playbackPreferences} from "./playback-preferences.js";

const finite = value => typeof value === "number" && Number.isFinite(value);
const clamp = (value, min, max) => Math.min(max, Math.max(min, value));
const pauseIcon = '<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5v14M16 5v14" stroke-width="3"/></svg>';
const bucketLabelFor=data=>data?.bucket_seconds===60||!data?.bucket_seconds?"每分钟消息":`每 ${num(data.bucket_seconds)} 秒消息`;
const clipIcon='<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 8 21 2M9 16l12 6M9 8l12 14M9 16 21 2M10 7a4 4 0 1 1-8 0 4 4 0 0 1 8 0M10 17a4 4 0 1 1-8 0 4 4 0 0 1 8 0"/></svg>';
const copyIcon='<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 9h11v12H9z M5 16H3V3h12v2"/></svg>';

function speakerSummary(overview) {
  const count=value=>finite(value)&&value>=0?num(value):"—";
  const stage=overview?.analysis_state&&overview.analysis_state!=="ready"?({analyzing:"统计中",paused:"已暂停",needs_review:"待确认"}[overview.analysis_state]||"统计中"):"";
  return `<span title="本场有聊天发言的去重账号数，不含身份未知的消息">发言人数 <b>${count(overview?.speakers)}</b></span><span class="timeline-valid-speakers" title="至少有一条有效发言，按账号 ID 去重；排除只有无效发言的账号">有效发言人数 <b>${count(overview?.valid_speakers)}</b></span>${stage?`<small title="当前人数为阶段结果，统计完成后更新">${stage}</small>`:""}`;
}

export function timelineTime(seconds) {
  if (!finite(seconds) || seconds < 0) return "—";
  const value = Math.floor(seconds);
  return [Math.floor(value / 3600), Math.floor(value / 60) % 60, value % 60]
    .map(part => String(part).padStart(2, "0")).join(":");
}

export function timelineClock(epoch) {
  return finite(epoch)?formatWorldTime(epoch):"时间未知";
}

// Missing offsets stay unknown. They must never become a burst at the start of a session.
export function normalizeTimeline(data = {}) {
  const records = Array.isArray(data.messages) ? data.messages : [];
  const messages = records.filter(message => message.message_type === "chat" && message.time_source !== "missing" && finite(message.offset_seconds) && message.offset_seconds >= 0)
    .map((message, index) => ({ ...message, _index: index }))
    .sort((a, b) => a.offset_seconds - b.offset_seconds || a._index - b._index);
  const duration = finite(data.duration_seconds) && data.duration_seconds >= 0 ? data.duration_seconds : null;
  const buckets = (Array.isArray(data.buckets) ? data.buckets : [])
    .filter(bucket => finite(bucket.offset_seconds) && bucket.offset_seconds >= 0 && finite(bucket.count) && bucket.count >= 0)
    .filter(bucket => duration === null || bucket.offset_seconds <= duration)
    .sort((a, b) => a.offset_seconds - b.offset_seconds);
  return {
    ...data, messages, buckets, activities: timelineActivities(data), duration_seconds: duration,
    total_messages: finite(data.total_messages) ? data.total_messages : records.length,
    chat_count: finite(data.chat_count) ? data.chat_count : records.filter(message => message.message_type === "chat").length,
    unknown_chat_times: finite(data.untimed_chat_count) ? data.untimed_chat_count : records.filter(message => message.message_type === "chat" && (message.time_source === "missing" || !finite(message.offset_seconds) || message.offset_seconds < 0)).length,
  };
}

// Step areas keep the height proportional to a bucket's actual message count.
export function timelineDensity(buckets, duration, bucketSeconds = 60, width = 1000, height = 64) {
  const empty = { area: "", line: "", peak: 0 };
  if (!finite(duration) || duration <= 0 || !buckets?.length) return empty;
  const validBuckets = buckets.filter(bucket => finite(bucket.offset_seconds) && finite(bucket.count) && bucket.offset_seconds >= 0 && bucket.offset_seconds <= duration && bucket.count >= 0);
  const peak = validBuckets.reduce((value, bucket) => Math.max(value, bucket.count), 0);
  if (!peak) return empty;
  const span = finite(bucketSeconds) && bucketSeconds > 0 ? bucketSeconds : 60;
  const points = [];
  let lastX = 0;
  const coord = value => Number(value.toFixed(2));
  for (const bucket of validBuckets) {
    const x = coord(bucket.offset_seconds / duration * width);
    const end = coord(Math.min(duration, bucket.offset_seconds + span) / duration * width);
    const y = coord(height - bucket.count / peak * (height - 7));
    if (x > lastX) points.push(`L${lastX},${height}`, `L${x},${height}`);
    points.push(`L${x},${y}`, `L${end},${y}`);
    lastX = end;
  }
  if (!points.length) return empty;
  const line = `M0,${height}${points.join("")}L${lastX},${height}L${width},${height}`;
  return { area: `${line}Z`, line, peak };
}

// A bounded window is enough to animate even very large full-session archives.
export function timelineDanmaku(messages, position, windowSeconds = 12) {
  if (!finite(position) || !messages?.length) return [];
  const lowerBound = offset => {
    let lo = 0, hi = messages.length;
    while (lo < hi) {
      const middle = (lo + hi) >> 1;
      if (messages[middle].offset_seconds < offset) lo = middle + 1;
      else hi = middle;
    }
    return lo;
  };
  const visible = [];
  // Stable four-second groups keep a busy room from continually replacing every
  // visible comment with a new one at the right edge of the frame.
  for (let slot = Math.floor(Math.max(0, position - windowSeconds) / 4); slot <= Math.floor(position / 4); slot++) {
    let index = lowerBound(slot * 4);
    for (let taken = 0; taken < 6 && index < messages.length; index++, taken++) {
      const message = messages[index];
      if (message.offset_seconds >= (slot + 1) * 4 || message.offset_seconds > position) break;
      if (message.offset_seconds < position - windowSeconds) continue;
      visible.push({ ...message, lane: index % 6, progress: clamp((position - message.offset_seconds) / windowSeconds, 0, 1) });
    }
  }
  return visible.slice(-16);
}

export function timelineHighlights(result, duration) {
  return (Array.isArray(result?.highlights) ? result.highlights : [])
    .filter(highlight => finite(highlight.offset_seconds) && highlight.offset_seconds >= 0 && finite(duration) && highlight.offset_seconds <= duration)
    .map(highlight => ({ ...highlight, title: String(highlight.title || "互动亮点"), reason: String(highlight.reason || ""), evidence: Array.isArray(highlight.evidence) ? highlight.evidence : [] }))
    .sort((a, b) => a.offset_seconds - b.offset_seconds);
}

export function createSessionTimeline({ state, bridge, toast = () => {}, onConfigure = () => {},onRender=()=>{},getMediaTransport=()=>null,onCreateClip=null,createMedia=createReplayMedia,audioPreferences=playbackPreferences }) {
  let current = null;
  let frameTimer = null;
  const results = new Map();
  const root = () => document.querySelector("#session-timeline");
  const valid = ctx => current === ctx && state.modal?.kind === "detail" && state.modal.rid === ctx.rid;
  const cacheKey = ctx => `${ctx.anchorId}:${ctx.sessionId}`;
  const isCurrentLive = ctx => ctx.sessionId === ctx.anchor.session_id && live(ctx.anchor);
  const hasDuration = ctx => finite(ctx.data?.duration_seconds) && ctx.data.duration_seconds > 0;
  const highlights = ctx => timelineHighlights(ctx.result, ctx.data?.duration_seconds);
  const canClip = ctx => Boolean(ctx.replay && ctx.mediaReady && ctx.player?.hasMedia && finite(ctx.player.duration) && ctx.player.duration > 0);
  const canDownloadSession = ctx => Boolean(ctx.replay && !ctx.replayLoading && ctx.sessionId);
  const bookmarkContext=()=>current&&valid(current)?{anchorId:current.anchorId,sessionId:current.sessionId,live:isCurrentLive(current),
    startEpoch:current.data?.start_epoch,duration:current.data?.duration_seconds,mediaDuration:current.player?.duration||current.replay?.duration_ms/1000,
    ready:canClip(current),toMedia:offset=>timeMap(current).toMedia(offset)}:null;
  const bookmarks=createSessionBookmarks({bridge,getContext:bookmarkContext,toast,changed:()=>{if(current&&valid(current))renderBookmarks(current);},
    onSeek:epoch=>{
      const ctx=current;if(!ctx||!valid(ctx)||!finite(ctx.data?.start_epoch))return;
      const offset=epoch-ctx.data.start_epoch;
      if(ctx.player?.hasMedia){const target=timeMap(ctx).toMedia(offset);if(!finite(target)||target<0||target>ctx.player.duration){toast("该打点没有对应的录播片段");return;}ctx.player.seek(target);}
      else if(seek(ctx,offset)===false)return;
      ctx.playing=true;if(ctx.player?.hasMedia)void ctx.player.play();paint(ctx,true);
    },
    onSelectClip:clip=>{
      const ctx=current;if(!ctx||!valid(ctx)||!canClip(ctx))return;
      ctx.clipMode=true;ctx.playing=false;ctx.player.pause();ctx.player.seek(clip.start);
      ctx.clipMeta={source:'bookmark',bookmark_id:clip.mark.id,title:clip.mark.title};setClipRange(ctx,clip);paintPlayerControls(ctx);
      root()?.querySelector('.timeline-clip-editor')?.scrollIntoView?.({block:'nearest'});
    },onCreateClip});

  function cancelJob(ctx) {
    clearTimeout(ctx.analysisTimer);
    ctx.analysisTimer = null;
    ctx.analysisRequest++;
    if (ctx.jobId) void Promise.resolve(bridge("highlight_cancel", { job_id: ctx.jobId })).catch(() => {});
    ctx.jobId = "";
    ctx.analyzing = false;
  }

  function unmount() {
    bookmarks.unmount();
    clearInterval(frameTimer);
    frameTimer = null;
    if (current) { current.clipPointerCleanup?.(); current.request++; current.replayRequest++; current.player?.destroy(); cancelJob(current); }
    current = null;
  }

  function mount(anchor) {
    unmount();
    const ctx = current = {
      rid: anchor.rid, anchorId: anchor.anchor_id || "", anchor, sessionId: anchor.session_id || "", explicitSession: false,
      request: 0, loading: false, lastLoad: 0, error: "", data: null, qualityOverview: null, position: 0, playing: true,
      speed: 1, danmaku: true, reducedMotion: globalThis.matchMedia?.("(prefers-reduced-motion: reduce)").matches || false,
      lastFrame: Date.now(), danmakuKey: "", activityKey: "", settings: null, settingsError: "",
      analysisRequest: 0, analysisTimer: null, analyzing: false, analysisError: "", analysisReadError: false,
      jobId: "", result: null, selectedHighlight: -1, selectedActivity: "",
      player:null,replay:null,replayLoading:false,replayError:"",replayRequest:0,replayLoadedFor:"",replayExpiredRetry:false,
      timingRequest:0,timingStartedFor:null,timingLoading:false,timingError:"",timingPatchedFor:null,
      replayCorrection:0,mediaMuted:audioPreferences.read().muted,mediaVolume:audioPreferences.read().volume,mediaReady:false,mediaLoadingStage:"manifest",mediaStartedAt:0,needsRender:false,
      clipMode:false,clipRange:null,clipError:"",clipNotice:"",clipBusy:false,clipDraft:null,clipPointerCleanup:null,
      clipMeta:null,fullDownloadNotice:"",fullDownloadError:false,fullDownloadRequestKey:null,
    };
    bookmarks.select();
    ctx.result = results.get(cacheKey(ctx)) || null;
    try{ctx.replayCorrection=Number(localStorage.getItem(`livescope.replay-offset.${cacheKey(ctx)}`))||0;}catch{}
    render(ctx);
    void load(ctx);
    void loadSettings(ctx);
    void loadReplay(ctx);
    void bookmarks.load();
    frameTimer = setInterval(() => frame(ctx), 50);
    frameTimer?.unref?.();
  }

  function timeMap(ctx) {
    const start=ctx.data?.start_epoch,correction=ctx.replayCorrection;
    if(!ctx.timeMapCache||ctx.timeMapCache.replay!==ctx.replay||ctx.timeMapCache.start!==start||ctx.timeMapCache.correction!==correction)
      ctx.timeMapCache={replay:ctx.replay,start,correction,value:replayTimeMap(ctx.replay,start,correction)};
    return ctx.timeMapCache.value;
  }
  function clockDuration(ctx){return ctx.replay?(ctx.player?.duration||ctx.replay.duration_ms/1000||0):(ctx.data?.duration_seconds||0);}
  function clockMetadata(ctx,time,duration=clockDuration(ctx)){
    return ctx.replay?replayClockMetadata(timeMap(ctx),time,duration):archiveClockMetadata(ctx.data?.start_epoch,duration);
  }
  function clockLabels(ctx){
    const duration=clockDuration(ctx),time=ctx.replay?(ctx.player?.hasMedia?ctx.player.currentTime:0):ctx.position;
    const epoch=ctx.replay?timeMap(ctx).toWorldEpoch(time):(finite(ctx.data?.start_epoch)?ctx.data.start_epoch+time:null);
    const key=`${getDisplayTimezone()}:${Math.floor(time)}:${Math.floor(epoch??-1)}:${duration}:${ctx.data?.start_epoch}`;
    if(ctx.clockCache?.key!==key||ctx.clockCache.map!==timeMap(ctx))
      ctx.clockCache={key,map:timeMap(ctx),value:playbackClock(clockMetadata(ctx,time,duration),time,duration)};
    return ctx.clockCache.value;
  }
  function displayOffset(ctx,time,media=false){
    const epoch=media?timeMap(ctx).toWorldEpoch(time):(finite(ctx.data?.start_epoch)?ctx.data.start_epoch+time:null);
    return finite(epoch)?timelineClock(epoch):timelineTime(time);
  }
  function axisMarkup(ctx,media=false,duration=clockDuration(ctx)){
    const toEpoch=time=>media?timeMap(ctx).toWorldEpoch(time):(finite(ctx.data?.start_epoch)&&ctx.data.start_epoch>0?ctx.data.start_epoch+time:null);
    const points=clockAxis({duration,toEpoch}),dates=points.some(point=>point.label.includes("/"));
    return `<div class="timeline-axis ${media?"timeline-media-axis":""} ${dates?"has-dates":""}" aria-label="${esc(`时间轴刻度 · ${getDisplayTimezone()}`)}">${points.map(point=>`<span data-offset="${point.offset??""}" title="${esc(point.aligned?formatWorldTime(point.epoch,{date:true}):"时间未对齐")}">${esc(point.label)}</span>`).join("")}</div>`;
  }
  function syncMediaTime(ctx,time) {
    if(!valid(ctx))return;
    const mapped=timeMap(ctx).toArchive(time);
    ctx.position=mapped===null?-1:mapped;
    paint(ctx);
  }
  async function loadReplay(ctx,force=false) {
    if(!valid(ctx)||!ctx.sessionId||(!force&&ctx.replayLoadedFor===cacheKey(ctx)))return;
    const request=++ctx.replayRequest;
    ctx.replayLoadedFor=cacheKey(ctx);ctx.replayLoading=true;ctx.replayError="";
    ctx.timingRequest++;ctx.timingStartedFor=null;ctx.timingLoading=false;ctx.timingError="";ctx.timingPatchedFor=null;
    ctx.mediaStartedAt=Date.now();ctx.mediaLoadingStage="lookup";
    paintPlayerControls(ctx);
    const previousTime=force&&ctx.player?.hasMedia?ctx.player.currentTime:null;
    try {
      const response=await bridge("session_replay",{session_id:ctx.sessionId});
      if(!valid(ctx)||request!==ctx.replayRequest)return;
      ctx.replay=response?.enabled&&response.available&&response.replay?.play_url?response.replay:null;
      ctx.mediaReady=false;
      if(!ctx.replay)ctx.mediaStartedAt=0;ctx.mediaLoadingStage="manifest";
      ctx.replayLoading=false;
      render(ctx);
      if(ctx.player){
        await ctx.player.setSource(ctx.replay,{startTime:previousTime??0});
        if(!valid(ctx)||request!==ctx.replayRequest)return;
        ctx.player.speed(ctx.speed);
        if(ctx.replay&&ctx.playing)void ctx.player.play();
      }
    } catch(error) {
      if(!valid(ctx)||request!==ctx.replayRequest)return;
      ctx.replay=null;ctx.replayLoading=false;ctx.replayError=String(error?.message||error);render(ctx);
    }
    if(valid(ctx))render(ctx);
  }

  function timingStatus(ctx) {
    if(!ctx.replay?.time_mapping_pending)return "";
    return `<p class="timeline-time-note timeline-timing-status" role="status">${ctx.timingError?'时间轴暂未对齐 <button class="small" data-action="timeline-timing-refresh">重试</button>':"正在对齐时间轴…"}</p>`;
  }

  async function loadReplayTiming(ctx,force=false) {
    if(!valid(ctx)||!ctx.mediaReady||ctx.replayLoading||!ctx.replay?.time_mapping_pending||ctx.timingLoading)return;
    if(!force&&ctx.timingStartedFor===ctx.replayRequest)return;
    const request=++ctx.timingRequest,playback=ctx.replayRequest,sessionId=ctx.sessionId,url=ctx.replay.play_url;
    const matching=()=>valid(ctx)&&ctx.timingRequest===request&&ctx.replayRequest===playback&&ctx.sessionId===sessionId&&ctx.replay?.play_url===url;
    ctx.timingStartedFor=playback;ctx.timingLoading=true;ctx.timingError="";
    try {
      const result=await bridge("session_replay_timing",{session_id:sessionId,play_url:url});
      if(!matching())return;
      if(result?.play_url!==url||result.time_mapping_pending!==false||!Array.isArray(result.time_mapping)
        ||result.time_mapping.some(span=>!span||!finite(span.offset_seconds)||!finite(span.duration)||span.duration<=0
          ||!(finite(span.program_date_time)||typeof span.program_date_time==="string"&&finite(Date.parse(span.program_date_time)))))
        throw Error("时间映射与当前录播不一致");
      ctx.replay={...ctx.replay,time_mapping:result.time_mapping,time_mapping_pending:false,
        playlist_duration_ms:result.playlist_duration_ms,
        ...(!finite(ctx.replay.duration_ms)&&finite(result.playlist_duration_ms)?{duration_ms:result.playlist_duration_ms}:{})};
      ctx.clockCache=null;ctx.timingLoading=false;ctx.timingError="";
      if(ctx.player?.isFullscreen)ctx.timingPatchedFor={playback,url};
      syncMediaTime(ctx,ctx.player?.currentTime||0);
      patchTimingProgress(ctx);
      render(ctx,{preservePlayerControls:true});
      ctx.player?.refreshOverlay?.();
    } catch(error) {
      if(!matching())return;
      ctx.timingError=String(error?.message||error);ctx.timingLoading=false;
      render(ctx,{preservePlayerControls:true});
    }
  }

  function attachPlayer(ctx,element,preserved,inPlace=false,preserveControls=false) {
    const fresh=element.querySelector(".timeline-player");
    if(!fresh)return;
    if(preserved){
      if(!preserveControls){
      const controls=fresh.querySelector(".timeline-controls");
      const old=preserved.querySelector(".timeline-controls");
      if(controls&&old)old.innerHTML=controls.innerHTML;
      const center=preserved.querySelector(".timeline-video-center");
      if(center){center.innerHTML=fresh.querySelector(".timeline-video-center")?.innerHTML||"";center.hidden=Boolean(ctx.mediaReady&&!ctx.replayError);}
      }
      if(!inPlace)fresh.replaceWith(preserved);
      return;
    }
    ctx.player=createMedia({element:fresh,initialAudio:audioPreferences.read(),fetchMedia:getMediaTransport(),getClockMetadata:(time,duration)=>clockMetadata(ctx,time,duration),
      onTime:time=>syncMediaTime(ctx,time),
      onPlayback:playing=>{if(valid(ctx)){ctx.playing=playing;paintPlayerControls(ctx);}},
      onReady:()=>{if(!valid(ctx))return;ctx.mediaReady=true;ctx.replayError="";render(ctx);if(ctx.playing)void ctx.player.play();void loadReplayTiming(ctx);},
      onError:error=>{if(valid(ctx)){ctx.replayError=error;ctx.mediaReady=false;ctx.playing=false;render(ctx);}},
      onExpired:details=>{if(!valid(ctx))return;if(ctx.replayExpiredRetry){ctx.replayError=details?.message||"录播链接已失效，请重试。";render(ctx);return;}ctx.replayExpiredRetry=true;void loadReplay(ctx,true);},
      onFullscreen:fullscreen=>{if(!valid(ctx))return;paintPlayerControls(ctx);if(!fullscreen&&ctx.needsRender){ctx.needsRender=false;const preserve=ctx.timingPatchedFor?.playback===ctx.replayRequest&&ctx.timingPatchedFor?.url===ctx.replay?.play_url;ctx.timingPatchedFor=null;if(preserve)patchTimingProgress(ctx);render(ctx,{preservePlayerControls:preserve});}},
      onVolume:({volume,muted})=>{if(valid(ctx)){ctx.mediaVolume=volume;ctx.mediaMuted=muted;paintPlayerControls(ctx);}},
      onLoading:stage=>{if(valid(ctx)){ctx.mediaLoadingStage=stage;paintPlayerControls(ctx);}},
    });
    // Handle media controls in the original user gesture, before asynchronous
    // page-wide actions, so fullscreen and audio retain browser activation.
    fresh.addEventListener("click",event=>{const control=event.target.closest?.('[data-action^="timeline-"]');if(control){event.stopPropagation();void action(control);}});
    fresh.addEventListener("input",event=>{event.stopPropagation();input(event.target);});
    fresh.addEventListener("change",event=>{event.stopPropagation();change(event.target);});
    attachClipSelection(ctx,fresh);
  }

  function paintPlayerControls(ctx) {
    const player=ctx.player;
    if(!player){paintClip(ctx);return;}
    const parent=player.element;
    parent.classList.toggle("has-recording",Boolean(ctx.replay));
    parent.classList.toggle("is-paused",!ctx.playing);
    const play=parent.querySelector('[data-action="timeline-play"]');
    const setIcon=(button,key,markup)=>{if(button.dataset.timelineIcon!==key){button.innerHTML=markup;button.dataset.timelineIcon=key;}};
    if(play){play.setAttribute("aria-label",ctx.playing?"暂停回放":"播放回放");play.title=play.getAttribute("aria-label");setIcon(play,ctx.playing?"pause":"play",ctx.playing?pauseIcon:icon("play"));play.disabled=!hasDuration(ctx)&&!player.hasMedia;}
    const center=parent.querySelector(".timeline-video-center");
    if(center){
      center.hidden=Boolean(ctx.mediaReady&&!ctx.replayError);
      const slow=Boolean((ctx.replayLoading||ctx.replay)&&!ctx.mediaReady&&ctx.mediaStartedAt&&Date.now()-ctx.mediaStartedAt>=15000);
      const label=center.querySelector("span");
      if(label)label.textContent=ctx.replayError?"录播播放失败":slow?"录播加载较慢，可重试":ctx.replayLoading?"正在获取录播…":ctx.replay&&!ctx.mediaReady?({manifest:"正在读取播放清单…",buffering:"正在缓冲画面…",seeking:"正在定位画面…"}[ctx.mediaLoadingStage]||"正在加载录播…"):"暂无录播回放";
      const retry=center.querySelector(".timeline-loading-retry");if(retry)retry.hidden=!slow||Boolean(ctx.replayError);
    }
    const labels=clockLabels(ctx);
    const duration=parent.querySelector(".timeline-duration");if(duration&&duration.textContent!==labels.end)duration.textContent=labels.end;
    const elapsed=parent.querySelector(".timeline-elapsed");if(elapsed&&elapsed.textContent!==labels.duration)elapsed.textContent=labels.duration;
    const note=parent.querySelector(".timeline-clock-note");if(note){note.hidden=!labels.note;if(note.textContent!==labels.note)note.textContent=labels.note;}
    const mediaSeek=parent.querySelector("#timeline-media-seek");if(mediaSeek){mediaSeek.hidden=!player.hasMedia;mediaSeek.max=String(player.duration||1);mediaSeek.value=String(player.currentTime);mediaSeek.setAttribute("aria-valuetext",labels.position);}
    const fullscreen=parent.querySelector('[data-action="timeline-fullscreen"]');if(fullscreen){setIcon(fullscreen,player.isFullscreen?"shrink":"fullscreen",icon(player.isFullscreen?"shrink":"fullscreen"));fullscreen.setAttribute("aria-label",player.isFullscreen?"退出全屏播放":"全屏播放（含弹幕）");fullscreen.title=fullscreen.getAttribute("aria-label");}
    const mute=parent.querySelector('[data-action="timeline-mute"]');if(mute){setIcon(mute,ctx.mediaMuted?"muted":"volume",icon(ctx.mediaMuted?"muted":"volume"));mute.setAttribute("aria-label",ctx.mediaMuted?"开启声音":"关闭声音");mute.title=mute.getAttribute("aria-label");}
    const volume=parent.querySelector("#timeline-volume");if(volume&&document.activeElement!==volume)volume.value=String(ctx.mediaMuted?0:ctx.mediaVolume);
    paintClip(ctx);
  }

  function setClipRange(ctx,range) {
    ctx.clipRange=range;ctx.clipDraft=null;ctx.clipError="";ctx.clipNotice="";paintClip(ctx);
  }
  function paintClip(ctx) {
    const element=root();if(!element)return;
    const available=canClip(ctx),duration=ctx.player?.duration||0;
    const range=ctx.clipRange&&normalizeClipRange(ctx.clipRange.start,ctx.clipRange.end,duration);
    const editor=element.querySelector(".timeline-clip-editor");if(editor)editor.hidden=!ctx.clipMode;
    const chart=element.querySelector(".timeline-integrated-progress .timeline-chart");chart?.classList.toggle("is-selecting-clip",ctx.clipMode&&available);
    const selection=element.querySelector(".timeline-clip-selection");
    if(selection){selection.hidden=!range||!ctx.clipMode;if(range){selection.style.left=`${range.start/duration*100}%`;selection.style.width=`${(range.end-range.start)/duration*100}%`;}}
    for(const edge of ["start","end"]){
      const handle=element.querySelector(`[data-clip-edge="${edge}"]`);
      if(handle){handle.hidden=!range||!ctx.clipMode;handle.disabled=!available||ctx.clipBusy;if(range){handle.style.left=`${range[edge]/duration*100}%`;handle.setAttribute("aria-valuemax",String(duration));handle.setAttribute("aria-valuenow",String(range[edge]));handle.setAttribute("aria-valuetext",clipTime(range[edge]));}}
      const input=element.querySelector(`#timeline-clip-${edge}`);
      if(input){input.disabled=!available||ctx.clipBusy;if(document.activeElement!==input)input.value=ctx.clipDraft?.[edge]??(range?clipTime(range[edge]):"");}
    }
    for(const button of element.querySelectorAll?.('[data-action="timeline-clip-toggle"],[data-action="timeline-highlight-clip"],[data-action="timeline-clip-start"],[data-action="timeline-clip-end"]')||[]){button.disabled=!available||ctx.clipBusy;if(button.dataset.action==="timeline-clip-toggle"){button.setAttribute("aria-pressed",String(ctx.clipMode));button.classList.toggle("active",ctx.clipMode);}}
    for(const button of element.querySelectorAll?.('[data-action="timeline-clip-download"]')||[])button.disabled=!available||!range||Boolean(ctx.clipError)||ctx.clipBusy;
    for(const button of element.querySelectorAll?.('[data-action="timeline-session-download"]')||[])button.disabled=!canDownloadSession(ctx)||ctx.clipBusy;
    const downloadNotice=element.querySelector('.timeline-session-download-notice');if(downloadNotice){downloadNotice.textContent=ctx.fullDownloadNotice||"";downloadNotice.classList.toggle('error',Boolean(ctx.fullDownloadError));}
    const length=element.querySelector(".timeline-clip-length");if(length)length.textContent=range?`时长 ${clipTime(range.end-range.start)}`:"拖动进度条选择片段";
    const message=element.querySelector(".timeline-clip-message");if(message){message.textContent=ctx.clipBusy?"正在创建下载任务…":ctx.clipError||ctx.clipNotice;message.classList.toggle("error",Boolean(ctx.clipError));}
    const hint=element.querySelector(".timeline-clip-hint");if(hint)hint.textContent=available?(ctx.clipMode?"拖动进度条框选，或调整起止时间。":"选择片段，下载 MP4。"):(ctx.replayLoading||ctx.replay&&!ctx.replayError?"录像加载完成后可切片。":"没有可播放录像，暂时无法切片。");
  }
  function attachClipSelection(ctx,element) {
    element.addEventListener("pointerdown",event=>{
      if(!canClip(ctx)||ctx.clipBusy||event.button!==0)return;
      const handle=event.target.closest?.("[data-clip-edge]");
      if(!handle&&!ctx.clipMode||!handle&&event.target.closest?.('[data-action="timeline-activity"],[data-action="timeline-highlight"],[data-action="timeline-bookmark"]'))return;
      const chart=event.target.closest?.(".timeline-integrated-progress .timeline-chart");
      if(!chart)return;
      const rect=chart.getBoundingClientRect();if(!rect.width)return;
      const at=x=>clamp((x-rect.left)/rect.width,0,1)*ctx.player.duration;
      const anchor=at(event.clientX),original=ctx.clipRange;
      event.preventDefault();event.stopPropagation();ctx.clipPointerCleanup?.();ctx.clipMode=true;ctx.playing=false;ctx.player.pause();
      const update=position=>{
        if(!valid(ctx))return;
        if(handle&&original){const range=handle.dataset.clipEdge==="start"?{start:clamp(position,0,original.end-.1),end:original.end}:{start:original.start,end:clamp(position,original.start+.1,ctx.player.duration)};setClipRange(ctx,normalizeClipRange(range.start,range.end,ctx.player.duration));}
        else setClipRange(ctx,normalizeClipRange(anchor,position,ctx.player.duration));
      };
      const view=element.ownerDocument?.defaultView||globalThis.window;
      const move=e=>{if(e.pointerId!==event.pointerId)return;e.preventDefault();update(at(e.clientX));};
      const end=e=>{if(e.pointerId!==event.pointerId)return;update(at(e.clientX));ctx.clipPointerCleanup?.();};
      ctx.clipPointerCleanup=()=>{view?.removeEventListener("pointermove",move);view?.removeEventListener("pointerup",end);view?.removeEventListener("pointercancel",end);ctx.clipPointerCleanup=null;};
      view?.addEventListener("pointermove",move,{passive:false});view?.addEventListener("pointerup",end);view?.addEventListener("pointercancel",end);
      paintPlayerControls(ctx);
    },true);
    element.addEventListener("keydown",event=>{
      const handle=event.target.closest?.("[data-clip-edge]");if(!handle||!canClip(ctx)||!ctx.clipRange)return;
      const step=event.shiftKey?10:1,edge=handle.dataset.clipEdge,range={...ctx.clipRange};
      if(["ArrowLeft","ArrowDown"].includes(event.key))range[edge]-=step;else if(["ArrowRight","ArrowUp"].includes(event.key))range[edge]+=step;else if(event.key==="Home")range[edge]=0;else if(event.key==="End")range[edge]=ctx.player.duration;else return;
      event.preventDefault();range[edge]=edge==="start"?clamp(range[edge],0,range.end-.1):clamp(range[edge],range.start+.1,ctx.player.duration);setClipRange(ctx,normalizeClipRange(range.start,range.end,ctx.player.duration));
    });
  }
  async function createClip(ctx,range,title,metadata=null) {
    if(ctx.clipBusy)return;
    if(!canClip(ctx)){toast("没有可播放录像，暂时无法切片。",true);return;}
    range=range&&normalizeClipRange(range.start,range.end,ctx.player.duration);
    if(!range){ctx.clipError="请先选择有效的起止时间。";paintClip(ctx);return;}
    if(typeof onCreateClip!=="function"){toast("切片下载暂时不可用",true);return;}
    ctx.clipBusy=true;ctx.clipError="";ctx.clipNotice="";ctx.clipMode=true;ctx.clipRange=range;ctx.clipDraft=null;paintClip(ctx);
    try {
      const source=metadata||ctx.clipMeta||{source:'manual'};
      const result=await onCreateClip({...source,session_id:ctx.sessionId,anchor_id:ctx.anchorId,start_seconds:range.start,end_seconds:range.end,title:title||source.title||`${ctx.anchor.name||"直播"} · ${clipTime(range.start)}—${clipTime(range.end)}`});
      if(valid(ctx))ctx.clipNotice=result===null||result===false?"":"已加入下载任务";
    }catch(error){if(valid(ctx))ctx.clipError=String(error?.message||error);}
    finally{if(valid(ctx)){ctx.clipBusy=false;paintClip(ctx);}}
  }
  async function downloadSession(ctx) {
    if(ctx.clipBusy)return;
    if(!canDownloadSession(ctx)){toast("暂无可下载录播，请等待录像同步完成。",true);return;}
    if(typeof onCreateClip!=="function"){toast("录像下载暂时不可用",true);return;}
    ctx.fullDownloadRequestKey??=`full:${ctx.anchorId}:${ctx.sessionId}:${Date.now()}`;
    const sessionId=ctx.sessionId,requestKey=ctx.fullDownloadRequestKey;
    const matching=()=>valid(ctx)&&ctx.sessionId===sessionId&&ctx.fullDownloadRequestKey===requestKey;
    let succeeded=false;
    ctx.clipBusy=true;ctx.fullDownloadNotice="正在创建整场下载任务…";ctx.fullDownloadError=false;paintClip(ctx);
    try {
      const result=await onCreateClip({mode:"full_session",source:"full_session",session_id:ctx.sessionId,anchor_id:ctx.anchorId,
        title:`${ctx.anchor.name||"直播"} · 整场直播`,request_key:ctx.fullDownloadRequestKey});
      if(matching()){ctx.fullDownloadNotice=result===null||result===false?"":"整场录像已加入下载任务";succeeded=result!==null&&result!==false;}
    }catch(error){if(matching()){ctx.fullDownloadError=true;ctx.fullDownloadNotice=String(error?.message||error);}}
    finally{if(matching()){ctx.clipBusy=false;if(succeeded)ctx.fullDownloadRequestKey=null;paintClip(ctx);}}
  }

  async function loadSettings(ctx) {
    try {
      const settings = await bridge("highlight_settings", {});
      if (!valid(ctx)) return;
      ctx.settings = settings;
      ctx.settingsError = "";
    } catch (error) {
      if (!valid(ctx)) return;
      ctx.settingsError = String(error?.message || error);
    }
    if (valid(ctx)) render(ctx);
  }

  async function load(ctx) {
    if (!valid(ctx) || ctx.loading || !ctx.anchorId || !ctx.sessionId) return;
    const request = ++ctx.request;
    ctx.loading = true;
    ctx.lastLoad = Date.now();
    try {
      const data = await bridge("session_timeline", { anchor_id: ctx.anchorId, session_id: ctx.sessionId });
      if (!valid(ctx) || ctx.request !== request) return;
      const initial = !ctx.data;
      ctx.data = normalizeTimeline(data);
      ctx.error = "";
      const duration = ctx.data.duration_seconds || 0;
      if (initial) {
        ctx.position = isCurrentLive(ctx) ? Math.max(0, duration - 24) : 0;
        ctx.lastFrame = Date.now();
      } else {
        ctx.position = Math.min(ctx.position, duration);
      }
    } catch (error) {
      if (valid(ctx) && ctx.request === request) ctx.error = String(error?.message || error);
    } finally {
      if (valid(ctx) && ctx.request === request) { ctx.loading = false; render(ctx); }
    }
  }

  function preserveScroll(element){
    const nodes=new Set([element,...(element.querySelectorAll?.('.bookmark-list,textarea')||[])]);
    for(let parent=element.parentElement;parent;parent=parent.parentElement)nodes.add(parent);
    const positions=[...nodes].filter(node=>typeof node.scrollTop==='number')
      .map(node=>({node,top:node.scrollTop,left:node.scrollLeft}));
    return ()=>{for(const {node,top,left} of positions)if(node.isConnected!==false){
      if(node.scrollTop!==top)node.scrollTop=top;
      if(typeof left==='number'&&node.scrollLeft!==left)node.scrollLeft=left;
    }};
  }
  function renderBookmarks(ctx){
    const element=root(),section=element?.querySelector('.session-bookmarks');
    if(!element||!valid(ctx))return;
    if(!section||!document.createElement||ctx.renderKey!==cacheKey(ctx)){render(ctx);return;}
    const restoreScroll=preserveScroll(element),template=document.createElement('template');
    template.innerHTML=bookmarks.markup();
    const fresh=template.content.firstElementChild;
    if(!fresh){render(ctx);return;}
    bookmarks.patchView(section,fresh);
    bookmarks.bindView(section);
    for(const layer of element.querySelectorAll('.timeline-bookmark-markers'))
      layer.innerHTML=bookmarks.markers(Boolean(layer.closest('.timeline-integrated-progress')));
    restoreScroll();ctx.player?.refreshOverlay?.();onRender();
  }
  function integratedProgressMarkup(ctx) {
    const element=root(),data=ctx.data,duration=data?.duration_seconds||0,items=highlights(ctx);
    const mediaDuration=ctx.player?.duration||ctx.replay?.duration_ms/1000||duration||0;
    const mediaProgress=replayProgressData(data,items,timeMap(ctx),mediaDuration);
    const mediaChart=timelineDensity(mediaProgress.buckets,mediaDuration,data?.bucket_seconds);
    const mediaActivities=activityMarkerLayout(mediaProgress.activities,mediaDuration,element.clientWidth);
    const clipOverlay=`<div class="timeline-clip-selection" hidden aria-hidden="true"></div>${["start","end"].map(edge=>`<button class="timeline-clip-handle" data-clip-edge="${edge}" role="slider" aria-label="切片${edge==="start"?"起点":"终点"}" aria-valuemin="0" aria-valuemax="${mediaDuration}" aria-valuenow="0" hidden></button>`).join("")}`;
    return ctx.replay?`<div class="timeline-integrated-progress" aria-label="录播互动进度条"><div class="timeline-chart"><svg class="timeline-area" viewBox="0 0 1000 64" preserveAspectRatio="none" role="img" aria-label="${esc(bucketLabelFor(data))}分布，峰值 ${num(mediaChart.peak)} 条"><defs><linearGradient id="timeline-media-density-fill" x1="0" x2="0" y1="0" y2="1"><stop offset="0%" stop-color="#bad6a8" stop-opacity=".8"/><stop offset="100%" stop-color="#bad6a8" stop-opacity=".08"/></linearGradient></defs><path class="timeline-density-area" style="fill:url(#timeline-media-density-fill)" d="${mediaChart.area}"/><path class="timeline-density-line" d="${mediaChart.line}"/></svg><div class="timeline-cursor" aria-hidden="true"><i></i></div>${clipOverlay}<input id="timeline-media-seek" type="range" min="0" max="${mediaDuration||1}" step=".1" value="0" aria-label="录播播放进度"><div class="timeline-markers">${mediaProgress.highlights.map(highlight=>`<button data-action="timeline-highlight" data-highlight="${highlight.highlight_index}" class="timeline-bulb" style="left:${clamp(highlight.offset_seconds/mediaDuration*100,1.5,98.5)}%" aria-label="${esc(`${displayOffset(ctx,highlight.offset_seconds,true)} ${highlight.title}`)}" title="${esc(highlight.title)}">💡</button>`).join("")}</div>${activityMarkerHtml(mediaActivities,{clock:timelineClock,time:time=>displayOffset(ctx,time,true)})}<div class="timeline-markers timeline-bookmark-markers">${bookmarks.markers(true)}</div></div>${axisMarkup(ctx,true,mediaDuration)}</div>`:"";
  }

  function patchTimingProgress(ctx) {
    const progress=ctx.player?.element?.querySelector('.timeline-integrated-progress');
    if(!progress||!document.createElement)return;
    const template=document.createElement('template');template.innerHTML=integratedProgressMarkup(ctx);
    const fresh=template.content.firstElementChild;if(!fresh)return;
    // Preserve the player, seek input, cursor, active selection and clip handles.
    // Only passive drawing/marker layers change when the PDT mapping arrives.
    for(const selector of ['.timeline-area','.timeline-markers:not(.timeline-bookmark-markers)','.timeline-bookmark-markers','.timeline-axis']){
      const current=progress.querySelector(selector),next=fresh.querySelector(selector);
      if(current&&next)current.replaceWith(next);
    }
    const track=progress.querySelector('.timeline-activity-track'),next=fresh.querySelector('.timeline-activity-track');
    if(track&&next)track.replaceWith(next);else if(track)track.remove();else if(next)progress.querySelector('.timeline-chart')?.append(next);
  }

  function render(ctx,{preservePlayerControls=false}={}) {
    const element = root();
    if (!element || !valid(ctx)) return;
    if(ctx.player?.isFullscreen){ctx.needsRender=true;paintPlayerControls(ctx);paint(ctx,true);return;}
    const preserved=ctx.player?.element;
    const sameSession=ctx.renderKey===cacheKey(ctx),restoreScroll=sameSession?preserveScroll(element):()=>{};
    const focusedNode=element.contains?.(document.activeElement)?document.activeElement:null;
    const focusedId=focusedNode?.id||"";
    const caret=focusedId&&Number.isInteger(focusedNode.selectionStart)?[focusedNode.selectionStart,focusedNode.selectionEnd,focusedNode.selectionDirection]:null;
    const data = ctx.data;
    const duration = data?.duration_seconds;
    const chart = timelineDensity(data?.buckets, duration, data?.bucket_seconds);
    const activityLayout = activityMarkerLayout(data?.activities || [], duration, element.clientWidth);
    const unpositionedActivities = (data?.activities || []).filter(activity => !finite(activity.offset_seconds)).length;
    const items = highlights(ctx);
    const clipEditor=ctx.replay?`<div class="timeline-clip-editor" ${ctx.clipMode?"":"hidden"}><div class="timeline-clip-endpoints"><label>起点<input id="timeline-clip-start" type="text" inputmode="decimal" placeholder="00:00:00" aria-label="切片起点时间"></label><button data-action="timeline-clip-start" title="当前画面设为起点">设为起点</button><label>终点<input id="timeline-clip-end" type="text" inputmode="decimal" placeholder="00:00:00" aria-label="切片终点时间"></label><button data-action="timeline-clip-end" title="当前画面设为终点">设为终点</button><span class="timeline-clip-length"></span><button data-action="timeline-clip-download">${icon("download")}下载 MP4</button></div><p class="timeline-clip-message" role="status"></p></div>`:"";
    const integrated=integratedProgressMarkup(ctx);
    const selected = items[ctx.selectedHighlight];
    const configured = ctx.settings?.base_url && ctx.settings?.key_configured && ctx.settings?.model;
    const snapshot = ctx.result?.snapshot_end_epoch;
    const sessionDate = finite(data?.start_epoch) && data.start_epoch > 0
      ? formatWorldTime(data.start_epoch,{date:true,seconds:false})
      : "场次时间待确认";
    const empty = !ctx.anchorId || !ctx.sessionId ? "等待登记直播场次" : !data ? (ctx.error ? "回放暂时不可用" : "正在读取本场归档…") : !data.chat_count && !data.activities.length ? "本场还没有归档聊天" : !hasDuration(ctx) || (!data.messages.length && !activityLayout.markers.length) ? "暂无可定位的回放时间" : "";
    const bucketLabel = data?.bucket_seconds === 60 || !data?.bucket_seconds ? "每分钟消息" : `每 ${num(data.bucket_seconds)} 秒消息`;
    const markup = `<section class="timeline-shell" aria-label="直播互动回放">
      <div class="timeline-heading"><div class="timeline-heading-context"><div class="timeline-heading-title"><h3>互动回放</h3>${ctx.sessionId?`<span class="timeline-session-id"><span>场次 ID</span><code data-selectable>${esc(ctx.sessionId)}</code><button data-action="timeline-copy-session" aria-label="复制场次 ID" title="复制场次 ID">${copyIcon}</button></span>`:""}</div><span class="timeline-heading-date">${esc(sessionDate)}${data?.summary?.coverage === "partial" ? " · 部分监控" : ""}</span></div><div class="timeline-speaker-summary" aria-label="本场发言人数统计">${speakerSummary(ctx.qualityOverview||data?.summary)}</div></div>
      <div class="timeline-player ${ctx.replay?"has-recording":""} controls-visible"><div class="timeline-video ${ctx.reducedMotion ? "reduced-motion" : ""}" aria-label="录播播放器与归档弹幕">
        <video class="timeline-media" playsinline preload="metadata" ${ctx.mediaMuted?"muted":""} disablepictureinpicture></video>
        <div class="timeline-danmaku" aria-hidden="true"></div>
        <div class="timeline-video-center"><span>${ctx.replayLoading?"正在获取录播…":ctx.replay?"正在加载录播…":"暂无录播回放"}</span><button class="timeline-loading-retry" data-action="timeline-replay-refresh" hidden>重试加载</button></div>
      </div>
      <div class="timeline-controls">${integrated}${clipEditor}<button class="timeline-play" data-action="timeline-play" aria-label="${ctx.playing ? "暂停回放" : "播放回放"}" ${!hasDuration(ctx)&&!ctx.replay ? "disabled" : ""}>${ctx.playing ? pauseIcon : icon("play")}</button><span class="timeline-time" title="${esc(getDisplayTimezone())}"><b class="timeline-position">${clockLabels(ctx).position}</b><span>—</span><span class="timeline-duration">${clockLabels(ctx).end}</span></span><small class="timeline-elapsed">${clockLabels(ctx).duration}</small><small class="timeline-clock-note" ${clockLabels(ctx).note?"":"hidden"}>${clockLabels(ctx).note}</small><span class="timeline-control-spacer"></span>${ctx.replay?`<button data-action="timeline-mute" aria-label="${ctx.mediaMuted?"开启声音":"关闭声音"}">${icon(ctx.mediaMuted?"muted":"volume")}</button><input id="timeline-volume" type="range" min="0" max="1" step=".01" value="${ctx.mediaMuted?0:ctx.mediaVolume}" aria-label="音量" title="音量">`:""}${ctx.replay?`<button data-action="timeline-clip-toggle" aria-label="框选录像切片" title="框选录像切片" aria-pressed="${ctx.clipMode}" ${!canClip(ctx)?"disabled":""}>${clipIcon}</button>`:""}<button data-action="timeline-fullscreen" aria-label="全屏播放（含弹幕）" title="全屏播放（含弹幕）">${icon("fullscreen")}</button></div></div>
      <div class="timeline-clip-toolbar"><button class="small" data-action="timeline-clip-toggle" aria-pressed="${ctx.clipMode}" ${!canClip(ctx)?"disabled":""}>${clipIcon}选择切片</button><button class="small" data-action="timeline-session-download" ${!canDownloadSession(ctx)||ctx.clipBusy?"disabled":""}>${icon("download")}下载整场</button><span class="timeline-clip-hint"></span><button class="small quiet timeline-clip-jobs" data-action="show-clip-jobs">${icon("download")}下载任务</button><span class="timeline-session-download-notice" role="status"></span></div>
      ${ctx.replayError?`<p class="timeline-error" role="alert">${esc(ctx.replayError)} <button data-action="timeline-replay-refresh">重试</button></p>`:""}
      ${timingStatus(ctx)}
      ${ctx.replay&&!ctx.replay.time_mapping_pending?`<details class="timeline-calibration"><summary>弹幕校准</summary><label>时间偏移<input id="timeline-replay-offset" type="number" min="-86400" max="86400" step="1" value="${ctx.replayCorrection}" aria-label="弹幕时间偏移（秒）">秒</label><p>${timeMap(ctx).estimated?"部分片段按录播时间推算，可微调偏移。":timeMap(ctx).dated?"已按录播时间标记对齐，可微调偏移。":"录播未提供可对齐的时间标记，请按画面微调偏移。"}正值让同一画面对应更晚的归档时间。</p></details>`:""}
      ${empty?`<p class="timeline-time-note">${esc(empty)}</p>`:""}
      ${!ctx.replay?`<div class="timeline-chart-heading"><span><i class="timeline-density-swatch"></i>${esc(bucketLabel)}</span><span>${chart.peak ? `峰值 ${num(chart.peak)} 条` : "暂无消息密度"}</span></div>
      <div class="timeline-chart ${!hasDuration(ctx) ? "disabled" : ""}"><svg class="timeline-area" viewBox="0 0 1000 64" preserveAspectRatio="none" role="img" aria-label="${esc(bucketLabel)}分布，峰值 ${num(chart.peak)} 条"><defs><linearGradient id="timeline-density-fill" x1="0" x2="0" y1="0" y2="1"><stop offset="0%" stop-color="#7d9d71" stop-opacity=".62"/><stop offset="100%" stop-color="#a9bea0" stop-opacity=".18"/></linearGradient></defs><path class="timeline-density-area" d="${chart.area}"/><path class="timeline-density-line" d="${chart.line}"/></svg>
        <div class="timeline-chart-grid" aria-hidden="true"><i></i><i></i><i></i><i></i><i></i></div><div class="timeline-cursor" style="left:${hasDuration(ctx) ? ctx.position / duration * 100 : 0}%" aria-hidden="true"><i></i></div>
        <input id="timeline-seek" type="range" min="0" max="${hasDuration(ctx) ? duration : 1}" step="0.1" value="${ctx.position}" aria-label="回放时间位置" aria-valuetext="${esc(clockLabels(ctx).position)}" ${!hasDuration(ctx) ? "disabled" : ""}>
        <div class="timeline-markers">${items.map((highlight, index) => `<button data-action="timeline-highlight" data-highlight="${index}" class="timeline-bulb ${index === ctx.selectedHighlight ? "selected" : ""}" style="left:${clamp(highlight.offset_seconds / duration * 100, 1.5, 98.5)}%;--marker-row:${index && highlight.offset_seconds - items[index - 1].offset_seconds < duration * .035 ? index % 2 : 0}" aria-label="${esc(`${displayOffset(ctx,highlight.offset_seconds)} ${highlight.title}`)}" title="${esc(`${displayOffset(ctx,highlight.offset_seconds)} · ${highlight.title}`)}">💡</button>`).join("")}</div>
      ${activityMarkerHtml(activityLayout, { clock: timelineClock, time: time=>displayOffset(ctx,time) })}<div class="timeline-markers timeline-bookmark-markers">${bookmarks.markers()}</div></div>
      ${axisMarkup(ctx,false,duration)}
      ${activityLayout.markers.length ? '<p class="timeline-activity-legend"><span class="lottery">福袋</span><span class="red_packet">红包</span><span>活动标记 · 悬停查看，点击回放</span></p>' : ""}
      `:""}
      ${ctx.error ? `<p class="timeline-error" role="alert">归档读取失败：${esc(ctx.error)} <button data-action="timeline-refresh">重试</button></p>` : ""}
      ${data?.unknown_chat_times ? `<p class="timeline-time-note">${num(data.unknown_chat_times)} 条聊天时间未知，未放入时间轴。</p>` : ""}
      ${unpositionedActivities ? `<p class="timeline-time-note">${num(unpositionedActivities)} 个活动通知时间未知，未放入时间轴。</p>` : ""}
      ${data?.unconfirmed_activity_count ? `<p class="timeline-time-note">${num(data.unconfirmed_activity_count)} 条活动通知缺少可靠活动标识，保留在归档中，暂未标记。</p>` : ""}
      ${bookmarks.markup()}
      <div class="timeline-analysis"><div class="timeline-analysis-heading"><div class="timeline-analysis-label"><span class="timeline-lightbulb" aria-hidden="true">💡</span><div><strong>发现互动亮点</strong><span>${ctx.analyzing ? "正在用单次请求分析本场完整归档…" : ctx.result ? `${esc(ctx.result.model || ctx.settings?.model || "当前模型")} · 已分析 ${num(ctx.result.analyzed_messages)} 条记录${finite(snapshot) ? ` · 截至 ${timelineClock(snapshot)}` : ""}` : configured ? `${esc(ctx.settings.model)} · 单次分析本场完整归档` : "连接模型，寻找真实互动与有趣瞬间"}</span></div></div><div class="timeline-analysis-actions"><button data-action="timeline-configure" class="timeline-model-settings" aria-label="配置分析模型" title="配置 OpenAI 兼容接口与模型">${icon("settings")}</button>${ctx.analyzing ? '<button data-action="timeline-cancel" class="small">取消</button>' : `<button data-action="${ctx.analysisReadError ? "timeline-analysis-refresh" : "timeline-analyze"}" class="timeline-analyze" ${!ctx.analysisReadError && (!data?.total_messages || !ctx.anchorId || !ctx.sessionId) ? "disabled" : ""}>${ctx.analysisReadError ? "继续读取结果" : ctx.result ? "重新分析" : "分析本场"}${icon("chevron")}</button>`}</div></div>
        ${ctx.settingsError && !ctx.result ? `<p class="timeline-time-note">模型配置读取失败：${esc(ctx.settingsError)}</p>` : ""}
        ${ctx.analysisError ? `<p class="timeline-error" role="alert">${esc(ctx.analysisError)}</p>` : ""}
        ${ctx.analyzing ? '<div class="timeline-analysis-progress" role="status"><span></span><p>正在核对聊天上下文与互动证据，完成后会在时间轴标出 💡</p></div>' : ctx.result ? `<p class="timeline-analysis-summary">${esc(ctx.result.summary || (items.length ? "点击 💡 跳转到对应时刻。" : "本次分析未发现有充分证据的互动亮点。"))}</p>${items.length ? `<div class="timeline-highlight-list">${items.map((highlight, index) => `<div class="timeline-highlight-entry"><button class="timeline-highlight-card ${index === ctx.selectedHighlight ? "selected" : ""}" data-action="timeline-highlight" data-highlight="${index}"><time>${displayOffset(ctx,highlight.offset_seconds)}</time><span>${esc(highlight.title)}</span>${icon("chevron")}</button><button class="timeline-highlight-clip" data-action="timeline-highlight-clip" data-highlight="${index}" aria-label="下载亮点切片：${esc(highlight.title)}" ${!canClip(ctx)?"disabled":""}>${clipIcon}<span>切片</span></button></div>`).join("")}</div>` : ""}` : '<p class="timeline-analysis-empty">亮点会以 💡 标在时间轴上，点击即可回看。</p>'}
        ${selected ? `<div class="timeline-evidence"><div><time>${displayOffset(ctx,selected.offset_seconds)}</time><strong>${esc(selected.title)}</strong><button class="timeline-evidence-clip" data-action="timeline-highlight-clip" data-highlight="${ctx.selectedHighlight}" ${!canClip(ctx)?"disabled":""}>${clipIcon}切片</button><button data-action="timeline-close-highlight" aria-label="收起亮点说明">${icon("close")}</button></div>${selected.reason ? `<p>${esc(selected.reason)}</p>` : ""}${selected.evidence.length ? `<div class="timeline-evidence-quotes">${selected.evidence.slice(0, 3).map(message => `<blockquote><span>${esc(message.sender_name || "观众")}</span><p>${esc(message.content)}</p></blockquote>`).join("")}</div>` : ""}</div>` : ""}
      </div>
    </section>`;
    if(preserved?.parentElement?.parentElement===element&&document.createElement){
      // Keep the video connected while updating charts and metadata. Removing it
      // from the document can abort a seek or pause an active HLS stream.
      const template=document.createElement("template");template.innerHTML=markup;
      const freshShell=template.content.firstElementChild,oldShell=preserved.parentElement;
      const savedBookmarks=sameSession&&freshShell.querySelector('.session-bookmarks')?oldShell.querySelector('.session-bookmarks'):null;
      attachPlayer(ctx,template.content,preserved,true,preservePlayerControls);
      for(const child of [...oldShell.childNodes])if(child!==preserved&&child!==savedBookmarks)child.remove();
      let afterPlayer=false,afterBookmarks=false;
      for(const child of [...freshShell.childNodes]){
        if(child.matches?.(".timeline-player")){afterPlayer=true;continue;}
        if(savedBookmarks&&child.matches?.('.session-bookmarks')){bookmarks.patchView(savedBookmarks,child);afterBookmarks=true;continue;}
        if(!afterPlayer)oldShell.insertBefore(child,preserved);
        else if(savedBookmarks&&!afterBookmarks)oldShell.insertBefore(child,savedBookmarks);
        else oldShell.append(child);
      }
    }else{element.innerHTML=markup;attachPlayer(ctx,element,preserved,false,preservePlayerControls);}
    ctx.renderKey=cacheKey(ctx);
    bookmarks.bindView(element.querySelector('.session-bookmarks'));
    paintPlayerControls(ctx);
    ctx.danmakuKey = "";
    ctx.activityKey = "";
    paint(ctx, true);
    if(focusedId&&document.activeElement!==focusedNode){const focused=element.querySelector(`#${focusedId}`);focused?.focus?.({preventScroll:true});if(caret)focused?.setSelectionRange?.(...caret);}
    restoreScroll();
    onRender();
  }

  function paint(ctx, force = false) {
    if (!valid(ctx)) return;
    const element = root();
    if (!element) return;
    const duration = ctx.data?.duration_seconds || 0;
    const clock = clockLabels(ctx).position;
    for (const selector of [".timeline-position", ".timeline-video-time"]) {
      const label = element.querySelector(selector);
      if (label && label.textContent !== clock) label.textContent = clock;
    }
    const seek = element.querySelector("#timeline-seek");
    if (seek) { seek.value = ctx.position; seek.setAttribute("aria-valuetext", clock); }
    const cursor = element.querySelector(".timeline-cursor");
    const cursorTime=ctx.replay?ctx.player?.currentTime:ctx.position,cursorDuration=ctx.replay?ctx.player?.duration:duration;
    if (cursor) cursor.style.left = `${cursorDuration > 0 ? clamp(cursorTime / cursorDuration * 100,0,100) : 0}%`;
    paintPlayerControls(ctx);
    const layer = element.querySelector(".timeline-danmaku");
    if (!layer) return;
    const inArchive=!ctx.replay?.time_mapping_pending&&finite(ctx.position)&&ctx.position>=0&&ctx.position<=duration;
    const messages = ctx.danmaku&&inArchive ? timelineDanmaku(ctx.data?.messages, ctx.position) : [];
    const visible = ctx.reducedMotion ? messages.slice(-4) : messages;
    const key = visible.map(message => `${message.id ?? message._index}:${message.offset_seconds}`).join("|");
    if (force || key !== ctx.danmakuKey) {
      layer.innerHTML = visible.map(message => `<span class="timeline-danmaku-item ${ctx.reducedMotion ? "static" : ""}" data-danmaku-key="${esc(`${message.id??message._index}:${message.offset_seconds}`)}" data-lane="${message.lane}"><b>${esc(message.sender_name || "观众")}</b><span>${esc(String(message.content || "").slice(0, 240))}</span></span>`).join("");
      ctx.danmakuKey = key;
    }
    if (!ctx.reducedMotion) layer.querySelectorAll(".timeline-danmaku-item").forEach((item, index) => {
      const message = visible[index];
      if (!message) return;
      item.style.left = `${(1 - message.progress) * 100}%`;
      item.style.transform = `translateX(-${message.progress * 100}%)`;
    });
  }

  function frame(ctx) {
    if (!valid(ctx)) { if (current === ctx) unmount(); return; }
    if (document.hidden || root()?.closest?.("[hidden]")) { ctx.lastFrame = Date.now(); return; }
    const now = Date.now();
    const elapsed = Math.min(2, Math.max(0, (now - ctx.lastFrame) / 1000));
    ctx.lastFrame = now;
    if(ctx.replayLoading||ctx.replay&&!ctx.mediaReady)paintPlayerControls(ctx);
    if(ctx.player?.hasMedia){syncMediaTime(ctx,ctx.player.currentTime);return;}
    if (!ctx.playing || !hasDuration(ctx)) return;
    ctx.position = Math.min(ctx.data.duration_seconds, ctx.position + elapsed * ctx.speed);
    if (ctx.position >= ctx.data.duration_seconds && !isCurrentLive(ctx)) {
      ctx.playing = false;
      render(ctx);
      return;
    }
    paint(ctx);
  }

  async function pollAnalysis(ctx, request) {
    if (!valid(ctx) || ctx.analysisRequest !== request || !ctx.jobId) return;
    try {
      const response = await bridge("highlight_status", { job_id: ctx.jobId });
      if (!valid(ctx) || ctx.analysisRequest !== request) return;
      if (response.session_id && response.session_id !== ctx.sessionId || response.anchor_id && response.anchor_id !== ctx.anchorId) throw Error("分析结果与当前场次不一致，请重新分析。");
      if (response.status === "completed") {
        if (!response.result) throw Error("分析结果为空，请重新分析。");
        ctx.result = response.result;
        results.set(cacheKey(ctx), response.result);
        ctx.analyzing = false;
        ctx.jobId = "";
        ctx.analysisReadError = false;
        ctx.analysisError = "";
        ctx.selectedHighlight = -1;
      } else if (response.status === "error" || response.status === "cancelled") {
        ctx.analyzing = false;
        ctx.jobId = "";
        ctx.analysisError = response.status === "cancelled" ? "本次分析已取消。" : String(response.error || "本场分析失败。");
      } else {
        ctx.analysisTimer = setTimeout(() => void pollAnalysis(ctx, request), 1200);
        ctx.analysisTimer?.unref?.();
      }
    } catch (error) {
      if (!valid(ctx) || ctx.analysisRequest !== request) return;
      ctx.analyzing = false;
      ctx.analysisReadError = true;
      ctx.analysisError = `读取分析结果失败：${String(error?.message || error)}`;
    }
    if (valid(ctx) && ctx.analysisRequest === request) render(ctx);
  }

  async function analyze(ctx) {
    if (ctx.analyzing || !ctx.data?.total_messages) return;
    const request = ++ctx.analysisRequest;
    ctx.analyzing = true;
    ctx.analysisError = "";
    ctx.analysisReadError = false;
    render(ctx);
    try {
      const settings = await bridge("highlight_settings", {});
      if (!valid(ctx) || ctx.analysisRequest !== request) return;
      ctx.settings = settings;
      if (!settings?.base_url || !settings?.key_configured || !settings?.model) {
        ctx.analyzing = false;
        render(ctx);
        toast("请先配置分析接口与模型");
        onConfigure();
        return;
      }
      const response = await bridge("highlight_analyze", { anchor_id: ctx.anchorId, session_id: ctx.sessionId });
      if (!valid(ctx) || ctx.analysisRequest !== request) {
        if (response?.job_id) void Promise.resolve(bridge("highlight_cancel", { job_id: response.job_id })).catch(() => {});
        return;
      }
      if (!response?.job_id) throw Error("未能创建分析任务。");
      ctx.jobId = response.job_id;
      await pollAnalysis(ctx, request);
    } catch (error) {
      if (!valid(ctx) || ctx.analysisRequest !== request) return;
      ctx.analyzing = false;
      ctx.analysisError = String(error?.message || error);
      render(ctx);
    }
  }

  function seek(ctx, value) {
    if (!hasDuration(ctx) || !finite(value)) return;
    if(ctx.player?.hasMedia){
      const target=timeMap(ctx).toMedia(value);
      if(target===null||target<0||(ctx.player.duration>0&&target>ctx.player.duration)){toast("该时刻没有对应的录播片段");return false;}
      ctx.player.seek(target);
    }
    ctx.position = clamp(value, 0, ctx.data.duration_seconds);
    ctx.lastFrame = Date.now();
    paint(ctx, true);
    return true;
  }

  async function action(element) {
    if(await bookmarks.action(element))return true;
    const actionName = element.dataset.action;
    if (!actionName?.startsWith("timeline-")) return false;
    const ctx = current;
    if (!ctx || !valid(ctx)) return true;
    if (actionName === "timeline-copy-session") {
      if(!ctx.sessionId)return true;
      try { await globalThis.navigator.clipboard.writeText(ctx.sessionId); toast("场次 ID 已复制"); }
      catch { toast("复制失败，请选中场次 ID 手动复制",true); }
    } else if (actionName === "timeline-play" && (hasDuration(ctx)||ctx.player?.hasMedia)) {
      if(ctx.player?.hasMedia){if(ctx.playing){ctx.playing=false;ctx.player.pause();paintPlayerControls(ctx);}else{ctx.playing=true;await ctx.player.play();}return true;}
      if (!ctx.playing && ctx.position >= ctx.data.duration_seconds) ctx.position = 0;
      ctx.playing = !ctx.playing;
      ctx.lastFrame = Date.now();
      render(ctx);
    } else if (actionName === "timeline-danmaku") { ctx.danmaku = !ctx.danmaku; render(ctx); }
    else if (actionName === "timeline-latest" && hasDuration(ctx)) {
      if(seek(ctx,isCurrentLive(ctx)?Math.max(0,ctx.data.duration_seconds-12):0)===false)return true;
      ctx.playing = true;
      if(ctx.player?.hasMedia)void ctx.player.play();
      ctx.lastFrame = Date.now();
      render(ctx);
    } else if (actionName === "timeline-highlight") {
      const index = Number(element.dataset.highlight);
      const highlight = highlights(ctx)[index];
      if (highlight) { ctx.selectedHighlight = index; if(seek(ctx,highlight.offset_seconds)===false)return true;ctx.playing = true;if(ctx.player?.hasMedia)void ctx.player.play();render(ctx); }
    } else if (actionName === "timeline-activity") {
      const activity = ctx.data?.activities[Number(element.dataset.activity)];
      if (activity && finite(activity.offset_seconds)) { ctx.selectedActivity = activity._key;if(seek(ctx,activity.offset_seconds)===false)return true;ctx.playing = true;if(ctx.player?.hasMedia)void ctx.player.play();render(ctx); }
    } else if (actionName === "timeline-close-highlight") { ctx.selectedHighlight = -1; render(ctx); }
    else if(actionName==="timeline-clip-toggle") {
      if(!canClip(ctx)){toast("没有可播放录像，暂时无法切片。",true);return true;}
      ctx.clipMode=!ctx.clipMode;if(ctx.clipMode)ctx.clipMeta=null;
      if(ctx.clipMode){ctx.playing=false;ctx.player.pause();if(!ctx.clipRange)setClipRange(ctx,clipAroundTime(ctx.player.currentTime,ctx.player.duration));}
      paintPlayerControls(ctx);
    }
    else if(actionName==="timeline-clip-start"||actionName==="timeline-clip-end") {
      if(!canClip(ctx))return true;
      const time=clamp(ctx.player.currentTime,0,ctx.player.duration),old=ctx.clipRange;
      const range=actionName==="timeline-clip-start"?normalizeClipRange(time,old?.end>time?old.end:Math.min(ctx.player.duration,time+30),ctx.player.duration):normalizeClipRange(old?.start<time?old.start:Math.max(0,time-15),time,ctx.player.duration);
      if(range)setClipRange(ctx,range);else{ctx.clipError="起点需要早于终点。";paintClip(ctx);}
    }
    else if(actionName==="timeline-clip-download")await createClip(ctx,ctx.clipError?null:ctx.clipRange);
    else if(actionName==="timeline-session-download")await downloadSession(ctx);
    else if(actionName==="timeline-highlight-clip") {
      const highlight=highlights(ctx)[Number(element.dataset.highlight)];
      if(!highlight)return true;
      const time=timeMap(ctx).toMedia(highlight.offset_seconds),range=clipAroundTime(time,ctx.player?.duration);
      if(!range){toast("该亮点没有可对齐的录像时间，请手动框选。",true);return true;}
      await createClip(ctx,range,highlight.title,{source:"highlight",title:highlight.title});
    }
    else if (actionName === "timeline-configure") onConfigure();
    else if (actionName === "timeline-refresh") await load(ctx);
    else if(actionName==="timeline-replay-refresh"){ctx.replayExpiredRetry=false;await loadReplay(ctx,true);}
    else if(actionName==="timeline-timing-refresh")await loadReplayTiming(ctx,true);
    else if(actionName==="timeline-fullscreen"){try{await ctx.player?.fullscreen();}catch{toast("当前窗口无法进入全屏",true);}}
    else if(actionName==="timeline-mute"){const audio=audioPreferences.setMuted(!ctx.mediaMuted);ctx.mediaMuted=audio.muted;ctx.mediaVolume=audio.volume;ctx.player?.muted(audio.muted);render(ctx);}
    else if (actionName === "timeline-analyze") await analyze(ctx);
    else if (actionName === "timeline-analysis-refresh" && ctx.jobId) {
      ctx.analyzing = true; ctx.analysisReadError = false; ctx.analysisError = ""; render(ctx);
      await pollAnalysis(ctx, ctx.analysisRequest);
    } else if (actionName === "timeline-cancel") { cancelJob(ctx); ctx.analysisError = "本次分析已取消。"; render(ctx); }
    return true;
  }

  function setVolume(ctx,value) {
    if(!Number.isFinite(value))return;
    const audio=audioPreferences.setVolume(value);
    ctx.mediaVolume=audio.volume;ctx.mediaMuted=audio.muted;
    ctx.player?.volume(audio.volume);
    paintPlayerControls(ctx);
  }

  function input(target) {
    bookmarks.input(target);
    const ctx = current;
    if (ctx && valid(ctx) && target.id === "timeline-seek") seek(ctx, Number(target.value));
    if(ctx&&valid(ctx)&&target.id==="timeline-media-seek")ctx.player?.seek(Number(target.value));
    if(ctx&&valid(ctx)&&target.id==="timeline-volume")setVolume(ctx,Number(target.value));
    if(ctx&&valid(ctx)&&["timeline-clip-start","timeline-clip-end"].includes(target.id)) {
      if(!canClip(ctx)||ctx.clipBusy)return;
      const edge=target.id.endsWith("start")?"start":"end";
      ctx.clipDraft={start:clipTime(ctx.clipRange?.start),end:clipTime(ctx.clipRange?.end),...ctx.clipDraft,[edge]:target.value};
      const start=parseClipTime(ctx.clipDraft.start),end=parseClipTime(ctx.clipDraft.end);
      if(start===null||end===null||start<0||end<=start||end>ctx.player.duration){ctx.clipRange=null;ctx.clipError="请设置有效的起止时间。";}
      else{ctx.clipRange=normalizeClipRange(start,end,ctx.player.duration);ctx.clipError="";}
      ctx.clipNotice="";paintClip(ctx);
    }
  }

  function change(target) {
    bookmarks.change(target);
    const ctx = current;
    if (!ctx || !valid(ctx)) return;
    if(target.id==="timeline-volume")setVolume(ctx,Number(target.value));
    if (target.id === "timeline-seek") seek(ctx, Number(target.value));
    if(target.id==="timeline-media-seek")ctx.player?.seek(Number(target.value));
    if(target.id==="timeline-replay-offset"){
      const value=Number(target.value);if(!Number.isFinite(value)||Math.abs(value)>86400)return;
      ctx.replayCorrection=value;
      try{localStorage.setItem(`livescope.replay-offset.${cacheKey(ctx)}`,String(value));}catch{}
      if(ctx.player?.hasMedia)syncMediaTime(ctx,ctx.player.currentTime);
      render(ctx);
    }
  }

  function selectSession(sessionId, explicit = true) {
    const ctx = current;
    if (!ctx || !valid(ctx) || !sessionId) return;
    ctx.explicitSession = explicit && sessionId !== ctx.anchor.session_id;
    if (ctx.sessionId === sessionId) return;
    cancelJob(ctx);
    ctx.clipPointerCleanup?.();ctx.clipMode=false;ctx.clipRange=null;ctx.clipDraft=null;ctx.clipError="";ctx.clipNotice="";ctx.clipBusy=false;ctx.fullDownloadNotice="";ctx.fullDownloadError=false;ctx.fullDownloadRequestKey=null;
    ctx.request++;
    ctx.sessionId = sessionId;ctx.clipMeta=null;bookmarks.select();
    ctx.replayRequest++;ctx.replayLoadedFor="";ctx.replay=null;ctx.mediaReady=false;ctx.replayError="";ctx.replayExpiredRetry=false;
    ctx.timingRequest++;ctx.timingStartedFor=null;ctx.timingLoading=false;ctx.timingError="";ctx.timingPatchedFor=null;
    if(ctx.player)void ctx.player.setSource(null);
    try{ctx.replayCorrection=Number(localStorage.getItem(`livescope.replay-offset.${cacheKey(ctx)}`))||0;}catch{ctx.replayCorrection=0;}
    ctx.data = null; ctx.qualityOverview = null; ctx.loading = false; ctx.error = ""; ctx.position = 0; ctx.playing = true;
    ctx.analysisError = ""; ctx.analysisReadError = false; ctx.selectedHighlight = -1; ctx.selectedActivity = "";
    ctx.result = results.get(cacheKey(ctx)) || null;
    render(ctx);
    void load(ctx);
    void loadReplay(ctx);
    void bookmarks.load();
  }

  function tick(anchor) {
    const ctx = current;
    if (!ctx || !valid(ctx)) return;
    // A room may recover its identity after startup or belong to a new anchor.
    // Remount first so old archives, explicit selections and analysis jobs cannot
    // survive that change, even if both anchors happen to use the same session ID.
    if (ctx.anchorId !== (anchor.anchor_id || "")) {
      results.clear();
      mount(anchor);
      return;
    }
    const wasCurrentLive = isCurrentLive(ctx);
    ctx.anchor = anchor;
    if (!ctx.explicitSession && anchor.session_id && ctx.sessionId !== anchor.session_id) { selectSession(anchor.session_id, false); return; }
    if (wasCurrentLive !== isCurrentLive(ctx)) render(ctx);
    if (!ctx.loading && Date.now() - ctx.lastLoad >= 5000 && isCurrentLive(ctx)) void load(ctx);
    if (!ctx.data && !ctx.loading) void load(ctx);
  }

  function refreshReplay(){if(current&&valid(current)){current.replayLoadedFor="";void loadReplay(current,true);}}
  function updateQuality(overview) {
    const ctx=current;
    if(!ctx||!valid(ctx)||overview&&(overview.session_id!==ctx.sessionId||overview.anchor_id&&overview.anchor_id!==ctx.anchorId))return;
    ctx.qualityOverview=overview;
    const summary=root()?.querySelector(".timeline-speaker-summary");
    if(summary)summary.innerHTML=speakerSummary(overview||ctx.data?.summary);
  }
  function refreshTimeDisplay(){
    const ctx=current;if(!ctx||!valid(ctx))return;
    ctx.clockCache=null;
    if(ctx.player?.isFullscreen){
      for(const axis of root()?.querySelectorAll?.(".timeline-axis")||[])
        axis.outerHTML=axisMarkup(ctx,axis.classList.contains("timeline-media-axis"));
    }
    render(ctx);ctx.player?.refreshOverlay?.();
  }
  return { mount, tick, action, input, change, unmount, selectSession,updateQuality,refreshReplay,refreshTimeDisplay,refreshBookmarks:()=>bookmarks.load() };
}
