// The archive is the source for filtered live views, independently of the
// bounded cross-room IPC cache. A stale filter response may never repaint it.
export function createRecentMessages({bridge,getFilter,onChange=()=>{},now=Date.now,interval=2000}){
 let rows=[],key="",request=0,loading=false,error="",last=0,watermark="0",before=null;
 const currentKey=()=>{const f=getFilter();return JSON.stringify([f.type||"all",f.web_rid||""]);};
 const snapshot=()=>({rows:currentKey()===key?rows:[],loading,error:currentKey()===key?error:""});
 async function refresh(force=false){
  const filter=getFilter();if(!filter.active||(filter.paused&&before===null))return;
  const next=currentKey();
  if(filter.paused&&next===key&&!force)return;
  if(next===key&&(loading||(!force&&now()-last<interval)))return;
  if(next!==key)rows=[];
  const token=++request;key=next;loading=true;error="";last=now();
  try{
   const result=await bridge("recent_messages",{type:filter.type||"all",web_rid:filter.web_rid||"",limit:150,...(filter.paused?{before_id:before}:{})});
   if(token!==request||next!==currentKey()||!getFilter().active)return;
   rows=Array.isArray(result.messages)?result.messages:[];
   if(!filter.paused && /^\d+$/.test(result.high_watermark||""))watermark=result.high_watermark;
  }catch(e){if(token===request&&next===currentKey())error=String(e?.message||e);}
  finally{if(token===request){loading=false;onChange();}}
 }
 function invalidate(){request++;key="";rows=[];loading=false;error="";last=0;}
 function setPaused(paused){before=paused?watermark:null;invalidate();}
 return {snapshot,refresh,invalidate,setPaused};
}
