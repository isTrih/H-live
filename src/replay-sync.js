const finite = value => typeof value === "number" && Number.isFinite(value);

// Binary search keeps full-session chat mapping inexpensive with thousands of
// HLS segments. Prefix ends preserve the first declared span if times overlap.
function spanLookup(spans, start, end) {
  const entries = spans.map((span,index) => ({span,index,start:start(span),end:end(span)}))
    .sort((a,b) => a.start - b.start || a.index - b.index);
  const ends = [];
  let furthest = -Infinity;
  for (const entry of entries) ends.push(furthest = Math.max(furthest,entry.end));
  return time => {
    let low = 0, high = entries.length;
    while (low < high) {
      const middle = (low + high) >> 1;
      if (entries[middle].start <= time) low = middle + 1;
      else high = middle;
    }
    let match = null;
    for (let index = low - 1; index >= 0 && ends[index] > time; index--) {
      const entry = entries[index];
      if (entry.end > time && (!match || entry.index < match.index)) match = entry;
    }
    return match?.span || null;
  };
}

// Use the backend's dated spans without filling gaps between fresh PDT anchors.
export function replayTimeMap(replay, archiveStart, correction = 0) {
  if(replay?.time_mapping_pending===true)return {
    dated:false,estimated:false,worldDated:false,
    toWorldEpoch:()=>null,clockWindow:()=>[],toArchive:()=>null,toMedia:()=>null,
  };
  const offset = finite(correction) ? correction : 0;
  const spans = (replay?.time_mapping || []).map(span=>({...span,epoch:typeof span.program_date_time==="string"?Date.parse(span.program_date_time)/1000:span.program_date_time}))
    .filter(span => finite(span.offset_seconds) && finite(span.duration) && span.duration > 0 && finite(span.epoch))
    .map(span => ({start:span.offset_seconds,end:span.offset_seconds+span.duration,epoch:span.epoch,estimated:span.time_source==="extrapolated"}));
  const dated = finite(archiveStart) && archiveStart > 0 && spans.length > 0;
  const mediaSpan = spanLookup(spans,span=>span.start,span=>span.end);
  const archiveSpan = spanLookup(spans,span=>span.epoch,span=>span.epoch+span.end-span.start);
  // Thousands of ordinary HLS segments usually share one wall-clock run.
  // Keep genuine media holes and PDT jumps separate; calibration is not involved.
  const runs=[];
  for(const span of spans){
    const previous=runs.at(-1);
    if(previous&&Math.abs(previous.end-span.start)<.00001&&Math.abs(previous.epoch+previous.end-previous.start-span.epoch)<.00001)
      previous.end=span.end;
    else runs.push({start:span.start,end:span.end,epoch:span.epoch});
  }
  const worldSpan=spanLookup(runs,span=>span.start,span=>span.end);
  const worldAt=time=>!finite(time)?null:worldSpan(time)||(runs.length&&time===runs.at(-1).end?runs.at(-1):null);
  return {
    dated: Boolean(dated),
    estimated: Boolean(dated && spans.some(span => span.estimated)),
    worldDated: Boolean(runs.length),
    toWorldEpoch(time) {
      const span=worldAt(time);
      return span?span.epoch+time-span.start:null;
    },
    clockWindow(time) {
      if(!finite(time)||!runs.length)return [];
      const current=worldAt(time);
      if(current){const index=runs.indexOf(current);return runs.slice(index,index+2);}
      const next=runs.findIndex(span=>span.start>time);
      return next<0?runs.slice(-1):runs.slice(Math.max(0,next-1),next+1);
    },
    toArchive(time) {
      if (!finite(time)) return null;
      if (!dated) return time + offset;
      const span = mediaSpan(time) || (time === spans.at(-1).end ? spans.at(-1) : null);
      return span ? span.epoch + time - span.start - archiveStart + offset : null;
    },
    toMedia(time) {
      if (!finite(time)) return null;
      if (!dated) return time - offset;
      const epoch = archiveStart + time - offset;
      const last=spans.at(-1);
      const span = archiveSpan(epoch)||(epoch===last.epoch+last.end-last.start?last:null);
      return span ? span.start + epoch - span.epoch : null;
    },
  };
}
