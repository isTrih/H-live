import { invoke } from "@tauri-apps/api/core";
import { check } from "@tauri-apps/plugin-updater";

const initialState = () => ({
  status: "idle", busy: false, version: "", body: "", date: "",
  downloadedBytes: 0, totalBytes: 0, progress: null,
  message: "点击检查更新，确认安装后会自动暂停并恢复监控。", canInstall: false,
});

const guardMessages = {
  monitoring: "请先停止全部监控，等待连接和归档完成后重试。",
  exporting: "正在导出消息，请等待导出完成后重试。",
  pending: "后台操作或归档尚未完成，请稍后重试。",
  closing: "应用正在退出，请重新打开后检查更新。",
};

function safeError(error, phase) {
  const text = String(error ?? "").toLowerCase();
  if (/signature|minisign|pubkey|public key/.test(text))
    return "更新包签名验证失败，已阻止安装。请稍后检查更新或联系发布者。";
  if (/timeout|timed out/.test(text)) return "连接更新服务超时，请稍后重试。";
  if (/certificate|tls|ssl/.test(text)) return "更新服务的安全连接验证失败，请检查网络和系统时间。";
  if (phase === "prepare") return "后台未能安全结束，已取消安装。请确认归档完成后重新打开应用再试。";
  if (phase === "install") return "无法启动更新安装程序，请稍后重试或重新下载安装包。";
  if (phase === "download") return "更新包下载失败，请检查网络后重试。";
  return "暂时无法检查更新，请检查网络或等待发布者上传更新文件后重试。";
}

// Dependencies are injected only by unit tests. Production uses the signed
// Tauri updater and Rust-side readiness checks; no automatic installation.
export function createUpdaterController({ checkUpdate = check, call = invoke } = {}) {
  let state = initialState();
  let update = null;
  let downloaded = false;
  const listeners = new Set();
  const snapshot = () => ({ ...state });
  function publish(patch) {
    state = { ...state, ...patch };
    for (const listener of listeners) {
      try { listener(snapshot()); } catch { /* UI rendering must not interrupt cleanup. */ }
    }
  }
  function subscribe(listener) {
    listeners.add(listener);
    listener(snapshot());
    return () => listeners.delete(listener);
  }
  async function checkForUpdates() {
    if (state.busy) return snapshot();
    publish({ ...initialState(), status: "checking", busy: true, message: "正在检查更新…" });
    const previous = update;
    update = null;
    downloaded = false;
    try {
      await previous?.close().catch(() => {});
      update = await checkUpdate({ timeout: 20000 });
      if (update) {
        publish({ status: "available", busy: false, canInstall: true,
          version: String(update.version), body: String(update.body ?? ""), date: String(update.date ?? ""),
          message: `发现新版本 ${update.version}，可查看更新说明后安装。` });
      } else {
        publish({ status: "up_to_date", busy: false, message: "当前已是最新版本。" });
      }
    } catch (error) {
      publish({ status: "error", busy: false, message: safeError(error, "check") });
    }
    return snapshot();
  }
  async function installUpdate() {
    if (state.busy || !update || !state.canInstall) return false;
    let prepared = false;
    let phase = "guard";
    publish({ status: "preparing", busy: true, canInstall: false, message: "正在检查更新安装条件…" });
    try {
      const initialReadiness = await call("update_install_guard");
      if (!initialReadiness?.ready && initialReadiness?.reason !== "monitoring") {
        publish({ status: "error", busy: false, canInstall: true,
          message: guardMessages[initialReadiness?.reason] ?? guardMessages.pending });
        return false;
      }
      if (!downloaded) {
        phase = "download";
        publish({ status: "downloading", downloadedBytes: 0, totalBytes: 0, progress: null,
          message: "正在下载更新，完成后将验证签名…" });
        await update.download((event) => {
          if (event.event === "Started") {
            const length = Number(event.data?.contentLength);
            publish({ totalBytes: Number.isFinite(length) && length > 0 ? length : 0 });
          } else if (event.event === "Progress") {
            const chunk = Math.max(0, Number(event.data?.chunkLength) || 0);
            const received = state.downloadedBytes + chunk;
            publish({ downloadedBytes: received,
              progress: state.totalBytes > 0 ? Math.min(99, received / state.totalBytes * 100) : null });
          } else if (event.event === "Finished") {
            publish({ message: "下载完成，正在验证更新包签名…" });
          }
        }, { timeout: 600000 });
        // download() resolves only after Tauri verifies the artifact signature.
        downloaded = true;
        publish({ progress: 100 });
      }
      phase = "prepare";
      publish({ status: "preparing", message: "正在保存恢复列表、暂停监控并完成归档…" });
      const readiness = await call("prepare_update_install", {targetVersion: state.version});
      if (!readiness?.ready) {
        publish({ status: "error", busy: false, canInstall: true,
          message: guardMessages[readiness?.reason] ?? guardMessages.pending });
        return false;
      }
      prepared = true;
      phase = "install";
      publish({ status: "installing", message: "正在启动安装程序，应用将关闭并在更新后重新打开。" });
      await update.install({ restartAfterInstall: true });
      return true;
    } catch (error) {
      if (prepared || phase === "prepare") {
        try { await call("cancel_update_install"); } catch { /* Safe failure message below. */ }
      }
      publish({ status: "error", busy: false, canInstall: Boolean(update), message: safeError(error, phase) });
      return false;
    }
  }
  return { getState: snapshot, subscribe, checkForUpdates, installUpdate };
}

const controller = createUpdaterController();
export const getUpdaterState = controller.getState;
export const subscribeUpdater = controller.subscribe;
export const checkForUpdates = controller.checkForUpdates;
export const installUpdate = controller.installUpdate;
