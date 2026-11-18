import { esc } from "./presentation.js";

const finite = value => typeof value === "number" && Number.isFinite(value);
const kinds = { lottery: { label: "福袋", image: new URL("./assets/lottery-bag.png", import.meta.url).href }, red_packet: { label: "红包", image: new URL("./assets/red-packet.png", import.meta.url).href } };

// Only the archive's canonical rounds become markers. Unknown notifications stay
// in the complete message snapshot and are counted separately by the backend.
export function timelineActivities(data = {}) {
  const duration = data.duration_seconds;
  const unique = new Map();
  for (const [index, value] of (Array.isArray(data.activities) ? data.activities : []).entries()) {
    if (!value || !Object.hasOwn(kinds, value.kind)) continue;
    const id = String(value.id ?? value.activity_id ?? "");
    if (!id) continue;
    const key = `${value.kind}:${id}`;
    const timeSource = ["server", "observed"].includes(value.time_source) ? value.time_source : "unknown";
    const offset = timeSource !== "unknown" && finite(value.offset_seconds) && value.offset_seconds >= 0 && finite(duration) && value.offset_seconds <= duration ? value.offset_seconds : null;
    const endEpoch=value.kind==="lottery"?value.draw_at:value.expires_at;
    const endOffset=offset!==null&&finite(data.start_epoch)&&finite(endEpoch)&&endEpoch>data.start_epoch+offset?endEpoch-data.start_epoch:null;
    unique.set(key, { ...value, id, _key: key, _index: index, label: kinds[value.kind].label, time_source: timeSource, offset_seconds: offset,end_offset_seconds:endOffset, details: value.details && typeof value.details === "object" ? value.details : {} });
  }
  return [...unique.values()].sort((a, b) => (a.offset_seconds ?? Infinity) - (b.offset_seconds ?? Infinity) || a._index - b._index);
}

// Keep canonical indices and marker coordinates for existing seek consumers;
// the interval layer independently compresses dense overlaps below.
export function activityMarkerLayout(activities, duration, width = 800) {
  const usableWidth = finite(width) && width >= 80 ? width : 800;
  const rows = [];
  const markers = [];
  if (!finite(duration) || duration <= 0) return { markers, rows: 0, duration:0, intervals:[] };
  for (const [index, activity] of activities.entries()) {
    if (!finite(activity.offset_seconds)) continue;
    const x = Math.max(16, Math.min(usableWidth - 16, activity.offset_seconds / duration * usableWidth));
    let row = rows.findIndex(last => x - last >= 32);
    if (row < 0) row = rows.length;
    rows[row] = x;
    markers.push({ activity, index, row, left: x / usableWidth * 100, align: x < usableWidth * .25 ? "left" : x > usableWidth * .75 ? "right" : "center" });
  }
  return { markers, rows: rows.length, duration, intervals:activityIntervalSegments(markers,duration) };
}

// Dense rounds share one bounded layer per kind. Each half-open sweep segment
// retains the latest real round for the existing click contract, plus at most
// four hover samples; it never duplicates the full active set at every point.
export function activityIntervalSegments(markers, duration) {
  if(!finite(duration)||duration<=0)return [];
  const result=[];
  for(const kind of Object.keys(kinds)) {
    const rounds=markers.filter(({activity})=>activity.kind===kind&&finite(activity.offset_seconds)&&finite(activity.end_offset_seconds)&&activity.end_offset_seconds>activity.offset_seconds)
      .map(marker=>({...marker,start:Math.max(0,marker.activity.offset_seconds),end:Math.min(duration,marker.activity.end_offset_seconds)})).filter(round=>round.end>round.start);
    const boundaries=new Map();
    const boundary=time=>{if(!boundaries.has(time))boundaries.set(time,{starts:[],ends:[]});return boundaries.get(time);};
    for(const round of rounds){boundary(round.start).starts.push(round);boundary(round.end).ends.push(round.index);}
    const points=[...boundaries.keys()].sort((a,b)=>a-b),active=new Map(),segments=[];
    let newest=null,peak=0;
    for(let point=0;point<points.length-1;point++) {
      const start=points[point],end=points[point+1],changes=boundaries.get(start);
      for(const index of changes.ends){
        const node=active.get(index);if(!node)continue;
        if(node.newer!==null)active.get(node.newer).older=node.older;else newest=node.older;
        if(node.older!==null)active.get(node.older).newer=node.newer;
        active.delete(index);
      }
      for(const round of changes.starts.sort((a,b)=>a.index-b.index)){
        if(newest!==null)active.get(newest).newer=round.index;
        active.set(round.index,{round,older:newest,newer:null});newest=round.index;
      }
      if(!active.size||end<=start)continue;
      const samples=[];
      for(let index=newest;index!==null&&samples.length<4;index=active.get(index).older)samples.push(active.get(index).round);
      peak=Math.max(peak,active.size);
      segments.push({kind,start,end,count:active.size,samples,activity:samples[0].activity,index:samples[0].index});
    }
    if(peak<4){result.push(...rounds.map(round=>({...round,kind,count:1,dense:false})));continue;}
    for(const segment of segments)result.push({...segment,dense:true,opacity:.06+.14*Math.log1p(segment.count)/Math.log1p(peak),key:`${kind}:${segment.start}:${segment.end}`});
  }
  return result.sort((a,b)=>a.start-b.start||a.index-b.index);
}

export function activityNoticeWindow(activities, position, seconds = 10, preferredKey = "") {
  if (!finite(position)) return [];
  const nearby = activities.filter(activity => finite(activity.offset_seconds) && activity.offset_seconds <= position && activity.offset_seconds > position - seconds);
  const visible = nearby.slice(-2);
  const preferred = nearby.find(activity => activity._key === preferredKey);
  return preferred && !visible.includes(preferred) ? [preferred, ...visible.slice(-1)] : visible;
}

const text = value => value === null || value === undefined ? "" : typeof value === "string" || typeof value === "number" ? String(value) : "";
export function activityDetailLines(activity, clock) {
  const lines = [];
  const eventTime = clock(activity.epoch);
  if (activity.time_source === "observed") lines.push(`观察时间 ${eventTime}（活动通知时间未知）`);
  else if (activity.time_source === "server") lines.push(`通知时间 ${eventTime}（平台时间）`);
  else lines.push("通知时间未知，未放入时间轴");
  if (activity.kind === "lottery") lines.push(finite(activity.draw_at) && activity.draw_at > 0 ? `开奖时间 ${clock(activity.draw_at)}` : "开奖时间未知");
  else if (finite(activity.expires_at) && activity.expires_at > 0) lines.push(`结束时间 ${clock(activity.expires_at)}`);
  const timeLines = lines.length;
  const details = activity.details || {};
  for (const [key, label] of [["title", "标题"], ["prize_name", "奖品"], ["prize_description", "奖品说明"], ["prize_count", "奖品数量"], ["winner_count", "中奖名额"], ["total_count", "红包份数"], ["diamond_count", "钻石数量"]]) {
    const value = text(details[key]);
    if (value) lines.push(`${label}：${value}`);
  }
  if (Array.isArray(details.conditions)) for (const condition of details.conditions) {
    const value = text(condition?.content);
    if (value) lines.push(`参与条件：${value}`);
  }
  if (lines.length === timeLines && text(activity.content)) lines.push(`活动说明：${text(activity.content)}`);
  return lines;
}

const activityImage = activity => `<img src="${esc(kinds[activity.kind].image)}" alt="" aria-hidden="true">`;
export function activityMarkerHtml(layout, { clock, time }) {
  if (!layout.markers.length) return "";
  const tooltip=(activity,index,range=false)=>{const lines=activityDetailLines(activity,clock);return `<span class="timeline-activity-tooltip" id="timeline-activity-${range?"range-":""}tip-${index}" role="tooltip"><strong>${activity.label}${activity.ended?" · 已结束":"活动"}</strong>${lines.map(line=>`<span>${esc(line)}</span>`).join("")}<small>点击回看${activity.time_source==="observed"?"观察":"通知"}开始时刻</small></span>`;};
  const intervals=(layout.intervals||activityIntervalSegments(layout.markers,layout.duration)).map(({activity,index,start,end,dense,count,samples,key,opacity})=>{
    if(!finite(end)||end<=start||!layout.duration)return "";
    const left=Math.max(0,start/layout.duration*100),right=Math.min(100,end/layout.duration*100),row=activity.kind==="red_packet"?1:0;
    const label=`${activity.label} · ${time(start)} 至 ${time(end)} · ${activityDetailLines(activity,clock).join(" · ")}`;
    if(!dense)return `<button class="timeline-activity-interval ${activity.kind}" data-action="timeline-activity" data-activity="${index}" style="left:${left}%;width:${Math.max(0,right-left)}%;--activity-row:${row}" aria-label="${esc(label)}" aria-describedby="timeline-activity-range-tip-${index}"><span class="timeline-activity-range-label">${activity.label}</span>${tooltip(activity,index,true)}</button>`;
    const tipId=`timeline-activity-density-tip-${activity.kind}-${start}-${end}`;
    const summary=count>1?`${activity.label} · ${count} 轮重叠`:activity.label;
    const details=samples.map(({activity:round})=>{
      const endEpoch=round.kind==="lottery"?round.draw_at:round.expires_at;
      const timing=`${round.time_source==="observed"?"观察":"开始"} ${clock(round.epoch)}${finite(endEpoch)?` · ${round.kind==="lottery"?"开奖":"结束"} ${clock(endEpoch)}`:""}`;
      const prize=text(round.details?.prize_name)||text(round.details?.title);
      return `<span class="timeline-activity-density-round"><b>${esc(time(round.offset_seconds))}${prize?` · ${esc(prize)}`:""}</b><span>${esc(timing)}</span></span>`;
    }).join("");
    return `<button class="timeline-activity-interval ${activity.kind} activity-density" data-action="timeline-activity" data-activity="${index}" data-activity-key="${esc(key)}" data-overlap="${count}" style="left:${left}%;width:${Math.max(0,right-left)}%;--activity-alpha:${opacity.toFixed(3)}" aria-label="${esc(`${summary} · ${time(start)} 至 ${time(end)} · 点击回看 ${time(activity.offset_seconds)}`)}" aria-describedby="${tipId}"><span class="timeline-activity-tooltip" id="${tipId}" role="tooltip"><strong>${esc(summary)}</strong><span>${esc(time(start))} 至 ${esc(time(end))}</span>${details}${count>samples.length?`<small>还有 ${count-samples.length} 轮未展开</small>`:""}<small>点击回看 ${esc(time(activity.offset_seconds))}${count>1?` · 另有 ${count-1} 轮`:""}</small></span></button>`;
  }).join("");
  return intervals?`<div class="timeline-activity-track" aria-label="福袋与红包活动时间轴">${intervals}</div>`:"";
}

export function activityNoticeHtml(activities) {
  return activities.map(activity => {
    const details = activity.details || {};
    const condition = Array.isArray(details.conditions) ? details.conditions.map(item => text(item?.content)).filter(Boolean).join("；") : "";
    const summary = [text(details.prize_name) || text(details.title), condition].filter(Boolean).join(" · ") || text(activity.content) || "已归档活动通知";
    return `<span class="timeline-activity-notice ${activity.kind}">${activityImage(activity)}<b>${activity.label}活动</b>${activity.time_source === "observed" ? '<small>观察记录</small>' : ""}<span>${esc(summary)}</span></span>`;
  }).join("");
}
