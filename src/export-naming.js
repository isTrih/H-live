// Windows save-dialog defaults. Names remain editable in the native dialog.
function fileLabel(value) {
  const cleaned = String(value ?? "")
    .replace(/[<>:"/\\|?*\u0000-\u001f\u007f]/g, "_")
    .trim().replace(/[. ]+$/g, "");
  let result = "";
  // Bound the component in UTF-16 units without splitting an emoji surrogate pair.
  for (const character of cleaned) {
    if (result.length + character.length > 80) break;
    result += character;
  }
  result = result.replace(/[. ]+$/g, "");
  return /^[_ .]*$/.test(result) ? "" : result;
}

export function exportFileName({ anchorIds, anchors, month, format }) {
  if (!/^\d{4}-(0[1-9]|1[0-2])$/.test(month)) throw new Error("请选择有效月份");
  if (!["xlsx", "csv"].includes(format)) throw new Error("请选择 XLSX 或 CSV");
  const ids = [...new Set(Array.from(anchorIds, String))];
  if (!ids.length) throw new Error("请至少选择一个已归档主播");
  if (ids.length > 1) return `直播消息-${month}.${format}`;
  const anchor = anchors.find(item => String(item.anchor_id) === ids[0]);
  const label = fileLabel(anchor?.anchor_name) || `主播-${fileLabel(ids[0]) || "未知"}`;
  return `直播消息-${label}-${month}.${format}`;
}
