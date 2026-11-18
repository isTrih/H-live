export function createStartupSettings({ invoke, request, preview, toast }) {
  let registered = false, monitoring = true, busy = false;
  const markup = () => `<section class="panel settings-section"><div class="setting-title"><h2>启动与后台</h2></div><label class="automatic-update-option"><input id="login-autostart" type="checkbox" disabled>开机启动 · 收起到托盘</label><label class="automatic-update-option"><input id="startup-monitoring" type="checkbox" disabled>启动后自动监听已添加主播</label><p class="help">红色按钮收起到系统托盘，黄色按钮最小化。需要退出时，右键托盘图标选择「退出」。</p><p class="help" id="startup-settings-status" role="status"></p></section>`;
  function paint() {
    for (const [id, value] of [["login-autostart", registered], ["startup-monitoring", monitoring]]) {
      const input = document.getElementById(id);
      if (input) { input.checked = value; input.disabled = busy; }
    }
    const status = document.getElementById("startup-settings-status");
    if (status) status.textContent = preview() ? "预览模式：此处演示设置，不修改系统开机启动。" : "更新安装完成后自动重启，并恢复更新前正在监听的主播。";
  }
  async function mount() {
    busy = true;
    try {
      const [os, settings] = await Promise.all([preview() ? { enabled: registered } : invoke("autostart_status"), request("startup_settings", {})]);
      registered = os.enabled;
      monitoring = settings.startup_monitoring;
    } catch (error) { toast(String(error), true); }
    finally { busy = false; paint(); }
  }
  async function change(input) {
    if (!["login-autostart", "startup-monitoring"].includes(input.id)) return;
    if (busy) { paint(); return; }
    const checked = input.checked;
    busy = true; paint();
    try {
      if (input.id === "login-autostart") {
        registered = preview() ? checked : (await invoke("configure_autostart", { enabled: checked })).enabled;
      } else {
        monitoring = (await request("configure_startup", { startup_monitoring: checked })).startup_monitoring;
      }
    } catch (error) { toast(String(error), true); }
    finally { busy = false; paint(); }
  }
  return { markup, mount, change };
}
