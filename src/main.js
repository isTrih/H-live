import {createRecentMessages} from "./recent-messages.js";
import {createViewerRefresh} from "./viewer-refresh.js";
import "./session-quality.css";
import { icon, esc, num } from "./presentation.js";
import { active, live, offline, faulty, status, selectAnchors, duration, viewerCount, compactViewerCount, pcUnsupported, messageConnection } from "./features/monitor/model.js";
import { activityView } from './features/monitor/activities.js';
import { paintClock } from './features/monitor/flip-clock.js';
import { invoke as nativeInvoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./style.css";
import "./desktop.css";
import "./fonts.css";
import "./proxy-import.css";
import "./archive-delete.css";
import { createArchiveDeletion } from "./archive-delete.js";
import { gridSectionWindow } from "./virtual-grid.js";
import { exportFileName } from "./export-naming.js";
import brandIconUrl from "./brand-icon.png";
import redPacketUrl from './assets/red-packet.png';
import lotteryBagUrl from './assets/lottery-bag.png';
import { version as appVersion } from "../package.json";
import { updatePanelMarkup, renderUpdatePanel, releaseNotesMarkup, bundledReleaseNotes } from "./update-panel.js";
import { getUpdaterState, subscribeUpdater, checkForUpdates, installUpdate } from "./updater.js";
import { createSessionQuality } from "./session-quality.js";
import { createAutomaticUpdates, automaticUpdatesEnabled, saveAutomaticUpdates } from "./automatic-updates.js";
import { proxyAssignments } from "./proxy-assignment.js";
import "./workspace.css";
import "./features/monitor/flip-clock.css";
import "./settings-tools.css";
import {createSessionTimeline} from "./session-timeline.js";
import {createReplayMedia} from "./replay-media.js";
import {createNativeReplayMedia} from "./native-replay-media.js";
import "./session-timeline.css";
import {createHighlightSettings} from "./highlight-settings.js";
import "./highlight-settings.css";
import {createEpisodeTooltip} from "./episode-tooltip.js";
import {createActivityTooltip} from "./activity-tooltip.js";
import {createStartupSettings} from "./startup-settings.js";
import {createUpdateNotice} from "./update-notice.js";
import {createCommunitySettings} from "./community-settings.js";
import "./replay-player.css";
import {createClipPanel} from "./clip-panel.js";
import "./clip-panel.css";
import {createDownloadSettings} from "./download-settings.js";
import "./download-settings.css";
import {createDataBackup} from "./data-backup.js";
import "./data-backup.css";
import "./session-bookmarks.css";
import {createTimeDisplaySettings} from "./time-display-settings.js";
import {setDisplayTimezone} from "./time-display.js";
import "./time-display-settings.css";
import {createCompatibilityChecks} from "./compatibility-checks.js";
import "./compatibility-checks.css";
import "./selection-policy.css";
import {normalizeTags, anchorTags, tagMatches, tagOptions, tagsMarkup, filterArchiveAnchors, UNTAGGED_TAG} from "./anchor-tags.js";
import "./anchor-tags.css";
const SERVER_MODE = import.meta.env.MODE === "server";
let invoke = nativeInvoke;

const TYPES = {
  chat: "聊天",
  gift: "礼物（历史）",
  like: "点赞",
  member: "进入",
  social: "关注 / 分享",
  fansclub: "粉丝团",
  emoji: "表情",
  activity: "红包 / 福袋",
};
const LIVE_TYPES = Object.fromEntries(Object.entries(TYPES).filter(([kind]) => kind !== "member" && kind !== "gift"));
const NAV = [
  ["monitor", "grid", "监控看板"],
  ["archive", "download", "导出数据"],
  ["clips", "play", "录像下载"],
  ["settings", "settings", "设置"],
];
const primaryPage = page => page === "messages" ? "monitor" : page;
const state = {
  page: "monitor",
  anchors: [],
  archiveAnchors: [],
  settings: {},
  metrics: {},
  proxy: {},
  proxyRefreshRequest: false,
  proxyConfigBusy: false,
  debugConfigBusy: false,
  proxySearch: "",
  proxyPage: 0,
  cursor: 0,
  messages: [],
  logs: [],
  jobs: [],
  selected: new Set(),
  selecting: false,
  font: "douyin",
  search: "",
  filter: "all",
  tagFilter: "",
  view: "grid",
  offlineExpanded: false,
  connected: false,
  error: "",
  paused: false,
  messageType: "all",
  messageAnchor: "",
  modal: null,
  preview: false,
  dropped: 0,
  exportSelection: new Set(),
  exportFormat: "xlsx",
  exportTypes: new Set(Object.keys(TYPES)),
  exportMonth: new Intl.DateTimeFormat("sv-SE", {
    timeZone: "Asia/Shanghai",
    year: "numeric",
    month: "2-digit",
  }).format(new Date()),
  exportSearch: "",
  exportTag: "",
  exportBusy: false,
};
let bridge = (method, params = {}) =>
  invoke("backend_request", { method, params });
const messageFeed=createRecentMessages({bridge:(...args)=>bridge(...args),getFilter:()=>({active:state.page==="messages",paused:state.paused,type:state.messageType,web_rid:state.messageAnchor}),onChange:()=>renderMessages()});
const viewerRefresh=createViewerRefresh({bridge:(...args)=>bridge(...args),changed:()=>renderViewerRefresh(),notify:toast});
const highlightSettings=createHighlightSettings({bridge:(...args)=>bridge(...args),toast});
const startupSettings=createStartupSettings({invoke:(...args)=>invoke(...args),request:(...args)=>request(...args),preview:()=>state.preview,toast});
const communitySettings=createCommunitySettings({bridge:(...args)=>bridge(...args),toast,onSaved:()=>sessionTimeline.refreshReplay()});
const episodeTooltip=createEpisodeTooltip();
const activityTooltip=createActivityTooltip();
const downloadSettings=createDownloadSettings({bridge:(...args)=>bridge(...args),invoke:(command,params)=>state.preview&&command==='choose_download_directory'?Promise.resolve('C:/氢看下载'):invoke(command,params),toast});
const timeDisplaySettings=createTimeDisplaySettings({bridge:(...args)=>bridge(...args),toast,onSaved:()=>sessionTimeline.refreshTimeDisplay()});
let previewBackup=null;
const dataBackup=createDataBackup({invoke:async(command,params)=>{
  if(import.meta.env.DEV&&state.preview){previewBackup??=(await import("./preview-backup.js")).createPreviewBackup({version:appVersion});return previewBackup.handle(command,params);}
  return invoke(command,params);
},toast,onRestored:()=>refreshAfterBackupRestore()});
const clipPanel=createClipPanel({bridge:(...args)=>bridge(...args),invoke:(command,params)=>state.preview&&command==='open_clip_folder'?Promise.reject(Error('预览任务未生成本地文件')):invoke(command,params),toast,
  onConfigure:()=>void navigate('settings').then(()=>document.querySelector('#download-settings')?.scrollIntoView({block:'start'}))});
const compatibilityChecks=createCompatibilityChecks({invoke:(command,params)=>state.preview?Promise.resolve({preview:true,app_version:appVersion}):invoke(command,params),toast});
const sessionTimeline=createSessionTimeline({state,bridge:(...args)=>bridge(...args),toast,
  createMedia:options=>(!SERVER_MODE&&!state.preview?createNativeReplayMedia:createReplayMedia)(options),
  getMediaTransport:()=>!SERVER_MODE&&!state.preview?({url,rangeStart,rangeEnd})=>invoke("replay_media_fetch",{url,rangeStart,rangeEnd}):null,
  onCreateClip:async clip=>{
    if(SERVER_MODE)throw Error("录像下载请使用桌面客户端");
    const result=await bridge("clip_create",clip);
    if(clip.source!=='bookmark')toast(clip.mode==='full_session'?"已加入整场下载任务":"已加入切片下载任务");
    return result;
  },
  onRender:()=>activityTooltip.refresh(),onConfigure:()=>{
  closeModal();
  void navigate("settings").then(()=>document.querySelector("#highlight-base-url")?.focus({preventScroll:true}));
}});
const sessionQuality = createSessionQuality({state, bridge:(method,params)=>bridge(method,params), invoke:(...args)=>invoke(...args), toast,
  onSession: sessionId=>sessionTimeline.selectSession(sessionId),onContext:(_date,overview)=>sessionTimeline.updateQuality(overview),onRender:()=>episodeTooltip.refresh()});
const automaticUpdates = createAutomaticUpdates({check:checkForUpdates,getState:getUpdaterState,
  onAvailable:update=>updateNotice.observe(update)});
const updateNotice=createUpdateNotice({canOpen:()=>!state.modal,show:showUpdateNotice});
function showUpdateNotice(update) {
  renderUpdateBadge(update);
  state.modal={kind:"update-available"};
  modal("发现新版本", `<div class="update-identity"><img src="${brandIconUrl}" alt="" width="72" height="72"><div><span class="update-kicker">氢看直播</span><h3>v${esc(update.version)} 可更新</h3><p>当前 v${esc(appVersion)}</p></div></div><div class="update-notes-scroll">${releaseNotesMarkup(update.body)}</div><p class="update-restart-note">安装后自动重启，恢复正在监听的主播。</p>`, `<button data-action="close-modal">稍后</button><button class="primary" data-action="install-update">下载并更新</button>`);
  document.querySelector(".modal").classList.add("update-available-dialog");
}
function renderUpdateBadge(update) {
  const settings = document.querySelector('[data-action="navigate"][data-page="settings"]');
  settings?.classList.toggle("has-update", Boolean(update.version));
  if (settings) { settings.setAttribute("aria-label", update.version ? `设置，有新版本 ${update.version}` : "设置"); settings.dataset.updateLabel=update.version ? "新版本" : ""; }
}
async function prepareBridge() {
  if (SERVER_MODE) {
    const remote = await import("./server/bridge.js");
    await remote.connect();
    bridge = remote.bridge;
    invoke = remote.invoke;
    document.documentElement.classList.add("server-mode");
    return;
  }
  if (
    import.meta.env.DEV &&
    new URLSearchParams(location.search).has("preview")
  ) {
    const { previewBridge } = await import("./preview.js");
    bridge = previewBridge;
    state.preview = true;
  }
}
function setFont(font, persist = true) {
  state.font = font === "chill" ? "chill" : "douyin";
  document.documentElement.dataset.font = state.font;
  if (persist) {
    try { localStorage.setItem("livescope.appearance.font", state.font); } catch (_) {}
  }
  scheduleTasks();
}
const time = (v) => {
  if (!v) return "—";
  const d = new Date(typeof v === "number" ? v * 1000 : v);
  return Number.isNaN(d.getTime())
    ? String(v)
    : d.toLocaleTimeString("zh-CN", {
        timeZone: "Asia/Shanghai",
        hour12: false,
      });
};
const statusBadge = (a) => {
  const [c, t] = status(a);
  return `<span class="status ${c}"><i class="dot"></i>${t}</span>${pcUnsupported(a) ? `<small class="message-connection">${messageConnection(a)}</small>` : ""}`;
};
const empty = (title, description, ico = "archive") =>
  `<div class="empty-state">${icon(ico)}<strong>${esc(title)}</strong><p>${esc(description)}</p></div>`;
const getJobs = (value) =>
  Array.isArray(value)
    ? value
    : Object.entries(value || {}).map(([id, v]) => ({ id, ...v }));

function shell() {
  document.querySelector("#app").innerHTML = `<div class="app-shell">
  <header class="window-toolbar" data-tauri-drag-region>
   <div class="window-left"><div class="window-controls" role="group" aria-label="窗口控制">
    <button class="traffic close" data-action="window-close" title="收起到托盘" aria-label="收起到系统托盘"><svg class="traffic-symbol" viewBox="0 0 12 12" aria-hidden="true"><path d="m3.5 3.5 5 5m0-5-5 5" fill="none" stroke="currentColor" stroke-width="1.2"/></svg></button>
    <button class="traffic minimize" data-action="window-minimize" title="最小化" aria-label="最小化窗口"><svg class="traffic-symbol" viewBox="0 0 12 12" aria-hidden="true"><path d="M3 6h6" fill="none" stroke="currentColor" stroke-width="1.2"/></svg></button>
    <button class="traffic maximize" data-action="window-maximize" title="最大化" aria-label="最大化窗口"><svg class="traffic-symbol traffic-expand" viewBox="0 0 12 12" aria-hidden="true"><path d="M3 3h4L3 7zm6 6H5l4-4z" fill="currentColor"/></svg><svg class="traffic-symbol traffic-restore" viewBox="0 0 12 12" aria-hidden="true"><path d="M5 5H1l4-4zm2 2h4l-4 4z" fill="currentColor"/></svg></button>
   </div><strong class="wordmark" data-tauri-drag-region><img class="brand-icon" src="${brandIconUrl}" alt="" aria-hidden="true" draggable="false" data-tauri-drag-region>氢看直播</strong></div>
   <nav class="top-nav">${NAV.filter(([id])=>!SERVER_MODE||id!=="clips").map(([id, ic, label]) => `<button class="nav-btn ${primaryPage(state.page) === id ? "selected" : ""}" data-action="navigate" data-page="${id}" title="${label}">${icon(ic)}<span>${label}</span></button>`).join("")}</nav>
   <div class="window-right"><span class="connection" id="connection"><i class="dot"></i>连接中</span><button class="icon-btn quiet" data-action="open-data" title="数据目录" aria-label="打开本地数据目录">${icon("folder")}</button></div>
  </header>
  ${state.preview ? '<div class="preview-banner">界面预览 · 300 个模拟任务</div>' : ""}
  <main class="main"><div class="workspace"><div id="error-area"></div><div id="page"></div></div></main>
 </div><div id="modal-root"></div><div class="toast-stack" aria-live="polite"></div>`;
  renderPage();
  updateConnection();
}
function heading(_eyebrow, title, _desc, actions = "") {
  return `<div class="page-heading"><h1>${title}</h1><div class="heading-actions">${actions}</div></div>`;
}
function proxyPanelMarkup() {
  return `<div class="proxy-settings-actions">` + `<button data-action="subscribe-proxy">${icon("network")}添加订阅</button><button data-action="import-proxy">${icon("plus")}导入代理</button><button class="primary" data-action="refresh-proxy">${icon("refresh")}获取并检测</button>` + `</div>` + `<section class="proxy-configuration"><label class="switch-label"><input type="checkbox" id="proxy-enabled" ${state.settings.proxy_enabled ? "checked" : ""}>启用代理</label><div class="source-control"><label for="proxy-source">来源</label><select id="proxy-source"><option value="public">公共代理池 · SCDN</option><option value="imported">已导入 / URL 订阅</option></select></div><label class="switch-label"><input type="checkbox" id="proxy-direct">同时使用本机直连</label><button data-action="manage-proxies">管理代理</button></section><div class="proxy-summary" id="proxy-summary"></div><p id="proxy-batch" class="help" role="status" hidden></p><div id="proxy-error"></div><section class="panel proxy-section"><div class="panel-header"><h2 class="panel-title">出口负载</h2><span class="badge" id="proxy-source-label"></span></div><div id="proxy-routes"></div></section><section class="panel proxy-section" id="proxy-subscriptions-panel" hidden><div class="panel-header"><h2 class="panel-title">URL 订阅</h2></div><div id="proxy-subscriptions"></div></section><details class="panel proxy-assignments"><summary>任务分配 <span id="proxy-assignment-count"></span></summary><div class="toolbar"><label class="search">${icon("search")}<input id="proxy-search" placeholder="搜索主播或出口" aria-label="搜索任务分配" value="${esc(state.proxySearch)}"></label><div id="proxy-assignment-pages" class="assignment-pages"></div></div><div id="proxy-leases"></div></details><p class="help" id="proxy-help"></p>`;
}
function settingsToolsMarkup() {
  return `<details class="panel settings-disclosure" id="proxy-settings"><summary><span><strong>代理池</strong><small id="proxy-setting-state">${state.settings.proxy_enabled ? "已启用" : "已关闭 · 本机直连"}</small></span></summary><div class="settings-disclosure-body" id="proxy-settings-content"></div></details>
    <details class="panel settings-disclosure" id="diagnostics-settings"><summary><span><strong>诊断</strong><small>排查问题时使用</small></span></summary><div class="settings-disclosure-body"><div class="diagnostics-option"><label class="switch-label"><input type="checkbox" id="debug-logging" ${state.settings.debug_logging ? "checked" : ""}>调试模式</label><p class="help">开启后记录连接过程和未开播检查，关闭后仅保存异常记录。</p></div><section id="debug-log-panel" ${state.settings.debug_logging ? "" : "hidden"}><div class="panel-header"><h3 class="panel-title">运行日志</h3><span class="badge">最近 200 条</span></div><div id="log-content"></div></section></div></details>`;
}
const proxyVisible = () => state.page === "settings" && Boolean(document.querySelector("#proxy-settings")?.open);
function renderSettingsTools() {
  const summary = document.querySelector("#proxy-setting-state");
  if (summary) summary.textContent = state.settings.proxy_enabled ? "已启用" : "已关闭 · 本机直连";
  const toggle = document.querySelector("#debug-logging");
  if (toggle) { toggle.checked = Boolean(state.settings.debug_logging); toggle.disabled = state.debugConfigBusy; }
  const panel = document.querySelector("#debug-log-panel");
  if (panel) panel.hidden = !state.settings.debug_logging;
}
async function configureDebugLogging(enabled) {
  if (state.debugConfigBusy) return;
  state.debugConfigBusy = true;
  renderSettingsTools();
  try {
    state.settings = await bridge("save_settings", {debug_logging: enabled});
    state.logs = [];
    toast(enabled ? "调试模式已开启" : "调试模式已关闭");
  } catch (error) {
    toast(String(error?.message || error), true);
  } finally {
    state.debugConfigBusy = false;
    renderSettingsTools();
    renderLogs();
  }
}
function renderPage() {
  clipPanel.unmount();
  downloadSettings.unmount();
  timeDisplaySettings.unmount();
  dataBackup.unmount();
  compatibilityChecks.unmount();
  const p = document.querySelector("#page");
  p.className = `page-${state.page}`;
  document.querySelector(".main").classList.toggle("monitor-view", state.page === "monitor");
  document
    .querySelectorAll(".nav-btn")
    .forEach((el) =>
      el.classList.toggle("selected", el.dataset.page === primaryPage(state.page)),
    );
  if (state.page === "monitor") {
    p.innerHTML = `<div class="overview-bar"><div class="stats-grid" id="stats"></div><div class="heading-actions"><button data-action="refresh-viewers" title="刷新正在监控的开播直播间在线人数">${icon("refresh")}刷新人数</button><button data-action="batch-stop">${icon("stop")}全部停止</button><button data-action="batch-start">${icon("play")}全部开始</button><button class="primary" data-action="add">${icon("plus")}添加主播</button></div></div>
  <section class="panel monitor-panel"><div class="toolbar"><div class="toolbar-group"><label class="search">${icon("search")}<input id="task-search" placeholder="搜索主播 / ID" aria-label="搜索主播" value="${esc(state.search)}"></label><select id="task-filter" aria-label="筛选直播状态"><option value="all">全部状态</option><option value="live">开播中</option><option value="offline">未开播</option><option value="unknown">待确认</option><option value="error">连接异常</option><option value="unsupported">非 PC 支持</option></select><select id="task-tag-filter" aria-label="按标签筛选主播"></select></div><div class="toolbar-group"><button class="small quiet" data-action="navigate" data-page="messages">${icon("chat")}消息流</button><button class="small quiet" data-action="toggle-selection">批量选择</button><div class="view-toggle"><button data-action="view-grid" class="${state.view === "grid" ? "active" : ""}" aria-label="宫格视图">${icon("grid")}</button><button data-action="view-list" class="${state.view === "list" ? "active" : ""}" aria-label="列表视图">${icon("list")}</button></div></div></div><div id="bulk-bar"></div><div id="task-content"></div></section>`;
    document.querySelector("#task-filter").value = state.filter;
    document.querySelector("#task-content").addEventListener("scroll", scheduleTasks, { passive: true });
    renderStats();
    renderTasks();
    scheduleTasks();
  } else if (state.page === "messages") {
    p.innerHTML =
      heading(
        "LIVE MESSAGES",
        "消息流",
        "聊天与互动按场次归档，运行日志独立展示。",
        `<button data-action="navigate" data-page="monitor">${icon("grid")}返回看板</button><button data-action="pause">${icon(state.paused ? "play" : "stop")}<span id="pause-label">${state.paused ? "恢复显示" : "暂停滚动"}</span></button><button data-action="navigate" data-page="archive">${icon("download")}导出数据</button>`,
      ) +
      `<section class="panel"><div class="toolbar"><div class="filter-chips">${[["all", "全部"], ...Object.entries(LIVE_TYPES)].map(([k, v]) => `<button class="chip ${state.messageType === k ? "active" : ""}" data-action="message-type" data-type="${k}">${v}</button>`).join("")}</div><select id="message-anchor" aria-label="选择主播"><option value="">全部主播</option>${state.anchors.map((a) => `<option value="${esc(a.rid)}">${esc(a.name)}</option>`).join("")}</select></div><div id="message-content"></div><div class="table-footer"><span id="message-count"></span><span class="live-indicator"><i class="dot"></i><span id="flow-label">持续归档</span></span></div></section>`;
    document.querySelector("#message-anchor").value = state.messageAnchor;
    renderMessages();
  } else if (state.page === "archive") {
    p.innerHTML =
      heading(
        "MONTHLY ARCHIVE",
        "导出数据",
        "选择主播与月份，导出完整场次消息，供内容质量分析使用。",
        `<button class="danger" data-action="delete-archive">${icon("trash")}删除记录</button>`,
      ) +
      `<div class="archive-layout"><section class="panel archive-form"><div class="panel-header"><h2 class="panel-title">导出消息</h2></div><div class="form-panel"><div class="field-row"><div class="field"><label for="export-month">月份</label><input id="export-month" type="month" value="${esc(state.exportMonth)}"></div><div class="field"><label>文件格式</label><div class="segment"><button class="${state.exportFormat === "xlsx" ? "active" : ""}" data-action="export-format" data-format="xlsx">XLSX</button><button class="${state.exportFormat === "csv" ? "active" : ""}" data-action="export-format" data-format="csv">CSV</button></div></div></div><div class="field"><div class="picker-heading"><span class="field-label">主播 <span class="muted" id="export-selected-count"></span></span><div class="picker-actions"><button class="small" data-action="select-export-all">全选筛选结果</button><button class="small quiet" data-action="clear-export-selection">清空</button></div></div><div class="anchor-picker-filters"><label class="search">${icon("search")}<input id="export-search" placeholder="搜索主播名称或 ID" value="${esc(state.exportSearch)}" aria-label="搜索已归档主播"></label><select id="export-tag-filter" aria-label="按标签选择导出主播"></select></div><div id="anchor-picker" class="anchor-picker"></div></div><div class="field"><label>消息类型</label><div class="type-checks">${Object.entries(
        TYPES,
      )
        .map(
          ([k, v]) =>
            `<label><input type="checkbox" data-export-type="${k}" ${state.exportTypes.has(k) ? "checked" : ""}>${v}</label>`,
        )
        .join(
          "",
        )}</div></div><div class="export-submit"><p class="help export-layout" id="export-layout-summary"></p><button class="primary" data-action="export" ${state.exportBusy ? "disabled" : ""}>${icon("download")}导出文件</button></div></div></section><aside class="archive-aside"><section class="panel"><div class="panel-header"><h2 class="panel-title">导出记录</h2></div><div id="export-jobs"></div></section><details class="panel export-schema"><summary>导出字段 · 7 列</summary><div class="form-panel">${[
        ["主播", "主播名称"],
        ["直播间场次", "真实场次 ID"],
        ["发消息人 ID", "按文本保留，避免数字精度丢失"],
        ["发消息人名字", "消息收到时的昵称，缺失时留空"],
        ["消息类型", "聊天、点赞、红包 / 福袋等"],
        ["消息内容", "原始内容文本"],
        ["发消息时间", "北京时间，缺失时留空"],
      ]
        .map(
          ([a, b]) =>
            `<div class="export-preview"><strong>${a}</strong><span>${b}</span></div>`,
        )
        .join(
          "",
        )}<p class="help">时间使用北京时间。发送时间缺失时留空，按接收月份归档。</p></div></details></aside></div>`;
    renderArchivePicker();
    renderJobs();
  } else if (state.page === "clips") {
    p.innerHTML = `<section id="clip-jobs"></section>`;
    void clipPanel.mount();
  } else if (state.page === "settings") {
    p.innerHTML =
      heading("", "设置", "") + `<div class="settings-layout"><section class="panel highlight-settings-panel"><div id="highlight-settings"></div></section><section class="panel settings-section appearance-panel"><div class="setting-row"><div><h2>界面字体</h2><p>主播名称与消息内容 · 0123456789</p></div><select id="ui-font" aria-label="界面字体"><option value="douyin" ${state.font === "douyin" ? "selected" : ""}>抖音美好体 · 默认</option><option value="chill" ${state.font === "chill" ? "selected" : ""}>寒蝉半圆体</option></select></div></section><section class="panel settings-section"><div class="setting-row"><div><h2>${SERVER_MODE ? "服务器数据" : "本地数据"}</h2><p>聊天与活动按主播和场次归档。</p></div>${SERVER_MODE ? "" : `<button data-action="open-data">${icon("folder")}打开数据目录</button>`}</div><div class="settings-runtime"><h3>运行状态</h3><div id="runtime-metrics"></div></div></section>${settingsToolsMarkup()}${SERVER_MODE ? `<section class="panel settings-section"><div class="setting-row"><div><h2>管理会话</h2><p>退出管理端后，服务器继续监控。</p></div><button data-action="server-logout">退出管理端</button></div></section>` : updatePanelMarkup(icon)}</div>`;
    renderRuntime();
    const diagnostics=document.querySelector("#diagnostics-settings .settings-disclosure-body");
    if(diagnostics){const checks=document.createElement("section");checks.id="compatibility-checks";diagnostics.prepend(checks);compatibilityChecks.mount(checks);}
    void highlightSettings.mount();
    document.querySelector(".settings-layout").insertAdjacentHTML("afterbegin",'<section class="panel community-settings-panel"><div id="community-settings"></div></section>');
    void communitySettings.mount();
    document.querySelector('.settings-layout').insertAdjacentHTML('afterbegin','<section class="panel time-display-settings-panel"><div id="time-display-settings"></div></section>');void timeDisplaySettings.mount();
    if(!SERVER_MODE){document.querySelector('.settings-layout').insertAdjacentHTML('afterbegin','<section class="panel download-settings-panel"><div id="download-settings"></div></section>');void downloadSettings.mount();}
    if (!SERVER_MODE) { document.querySelector(".settings-layout").insertAdjacentHTML("afterbegin", startupSettings.markup()); void startupSettings.mount(); }
    if(!SERVER_MODE){document.querySelector('.settings-layout').insertAdjacentHTML('afterbegin','<section class="panel data-backup-panel"><div id="data-backup"></div></section>');dataBackup.mount();}
  }
}
function renderProxy() {
  if (!proxyVisible()) return;
  const el = document.querySelector("#proxy-summary");
  if (!el) return;
  const p = state.proxy;
  const source = p.source || state.settings.proxy_source || "public";
  const sourceSelect = document.querySelector("#proxy-source");
  if (sourceSelect) {
    sourceSelect.disabled = state.proxyConfigBusy || state.proxyRefreshRequest || Boolean(p.refresh?.active);
    if (!sourceSelect.disabled) sourceSelect.value = source;
  }
  document.querySelector("#proxy-source-label").textContent = source === "imported" ? "已导入 / URL 订阅" : "proxy.scdn.io";
  document.querySelector("#proxy-help").textContent = source === "imported"
    ? "每批最多检测 50 个已导入地址，跳过本次运行中已失败的代理。首页通过仅代表初检；启用或切换来源前请停止全部监控。"
    : "每批检测 50 个新地址，跳过本次运行中已失败的代理。首页通过仅代表初检；启用或切换来源前请停止全部监控。";
  const batch = p.refresh;
  const refreshButton = document.querySelector('[data-action="refresh-proxy"]');
  if (refreshButton) {
    refreshButton.disabled = state.proxyConfigBusy || state.proxyRefreshRequest || Boolean(batch?.active);
    replaceContent(refreshButton, batch?.active
      ? `${icon("refresh")}检测中 ${num(batch.checked)} / ${num(batch.target || 50)}`
      : `${icon("refresh")}${source === "imported" ? "检测导入代理" : "获取并检测 50 个"}`);
  }
  const progress = document.querySelector("#proxy-batch");
  if (progress) {
    progress.hidden = !batch || batch.state === "idle";
    progress.textContent = batch
      ? `${batch.active ? "本批" : "上批"}已检测 ${num(batch.checked)} / ${num(batch.target || 50)} · 首页通过 ${num(batch.passed)} · 失败 ${num(batch.failed)}${batch.detail ? ` · ${batch.detail}` : ""}`
      : "";
  }
  const summary = [
    ["首页通过", p.available],
    ["已分配任务", p.assigned],
    ["检测中", p.checking],
    ["失败", p.failed],
  ]
    .map(
      ([label, value]) =>
        `<div class="proxy-number"><span>${label}</span><strong>${value === undefined ? "—" : num(value)}</strong></div>`,
    )
    .join("");
  replaceContent(el, summary);
  const enabled = document.querySelector("#proxy-enabled");
  if (enabled) enabled.disabled = state.proxyConfigBusy;
  if (enabled && !enabled.disabled)
    enabled.checked = Boolean(p.enabled ?? state.settings.proxy_enabled);
  const direct = document.querySelector("#proxy-direct");
  if (direct) {
    direct.disabled = state.proxyConfigBusy || !Boolean(p.enabled ?? state.settings.proxy_enabled);
    direct.checked = Boolean(p.include_direct ?? state.settings.proxy_include_direct ?? true);
  }
  document.querySelector("#proxy-error").innerHTML =
    p.last_error || p.fetch_error
      ? `<p class="proxy-error">${esc(p.last_error || p.fetch_error)}</p>`
      : "";
  const routes = p.routes || [];
  const routeStates = { ready: "可分配", active: "使用中", cooling: "冷却中", cooldown: "冷却中", checking: "检测中", draining: "待连接结束", suspended: "暂停分配", failed: "待处理", unavailable: "不可分配" };
  replaceContent(document.querySelector("#proxy-routes"), routes.length
    ? `<div class="table-wrap proxy-route-table"><table><thead><tr><th>出口</th><th>任务数</th><th>正在建连</th><th>状态</th></tr></thead><tbody>${routes.map(r => `<tr><td class="mono">${esc(r.kind === "direct" ? "本机直连" : r.endpoint)}</td><td>${num(r.assigned_tasks)}</td><td>${num(r.connecting)}</td><td>${esc(routeStates[r.state] || r.state || "可分配")}${r.cooldown_seconds > 0 ? ` · ${num(Math.ceil(r.cooldown_seconds))} 秒` : ""}</td></tr>`).join("")}</tbody></table></div>`
    : empty("暂无出口负载", "开始监控后可查看各出口的任务和连接状态。", "network"));
  const subscriptions = p.subscriptions || [];
  document.querySelector("#proxy-subscriptions-panel").hidden = subscriptions.length === 0;
  replaceContent(document.querySelector("#proxy-subscriptions"), subscriptions.map(s => `<div class="proxy-subscription-row"><div><strong>${esc(s.label)}</strong><small>${num(s.count)} 个候选 · 每 ${num(s.interval_minutes)} 分钟刷新${s.updated_at ? ` · 上次 ${esc(time(s.updated_at))}` : ""}</small>${s.error ? `<small class="danger">${esc(s.error)}</small>` : ""}</div><span class="badge">${esc({ queued: "等待刷新", refreshing: "刷新中", ready: "已同步", error: "刷新失败", idle: "等待刷新" }[s.state] || "等待刷新")}</span><button class="icon-btn quiet" data-action="refresh-subscription" data-id="${esc(s.id)}" title="刷新订阅" aria-label="刷新 ${esc(s.label)}" ${["queued", "refreshing"].includes(s.state) ? "disabled" : ""}>${icon("refresh")}</button><button class="icon-btn quiet danger" data-action="remove-subscription" data-id="${esc(s.id)}" title="移除订阅" aria-label="移除 ${esc(s.label)}">${icon("trash")}</button></div>`).join(""));
  document.querySelector("#proxy-assignment-count").textContent = num(Array.isArray(p.leases)
    ? p.leases.length : state.anchors.reduce((count, anchor) => count + Boolean(anchor.proxy), 0));
  if (!document.querySelector(".proxy-assignments")?.open) return;
  const leases = Array.isArray(p.leases)
    ? p.leases
    : state.anchors
        .filter((a) => a.proxy)
        .map((a) => ({ rid: a.rid, proxy: a.proxy }));
  const assignment = proxyAssignments(leases, state.anchors, state.proxySearch, state.proxyPage);
  state.proxyPage = assignment.page;
  replaceContent(document.querySelector("#proxy-assignment-pages"), `<span>${assignment.first}–${assignment.last} / ${num(assignment.total)}</span><button class="small" data-action="proxy-prev" ${assignment.page === 0 ? "disabled" : ""}>上一页</button><button class="small" data-action="proxy-next" ${assignment.page + 1 >= assignment.pages ? "disabled" : ""}>下一页</button>`);
  replaceContent(document.querySelector("#proxy-leases"), assignment.items.length
      ? `<div class="table-wrap"><table class="proxy-table"><thead><tr><th>主播</th><th>出口</th><th>延迟</th><th>状态</th></tr></thead><tbody>${assignment.items
        .map((l) => {
          return `<tr><td><span class="anchor-name">${esc(l.name)}</span><span class="anchor-id">${esc(l.rid)}</span></td><td class="mono">${esc(l.kind === "direct" || l.proxy === "direct" ? "本机直连" : l.proxy || l.endpoint || "—")}</td><td>${l.latency_ms !== undefined ? `${Math.round(l.latency_ms)} ms` : "—"}</td><td><span class="status online">已分配</span></td></tr>`;
        })
        .join("")}</tbody></table></div>`
    : empty(
        state.proxySearch ? "没有匹配的任务" : "暂无任务分配",
        state.proxySearch ? "试试主播名称、直播间 ID 或出口地址。" : "开始监控后可查看任务使用的出口。",
        "network",
      ));
}
async function loadProxy() {
  if (proxyLoadPromise) return proxyLoadPromise;
  proxyPolledAt = Date.now();
  proxyLoadPromise = Promise.resolve().then(async () => {
    try {
      state.proxy = await bridge("proxy_status");
      if (typeof state.proxy.enabled === "boolean")
        state.settings.proxy_enabled = state.proxy.enabled;
      if (typeof state.proxy.include_direct === "boolean")
        state.settings.proxy_include_direct = state.proxy.include_direct;
      renderProxy();
    } catch (e) {
      toast(String(e?.message || e), true);
    } finally {
      proxyPolledAt = Date.now();
      proxyLoadPromise = null;
    }
  });
  return proxyLoadPromise;
}
async function configureProxy(values) {
  if (state.proxyConfigBusy) return;
  state.proxyConfigBusy = true;
  renderProxy();
  try {
    const result = await request("configure_proxy", {
      enabled: Boolean(state.proxy.enabled ?? state.settings.proxy_enabled), ...values,
    });
    state.proxy.enabled = result.enabled;
    state.proxy.source = result.source;
    state.settings.proxy_enabled = result.enabled;
    state.settings.proxy_source = result.source;
    if (values.include_direct !== undefined) state.settings.proxy_include_direct = values.include_direct;
    toast(result.enabled ? "代理设置已更新" : "代理设置已更新 · 当前使用直连");
    await loadProxy();
  } catch (error) {
    toast(String(error?.message || error), true);
  } finally {
    state.proxyConfigBusy = false;
    renderProxy();
  }
}

function renderViewerRefresh(){ const button=document.querySelector('[data-action="refresh-viewers"]'); if(!button)return; const value=viewerRefresh.snapshot();button.disabled=value.disabled;replaceContent(button,`${icon("refresh")}${value.label}`); }
function renderStats() {
  renderViewerRefresh();
  const el = document.querySelector("#stats");
  if (!el) return;
  const a = state.anchors,
    online = a.filter(live).length,
    off = a.filter(offline).length,
    err = a.filter(faulty).length,
    unknown = a.length - online - off - err;
  replaceContent(el,
    [
      ["开播中", online, "live"],
      ["未开播", off, "offline"],
      ["待确认", Math.max(0, unknown), "unknown"],
      ["异常", err, "error"],
    ]
      .map(
        ([label, value, cls]) =>
          `<div class="summary-item ${cls}"><span class="summary-label"><i class="dot"></i>${label}</span><strong>${num(value)}</strong></div>`,
      )
      .join("") +
    `<div class="summary-item total"><span class="summary-label">已收消息</span><strong>${num(state.metrics.total_messages ?? a.reduce((n, x) => n + (x.message_count || 0), 0))}</strong></div>`);
}
function filteredAnchors() {
  return selectAnchors(state.anchors, state.search, state.filter, state.tagFilter);
}
function renderTagFilter(id, anchors, selected) {
  const select = document.getElementById(id);
  if (!select) return;
  const tags = tagOptions(anchors);
  if (selected && selected !== UNTAGGED_TAG && !tags.includes(selected)) tags.push(selected);
  replaceContent(select, `<option value="">全部标签</option><option value="${UNTAGGED_TAG}">无标签</option>${tags.map(tag=>`<option value="${esc(tag)}">${esc(tag === "无标签" || tag === "全部标签" ? `标签：${tag}` : tag)}</option>`).join("")}`);
  select.value = selected;
}
function renderTagFilters() {
  renderTagFilter("task-tag-filter", state.anchors, state.tagFilter);
  renderTagFilter("export-tag-filter", state.archiveAnchors, state.exportTag);
}
function tagEditorMarkup(id, tags = [], labelled = true) {
  const suggestions = tagOptions([...state.anchors, ...state.archiveAnchors]).slice(0, 12);
  return `<div class="field tag-editor">${labelled ? `<label for="${id}">标签（可选）</label>` : ""}<input id="${id}" aria-label="主播标签" value="${esc(tags.join("，"))}" maxlength="1000" placeholder="如：万龙觉醒，重点关注" autocomplete="off" spellcheck="false"><small>多个标签用逗号分隔，每个最多 24 字。</small>${suggestions.length ? `<div class="tag-suggestions" role="group" aria-label="已有标签">${suggestions.map(tag=>`<button class="small quiet" data-action="tag-choice" data-target="${id}" data-tag="${esc(tag)}" aria-pressed="${tags.includes(tag)}">${esc(tag)}</button>`).join("")}</div>` : ""}</div>`;
}
function syncTagChoices(input) {
  if (!input?.closest(".tag-editor")) return;
  let tags;
  try { tags = normalizeTags(input.value); } catch { return; }
  input.closest(".tag-editor").querySelectorAll("[data-tag]").forEach(button=>button.setAttribute("aria-pressed", String(tags.includes(button.dataset.tag))));
}
function showAnchorTags(rids) {
  const chosen = state.anchors.filter(a=>rids.includes(a.rid));
  if (!chosen.length) return toast("请先选择主播", true);
  state.modal = {kind:"anchor-tags", rids:chosen.map(a=>a.rid)};
  modal(chosen.length === 1 ? `${chosen[0].name || chosen[0].rid} · 标签` : `设置 ${chosen.length} 位主播的标签`,
    `${chosen.length > 1 ? '<p class="help">保存后替换所选主播的标签，留空则清除。</p>' : ""}${tagEditorMarkup("edit-anchor-tags", chosen.length === 1 ? anchorTags(chosen[0]) : [])}`,
    '<button data-action="close-modal">取消</button><button class="primary" data-action="save-anchor-tags">保存标签</button>');
  document.getElementById("edit-anchor-tags")?.focus({preventScroll:true});
}
function taskActions(a, compact = false) {
  return `<div class="row-actions task-actions">${live(a)&&a.anchor_id&&a.session_id?`<button class="quick-mark" data-action="quick-mark" data-rid="${esc(a.rid)}" title="打点当前直播时刻" aria-label="为${esc(a.name)}打点">${icon("bookmark")}${compact?"":"<span>打点</span>"}</button>`:""}<button class="monitor-toggle ${active(a) ? "is-running" : ""}" data-action="${active(a) ? "stop-one" : "start-one"}" data-rid="${esc(a.rid)}" title="${active(a) ? "停止" : "开始"}监控" aria-label="${active(a) ? "停止" : "开始"}监控${esc(a.name)}">${icon(active(a) ? "stop" : "play")}${compact ? '' : `<span>${active(a) ? '停止' : '开始'}</span>`}</button><button class="open-live" data-action="open-live" data-rid="${esc(a.rid)}" title="使用默认浏览器打开直播间" aria-label="打开${esc(a.name)}的直播间">${icon("external")}<span>直播间</span></button></div>`;
}
function renderCard(a) {
  const [statusClass, statusText] = status(a);
  const activity = (kind, label) => {
    const view = activityView(a, kind);
    return `<div class="activity-metric ${kind} ${['0','—'].includes(view.count) ? 'empty' : ''}"><span class="metric-label">${label}</span><strong>${esc(view.count)}</strong></div>`;
  };
  const indicator = kind => {
    const view = activityView(a, kind);
    if (view.state !== 'active') return '';
    return `<span class="activity-indicator ${view.state}" title="${esc(view.label)}" data-action="detail" data-rid="${esc(a.rid)}"><img src="${kind === 'red_packet' ? redPacketUrl : lotteryBagUrl}" alt="${esc(view.label)}" width="22" height="22"><i></i></span>`;
  };
  const indicators = indicator('red_packet') + indicator('lottery');
  const clock = live(a) ? `<span class="flip-clock" data-duration="${esc(a.rid)}" role="timer" aria-live="off" title="当前场次累计监控时长，停止期间不计入">—</span>` : '';
  return `<article class="monitor-card ${pcUnsupported(a) ? "unsupported" : live(a) ? "live" : offline(a) ? "offline" : faulty(a) ? "faulty" : "pending"} ${state.selected.has(a.rid) ? "checked" : ""}" data-rid="${esc(a.rid)}"><button class="card-opener" data-action="detail" data-rid="${esc(a.rid)}" aria-label="查看${esc(a.name || a.rid)}"></button><div class="card-heading"><span class="card-identity">${indicators ? `<span class="activity-indicators">${indicators}</span>` : ''}<span class="anchor-name" title="${esc(a.name || a.rid)}">${esc(a.name || a.rid)}</span></span><span class="card-live-state"><span class="status ${statusClass}"><i class="dot"></i><span class="status-label">${statusText}</span></span>${clock}</span>${state.selecting ? `<input type="checkbox" data-select="${esc(a.rid)}" aria-label="选择${esc(a.name)}" ${state.selected.has(a.rid) ? "checked" : ""}>` : ''}</div><div class="card-tagline">${tagsMarkup(a,{limit:2})}<button class="tag-edit quiet" data-action="edit-tags" data-rid="${esc(a.rid)}" title="编辑标签" aria-label="编辑${esc(a.name || a.rid)}的标签">${icon("edit")}${anchorTags(a).length ? "" : "<span>添加标签</span>"}</button></div><div class="room-metrics"><div><span class="metric-label">当前在线</span><strong data-viewers="${esc(a.rid)}">—</strong></div>${activity('red_packet','本场红包')}${activity('lottery','本场福袋')}</div><div class="card-foot">${taskActions(a, true)}</div></article>`;
}
function replaceContent(el, html) {
  if (!el) return;
  if (el._renderedHTML === html) return;
  const focused = el.contains(document.activeElement) ? document.activeElement : null;
  const key = focused?.dataset.select ? `[data-select="${CSS.escape(focused.dataset.select)}"]`
    : focused?.dataset.action ? `[data-action="${CSS.escape(focused.dataset.action)}"]${focused.dataset.rid === undefined ? "" : `[data-rid="${CSS.escape(focused.dataset.rid)}"]`}` : null;
  el.innerHTML = html;
  el._renderedHTML = html;
  if (key) el.querySelector(key)?.focus({ preventScroll: true });
}
let taskFrame = 0;
function scheduleTasks() {
  if (taskFrame) return;
  taskFrame = requestAnimationFrame(() => { taskFrame = 0; renderTasks(); });
}
window.addEventListener("resize", scheduleTasks);
function renderTasks() {
  const root = document.querySelector("#task-content");
  if (!root) return;
  renderTagFilters();
  const filtered = filteredAnchors();
  root.classList.toggle("selecting", state.selecting);
  document.querySelector('[data-action="toggle-selection"]')?.classList.toggle("active", state.selecting);
  replaceContent(document.querySelector("#bulk-bar"), state.selecting
    ? `<div class="bulk-bar"><span class="bulk-count">已选择 ${state.selected.size} 位</span><button class="small" data-action="select-visible">选择可见</button><button class="small" data-action="start-selected" ${state.selected.size ? "" : "disabled"}>${icon("play")}开始</button><button class="small" data-action="stop-selected" ${state.selected.size ? "" : "disabled"}>${icon("stop")}停止</button><button class="small" data-action="tag-selected" ${state.selected.size ? "" : "disabled"}>${icon("edit")}设置标签</button><button class="small danger" data-action="remove-selected" ${state.selected.size ? "" : "disabled"}>${icon("trash")}移除</button><button class="small quiet" data-action="clear-selection">退出选择</button></div>` : "");
  if (!filtered.length) {
    replaceContent(root, empty(state.anchors.length ? "没有匹配的主播" : "开始建立你的监控看板", state.anchors.length ? "试试其他名字、ID、标签或状态筛选。" : "添加直播间链接或 ID，即可按主播持续归档消息。", "grid"));
  } else {
    const grouped = state.filter === "all" && !state.search.trim();
    const resting = grouped ? filtered.filter(offline) : [];
    const primary = grouped ? filtered.filter(a => !offline(a)) : filtered;
    const style = getComputedStyle(root);
    const previousScroll = root.scrollTop;
    const geometry = { width: root.clientWidth - 4, height: root.clientHeight, scrollTop: previousScroll,
      minWidth: parseFloat(style.getPropertyValue("--card-min-width")),
      cardHeight: parseFloat(style.getPropertyValue("--card-height")), gap: parseFloat(style.getPropertyValue("--card-gap")) };
    const section = (anchors, offset = 0) => {
      if (state.view !== "grid") return { html: anchors.length ? renderTaskTable(anchors) : "", height: 0 };
      const range = gridSectionWindow({ ...geometry, count: anchors.length, offset });
      return { height: range.totalHeight, html: range.visible
        ? `<div class="virtual-space" style="padding-top:${range.top}px;padding-bottom:${range.bottom}px"><div class="monitor-grid" style="grid-template-columns:repeat(${range.columns},minmax(0,1fr))">${anchors.slice(range.start, range.end).map(renderCard).join("")}</div></div>`
        : `<div class="virtual-space" style="height:${range.totalHeight}px"></div>` };
    };
    const top = section(primary);
    const groupGap = primary.length ? parseFloat(style.getPropertyValue("--group-gap")) : 0;
    const offlineOffset = top.height + groupGap + parseFloat(style.getPropertyValue("--group-header-height")) + parseFloat(style.getPropertyValue("--group-content-gap"));
    const bottom = resting.length ? `<section class="offline-group" style="margin-top:${groupGap}px"><button class="offline-toggle" data-action="toggle-offline" aria-expanded="${state.offlineExpanded}" aria-controls="offline-tasks"><span>未开播</span><span class="offline-count">${num(resting.length)}</span>${icon("down")}</button><div id="offline-tasks" class="offline-content" ${state.offlineExpanded ? "" : "hidden"}>${state.offlineExpanded ? section(resting, offlineOffset).html : ""}</div></section>` : "";
    replaceContent(root, top.html + bottom);
    if (root.scrollTop !== previousScroll) scheduleTasks();
  }
  const byRid = new Map(filtered.map(a => [a.rid, a]));
  root.querySelectorAll('[data-duration]').forEach(el => {
    const value = duration(byRid.get(el.dataset.duration));
    if (el.classList.contains('flip-clock')) paintClock(el, value);
    else el.textContent = value;
  });
  root.querySelectorAll('[data-viewers]').forEach(el => {
    const anchor = byRid.get(el.dataset.viewers);
    el.textContent = el.closest('.room-metrics') ? compactViewerCount(anchor) : viewerCount(anchor);
    el.setAttribute('aria-label', `当前在线 ${viewerCount(anchor)}`);
  });
}

function renderTaskTable(anchors) {
  return `<div class="table-wrap"><table><thead><tr><th class="check-cell"></th><th>主播</th><th>直播状态</th><th>当前在线</th><th>开播时长</th><th>操作</th></tr></thead><tbody>${anchors.map(a => `<tr data-rid="${esc(a.rid)}" class="${state.selected.has(a.rid) ? "checked" : ""}"><td class="check-cell"><input type="checkbox" data-select="${esc(a.rid)}" aria-label="选择${esc(a.name)}" ${state.selected.has(a.rid) ? "checked" : ""}></td><td><button class="row-detail anchor-info" data-action="detail" data-rid="${esc(a.rid)}"><span class="anchor-name">${esc(a.name || a.rid)}</span></button><div class="table-anchor-tags">${tagsMarkup(a,{limit:3})}<button class="tag-edit quiet" data-action="edit-tags" data-rid="${esc(a.rid)}" aria-label="编辑${esc(a.name || a.rid)}的标签" title="编辑标签">${icon("edit")}${anchorTags(a).length ? "" : "<span>添加标签</span>"}</button></div></td><td>${statusBadge(a)}</td><td class="number" data-viewers="${esc(a.rid)}">—</td><td class="message-time" data-duration="${esc(a.rid)}">—</td><td>${taskActions(a)}</td></tr>`).join("")}</tbody></table></div>`;
}

function renderMessages() {
  const root = document.querySelector("#message-content");
  if (!root) return;
  void messageFeed.refresh();
  const feed=messageFeed.snapshot();
  const rows = feed.rows
    .filter(
      (m) =>
        LIVE_TYPES[m.msg_type || m.message_type] && (state.messageType === "all" ||
          (m.msg_type || m.message_type) === state.messageType) &&
        (!state.messageAnchor || m.rid === state.messageAnchor),
    )
    .slice(0,150);
  const html = rows.length
    ? `<div class="table-wrap"><table><thead><tr><th>发送时间</th><th>主播 / 场次</th><th>发消息人</th><th>类型</th><th>消息内容</th></tr></thead><tbody>${rows.map((m) => {
      const kind = m.msg_type || m.message_type;
      const activity = kind === "activity";
      const sender = m.sender_name || m.payload?.sender_name || (activity ? "直播间活动" : "未提供昵称");
      const senderId = m.sender_id || m.payload?.sender_id || (activity ? "" : "ID 未提供");
      const label = activity ? ({ red_packet: "红包", lottery: "福袋" }[m.payload?.activity_kind] || TYPES.activity) : TYPES[kind] || kind;
      return `<tr><td class="message-time">${esc(m.time_str || (m.message_time ? time(m.message_time) : "时间未知"))}</td><td><span class="anchor-name">${esc(m.name || m.anchor_name || m.rid)}</span><span class="anchor-id">${esc(m.session_id || m.payload?.room_id || "")}</span></td><td class="sender">${esc(sender)}${senderId ? `<span class="mono">${esc(senderId)}</span>` : ""}</td><td><span class="message-type ${esc(kind)}">${esc(label)}</span></td><td class="message-content">${esc(m.content ?? m.payload?.content ?? m.text ?? "")}${m.content_truncated?'<span class="help">（完整内容见导出）</span>':""}</td></tr>`;
    }).join("")}</tbody></table></div>`
    : empty(feed.loading?"正在读取消息":feed.error?"读取消息失败":"暂无匹配消息", feed.error||"已归档的聊天与互动会按当前分类显示。", "chat");
  replaceContent(root, html);
  document.querySelector("#message-count").textContent =
    `显示最近 ${rows.length} 条匹配消息 · ${feed.error ? "刷新失败，将自动重试" : state.paused ? "显示已暂停，采集继续" : "自动刷新"} · 完整数据请使用导出数据`;
}
function renderLogs() {
  if (!state.settings.debug_logging || !document.querySelector("#diagnostics-settings")?.open) return;
  const root = document.querySelector("#log-content");
  if (!root) return;
  const html = state.logs.length
      ? state.logs
          .slice(-200)
          .reverse()
          .map(
            (l) =>
              `<div class="log-row"><time>${esc(l.time_str || time(l.timestamp))}</time><span>${esc(l.name || l.rid || "采集服务")}</span><span class="log-level ${l.state === "error" ? "error" : ""}">${esc({ connecting: "连接", offline: "未开播", monitoring: "监控", stopped: "停止", error: "异常", retrying: "重试" }[l.state] || l.msg_type || "状态")}</span><span class="log-detail">${esc(l.detail || l.text || "")}</span></div>`,
          )
          .join("")
      : empty(
          "暂无运行记录",
          "开始监控后，连接状态与错误会显示在这里。",
          "logs",
        );
  replaceContent(root, html);
}
function renderArchivePicker() {
  const el = document.querySelector("#anchor-picker");
  if (!el) return;
  renderTagFilters();
  const allowed = new Set(state.archiveAnchors.filter(a=>tagMatches(a,state.exportTag)).map(a=>a.anchor_id));
  state.exportSelection = new Set([...state.exportSelection].filter(id=>allowed.has(id)));
  const rows = filterArchiveAnchors(state.archiveAnchors, state.exportSearch, state.exportTag);
  el.innerHTML = rows.length
    ? rows
        .map(
          (a) =>
            `<label class="anchor-pick"><input type="checkbox" data-export-anchor="${esc(a.anchor_id)}" ${state.exportSelection.has(a.anchor_id) ? "checked" : ""}><span>${esc(a.anchor_name || a.anchor_id)}<small>${esc(a.anchor_id)}</small>${tagsMarkup(a)}</span></label>`,
        )
        .join("")
    : empty(
        "暂无匹配的归档主播",
        state.exportTag || state.exportSearch ? "换个标签或搜索条件。" : "收到首条消息后，主播会出现在这里。",
        "archive",
      );
  document.querySelector("#export-selected-count").textContent =
    `已选 ${state.exportSelection.size} 位`;
  renderExportLayout();
}
function renderExportLayout() {
  const el = document.querySelector("#export-layout-summary");
  if (!el) return;
  const layout = state.exportFormat === "xlsx" ? "按主播分工作表，表名使用主播名字" : "CSV 合并为一张表";
  let filename = "";
  if (state.exportSelection.size) {
    try {
      filename = exportFileName({ anchorIds: state.exportSelection, anchors: state.archiveAnchors, month: state.exportMonth, format: state.exportFormat });
    } catch { /* The month input may temporarily be empty while editing. */ }
  }
  el.textContent = filename ? `${layout} · ${filename}` : layout;
}
function renderJobs() {
  const el = document.querySelector("#export-jobs");
  if (!el) return;
  const html = state.jobs.length
    ? state.jobs
        .slice()
        .reverse()
        .map((j) => {
          const r = j.report || {},
            running = ["running", "pending", "queued"].includes(j.status);
          const download=SERVER_MODE && /^\/api\/v1\/files\/[a-f0-9]{32}$/.test(j.download_url||"") ? `<a class="server-download" href="${esc(j.download_url)}" download>${esc(j.file_name||"下载文件")}</a>` : "";
          return `<div class="job-item"><strong>${esc(r.month || j.month || "月度导出")} · ${esc({ running: "正在导出", completed: "导出完成", done: "导出完成", cancelled: "已取消", error: "导出失败", failed: "导出失败" }[j.status] || j.status)}</strong>${r.path ? `<p>${esc(r.path)}</p>` : ""}${download}${j.error ? `<p class="danger">${esc(j.error)}</p>` : ""}${["completed", "done"].includes(j.status) ? `<div class="job-result">${num(r.row_count)} 条消息 · ${(r.format || "").toUpperCase()}</div>${r.missing_time_count || r.missing_sender_count ? `<p>${num(r.missing_time_count)} 条缺发送时间，${num(r.missing_sender_count)} 条缺发送人 ID，已保留空值。</p>` : ""}${r.formula_protected_count ? `<p>CSV 中 ${num(r.formula_protected_count)} 个字段已加前缀，防止被表格软件识别为公式。</p>` : ""}` : ""}${running ? `<button class="small quiet" data-action="cancel-export" data-job="${esc(j.id || j.job_id)}">取消导出</button>` : ""}</div>`;
        })
        .join("")
    : empty("尚无导出任务", "选择月份和主播即可生成文件。", "download");
  replaceContent(el, html);
}
function renderRuntime() {
  renderSettingsTools();
  renderUpdatePanel(state.anchors.some(active));
  const el = document.querySelector("#runtime-metrics");
  if (el)
    el.innerHTML = [
      [
        "采集进程内存",
        state.metrics.process_rss_mb
          ? `${Number(state.metrics.process_rss_mb).toFixed(1)} MB`
          : "—",
      ],
      ["采集线程", state.metrics.thread_count ?? "—"],
      ["待写入消息", state.metrics.archive_queue ?? "—"],
    ]
      .map(
        ([k, v]) =>
          `<div class="kv"><span>${k}</span><strong data-detail-field="${k}">${esc(v)}</strong></div>`,
      )
      .join("");
}
function updateConnection() {
  const e = document.querySelector("#connection");
  if (e) {
    e.className = `connection ${state.connected ? "" : "bad"}`;
    replaceContent(e, `<i class="dot"></i>${state.preview ? "预览" : state.connected ? "已连接" : "未连接"}`);
  }
  const err = document.querySelector("#error-area");
  if (err)
    err.innerHTML = state.error
      ? `<div class="error-banner"><span>${esc(state.error)}</span><button class="small" data-action="reconnect">重试</button></div>`
      : "";
}
function toast(message, error = false) {
  const e = document.createElement("div");
  e.className = `toast ${error ? "error" : ""}`;
  e.textContent = message;
  document.querySelector(".toast-stack").append(e);
  setTimeout(() => e.remove(), 6500);
}
let modalReturnFocus = null;
function modal(title, body, footer = "", drawer = false) {
  if (!document.querySelector('.modal')) modalReturnFocus = document.activeElement;
  document.querySelector("#modal-root").innerHTML =
    `<div class="overlay ${drawer ? "drawer-overlay" : ""}"><section role="dialog" aria-modal="true" aria-label="${esc(title)}" class="modal ${drawer ? "drawer" : ""}"><header class="modal-header"><h2>${esc(title)}</h2><button class="icon-btn quiet" data-action="close-modal" aria-label="关闭">${icon("close")}</button></header><div class="modal-body">${body}</div>${footer ? `<footer class="modal-footer">${footer}</footer>` : ""}</section></div>`;
  if (state.modal?.kind === "archive-delete") document.querySelector(".modal").classList.add("archive-delete-modal");
  document
    .querySelector(
      ".modal input:not([type=checkbox]),.modal textarea,.modal button",
    )
    ?.focus();
}
function closeModal() {
  activityTooltip.hide();
  episodeTooltip.hide();
  sessionTimeline.unmount();
  if(state.modal?.kind==="detail")highlightSettings.unmount();
  document.querySelector("#modal-root").innerHTML = "";
  state.modal = null;
  if (modalReturnFocus?.isConnected) modalReturnFocus.focus({preventScroll:true});
  modalReturnFocus = null;
  queueMicrotask(()=>updateNotice.flush());
}
const archiveDeletion = createArchiveDeletion({ state, bridge: (...args) => bridge(...args), modal, closeModal, toast,
  onDeleted: async () => {
    state.messages = [];
    messageFeed.invalidate();
    await loadArchive();
    const existing = new Set(state.archiveAnchors.map(a => a.anchor_id));
    state.exportSelection = new Set([...state.exportSelection].filter(id => existing.has(id)));
    await init();
  },
});
function showAdd() {
  state.modal = { kind: "add" };
  modal(
    "添加监控主播",
    `<div class="field"><label for="add-values">直播间链接或 ID</label><textarea id="add-values" placeholder="粘贴抖音直播间链接或数字 ID&#10;每行一个，可一次添加多个主播" spellcheck="false"></textarea><small>每行一个，重复直播间自动跳过。</small></div>${tagEditorMarkup("add-tags")}`,
    `<button data-action="close-modal">取消</button><button class="primary" data-action="submit-add">${icon("plus")}添加主播</button>`,
  );
  document.getElementById("add-values")?.focus({preventScroll:true});
}
function showProxyImport() {
  state.modal = { kind: "proxy-import", revision: 0, valid: false };
  modal("导入代理", `<div class="proxy-import-intro">粘贴代理列表，或选择 JSON / 文本文件。</div><div class="proxy-import-controls"><button data-action="choose-proxy-file">${icon("folder")}选择文件</button><input id="proxy-file" type="file" accept=".json,.txt,.csv,.tsv" hidden><label for="proxy-protocol">未注明协议时使用</label><select id="proxy-protocol"><option value="socks5">SOCKS5</option><option value="http">HTTP CONNECT</option></select></div><div class="field"><label for="proxy-text">代理信息</label><textarea id="proxy-text" rows="7" autocomplete="off" spellcheck="false" placeholder="支持 JSON 数组、IP:端口:账号:密码、代理 URL，或复制的代理表格"></textarea><small>账号密码加密保存在${SERVER_MODE ? "服务器" : "本机"}。相同地址的信息有变化时会更新。</small></div><div id="proxy-preview" aria-live="polite"></div>`, `<button data-action="preview-proxy">${icon("search")}识别预览</button><button class="primary" data-action="commit-proxy" disabled>${icon("plus")}导入或更新</button>`);
  document.querySelector(".modal").classList.add("proxy-modal");
  document.querySelector("#proxy-text").focus();
}
function showProxySubscription() {
  state.modal = { kind: "proxy-subscription" };
  modal("添加 URL 订阅", `<div class="field"><label for="proxy-subscription-url">HTTPS 订阅地址</label><input id="proxy-subscription-url" type="url" autocomplete="off" spellcheck="false" placeholder="https://proxylist.geonode.com/api/proxy-list?page=1&limit=500"><small>支持 Geonode、JSON 代理列表和逐行代理地址。订阅归入「已导入 / URL 订阅」来源。</small></div><div class="field"><label for="proxy-subscription-interval">自动刷新</label><select id="proxy-subscription-interval"><option value="15">每 15 分钟</option><option value="60" selected>每小时</option><option value="360">每 6 小时</option><option value="1440">每天</option></select></div>`, `<button data-action="close-modal">取消</button><button class="primary" data-action="save-subscription">${icon("plus")}添加订阅</button>`);
}
async function saveProxySubscription() {
  const context = state.modal;
  if (context?.kind !== "proxy-subscription") return;
  await bridge("add_proxy_subscription", { url: document.querySelector("#proxy-subscription-url").value.trim(), interval_minutes: Number(document.querySelector("#proxy-subscription-interval").value) });
  if (state.modal === context) closeModal();
  toast("订阅已添加，正在获取候选");
  await loadProxy();
}
function resetProxyPreview() {
  if (state.modal?.kind !== "proxy-import") return;
  state.modal.revision++;
  state.modal.valid = false;
  document.querySelector("#proxy-preview").innerHTML = "";
  document.querySelector('[data-action="commit-proxy"]').disabled = true;
}
const PROXY_STATES = { unchecked: "未检测", queued: "排队中", checking: "检测中", available: "首页通过", assigned: "已分配", failed: "失败", expired: "已过期" };
function proxyRows(items, removable = false) {
  return `<div class="proxy-import-table table-wrap"><table><thead><tr><th>代理地址</th><th>协议</th><th>认证</th><th>状态</th>${removable ? "<th></th>" : ""}</tr></thead><tbody>${items.map(p => `<tr><td class="mono">${esc(p.endpoint)}</td><td>${p.protocol === "socks5h" ? "SOCKS5" : "HTTP"}</td><td>${p.auth_present ? "已配置" : "无"}</td><td>${esc(PROXY_STATES[p.status] || "未检测")}</td>${removable ? `<td><button class="icon-btn quiet danger" data-action="remove-proxy" data-id="${esc(p.id)}" aria-label="移除 ${esc(p.endpoint)}" title="移除代理">${icon("trash")}</button></td>` : ""}</tr>`).join("")}</tbody></table></div>`;
}
async function previewProxyImport() {
  const context = state.modal;
  if (context?.kind !== "proxy-import") return;
  const text = document.querySelector("#proxy-text").value.trim();
  if (!text) throw new Error("请先粘贴代理信息或选择文件");
  if (text.length > 1024 * 1024) throw new Error("代理信息请控制在 1 MB 以内");
  const revision = context.revision;
  const result = await bridge("preview_proxies", { text, default_protocol: document.querySelector("#proxy-protocol").value });
  if (state.modal !== context || revision !== context.revision) return;
  context.valid = Number(result.valid) > 0;
  document.querySelector('[data-action="commit-proxy"]').disabled = !context.valid;
  document.querySelector("#proxy-preview").innerHTML = `<div class="proxy-import-result"><strong>新增 ${num(result.added)} · 更新 ${num(result.updated)}</strong><span>未变化 ${num(result.duplicates)} · 无效 ${num(result.invalid)}</span></div>${result.items?.length ? proxyRows(result.items) : ""}${result.errors?.length ? `<ul class="proxy-import-errors">${result.errors.slice(0, 8).map(e => `<li>第 ${num(e.line)} 项：${esc(e.message)}</li>`).join("")}</ul>` : ""}`;
}
async function commitProxyImport() {
  const context = state.modal;
  if (context?.kind !== "proxy-import" || !context.valid) return;
  const text = document.querySelector("#proxy-text").value.trim();
  const default_protocol = document.querySelector("#proxy-protocol").value;
  const result = await bridge("import_proxies", { text, default_protocol });
  if (state.modal === context) closeModal();
  toast(`已新增 ${num(result.added)} 个代理，更新 ${num(result.updated)} 个`);
  await loadProxy();
}
async function showImportedProxies() {
  const context = { kind: "proxy-list" };
  state.modal = context;
  modal("已导入代理", '<p class="help" role="status">正在读取…</p>', `<button data-action="import-proxy">${icon("plus")}导入代理</button>`);
  document.querySelector(".modal").classList.add("proxy-modal");
  try {
    const result = await bridge("list_imported_proxies");
    if (state.modal !== context) return;
    document.querySelector(".modal-body").innerHTML = result.items?.length
      ? `<div class="proxy-import-result"><strong>${num(result.total)} 个代理</strong><span>账号密码已隐藏</span></div>${proxyRows(result.items, true)}`
      : empty("尚未导入代理", "支持粘贴代理信息或选择文件。", "network");
  } catch (error) {
    if (state.modal === context) document.querySelector(".modal-body").innerHTML = '<p class="proxy-error">读取失败，请关闭后重试。</p>';
    throw error;
  }
}
async function readProxyFile(input) {
  const context = state.modal;
  const file = input.files?.[0];
  if (!file || context?.kind !== "proxy-import") return;
  try {
    if (file.size > 1024 * 1024) throw new Error("请选择 1 MB 以内的文件");
    const contents = await file.text();
    if (state.modal !== context) return;
    document.querySelector("#proxy-text").value = contents;
    resetProxyPreview();
    await previewProxyImport();
  } catch (error) { toast(String(error?.message || error), true); }
  finally { input.value = ""; }
}
function showDetail(rid, tab = "analysis") {
  const a = state.anchors.find((x) => x.rid === rid);
  if (!a) return;
  state.modal = { kind: "detail", rid, tab };
  modal(
    a.name,
    `<div class="detail-overview"><span id="detail-status">${statusBadge(a)}</span><div class="detail-live"><span>在线 <strong data-detail-field="当前在线">${viewerCount(a)}</strong></span><span>已监控 <strong data-detail-field="开播时长">${duration(a)}</strong></span></div></div><div class="detail-tabs" role="tablist" aria-label="主播详情">${[["analysis","直播回看"],["connection","连接信息"],["preferences","主播设置"]].map(([id,label])=>`<button role="tab" id="detail-tab-${id}" aria-controls="detail-panel-${id}" aria-selected="${tab===id}" tabindex="${tab===id?0:-1}" data-action="detail-tab" data-tab="${id}">${label}</button>`).join("")}</div><div class="detail-panel" id="detail-panel-analysis" role="tabpanel" aria-labelledby="detail-tab-analysis" ${tab!=="analysis"?"hidden":""}><section id="session-timeline" aria-label="直播回看与互动时间轴"></section><section id="session-quality" class="session-quality" aria-label="选择场次与发言统计"></section></div><div class="detail-panel" id="detail-panel-connection" role="tabpanel" aria-labelledby="detail-tab-connection" ${tab!=="connection"?"hidden":""}><div class="connection-grid">${[
      ["直播间入口 ID", a.rid],
      ["主播 ID", a.anchor_id || "等待识别"],
      ["当前场次", a.session_id || "等待识别"],
      ["本次已收消息", num(a.message_count)],
      ["最近消息", time(a.last_message_at)],
      ["直播状态查询时间", time(a.live_checked_at)],
      ["连接状态", a.detail || "尚未启动监控"],
    ]
      .map(
        ([k, v]) =>
          `<div class="kv"><span>${k}</span><strong data-detail-field="${k}">${esc(v)}</strong></div>`,
      )
      .join(
        "",
      )}<div class="kv"><span>代理地址</span><strong data-detail-field="代理地址">${esc(a.proxy || "本机直连")}</strong></div></div></div><div class="detail-panel" id="detail-panel-preferences" role="tabpanel" aria-labelledby="detail-tab-preferences" ${tab!=="preferences"?"hidden":""}><div class="detail-setting"><h3>显示名称</h3><div class="inline-form"><input id="rename-value" aria-label="显示名称" value="${esc(a.name)}"><button data-action="rename" data-rid="${esc(rid)}">保存名称</button></div></div><div class="detail-setting"><h3>标签</h3>${tagEditorMarkup("detail-tags",anchorTags(a),false)}<button class="small" data-action="save-detail-tags" data-rid="${esc(rid)}">保存标签</button></div><div class="detail-setting"><h3>电脑端兼容性</h3><p>${a.pc_support_source === "detected" ? "平台已明确返回电脑端不支持提示。" : "在浏览器确认后手动标记，不改变消息连接状态。"}</p><button data-action="set-pc-unsupported" data-rid="${esc(rid)}">${pcUnsupported(a) ? "清除非 PC 支持标记" : "标记为非 PC 支持"}</button></div><div class="detail-setting danger-zone"><div><h3>移除监控任务</h3><p>停止监控并从看板移除，历史归档仍然保留。</p></div><button class="danger" data-action="remove-one" data-rid="${esc(rid)}">${icon("trash")}移除任务</button></div></div>`,
    `<button data-action="open-live" data-rid="${esc(rid)}">${icon("external")}打开直播间</button><button id="detail-monitor-toggle" class="${active(a) ? "detail-stop" : "primary"}" data-action="${active(a) ? "stop-one" : "start-one"}" data-rid="${esc(rid)}">${icon(active(a) ? "stop" : "play")}${active(a) ? "停止监控" : "开始监控"}</button>`,
    false,
  );
  sessionTimeline.mount(a);
  sessionQuality.mount(a);
}

function renderDetailTitle() {
  if (state.modal?.kind !== "detail") return;
  const a = state.anchors.find(x => x.rid === state.modal.rid);
  const title = document.querySelector('.anchor-detail-dialog .modal-header h2');
  if (a && title) title.textContent = a.name;
}
function updateDetail() {
  if (state.modal?.kind !== "detail") return;
  const a = state.anchors.find(x => x.rid === state.modal.rid);
  if (!a) return closeModal();
  renderDetailTitle();
  if (state.modal.tab === "analysis") {sessionTimeline.tick(a);sessionQuality.tick(a);}
  const values = {
    "直播间入口 ID": a.rid, "主播 ID": a.anchor_id || "等待识别", "当前场次": a.session_id || "等待识别",
    "本次已收消息": num(a.message_count), "开播时长": duration(a), "当前在线": viewerCount(a),
    "最近消息": time(a.last_message_at), "直播状态查询时间": time(a.live_checked_at),
    "连接状态": a.detail || "尚未启动监控", "代理地址": a.proxy || "未分配"
  };
  document.querySelectorAll('[data-detail-field]').forEach(el => { el.textContent = values[el.dataset.detailField] ?? "—"; });
  const badge = document.querySelector('#detail-status');
  if (badge) replaceContent(badge, statusBadge(a));
  const toggle = document.querySelector('#detail-monitor-toggle');
  if (toggle) {
    toggle.dataset.action = active(a) ? "stop-one" : "start-one";
    toggle.className = active(a) ? "detail-stop" : "primary";
    replaceContent(toggle, `${icon(active(a) ? "stop" : "play")}${active(a) ? "停止监控" : "开始监控"}`);
  }
}

let proxyPolledAt = 0;
let proxyLoadPromise = null;
let polling = false,
  initialized = false;
function merge(data) {
  if (Array.isArray(data.anchors)) state.anchors = data.anchors;
  if (Array.isArray(data.archive_anchors))
    state.archiveAnchors = data.archive_anchors;
  if (data.settings) state.settings = data.settings;
  if (data.metrics) {
    state.metrics = data.metrics;
    viewerRefresh.update(data.metrics.viewer_refresh);
    if (data.metrics.proxy)
      state.proxy = { ...state.proxy, ...data.metrics.proxy };
  }
  if (data.export_jobs) state.jobs = getJobs(data.export_jobs);
  if (Number.isFinite(data.cursor)) state.cursor = data.cursor;
  if (data.dropped) state.dropped += Number(data.dropped) || 1;
  for (const e of data.events || []) {
    if (e.kind === "message" && LIVE_TYPES[e.msg_type || e.message_type])
      state.messages.push(e);
    else if (e.kind === "status" || e.kind === "log") state.logs.push(e);
  }
  state.messages = state.messages.slice(-1200);
  state.logs = state.logs.slice(-500);
  state.connected = true;
  state.error = "";
  updateConnection();
  renderStats();
  renderTasks();
  renderMessages();
  renderLogs();
  renderJobs();
  renderRuntime();
  renderProxy();
  updateDetail();
}
async function init() {
  try {
    const data = await bridge("init");
    if (initialized && data.cursor >= state.cursor) data.cursor = state.cursor;
    merge(data);
    initialized = true;
    renderPage();
  } catch (e) {
    state.connected = false;
    state.error = String(e?.message || e || "无法连接采集服务");
    updateConnection();
  }
}
async function poll(force = false) {
  if (polling || !state.connected || (document.hidden && !force)) return;
  polling = true;
  try {
    merge(await bridge("poll", { after: state.cursor, limit: 300, include_proxy: false }));
    if (proxyVisible() && Date.now() - proxyPolledAt > 3000) {
      await loadProxy();
    }
  } catch (e) {
    state.connected = false;
    state.error = `采集服务连接中断：${String(e?.message || e)}`;
    updateConnection();
  } finally {
    polling = false;
  }
}
async function request(method, params = {}, success = "") {
  const result = await bridge(method, params);
  if (success) toast(success);
  await poll(true);
  return result;
}
async function loadArchive() {
  try {
    const r = await bridge("archive_anchors");
    state.archiveAnchors = Array.isArray(r)
      ? r
      : r.anchors || r.archive_anchors || [];
    renderArchivePicker();
  } catch (e) {
    toast(String(e?.message || e), true);
  }
}
async function navigate(page) {
  const proxyRequested = page === "proxy";
  if (proxyRequested || page === "logs") page = "settings";
  if(page!=="settings")highlightSettings.unmount();
  if(page!=="settings")communitySettings.unmount();
  state.page = page;
  document.querySelector(".main").scrollTop = 0;
  renderPage();
  if (page === "archive") await loadArchive();
  if (proxyRequested) document.querySelector("#proxy-settings").open = true;
}
async function exportMonth() {
  if (!/^\d{4}-\d{2}$/.test(state.exportMonth)) throw new Error("请选择月份");
  if (!state.exportSelection.size) throw new Error("请至少选择一个已归档主播");
  if (!state.exportTypes.size) throw new Error("请至少选择一种消息类型");
  const available = new Set(state.archiveAnchors.filter(a=>tagMatches(a,state.exportTag)).map(a=>a.anchor_id));
  const anchorIds = [...state.exportSelection].filter(id=>available.has(id));
  if (!anchorIds.length) throw new Error("请至少选择一个当前标签下的主播");
  const month = state.exportMonth;
  const format = state.exportFormat;
  const types = [...state.exportTypes];
  state.exportBusy = true;
  const btn = document.querySelector("[data-action=export]");
  if (btn) btn.disabled = true;
  try {
    const defaultName = exportFileName({ anchorIds, anchors: state.archiveAnchors, month, format });
    const path = state.preview
      ? `演示导出/${defaultName}`
      : await invoke("choose_export_path", {
          format,
          defaultName,
        });
    if (!path) return;
    await request(
      "export",
      {
        anchor_ids: anchorIds,
        month,
        types,
        format,
        output_path: path,
      },
      "导出任务已开始，采集继续运行",
    );
  } finally {
    state.exportBusy = false;
    if (btn) btn.disabled = false;
  }
}
const quickMarks=new Map();
async function action(el) {
  if (await dataBackup.action(el)) return;
  if (await timeDisplaySettings.action(el)) return;
  if (await downloadSettings.action(el)) return;
  if (await clipPanel.action(el)) return;
  if (await compatibilityChecks.action(el)) return;
  if (await highlightSettings.action(el)) return;
  if (await communitySettings.action(el)) return;
  if (await sessionTimeline.action(el)) return;
  if (await sessionQuality.action(el)) return;
  const a = el.dataset.action,
    rid = el.dataset.rid;
  switch (a) {
    case "quick-mark": {
      const anchor=state.anchors.find(item=>item.rid===rid);
      if(!anchor||!live(anchor)||!anchor.anchor_id||!anchor.session_id){toast("直播状态已变化，请刷新后打点",true);return;}
      let operation=quickMarks.get(rid);
      if(operation?.busy)return;
      if(!operation||operation.session!==anchor.session_id)operation={session:anchor.session_id,key:crypto.randomUUID(),busy:false};
      operation.busy=true;quickMarks.set(rid,operation);el.disabled=true;
      try{await bridge("mark_create",{anchor_id:anchor.anchor_id,session_id:anchor.session_id,request_key:operation.key});quickMarks.delete(rid);toast(`${anchor.name} · 已打点`);void sessionTimeline.refreshBookmarks();}
      catch(error){toast(String(error?.message||error),true);}
      finally{operation.busy=false;if(el.isConnected)el.disabled=false;}
      return;
    }
    case "show-clip-jobs":
      closeModal();
      return navigate("clips");
    case "detail-tab": {
      if (state.modal?.kind !== "detail") return;
      state.modal.tab = el.dataset.tab;
      document.querySelectorAll('.detail-tabs [role="tab"]').forEach(t=>{const on=t.dataset.tab===state.modal.tab;t.setAttribute("aria-selected",String(on));t.tabIndex=on?0:-1;});
      document.querySelectorAll('.detail-panel').forEach(p=>{p.hidden=p.id!==`detail-panel-${state.modal.tab}`;});
      return updateDetail();
    }
    case "proxy-prev": state.proxyPage--; return renderProxy();
    case "proxy-next": state.proxyPage++; return renderProxy();
    case "server-logout":
      if (SERVER_MODE) return (await import("./server/bridge.js")).logout();
      return;
    case "window-close":
    case "window-minimize":
    case "window-maximize":
      if (state.preview) return toast("预览模式");
      {
        const action =
          a === "window-close"
            ? "close"
            : a === "window-minimize"
              ? "minimize"
              : "toggle_maximize";
        const r = await invoke("window_action", { action });
        applyWindowState(r);
        return;
      }
    case "navigate":
      return navigate(el.dataset.page);
    case "check-update":
      if (state.preview) return toast("请在桌面应用中检查更新");
      return checkForUpdates();
    case "install-update":
      if (state.preview) return;
      if (state.modal?.kind === "update-available") closeModal();
      if (state.anchors.some(anchor => anchor.active)) {
        state.modal = {kind: "update-confirm"};
        modal("暂停监控并更新", `<p>将保存消息并暂停监听。安装完成后自动重启、恢复监听。</p>`, `<button data-action="close-modal">取消</button><button class="primary" data-action="confirm-update">确认更新</button>`);
        return;
      }
      return installUpdate();
    case "confirm-update":
      closeModal();
      return installUpdate();
    case "release-notes":
    case "available-notes": {
      const current = a === "release-notes";
      const release = getUpdaterState();
      state.modal = { kind: "release-notes" };
      modal(`v${current ? esc(appVersion) : esc(release.version)} 更新日志`, releaseNotesMarkup(current ? bundledReleaseNotes : release.body));
      document.querySelector(".modal").classList.add("release-notes-dialog");
      return;
    }
    case "add":
      return showAdd();
    case "edit-tags":
      return showAnchorTags([rid]);
    case "tag-selected":
      return showAnchorTags([...state.selected]);
    case "tag-choice": {
      const input = document.getElementById(el.dataset.target);
      if (!input) return;
      const tags = normalizeTags(input.value);
      const tag = el.dataset.tag;
      input.value = normalizeTags(tags.includes(tag) ? tags.filter(value=>value!==tag) : [...tags, tag]).join("，");
      syncTagChoices(input);
      input.focus({preventScroll:true});
      return;
    }
    case "save-anchor-tags": {
      const context = state.modal;
      if (context?.kind !== "anchor-tags") return;
      const tags = normalizeTags(document.getElementById("edit-anchor-tags").value);
      await request("set_anchor_tags", {rids:context.rids, tags}, "标签已保存");
      await loadArchive();
      if (state.modal === context) closeModal();
      return;
    }
    case "save-detail-tags": {
      const context = state.modal;
      const input = document.getElementById("detail-tags");
      if (context?.kind !== "detail" || !input) return;
      const tags = normalizeTags(input.value);
      await request("set_anchor_tags", {rids:[rid], tags}, "标签已保存");
      await loadArchive();
      // Polling updates the data without replacing an input the user is editing.
      if (state.modal === context && input.isConnected) syncTagChoices(input);
      return;
    }
    case "close-modal":
      return closeModal();
    case "detail":
      return showDetail(rid);
    case "view-grid":
    case "view-list":
      state.view = a === "view-grid" ? "grid" : "list";
      return renderPage();
    case "toggle-offline":
      state.offlineExpanded = !state.offlineExpanded;
      return renderTasks();
    case "toggle-selection":
      state.selecting = !state.selecting;
      if (!state.selecting) state.selected.clear();
      return renderTasks();
    case "clear-selection":
      state.selecting = false;
      state.selected.clear();
      return renderTasks();
    case "select-visible": {
      const root = document.querySelector("#task-content"), viewport = root.getBoundingClientRect();
      root.querySelectorAll("article[data-rid], tr[data-rid]").forEach(el => {
        const rect = el.getBoundingClientRect();
        if (rect.bottom > viewport.top && rect.top < viewport.bottom) state.selected.add(el.dataset.rid);
      });
      return renderTasks();
    }
    case "open-live":
      if (state.preview) return toast("界面预览：正式应用使用默认浏览器打开直播间");
      return invoke("open_live_room", { rid });
    case "submit-add": {
      const values = document.querySelector("#add-values").value.trim();
      if (!values) throw new Error("请粘贴直播间链接或 ID");
      const tags = normalizeTags(document.querySelector("#add-tags").value);
      const r = await request("add_anchors", { values, tags });
      closeModal();
      await init();
      toast(
        `主播列表已更新${r.invalid?.length ? `，${r.invalid.length} 项无效` : ""}`,
      );
      break;
    }
    case "refresh-viewers":
      await viewerRefresh.start();
      await poll();
      break;
    case "batch-start":
    case "batch-stop":
    case "start-selected":
    case "stop-selected":
    case "start-one":
    case "stop-one": {
      const start = a.includes("start");
      const rids = a.endsWith("one")
        ? [rid]
        : a.endsWith("selected")
          ? [...state.selected]
          : state.anchors.map((x) => x.rid);
      if (!rids.length) return toast("请先添加或选择主播");
      await request(
        start ? "start" : "stop",
        { rids },
        start ? "监控任务已提交，将分批连接" : "停止请求已提交",
      );
      if (state.modal?.kind === "detail") updateDetail();
      break;
    }
    case "remove-one":
    case "remove-selected": {
      const rids = rid ? [rid] : [...state.selected];
      await request("remove", { rids }, "已移除监控任务，历史归档仍保留");
      rids.forEach((x) => state.selected.delete(x));
      closeModal();
      await init();
      break;
    }
    case "set-pc-unsupported": {
      const room = state.anchors.find(x => x.rid === rid);
      await request("set_pc_unsupported", { rid, unsupported: !pcUnsupported(room || {}) });
      await init();
      return showDetail(rid, "preferences");
    }
    case "rename":
      await request(
        "rename",
        { rid, name: document.querySelector("#rename-value").value },
        "显示名称已保存",
      );
      await init();
      return showDetail(rid, "preferences");
    case "pause":
      state.paused = !state.paused;
      messageFeed.setPaused(state.paused);
      document.querySelector("#pause-label").textContent = state.paused
        ? "恢复显示"
        : "暂停滚动";
      document.querySelector("#flow-label").textContent = state.paused
        ? "显示已暂停 · 归档继续"
        : "持续归档";
      return renderMessages();
    case "message-type":
      state.messageType = el.dataset.type;
      document
        .querySelectorAll("[data-action=message-type]")
        .forEach((x) =>
          x.classList.toggle("active", x.dataset.type === state.messageType),
        );
      return renderMessages();
    case "export-format":
      state.exportFormat = el.dataset.format;
      document
        .querySelectorAll("[data-action=export-format]")
        .forEach((x) =>
          x.classList.toggle("active", x.dataset.format === state.exportFormat),
        );
      return renderExportLayout();
    case "select-export-all":
      state.exportSelection = new Set(filterArchiveAnchors(state.archiveAnchors, state.exportSearch, state.exportTag).map(x=>x.anchor_id));
      return renderArchivePicker();
    case "clear-export-selection":
      state.exportSelection.clear();
      return renderArchivePicker();
    case "export":
      return exportMonth();
    case "delete-archive": return archiveDeletion.open();
    case "delete-load-sessions": return archiveDeletion.loadSessions();
    case "delete-more-sessions": return archiveDeletion.loadSessions(true);
    case "delete-preview": return archiveDeletion.preview();
    case "delete-back": return archiveDeletion.back();
    case "delete-execute": return archiveDeletion.execute();
    case "refresh-proxy": {
      if (state.proxyConfigBusy || state.proxyRefreshRequest || state.proxy.refresh?.active) return;
      state.proxyRefreshRequest = true;
      renderProxy();
      try {
        const result = await request("refresh_proxies");
        toast(result.accepted === false ? "当前批次仍在检测中" : state.proxy.source === "imported" ? "开始检测导入代理" : "开始获取并检测 50 个新代理");
        await loadProxy();
      } finally {
        state.proxyRefreshRequest = false;
        renderProxy();
      }
      return;
    }
    case "import-proxy": return showProxyImport();
    case "subscribe-proxy": return showProxySubscription();
    case "save-subscription": return saveProxySubscription();
    case "refresh-subscription": {
      await bridge("refresh_proxy_subscription", { id: el.dataset.id });
      return loadProxy();
    }
    case "remove-subscription": {
      await bridge("remove_proxy_subscription", { id: el.dataset.id });
      return loadProxy();
    }
    case "choose-proxy-file": return document.querySelector("#proxy-file").click();
    case "preview-proxy": return previewProxyImport();
    case "commit-proxy": return commitProxyImport();
    case "manage-proxies": return showImportedProxies();
    case "remove-proxy": {
      await bridge("remove_imported_proxies", { ids: [el.dataset.id] });
      if (state.modal?.kind === "proxy-list") await showImportedProxies();
      return loadProxy();
    }
    case "cancel-export":
      return request(
        "cancel_export",
        { job_id: el.dataset.job },
        "正在取消导出",
      );
    case "open-data":
      if (state.preview) return toast("界面演示：正式应用会打开本地数据目录");
      return invoke("open_data_folder");
    case "reconnect":
      return init();
  }
}
document.addEventListener("click", async (e) => {
  const el = e.target.closest("[data-action]");
  if (!el || el.disabled) return;
  const busyActions = [
    "submit-add",
    "batch-start",
    "batch-stop",
    "start-selected",
    "stop-selected",
    "start-one",
    "stop-one",
    "remove-one",
    "remove-selected",
    "rename",
    "save-detail-tags", "save-anchor-tags",
    "set-pc-unsupported",
    "preview-proxy",
    "commit-proxy",
    "remove-proxy",
    "save-subscription", "refresh-subscription", "remove-subscription",
    "delete-load-sessions", "delete-more-sessions", "delete-preview", "delete-execute",
  ];
  const busy = busyActions.includes(el.dataset.action) || el.dataset.action.startsWith("quality-");
  if (busy) el.disabled = true;
  try {
    await action(el);
  } catch (err) {
    toast(String(err?.message || err), true);
  } finally {
    if (busy && el.isConnected) el.disabled = false;
  }
});
document.addEventListener("input", (e) => {
  timeDisplaySettings.input(e.target);
  downloadSettings.input(e.target);
  clipPanel.input(e.target);
  const t = e.target;
  syncTagChoices(t);
  highlightSettings.input(t);
  communitySettings.input(t);
  sessionTimeline.input(t);
  sessionQuality.input(t);
  if (t.id === "proxy-text") resetProxyPreview();
  if (t.id === "proxy-search") {state.proxySearch=t.value;state.proxyPage=0;renderProxy();}
  if (t.id === "task-search") {
    state.search = t.value;
    document.querySelector("#task-content").scrollTop = 0;
    renderTasks();
  }
  if (t.id === "export-search") {
    state.exportSearch = t.value;
    renderArchivePicker();
  }
});
document.addEventListener("pointerover",e=>episodeTooltip.show(e.target));
document.addEventListener("pointerout",e=>{if(!e.relatedTarget)episodeTooltip.hide();});
document.addEventListener("focusin",e=>episodeTooltip.show(e.target));
document.addEventListener("toggle", e => {
  if (e.target.id === "proxy-settings" && e.target.open) {
    const content = document.querySelector("#proxy-settings-content");
    if (content && !content.childElementCount) content.innerHTML = proxyPanelMarkup();
    void loadProxy();
  }
  if (e.target.id === "diagnostics-settings" && e.target.open) renderLogs();
  if (e.target.classList?.contains("proxy-assignments") && e.target.open) renderProxy();
}, true);
document.addEventListener("change", (e) => {
  timeDisplaySettings.change(e.target);
  downloadSettings.change(e.target);
  clipPanel.change(e.target);
  const t = e.target;
  highlightSettings.change(t);
  communitySettings.change(t);
  void startupSettings.change(t);
  void sessionTimeline.change(t);
  void sessionQuality.change(t);
  if (t.id === "debug-logging") void configureDebugLogging(t.checked);
  if (t.id === "automatic-updates") {
    saveAutomaticUpdates(t.checked);
    if(t.checked && !state.preview) automaticUpdates.start(); else automaticUpdates.stop();
  }
  archiveDeletion.change(t);
  if (t.id === "proxy-protocol") resetProxyPreview();
  if (t.id === "proxy-file") void readProxyFile(t);
  if (t.id === "proxy-source") {
    void configureProxy({ source: t.value });
  }
  if (t.id === "ui-font") {
    setFont(t.value);
    return;
  }
  if (t.id === "proxy-enabled") {
    void configureProxy({ enabled: t.checked });
  }
  if (t.id === "proxy-direct") void configureProxy({ include_direct: t.checked });
  if (t.dataset.select) {
    t.checked
      ? state.selected.add(t.dataset.select)
      : state.selected.delete(t.dataset.select);
    renderTasks();
  }
  if (t.id === "task-tag-filter") {
    state.tagFilter = t.value;
    document.querySelector("#task-content").scrollTop = 0;
    renderTasks();
  }
  if (t.id === "export-tag-filter") {
    state.exportTag = t.value;
    state.exportSelection = new Set(filterArchiveAnchors(state.archiveAnchors,state.exportSearch,state.exportTag).map(a=>a.anchor_id));
    renderArchivePicker();
  }
  if (t.id === "task-filter") {
    state.filter = t.value;
    document.querySelector("#task-content").scrollTop = 0;
    renderTasks();
  }
  if (t.id === "message-anchor") {
    state.messageAnchor = t.value;
    renderMessages();
  }
  if (t.id === "export-month") {
    state.exportMonth = t.value;
    renderExportLayout();
  }
  if (t.dataset.exportAnchor) {
    t.checked
      ? state.exportSelection.add(t.dataset.exportAnchor)
      : state.exportSelection.delete(t.dataset.exportAnchor);
    renderArchivePicker();
  }
  if (t.dataset.exportType)
    t.checked
      ? state.exportTypes.add(t.dataset.exportType)
      : state.exportTypes.delete(t.dataset.exportType);
});
document.addEventListener("keydown", (e) => {
  if (e.target.matches?.('[role="tab"][data-action="detail-tab"]') && ["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key)) {
    const tabs = [...e.target.closest('[role="tablist"]').querySelectorAll('[role="tab"]')];
    const index = tabs.indexOf(e.target);
    const next = e.key === "Home" ? 0 : e.key === "End" ? tabs.length - 1 : (index + (e.key === "ArrowRight" ? 1 : tabs.length - 1)) % tabs.length;
    e.preventDefault(); tabs[next].click(); tabs[next].focus();
  }
  if (e.key === "Escape"&&!document.fullscreenElement) closeModal();
  if (e.key === "Tab" && state.modal) {
    const els = [
      ...(document.fullscreenElement||document.querySelector(".modal"))?.querySelectorAll(
        "button:not(:disabled),input,textarea,select",
      ),
    ].filter((el) => el.offsetParent !== null);
    if (!els.length) return;
    const first = els[0],
      last = els.at(-1);
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }
});
function applyWindowState(value = {}) {
  const maximized = Boolean(value.maximized);
  document.documentElement.classList.toggle("maximized", maximized);
  if (typeof value.focused === "boolean") document.documentElement.classList.toggle("window-inactive", !value.focused);
  const maximize = document.querySelector('[data-action="window-maximize"]');
  if (maximize) {
    maximize.title = maximized ? "还原窗口" : "最大化";
    maximize.setAttribute("aria-label", maximized ? "还原窗口" : "最大化窗口");
  }
}
async function refreshAfterBackupRestore() {
  sessionTimeline.unmount();
  if (!state.preview && window.__TAURI_INTERNALS__) {
    await invoke("native_replay_close").catch(() => {});
  }
  // Recreate every view and polling cursor from the newly restored backend.
  // A browser fixture keeps its in-memory restore result available for review.
  if (!state.preview) window.location.reload();
}
async function boot() {
  let savedFont;
  try { savedFont = localStorage.getItem("livescope.appearance.font"); } catch (_) {}
  setFont(savedFont, false);
  await prepareBridge();
  try{setDisplayTimezone((await bridge('time_display_settings',{})).timezone);}catch{}
  shell();
  subscribeUpdater(update => {
    renderUpdatePanel(state.anchors.some(active));
    renderUpdateBadge(update);
    updateNotice.observe(update);
  });
  if (!state.preview && window.__TAURI_INTERNALS__) {
    try {
      const r = await invoke("window_state");
      applyWindowState(r);
      await listen("window-state", (e) => applyWindowState(e.payload));
      await listen("backend-close-failed", (e) => toast(String(e.payload), true));
    } catch (_) {}
  }
  await init();
  if (state.preview && new URLSearchParams(location.search).has("update")) showUpdateNotice({version:"0.3.7",body:"- 改进直播状态展示\n- 优化场次回看体验"});
  if (!state.preview && window.__TAURI_INTERNALS__ && automaticUpdatesEnabled()) automaticUpdates.start();
  window.addEventListener("beforeunload",()=>automaticUpdates.stop(),{once:true});
  document.addEventListener("visibilitychange", () => { if (!document.hidden) void poll(true); });
  setInterval(() => poll(), 1000);
}
boot();
