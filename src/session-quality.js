import {duration,live} from "./features/monitor/model.js";
import { esc, icon, num } from "./presentation.js";
import {sessionDate} from "./session-date.js";

export const qualityReasons = {normal:"正常互动",self_repeat:"个人刷屏",crowd_repeat:"集中重复",session_repeat:"整场重复",activity_command:"活动口令",activity_uncertain:"口令待确认",empty:"空内容"};
export function qualityTime(seconds) {
  if (!Number.isFinite(seconds) || seconds < 0) return "—";
  const n=Math.floor(seconds);return [Math.floor(n/3600),Math.floor(n/60)%60,n%60].map(v=>String(v).padStart(2,"0")).join(":");
}
export function qualityRate(row) {
  return row.analysis_state === "ready" && Number.isFinite(row.valid_speaker_rate) ? `${(row.valid_speaker_rate*100).toFixed(1)}%` : "—";
}
export function onlineNumber(value){return Number.isFinite(value)&&value>=0?(value>0&&value<.1?"<0.1":new Intl.NumberFormat('zh-CN',{maximumFractionDigits:1}).format(value)):"—";}
export function speakingOnlineRate(row){return row.analysis_state==="ready"&&Number.isFinite(row.valid_speakers_to_online_rate)?`${(row.valid_speakers_to_online_rate*100).toFixed(1)}%`:"—";}
const date = row => sessionDate(row.started_at || row.first_seen,true);
const speakerCount = value => Number.isInteger(value) && value >= 0 ? num(value) : "—";
const status = row => ({analyzing:"统计中",paused:"已暂停",needs_review:"需确认"}[row.analysis_state] || (row.records_cleared ? "记录已清理" : "已统计"));
function episodeCard(ctx,row,index){
  const current=ctx.anchor.session_id===row.session_id,currentLive=current&&live(ctx.anchor),selected=ctx.selected===row.session_id,stamp=date(row);
  const elapsed=currentLive&&Number.isFinite(ctx.anchor.duration_seconds)?duration(ctx.anchor):qualityTime(row.monitor_seconds);
  return `<button id="quality-episode-${esc(ctx.anchorId)}-${esc(row.session_id)}" class="quality-episode ${selected?"selected":""}" data-action="quality-session" data-session="${esc(row.session_id)}" aria-pressed="${selected}" aria-describedby="session-tip-${index}"><span class="episode-label">${currentLive?'<i class="dot"></i>':""}${esc(stamp.slice(0,8))}</span><span class="episode-time">${current?(currentLive?"当前场 · ":"最近场 · "):""}${elapsed}</span><span class="episode-tooltip" id="session-tip-${index}" role="tooltip"><strong>${esc(stamp)}</strong><span class="episode-tooltip-status">${currentLive?"开播中":"已监控 "+elapsed} · ${row.coverage==="partial"?"部分监控":"覆盖未知"}</span><span class="episode-tooltip-grid"><span title="本场发言账号，按账号 ID 去重">发言人数<b>${speakerCount(row.speakers)}</b></span><span title="去除只有无效发言的账号；至少一条有效发言，按账号 ID 去重">有效发言人数<b>${speakerCount(row.valid_speakers)}</b></span><span title="有效发言人数 ÷ 总发言人数，均按本场账号 ID 去重">有效发言占比<b>${qualityRate(row)}</b></span><span title="最高在线人数 / 各已采样分钟的平均在线人数">最高 / 分钟均在线<b>${onlineNumber(row.peak_viewers)} / ${onlineNumber(row.minute_avg_viewers)}</b></span><span title="去重有效发言人数 ÷ 分钟平均在线人数">有效／在线<b>${speakingOnlineRate(row)}</b></span></span><span class="episode-tooltip-status">${esc(status(row))}</span></span></button>`;
}

export function createSessionQuality({state,bridge,invoke,toast,onContext=()=>{},onSession=()=>{},onRender=()=>{}}) {
  let current = null;
  const valid = ctx => current===ctx && state.modal?.kind==="detail" && state.modal.rid===ctx.rid;
  function mount(anchor) {
    current={rid:anchor.rid,anchorId:anchor.anchor_id || "",anchor,month:"",offset:0,data:null,selected:"",detail:null,reason:"",messageOffset:0,
      loading:false,error:"",lastLoad:0,request:0,commandDraft:null,rulesDraft:null,format:"xlsx",sidebarCollapsed:globalThis.matchMedia?.("(max-width: 600px)")?.matches||false,followCurrent:true};
    document.querySelector(".modal")?.classList.add("anchor-detail-dialog");
    render(current);void load(current);
  }
  function render(ctx) {
    const root=document.querySelector("#session-quality");if(!root || !valid(ctx))return;
    const focused=document.activeElement;
    const focusId=root.contains?.(focused) ? focused.id : "";
    const selection=focusId && /TEXTAREA|INPUT/.test(focused.tagName) ? [focused.selectionStart,focused.selectionEnd] : null;
    const scrolls=[".quality-sidebar-content",".quality-episodes",".quality-table-wrap",".quality-messages"].map(selector=>[selector,root.querySelector(selector)?.scrollTop||0]);
    const toolsOpen=root.querySelector(".quality-sidebar-tools")?.open;
    const rulesOpen=root.querySelector(".quality-rules")?.open;
    const evidenceOpen=root.querySelector(".quality-evidence-disclosure")?.open;
    const commandsOpen=root.querySelector(".quality-commands")?.open;
    const count=ctx.data?.total;
    const rows=ctx.data?.sessions || [];
    const selected=ctx.detail?.overview;
    onContext(selected ? date(selected) : "",selected || null);
    const busy=ctx.loading;
    const cfg=ctx.rulesDraft || ctx.data?.rules;
    const applied=ctx.data?.rules;
    const basis=applied?{self_repeat:`同一账号 ${applied.window_seconds} 秒内相同内容至少 ${applied.self_count} 条，对应分组全部排除。`,crowd_repeat:`${applied.window_seconds} 秒内相同内容至少 ${applied.crowd_count} 条，且来自至少 ${applied.crowd_users} 人，对应分组全部排除。`,session_repeat:`整场相同内容至少 ${applied.session_count} 条，且来自至少 ${applied.session_users} 人，整组排除。`,activity_command:"与已确认的本场参与口令相同。",activity_uncertain:"口令缺少可核对的活动时间或消息发送时间，暂不确认有效率。",normal:"未命中重复与口令排除规则的正常互动。",empty:"内容为空，未计为有效发言。"}[ctx.reason]:"";
    document.querySelector("#detail-panel-analysis")?.classList?.toggle?.("episodes-collapsed",ctx.sidebarCollapsed);
    root.innerHTML=`<div class="quality-heading quality-sidebar-heading"><h3>场次 <span>${count==null ? "—" : num(count)}</span></h3><button id="quality-sidebar-toggle" class="quality-sidebar-toggle" data-action="quality-toggle-sidebar" aria-controls="quality-sidebar-content" aria-expanded="${!ctx.sidebarCollapsed}" aria-label="${ctx.sidebarCollapsed?"展开场次列表":"收起场次列表"}" title="${ctx.sidebarCollapsed?"展开场次列表":"收起场次列表"}">${icon("chevron")}</button><span class="quality-sidebar-rail-label" aria-hidden="true">场次</span></div><div class="quality-sidebar-content" id="quality-sidebar-content" ${ctx.sidebarCollapsed?"hidden":""}>
      <details class="quality-sidebar-tools" ${toolsOpen?"open":""}><summary>筛选与导出</summary><div class="quality-tools"><input id="quality-month" type="month" aria-label="按月份筛选场次" value="${esc(ctx.month)}">${ctx.month?'<button class="small" data-action="quality-clear-month">全部月份</button>':""}<select id="quality-format" aria-label="场次分析导出格式"><option value="xlsx" ${ctx.format==="xlsx"?"selected":""}>XLSX</option><option value="csv" ${ctx.format==="csv"?"selected":""}>CSV</option></select><button class="small" data-action="quality-export" ${!count||busy?"disabled":""}>${icon("download")}导出</button></div></details>
      ${ctx.error?`<p class="quality-error" role="alert">${esc(ctx.error)} <button class="small" data-action="quality-refresh">重试</button></p>`:""}
      ${ctx.data?.worker?.retrying?'<p class="help">分析暂时重试中，原始消息持续归档。</p>':""}
      ${!ctx.anchorId?'<p class="quality-empty">等待识别真实主播与直播场次。</p>':!ctx.data?'<p class="quality-empty">正在读取场次…</p>':!rows.length?'<p class="quality-empty">此范围尚无已登记场次。</p>':`<div class="quality-episodes" aria-label="选择直播场次">${rows.map((row,i)=>episodeCard(ctx,row,i)).join("")}</div>`}
      ${count>20?`<div class="quality-pages"><button class="small" data-action="quality-prev" aria-label="上一页场次" ${!ctx.offset||busy?"disabled":""}>${icon("chevron")}</button><span>${Math.floor(ctx.offset/20)+1} / ${Math.ceil(count/20)}</span><button class="small" data-action="quality-next" aria-label="下一页场次" ${ctx.data?.next_offset==null||busy?"disabled":""}>${icon("chevron")}</button></div>`:""}
      ${selected?`<details class="quality-evidence-disclosure" ${evidenceOpen?"open":""}><summary>查看发言明细</summary><div class="quality-evidence"><div class="quality-evidence-heading"><strong>发言统计</strong><button class="small" data-action="quality-rebuild-session">${icon("refresh")}重算本场</button>${selected.job_state==="queued"?'<button class="small" data-action="quality-pause">暂停重算</button>':""}</div>
        <div class="quality-metrics"><span>聊天 <b>${num(selected.chat_messages)}</b></span><span>有效消息 <b>${num(selected.valid_messages)}</b></span><span>待判定 <b>${num(selected.pending)}</b></span><span>身份未知 <b>${num(selected.unknown_sender_messages)}</b></span></div>
        ${selected.analysis_state!=="ready"?`<p class="help">${selected.analysis_state==="paused"?"分析已暂停，点击重算本场可继续。":selected.analysis_state==="needs_review"?"活动口令缺少可核对的时间，相关消息保持待判定；可确认本场口令后重算。":"正在分析已归档消息，当前人数为阶段结果，有效率完成后显示。"}</p>`:""}
        <div class="quality-reason-bar"><select id="quality-reason" aria-label="查看消息分类"><option value="">全部已分析消息</option>${Object.entries(qualityReasons).map(([key,name])=>`<option value="${key}" ${ctx.reason===key?"selected":""}>${name} ${num(ctx.detail.reasons[key]||0)}</option>`).join("")}</select>${!ctx.reason?`<span>${num(ctx.detail.total)} 条</span>`:""}</div>${basis?`<p class="quality-basis">${esc(basis)}</p>`:""}
        <div class="quality-messages">${ctx.detail.messages.length?ctx.detail.messages.map(m=>`<article><div><strong>${esc(m.sender_name||"未提供昵称")}</strong><span class="mono">${esc(m.sender_id||"身份未知")}</span><time>${esc((m.message_time||m.received_at||"").slice(11,19))}${!m.message_time?" · 接收时间":""}</time>${!ctx.reason?`<span class="quality-message-reason">${esc(qualityReasons[m.reason]||m.reason)}</span>`:""}</div><p>${esc(m.content)}</p></article>`).join(""):'<p class="help">此分类暂无消息。</p>'}</div>
        ${ctx.detail.total>30?`<div class="quality-pages"><button class="small" data-action="quality-message-prev" ${!ctx.messageOffset?"disabled":""}>上一页</button><span>${Math.floor(ctx.messageOffset/30)+1} / ${Math.ceil(ctx.detail.total/30)}</span><button class="small" data-action="quality-message-next" ${ctx.detail.next_offset==null?"disabled":""}>下一页</button></div>`:""}
        <details class="quality-commands" ${commandsOpen?"open":""}><summary>本场活动口令</summary><label for="quality-commands">已确认的参与口令，每行一条</label><textarea id="quality-commands" rows="3" placeholder="只填写需要排除的完整口令">${esc(ctx.commandDraft??ctx.detail.commands.join("\n"))}</textarea><button class="small" data-action="quality-save-commands">保存并重算本场</button></details></div></details>`:ctx.selected?'<p class="quality-empty">正在读取本场统计…</p>':""}
      ${cfg?`<details class="quality-rules" ${rulesOpen?"open":""}><summary>本主播统计规则</summary><div class="quality-rule-grid">${[["window_seconds","窗口秒数",5,600],["self_count","个人重复条数",2,20],["crowd_count","集中重复条数",5,200],["crowd_users","集中重复人数",2,100],["session_count","整场重复条数",10,1000],["session_users","整场重复人数",2,500]].map(([key,label,min,max])=>`<label>${label}<input type="number" data-quality-rule="${key}" aria-label="${label}" min="${min}" max="${max}" value="${cfg[key]}"></label>`).join("")}</div><p class="help">正常提问、回答、评论、感谢与反馈均可计入。达到重复阈值时整组排除；新规则对本主播所有已登记场次重新分析。</p><div class="quality-rule-actions"><button class="small" data-action="quality-save-rules">保存并重新分析</button><button class="small" data-action="quality-rebuild" ${!ctx.anchorId||busy?"disabled":""}>${icon("refresh")}重新分析全部场次</button></div></details>`:""}</div>`;
    for(const [selector,scroll] of scrolls){const area=root.querySelector(selector);if(area)area.scrollTop=scroll;}
    if(focusId){const element=root.querySelector(`#${focusId}`);element?.focus({preventScroll:true});if(selection && typeof element?.setSelectionRange==="function")try{element.setSelectionRange(...selection);}catch{ /* numeric/month inputs do not support selection */ }}
    onRender();
  }
  async function load(ctx) {
    if(!valid(ctx)||!ctx.anchorId||ctx.loading)return;
    ctx.loading=true;const token=++ctx.request;ctx.lastLoad=Date.now();
    try {
      const data=await bridge("quality_sessions",{anchor_id:ctx.anchorId,month:ctx.month,limit:20,offset:ctx.offset,current_session_id:ctx.anchor.session_id||""});
      if(!valid(ctx)||token!==ctx.request)return;
      ctx.data=data;ctx.error="";
      if(!ctx.selected&&data.sessions?.length){ctx.selected=data.sessions.find(row=>row.session_id===ctx.anchor.session_id)?.session_id||data.sessions[0].session_id;ctx.followCurrent=ctx.selected===ctx.anchor.session_id;onSession(ctx.selected);}
      if(ctx.selected)await loadDetail(ctx,token);
    } catch(e){if(valid(ctx)&&token===ctx.request)ctx.error=String(e?.message||e);}
    finally{if(valid(ctx)&&token===ctx.request){ctx.loading=false;render(ctx);}}
  }
  async function loadDetail(ctx,token=ctx.request) {
    const selected=ctx.selected;
    const result=await bridge("quality_detail",{anchor_id:ctx.anchorId,session_id:selected,reason:ctx.reason,limit:30,offset:ctx.messageOffset});
    if(!valid(ctx)||token!==ctx.request||selected!==ctx.selected)return;
    ctx.detail=result;
  }
  async function reload(ctx) {ctx.loading=false;ctx.request++;await load(ctx);}
  async function action(el) {
    if(!el.dataset.action?.startsWith("quality-"))return false;
    const ctx=current;if(!ctx||!valid(ctx))return true;
    const a=el.dataset.action;
    try {
      if(a==="quality-session") {
        const selected=el.dataset.session;
        if(ctx.selected===selected)return true;
        ctx.selected=selected;ctx.followCurrent=selected===ctx.anchor.session_id;ctx.detail=null;ctx.reason="";ctx.messageOffset=0;ctx.commandDraft=null;onSession(selected);render(ctx);await reload(ctx);
      } else if(a==="quality-toggle-sidebar"){ctx.sidebarCollapsed=!ctx.sidebarCollapsed;render(ctx);}
      else if(a==="quality-clear-month") {ctx.month="";ctx.offset=0;ctx.selected="";ctx.detail=null;await reload(ctx);}
      else if(a==="quality-prev"||a==="quality-next") {ctx.offset=a==="quality-prev"?Math.max(0,ctx.offset-20):ctx.data.next_offset;await reload(ctx);}
      else if(a==="quality-message-prev"||a==="quality-message-next") {ctx.messageOffset=a==="quality-message-prev"?Math.max(0,ctx.messageOffset-30):ctx.detail.next_offset;await reload(ctx);}
      else if(a==="quality-refresh")await reload(ctx);
      else if(a==="quality-export") {
        const safeName=(ctx.anchor.name||"主播").replace(/[<>:"/\\|?*\u0000-\u001f]/g,"_").slice(0,80);
        const path=state.preview?`演示导出/场次分析-${safeName}.${ctx.format}`:await invoke("choose_export_path",{format:ctx.format,defaultName:`场次分析-${safeName}-${ctx.month||"全部"}.${ctx.format}`});
        if(path){await bridge("quality_export",{anchor_id:ctx.anchorId,month:ctx.month,format:ctx.format,output_path:path});toast("场次分析导出已开始");}
      } else {
        let method,params={anchor_id:ctx.anchorId};
        if(a==="quality-rebuild")method="quality_rebuild";
        else if(a==="quality-rebuild-session"){method="quality_rebuild";params.session_id=ctx.selected;}
        else if(a==="quality-pause"){method="quality_pause";params.session_id=ctx.selected;}
        else if(a==="quality-save-commands"){method="quality_commands";params.session_id=ctx.selected;params.commands=(ctx.commandDraft??ctx.detail.commands.join("\n")).split(/\r?\n/).map(v=>v.trim()).filter(Boolean);}
        else if(a==="quality-save-rules"){method="quality_configure";params.rules=ctx.rulesDraft||ctx.data.rules;}
        if(method){await bridge(method,params);if(!valid(ctx))return true;ctx.rulesDraft=null;ctx.commandDraft=null;await reload(ctx);toast(a==="quality-pause"?"已暂停本场重算":"已排入后台分析");}
      }
    }catch(e){toast(String(e?.message||e),true);}
    return true;
  }
  function input(target) {
    const ctx=current;if(!ctx||!valid(ctx))return;
    if(target.dataset.qualityRule){ctx.rulesDraft={...(ctx.rulesDraft||ctx.data.rules),[target.dataset.qualityRule]:Number(target.value)};}
    if(target.id==="quality-commands")ctx.commandDraft=target.value;
  }
  async function change(target) {
    const ctx=current;if(!ctx||!valid(ctx))return;
    if(target.id==="quality-format")ctx.format=target.value;
    if(target.id==="quality-month"){ctx.month=target.value;ctx.offset=0;ctx.selected="";ctx.detail=null;await reload(ctx);}
    if(target.id==="quality-reason"){ctx.reason=target.value;ctx.messageOffset=0;await reload(ctx);}
  }
  function tick(anchor) {
    const ctx=current;if(!ctx||!valid(ctx))return;
    // Identity changes invalidate every in-flight list/detail response and draft.
    // Do this before following a restored session, including empty -> known IDs.
    if(ctx.anchorId!==(anchor.anchor_id||"")){mount(anchor);return;}
    const previousSession=ctx.anchor.session_id,previousLive=live(ctx.anchor);ctx.anchor=anchor;
    if(ctx.followCurrent&&anchor.session_id&&previousSession!==anchor.session_id){
      ctx.selected=anchor.session_id;ctx.detail=null;ctx.month="";ctx.offset=0;ctx.reason="";ctx.messageOffset=0;ctx.commandDraft=null;
      onSession(anchor.session_id);void reload(ctx);return;
    }
    const editing=document.activeElement?.closest("#session-quality") && /INPUT|TEXTAREA|SELECT/.test(document.activeElement.tagName);
    if(!editing&&previousLive!==live(anchor))render(ctx);
    if(!editing && !ctx.loading && Date.now()-ctx.lastLoad>3000)void load(ctx);
  }
  return {mount,tick,action,change,input};
}
