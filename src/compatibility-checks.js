import {esc,icon} from "./presentation.js";

const codecs={avc:{label:"H.264 / AVC",contentType:'video/mp4; codecs="avc1.640028"'},hevc:{label:"HEVC / H.265",contentType:'video/mp4; codecs="hvc1.1.6.L93.B0"'}};
const unavailable={state:"unavailable"};
async function bounded(operation,ms=4500) {
  let timer;
  try { return await Promise.race([Promise.resolve().then(operation),new Promise(resolve=>{timer=setTimeout(()=>resolve({state:"timeout"}),ms);})]); }
  catch { return {state:"error"}; }
  finally { clearTimeout(timer); }
}

export function redactCompatibilityReport(value,depth=0) {
  if(depth>8)return "<内容已省略>";
  if(Array.isArray(value))return value.slice(0,100).map(item=>redactCompatibilityReport(item,depth+1));
  if(value&&typeof value==="object")return Object.fromEntries(Object.entries(value).slice(0,100).map(([key,item])=>[key,/(?:token|secret|password|authorization|cookie|api[_-]?key|user[_-]?name|computer[_-]?name|host[_-]?name|machine[_-]?name)/i.test(key)?"<已隐藏>":redactCompatibilityReport(item,depth+1)]));
  if(typeof value==="string")return value.replace(/Bearer\s+\S+/gi,"Bearer <已隐藏>").replace(/https?:\/\/[^\s"<>]+/gi,"<链接已隐藏>").replace(/[a-z]:[\\/][^\r\n"<>]*/gi,"<本地路径已隐藏>").replace(/\/(?:Users|home)\/[^\s"<>]+/g,"<本地路径已隐藏>").slice(0,4000);
  return typeof value==="bigint"?String(value):value;
}

export async function collectBrowserCompatibility({document:doc=globalThis.document,window:view=globalThis.window,navigator:nav=globalThis.navigator,timeoutMs=4500}={}) {
  const video=doc?.createElement?.("video"),audio=doc?.createElement?.("audio");
  const source=view?.MediaSource;
  const declaration=(element,type)=>{try{return element?.canPlayType?.(type)||"";}catch{return "";}};
  const mediaSource=type=>{try{return typeof source?.isTypeSupported==="function"?source.isTypeSupported(type):null;}catch{return null;}};
  const decoding=async(type,contentType)=>{
    if(typeof nav?.mediaCapabilities?.decodingInfo!=="function")return unavailable;
    return bounded(async()=>{const info=await nav.mediaCapabilities.decodingInfo({type,video:{contentType,width:1920,height:1080,bitrate:5000000,framerate:30}});return {state:"checked",supported:Boolean(info.supported),smooth:Boolean(info.smooth),power_efficient:Boolean(info.powerEfficient)};},timeoutMs);
  };
  const videoCodecs=Object.fromEntries(await Promise.all(Object.entries(codecs).map(async([key,codec])=>[key,{content_type:codec.contentType,can_play_type:declaration(video,codec.contentType),media_source:mediaSource(codec.contentType),decoding:{file:await decoding("file",codec.contentType),media_source:await decoding("media-source",codec.contentType)}}])));
  let webgl={available:false};
  try {
    const canvas=doc?.createElement?.("canvas"),options={powerPreference:"high-performance",antialias:false};
    const gl=canvas?.getContext?.("webgl2",options)||canvas?.getContext?.("webgl",options);
    if(gl){try{const debug=gl.getExtension("WEBGL_debug_renderer_info");webgl={available:true,renderer:String(gl.getParameter(debug?debug.UNMASKED_RENDERER_WEBGL:gl.RENDERER)||""),vendor:String(gl.getParameter(debug?debug.UNMASKED_VENDOR_WEBGL:gl.VENDOR)||""),max_texture_size:gl.getParameter(gl.MAX_TEXTURE_SIZE),requested_power_preference:"high-performance"};}finally{gl.getExtension("WEBGL_lose_context")?.loseContext();}}
  }catch{webgl={available:false,state:"error"};}
  return {video_codecs:videoCodecs,webgl,audio:{aac:declaration(audio,'audio/mp4; codecs="mp4a.40.2"'),mp3:declaration(audio,"audio/mpeg"),opus:declaration(audio,'audio/webm; codecs="opus"'),web_audio:Boolean(view?.AudioContext||view?.webkitAudioContext),output_selection:typeof audio?.setSinkId==="function"},apis:{request_video_frame_callback:typeof video?.requestVideoFrameCallback==="function",fullscreen_enabled:Boolean(doc?.fullscreenEnabled),media_capabilities:typeof nav?.mediaCapabilities?.decodingInfo==="function",media_source:typeof source?.isTypeSupported==="function"},estimate_note:"能力声明与估计不能证明当前录像已启用硬件解码。"};
}

const yesNo=value=>value===true?"支持":value===false?"未声明支持":"未提供";
const playable=value=>value==="probably"?"声明可播放":value==="maybe"?"可能可播放":"未声明支持";
export function createCompatibilityChecks({invoke,toast=()=>{},collectBrowser=collectBrowserCompatibility,navigator:nav=globalThis.navigator}) {
  let current=null,lastReport=null;
  const valid=ctx=>current===ctx&&ctx.root.isConnected!==false;
  function render(ctx) {
    if(!valid(ctx))return;
    const browser=ctx.report?.browser;
    ctx.root.innerHTML=`<div class="compatibility-heading"><h3>兼容性检查</h3><div><button class="small" data-action="compatibility-run" ${ctx.busy?"disabled":""}>${icon("refresh")}${ctx.busy?"正在检查…":"运行检查"}</button><button class="small quiet" data-action="compatibility-copy" ${!ctx.report||ctx.busy?"disabled":""}>复制报告</button></div></div>
      ${!ctx.report?'<p class="compatibility-description">检查播放、图形与媒体工具。</p>':`<div class="compatibility-results">${Object.entries(codecs).map(([key,codec])=>{const result=browser?.video_codecs?.[key],info=result?.decoding?.file;return `<div class="compatibility-codec"><strong>${codec.label}</strong><span>视频 ${playable(result?.can_play_type)}</span><span>MSE ${yesNo(result?.media_source)}</span><span>解码能力 ${info?.state==="checked"?yesNo(info.supported):"未取得结果"}</span><span>流畅估计 ${info?.state==="checked"?yesNo(info.smooth):"未提供"}</span><span>节能估计 ${info?.state==="checked"?yesNo(info.power_efficient):"未提供"}</span></div>`;}).join("")}<div class="compatibility-device"><span>WebGL 图形设备</span><strong>${esc(browser?.webgl?.renderer||"未提供")}</strong></div><div class="compatibility-device"><span>最大纹理尺寸</span><strong>${esc(browser?.webgl?.max_texture_size??"未提供")}</strong></div></div><p class="compatibility-description">能力声明与估计，不能证明当前录像已启用硬件解码。</p><details class="compatibility-report"><summary>完整报告</summary><pre>${esc(JSON.stringify(ctx.report,null,2))}</pre></details>`}
      <p class="compatibility-status" role="status">${esc(ctx.notice)}</p>`;
  }
  function mount(root=document.querySelector("#compatibility-checks")) {
    unmount();if(!root)return;current={root,report:lastReport,busy:false,notice:""};render(current);
  }
  function unmount(){current=null;}
  async function action(element) {
    const name=element?.dataset?.action;if(!["compatibility-run","compatibility-copy"].includes(name))return false;
    const ctx=current;if(!ctx||!valid(ctx)||ctx.busy)return true;
    if(name==="compatibility-copy") {
      if(!ctx.report)return true;
      try{if(typeof nav?.clipboard?.writeText!=="function")throw Error();await nav.clipboard.writeText(JSON.stringify(redactCompatibilityReport(ctx.report),null,2));if(valid(ctx)){ctx.notice="已复制脱敏报告";render(ctx);}}
      catch{if(valid(ctx)){ctx.notice="复制失败，可在完整报告中手动复制。";render(ctx);}}
      return true;
    }
    ctx.busy=true;ctx.notice="";render(ctx);
    try {
      const [browser,native]=await Promise.all([bounded(()=>collectBrowser(),11000),bounded(()=>invoke("compatibility_report"),10000)]);
      if(!valid(ctx))return true;
      ctx.report=lastReport=redactCompatibilityReport({format:1,checked_at:new Date().toISOString(),browser,native});
      ctx.notice="检查完成";
    }catch{if(valid(ctx)){ctx.notice="检查未完成，请重试。";toast(ctx.notice,true);}}
    finally{if(valid(ctx)){ctx.busy=false;render(ctx);}}
    return true;
  }
  return {mount,unmount,action};
}
