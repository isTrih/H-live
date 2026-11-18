const clamp=value=>Math.max(0,Math.min(1,value));
export function danmakuProgress(motion,now){return clamp(motion.progress+(motion.paused?0:Math.max(0,(now-motion.at)/1000)*motion.rate/12));}
export function syncDanmakuMotion(previous,item,{now,paused,rate=1,revision=0}){
  const left=Number.parseFloat(item.left),progress=clamp(Number.isFinite(item.progress)?item.progress:1-(Number.isFinite(left)?left:100)/100);
  const restart=!previous||previous.revision!==revision||previous.static!==Boolean(item.static);
  return {progress:restart?progress:danmakuProgress(previous,now),at:now,paused:Boolean(paused),rate:Number.isFinite(rate)&&rate>0?rate:1,revision,static:Boolean(item.static)};
}
export function isCurrentReplaySnapshot(value,previous){
  if(Number.isSafeInteger(value.sequence)&&Number.isSafeInteger(previous.sequence)&&value.sequence<=previous.sequence)return false;
  if(Number.isSafeInteger(value.danmakuRevision)&&Number.isSafeInteger(previous.danmakuRevision)&&value.danmakuRevision<previous.danmakuRevision)return false;
  return true;
}
