export const UPDATE_INTERVAL_MS = 60 * 60 * 1000;
const preferenceKey = "livescope.updates.automatic";
export function automaticUpdatesEnabled(storage = globalThis.localStorage) {
  try { return storage?.getItem(preferenceKey) !== "false"; } catch { return true; }
}
export function saveAutomaticUpdates(enabled, storage = globalThis.localStorage) {
  try { storage?.setItem(preferenceKey, String(Boolean(enabled))); } catch { /* Session preference still applies. */ }
}

// Timer triggers a check only. Download/install are deliberately absent.
export function createAutomaticUpdates({ check, getState, onAvailable = () => {},
  setTimer = setTimeout, clearTimer = clearTimeout, interval = UPDATE_INTERVAL_MS } = {}) {
  let timer = null, enabled = false, running = false, generation = 0;
  const announced = new Set();
  function schedule(delay) {
    if (enabled && timer === null) timer = setTimer(tick, delay);
  }
  async function tick() {
    timer = null;
    if (!enabled || running) return;
    const currentGeneration = generation;
    const state = getState();
    if (state.busy || state.canInstall) { schedule(interval); return; }
    running = true;
    try {
      const result = await check();
      if (enabled && currentGeneration === generation && result.status === "available" && !announced.has(result.version)) {
        announced.add(result.version); onAvailable(result);
      }
    } catch { /* Background failure stays quiet; next hour retries. */ }
    finally { running = false; if (enabled) schedule(interval); }
  }
  function start() {
    if (enabled) return;
    enabled = true; generation++; schedule(1000);
  }
  function stop() {
    enabled = false; generation++;
    if (timer !== null) clearTimer(timer);
    timer = null;
  }
  return { start, stop };
}
