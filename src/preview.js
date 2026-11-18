// Development-only fixtures. Never selected in a packaged build.
import {DEFAULT_AI_BASE_URL} from "./ai-defaults.js";
import {version as appVersion} from "../package.json";
import {createPreviewMarks} from './preview-marks.js';
import {createPreviewDownloads} from './preview-downloads.js';
import {normalizeDisplayTimezone,DEFAULT_DISPLAY_TIMEZONE} from './time-display.js';
import {normalizeTags} from './anchor-tags.js';
const names = [
  "测试主播甲",
  "测试主播乙",
  "测试主播丙",
  "林间晚风",
  "阿梨的日常",
  "周末音乐角",
  "小鹿同学",
  "橙子慢生活",
  "西西的花园",
  "鹿鸣工作室",
  "一颗小青柠",
  "山海之间",
  "小满茶馆",
  "星野放映室",
  "南风过境",
  "桃子汽水",
  "云边书屋",
  "陈陈说生活",
  "春日来信",
  "拾光漫步",
  "青禾小铺",
  "元气小雨",
  "白日梦想家",
  "小岛来客",
];
const anchors = Array.from({ length: 300 }, (_, i) => ({
  rid: String(100000000000n + BigInt(i)),
  name:
    i < names.length
      ? names[i]
      : `${names[i % names.length]} ${Math.floor(i / names.length) + 1}`,
  anchor_id: String(100000003000n + BigInt(i)),
  session_id: i < 148 ? String(7696456021958609673n + BigInt(i)) : "",
  state:
    i < 148
      ? "monitoring"
      : i < 284
        ? "offline"
        : i < 296
          ? "stopped"
          : "error",
  active: i < 284,
  is_live: i < 148 ? true : i < 284 ? false : null,
  broadcast_started_at: i < 148 ? Math.floor(Date.now() / 1000) - 3670 - i * 89 : null,
  duration_seconds: i < 148 ? 3670 + i * 89 : null,
  duration_running: i < 148,
  duration_updated_at: Date.now() / 1000,
  detail:
    i < 148
      ? "连接正常 · 消息持续归档"
      : i < 284
        ? "未开播，等待下次查询"
        : i < 296
          ? "监控已停止"
          : "连接失败，等待重试",
  message_count: i < 148 ? (i + 1) * 137 : 0,
  last_message_at: i < 148 ? "2026-10-14T16:42:08+08:00" : "",
  live_checked_at: "2026-10-14T16:42:00+08:00",
  viewer_count: i < 148 ? 673 + i * 371 : null,
  activities: i < 148 ? {
    red_packet:{total:Math.max(i % 4, i % 3 === 0 ? 1 : 0),active_until:i % 3 === 0 ? Math.floor(Date.now()/1000)+300 : null,uncertain:i % 5 === 0},
    lottery:{total:Math.max(i % 5, i % 3 === 1 ? 1 : 0),active_until:i % 3 === 1 ? Math.floor(Date.now()/1000)+180 : null,uncertain:false},
  } : null,
  cookie_configured: false,
}));
Object.assign(anchors[0], { pc_unsupported: true, pc_support_source: "manual", is_live: null, state: "retrying", detail: "直播状态待确认；浏览器已手动标记非 PC 支持" });
const tagsByAnchor = new Map([[anchors[0].anchor_id, ["重点关注", "游戏"]], [anchors[1].anchor_id, ["娱乐"]]]);
const tagsByRoom = new Map();
const previewTags = anchor => [...(anchor?.anchor_id ? tagsByAnchor.get(anchor.anchor_id) || [] : tagsByRoom.get(anchor?.rid) || [])];
let archiveSessions = anchors.slice(0, 148).flatMap(a => [0, 1, 2].map(i => ({
  anchor_id: a.anchor_id, anchor_name: a.name, web_rid: a.rid, session_id: String(BigInt(a.session_id) + BigInt(i) * 1000n),
  started_at: `2026-10-${String(i + 10).padStart(2, "0")}T12:00:00+08:00`, records_count: 100 + i * 10,
})));
const deleteJobs = new Map();
const matchingSessions = p => archiveSessions.filter(s => p.anchor_ids.includes(s.anchor_id)
  && (!p.month || s.started_at.startsWith(p.month)) && (!p.session_ids || p.session_ids.includes(s.session_id)));
let proxyEnabled = false;
let debugLogging = false;
let proxySource = 'public';
let proxyIncludeDirect = true;
let subscriptions = [];
let importedProxies = [];
function previewProxyInput(p) {
  let rows;
  try { rows = JSON.parse(p.text); } catch { rows = p.text.trim().split(/\n+/).map(s => {
    const [ip, port, username, password] = s.trim().split(':');
    return { ip, port, username, password };
  }); }
  if (!Array.isArray(rows)) throw new Error('请提供 JSON 数组或代理列表');
  let duplicates = 0;
  const items = [], errors = [], seen = new Set(importedProxies.map(p => p.endpoint));
  for (const [index, r] of rows.entries()) {
    if (!r || !/^[\d.]+$/.test(r.ip || '') || !Number.isInteger(Number(r.port)) || Number(r.port) < 1 || Number(r.port) > 65535) {
      errors.push({line:index + 1,message:'地址或端口无效'}); continue;
    }
    const endpoint = `${r.ip}:${r.port}`;
    if (seen.has(endpoint)) { duplicates++; continue; }
    seen.add(endpoint);
    items.push({id:`fixture-${endpoint}`,endpoint,protocol:(r.protocol || p.default_protocol || 'http').startsWith('socks5') ? 'socks5h' : 'http',auth_present:Boolean(r.username && r.password),status:'unchecked'});
  }
  return {total:rows.length,valid:items.length,added:items.length,updated:0,duplicates,invalid:errors.length,items,errors};
}
let proxyBatchStartedAt = 0;
const previewBatch = () => {
  const checked = proxyBatchStartedAt ? Math.min(50, Math.floor((Date.now() - proxyBatchStartedAt) / 160)) : 0;
  const active = Boolean(proxyBatchStartedAt && checked < 50);
  const passed = Math.floor(checked / 10);
  return { active, target: 50, checked, passed, failed: checked - passed,
    state: !proxyBatchStartedAt ? 'idle' : active ? 'running' : 'completed', detail: '' };
};
if (new URLSearchParams(location.search).get('scene') === 'mixed') {
  anchors.forEach((a, i) => {
    a.active = true;
    a.is_live = i < 8;
    a.state = i < 8 ? 'monitoring' : 'offline';
    a.detail = i < 8 ? '连接正常' : '未开播';
    a.message_count = i < 8 ? (i + 1) * 137 : 0;
    a.session_id = i < 8 ? String(7696456021958609673n + BigInt(i)) : '';
    a.last_message_at = i < 8 ? '2026-10-14T16:42:08+08:00' : '';
  });
}
const demoRoutes = ['direct', '192.0.3.01:8080', '192.0.3.02:8080', '192.0.3.03:8080', '192.0.3.04:8080'];
anchors.forEach(a => { if (a.active) a.proxy = 'direct'; });
const proxySnapshot = () => {
  const leases=anchors.filter(a=>a.active).map((a,i)=>{
    const endpoint=proxyEnabled?demoRoutes[(i%(demoRoutes.length-(proxyIncludeDirect?0:1)))+(proxyIncludeDirect?0:1)]:'direct';
    return {rid:a.rid,endpoint,kind:endpoint==='direct'?'direct':'proxy',latency_ms:endpoint==='direct'?0:128};
  });
  const routes=(proxyEnabled?demoRoutes.filter(endpoint=>proxyIncludeDirect||endpoint!=='direct'):['direct']).map(endpoint=>({route_id:endpoint,endpoint,kind:endpoint==='direct'?'direct':'proxy',assigned_tasks:leases.filter(l=>l.endpoint===endpoint).length,connecting:0,request_inflight:0,state:'ready',cooldown_seconds:0}));
  return {enabled:proxyEnabled,include_direct:proxyIncludeDirect,refresh:previewBatch(),available:proxyEnabled?4:0,assigned:leases.length,checking:0,failed:0,total:routes.length,last_error:'',subscriptions,routes,leases};
};
let seq = 0;
let jobs = [];
const previewMarks=createPreviewMarks({getAnchor:p=>anchors.find(anchor=>anchor.anchor_id===p.anchor_id)});
const previewDownloads=createPreviewDownloads({getAnchor:p=>anchors.find(anchor=>(!p.anchor_id||anchor.anchor_id===p.anchor_id)&&anchor.session_id===p.session_id),getMark:id=>previewMarks.get(id)});
let emitted = false;
let viewerRefreshStatus={id:"0",active:false,total:0,checked:0,updated:0,failed:0,skipped:0};
const events = Array.from({ length: 28 }, (_, i) => ({
  seq: ++seq,
  kind: "message",
  rid: anchors[i % 8].rid,
  name: anchors[i % 8].name,
  session_id: anchors[i % 8].session_id,
  msg_type: ["chat", "chat", "like", "member"][i % 4],
  sender_id: String(8301694728225720001n + BigInt(i)),
  sender_name: ["小麦", "山风", "橙子", "星星"][i % 4],
  content: [
    "这一段分享很有帮助，已经记下啦",
    "今天的内容很精彩",
    "为主播点赞",
    "进入直播间",
  ][i % 4],
  time_str: `16:42:${String(i).padStart(2, "0")}`,
}));
// Synthetic activity fixtures for UI verification, not captured network messages.
for (const [i, activity] of [
  { activity_kind: "red_packet", content: "红包通知 · 周末小红包 · 红包钻石数：1000" },
  { activity_kind: "lottery", content: "福袋通知 · 奖品：定制帆布袋 · 中奖名额：10 · 参与条件：发送评论「周末快乐」 · 开奖时间：2026-10-15 16:50:00" },
].entries()) {
  events.push({ seq: ++seq, kind: "message", rid: anchors[1].rid, name: anchors[1].name,
    session_id: anchors[1].session_id, msg_type: "activity", sender_id: "", sender_name: "",
    content: activity.content, time_str: `16:43:0${i}`, payload: { ...activity, activity_stage: "notice" } });
}
const snapshot = (includeProxy = true) => ({
  version: appVersion,
  anchors: anchors.map(anchor => ({...anchor,tags:previewTags(anchor)})),
  archive_anchors: [...new Map(archiveSessions.map(s => [s.anchor_id, {
    anchor_id: s.anchor_id, anchor_name: s.anchor_name, web_rid: s.web_rid, tags: previewTags(s),
  }])).values()],
  settings: { cookie_configured: false, proxy_enabled: proxyEnabled, proxy_include_direct: proxyIncludeDirect, proxy_source: proxySource, imported_proxy_count: importedProxies.length, debug_logging: debugLogging },
  metrics: {
    total_messages: anchors.reduce((n, a) => n + a.message_count, 0),
    process_rss_mb: 55.2,
    thread_count: 305,
    archive_queue: 0,
    viewer_refresh: viewerRefreshStatus,
    ...(includeProxy ? {proxy: proxySnapshot()} : {}),
  },
  export_jobs: jobs,
  cursor: seq,
});
let startupMonitoring=true;
let communitySettings={enabled:false,key_configured:false};
let highlightSettings={base_url:DEFAULT_AI_BASE_URL,model:"",key_configured:false};
const highlightJobs=new Map();
function previewTimeline(p){
  const anchor=anchors.find(a=>a.anchor_id===p.anchor_id)||anchors[1];
  const current=anchor.session_id===p.session_id;
  const index=current?0:Math.max(1,Math.min(24,Number(BigInt(p.session_id)-7000000000000000000n)));
  const start=current?anchor.broadcast_started_at:Date.UTC(2026,9,23-index,4)/1000;
  const duration=current?Math.floor((anchor.duration_seconds||2675)+Math.max(0,Date.now()/1000-anchor.duration_updated_at)):3600+index*133;
  const end=start+duration;
  const lines=["这个新手也能操作吗？","刚才这个走位太漂亮了","谢谢讲解，原来还可以这样","哈哈哈，差一点就成功了","主播能再演示一次吗？","看懂了，这次我也试试","这波配合满分！","原来要先等队友到齐","前面提问的朋友，看这里","这一段真的笑到了"];
  const messages=[];const buckets=[];
  for(let minute=0;minute<=Math.floor(duration/60);minute++){
    const count=Math.round(3+Math.abs(Math.sin(minute*.56))*8+(minute%19===8?24:0)+(minute%23===12?13:0));
    let actual=0;
    for(let index=0;index<count;index++){
      const offset=minute*60+index*60/count;if(offset>duration)break;actual++;
      messages.push({id:String(messages.length+1),offset_seconds:offset,sender_name:["橘子","小云","晚风","阿圆","夏天"][index%5],content:lines[(minute+index)%lines.length],message_type:"chat",time_source:"server"});
    }
    buckets.push({offset_seconds:minute*60,count:actual});
  }
  const chatCount=messages.length;
  const activityEnd=current ? (anchor.duration_seconds || duration) : duration;
  const activities=[
    {kind:"lottery",offset_seconds:Math.max(0,activityEnd-26),details:{prize_name:"定制帆布袋",conditions:[{content:"周末快乐"}]},content:"福袋 · 定制帆布袋",time_source:"server"},
    {kind:"red_packet",offset_seconds:Math.max(0,activityEnd-25),details:{title:"周末小红包",diamond_count:1000},content:"红包 · 周末小红包",time_source:"server"},
    {kind:"lottery",offset_seconds:Math.floor(activityEnd*.3),details:{prize_name:"游戏周边",conditions:[{content:"一起出发"}]},content:"福袋 · 游戏周边",time_source:"observed"},
  ].map((item,index)=>({...item,id:String(9000001+index),activity_id:`fixture-${p.session_id}-${index}`,epoch:start+item.offset_seconds,observed_epoch:start+item.offset_seconds,draw_at:item.kind==="lottery"?start+item.offset_seconds+180:null,expires_at:item.kind==="red_packet"?start+item.offset_seconds+180:null,ended:start+item.offset_seconds+180<Date.now()/1000}));
  for(const item of activities)messages.push({id:item.id,message_type:"activity",content:item.content,sender_name:"直播间活动",time_source:item.time_source==="server"?"server":"missing",offset_seconds:item.time_source==="server"?item.offset_seconds:null,observed_offset_seconds:item.offset_seconds,payload:{activity_kind:item.kind,activity_id:item.activity_id,activity_stage:"notice",activity_details:item.details}});
  return {anchor_id:p.anchor_id,session_id:p.session_id,start_epoch:start,end_epoch:end,duration_seconds:duration,bucket_seconds:60,buckets,messages,activities,activity_count:activities.length,unpositioned_activity_count:0,unconfirmed_activity_count:0,online:[],summary:{coverage:"partial",anchor_name:anchor.name},total_messages:messages.length,chat_count:chatCount,complete:true};
}
export async function previewBridge(method, p = {}) {
  if(method==='time_display_settings'){let timezone=DEFAULT_DISPLAY_TIMEZONE;try{timezone=normalizeDisplayTimezone(localStorage.getItem('qingkan.preview.timezone.v1')||timezone);}catch{}return {timezone};}
  if(method==='time_display_save'){const timezone=normalizeDisplayTimezone(p.timezone);localStorage.setItem('qingkan.preview.timezone.v1',timezone);return {timezone};}
  const markResult=previewMarks.handle(method,p);if(markResult!==undefined)return markResult;
  const downloadResult=await previewDownloads.handle(method,p);if(downloadResult!==undefined)return downloadResult;
  await new Promise((r) => setTimeout(r, 30));
  if(method==="startup_settings")return {startup_monitoring:startupMonitoring};
  if(method==="community_settings")return {...communitySettings};
  if(method==="community_configure"){
    if(typeof p.enabled!=="boolean")throw Error("社区生态设置无效");
    const key=p.clear_key?false:Boolean(p.api_key)||communitySettings.key_configured;
    if(p.enabled&&!key)throw Error("请输入社区生态 KEY");
    communitySettings={enabled:p.enabled,key_configured:key};return {...communitySettings};
  }
  if(method==="session_replay"){
    if(import.meta.env.DEV&&new URLSearchParams(location.search).has("media-fixture")){
      const response=await (await fetch("/build/replay-fixture.json")).json();
      return response.replay?.media_kind==="hls"?{...response,replay:{...response.replay,time_mapping:[],time_mapping_pending:true}}:response;
    }
    return {enabled:communitySettings.enabled,available:false,replay:null};
  }
  if(method==="session_replay_timing"){
    if(import.meta.env.DEV&&new URLSearchParams(location.search).has("media-fixture")){
      const response=await (await fetch("/build/replay-fixture.json")).json();
      if(response.replay?.play_url!==p.play_url)throw Error("录播地址已更新，请重试");
      return {play_url:p.play_url,time_mapping:response.replay.time_mapping||[],playlist_duration_ms:response.replay.playlist_duration_ms??response.replay.duration_ms,time_mapping_pending:false};
    }
    throw Error("暂无可对齐的录播时间轴");
  }
  if(method==="configure_startup") { if(typeof p.startup_monitoring!=="boolean")throw Error("启动监听设置无效"); startupMonitoring=p.startup_monitoring; return {startup_monitoring:startupMonitoring}; }
  if(method==="save_settings") {
    if(typeof p.debug_logging!=="boolean")throw Error("调试日志设置无效");
    debugLogging=p.debug_logging;
    return snapshot().settings;
  }
  if (method === "init") return snapshot();
  if(method==="session_timeline")return previewTimeline(p);
  if(method==="highlight_settings")return {...highlightSettings};
  if(method==="highlight_configure"){
    highlightSettings={base_url:p.base_url,model:p.model||"",key_configured:p.clear_key?false:Boolean(p.api_key)||highlightSettings.key_configured};return {...highlightSettings};
  }
  if(method==="highlight_models")return {models:[{id:"demo-interaction-model"},{id:"demo-reasoning-model"}],model:highlightSettings.model};
  if(method==="highlight_analyze"){
    const data=previewTimeline(p);const job_id=`preview-${Date.now()}`;
    const highlights=[.2,.48,.76].map((fraction,index)=>{const message=data.messages[Math.floor(data.messages.length*fraction)];return {offset_seconds:message.offset_seconds,title:["观众提问引出操作讲解","连续配合引发讨论","轻松失误带来笑点"][index],reason:["多位观众连续追问操作方式，出现相关回应。","聊天从单句喝彩转为讨论配合细节。","多位观众围绕同一失误接话，形成有趣讨论。"][index],evidence:[{...message,message_id:message.id}]};});
    highlightJobs.set(job_id,{job_id,status:"completed",anchor_id:p.anchor_id,session_id:p.session_id,result:{highlights,summary:"观众提问、配合讨论和轻松失误形成三段可回看的互动。",model:highlightSettings.model,analyzed_messages:data.total_messages,snapshot_end_epoch:data.end_epoch}});
    return {job_id,status:"running"};
  }
  if(method==="highlight_status")return highlightJobs.get(p.job_id)||{job_id:p.job_id,status:"cancelled"};
  if(method==="highlight_cancel"){highlightJobs.delete(p.job_id);return {job_id:p.job_id,status:"cancelled"};}
  if (method === "refresh_viewers") {
    const items=anchors.filter(a=>a.active&&a.is_live);
    viewerRefreshStatus={id:String(Date.now()),active:false,total:items.length,checked:items.length,updated:items.length,failed:0,skipped:0};
    for(const a of items){a.viewer_count=(a.viewer_count||0)+17;a.viewer_updated_at=new Date().toISOString();}
    return {accepted:true,...viewerRefreshStatus};
  }
  if (method === "viewer_refresh_status")return viewerRefreshStatus;
  if (method === "poll") {
    const ev = emitted ? [] : [...events];
    emitted = true;
    if(debugLogging)ev.push({seq:++seq,kind:"status",rid:anchors[148].rid,name:anchors[148].name,state:"offline",detail:"尚未开播，等待下次查询",timestamp:new Date().toISOString()});
    return { ...snapshot(p.include_proxy !== false), events: ev };
  }
  if (method === "quality_sessions") {
    const anchor=anchors.find(a=>a.anchor_id===p.anchor_id);
    const sessions=Array.from({length:25},(_,i)=>({anchor_id:p.anchor_id,session_id:i===0?anchor.session_id:String(7000000000000000000n+BigInt(i)),anchor_name:anchor.name,started_at:i===0?new Date(anchor.broadcast_started_at*1000).toISOString():`${new Date(Date.UTC(2026,9,23-i)).toISOString().slice(0,10)}T12:00:00+08:00`,first_seen:"2026-10-23T12:30:00+08:00",coverage:i<2?"partial":"unknown",monitor_seconds:3600+i*133,speakers:i===2?0:320+i*7,valid_speakers:i===2?0:168+i*3,valid_speaker_rate:i===2?null:(168+i*3)/(320+i*7),analysis_state:i===1?"analyzing":"ready",chat_messages:1000+i*20,valid_messages:600,unknown_sender_messages:0,pending:i===1?40:0,job_state:i===1?"queued":"",generation:1,records_cleared:false,peak_viewers:i===2?null:1280+i*20,minute_avg_viewers:i===2?null:840+i*15,audience_sample_minutes:i===2?0:60+i,valid_speakers_to_online_rate:i===2?null:(168+i*3)/(840+i*15)})).filter(row=>!p.month||row.started_at.startsWith(p.month));
    const offset=p.offset||0,limit=p.limit||20;
    return {sessions:sessions.slice(offset,offset+limit),total:sessions.length,next_offset:offset+limit<sessions.length?offset+limit:null,rules:{window_seconds:60,self_count:3,crowd_count:20,crowd_users:10,session_count:50,session_users:20},worker:{available:true,retrying:false}};
  }
  if (method === "quality_detail") {
    const data=await previewBridge("quality_sessions",{anchor_id:p.anchor_id,limit:100});
    const overview=data.sessions.find(s=>s.session_id===p.session_id);
    const messages=Array.from({length:38},(_,i)=>({id:i,sender_id:String(88000000000n+BigInt(i)),sender_name:["小云","橘子","晚风"][i%3],content:["这个搭配适合新手吗？","谢谢讲解，已经了解了","今晚的内容很实用","领福利"][i%4],reason:i%4===3?"crowd_repeat":"normal",message_time:`2026-10-16T12:40:${String(i).padStart(2,"0")}+08:00`})).filter(m=>!p.reason||p.reason===m.reason);
    const offset=p.offset||0,limit=p.limit||30;
    return {overview,reasons:{normal:29,crowd_repeat:9},messages:messages.slice(offset,offset+limit),total:messages.length,next_offset:offset+limit<messages.length?offset+limit:null,commands:[]};
  }
  if (method.startsWith("quality_"))return {queued:1};
  if (method === "recent_messages") {
    return {high_watermark:String(seq),messages:events.filter(e=>e.kind==="message" && (p.type&&p.type!=="all"||(e.msg_type||e.message_type)!=="member") && (!p.before_id||BigInt(e.seq)<=BigInt(p.before_id))&&(!p.type||p.type==="all"||(e.msg_type||e.message_type)===p.type)&&(!p.web_rid||e.rid===p.web_rid)).slice(-(p.limit||150)).reverse().map(e=>({...e,archive_id:String(e.seq)})),source:"archive"};
  }
  if (method === "archive_anchors") return snapshot().archive_anchors;
  if (method === "archive_sessions") {
    const items = matchingSessions(p), offset = p.offset || 0, limit = p.limit || 200;
    return { sessions: items.slice(offset, offset + limit), total: items.length, next_offset: offset + limit < items.length ? offset + limit : null };
  }
  if (method === "preview_archive_delete" || method === "execute_archive_delete") {
    const selected = matchingSessions(p), count = selected.reduce((sum, item) => sum + item.records_count, 0);
    const ready = !anchors.some(a => p.anchor_ids.includes(a.anchor_id) && a.active);
    if (method === "preview_archive_delete") return { ...p, records_count: count, sessions_count: selected.length, ready,
      blocked_reason: ready ? "" : "请先停止所选主播的监控，再删除聊天记录" };
    if (!ready) throw new Error("请先停止所选主播的监控");
    archiveSessions = archiveSessions.filter(item => !selected.includes(item));
    const job_id = crypto.randomUUID();
    deleteJobs.set(job_id, {status: "completed", report: { deleted_records: count, deleted_sessions: selected.length, remaining_records: archiveSessions.reduce((sum, item) => sum + item.records_count, 0) }});
    return {job_id, accepted: true};
  }
  if (method === "archive_delete_status") return deleteJobs.get(p.job_id);
  if (method === "set_pc_unsupported") {
    const item = anchors.find(a => a.rid === p.rid);
    if (item) Object.assign(item, { pc_unsupported: p.unsupported, pc_support_source: p.unsupported ? "manual" : "" });
    return { saved: true };
  }
  if (method === 'proxy_status') return {...proxySnapshot(),source:proxySource};
  if (method === 'configure_proxy') {
    if (anchors.some(a => a.active)) throw new Error('请先停止全部监控，再修改代理设置');
    proxyEnabled = Boolean(p.enabled); proxySource = p.source || proxySource;
    if (p.include_direct !== undefined) proxyIncludeDirect = p.include_direct;
    return {enabled:proxyEnabled,source:proxySource};
  }
  if (method === 'preview_proxies') return previewProxyInput(p);
  if (method === 'add_proxy_subscription') {
    const url = new URL(p.url);
    if (url.protocol !== 'https:' || url.username || url.password) throw new Error('请使用 HTTPS 订阅地址');
    const item = {id:crypto.randomUUID().replaceAll('-', ''),label:url.hostname,interval_minutes:p.interval_minutes || 60,count:500,state:'ready',error:'',updated_at:new Date().toISOString()};
    subscriptions.push(item);
    return {id:item.id,accepted:true};
  }
  if (method === 'refresh_proxy_subscription') {
    const item = subscriptions.find(s => s.id === p.id);
    if (item) item.updated_at = new Date().toISOString();
    return {accepted:true};
  }
  if (method === 'remove_proxy_subscription') {
    subscriptions = subscriptions.filter(s => s.id !== p.id);
    return {removed:1};
  }
  if (method === 'import_proxies') {
    if (proxyEnabled && proxySource === 'imported' && anchors.some(a => a.active)) throw new Error('请先停止全部监控，再修改当前代理来源');
    const result = previewProxyInput(p); importedProxies.push(...result.items);
    return {...result,added:result.valid};
  }
  if (method === 'list_imported_proxies') return {items:importedProxies,total:importedProxies.length,source:proxySource};
  if (method === 'remove_imported_proxies') {
    const before = importedProxies.length;
    importedProxies = importedProxies.filter(pxy => !p.ids.includes(pxy.id));
    return {removed:before-importedProxies.length};
  }
  if (method === 'refresh_proxies') {
    if (previewBatch().active) return {accepted:false,refresh:previewBatch()};
    proxyBatchStartedAt = Date.now();
    return {accepted:true,refresh:previewBatch()};
  }
  if (method === "add_anchors") {
    if (p.tags !== undefined && !Array.isArray(p.tags)) throw Error("标签格式无效，请提供标签列表");
    const tags = p.tags === undefined ? [] : normalizeTags(p.tags);
    if (typeof p.values !== "string") throw Error("请输入直播间链接或房间号");
    let added = 0;
    for (const value of p.values.split(/\s+/)) {
      const rid = value.match(/\d{5,}/)?.[0];
      if (rid && !anchors.some((a) => a.rid === rid)) {
        anchors.push({
          rid,
          name: rid,
          state: "stopped",
          active: false,
          is_live: null,
          message_count: 0,
          detail: "尚未启动监控",
        });
        tagsByRoom.set(rid, [...tags]);
        added++;
      }
    }
    return { added };
  }
  if (method === "set_anchor_tags") {
    if (!Array.isArray(p.tags)) throw Error("标签格式无效，请提供标签列表");
    const tags = normalizeTags(p.tags);
    if (!Array.isArray(p.rids) || !p.rids.length || p.rids.some(rid => typeof rid !== "string" || !rid)) throw Error("请先选择主播");
    const rids = new Set(p.rids), selected = anchors.filter(anchor => rids.has(anchor.rid));
    if (selected.length !== rids.size) throw Error("主播不存在，请刷新后重试");
    for (const anchor of selected) (anchor.anchor_id ? tagsByAnchor : tagsByRoom).set(anchor.anchor_id || anchor.rid, [...tags]);
    return { saved: true, updated: selected.length, tags: [...tags] };
  }
  if (method === "start" || method === "stop") {
    anchors
      .filter((a) => p.rids.includes(a.rid))
      .forEach((a) => {
        a.active = method === "start";
        a.state = a.active ? "connecting" : "stopped";
        a.is_live = null;
        a.detail = a.active ? "演示：等待查询" : "监控已停止";
      });
    return {};
  }
  if (method === "remove") {
    for (let i = anchors.length - 1; i >= 0; i--)
      if (p.rids.includes(anchors[i].rid)) anchors.splice(i, 1);
    return {};
  }
  if (method === "rename") {
    const a = anchors.find((a) => a.rid === p.rid);
    if (a) a.name = p.name;
    return {};
  }
  if (method === "export") {
    const id = String(Date.now());
    jobs.push({
      id,
      status: "completed",
      report: {
        row_count: 9286,
        month: p.month,
        format: p.format,
        path: p.output_path,
      },
    });
    return { id };
  }
  return {};
}
