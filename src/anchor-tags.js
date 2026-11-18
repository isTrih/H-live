import {esc} from "./presentation.js";

const MAX_TAGS = 20, MAX_TAG_LENGTH = 24;
const invalidTag = /[\u0000-\u001f\u007f-\u009f,，]/;
// This value is longer than a valid tag, so a user tag can never select this filter.
export const UNTAGGED_TAG = "__livescope_anchor_without_tags__";

export function normalizeTags(input) {
  const values = typeof input === "string" ? input.split(/[,，\r\n]+/) : input;
  if (!Array.isArray(values)) throw Error("标签格式无效，请输入标签文字");
  const tags = [], seen = new Set();
  for (const value of values) {
    if (typeof value !== "string") throw Error("每个标签都必须是文字");
    const tag = value.trim();
    if (!tag) continue;
    if (invalidTag.test(tag)) throw Error("标签不能包含逗号、换行或控制字符");
    if ([...tag].length > MAX_TAG_LENGTH) throw Error("每个标签最多 24 个字符");
    if (!seen.has(tag)) { seen.add(tag); tags.push(tag); }
    if (tags.length > MAX_TAGS) throw Error("每位主播最多设置 20 个标签");
  }
  return tags;
}

// Older snapshots may omit tags or contain a malformed entry. Render the valid entries only.
export function anchorTags(anchor) {
  if (!Array.isArray(anchor?.tags)) return [];
  const tags = [], seen = new Set();
  for (const value of anchor.tags) {
    if (typeof value !== "string") continue;
    const tag = value.trim();
    if (!tag || invalidTag.test(tag) || [...tag].length > MAX_TAG_LENGTH || seen.has(tag)) continue;
    seen.add(tag); tags.push(tag);
    if (tags.length === MAX_TAGS) break;
  }
  return tags;
}

export function tagMatches(anchor, filter = "") {
  if (!filter) return true;
  const tags = anchorTags(anchor);
  return filter === UNTAGGED_TAG ? tags.length === 0 : tags.includes(filter);
}

export function tagOptions(anchors) {
  const tags = new Set((Array.isArray(anchors) ? anchors : []).flatMap(anchorTags));
  return [...tags].sort((a, b) => a.localeCompare(b, "zh-CN") || (a < b ? -1 : a > b ? 1 : 0));
}

export function tagsMarkup(anchor, {limit = 3} = {}) {
  const tags = anchorTags(anchor);
  if (!tags.length) return "";
  const shown = Number.isInteger(limit) && limit >= 0 ? Math.min(limit, MAX_TAGS) : 3;
  const badges = tags.slice(0, shown).map(tag => `<span class="anchor-tag" title="${esc(tag)}">${esc(tag)}</span>`).join("");
  const more = tags.length > shown ? `<span class="anchor-tags-more">+${tags.length - shown}</span>` : "";
  return `<span class="anchor-tags" title="${esc(tags.join("、"))}">${badges}${more}</span>`;
}

export function filterArchiveAnchors(anchors, search = "", tag = "") {
  const query = String(search ?? "").trim().toLocaleLowerCase("zh-CN");
  return (Array.isArray(anchors) ? anchors : []).filter(anchor => tagMatches(anchor, tag) &&
    (!query || [anchor?.anchor_name, anchor?.name, anchor?.anchor_id, anchor?.web_rid, anchor?.rid]
      .some(value => String(value ?? "").toLocaleLowerCase("zh-CN").includes(query))));
}
