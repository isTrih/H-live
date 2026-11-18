export function normalizeClipRange(start, end, duration = Infinity) {
  if (!Number.isFinite(start) || !Number.isFinite(end) || !(duration > 0)) return null;
  const from = Math.max(0, Math.min(start, end, duration));
  const to = Math.min(duration, Math.max(0, start, end));
  return to-from >= .1 ? {start: Math.round(from*1000)/1000, end: Math.round(to*1000)/1000} : null;
}
export function clipAroundTime(time, duration, before = 15, after = 30) {
  return Number.isFinite(time) && Number.isFinite(duration) && time >= 0 && time <= duration ? normalizeClipRange(time-before,time+after,duration) : null;
}
export function clipTime(value) {
  if (!Number.isFinite(value) || value < 0) return "—";
  const seconds = Math.floor(value);
  return [Math.floor(seconds/3600),Math.floor(seconds/60)%60,seconds%60].map(item=>String(item).padStart(2,"0")).join(":");
}
export function parseClipTime(value) {
  const raw = String(value).trim();
  if (/^\d+(?:\.\d+)?$/.test(raw)) return Number(raw);
  if (!/^\d+:\d{1,2}(?::\d{1,2}(?:\.\d+)?)?$/.test(raw)) return null;
  const parts = raw.split(":").map(Number);
  if (parts.slice(1).some(item=>item>=60)) return null;
  return parts.reduce((sum,item)=>sum*60+item,0);
}
