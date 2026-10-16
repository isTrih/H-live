import test from "node:test";
import assert from "node:assert/strict";
import { createArchiveDeletion, deletionFilter } from "./archive-delete.js";

test("archive deletion requires explicit anchors, scope and session selection", () => {
  for (const input of [
    { anchorIds: [], scope: "anchors" }, { anchorIds: ["101"], scope: "invalid" },
    { anchorIds: ["101"], scope: "month", month: "2026-13" },
    { anchorIds: ["101"], scope: "sessions", month: "2026-10", sessionIds: [] },
  ]) assert.throws(() => deletionFilter(input));
});

test("archive deletion preserves exact intersections and does not inherit message-type filters", () => {
  assert.deepEqual(deletionFilter({ anchorIds: ["101"], scope: "month", month: "2026-10" }),
    { anchor_ids: ["101"], month: "2026-10" });
  assert.deepEqual(deletionFilter({ anchorIds: ["101", "102"], scope: "sessions", month: "2026-10", sessionIds: ["1001"] }),
    { anchor_ids: ["101", "102"], month: "2026-10", session_ids: ["1001"] });
  assert.deepEqual(deletionFilter({ anchorIds: ["101"], scope: "anchors", month: "2026-10" }), { anchor_ids: ["101"] });
});

test("changing the session month clears pagination and rejects stale load-more actions", async () => {
  const state = { archiveAnchors: [{ anchor_id: "101", anchor_name: "测试主播" }],
    exportSelection: new Set(["101"]), exportMonth: "2026-10", modal: null };
  let body = "";
  const requests = [];
  const deletion = createArchiveDeletion({ state,
    bridge: async (_method, params) => {
      requests.push(params);
      return { sessions: [{ anchor_name: "测试主播", session_id: "1001", records_count: 1 }],
        total: 201, next_offset: 200 };
    },
    modal: (_title, html) => { body = html; }, closeModal() {}, toast() {}, onDeleted() {},
  });
  deletion.open();
  deletion.change({ id: "delete-scope", value: "sessions", dataset: {} });
  await deletion.loadSessions();
  assert.match(body, /data-action="delete-more-sessions"/);
  deletion.change({ id: "delete-month", value: "2026-09", dataset: {} });
  assert.doesNotMatch(body, /data-action="delete-more-sessions"/);
  await deletion.loadSessions(true);
  assert.equal(requests.length, 1);
  await deletion.loadSessions();
  assert.equal(requests[1].offset, 0);
  assert.equal(requests[1].month, "2026-09");
});
