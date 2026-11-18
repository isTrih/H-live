// Queue a notice while another dialog is in use; announce each version once per launch.
export function createUpdateNotice({canOpen, show}) {
  const shown = new Set();
  let pending = null;
  function flush() {
    if (!pending || shown.has(pending.version) || !canOpen()) return;
    const update = pending;
    pending = null;
    shown.add(update.version);
    show(update);
  }
  function observe(update) {
    if (update.status === "available" && update.canInstall && update.version && !shown.has(update.version)) pending = {...update};
    else if (["checking", "up_to_date"].includes(update.status)) pending = null;
    flush();
  }
  return {observe, flush};
}
