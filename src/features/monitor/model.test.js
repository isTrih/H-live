import test from "node:test";
import assert from "node:assert/strict";
import { live, offline, status, selectAnchors, duration, viewerCount, pcUnsupported, messageConnection } from "./model.js";

test("PC compatibility is explicit and independent from message connectivity", () => {
  const room = { active: true, is_live: null, state: "retrying", pc_unsupported: true };
  assert.deepEqual(status(room), ["unsupported", "非 PC 支持"]);
  assert.equal(messageConnection(room), "消息未连接");
  assert.equal(messageConnection({ ...room, state: "connected" }), "消息已连接");
  assert.equal(messageConnection({ ...room, state: "stopped", active: false }), "未监控");
  assert.equal(live(room), false);
  assert.equal(offline(room), false);
  assert.equal(pcUnsupported({ state: "retrying", detail: "HTTP 444", pc_unsupported: null }), false);
  assert.deepEqual(selectAnchors([room, { active: true, state: "retrying" }], "", "unsupported"), [room]);
});

test("uncertain room states never appear as confirmed live/offline", () => {
  for (const state of ["error", "retrying", "failed", "waiting_proxy", "confirming_offline"]) {
    assert.equal(live({ active: true, is_live: true, state }), false);
    assert.equal(offline({ active: true, is_live: false, state }), false);
  }
  assert.deepEqual(status({ active: true, state: "confirming_offline" }), ["waiting", "疑似下播"]);
  assert.deepEqual(status({ active: false, is_live: true, state: "stopped" }), ["idle", "未监控"]);
});

test("monitor filter keeps live rooms first and searches names and identities", () => {
  const anchors = [
    { rid: "1", anchor_id: "101", name: "未播主播", active: true, is_live: false, state: "offline" },
    { rid: "2", anchor_id: "102", name: "直播主播", active: true, is_live: true, state: "connected" },
    { rid: "3", anchor_id: "103", name: "待确认", active: true, is_live: null, state: "checking" },
  ];
  assert.deepEqual(selectAnchors(anchors).map(a => a.rid), ["2", "3", "1"]);
  assert.deepEqual(selectAnchors(anchors, "102").map(a => a.rid), ["2"]);
  assert.deepEqual(selectAnchors(anchors, "主播", "offline").map(a => a.rid), ["1"]);
  assert.deepEqual(selectAnchors(anchors, "", "unknown").map(a => a.rid), ["3"]);
  assert.deepEqual(anchors.map(a => a.rid), ["1", "2", "3"]);
});

test("confirmed live rooms sort by known audience descending with zero before unknown", () => {
  const base = { active: true, is_live: true, state: "connected" };
  const anchors = [
    { ...base, rid: "1", name: "A unknown", viewer_count: null },
    { ...base, rid: "2", name: "B zero", viewer_count: 0 },
    { ...base, rid: "3", name: "C small", viewer_count: 12 },
    { ...base, rid: "4", name: "D large", viewer_count: 1200 },
    { ...base, rid: "5", name: "E absent" },
  ];
  assert.deepEqual(selectAnchors(anchors).map(a => a.rid), ["4", "3", "2", "1", "5"]);
  assert.deepEqual(anchors.map(a => a.rid), ["1", "2", "3", "4", "5"]);
});

test("audience ties and unknown counts retain deterministic name and room ordering", () => {
  const base = { active: true, is_live: true, state: "connected" };
  const anchors = [
    { ...base, rid: "3", name: "B", viewer_count: 20 },
    { ...base, rid: "2", name: "B", viewer_count: 20 },
    { ...base, rid: "1", name: "A", viewer_count: 20 },
    { ...base, rid: "6", name: "Unknown B", viewer_count: null },
    { ...base, rid: "5", name: "Unknown A" },
    { ...base, rid: "4", name: "Unknown A", viewer_count: null },
  ];
  assert.deepEqual(selectAnchors(anchors).map(a => a.rid), ["1", "2", "3", "4", "5", "6"]);
});

test("invalid audience values are unknown rather than coercing to a known count", () => {
  const base = { active: true, is_live: true, state: "connected" };
  const anchors = [NaN, Infinity, -1, "1000", false].map((viewer_count, i) => (
    { ...base, rid: String(i + 1), name: "A unknown", viewer_count }
  ));
  anchors.push({ ...base, rid: "6", name: "Z known", viewer_count: 0 });
  assert.deepEqual(selectAnchors(anchors).map(a => a.rid), ["6", "1", "2", "3", "4", "5"]);
});

test("audience ranking only changes the confirmed live group and preserves filters", () => {
  const base = { active: true, anchor_id: "anchor", is_live: true, viewer_count: 9999 };
  const anchors = [
    { ...base, rid: "7", name: "B offline", is_live: false, state: "offline" },
    { ...base, rid: "6", name: "A offline", is_live: false, state: "offline", viewer_count: 0 },
    { ...base, rid: "5", name: "B unknown", state: "waiting_proxy" },
    { ...base, rid: "4", name: "A unknown", active: false, state: "stopped", viewer_count: 0 },
    { ...base, rid: "3", name: "B failed", state: "retrying" },
    { ...base, rid: "2", name: "A failed", state: "error", viewer_count: 0 },
    { ...base, rid: "1", name: "Z live", state: "connected", viewer_count: 0 },
  ];
  assert.deepEqual(selectAnchors(anchors).map(a => a.rid), ["1", "2", "3", "4", "5", "6", "7"]);
  assert.deepEqual(selectAnchors(anchors, "", "live").map(a => a.rid), ["1"]);
  assert.deepEqual(selectAnchors(anchors, "", "offline").map(a => a.rid), ["6", "7"]);
  assert.deepEqual(selectAnchors(anchors, "", "unknown").map(a => a.rid), ["4", "5"]);
  assert.deepEqual(selectAnchors(anchors, "", "error").map(a => a.rid), ["2", "3"]);
  assert.deepEqual(selectAnchors(anchors, "b", "error").map(a => a.rid), ["3"]);
});

test("duration uses task start and does not invent missing/future times", () => {
  const room = { active: true, is_live: true, state: "connected", duration_started_at: 1000, broadcast_started_at: 500 };
  assert.equal(duration(room, 4661), "01:01:01");
  assert.equal(duration(room, 999), "—");
  assert.equal(duration({ ...room, active: false }, 4661), "—");
  assert.equal(duration({ ...room, duration_started_at: null, broadcast_started_at: null }, 4661), "—");
});

test("viewer count preserves a known zero and hides absent/offline counts", () => {
  const room = { active: true, is_live: true, state: "connected", viewer_count: 0 };
  assert.equal(viewerCount(room), "0");
  assert.equal(viewerCount({ ...room, viewer_count: null }), "—");
  assert.equal(viewerCount({ ...room, is_live: false }), "—");
});

test("same-session duration resumes the saved total without counting stopped time", () => {
  const room = { active: true, is_live: true, state: "connected", duration_seconds: 3600,
    duration_running: true, duration_updated_at: 10000 };
  assert.equal(duration(room, 10020), "01:00:20");
  const paused = { ...room, duration_seconds: 3620, duration_running: false, duration_updated_at: 10020 };
  assert.equal(duration(paused, 20000), "01:00:20");
  assert.equal(duration({ ...paused, duration_running: true, duration_updated_at: 20000 }, 20030), "01:00:50");
  assert.equal(duration({ ...room, duration_seconds: 0, duration_updated_at: 30000 }, 30010), "00:00:10");
});

test("duration uses a valid checkpoint and never invents missing or negative elapsed time", () => {
  const room = { active: true, is_live: true, state: "connected", duration_seconds: 0, duration_running: false };
  assert.equal(duration(room, 1000), "00:00:00");
  assert.equal(duration({ ...room, duration_running: true }, 1000), "00:00:00");
  assert.equal(duration({ ...room, duration_running: true, duration_updated_at: 2000 }, 1000), "00:00:00");
  for (const duration_seconds of [-1, NaN, Infinity, "10"]) {
    assert.equal(duration({ ...room, duration_seconds }, 1000), "—");
  }
});
