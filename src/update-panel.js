import { version as appVersion } from "../package.json";
import currentNotes from "../release-notes/0.3.1.md?raw";
import { getUpdaterState } from "./updater.js";
import "./updater.css";
import { automaticUpdatesEnabled } from "./automatic-updates.js";

const escapeHtml = value => String(value ?? "").replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"})[c]);

export function updatePanelMarkup(icon) {
  return `<section class="panel form-panel update-panel"><div class="setting-title"><h2>关于与更新</h2><span class="badge">v${escapeHtml(appVersion)}</span></div><label class="automatic-update-option"><input id="automatic-updates" type="checkbox" ${automaticUpdatesEnabled()?"checked":""}>自动检查更新 · 每小时</label><div class="update-actions"><button data-action="release-notes">${icon("logs")}更新日志</button><button data-action="check-update">${icon("refresh")}检查更新</button></div><div id="update-state" aria-live="polite"></div></section>`;
}

export function renderUpdatePanel(hasActiveTasks = false) {
  const area = document.querySelector("#update-state");
  if (!area) return;
  const state = getUpdaterState();
  const check = document.querySelector('[data-action="check-update"]');
  if (check) check.disabled = state.busy;
  if (state.status === "idle") { area.innerHTML = ""; return; }
  const showRelease = Boolean(state.version && state.body);
  const downloading = state.status === "downloading";
  const progress = Number.isFinite(state.progress) ? Math.min(100, Math.max(0, state.progress)) : null;
  area.innerHTML = `<p class="update-message ${state.status === "error" ? "update-error" : ""}">${escapeHtml(state.message)}</p>${showRelease ? `<div class="update-release"><strong>v${escapeHtml(state.version)}</strong><button class="small" data-action="available-notes">查看更新内容</button></div>` : ""}${downloading ? `<progress aria-label="更新下载进度" max="100" ${progress === null ? "" : `value="${progress}"`}></progress><span class="help">${progress === null ? "正在下载…" : `已下载 ${Math.round(progress)}%`}</span>` : ""}${state.canInstall ? `<div class="update-install"><button class="primary" data-action="install-update" ${state.busy ? "disabled" : ""}>${hasActiveTasks ? "暂停监控并更新" : "下载并安装更新"}</button><span class="help">${hasActiveTasks ? "确认后暂停监控，更新完成后恢复。" : "安装时会关闭并重新启动应用。"}</span></div>` : ""}`;
}

// Render release text with a tiny, escaped subset of Markdown. Remote notes
// never introduce raw HTML, links, scripts or images into the WebView.
export function releaseNotesMarkup(text) {
  const lines = String(text || "暂无更新说明。").split(/\r?\n/);
  let listOpen = false;
  const chunks = [];
  for (const line of lines) {
    const item = line.match(/^[-*] (.+)$/);
    if (item) {
      if (!listOpen) { chunks.push("<ul>"); listOpen = true; }
      chunks.push(`<li>${escapeHtml(item[1])}</li>`);
      continue;
    }
    if (listOpen) { chunks.push("</ul>"); listOpen = false; }
    if (!line.trim()) continue;
    const title = line.match(/^#{1,4} (.+)$/);
    chunks.push(title ? `<h3>${escapeHtml(title[1])}</h3>` : `<p>${escapeHtml(line)}</p>`);
  }
  if (listOpen) chunks.push("</ul>");
  return `<article class="release-notes">${chunks.join("")}</article>`;
}

export const bundledReleaseNotes = currentNotes;
