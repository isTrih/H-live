const finite = value => typeof value === "number" && Number.isFinite(value);

// The video progress bar uses media time. Preserve activity array positions so
// its existing action indices still resolve to the corresponding archive item.
export function replayProgressData(data = {}, highlights = [], timeMap, mediaDuration) {
  const mapOffset = offset => {
    if (!finite(offset) || offset < 0 || !finite(mediaDuration) || mediaDuration <= 0 || typeof timeMap?.toMedia !== "function") return null;
    const mapped = timeMap.toMedia(offset);
    return finite(mapped) && mapped >= 0 && mapped <= mediaDuration ? mapped : null;
  };
  let buckets;
  if (Array.isArray(data?.messages) && data.messages.length) {
    // An archive bucket can straddle the recording start or a wall-clock gap.
    // Map each known chat first, then count on the media clock instead of moving
    // an entire archive bucket according to its (possibly unmapped) start.
    const span = finite(data.bucket_seconds) && data.bucket_seconds > 0 ? data.bucket_seconds : 60;
    const lastBucket = Math.max(0, Math.ceil(mediaDuration / span) - 1);
    const counts = new Map();
    for (const message of data.messages) {
      if (message?.message_type !== "chat" || message.time_source === "missing") continue;
      const mapped = mapOffset(message.offset_seconds);
      if (mapped === null) continue;
      const bucket = Math.min(lastBucket, Math.floor(mapped / span));
      counts.set(bucket, (counts.get(bucket) || 0) + 1);
    }
    buckets = [...counts].sort(([a],[b]) => a - b)
      .map(([bucket,count]) => ({offset_seconds:bucket * span,count}));
  } else {
    // Older archives may provide only aggregate buckets.
    buckets = (Array.isArray(data?.buckets) ? data.buckets : [])
      .filter(bucket => bucket && finite(bucket.count) && bucket.count >= 0)
      .map(bucket => ({...bucket, offset_seconds:mapOffset(bucket.offset_seconds)}))
      .filter(bucket => bucket.offset_seconds !== null)
      .sort((a,b) => a.offset_seconds - b.offset_seconds);
  }
  const activities = (Array.isArray(data?.activities) ? data.activities : [])
    .map(activity => {
      const result={...activity,offset_seconds:mapOffset(activity?.offset_seconds)};
      if(Object.hasOwn(activity||{},"end_offset_seconds")){
        const end=finite(activity.end_offset_seconds)?timeMap?.toMedia?.(activity.end_offset_seconds):null;
        result.end_offset_seconds=finite(end)&&end>=0&&finite(mediaDuration)&&mediaDuration>0?Math.min(end,mediaDuration):null;
      }
      return result;
    });
  const markers = (Array.isArray(highlights) ? highlights : [])
    .map((highlight, index) => ({...highlight, highlight_index:index, offset_seconds:mapOffset(highlight?.offset_seconds)}))
    .filter(highlight => highlight.offset_seconds !== null);
  return {buckets, activities, highlights:markers};
}
