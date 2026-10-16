import { esc, icon, num } from "./presentation.js";

export function deletionFilter({ anchorIds, scope, month, sessionIds = [] }) {
  if (!anchorIds.length) throw new Error("请先选择要清理的主播");
  const result = { anchor_ids: [...anchorIds] };
  if (scope === "month" || (scope === "sessions" && month)) {
    if (!/^\d{4}-(0[1-9]|1[0-2])$/.test(month)) throw new Error("请选择有效月份");
    result.month = month;
  }
  if (scope === "sessions") {
    if (!sessionIds.length) throw new Error("请至少选择一个场次");
    result.session_ids = [...sessionIds];
  } else if (!["month", "anchors"].includes(scope)) {
    throw new Error("请选择删除范围");
  }
  return result;
}

export function createArchiveDeletion({ state, bridge, modal, closeModal, toast, onDeleted }) {
  const current = () => state.modal?.kind === "archive-delete" ? state.modal : null;
  const sessionTime = value => {
    if (!value) return "开播时间未提供";
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? "开播时间未提供" : date.toLocaleString("zh-CN", { timeZone: "Asia/Shanghai", hour12: false });
  };
  function render(context) {
    const sessions = context.scope === "sessions";
    modal("删除聊天记录", `<p class="archive-delete-anchors">已选主播：${context.anchors.map(a => esc(a.anchor_name)).join("、")}</p>
      <div class="field"><label for="delete-scope">删除范围</label><select id="delete-scope">
        <option value="month" ${context.scope === "month" ? "selected" : ""}>指定月份</option>
        <option value="sessions" ${sessions ? "selected" : ""}>指定直播场次</option>
        <option value="anchors" ${context.scope === "anchors" ? "selected" : ""}>所选主播的全部记录</option></select></div>
      ${context.scope !== "anchors" ? `<div class="field"><label for="delete-month">${sessions ? "按月份筛选场次（留空显示全部）" : "月份"}</label><input id="delete-month" type="month" value="${esc(context.month)}"></div>` : ""}
      ${sessions ? `<button data-action="delete-load-sessions">${icon("refresh")}读取场次</button><div id="delete-sessions" class="archive-session-picker">${context.sessions === null ? '<p class="help">读取后选择要删除的场次。</p>' : context.sessions.length ? context.sessions.map(s => `<label><input type="checkbox" data-delete-session="${esc(s.session_id)}" ${context.selected.has(s.session_id) ? "checked" : ""}><span><strong>${esc(s.anchor_name)}</strong><small>${esc(sessionTime(s.started_at))} · ${num(s.records_count)} 条</small><small class="mono">场次 ${esc(s.session_id)}</small></span></label>`).join("") : '<p class="help">此范围没有已归档场次。</p>'}</div>` : ""}
      ${sessions && context.nextOffset != null ? `<button class="small" data-action="delete-more-sessions">加载更多场次 · 已显示 ${num(context.sessions.length)} / ${num(context.total)}</button>` : ""}
      <p class="help">删除范围包含所有消息类型。已有导出文件保留。</p>`,
      `<button data-action="close-modal">取消</button><button class="danger" data-action="delete-preview">${icon("search")}预览删除范围</button>`);
  }
  function open() {
    const anchors = state.archiveAnchors.filter(a => state.exportSelection.has(a.anchor_id));
    if (!anchors.length) throw new Error("请先在下方选择要清理的主播");
    const context = { kind: "archive-delete", anchors, scope: "month", month: state.exportMonth,
      sessions: null, selected: new Set(), preview: null, generation: 0, nextOffset: null, total: 0 };
    state.modal = context;
    render(context);
  }
  function change(target) {
    const context = current();
    if (!context) return;
    if (target.id === "delete-scope" || target.id === "delete-month") {
      if (target.id === "delete-scope") context.scope = target.value;
      else context.month = target.value;
      context.sessions = null;
      context.nextOffset = null;
      context.total = 0;
      context.selected.clear();
      context.preview = null;
      context.generation++;
      render(context);
    }
    if (target.dataset.deleteSession) {
      target.checked ? context.selected.add(target.dataset.deleteSession) : context.selected.delete(target.dataset.deleteSession);
      context.generation++;
      context.preview = null;
    }
  }
  async function loadSessions(more = false) {
    const context = current();
    if (!context || context.scope !== "sessions") return;
    if (more && (!Array.isArray(context.sessions) || !Number.isInteger(context.nextOffset) || context.nextOffset < 0)) return;
    const generation = ++context.generation;
    const result = await bridge("archive_sessions", { anchor_ids: context.anchors.map(a => a.anchor_id),
      offset: more ? context.nextOffset : 0, limit: 200,
      ...(context.month ? { month: context.month } : {}) });
    if (current() !== context || context.generation !== generation) return;
    context.sessions = more ? [...context.sessions, ...result.sessions] : result.sessions;
    context.nextOffset = result.next_offset;
    context.total = result.total;
    if (!more) context.selected.clear();
    render(context);
  }
  async function preview() {
    const context = current();
    if (!context) return;
    const generation = context.generation;
    const filter = deletionFilter({ anchorIds: context.anchors.map(a => a.anchor_id), scope: context.scope,
      month: context.month, sessionIds: [...context.selected] });
    const result = await bridge("preview_archive_delete", filter);
    if (current() !== context || generation !== context.generation) return;
    context.preview = { filter, result };
    modal("确认删除聊天记录", `<p class="archive-delete-anchors">${context.anchors.map(a => esc(a.anchor_name)).join("、")}</p>
      <div class="archive-delete-count"><strong>${num(result.records_count)}</strong><span>条消息 · ${num(result.sessions_count)} 个场次</span></div>
      <p>${filter.month ? `${esc(filter.month)} 月` : "全部月份"}${filter.session_ids ? ` · 已选 ${num(filter.session_ids.length)} 个场次` : ""}</p>
      ${result.ready ? '<p class="danger">删除后无法恢复，请确认已完成需要的导出。</p>' : `<p class="proxy-error">${esc(result.blocked_reason)}</p>`}`,
      `<button data-action="delete-back">返回</button><button class="danger" data-action="delete-execute" ${!result.ready || !result.records_count ? "disabled" : ""}>${icon("trash")}删除 ${num(result.records_count)} 条消息</button>`);
  }
  async function execute() {
    const context = current();
    if (!context?.preview?.result.ready) return;
    const job = await bridge("execute_archive_delete", context.preview.filter);
    if (current() === context) modal("正在删除聊天记录", '<p role="status">正在清理所选范围，请保持应用运行。完成后会通知你。</p>', '<button data-action="close-modal">收起</button>');
    let status;
    try {
      do {
        await new Promise(resolve => setTimeout(resolve, 500));
        status = await bridge("archive_delete_status", { job_id: job.job_id });
      } while (["running", "queued"].includes(status.status));
    } catch (error) {
      if (current() === context) modal("删除结果待确认", '<p>暂时无法读取后台结果，请重新连接后检查归档记录。</p>', '<button data-action="close-modal">关闭</button>');
      throw error;
    }
    if (status.status !== "completed") {
      if (current() === context) modal("删除未完成", `<p>${esc(status.error || "本次操作已取消")}</p>`, '<button data-action="close-modal">关闭</button>');
      throw new Error(status.error || "本次操作已取消");
    }
    const result = status.report;
    if (current() === context) closeModal();
    await onDeleted();
    toast(`已删除 ${num(result.deleted_records)} 条聊天消息`);
  }
  return { open, change, loadSessions, preview, execute, back: () => { const context = current(); if (context) render(context); } };
}
