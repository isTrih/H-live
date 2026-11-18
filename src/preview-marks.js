// Development-only local fixtures. No recording lookup, file write or live API.
export function createPreviewMarks({getAnchor,storage=globalThis.localStorage,now=()=>Date.now()/1000}={}){
  const storageKey='qingkan.preview.marks.v1';
  let marks=[];try{const value=JSON.parse(storage?.getItem(storageKey)||'[]');if(Array.isArray(value))marks=value;}catch{}
  const save=next=>{storage?.setItem(storageKey,JSON.stringify(next));marks=next;};
  const clean=(value,max)=>{if(typeof value!=='string'||[...value].length>max||/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/.test(value))throw Error('打点内容无效');return value.trim();};
  return {
    get:id=>marks.find(mark=>mark.id===id),
    handle(method,p={}){
      if(!['mark_list','mark_create','mark_update','mark_delete'].includes(method))return undefined;
      if(method==='mark_list'){
        const matching=marks.filter(mark=>(!p.anchor_id||mark.anchor_id===p.anchor_id)&&(!p.session_id||mark.session_id===p.session_id)).sort((a,b)=>b.marked_at_epoch-a.marked_at_epoch);
        return {marks:structuredClone(matching.slice(p.offset||0,(p.offset||0)+(p.limit||1000))),total:matching.length,server_epoch:now(),preview:true};
      }
      if(method==='mark_create'){
        const existing=p.request_key&&marks.find(mark=>mark.request_key===p.request_key&&mark.anchor_id===p.anchor_id&&mark.session_id===p.session_id);
        if(existing)return {mark:structuredClone(existing),created:false,preview:true};
        const anchor=getAnchor(p);
        if(!anchor||!anchor.active||anchor.is_live!==true||anchor.session_id!==p.session_id)throw Error('当前直播场次已变化，请刷新后打点');
        const epoch=now(),stamp=new Date(epoch*1000).toISOString();
        const mark={id:crypto.randomUUID(),anchor_id:p.anchor_id,session_id:p.session_id,room_id:anchor.rid,anchor_name:anchor.name,
          title:clean(p.title??'',80)||'直播打点',note:clean(p.note??'',2000),marked_at_epoch:epoch,marked_at:stamp,created_at:stamp,updated_at:stamp,broadcast_started_at:anchor.broadcast_started_at,request_key:p.request_key};
        save([...marks,mark]);return {mark:structuredClone(mark),created:true,preview:true};
      }
      const found=marks.find(mark=>mark.id===p.id);if(!found)throw Error('打点不存在');
      if(method==='mark_delete'){save(marks.filter(mark=>mark.id!==p.id));return {deleted:true,id:p.id,preview:true};}
      const mark={...found,...(p.title!==undefined?{title:clean(p.title,80)||'直播打点'}:{}),...(p.note!==undefined?{note:clean(p.note,2000)}:{}),updated_at:new Date(now()*1000).toISOString()};
      save(marks.map(item=>item.id===p.id?mark:item));return {mark:structuredClone(mark),preview:true};
    }
  };
}
