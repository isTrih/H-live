import test from "node:test";
import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import {normalizeTags,anchorTags,tagMatches,tagOptions,tagsMarkup,filterArchiveAnchors,UNTAGGED_TAG} from "./anchor-tags.js";

test("tag input trims, splits commas and lines, and deduplicates without changing case", () => {
  assert.deepEqual(normalizeTags(" 游戏， 重点关注\n游戏\r\n音乐, ,Music,music "), ["游戏", "重点关注", "音乐", "Music", "music"]);
  const source = [" 游戏 ", "游戏", "", "音乐"];
  assert.deepEqual(normalizeTags(source), ["游戏", "音乐"]);
  assert.deepEqual(source, [" 游戏 ", "游戏", "", "音乐"]);
  assert.deepEqual(normalizeTags(" ,， \n"), []);
  assert.deepEqual(normalizeTags([]), []);
});

test("tag limits count Unicode code points and unique nonempty tags", () => {
  const emoji = "😀".repeat(24), twenty = Array.from({length: 20}, (_, i) => `标签${i}`);
  assert.deepEqual(normalizeTags([emoji]), [emoji]);
  assert.equal(normalizeTags([...twenty, twenty[0], " "]).length, 20);
  assert.throws(() => normalizeTags(["😀".repeat(25)]), /24/);
  assert.throws(() => normalizeTags([...twenty, "第二十一个"]), /20/);
  for (const input of [null, undefined, 2, {}, [3], [null], ["a\tb"], ["a\u0000b"], ["a\u0085b"], ["a,b"], ["a，b"], ["a\nb"]]) {
    assert.throws(() => normalizeTags(input), /标签/);
  }
});

test("snapshot tag reading tolerates old or malformed entries and returns an isolated array", () => {
  for (const anchor of [null, {}, {tags: "游戏"}, {tags: 4}]) assert.deepEqual(anchorTags(anchor), []);
  const anchor = {tags: [null, " 游戏 ", "游戏", 3, "", "bad\u0000tag", "太".repeat(25), "音乐"]};
  const tags = anchorTags(anchor);
  assert.deepEqual(tags, ["游戏", "音乐"]);
  tags.push("新标签");
  assert.equal(anchor.tags.includes("新标签"), false);
  assert.equal(anchorTags({tags: Array.from({length: 30}, (_, i) => `标签${i}`)}).length, 20);
});

test("all, untagged and real tag filters stay distinct and options are sorted and unique", () => {
  const tagged = {tags: ["无标签", "b", "a"]}, empty = {tags: []};
  assert.equal(tagMatches(tagged, ""), true);
  assert.equal(tagMatches(empty), true);
  assert.equal(tagMatches(tagged, "无标签"), true);
  assert.equal(tagMatches(empty, "无标签"), false);
  assert.equal(tagMatches(tagged, UNTAGGED_TAG), false);
  assert.equal(tagMatches(empty, UNTAGGED_TAG), true);
  assert.equal(tagMatches({}, UNTAGGED_TAG), true);
  assert.throws(() => normalizeTags([UNTAGGED_TAG]), /24/);
  const options = tagOptions([tagged, {tags: ["a", "b"]}, {}]);
  assert.deepEqual(new Set(options), new Set(["a", "b", "无标签"]));
  assert.equal(options.length, 3);
  assert.ok(options.indexOf("a") < options.indexOf("b"));
  assert.deepEqual(tagOptions(null), []);
});

test("tag badges escape text and titles, limit visible badges and retain the full title", () => {
  const html = tagsMarkup({tags: ["<img onerror=alert(1)>", "引号\"&'", "最后一个"]}, {limit: 2});
  assert.doesNotMatch(html, /<img|<script|onclick=/);
  assert.match(html, /&lt;img onerror=alert\(1\)&gt;/);
  assert.match(html, /引号&quot;&amp;&#39;/);
  assert.equal((html.match(/class="anchor-tag"/g) || []).length, 2);
  assert.match(html, /class="anchor-tags-more">\+1/);
  assert.match(html, /title="[^"]*最后一个/);
  assert.equal(tagsMarkup({}), "");
  assert.match(tagsMarkup({tags: ["游戏"]}, {limit: 0}), /\+1/);
});

test("archive filtering combines name or exact ID text search with tag selection", () => {
  const anchors = [
    {anchor_id: "7698991795467832114", anchor_name: "Test主播", web_rid: "111002", tags: ["游戏"]},
    {anchor_id: "22", name: "音乐主播", rid: "222003", tags: ["音乐"]},
    {anchor_id: "33", anchor_name: "旧主播"},
  ];
  assert.deepEqual(filterArchiveAnchors(anchors, " TEST ", "游戏"), [anchors[0]]);
  assert.deepEqual(filterArchiveAnchors(anchors, "Test", "音乐"), []);
  assert.deepEqual(filterArchiveAnchors(anchors, "7698991795467832114"), [anchors[0]]);
  assert.deepEqual(filterArchiveAnchors(anchors, "222003"), [anchors[1]]);
  assert.deepEqual(filterArchiveAnchors(anchors, "", UNTAGGED_TAG), [anchors[2]]);
  assert.deepEqual(filterArchiveAnchors(null), []);
  assert.equal(anchors.length, 3);
});

test("preview tags survive polling, apply only to new imports, replace in batches and follow stable archive IDs", async t => {
  const previousLocation = globalThis.location, previousStorage = globalThis.localStorage;
  const memory = new Map();
  globalThis.location = {search: ""};
  globalThis.localStorage = {getItem: key => memory.get(key) ?? null, setItem: (key, value) => memory.set(key, value)};
  t.after(() => {
    if (previousLocation === undefined) delete globalThis.location; else globalThis.location = previousLocation;
    if (previousStorage === undefined) delete globalThis.localStorage; else globalThis.localStorage = previousStorage;
  });
  // Node does not support the browser's named JSON import. Load the real preview module with
  // only that build-time version replaced, preserving all tag and snapshot behavior.
  const file = new URL("./preview.js", import.meta.url);
  const source = (await readFile(file, "utf8"))
    .replace('import {version as appVersion} from "../package.json";', 'const appVersion = "test";')
    .replace(/from (["'])(\.\/[^"']+)\1/g, (_, quote, path) => `from ${JSON.stringify(new URL(path, file).href)}`);
  const {previewBridge} = await import(`data:text/javascript;base64,${Buffer.from(source).toString("base64")}`);
  const initial = await previewBridge("init"), first = initial.anchors[0], second = initial.anchors[1];
  assert.ok(first.tags.length && second.tags.length);
  await previewBridge("set_anchor_tags", {rids: [first.rid, second.rid], tags: ["回看", "游戏"]});
  const polled = await previewBridge("poll", {include_proxy: false});
  assert.deepEqual(polled.anchors.find(a => a.rid === first.rid).tags, ["回看", "游戏"]);
  assert.deepEqual(polled.archive_anchors.find(a => a.anchor_id === first.anchor_id).tags, ["回看", "游戏"]);
  polled.anchors.find(a => a.rid === first.rid).tags.push("外部改动");
  assert.equal((await previewBridge("init")).anchors.find(a => a.rid === first.rid).tags.includes("外部改动"), false);
  const added = await previewBridge("add_anchors", {values: `${first.rid}\n555555555555\n555555555555`, tags: ["新加入"]});
  assert.equal(added.added, 1);
  assert.equal((await previewBridge("add_anchors", {values: "555555555555", tags: ["覆盖尝试"]})).added, 0);
  const afterAdd = await previewBridge("poll");
  assert.deepEqual(afterAdd.anchors.find(a => a.rid === first.rid).tags, ["回看", "游戏"]);
  assert.deepEqual(afterAdd.anchors.find(a => a.rid === "555555555555").tags, ["新加入"]);
  await assert.rejects(previewBridge("set_anchor_tags", {rids: [first.rid, "not-found"], tags: ["不应写入"]}), /不存在/);
  assert.deepEqual((await previewBridge("init")).anchors.find(a => a.rid === first.rid).tags, ["回看", "游戏"]);
  await previewBridge("set_anchor_tags", {rids: [second.rid, "555555555555"], tags: []});
  assert.deepEqual((await previewBridge("init")).anchors.find(a => a.rid === second.rid).tags, []);
  await previewBridge("remove", {rids: [first.rid]});
  await previewBridge("add_anchors", {values: first.rid, tags: ["同入口新任务"]});
  const archive = await previewBridge("archive_anchors");
  assert.deepEqual(archive.find(a => a.anchor_id === first.anchor_id).tags, ["回看", "游戏"]);
});
