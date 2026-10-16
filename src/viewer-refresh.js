export function createViewerRefresh({bridge,changed=()=>{},notify=()=>{}}){
 let pending=false,status={},completedId="";
 const finished=()=>{if(completedId===status.id)return;completedId=status.id;notify(`已刷新 ${status.updated||0} 个直播间人数${status.failed?`，${status.failed} 个未取得新数据`:""}${status.skipped?`，${status.skipped} 个已跳过`:""}`);};
 function update(next){
  if(!next||!/^\d+$/.test(next.id||""))return;
  if(/^\d+$/.test(status.id||"")&&BigInt(next.id)<BigInt(status.id))return;
  if(next.id===status.id&&(next.checked||0)<(status.checked||0))return;
  const wasActive=status.active;status=next;
  if(wasActive&&!status.active)finished();changed();
 }
 async function start(){
  if(pending||status.active)return;
  pending=true;changed();
  try{const next=await bridge("refresh_viewers");update(next);
   if(!next.active){if(!next.accepted)notify("请稍后再刷新");else if(!next.total)notify("暂无正在监控的开播直播间");else finished();}
  }catch(e){notify(String(e?.message||e),true);}finally{pending=false;changed();}
 }
 function snapshot(){return {disabled:pending||!!status.active,label:status.active?`刷新 ${status.checked||0}/${status.total||0}`:pending?"刷新中":"刷新人数"};}
 return {start,update,snapshot};
}
