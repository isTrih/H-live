import {formatWorldClock,worldDayKey,getDisplayTimezone} from "./time-display.js";

const finite=value=>typeof value==="number"&&Number.isFinite(value);
export function elapsedClock(value){
  if(!finite(value)||value<0)return "—";
  const seconds=Math.floor(value);
  return [Math.floor(seconds/3600),Math.floor(seconds/60)%60,seconds%60].map(part=>String(part).padStart(2,"0")).join(":");
}
export function archiveClockMetadata(start,duration){
  const known=finite(start)&&start>0&&finite(duration)&&duration>=0;
  return {mode:known?"archive":"elapsed",timezone:getDisplayTimezone(),mediaDuration:duration,startEpoch:known?start:null,
    endEpoch:known?start+duration:null,spans:known?[{start:0,end:duration,epoch:start}]:[]};
}
export function replayClockMetadata(map,time,duration){
  return {mode:map.worldDated?"pdt":"elapsed",timezone:getDisplayTimezone(),mediaDuration:duration,
    startEpoch:map.toWorldEpoch(0),endEpoch:map.toWorldEpoch(duration),spans:map.clockWindow(time)};
}
export function clockEpoch(metadata,time){
  if(!finite(time))return null;
  const spans=metadata?.spans||[];
  const span=spans.find(span=>finite(span.start)&&finite(span.end)&&finite(span.epoch)&&time>=span.start&&time<span.end)
    ||(time===metadata?.mediaDuration?spans.findLast(span=>time===span.end):null);
  return span?span.epoch+time-span.start:null;
}
export function playbackClock(metadata,time,duration){
  const epoch=clockEpoch(metadata,time),end=metadata?.endEpoch,zone=metadata?.timezone||getDisplayTimezone();
  const crossDay=finite(metadata?.startEpoch)&&finite(end)&&worldDayKey(metadata.startEpoch,zone)!==worldDayKey(end,zone);
  const dated=metadata?.mode!=="elapsed"&&finite(epoch);
  return {position:dated?formatWorldClock(epoch,zone,{date:crossDay}):elapsedClock(time),
    end:metadata?.mode!=="elapsed"?(finite(end)?formatWorldClock(end,zone,{date:crossDay}):"—"):elapsedClock(duration),
    duration:`时长 ${elapsedClock(duration)}`,
    note:dated?"":"时间未对齐",crossDay,epoch:dated?epoch:null};
}
export function clockAxis({duration,toEpoch}){
  if(!finite(duration)||duration<=0)return [0,.25,.5,.75,1].map(fraction=>({fraction,offset:null,epoch:null,label:"—",aligned:false}));
  const points=[0,.25,.5,.75,1].map(fraction=>({offset:duration*fraction,fraction,epoch:toEpoch(duration*fraction)}));
  const days=new Set(points.map(point=>worldDayKey(point.epoch)).filter(Boolean));
  const dated=points.some(point=>finite(point.epoch));
  return points.map(point=>({...point,label:finite(point.epoch)?formatWorldClock(point.epoch,getDisplayTimezone(),{date:days.size>1})
    :dated?"—":elapsedClock(point.offset),aligned:finite(point.epoch)}));
}
