import { num } from "../../presentation.js";

export const active = (a) =>
  Boolean(
    a.active ?? !["stopped", "error", "idle", ""].includes(a.state || ""),
  );
export const live = (a) =>
  active(a) && !faulty(a) && !["waiting_proxy", "confirming_offline"].includes(a.state) && a.is_live === true;
export const offline = (a) =>
  active(a) && !faulty(a) && !["waiting_proxy", "confirming_offline"].includes(a.state) && a.is_live === false;
export const faulty = (a) => ["error", "retrying", "failed"].includes(a.state);
export const pcUnsupported = (a) => a.pc_unsupported === true;
export const messageConnection = (a) => !active(a) ? "未监控" : a.state === "connected" ? "消息已连接" : "消息未连接";
export const rank = (a) => (live(a) ? 0 : offline(a) ? 3 : faulty(a) ? 1 : 2);
export const status = (a) =>
  pcUnsupported(a) ? ["unsupported", "非 PC 支持"] : a.state === "confirming_offline" ? ["waiting", "疑似下播"] : faulty(a)
    ? ["error", "状态待确认"]
    : live(a)
      ? ["online", "开播中"]
      : offline(a)
        ? ["offline", "未开播"]
        : faulty(a)
          ? ["error", "连接异常"]
          : a.state === "waiting_proxy"
            ? ["waiting", "等待代理"]
            : active(a)
              ? ["waiting", "查询中"]
              : ["idle", "未监控"];
export function selectAnchors(anchors, search = "", filter = "all") {
  const q = search.trim().toLowerCase();
  return anchors
    .filter(
      (a) =>
        (!q || `${a.name} ${a.rid} ${a.anchor_id}`.toLowerCase().includes(q)) &&
        (filter === "all" ||
          (filter === "unsupported" && pcUnsupported(a)) ||
          (filter === "live" && live(a)) ||
          (filter === "offline" && offline(a)) ||
          (filter === "error" && faulty(a)) ||
          (filter === "unknown" &&
            !live(a) &&
            !offline(a) &&
            !faulty(a))),
    )
    .sort((a, b) => {
      const group = rank(a), groupOrder = group - rank(b);
      if (groupOrder) return groupOrder;
      if (group === 0) {
        const aCount = Number.isFinite(a.viewer_count) && a.viewer_count >= 0 ? a.viewer_count : -1;
        const bCount = Number.isFinite(b.viewer_count) && b.viewer_count >= 0 ? b.viewer_count : -1;
        if (aCount !== bCount) return bCount - aCount;
      }
      return String(a.name).localeCompare(String(b.name), "zh-CN") ||
        String(a.rid).localeCompare(String(b.rid));
    });
}
export function duration(a, now = Date.now() / 1000) {
  if (!live(a)) return "—";
  let elapsed;
  if (a.duration_seconds != null) {
    if (!Number.isFinite(a.duration_seconds) || a.duration_seconds < 0) return "—";
    elapsed = a.duration_seconds;
    if (a.duration_running === true && Number.isFinite(a.duration_updated_at)) {
      elapsed += Math.max(0, now - a.duration_updated_at);
    }
  } else {
    const startedAt = a.duration_started_at ?? a.broadcast_started_at;
    if (!startedAt) return "—";
    elapsed = now - Number(startedAt);
  }
  const seconds = Math.floor(elapsed);
  if (!Number.isFinite(seconds) || seconds < 0) return "—";
  const h = Math.floor(seconds / 3600), m = Math.floor(seconds / 60) % 60, sec = seconds % 60;
  return [h, m, sec].map(v => String(v).padStart(2, "0")).join(":");
}
export function viewerCount(a) {
  return live(a) && a.viewer_count != null && Number.isFinite(Number(a.viewer_count)) ? num(a.viewer_count) : "—";
}
export function compactViewerCount(a) {
  const count = viewerCount(a);
  return count !== '—' && Number(a.viewer_count) >= 10000
    ? new Intl.NumberFormat('zh-CN', {notation:'compact', maximumFractionDigits:1}).format(a.viewer_count)
    : count;
}
