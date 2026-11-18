import test from "node:test";
import assert from "node:assert/strict";
import { timelineActivities, activityMarkerLayout, activityNoticeWindow, activityDetailLines,activityMarkerHtml } from "./timeline-activities.js";

const activity = (id, offset, kind = "lottery", source = "server") => ({ id, activity_id: id, kind, offset_seconds: offset, time_source: source, epoch: 1000 + (offset || 0), details: {} });

test("only canonical supported activities receive explicitly sourced positions", () => {
  const data = { duration_seconds: 120, messages: [{ message_type: "activity", offset_seconds: 0, payload: { activity_kind: "lottery", activity_id: "raw-only" } }], activities: [
    activity("server", 30), activity("observed", 60, "red_packet", "observed"),
    activity("unknown", 0, "lottery", "unknown"), activity("null", null),
    activity("before", -1), activity("future", 121), activity("unsupported", 10, "gift"), activity("prototype", 10, "__proto__"),
  ] };
  const normalized = timelineActivities(data);
  assert.equal(normalized.length, 6);
  assert.deepEqual(normalized.filter(a => a.offset_seconds !== null).map(a => a.id), ["server", "observed"]);
  assert.equal(normalized.find(a => a.id === "observed").time_source, "observed");
  assert.deepEqual(timelineActivities({ ...data, activities: undefined }), []);
  assert.equal(data.activities[2].offset_seconds, 0);
});

test("canonical IDs deduplicate while same-looking red packets remain separate", () => {
  const normalized = timelineActivities({ duration_seconds: 100, activities: [
    activity("packet-1", 10, "red_packet"), activity("packet-2", 10, "red_packet"),
    { ...activity("packet-1", 10, "red_packet"), details: { title: "最新详情" } },
  ] });
  assert.equal(normalized.length, 2);
  assert.equal(normalized.find(a => a.id === "packet-1").details.title, "最新详情");
});

test("fixed-size markers use separate collision rows even on narrow screens", () => {
  const activities = timelineActivities({ duration_seconds: 100, activities: [activity("a", 0), activity("b", 0, "red_packet"), activity("c", 1), activity("d", 100)] });
  const narrow = activityMarkerLayout(activities, 100, 320);
  assert.equal(narrow.rows, 3);
  assert.equal(new Set(narrow.markers.slice(0, 3).map(marker => marker.row)).size, 3);
  assert.ok(narrow.markers.every(marker => marker.left > 0 && marker.left < 100));
  assert.equal(narrow.markers.at(-1).row, 0);
  const dense = activityMarkerLayout(activities.map(a => ({ ...a, count: 100000 })), 100, 320);
  assert.deepEqual(dense.markers.map(a => [a.row, a.left]), narrow.markers.map(a => [a.row, a.left]));
});

test("seeking shows only already-observed nearby activity notices and bounds a flood", () => {
  const activities = timelineActivities({ duration_seconds: 120, activities: Array.from({ length: 100 }, (_, i) => activity(String(i), i)) });
  assert.deepEqual(activityNoticeWindow(activities, 20).map(a => a.id), ["19", "20"]);
  assert.deepEqual(activityNoticeWindow(activities, 110), []);
  assert.deepEqual(activityNoticeWindow([activity("unknown", null)], 0), []);
  assert.deepEqual(activityNoticeWindow(activities, 20, 10, "lottery:12").map(a => a.id), ["12", "20"], "a clicked event stays visible when other nearby events compete for notification space");
});

test("draw and expiry details stay separate from notification or observation time", () => {
  const observed = { ...activity("packet", 20, "red_packet", "observed"), expires_at: 2000, details: { title: "红包", total_count: 3 } };
  const lines = activityDetailLines(observed, String);
  assert.ok(lines.includes("观察时间 1020（活动通知时间未知）"));
  assert.ok(lines.includes("结束时间 2000"));
  assert.ok(lines.includes("红包份数：3"));
  assert.ok(activityDetailLines(activity("lottery", 20), String).includes("开奖时间未知"));
  assert.ok(activityDetailLines({ ...activity("legacy", 20), content: "福袋：可爱玩偶 3 份" }, String).includes("活动说明：福袋：可爱玩偶 3 份"));
});
test("activity intervals use known draw or expiry times and preserve click targets",()=>{
  const activities=timelineActivities({start_epoch:1000,duration_seconds:120,activities:[
    {...activity("bag",20),draw_at:1060},
    {...activity("packet",40,"red_packet"),expires_at:1090},
    activity("unknown-end",80),
  ]});
  assert.deepEqual(activities.map(a=>a.end_offset_seconds),[60,90,null]);
  const markup=activityMarkerHtml(activityMarkerLayout(activities,120,600),{clock:String,time:String});
  assert.equal((markup.match(/class="timeline-activity-interval/g)||[]).length,2);
  assert.match(markup,/timeline-activity-interval red_packet" data-action="timeline-activity" data-activity="1"/);
  assert.match(markup,/timeline-activity-range-tip-0/);
  assert.match(markup,/20 至 60/);
  assert.doesNotMatch(markup,/<img|class="timeline-activity-marker /);
});

test("hundreds of overlapping rounds become one bounded density layer per kind",()=>{
  const activities=timelineActivities({start_epoch:1000,duration_seconds:300,activities:Array.from({length:500},(_,index)=>[
    {...activity(`bag-${index}`,10),draw_at:1100,details:{prize_name:`奖品 ${index}<script>`}},
    {...activity(`red-${index}`,20,"red_packet"),expires_at:1120},
  ]).flat()});
  const layout=activityMarkerLayout(activities,300,600);
  assert.equal(layout.markers.length,1000,"canonical activities and click indices are retained");
  assert.equal(layout.intervals.length,2,"same-kind overlays cannot accumulate opacity");
  for(const band of layout.intervals){assert.equal(band.count,500);assert.equal(band.samples.length,4);assert.ok(band.opacity<=.2);}
  const markup=activityMarkerHtml(layout,{clock:String,time:String});
  assert.equal((markup.match(/class="timeline-activity-interval /g)||[]).length,2);
  assert.equal((markup.match(/timeline-activity-density-round/g)||[]).length,8);
  assert.match(markup,/500 轮重叠/);assert.match(markup,/还有 496 轮未展开/);assert.match(markup,/另有 499 轮/);
  assert.doesNotMatch(markup,/<script>|<img/);
});

test("density sweep uses half-open ranges and clicks the latest real start",()=>{
  const activities=timelineActivities({start_epoch:1000,duration_seconds:400,activities:Array.from({length:100},(_,index)=>({...activity(String(index),index),draw_at:1200+index}))});
  const layout=activityMarkerLayout(activities,400,800),bands=layout.intervals;
  assert.equal(bands.length,199);
  for(let index=1;index<bands.length;index++)assert.ok(bands[index-1].end<=bands[index].start);
  const at50=bands.find(band=>band.start===50);
  assert.equal(at50.count,51);assert.equal(at50.activity.offset_seconds,50);assert.equal(at50.index,50);
  const at200=bands.find(band=>band.start===200);
  assert.equal(at200.count,99,"a round ending exactly at the boundary is already removed");
  assert.equal(at200.activity.offset_seconds,99);
  const markup=activityMarkerHtml(layout,{clock:String,time:String});
  assert.match(markup,/data-activity="50" data-activity-key="lottery:50:51"/);
  assert.match(markup,/开始 1050 · 开奖 1250/);
});

test("ordinary overlaps preserve individual interval details and targets",()=>{
  const activities=timelineActivities({start_epoch:1000,duration_seconds:100,activities:Array.from({length:3},(_,index)=>({...activity(String(index),10+index),draw_at:1060}))});
  const layout=activityMarkerLayout(activities,100,320);
  assert.equal(layout.intervals.length,3);assert.ok(layout.intervals.every(band=>!band.dense));
  const markup=activityMarkerHtml(layout,{clock:String,time:String});
  assert.doesNotMatch(markup,/activity-density/);assert.match(markup,/data-activity="2"/);
});
