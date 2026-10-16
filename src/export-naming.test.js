import { test } from "node:test";
import assert from "node:assert/strict";
import { exportFileName } from "./export-naming.js";

const options = { anchors: [{ anchor_id: "1", anchor_name: "小野uu" }, { anchor_id: "2", anchor_name: "阿梨" }], month: "2026-10", format: "xlsx" };

test("multiple selected anchors use the month-only file name in both formats", () => {
  for (const format of ["xlsx", "csv"]) {
    assert.equal(exportFileName({ ...options, anchorIds: ["1", "2"], format }), `直播消息-2026-10.${format}`);
  }
});

test("one unique anchor adds its name, including when the selection repeats", () => {
  assert.equal(exportFileName({ ...options, anchorIds: ["1"] }), "直播消息-小野uu-2026-10.xlsx");
  assert.equal(exportFileName({ ...options, anchorIds: ["1", "1"], format: "csv" }), "直播消息-小野uu-2026-10.csv");
});

test("single-anchor defaults cannot introduce a directory or Windows-invalid file characters", () => {
  const result = exportFileName({ ...options, anchorIds: ["1"], anchors: [{ anchor_id: "1", anchor_name: ' ../阿梨\\直播:*?"<>|\n. ' }] });
  assert.equal(result, "直播消息-.._阿梨_直播________-2026-10.xlsx");
  assert.doesNotMatch(result, /[<>:"/\\|?*\u0000-\u001f]/);
});

test("missing or unusable names fall back to the selected anchor ID", () => {
  assert.equal(exportFileName({ ...options, anchorIds: ["3"] }), "直播消息-主播-3-2026-10.xlsx");
  assert.equal(exportFileName({ ...options, anchorIds: ["1"], anchors: [{ anchor_id: "1", anchor_name: '  /\\  ' }] }), "直播消息-主播-1-2026-10.xlsx");
});

test("long names are bounded without cutting emoji surrogate pairs", () => {
  const result = exportFileName({ ...options, anchorIds: ["1"], anchors: [{ anchor_id: "1", anchor_name: "中" + "😀".repeat(60) }] });
  assert.equal(result, `直播消息-中${"😀".repeat(39)}-2026-10.xlsx`);
  assert.ok(result.length < 110);
});

test("empty selections and invalid format or month do not yield a misleading filename", () => {
  assert.throws(() => exportFileName({ ...options, anchorIds: [] }));
  assert.throws(() => exportFileName({ ...options, anchorIds: ["1"], month: "2026-13" }));
  assert.throws(() => exportFileName({ ...options, anchorIds: ["1"], format: "exe" }));
});
