// Development-only fixtures. Never selected in a packaged build.
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
  cookie_configured: false,
}));
Object.assign(anchors[0], { pc_unsupported: true, pc_support_source: "manual", is_live: null, state: "retrying", detail: "直播状态待确认；浏览器已手动标记非 PC 支持" });
let archiveSessions = anchors.slice(0, 148).flatMap(a => [0, 1, 2].map(i => ({
  anchor_id: a.anchor_id, anchor_name: a.name, web_rid: a.rid, session_id: String(BigInt(a.session_id) + BigInt(i) * 1000n),
  started_at: `2026-10-${String(i + 10).padStart(2, "0")}T12:00:00+08:00`, records_count: 100 + i * 10,
})));
const deleteJobs = new Map();
const matchingSessions = p => archiveSessions.filter(s => p.anchor_ids.includes(s.anchor_id)
  && (!p.month || s.started_at.startsWith(p.month)) && (!p.session_ids || p.session_ids.includes(s.session_id)));
let proxyEnabled = true;
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
const demoRoutes = ['direct', '192.0.2.41:8080', '192.0.2.42:8080', '192.0.2.43:8080', '192.0.2.44:8080'];
anchors.forEach((a, i) => { if (a.active) a.proxy = demoRoutes[i % demoRoutes.length]; });
const proxySnapshot = () => ({ enabled: proxyEnabled, include_direct: proxyIncludeDirect, refresh: previewBatch(), available: 4, assigned: anchors.filter(a => a.proxy).length, checking: 0, failed: 1, total: 5, last_error: '', subscriptions,
  routes: demoRoutes.map((endpoint, i) => ({ route_id: endpoint, endpoint, kind: i ? 'proxy' : 'direct', assigned_tasks: anchors.filter(a => a.proxy === endpoint).length, connecting: i === 1 ? 1 : 0, request_inflight: 0, state: i === 4 ? 'cooling' : 'ready', cooldown_seconds: i === 4 ? 42 : 0 })),
  leases: anchors.filter(a => a.proxy).map(a => ({rid:a.rid, endpoint:a.proxy, kind:a.proxy === 'direct' ? 'direct' : 'proxy', latency_ms:a.proxy === 'direct' ? 0 : 128})) });
let seq = 0;
let jobs = [];
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
const snapshot = () => ({
  version: "0.2.4",
  anchors: [...anchors],
  archive_anchors: [...new Map(archiveSessions.map(s => [s.anchor_id, {
    anchor_id: s.anchor_id, anchor_name: s.anchor_name, web_rid: s.web_rid,
  }])).values()],
  settings: { cookie_configured: false, proxy_enabled: proxyEnabled, proxy_include_direct: proxyIncludeDirect, proxy_source: proxySource, imported_proxy_count: importedProxies.length },
  metrics: {
    total_messages: anchors.reduce((n, a) => n + a.message_count, 0),
    process_rss_mb: 55.2,
    thread_count: 305,
    archive_queue: 0,
    viewer_refresh: viewerRefreshStatus,
    proxy: proxySnapshot(),
  },
  export_jobs: jobs,
  cursor: seq,
});
export async function previewBridge(method, p = {}) {
  await new Promise((r) => setTimeout(r, 30));
  if (method === "init") return snapshot();
  if (method === "refresh_viewers") {
    const items=anchors.filter(a=>a.active&&a.is_live);
    viewerRefreshStatus={id:String(Date.now()),active:false,total:items.length,checked:items.length,updated:items.length,failed:0,skipped:0};
    for(const a of items){a.viewer_count=(a.viewer_count||0)+17;a.viewer_updated_at=new Date().toISOString();}
    return {accepted:true,...viewerRefreshStatus};
  }
  if (method === "viewer_refresh_status")return viewerRefreshStatus;
  if (method === "poll") {
    const ev = emitted ? [] : events;
    emitted = true;
    return { ...snapshot(), events: ev };
  }
  if (method === "quality_sessions") {
    const anchor=anchors.find(a=>a.anchor_id===p.anchor_id);
    const sessions=Array.from({length:25},(_,i)=>({anchor_id:p.anchor_id,session_id:i===0?anchor.session_id:String(7000000000000000000n+BigInt(i)),anchor_name:anchor.name,started_at:`2026-10-${String(16-i%15).padStart(2,"0")}T12:00:00+08:00`,first_seen:"2026-10-16T12:30:00+08:00",coverage:i<2?"partial":"unknown",monitor_seconds:3600+i*133,speakers:i===2?0:320+i*7,valid_speakers:i===2?0:168+i*3,valid_speaker_rate:i===2?null:(168+i*3)/(320+i*7),analysis_state:i===1?"analyzing":"ready",chat_messages:1000+i*20,valid_messages:600,unknown_sender_messages:0,pending:i===1?40:0,job_state:i===1?"queued":"",generation:1,records_cleared:false,peak_viewers:i===2?null:1280+i*20,minute_avg_viewers:i===2?null:840+i*15,audience_sample_minutes:i===2?0:60+i,valid_speakers_to_online_rate:i===2?null:(168+i*3)/(840+i*15)})).filter(row=>!p.month||row.started_at.startsWith(p.month));
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
    for (const value of p.values.split(/\s+/)) {
      const rid = value.match(/\d{5,}/)?.[0];
      if (rid && !anchors.some((a) => a.rid === rid))
        anchors.push({
          rid,
          name: rid,
          state: "stopped",
          active: false,
          is_live: null,
          message_count: 0,
          detail: "尚未启动监控",
        });
    }
    return { added: 1 };
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
