// Replay display preferences only. Archive grouping and download naming retain their own date rules.
export const DEFAULT_DISPLAY_TIMEZONE="Asia/Shanghai";
export const TIME_DISPLAY_MODE="worldclock";
let timezone=DEFAULT_DISPLAY_TIMEZONE;
const resolved=new Map(),formatters=new Map();

export function normalizeDisplayTimezone(value) {
  const raw=String(value??"").trim();
  if(!raw||raw.length>100||!/^[A-Za-z][A-Za-z0-9_+./-]*$/.test(raw))throw Error("请输入有效的 IANA 时区");
  if(resolved.has(raw))return resolved.get(raw);
  try {
    const canonical=new Intl.DateTimeFormat("zh-CN",{timeZone:raw}).resolvedOptions().timeZone;
    resolved.set(raw,canonical);resolved.set(canonical,canonical);return canonical;
  } catch {throw Error("无法识别此时区，请选择列表中的时区");}
}
export const getDisplayTimezone=()=>timezone;
export const currentDisplayTimezone=getDisplayTimezone;
export const getTimeDisplayPreferences=()=>({mode:TIME_DISPLAY_MODE,timezone});
export function setDisplayTimezone(value) {timezone=normalizeDisplayTimezone(value);return timezone;}

function parts(epochSeconds,timeZone=timezone) {
  if(typeof epochSeconds!=="number"||!Number.isFinite(epochSeconds))return null;
  const date=new Date(epochSeconds*1000);if(!Number.isFinite(+date))return null;
  let zone;try{zone=normalizeDisplayTimezone(timeZone);}catch{return null;}
  if(!formatters.has(zone))formatters.set(zone,new Intl.DateTimeFormat("en-CA",{timeZone:zone,year:"numeric",month:"2-digit",day:"2-digit",hour:"2-digit",minute:"2-digit",second:"2-digit",hourCycle:"h23"}));
  return Object.fromEntries(formatters.get(zone).formatToParts(date).map(part=>[part.type,part.value]));
}
export function worldDayKey(epochSeconds,timeZone=timezone) {
  const value=parts(epochSeconds,timeZone);return value?`${value.year}-${value.month}-${value.day}`:"";
}
export function formatWorldClock(epochSeconds,timeZone=timezone,{seconds=true,date=false}={}) {
  const value=parts(epochSeconds,timeZone);if(!value)return "—";
  const clock=`${value.hour}:${value.minute}${seconds?`:${value.second}`:""}`;
  return date?`${value.year.slice(-2)}/${value.month}/${value.day} ${clock}`:clock;
}
export function formatWorldTime(epochSeconds,options={}) {return formatWorldClock(epochSeconds,timezone,options);}
