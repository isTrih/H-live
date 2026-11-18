import test from "node:test";
import assert from "node:assert/strict";
import { createSessionTimeline, normalizeTimeline, timelineDanmaku, timelineDensity, timelineHighlights, timelineTime } from "./session-timeline.js";
import {replayProgressData} from "./replay-progress.js";
import {replayTimeMap} from "./replay-sync.js";
import {setDisplayTimezone} from "./time-display.js";

const flush = () => new Promise(resolve => setImmediate(resolve));
const anchor = { rid: "111", anchor_id: "11", session_id: "current", state: "connected", active: true, is_live: true };
const settings = { base_url: "https://example.test/v1", key_configured: true, model: "test-model" };
const archive = (sessionId = "current", duration = 120) => ({
  anchor_id: "11", session_id: sessionId, start_epoch: 1792735200, duration_seconds: duration, bucket_seconds: 60,
  buckets: [{ offset_seconds: 0, count: 2 }, { offset_seconds: 60, count: 8 }], total_messages: 3, chat_count: 2,
  messages: [
    { id: "a", message_type: "chat", offset_seconds: 12, sender_name: "观众甲", content: "这个回答好有趣" },
    { id: "b", message_type: "chat", offset_seconds: 99, sender_name: "观众乙", content: "能再演示一遍吗" },
    { id: "c", message_type: "member", offset_seconds: 100, sender_name: "观众丙", content: "进入直播间" },
  ],
});
const analysis = { highlights: [{ offset_seconds: 99, title: "演示引发提问", reason: "观众主动追问。", evidence: [{ message_id: "b", sender_name: "观众乙", content: "能再演示一遍吗" }] }], summary: "一处有证据的互动。", model: "test-model", analyzed_messages: 3 };

function fixture(t, bridge, replay = null, marks = [], onCreateClip = null) {
  const previousDocument = globalThis.document;
  const position = { textContent: "" };
  const seek = { value: 0, attributes: {}, setAttribute(name, value) { this.attributes[name] = value; } };
  const cursor = { style: {} };
  const layer = { innerHTML: "", querySelectorAll() { return []; } };
  const activityLayer = { innerHTML: "" };
  const speakerSummary = { innerHTML: "" };
  const root = { innerHTML: "", contains: () => false, querySelector(selector) {
    return { ".timeline-position": position, "#timeline-seek": seek, ".timeline-cursor": cursor, ".timeline-danmaku": layer, ".timeline-activity-danmaku": activityLayer, ".timeline-speaker-summary": speakerSummary }[selector] || null;
  } };
  globalThis.document = { querySelector: selector => selector === "#session-timeline" ? root : null, activeElement: null };
  const state = { modal: { kind: "detail", rid: "111" } };
  const calls = [];
  const notices = [];
  let configured = 0;
  const controller = createSessionTimeline({ state, onCreateClip, bridge: async (method, params) => { calls.push([method, params]);if(method==="mark_list")return {marks:marks.filter(mark=>mark.session_id===params.session_id),total:marks.length};if(method==="session_replay")return {enabled:Boolean(replay),available:Boolean(replay),replay};return bridge(method, params); }, toast: (...args) => notices.push(args), onConfigure: () => configured++ });
  t.after(() => { controller.unmount(); globalThis.document = previousDocument; });
  return { controller, state, root, position, seek, cursor, layer, activityLayer, speakerSummary, calls, notices, configured: () => configured };
}

test("replay heading exposes the selected session ID and explicit distinct speaker counts",async t=>{
  const id="7698991795467832114";
  const data={...archive(id),summary:{analysis_state:"ready",speakers:122,valid_speakers:13}};
  const f=fixture(t,async method=>method==="highlight_settings"?settings:data);
  f.controller.mount({...anchor,session_id:id});await flush();
  assert.match(f.root.innerHTML,/场次 ID<\/span><code data-selectable>7698991795467832114<\/code>/);
  assert.match(f.root.innerHTML,/aria-label="复制场次 ID"/);
  assert.match(f.root.innerHTML,/发言人数 <b>122<\/b>/);
  assert.match(f.root.innerHTML,/有效发言人数 <b>13<\/b>/);
  assert.match(f.root.innerHTML,/排除只有无效发言的账号/);
  assert.doesNotMatch(f.root.innerHTML,/>统计中</);
  f.controller.selectSession("7000000000000000001");
  assert.match(f.root.innerHTML,/<code data-selectable>7000000000000000001<\/code>/);
  assert.doesNotMatch(f.root.innerHTML,/<code data-selectable>7698991795467832114/);
});

test("quality refresh patches only current speaker statistics and marks unfinished counts as provisional",async t=>{
  const f=fixture(t,async method=>method==="highlight_settings"?settings:archive());
  f.controller.mount(anchor);await flush();
  const original=f.root.innerHTML;
  f.controller.updateQuality({anchor_id:"11",session_id:"current",analysis_state:"analyzing",speakers:8,valid_speakers:4});
  assert.match(f.speakerSummary.innerHTML,/有效发言人数 <b>4<\/b>/);
  assert.match(f.speakerSummary.innerHTML,/>统计中</);
  assert.equal(f.root.innerHTML,original,"a stats refresh must not recreate the player or interrupt editing");
  const stage=f.speakerSummary.innerHTML;
  f.controller.updateQuality({anchor_id:"wrong",session_id:"current",analysis_state:"ready",speakers:999,valid_speakers:999});
  f.controller.updateQuality({anchor_id:"11",session_id:"older",analysis_state:"ready",speakers:999,valid_speakers:999});
  assert.equal(f.speakerSummary.innerHTML,stage,"late counts from another session or anchor must be ignored");
  f.controller.updateQuality({anchor_id:"11",session_id:"current",analysis_state:"ready",speakers:8,valid_speakers:0});
  assert.match(f.speakerSummary.innerHTML,/有效发言人数 <b>0<\/b>/);
  assert.doesNotMatch(f.speakerSummary.innerHTML,/>统计中</);
  f.controller.selectSession("older");
  assert.match(f.root.innerHTML,/有效发言人数 <b>—<\/b>/,"new session must not inherit previous counts");
});

test("copy uses the complete selected string ID without numeric rounding and reports clipboard failure",async t=>{
  const previous=Object.getOwnPropertyDescriptor(globalThis,"navigator");
  t.after(()=>previous?Object.defineProperty(globalThis,"navigator",previous):delete globalThis.navigator);
  const copied=[];
  Object.defineProperty(globalThis,"navigator",{configurable:true,value:{clipboard:{writeText:async value=>copied.push(value)}}});
  const f=fixture(t,async method=>method==="highlight_settings"?settings:archive());
  f.controller.mount({...anchor,session_id:"7698991795467832114"});await flush();
  await f.controller.action({dataset:{action:"timeline-copy-session"}});
  f.controller.selectSession("7698673632112937737");
  await f.controller.action({dataset:{action:"timeline-copy-session"}});
  assert.deepEqual(copied,["7698991795467832114","7698673632112937737"]);
  assert.equal(f.notices.at(-1)[0],"场次 ID 已复制");
  globalThis.navigator.clipboard.writeText=async()=>{throw Error("denied");};
  await f.controller.action({dataset:{action:"timeline-copy-session"}});
  assert.deepEqual(f.notices.at(-1),["复制失败，请选中场次 ID 手动复制",true]);
});

test("whole recording download is disabled without replay and never creates a fake range",async t=>{
  const requests=[],f=fixture(t,async method=>method==="highlight_settings"?settings:archive(),null,[],p=>requests.push(p));
  f.controller.mount(anchor);await flush();
  assert.match(f.root.innerHTML,/data-action="timeline-session-download" disabled/);
  await f.controller.action({dataset:{action:"timeline-session-download"}});
  assert.equal(requests.length,0);assert.match(f.notices.at(-1)[0],/暂无可下载录播/);
});

test("whole recording download submits only session identity even before decoder readiness and deduplicates a pending click",async t=>{
  const requests=[];let finish;
  const f=fixture(t,async method=>method==="highlight_settings"?settings:archive("current",60),{play_url:"https://example.test/vod.m3u8",duration_ms:21600000},[],p=>{requests.push(p);return new Promise(resolve=>finish=resolve);});
  f.controller.mount(anchor);await flush();
  assert.match(f.root.innerHTML,/data-action="timeline-session-download" >/);
  const pending=f.controller.action({dataset:{action:"timeline-session-download"}});await flush();
  await f.controller.action({dataset:{action:"timeline-session-download"}});
  assert.equal(requests.length,1);assert.equal(requests[0].mode,"full_session");assert.equal(requests[0].session_id,"current");
  assert.equal(Object.hasOwn(requests[0],"start_seconds"),false);assert.equal(Object.hasOwn(requests[0],"end_seconds"),false);
  assert.match(requests[0].request_key,/^full:/);finish({job_id:"queued"});await pending;
});

test("recording PDT is displayed before archive loading and timezone refresh never recalibrates seek values",async t=>{
  t.after(()=>setDisplayTimezone("Asia/Shanghai"));setDisplayTimezone("Asia/Shanghai");
  let finishArchive;
  const start=Date.parse("2026-10-25T06:00:00Z")/1000;
  const replay={play_url:"https://example.test/replay.m3u8",duration_ms:120000,time_mapping:[{offset_seconds:0,duration:120,program_date_time:start}]};
  const f=fixture(t,async method=>method==="highlight_settings"?settings:new Promise(resolve=>{finishArchive=resolve;}),replay);
  f.controller.mount({...anchor,is_live:false});await flush();
  assert.equal(f.position.textContent,"14:00:00");
  assert.match(f.root.innerHTML,/timeline-media-axis/);assert.match(f.root.innerHTML,/data-offset="60"[^>]*>14:01:00/);
  assert.match(f.root.innerHTML,/id="timeline-media-seek"[^>]*max="120"/);
  finishArchive(archive());await flush();
  f.controller.change({id:"timeline-replay-offset",value:"3600"});
  assert.equal(f.position.textContent,"14:00:00","danmaku calibration must not change the picture clock");
  setDisplayTimezone("UTC");f.controller.refreshTimeDisplay();
  assert.equal(f.position.textContent,"06:00:00");assert.match(f.root.innerHTML,/data-offset="60"[^>]*>06:01:00/);
  assert.match(f.root.innerHTML,/id="timeline-media-seek"[^>]*max="120"/);
});

test("video density remains visible at a partial archive boundary and labels extrapolated alignment",async t=>{
  const data={...archive(),start_epoch:1000,duration_seconds:40,bucket_seconds:60,
    buckets:[{offset_seconds:0,count:1}],chat_count:1,
    messages:[{message_type:"chat",time_source:"server",offset_seconds:35.5}]};
  const replay={play_url:"https://example.test/replay.m3u8",duration_ms:5000,time_mapping:[
    {offset_seconds:0,duration:5,program_date_time:1035,time_source:"extrapolated"},
  ]};
  const f=fixture(t,async method=>method==="highlight_settings"?settings:data,replay);
  f.controller.mount({...anchor,is_live:false});await flush();
  assert.match(f.root.innerHTML,/录播互动进度条/);
  assert.match(f.root.innerHTML,/分布，峰值 1 条/);
  assert.match(f.root.innerHTML,/class="timeline-density-area"[^>]* d="M[^\"]+"/);
  assert.match(f.root.innerHTML,/部分片段按录播时间推算，可微调偏移/);
});

test("activity markers expose distinct hover details and seek into notification replay without changing chat density", async t => {
  const data = { ...archive(), activities: [
    { id: "lottery", kind: "lottery", activity_id: "000123", offset_seconds: 30, time_source: "server", epoch: 1792735230, draw_at: 1792735500, details: { prize_name: "限定玩偶<script>", conditions: [{ content: "发送评论「开心冒险」" }] } },
    { id: "packet", kind: "red_packet", activity_id: "packet-1", offset_seconds: 30, time_source: "observed", epoch: 1792735230, expires_at: 1792735800, details: { title: "观众红包", total_count: 20 } },
  ] };
  const original = structuredClone(data);
  const f = fixture(t, async method => method === "highlight_settings" ? settings : data);
  f.controller.mount({ ...anchor, is_live: false }); await flush();
  assert.deepEqual(data, original, "replay must not alter the full archive used by analysis");
  assert.doesNotMatch(f.root.innerHTML,/class="timeline-activity-marker |timeline-activity-danmaku/);
  assert.equal((f.root.innerHTML.match(/class="timeline-activity-interval /g) || []).length, 2);
  assert.match(f.root.innerHTML, /timeline-activity-interval lottery/);
  assert.match(f.root.innerHTML, /timeline-activity-interval red_packet/);
  assert.match(f.root.innerHTML, /role="tooltip"/);
  assert.match(f.root.innerHTML, /通知时间.+平台时间/);
  assert.match(f.root.innerHTML, /观察时间.+活动通知时间未知/);
  assert.match(f.root.innerHTML, /开奖时间/);
  assert.match(f.root.innerHTML, /开心冒险|限定玩偶&lt;script&gt;/);
  assert.doesNotMatch(f.root.innerHTML, /<script>/);
  assert.match(f.root.innerHTML, /峰值 8 条/);
  assert.equal(f.activityLayer.innerHTML, "");
  await f.controller.action({ dataset: { action: "timeline-activity", activity: "0" } });
  assert.equal(f.seek.value, 30);
  assert.equal(f.activityLayer.innerHTML, "");
  f.controller.input({ id: "timeline-seek", value: "80" });
  assert.equal(f.activityLayer.innerHTML, "");
  f.controller.input({ id: "timeline-seek", value: "30" });
  await f.controller.action({ dataset: { action: "timeline-danmaku" } });
  assert.equal(f.activityLayer.innerHTML, "");
  assert.equal(f.calls.filter(([method]) => method === "highlight_analyze").length, 0);
});

test("unknown activity times and unidentified notifications never become markers at zero", async t => {
  const data = { ...archive(), unconfirmed_activity_count: 2, activities: [
    { id: "unknown", kind: "lottery", offset_seconds: 0, time_source: "unknown", details: { title: "时间未知活动" } },
  ] };
  const f = fixture(t, async method => method === "highlight_settings" ? settings : data);
  f.controller.mount({ ...anchor, is_live: false }); await flush();
  assert.doesNotMatch(f.root.innerHTML, /data-action="timeline-activity"/);
  assert.match(f.root.innerHTML, /1 个活动通知时间未知，未放入时间轴/);
  assert.match(f.root.innerHTML, /2 条活动通知缺少可靠活动标识/);
  assert.equal(f.activityLayer.innerHTML, "");
});

test("an activity-only recording seeks without adding a video notification", async t => {
  const data = { ...archive(), messages: [], chat_count: 0, total_messages: 1, buckets: [], activities: [
    { id: "packet", kind: "red_packet", activity_id: "packet", offset_seconds: 30, time_source: "server", epoch: 1792735230, details: { title: "小红包" } },
  ] };
  const f = fixture(t, async method => method === "highlight_settings" ? settings : data);
  f.controller.mount({ ...anchor, is_live: false }); await flush();
  assert.doesNotMatch(f.root.innerHTML, /本场还没有归档聊天|暂无可定位的回放时间/);
  await f.controller.action({ dataset: { action: "timeline-activity", activity: "0" } });
  assert.equal(f.seek.value,30);
  assert.equal(f.activityLayer.innerHTML, "");
});

test("restored offline identity loads its last-known archive without reopening a broadcast",async t=>{
 const f=fixture(t,async(method,params)=>method==="highlight_settings"?settings:archive(params.session_id));
 f.controller.mount({...anchor,anchor_id:"",session_id:"",is_live:null});await flush();
 f.controller.tick({...anchor,is_live:false});await flush();
 assert.deepEqual(f.calls.filter(([method])=>method==="session_timeline").map(([,p])=>[p.anchor_id,p.session_id]),[["11","current"]]);
 assert.equal(f.position.textContent,"14:00:00");assert.match(f.root.innerHTML,/data-action="timeline-play"/);assert.doesNotMatch(f.root.innerHTML,/data-action="timeline-latest"/);
});

test("a paused replay updates live controls when the same session becomes offline",async t=>{
 const f=fixture(t,async(method,params)=>method==="highlight_settings"?settings:archive(params.session_id));
 f.controller.mount(anchor);await flush();
 await f.controller.action({dataset:{action:"timeline-play"}});
 f.controller.tick({...anchor,is_live:false});
 assert.match(f.root.innerHTML,/data-action="timeline-play"/);assert.doesNotMatch(f.root.innerHTML,/data-action="timeline-latest"/);
 assert.match(f.root.innerHTML,/aria-label="播放回放"/);
 assert.equal(f.calls.filter(([method])=>method==="session_timeline").length,1);
});

test("identity replacement resets explicit history and ignores its delayed archive",async t=>{
 let finishOld;
 const f=fixture(t,async(method,params)=>{
  if(method==="highlight_settings")return settings;
  if(params.anchor_id==="11"&&params.session_id==="older")return new Promise(resolve=>finishOld=resolve);
  return {...archive(params.session_id,params.anchor_id==="22"?360:120),anchor_id:params.anchor_id};
 });
 f.controller.mount(anchor);await flush();f.controller.selectSession("older");
 f.controller.tick({...anchor,anchor_id:"22",session_id:"new-owner",is_live:false});await flush();
 assert.ok(f.calls.some(([method,p])=>method==="session_timeline"&&p.anchor_id==="22"&&p.session_id==="new-owner"));
 finishOld({...archive("older",999),messages:[{message_type:"chat",offset_seconds:0,content:"原主播迟到弹幕"}]});await flush();
 assert.match(f.root.innerHTML,/00:06:00/);assert.doesNotMatch(f.root.innerHTML,/00:16:39/);assert.doesNotMatch(f.layer.innerHTML,/原主播迟到弹幕/);
 f.controller.tick({...anchor,anchor_id:"22",session_id:"new-owner",is_live:false});await flush();
 assert.equal(f.calls.filter(([method])=>method==="session_timeline").length,3);
});

test("identity replacement cancels an old analysis and rejects its late highlight result",async t=>{
 let finishStatus;
 const f=fixture(t,async(method,params)=>{
  if(method==="highlight_settings")return settings;
  if(method==="highlight_analyze")return {job_id:"old-identity-job"};
  if(method==="highlight_status")return new Promise(resolve=>finishStatus=resolve);
  if(method==="highlight_cancel")return {};
  return {...archive(params.session_id),anchor_id:params.anchor_id};
 });
 f.controller.mount(anchor);await flush();
 const pending=f.controller.action({dataset:{action:"timeline-analyze"}});await flush();
 f.controller.tick({...anchor,anchor_id:"22",session_id:"current",is_live:false});await flush();
 finishStatus({status:"completed",anchor_id:"11",session_id:"current",result:analysis});await pending;
 assert.ok(f.calls.some(([method,p])=>method==="highlight_cancel"&&p.job_id==="old-identity-job"));
 assert.doesNotMatch(f.root.innerHTML,/演示引发提问|class="timeline-bulb|正在用单次请求/);
 assert.ok(f.calls.some(([method,p])=>method==="session_timeline"&&p.anchor_id==="22"));
});

test("identity changes discard cached analysis even when returning to a previous owner",async t=>{
 const f=fixture(t,async(method,params)=>{
  if(method==="highlight_settings")return settings;
  if(method==="highlight_analyze")return {job_id:"cached-job"};
  if(method==="highlight_status")return {status:"completed",anchor_id:"11",session_id:"current",result:analysis};
  return {...archive(params.session_id),anchor_id:params.anchor_id};
 });
 f.controller.mount(anchor);await flush();await f.controller.action({dataset:{action:"timeline-analyze"}});
 assert.match(f.root.innerHTML,/演示引发提问/);
 f.controller.tick({...anchor,anchor_id:"22",session_id:"current",is_live:false});await flush();
 assert.doesNotMatch(f.root.innerHTML,/演示引发提问/);
 f.controller.tick({...anchor,is_live:false});await flush();
 assert.doesNotMatch(f.root.innerHTML,/演示引发提问/);
});

test("unknown chat timestamps never become a burst at zero or a received-time replay", () => {
  const data = normalizeTimeline({ duration_seconds: 90, messages: [
    { id: "missing", message_type: "chat", time_source: "missing", offset_seconds: 0, observed_offset_seconds: 10 },
    { id: "null", message_type: "chat", offset_seconds: null },
    { id: "late", message_type: "chat", offset_seconds: 20 },
    { id: "earlier", message_type: "chat", offset_seconds: 10 },
    { id: "join", message_type: "member", offset_seconds: 0 },
  ] });
  assert.deepEqual(data.messages.map(message => message.id), ["earlier", "late"]);
  assert.equal(data.unknown_chat_times, 2);
  assert.equal(data.chat_count, 4);
  assert.deepEqual(timelineDanmaku(data.messages, 0), []);
  assert.equal(normalizeTimeline({ duration_seconds: null }).duration_seconds, null);
  assert.equal(timelineTime(90061), "25:01:01");
  assert.equal(timelineTime(null), "—");
});

test("message area preserves zero gaps and uses proportional count heights", () => {
  const chart = timelineDensity([{ offset_seconds: 0, count: 5 }, { offset_seconds: 120, count: 10 }, { offset_seconds: null, count: 1000 }], 180, 60);
  assert.equal(chart.peak, 10);
  assert.match(chart.line, /L0,35\.5L333\.33,35\.5L333\.33,64L666\.67,64L666\.67,7/);
  assert.doesNotMatch(chart.area, /NaN|Infinity/);
  assert.equal(timelineDensity([{ offset_seconds: 0, count: 0 }], 180).area, "");
  assert.equal(timelineDensity([{ offset_seconds: 0, count: 9 }], null).area, "");
});

test("rebinned video density returns to zero between sparse chat buckets",()=>{
  const data={bucket_seconds:60,messages:[
    {message_type:"chat",offset_seconds:5},
    {message_type:"chat",offset_seconds:125},
  ]};
  const progress=replayProgressData(data,[],replayTimeMap({},1000),180);
  assert.deepEqual(progress.buckets,[{offset_seconds:0,count:1},{offset_seconds:120,count:1}]);
  const chart=timelineDensity(progress.buckets,180,60);
  assert.match(chart.line,/L0,7L333\.33,7L333\.33,64L666\.67,64L666\.67,7/);
});

test("seeking replays only messages at that time and keeps a flood bounded", () => {
  const data = normalizeTimeline(archive());
  assert.deepEqual(timelineDanmaku(data.messages, 101).map(message => message.id), ["b"]);
  assert.deepEqual(timelineDanmaku(data.messages, 16).map(message => message.id), ["a"]);
  assert.deepEqual(timelineDanmaku(data.messages, 80), []);
  const flood = Array.from({ length: 5000 }, (_, index) => ({ id: String(index), offset_seconds: index / 100 }));
  assert.ok(timelineDanmaku(flood, 49).length <= 16);
  assert.ok(timelineDanmaku(flood, 49).every(message => message.offset_seconds <= 49 && message.offset_seconds >= 37));
  assert.ok(timelineDanmaku(flood, 49).some(message => message.progress > .5));
  assert.deepEqual(timelineDanmaku(flood, 49).map(message => message.id), timelineDanmaku(flood, 49.05).map(message => message.id));
});

test("archive-only loading has no invented highlights or automatic model request", async t => {
  const f = fixture(t, async method => method === "highlight_settings" ? settings : archive());
  f.controller.mount(anchor); await flush();
  assert.match(f.root.innerHTML, /互动回放/);
  assert.equal((f.root.innerHTML.match(/>暂无录播回放</g) || []).length, 1);
  assert.doesNotMatch(f.root.innerHTML, /class="timeline-bulb/);
  assert.equal(f.calls.filter(([method]) => method === "highlight_analyze").length, 0);
  assert.equal(f.position.textContent, "14:01:36");
});

test("archive polling preserves a paused user's seek position", async t => {
  const originalNow = Date.now;
  let now = 10000;
  Date.now = () => now;
  t.after(() => { Date.now = originalNow; });
  let reads = 0;
  const f = fixture(t, async method => method === "highlight_settings" ? settings : archive("current", ++reads === 1 ? 120 : 180));
  f.controller.mount(anchor); await flush();
  f.controller.input({ id: "timeline-seek", value: "36.5" });
  await f.controller.action({ dataset: { action: "timeline-play" } });
  now += 6000; f.controller.tick(anchor); await flush();
  assert.equal(reads, 2);
  assert.equal(f.position.textContent, "14:00:36");
  assert.equal(f.seek.value, 36.5);
  assert.match(f.root.innerHTML, /aria-label="播放回放"/);
});

test("switching episodes rejects slow archive results from the previous session", async t => {
  let finishOld;
  const f = fixture(t, async (method, params) => {
    if (method === "highlight_settings") return settings;
    if (params.session_id === "current") return new Promise(resolve => { finishOld = resolve; });
    return { ...archive("older", 360), start_epoch: null };
  });
  f.controller.mount(anchor); f.controller.selectSession("older"); await flush();
  finishOld(archive()); await flush();
  assert.match(f.root.innerHTML, /00:06:00/);
  assert.match(f.root.innerHTML, /场次时间待确认/);
  assert.equal(f.position.textContent, "00:00:00");
  f.controller.tick(anchor); await flush();
  assert.equal(f.calls.filter(([method]) => method === "session_timeline").length, 2);
});

test("selecting the current episode after history resumes following future broadcasts", async t => {
  const f = fixture(t, async (method, params) => method === "highlight_settings" ? settings : archive(params.session_id));
  f.controller.mount(anchor); await flush();
  f.controller.selectSession("older"); await flush();
  f.controller.tick(anchor); await flush();
  assert.deepEqual(f.calls.filter(([method]) => method === "session_timeline").map(([, params]) => params.session_id), ["current", "older"]);
  f.controller.selectSession("current"); await flush();
  f.controller.input({ id: "timeline-seek", value: "35" });
  f.controller.selectSession("current");
  assert.equal(f.seek.value, 35);
  f.controller.tick({ ...anchor, session_id: "next-broadcast" }); await flush();
  f.controller.selectSession("next-broadcast");
  f.controller.tick({ ...anchor, session_id: "third-broadcast" }); await flush();
  assert.deepEqual(f.calls.filter(([method]) => method === "session_timeline").map(([, params]) => params.session_id), ["current", "older", "current", "next-broadcast", "third-broadcast"]);
});

test("explicit analysis uses one upstream job and turns evidence into seekable markers", async t => {
  const result = { ...analysis, highlights: [{ ...analysis.highlights[0], title: '<script>unsafe</script>', evidence: [{ content: '<img src=x>', sender_name: '<b>观众</b>' }] }] };
  const f = fixture(t, async method => {
    if (method === "highlight_settings") return settings;
    if (method === "highlight_analyze") return { job_id: "job-1", status: "running" };
    if (method === "highlight_status") return { status: "completed", anchor_id: "11", session_id: "current", result };
    return archive();
  });
  f.controller.mount(anchor); await flush();
  await f.controller.action({ dataset: { action: "timeline-analyze" } });
  assert.equal(f.calls.filter(([method]) => method === "highlight_analyze").length, 1);
  assert.match(f.root.innerHTML, /class="timeline-bulb/);
  assert.match(f.root.innerHTML, /&lt;script&gt;/);
  assert.doesNotMatch(f.root.innerHTML, /<script>|<img src=x>/);
  await f.controller.action({ dataset: { action: "timeline-highlight", highlight: "0" } });
  assert.equal(f.seek.value, 99);
  assert.match(f.root.innerHTML, /&lt;img src=x&gt;/);
  assert.match(f.root.innerHTML, /&lt;b&gt;观众&lt;\/b&gt;/);
  assert.deepEqual(timelineHighlights({ highlights: [{ offset_seconds: null }, { offset_seconds: 121 }] }, 120), []);
});

test("a status-read failure resumes the same job without another model request", async t => {
  let statusReads = 0;
  const f = fixture(t, async method => {
    if (method === "highlight_settings") return settings;
    if (method === "highlight_analyze") return { job_id: "job-resume" };
    if (method === "highlight_status") { if (++statusReads === 1) throw Error("暂时断线"); return { status: "completed", result: analysis }; }
    return archive();
  });
  f.controller.mount(anchor); await flush();
  await f.controller.action({ dataset: { action: "timeline-analyze" } });
  assert.match(f.root.innerHTML, /继续读取结果/);
  await f.controller.action({ dataset: { action: "timeline-analysis-refresh" } });
  assert.equal(f.calls.filter(([method]) => method === "highlight_analyze").length, 1);
  assert.deepEqual(f.calls.filter(([method]) => method === "highlight_status").map(([, params]) => params.job_id), ["job-resume", "job-resume"]);
  assert.match(f.root.innerHTML, /演示引发提问/);
});

test("missing model configuration opens settings before starting a paid analysis", async t => {
  const f = fixture(t, async method => method === "highlight_settings" ? { ...settings, model: "" } : archive());
  f.controller.mount(anchor); await flush();
  await f.controller.action({ dataset: { action: "timeline-analyze" } });
  assert.equal(f.configured(), 1);
  assert.equal(f.calls.filter(([method]) => method === "highlight_analyze").length, 0);
  assert.equal(f.notices[0][0], "请先配置分析接口与模型");
});

test("closing while a job starts cancels the late job and never repaints the modal", async t => {
  let finishJob;
  const f = fixture(t, async method => {
    if (method === "highlight_settings") return settings;
    if (method === "highlight_analyze") return new Promise(resolve => { finishJob = resolve; });
    if (method === "highlight_cancel") return {};
    return archive();
  });
  f.controller.mount(anchor); await flush();
  const pending = f.controller.action({ dataset: { action: "timeline-analyze" } });
  await flush();
  f.controller.unmount(); f.state.modal = null; f.root.innerHTML = "closed";
  finishJob({ job_id: "late-job" }); await pending;
  assert.equal(f.root.innerHTML, "closed");
  assert.ok(f.calls.some(([method, params]) => method === "highlight_cancel" && params.job_id === "late-job"));
  assert.equal(f.calls.filter(([method]) => method === "highlight_status").length, 0);
});

test("switching sessions cancels an in-flight analysis and rejects its evidence", async t => {
  let finishStatus;
  const f = fixture(t, async (method, params) => {
    if (method === "highlight_settings") return settings;
    if (method === "highlight_analyze") return { job_id: "old-job" };
    if (method === "highlight_status") return new Promise(resolve => { finishStatus = resolve; });
    if (method === "highlight_cancel") return {};
    return archive(params.session_id);
  });
  f.controller.mount(anchor); await flush();
  const pending = f.controller.action({ dataset: { action: "timeline-analyze" } }); await flush();
  f.controller.selectSession("older"); await flush();
  finishStatus({ status: "completed", result: analysis, session_id: "current" }); await pending;
  assert.doesNotMatch(f.root.innerHTML, /演示引发提问|class="timeline-bulb/);
  assert.ok(f.calls.some(([method, params]) => method === "highlight_cancel" && params.job_id === "old-job"));
});
