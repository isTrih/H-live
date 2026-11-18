import {esc,icon} from './presentation.js';
import {clipTime,normalizeClipRange} from './clip-selection.js';
import {formatWorldTime} from './time-display.js';

const finite=value=>typeof value==='number'&&Number.isFinite(value);
const key=context=>`${context?.anchorId||''}:${context?.sessionId||''}`;
const clock=epoch=>finite(epoch)?formatWorldTime(epoch):'时间未知';
const seconds=value=>String(value).trim()===''?NaN:Number(value);
const composingInputs=new WeakSet();

export function bookmarkPosition(mark,context,media=false){
  if(!finite(mark?.marked_at_epoch)||!finite(context?.startEpoch))return null;
  const offset=mark.marked_at_epoch-context.startEpoch;
  const position=media?context.toMedia?.(offset):offset;
  const duration=media?context.mediaDuration:context.duration;
  return finite(position)&&position>=0&&finite(duration)&&duration>0&&position<=duration?position:null;
}
export function bookmarkClip(mark,context,before=15,after=30){
  if(!context?.ready||!finite(before)||!finite(after)||before<0||after<0||before>600||after>600||before+after<=0)return null;
  const position=bookmarkPosition(mark,context,true);
  if(position===null)return null;
  const range=normalizeClipRange(Math.max(0,position-before),Math.min(context.mediaDuration,position+after),context.mediaDuration);
  return range&&range.end-range.start<=900?{...range,mark}:null;
}

// Keep editors and scrolling lists connected. Replacing a focused input and
// restoring its caret afterwards still cancels an active Chinese IME session.
function viewKey(node){
  if(node.nodeType===3){
    let next=node.nextSibling;while(next&&next.nodeType!==1)next=next.nextSibling;
    return `text:${next?(viewKey(next)||next.tagName):'end'}`;
  }
  if(node.nodeType!==1)return '';
  const key=node.id||node.getAttribute('data-bookmark-id')||node.getAttribute('data-bookmark-select')
    ||node.getAttribute('data-action')||node.getAttribute('class')?.split(/\s+/)[0]||'';
  return key?`${node.tagName}:${key}`:'';
}
function sameViewNode(current,next){
  return current.nodeType===next.nodeType&&(current.nodeType!==1||current.tagName===next.tagName);
}
function patchBookmarkView(current,next){
  if(current.nodeType===3){if(current.data!==next.data)current.data=next.data;return;}
  if(current.nodeType!==1)return;
  const field=current.tagName==='INPUT'||current.tagName==='TEXTAREA';
  for(const attribute of [...current.attributes]){
    if(field&&attribute.name==='value')continue;
    if(!next.hasAttribute(attribute.name))current.removeAttribute(attribute.name);
  }
  for(const attribute of [...next.attributes]){
    if(field&&attribute.name==='value')continue;
    if(current.getAttribute(attribute.name)!==attribute.value)current.setAttribute(attribute.name,attribute.value);
  }
  if(field){
    if(!composingInputs.has(current)&&current.value!==next.value)current.value=next.value;
    if(current.tagName==='INPUT'&&current.checked!==next.checked)current.checked=next.checked;
    return;
  }
  const previous=[...current.childNodes].map(node=>({node,key:viewKey(node)})),keyed=new Map(),used=new Set();
  for(const {node,key} of previous)if(key){if(!keyed.has(key))keyed.set(key,[]);keyed.get(key).push(node);}
  const plan=[...next.childNodes].map(child=>{
    const key=viewKey(child),match=key?keyed.get(key)?.find(node=>!used.has(node)&&sameViewNode(node,child))
      :previous.find(item=>!item.key&&!used.has(item.node)&&sameViewNode(item.node,child))?.node;
    if(match)used.add(match);return {child,match};
  });
  // Remove stale siblings before placing retained ones. Moving an editor's
  // ancestor around an obsolete error message would also cancel composition.
  for(const {node} of previous)if(!used.has(node))node.remove();
  let cursor=current.firstChild;
  for(const {child,match} of plan){
    if(match){
      if(match!==cursor)current.insertBefore(match,cursor);
      patchBookmarkView(match,child);cursor=match.nextSibling;
    }else current.insertBefore(child,cursor);
  }
}

export function createSessionBookmarks({bridge,getContext,changed=()=>{},toast=()=>{},onSeek=()=>{},onSelectClip=()=>{},onCreateClip}){
  let state=null;
  const boundViews=new WeakSet();
  const valid=current=>state===current&&key(getContext())===current.key;
  function select(){
    const context=getContext();
    state={key:key(context),anchorId:context?.anchorId,sessionId:context?.sessionId,marks:[],total:0,reading:false,busy:false,
      selected:new Set(),title:'',note:'',edit:null,deleting:'',before:'15',after:'30',error:'',requestKey:'',operations:new Map(),composing:new Set()};
  }
  function unmount(){state=null;}
  function bindView(section){
    const current=state;if(!section||boundViews.has(section)||!current||!valid(current))return;
    boundViews.add(section);
    section.addEventListener('compositionstart',event=>{
      if(!valid(current)||!event.target?.id?.startsWith('bookmark-'))return;
      composingInputs.add(event.target);current.composing.add(event.target);
    });
    section.addEventListener('compositionend',event=>{
      composingInputs.delete(event.target);current.composing.delete(event.target);
      if(valid(current))input(event.target);
    });
  }
  const composing=(current,ids)=>[...current.composing].some(node=>node.isConnected&&ids.includes(node.id));
  async function load(force=false){
    const current=state;if(!current||!valid(current)||!current.anchorId||!current.sessionId)return;
    if(current.reading){await current.loadPromise;if(force&&valid(current))return load();return;}
    current.reading=true;
    current.loadPromise=(async()=>{
    try{
      const response=await bridge('mark_list',{anchor_id:current.anchorId,session_id:current.sessionId,limit:1000});
      if(!valid(current))return;
      current.marks=(Array.isArray(response?.marks)?response.marks:[]).filter(mark=>mark.anchor_id===current.anchorId&&mark.session_id===current.sessionId)
        .sort((a,b)=>a.marked_at_epoch-b.marked_at_epoch||String(a.id).localeCompare(String(b.id)));
      current.total=Number.isFinite(response?.total)?response.total:current.marks.length;
      const ids=new Set(current.marks.map(mark=>mark.id));for(const id of current.selected)if(!ids.has(id))current.selected.delete(id);
      current.error='';
    }catch(error){if(valid(current))current.error=String(error?.message||error);}
    finally{if(valid(current)){current.reading=false;changed();}}
    })();
    await current.loadPromise;
  }
  function markup(){
    const current=state,context=getContext();if(!current||!valid(current)||!current.anchorId||!current.sessionId)return '';
    const live=context.live;
    return `<section class="session-bookmarks" aria-label="直播打点">
      <div class="bookmark-heading"><h3>${icon('bookmark')}直播打点 <span>${current.total}</span></h3><button class="small quiet" data-action="timeline-mark-refresh" ${current.reading?'disabled':''} aria-label="刷新打点">${icon('refresh')}</button></div>
      ${live?`<div class="bookmark-create"><input id="bookmark-title" aria-label="打点标题" maxlength="80" placeholder="记下精彩时刻（标题可选）" value="${esc(current.title)}"><button class="primary" data-action="timeline-mark-create" ${current.busy?'disabled':''}>${icon('bookmark')}${current.busy?'保存中…':'打点'}</button></div>`:''}
      ${current.error?`<p class="bookmark-error" role="alert">${esc(current.error)}</p>`:''}
      ${current.marks.length?`<div class="bookmark-tools"><label><input id="bookmark-select-all" type="checkbox" ${current.marks.every(mark=>current.selected.has(mark.id))?'checked':''}>全选</label><span>片段保留</span><label>前 <input id="bookmark-before" type="number" min="0" max="600" value="${esc(current.before)}" aria-label="打点前保留秒数"> 秒</label><label>后 <input id="bookmark-after" type="number" min="0" max="600" value="${esc(current.after)}" aria-label="打点后保留秒数"> 秒</label><button class="small" data-action="timeline-mark-download" ${current.busy||!context.ready||!current.selected.size?'disabled':''}>${icon('download')}下载精选${current.selected.size?`（${current.selected.size}）`:''}</button></div>
      <div class="bookmark-list">${current.marks.map(mark=>{
        const offset=bookmarkPosition(mark,context),media=bookmarkPosition(mark,context,true),edit=current.edit?.id===mark.id;
        return `<article class="bookmark-row ${current.selected.has(mark.id)?'selected':''}" data-bookmark-id="${esc(mark.id)}"><label class="bookmark-check"><input type="checkbox" data-bookmark-select="${esc(mark.id)}" aria-label="精选：${esc(mark.title)}" ${current.selected.has(mark.id)?'checked':''}></label>
          <button class="bookmark-position" data-action="timeline-bookmark" data-mark="${esc(mark.id)}" ${offset===null&&media===null?'disabled':''} title="跳转到 ${esc(clock(mark.marked_at_epoch))}"><time>${esc(clock(mark.marked_at_epoch))}</time><small>${offset===null&&media===null?'等待归档':clipTime(media??offset)}</small></button>
          <div class="bookmark-content">${edit?`<input id="bookmark-edit-title" aria-label="修改打点标题" maxlength="80" value="${esc(current.edit.title)}"><textarea id="bookmark-edit-note" aria-label="打点备注" maxlength="2000" rows="2" placeholder="备注（可选）">${esc(current.edit.note)}</textarea>`:`<strong>${esc(mark.title||'直播打点')}</strong>${mark.note?`<p>${esc(mark.note)}</p>`:''}`}</div>
          <div class="bookmark-actions">${edit?`<button class="small" data-action="timeline-mark-save" ${current.busy?'disabled':''}>保存</button><button class="small quiet" data-action="timeline-mark-edit-cancel">取消</button>`:current.deleting===mark.id?`<span>删除打点？</span><button class="small" data-action="timeline-mark-delete-confirm" data-mark="${esc(mark.id)}" ${current.busy?'disabled':''}>删除</button><button class="small quiet" data-action="timeline-mark-delete-cancel">保留</button>`:`<button class="small" data-action="timeline-mark-clip" data-mark="${esc(mark.id)}" ${!context.ready||media===null||current.busy?'disabled':''}>选片</button><button class="icon-btn quiet" data-action="timeline-mark-edit" data-mark="${esc(mark.id)}" aria-label="编辑打点：${esc(mark.title)}">${icon('edit')}</button><button class="icon-btn quiet" data-action="timeline-mark-delete" data-mark="${esc(mark.id)}" aria-label="删除打点：${esc(mark.title)}">${icon('trash')}</button>`}</div></article>`;
      }).join('')}</div>${!context.ready?'<p class="bookmark-hint">打点已保存，有录播后可选片下载。</p>':''}${current.total>current.marks.length?`<p class="bookmark-hint">显示最近 ${current.marks.length} 个打点，共 ${current.total} 个。</p>`:''}`:`<p class="bookmark-hint">${current.reading?'正在读取打点…':live?'点击打点，记录当前直播时刻。':'本场没有打点。'}</p>`}
    </section>`;
  }
  function markers(media=false){
    const current=state,context=getContext();if(!current||!valid(current))return '';
    const duration=media?context.mediaDuration:context.duration;
    return current.marks.map(mark=>{const position=bookmarkPosition(mark,context,media);return position===null?'':`<button class="timeline-bookmark" data-action="timeline-bookmark" data-mark="${esc(mark.id)}" style="left:${Math.max(1,Math.min(99,position/duration*100))}%" aria-label="打点 ${esc(clock(mark.marked_at_epoch))} ${esc(mark.title)}" title="${esc(clock(mark.marked_at_epoch))} · ${esc(mark.title)}${mark.note?` · ${esc(mark.note)}`:''}">${icon('bookmark')}</button>`;}).join('');
  }
  function input(target){
    const current=state;if(!current||!valid(current))return;
    if(target.id==='bookmark-title'){current.title=target.value;current.requestKey='';}
    if(target.id==='bookmark-before')current.before=target.value;
    if(target.id==='bookmark-after')current.after=target.value;
    if(target.id==='bookmark-edit-title'&&current.edit)current.edit.title=target.value;
    if(target.id==='bookmark-edit-note'&&current.edit)current.edit.note=target.value;
  }
  function change(target){
    const current=state;if(!current||!valid(current))return;
    if(target.id==='bookmark-select-all'){current.selected=target.checked?new Set(current.marks.map(mark=>mark.id)):new Set();changed();}
    if(target.dataset?.bookmarkSelect){if(target.checked)current.selected.add(target.dataset.bookmarkSelect);else current.selected.delete(target.dataset.bookmarkSelect);changed();}
    input(target);
  }
  async function action(element){
    const name=element?.dataset?.action;
    if(name!=='timeline-bookmark'&&!name?.startsWith('timeline-mark-'))return false;
    const current=state;if(!current||!valid(current))return true;
    const context=getContext(),mark=current.marks.find(mark=>mark.id===element.dataset.mark);
    if(name==='timeline-mark-refresh'){await load();return true;}
    if(name==='timeline-bookmark'){if(mark)onSeek(mark.marked_at_epoch);return true;}
    if(current.busy)return true;
    if(name==='timeline-mark-edit'){if(mark){current.edit={id:mark.id,title:mark.title,note:mark.note||''};current.deleting='';changed();}return true;}
    if(name==='timeline-mark-edit-cancel'){current.edit=null;changed();return true;}
    if(name==='timeline-mark-delete'){if(mark){current.deleting=mark.id;current.edit=null;changed();}return true;}
    if(name==='timeline-mark-delete-cancel'){current.deleting='';changed();return true;}
    if(name==='timeline-mark-clip'){
      const clip=bookmarkClip(mark,context,seconds(current.before),seconds(current.after));
      if(clip)onSelectClip(clip);else toast('请确认打点对应录像，片段总长需在 15 分钟内。',true);
      return true;
    }
    if(name==='timeline-mark-create'&&!context.live)return true;
    current.busy=true;current.error='';changed();
    try{
      if(name==='timeline-mark-create'){
        current.requestKey ||= crypto.randomUUID();
        const title=current.title,requestKey=current.requestKey;
        await bridge('mark_create',{anchor_id:current.anchorId,session_id:current.sessionId,title:title.trim(),request_key:requestKey});
        if(valid(current)){if(current.title===title&&!composing(current,['bookmark-title'])){current.title='';current.requestKey='';}toast('已打点');await load(true);}
      }else if(name==='timeline-mark-save'&&current.edit){
        const edit=current.edit,submitted={...edit};
        await bridge('mark_update',{...submitted,title:submitted.title.trim()});
        if(valid(current)){if(current.edit===edit&&edit.title===submitted.title&&edit.note===submitted.note&&!composing(current,['bookmark-edit-title','bookmark-edit-note']))current.edit=null;await load(true);}
      }else if(name==='timeline-mark-delete-confirm'&&mark&&current.deleting===mark.id){
        await bridge('mark_delete',{id:mark.id});if(valid(current)){current.deleting='';await load(true);}
      }else if(name==='timeline-mark-download'){
        if(typeof onCreateClip!=='function')throw Error('切片下载暂时不可用');
        const clips=current.marks.filter(mark=>current.selected.has(mark.id)).map(mark=>bookmarkClip(mark,context,seconds(current.before),seconds(current.after)));
        if(!clips.length||clips.some(clip=>!clip))throw Error('请确认精选打点均有对应录像，片段总长需在 15 分钟内。');
        let count=0,existing=0;
        for(const clip of clips){
          if(!valid(current))break;
          const operation=`${clip.mark.id}:${clip.start}:${clip.end}`;
          if(!current.operations.has(operation))current.operations.set(operation,crypto.randomUUID());
          const result=await onCreateClip({anchor_id:current.anchorId,session_id:current.sessionId,start_seconds:clip.start,end_seconds:clip.end,title:clip.mark.title,source:'bookmark',bookmark_id:clip.mark.id,request_key:current.operations.get(operation)});
          if(result===null||result===false)break;
          if(result?.existing&&['failed','cancelled'].includes(result.job?.status)){await bridge('clip_retry',{job_id:result.job.id});count++;}
          else if(result?.existing)existing++;else count++;
          current.selected.delete(clip.mark.id);
        }
        if(valid(current)&&(count||existing))toast(count?`已加入 ${count} 个精选下载任务${existing?`，另有 ${existing} 个已存在`:''}`:`已有 ${existing} 个下载任务`);
      }
    }catch(error){if(valid(current))current.error=String(error?.message||error);}
    finally{if(valid(current)){current.busy=false;changed();}}
    return true;
  }
  return {select,unmount,load,markup,markers,input,change,action,patchView:patchBookmarkView,bindView};
}
