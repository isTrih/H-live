import {esc} from "./presentation.js";

const publicSettings = value => ({
  enabled: Boolean(value?.enabled),
  key_configured: Boolean(value?.key_configured),
  development_key_available: Boolean(value?.development_key_available),
});

export function createCommunitySettings({bridge, toast = () => {}, onSaved = () => {}}) {
  let current = null;
  const valid = ctx => current === ctx && ctx.root.isConnected;
  const keyInput = ctx => ctx.root.querySelector?.("#community-api-key");
  function clearKey(ctx) {
    ctx.draft.api_key = "";
    const input = keyInput(ctx);
    if (input) input.value = "";
  }
  function render(ctx) {
    if (!valid(ctx)) return;
    const disabled = ctx.loading || ctx.busy || ctx.loadFailed;
    const source = ctx.saved.key_configured ? "已保存 KEY，留空保留" : ctx.saved.development_key_available ? "留空使用本机 KEY" : "输入社区生态 KEY";
    const changed = ctx.draft.enabled !== ctx.saved.enabled || Boolean(ctx.draft.api_key);
    const status = ctx.loading ? "正在读取配置…" : ctx.busy ? "正在保存…" : ctx.notice || (changed ? "尚未保存" : ctx.saved.enabled ? "已启用" : "已关闭");
    ctx.root.innerHTML = `<div class="community-settings-heading"><h3>社区生态</h3></div>
      <p class="community-settings-description">接入已同步的直播回放。</p>
      <label class="community-settings-toggle"><span>启用回放</span><input id="community-enabled" type="checkbox" role="switch" ${ctx.draft.enabled ? "checked" : ""} ${disabled ? "disabled" : ""}></label>
      ${ctx.draft.enabled ? `<label class="community-settings-key">社区生态 KEY<input id="community-api-key" type="password" autocomplete="off" spellcheck="false" placeholder="${esc(source)}" ${disabled ? "disabled" : ""}></label>${ctx.saved.development_key_available && !ctx.saved.key_configured ? '<p class="community-settings-hint">已检测到本机 KEY。</p>' : ""}` : ""}
      ${ctx.error ? `<p class="community-settings-error" role="alert">${esc(ctx.error)}</p>` : ""}
      <div class="community-settings-actions"><span id="community-settings-status" role="status">${esc(status)}</span>${ctx.loadFailed ? '<button data-action="community-reload">重新读取</button>' : `<button class="primary" data-action="community-save" ${disabled ? "disabled" : ""}>保存设置</button>`}</div>`;
    // Passwords stay in the input property; never serialize them into markup.
    const input = keyInput(ctx);
    if (input) input.value = ctx.draft.api_key;
  }
  async function mount(root = document.querySelector("#community-settings")) {
    unmount();
    if (!root) return;
    const ctx = current = {root, saved: publicSettings(), draft: {enabled: false, api_key: ""}, loading: true, busy: false, loadFailed: false, error: "", notice: ""};
    render(ctx);
    try {
      const result = await bridge("community_settings");
      if (!valid(ctx)) return;
      ctx.saved = publicSettings(result);
      ctx.draft.enabled = ctx.saved.enabled;
    } catch {
      if (valid(ctx)) {
        ctx.loadFailed = true;
        ctx.error = "暂时无法读取社区生态配置，请重试。";
      }
    } finally {
      if (valid(ctx)) { ctx.loading = false; render(ctx); }
    }
  }
  function input(target) {
    const ctx = current;
    if (!ctx || !valid(ctx) || ctx.loading || ctx.busy || ctx.loadFailed || !ctx.draft.enabled || target.id !== "community-api-key") return;
    ctx.draft.api_key = target.value;
    ctx.notice = "";
    const status = ctx.root.querySelector?.("#community-settings-status");
    if (status) status.textContent = "尚未保存";
  }
  function change(target) {
    if (target.id !== "community-enabled") { input(target); return; }
    const ctx = current;
    if (!ctx || !valid(ctx) || ctx.loading || ctx.busy || ctx.loadFailed) return;
    ctx.draft.enabled = target.checked;
    if (!ctx.draft.enabled) clearKey(ctx);
    ctx.error = "";
    ctx.notice = "";
    render(ctx);
  }
  async function action(el) {
    const name = el?.dataset?.action;
    if (!["community-save", "community-reload"].includes(name)) return false;
    const ctx = current;
    if (!ctx || !valid(ctx) || ctx.loading || ctx.busy) return true;
    if (name === "community-reload") { await mount(ctx.root); return true; }
    if (ctx.loadFailed) return true;
    const apiKey = ctx.draft.api_key.trim();
    if (ctx.draft.enabled && !apiKey && !ctx.saved.key_configured && !ctx.saved.development_key_available) {
      ctx.error = "请输入社区生态 KEY。";
      render(ctx);
      keyInput(ctx)?.focus?.();
      return true;
    }
    const payload = {enabled: ctx.draft.enabled, ...(ctx.draft.enabled && apiKey ? {api_key: apiKey} : {})};
    clearKey(ctx);
    ctx.busy = true;
    ctx.error = "";
    ctx.notice = "";
    render(ctx);
    try {
      const saved = publicSettings(await bridge("community_configure", payload));
      // A completed save still invalidates replay data after the settings page closes.
      try { await onSaved(saved); } catch { /* The saved configuration remains authoritative. */ }
      if (!valid(ctx)) return true;
      ctx.saved = saved;
      ctx.draft.enabled = saved.enabled;
      ctx.notice = "设置已保存";
      toast("社区生态设置已保存");
    } catch {
      if (valid(ctx)) ctx.error = "保存失败，请检查社区生态 KEY 后重试。";
    } finally {
      if (valid(ctx)) { ctx.busy = false; render(ctx); }
    }
    return true;
  }
  function unmount() {
    if (current) clearKey(current);
    current = null;
  }
  return {mount, input, change, action, unmount};
}
