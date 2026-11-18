import {esc, icon} from "./presentation.js";
import {clipTime, normalizeClipRange} from "./clip-selection.js";

const active = job => ["queued", "running"].includes(job.status);
const labels = {queued:"等待下载",running:"下载中",completed:"已完成",failed:"失败",cancelled:"已取消"};
const phases = {resolving:"获取录像",planning:"确认片段",downloading:"下载片段",muxing:"封装 MP4",validating:"校验文件"};
const sources = {manual:"手动切片",highlight:"互动亮点",bookmark:"直播打点",full_session:"整场直播"};
export const clipBytes = value => Number.isFinite(value) && value >= 0 ? (value >= 1048576 ? `${(value/1048576).toFixed(1)} MB` : `${Math.round(value/1024)} KB`) : "";
export function clipJobDay(job) {
  const date = new Date(job.created_at || "");
  return Number.isFinite(date.getTime()) ? new Date(date.getTime()+8*3600000).toISOString().slice(0,10) : "";
}
const shortDay = day => day ? day.slice(2).replaceAll("-","/") : "日期未知";
export function filterClipJobs(jobs, {search="",date="",status=""} = {}) {
  const query = search.trim().toLocaleLowerCase();
  return jobs.filter(job => (!date || clipJobDay(job) === date) && (!status || (status === "active" ? active(job) : job.status === status)) && (!query || [job.title,job.anchor,job.anchor_name,job.anchor_id,job.session_id,job.id,job.output_path,job.bookmark_id,sources[job.source||"manual"]].some(value=>String(value||"").toLocaleLowerCase().includes(query))));
}
export function groupClipJobs(jobs) {
  const groups = new Map();
  for (const job of jobs) { const day=clipJobDay(job); if(!groups.has(day))groups.set(day,[]); groups.get(day).push(job); }
  return [...groups].sort(([a],[b])=>b.localeCompare(a)).map(([day,items])=>({day,jobs:items.sort((a,b)=>(Date.parse(b.created_at)||0)-(Date.parse(a.created_at)||0)||String(b.id||"").localeCompare(String(a.id||"")))}));
}

export function clipJobMarkup(job, busy = false) {
  const progress = Number.isFinite(job.progress) ? Math.max(0, Math.min(100, job.progress)) : null;
  const range = normalizeClipRange(job.requested_start_seconds, job.requested_end_seconds);
  const actual = normalizeClipRange(job.actual_start_seconds, job.actual_end_seconds);
  const adjusted = actual && range && (Math.abs(actual.start-range.start)>.1 || Math.abs(actual.end-range.end)>.1);
  const caption = job.status === "running" ? phases[job.phase] || "正在处理" : labels[job.status] || "状态未知";
  const created = new Date(job.created_at || "");
  const clock = Number.isFinite(created.getTime()) ? new Date(created.getTime()+8*3600000).toISOString().slice(11,16) : "";
  return `<article class="clip-job ${esc(job.status)}"><div class="clip-job-heading"><strong>${esc(job.title || "直播切片")}</strong><span>${esc(caption)}</span></div>
    <p class="clip-job-context"><span>${esc(job.anchor||job.anchor_name||"主播未记录")}</span><span>${esc(sources[job.source||"manual"]||"直播切片")}</span>${clock?`<time datetime="${esc(job.created_at)}">${clock}</time>`:""}</p>
    ${range ? `<p class="clip-job-range">${clipTime(range.start)} — ${clipTime(range.end)}<span>${clipTime(range.end-range.start)}</span></p>` : job.mode==="full_session" ? '<p class="clip-job-range">整场录像 · 等待确认完整时长</p>' : ""}
    ${active(job) ? `<progress aria-label="${esc(job.title || "切片")}进度" max="100" ${progress === null ? "" : `value="${progress}"`}></progress><p class="clip-job-transfer">${progress === null ? "正在处理…" : `${Math.floor(progress)}%`}${clipBytes(job.downloaded_bytes) ? ` · ${clipBytes(job.downloaded_bytes)}${clipBytes(job.total_bytes) ? ` / ${clipBytes(job.total_bytes)}` : ""}` : ""}</p>` : ""}
    ${adjusted ? `<p class="clip-job-note">实际片段 ${clipTime(actual.start)} — ${clipTime(actual.end)}</p>` : ""}
    <p class="clip-job-session">场次 <span class="selectable">${esc(job.session_id)}</span></p>
    ${job.output_path && job.status === "completed" ? `<p class="clip-job-path selectable" title="${esc(job.output_path)}">${esc(job.output_path)}</p>` : ""}
    ${job.error ? `<p class="clip-job-error">${esc(job.error)}</p>` : ""}
    <div class="clip-job-actions">${active(job) ? `<button class="small quiet" data-action="clip-job-cancel" data-job="${esc(job.id)}" ${busy ? "disabled" : ""}>${busy ? "正在取消…" : "取消"}</button>` : ["failed","cancelled"].includes(job.status) ? `<button class="small" data-action="clip-job-retry" data-job="${esc(job.id)}" ${busy ? "disabled" : ""}>重试</button>` : ""}${job.status === "completed" && job.output_path ? `<button class="small" data-action="clip-job-folder" data-job="${esc(job.id)}">${icon("folder")}打开文件夹</button>` : ""}</div></article>`;
}

export function createClipPanel({bridge, invoke, toast = () => {}, onConfigure = () => {}}) {
  let current = null, timer = null, jobs = [];
  const filters = {search:"",date:"",status:""};
  const valid = ctx => current === ctx && ctx.root.isConnected !== false;
  const filtered = ctx => ctx.history ? jobs : filterClipJobs(jobs,filters);
  function listMarkup(ctx) {
    if(ctx.stale)return '<p class="clip-panel-empty" role="status">正在筛选任务…</p>';
    const visible=filtered(ctx);
    const groups=groupClipJobs(visible);
    const list=groups.length ? groups.map(group=>`<section class="clip-day-group"><h3>${shortDay(group.day)}<span>${group.jobs.length} 个任务</span></h3><div class="clip-job-list">${group.jobs.map(job=>clipJobMarkup(job,ctx.pending.has(job.id))).join("")}</div></section>`).join("") : `<p class="clip-panel-empty">${ctx.reading&&!jobs.length ? "正在读取任务…" : filters.search||filters.date||filters.status ? "没有符合条件的下载任务。" : "暂无下载任务。"}</p>`;
    return list+(ctx.history?`<nav class="clip-jobs-pagination" aria-label="下载任务分页"><span>${ctx.total&&visible.length?`${ctx.page*ctx.limit+1}–${Math.min(ctx.total,ctx.page*ctx.limit+visible.length)}`:"0"} / ${ctx.total} 个任务</span><div><button class="small quiet" data-action="clip-jobs-previous" ${ctx.page<=0||ctx.reading?"disabled":""}>上一页</button><button class="small quiet" data-action="clip-jobs-next" ${!ctx.hasMore||ctx.reading?"disabled":""}>下一页</button></div></nav>`:"");
  }
  function shell(ctx) {
    return `<div class="clip-panel-heading"><div><h2>下载任务</h2><span id="clip-jobs-count"></span></div><div class="clip-panel-tools"><button class="small quiet" data-action="clip-download-settings">${icon("settings")}命名规则</button><button class="small" data-action="clip-jobs-refresh">${icon("refresh")}刷新</button></div></div>
      <div class="clip-job-filters"><label class="clip-job-search">${icon("search")}<input id="clip-jobs-search" type="search" aria-label="搜索下载任务" placeholder="标题、主播、场次或文件名" value="${esc(filters.search)}"></label><label><span>下载日期</span><input id="clip-jobs-date" type="date" aria-label="下载日期" value="${esc(filters.date)}"></label><label><span>状态</span><select id="clip-jobs-status" aria-label="下载任务状态">${[["","全部状态"],["active","处理中"],["completed","已完成"],["failed","失败"],["cancelled","已取消"]].map(([value,label])=>`<option value="${value}" ${value===filters.status?"selected":""}>${label}</option>`).join("")}</select></label><button class="small quiet" data-action="clip-jobs-clear">清除筛选</button></div>
      <div id="clip-jobs-notice"></div><div id="clip-jobs-results">${listMarkup(ctx)}</div>`;
  }
  function render(ctx) {
    if (!valid(ctx)) return;
    const results=ctx.root.querySelector?.("#clip-jobs-results");
    const notice=`${ctx.tools?.available === false ? '<p class="clip-panel-notice">媒体工具未准备好，暂时无法生成 MP4。</p>' : ""}${ctx.error ? `<p class="clip-job-error" role="alert">${esc(ctx.error)}</p>` : ""}`;
    // The filter controls remain mounted during polling, preserving focus and IME input.
    if(results)results.innerHTML=listMarkup(ctx);else ctx.root.innerHTML=shell(ctx);
    const noticeRoot=ctx.root.querySelector?.("#clip-jobs-notice");if(noticeRoot)noticeRoot.innerHTML=notice;
    else if(notice)ctx.root.innerHTML+=notice;
    const total=ctx.history?ctx.total:filtered(ctx).length,activeCount=ctx.history?ctx.activeCount:jobs.filter(active).length;
    const counter=ctx.root.querySelector?.("#clip-jobs-count");if(counter)counter.textContent=`${total} 个任务${activeCount?` · ${activeCount} 个处理中`:""}`;
    const refresh=ctx.root.querySelector?.('[data-action="clip-jobs-refresh"]');if(refresh)refresh.disabled=ctx.reading;
  }
  async function tick(force = false) {
    const ctx = current;
    if (!ctx || !valid(ctx) || !force&&(ctx.reading||ctx.searchTimer||Date.now()-ctx.lastRead<1500)) return;
    clearTimeout(ctx.searchTimer);ctx.searchTimer=null;
    const revision=++ctx.revision,params={...filters,page:ctx.page,limit:30};
    ctx.reading = true; ctx.lastRead = Date.now();
    render(ctx);
    try {
      const response = await bridge("clip_jobs", params);
      if (!valid(ctx)||revision!==ctx.revision) return;
      jobs = Array.isArray(response?.jobs) ? response.jobs : [];
      ctx.history=response?.history===true;
      ctx.total=ctx.history&&Number.isFinite(response.total)?Math.max(0,response.total):filtered(ctx).length;
      ctx.activeCount=ctx.history&&Number.isFinite(response.active_count)?Math.max(0,response.active_count):jobs.filter(active).length;
      ctx.page=ctx.history&&Number.isInteger(response.page)?Math.max(0,response.page):0;
      ctx.limit=ctx.history&&Number.isInteger(response.limit)&&response.limit>0?response.limit:30;
      ctx.hasMore=ctx.history&&response.has_more===true;ctx.stale=false;
      ctx.error = "";
    } catch (error) { if (valid(ctx)&&revision===ctx.revision){ctx.error=String(error?.message||error);ctx.stale=false;} }
    finally { if (valid(ctx)&&revision===ctx.revision) { ctx.reading = false; render(ctx); } }
  }
  async function mount(root = document.querySelector("#clip-jobs")) {
    unmount();
    if (!root) return;
    const ctx = current = {root, tools:null, error:"", reading:false, lastRead:0, pending:new Set(),history:false,total:0,activeCount:0,page:0,limit:30,hasMore:false,revision:0,searchTimer:null,stale:false};
    render(ctx);
    timer = setInterval(() => void tick(), 2000); timer?.unref?.();
    const tools = Promise.resolve().then(() => bridge("clip_tools", {})).then(result => { if (valid(ctx)) { ctx.tools = result; render(ctx); } }).catch(() => {});
    await Promise.all([tick(true), tools]);
  }
  function unmount() { clearInterval(timer);if(current)clearTimeout(current.searchTimer); timer = null; current = null; }
  function changed(ctx,delay=0) {
    clearTimeout(ctx.searchTimer);ctx.searchTimer=null;
    ++ctx.revision;ctx.page=0;ctx.reading=false;ctx.lastRead=0;
    if(ctx.history){jobs=[];ctx.total=0;ctx.stale=true;}
    render(ctx);
    if(delay){ctx.searchTimer=setTimeout(()=>{ctx.searchTimer=null;void tick(true);},delay);ctx.searchTimer?.unref?.();}
    else void tick(true);
  }
  function input(target) {
    const key={"clip-jobs-search":"search","clip-jobs-date":"date","clip-jobs-status":"status"}[target?.id];
    if(!key||!current||!valid(current))return false;
    if(filters[key]===target.value)return true;
    filters[key]=target.value;changed(current,key==="search"?220:0);return true;
  }
  async function action(element) {
    const name = element?.dataset?.action;
    if (!["clip-jobs-refresh","clip-job-cancel","clip-job-retry","clip-job-folder","clip-jobs-clear","clip-download-settings","clip-jobs-previous","clip-jobs-next"].includes(name)) return false;
    const ctx = current;
    if (!ctx || !valid(ctx)) return true;
    if(name==="clip-download-settings"){onConfigure();return true;}
    if(name==="clip-jobs-clear"){
      for(const key of Object.keys(filters))filters[key]="";
      for(const id of ["clip-jobs-search","clip-jobs-date","clip-jobs-status"]){const field=ctx.root.querySelector?.(`#${id}`);if(field)field.value="";}
      clearTimeout(ctx.searchTimer);ctx.searchTimer=null;++ctx.revision;ctx.page=0;ctx.reading=false;ctx.lastRead=0;if(ctx.history){jobs=[];ctx.total=0;ctx.stale=true;}
      await tick(true);return true;
    }
    if(name==="clip-jobs-previous"||name==="clip-jobs-next"){
      if(!ctx.history||ctx.reading||ctx.stale||name==="clip-jobs-previous"&&ctx.page===0||name==="clip-jobs-next"&&!ctx.hasMore)return true;
      ctx.page+=name==="clip-jobs-next"?1:-1;ctx.stale=true;jobs=[];
      await tick(true);return true;
    }
    if (name === "clip-jobs-refresh") { await tick(true); return true; }
    const job = jobs.find(item => item.id === element.dataset.job);
    if (!job || ctx.pending.has(job.id)) return true;
    if (name === "clip-job-folder") {
      if (job.status !== "completed" || !job.output_path) return true;
      try { await invoke("open_clip_folder", {path:job.output_path,jobId:job.id}); } catch (error) { toast(String(error?.message || error),true); }
      return true;
    }
    if (name === "clip-job-cancel" && !active(job) || name === "clip-job-retry" && !["failed","cancelled"].includes(job.status)) return true;
    ctx.pending.add(job.id); render(ctx);
    try {
      await bridge(name === "clip-job-cancel" ? "clip_cancel" : "clip_retry", {job_id:job.id});
      if (valid(ctx)) { ctx.lastRead = 0; await tick(true); }
    } catch (error) { if (valid(ctx)) ctx.error = String(error?.message || error); }
    finally { if (valid(ctx)) { ctx.pending.delete(job.id); render(ctx); } }
    return true;
  }
  return {mount, unmount, input, change:input, action, tick};
}
