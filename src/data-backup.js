import {esc,icon,num} from "./presentation.js";
import {formatWorldTime} from "./time-display.js";
import {normalizeTags,tagMatches,tagOptions,UNTAGGED_TAG,tagsMarkup} from "./anchor-tags.js";

const FONT="livescope.appearance.font",UPDATES="livescope.updates.automatic";
const MAX_SELECTED_ANCHORS=10000,selectionLimitMessage="一次最多选择10000位主播，请搜索并勾选所需主播";
const replayKey=/^livescope\.replay-offset\.(\d{1,64}):(\d{1,64})$/;
const knownKey=key=>key===FONT||key===UPDATES||Boolean(replayKey.test(key));
function checkedValue(key,value){
  if(typeof value!=="string")throw Error("备份中的界面设置格式无效");
  if(key===FONT&&["douyin","chill"].includes(value))return value;
  if(key===UPDATES&&["true","false"].includes(value))return value;
  const match=replayKey.exec(key);
  if(match&&/[1-9]/.test(match[1])&&/[1-9]/.test(match[2])&&value.trim()!==""&&Number.isFinite(Number(value))&&Math.abs(Number(value))<=86400)
    return String(Number(value));
  throw Error("备份包含不支持的界面设置");
}
export function validateBackupPreferences(preferences){
  if(!preferences||typeof preferences!=="object"||Array.isArray(preferences))throw Error("备份中的界面设置格式无效");
  const entries=Object.entries(preferences);
  if(entries.length>10000)throw Error("备份中的回看校准设置过多");
  const valid={};
  for(const [key,value]of entries){
    if(!knownKey(key))throw Error("备份包含不支持的界面设置");
    valid[key]=checkedValue(key,value);
  }
  return valid;
}
function storedEntries(storage){
  if(!storage)return [];
  const entries=[];
  for(let index=0;index<storage.length;index++){
    const key=storage.key(index);
    if(typeof key==="string"&&knownKey(key))entries.push([key,storage.getItem(key)]);
  }
  return entries;
}
export function collectBackupPreferences(storage=globalThis.localStorage){
  const preferences={};
  for(const [key,value]of storedEntries(storage)){
    try{preferences[key]=checkedValue(key,value);}catch{/* Invalid local values do not become portable preferences. */}
  }
  return validateBackupPreferences(preferences);
}
export function applyBackupPreferences(preferences,storage=globalThis.localStorage){
  const valid=validateBackupPreferences(preferences),written=[];
  if(!storage){if(Object.keys(valid).length)throw Error("本机界面设置不可写");return valid;}
  try{
    for(const [key,value]of Object.entries(valid)){
      // Existing settings, including invalid legacy values, belong to this device.
      if(storage.getItem(key)!==null)continue;
      written.push(key);storage.setItem(key,value);
    }
    return collectBackupPreferences(storage);
  }catch(error){
    // Undo only this merge's additions; do not remove or rewrite pre-existing keys.
    for(const key of written){try{storage.removeItem(key);}catch{}}
    throw Error(`界面设置合并失败：${String(error?.message||error)}`);
  }
}
export function validateBackupAnchors(response){
  if(!Array.isArray(response?.anchors)||response.anchors.length>100000)throw Error("主播列表无效，请重新读取");
  const seen=new Set();
  return response.anchors.map(anchor=>{
    const id=anchor?.anchor_id;
    if(typeof id!=="string"||!id.trim()||id.length>160||/[\u0000-\u001f\u007f]/.test(id)||seen.has(id))throw Error("主播列表无效，请重新读取");
    seen.add(id);
    const name=String(anchor.name||anchor.nickname||anchor.display_name||anchor.web_rid||id).slice(0,512);
    const count=key=>Number.isSafeInteger(anchor[key])&&anchor[key]>=0?anchor[key]:null;
    if(anchor.tags!==undefined&&(!Array.isArray(anchor.tags)||anchor.tags.length>200||anchor.tags.some(tag=>typeof tag!=="string"||!tag.trim())))throw Error("主播标签格式无效，请重新读取");
    const tags=anchor.tags===undefined?[]:normalizeTags(anchor.tags);
    return {anchor_id:id,name,web_rid:String(anchor.web_rid||"").slice(0,160),sessions:count("sessions"),messages:count("messages"),tags};
  });
}
export function validateBackupSummary(summary){
  if(!summary||summary.format!==1||typeof summary.app_version!=="string"||summary.app_version.length>80||
    typeof summary.created_at!=="string"||summary.created_at.length>80||!Number.isFinite(Date.parse(summary.created_at))||
    !summary.counts||typeof summary.counts!=="object")throw Error("备份信息不完整，请重新选择");
  const counts={};
  for(const key of ["anchors","sessions","messages","marks","downloads"]){
    const value=summary.counts[key];
    if(!Number.isSafeInteger(value)||value<0)throw Error("备份数量信息无效，请重新选择");
    counts[key]=value;
  }
  const monitored=summary.counts.monitored_anchors??counts.anchors;
  if(!Number.isSafeInteger(monitored)||monitored<0)throw Error("备份数量信息无效，请重新选择");
  counts.monitored_anchors=monitored;
  return {format:1,app_version:summary.app_version,created_at:summary.created_at,counts};
}
const errorText=error=>String(error?.message||error||"操作失败，请重试").slice(0,600);
const invalidToken=error=>/过期|失效|无效.*(?:token|令牌|预览)|(?:token|令牌|预览).*(?:无效|不存在|找不到)|expired|invalid.?token/i.test(errorText(error))||["BACKUP_TOKEN_EXPIRED","INVALID_BACKUP_TOKEN"].includes(error?.code);
const labels={export:"正在导出备份…",inspect:"正在读取备份…",restore:"正在增量导入，请稍候…",discard:"正在关闭预览…",refresh:"正在刷新应用…"};

export function createDataBackup({invoke,storage=globalThis.localStorage,toast=()=>{},onRestored=()=>{}}){
  let root=null,generation=0,anchorsRevision=0;
  const state={busy:"",preview:null,error:"",notice:"",savedPath:"",needsRefresh:false,restoredPreferences:null,
    scope:"all",anchors:[],anchorsLoaded:false,anchorsLoading:false,anchorsError:"",selected:new Set(),search:"",tagFilter:""};
  const connected=()=>root&&root.isConnected!==false;
  const discard=token=>Promise.resolve().then(()=>invoke("data_backup_discard",{token}));
  const query=selector=>root?.querySelector?.(selector);
  function matchingAnchors(){
    const search=state.search.trim().toLocaleLowerCase();
    return state.anchors.filter(anchor=>tagMatches(anchor,state.tagFilter)&&(!search||`${anchor.name} ${anchor.anchor_id} ${anchor.web_rid} ${anchor.tags.join(" ")}`.toLocaleLowerCase().includes(search)));
  }
  function anchorRows(){
    const matches=matchingAnchors();
    if(state.anchorsLoading&&!state.anchorsLoaded)return '<p class="data-backup-anchor-empty">正在读取主播…</p>';
    if(!matches.length)return `<p class="data-backup-anchor-empty">${state.anchorsLoaded?(state.search||state.tagFilter?"未找到相关主播":"暂无可导出的主播"):"请重新读取主播列表"}</p>`;
    return matches.slice(0,500).map(anchor=>`<label class="data-backup-anchor"><input type="checkbox" data-backup-anchor="${esc(anchor.anchor_id)}" ${state.selected.has(anchor.anchor_id)?"checked":""} ${state.busy?"disabled":""}><span><strong>${esc(anchor.name)}</strong>${tagsMarkup(anchor,{limit:3})}<small>${[anchor.sessions===null?"":`${num(anchor.sessions)} 场`,anchor.messages===null?"":`${num(anchor.messages)} 条消息`].filter(Boolean).join(" · ")||esc(anchor.web_rid||anchor.anchor_id)}</small></span></label>`).join("")+
      (matches.length>500?'<p class="data-backup-anchor-empty">仅显示前 500 位，请搜索查找更多主播。</p>':"");
  }
  function selectionText(){return `已选择 ${num(state.selected.size)} / ${num(state.anchors.length)} 位主播`;}
  function renderSelection(){
    const list=query("#data-backup-anchor-list"),scroll=list?.scrollTop;
    if(list){list.innerHTML=anchorRows();list.scrollTop=scroll||0;}
    const count=query("#data-backup-anchor-count");if(count)count.textContent=selectionText();
  }
  function handleInput(event){
    if(event.target?.id!=="data-backup-anchor-search"||state.busy)return;
    state.search=event.target.value;renderSelection();
  }
  function handleChange(event){
    if(state.busy)return;
    if(event.target?.id==="data-backup-anchor-tag"){
      const tag=event.target.value;
      if(tag!==""&&tag!==UNTAGGED_TAG&&!tagOptions(state.anchors).includes(tag))return;
      state.tagFilter=tag;
      const matches=matchingAnchors();
      state.selected=new Set(matches.length<=MAX_SELECTED_ANCHORS?matches.map(anchor=>anchor.anchor_id):[]);
      state.error=matches.length>MAX_SELECTED_ANCHORS?selectionLimitMessage:"";
      const error=query("#data-backup-operation-error");if(error)error.textContent=state.error;
      if(state.error)toast(state.error,true);
      renderSelection();return;
    }
    const id=event.target?.dataset?.backupAnchor;
    if(typeof id!=="string"||!state.anchors.some(anchor=>anchor.anchor_id===id&&tagMatches(anchor,state.tagFilter)))return;
    if(event.target.checked&&!state.selected.has(id)&&state.selected.size>=MAX_SELECTED_ANCHORS){
      event.target.checked=false;state.error=selectionLimitMessage;
      const error=query("#data-backup-operation-error");if(error)error.textContent=state.error;
      toast(state.error,true);return;
    }
    if(event.target.checked)state.selected.add(id);else state.selected.delete(id);
    const count=query("#data-backup-anchor-count");if(count)count.textContent=selectionText();
    state.error="";const error=query("#data-backup-operation-error");if(error)error.textContent="";
  }
  async function readAnchors(){
    if(state.anchorsLoading)return;
    const requestGeneration=generation,requestRevision=anchorsRevision;state.anchorsLoading=true;state.anchorsError="";render();
    try{
      const anchors=validateBackupAnchors(await invoke("data_backup_anchors",{}));
      if(requestGeneration!==generation||requestRevision!==anchorsRevision||!connected())return;
      const available=new Set(anchors.map(anchor=>anchor.anchor_id));
      state.anchors=anchors;state.anchorsLoaded=true;
      if(state.tagFilter&&state.tagFilter!==UNTAGGED_TAG&&!tagOptions(anchors).includes(state.tagFilter)){state.tagFilter="";state.selected.clear();}
      const eligible=new Set(anchors.filter(anchor=>tagMatches(anchor,state.tagFilter)).map(anchor=>anchor.anchor_id));
      state.selected=new Set([...state.selected].filter(id=>available.has(id)&&eligible.has(id)));
    }catch(error){if(requestGeneration===generation&&requestRevision===anchorsRevision&&connected())state.anchorsError=errorText(error);}
    finally{if(requestGeneration===generation&&requestRevision===anchorsRevision){state.anchorsLoading=false;render();}}
  }
  function render(){
    if(!connected())return;
    const active=globalThis.document?.activeElement,searchFocused=active?.id==="data-backup-anchor-search"&&root.contains?.(active);
    const selection=searchFocused?[active.selectionStart,active.selectionEnd]:null,listScroll=query("#data-backup-anchor-list")?.scrollTop||0;
    const preview=state.preview,disabled=Boolean(state.busy),summary=preview?.summary;
    root.innerHTML=`<div class="data-backup-heading"><h3>${icon("folder")}备份与恢复</h3><span>数据备份包</span></div>
      <p class="data-backup-description">直播记录、弹幕、打点、下载记录与设置。</p>
      <p class="data-backup-footnote">不包含录像文件和服务密钥。导入时会暂时停止监听，保留已有数据和设置。</p>
      <div class="data-backup-scope" role="group" aria-label="导出范围"><span>导出范围</span><button class="small ${state.scope==="all"?"selected":"quiet"}" data-action="data-backup-scope-all" aria-pressed="${state.scope==="all"}" ${disabled?"disabled":""}>全部主播</button><button class="small ${state.scope==="selected"?"selected":"quiet"}" data-action="data-backup-scope-selected" aria-pressed="${state.scope==="selected"}" ${disabled?"disabled":""}>指定主播</button></div>
      ${state.scope==="selected"?`<section class="data-backup-anchor-picker" aria-label="选择导出主播"><div class="data-backup-anchor-tools"><input id="data-backup-anchor-search" type="search" placeholder="搜索主播名称或 ID" aria-label="搜索导出主播" value="${esc(state.search)}" ${disabled?"disabled":""}><select id="data-backup-anchor-tag" aria-label="按标签选择导出主播" ${disabled||!state.anchorsLoaded?"disabled":""}><option value="">全部标签</option><option value="${UNTAGGED_TAG}" ${state.tagFilter===UNTAGGED_TAG?"selected":""}>无标签</option>${tagOptions(state.anchors).map(tag=>`<option value="${esc(tag)}" ${state.tagFilter===tag?"selected":""}>${esc(tag === "无标签" || tag === "全部标签" ? `标签：${tag}` : tag)}</option>`).join("")}</select><button class="small quiet" data-action="data-backup-anchor-all" ${disabled||!state.anchorsLoaded?"disabled":""}>全选当前结果</button><button class="small quiet" data-action="data-backup-anchor-clear" ${disabled?"disabled":""}>清空</button></div><div class="data-backup-anchor-meta"><span id="data-backup-anchor-count">${selectionText()}</span><button class="small quiet" data-action="data-backup-anchors-retry" ${disabled||state.anchorsLoading?"disabled":""}>${state.anchorsLoading?"读取中…":"刷新列表"}</button></div><div id="data-backup-anchor-list" class="data-backup-anchor-list">${anchorRows()}</div>${state.anchorsError?`<p class="data-backup-error" role="alert">${esc(state.anchorsError)}</p>`:""}</section>`:""}
      <div class="data-backup-actions"><button data-action="data-backup-export" ${disabled?"disabled":""}>${icon("download")}导出备份</button><button data-action="data-backup-inspect" ${disabled?"disabled":""}>${icon("folder")}导入备份</button></div>
      ${preview?`<section class="data-backup-preview" aria-label="待导入备份"><div class="data-backup-file"><strong>${esc(preview.file_name)}</strong><span>v${esc(summary.app_version)} · ${esc(formatWorldTime(Date.parse(summary.created_at)/1000,{date:true}))}</span></div>
        <dl class="data-backup-counts">${[["monitored_anchors","监控主播"],["anchors","归档主播"],["sessions","场次"],["messages","消息"],["marks","打点"],["downloads","下载记录"]].map(([key,label])=>`<div><dt>${label}</dt><dd>${num(summary.counts[key])}</dd></div>`).join("")}</dl>
        <p class="data-backup-replace">增量合并备份，保留已有记录与设置，自动跳过重复内容。导入前自动备份。</p>
        <div class="data-backup-confirm"><button class="quiet" data-action="data-backup-cancel" ${disabled?"disabled":""}>取消预览</button><button class="primary" data-action="data-backup-restore" ${disabled?"disabled":""}>确认增量导入</button></div></section>`:""}
      <p class="data-backup-status" role="status">${esc(labels[state.busy]||state.notice)}</p>
      ${state.savedPath?`<p class="data-backup-path selectable">${esc(state.savedPath)}</p>`:""}
      <p id="data-backup-operation-error" class="data-backup-error" role="alert">${esc(state.error)}</p>
      ${state.needsRefresh?`<button data-action="data-backup-refresh" ${disabled?"disabled":""}>刷新应用</button>`:""}`;
    const list=query("#data-backup-anchor-list");if(list)list.scrollTop=listScroll;
    if(searchFocused&&!disabled){const input=query("#data-backup-anchor-search");input?.focus?.({preventScroll:true});if(selection)input?.setSelectionRange?.(...selection);}
  }
  function mount(element=globalThis.document?.querySelector("#data-backup")){
    unmount();root=element||null;root?.addEventListener?.("input",handleInput);root?.addEventListener?.("change",handleChange);render();
    if(connected()&&state.scope==="selected"&&!state.anchorsLoaded)void readAnchors();
  }
  function unmount(){
    root?.removeEventListener?.("input",handleInput);root?.removeEventListener?.("change",handleChange);
    root=null;generation++;
    state.anchorsLoading=false;
    if(state.preview&&state.busy!=="restore"){
      const token=state.preview.token;state.preview=null;void discard(token).catch(()=>{});
    }
  }
  async function action(element){
    const name=element?.dataset?.action;
    if(!["data-backup-export","data-backup-inspect","data-backup-cancel","data-backup-restore","data-backup-refresh","data-backup-scope-all","data-backup-scope-selected","data-backup-anchor-all","data-backup-anchor-clear","data-backup-anchors-retry"].includes(name))return false;
    if(!connected()||state.busy)return true;
    state.error="";state.notice="";
    const requestGeneration=generation;
    let restored=false;
    try{
      if(name==="data-backup-scope-all"||name==="data-backup-scope-selected"){
        state.scope=name==="data-backup-scope-all"?"all":"selected";render();
        if(state.scope==="selected"&&!state.anchorsLoaded)await readAnchors();
      }else if(name==="data-backup-anchor-all"||name==="data-backup-anchor-clear"){
        const matches=matchingAnchors();
        if(name==="data-backup-anchor-all"&&matches.length>MAX_SELECTED_ANCHORS)throw Error(selectionLimitMessage);
        state.selected=name==="data-backup-anchor-all"?new Set(matches.map(anchor=>anchor.anchor_id)):new Set();renderSelection();
      }else if(name==="data-backup-anchors-retry"){
        await readAnchors();
      }else if(name==="data-backup-export"){
        if(state.scope==="selected"&&(state.anchorsLoading||!state.anchorsLoaded))throw Error("请先读取主播列表");
        if(state.scope==="selected"&&!state.selected.size)throw Error("请至少选择一位主播");
        if(state.scope==="selected"&&state.selected.size>MAX_SELECTED_ANCHORS)throw Error(selectionLimitMessage);
        const eligible=new Set(state.anchors.filter(anchor=>tagMatches(anchor,state.tagFilter)).map(anchor=>anchor.anchor_id));
        const anchorIds=state.scope==="all"?null:[...state.selected].filter(id=>eligible.has(id));
        if(anchorIds&&!anchorIds.length)throw Error("请至少选择一位主播");
        state.busy="export";render();
        const result=await invoke("data_backup_export",{preferences:collectBackupPreferences(storage),anchorIds});
        if(result===null){state.notice="已取消导出";return true;}
        validateBackupSummary(result?.summary??result);
        state.savedPath=String(result?.path||"");state.notice=result?.preview?"预览演示，未写入文件":"备份已导出";
        toast(state.notice);
      }else if(name==="data-backup-inspect"){
        state.busy="inspect";render();
        const result=await invoke("data_backup_inspect",{});
        if(result===null){state.notice="已取消选择";return true;}
        const token=result?.token;
        if(typeof token!=="string"||!token||token.length>256||/[\u0000-\u001f]/.test(token))throw Error("备份预览已失效，请重新选择");
        if(requestGeneration!==generation||!connected()){await discard(token).catch(()=>{});return true;}
        let summary;
        try{summary=validateBackupSummary(result.summary);}catch(error){await discard(token).catch(()=>{});throw error;}
        const previous=state.preview?.token;
        state.preview={token,file_name:String(result.file_name||"数据备份").slice(0,512),summary};
        if(previous&&previous!==token)await discard(previous).catch(()=>{});
      }else if(name==="data-backup-cancel"){
        if(!state.preview)return true;
        state.busy="discard";const token=state.preview.token;state.preview=null;render();
        await discard(token);state.notice="已取消导入";
      }else if(name==="data-backup-restore"){
        if(!state.preview)return true;
        const token=state.preview.token;state.busy="restore";render();
        const result=await invoke("data_backup_restore",{token});
        if(result?.restored!==true)throw Error("导入未完成，请重试");
        restored=true;state.preview=null;
        anchorsRevision++;state.anchorsLoaded=false;state.anchorsLoading=false;state.anchorsError="";
        let preferences={};
        try{preferences=applyBackupPreferences(result.preferences,storage);}
        catch{try{preferences=collectBackupPreferences(storage);}catch{}state.error="数据已导入，界面偏好未合并";toast(state.error,true);}
        state.restoredPreferences=preferences;
        const previous=result.before_import_path||result.rollback_path;
        if(previous)state.savedPath=`导入前备份：${String(previous)}`;
        if(result.startup_error){state.error=[state.error,`数据已导入，监听未能自动恢复：${errorText(result.startup_error)}`].filter(Boolean).join("；");toast(state.error,true);}
        state.notice="数据已增量导入，正在刷新应用…";render();
        await onRestored(preferences);
        state.notice=result.preview?"预览增量导入演示完成":"数据已增量导入";
      }else{
        if(!state.needsRefresh)return true;
        restored=true;state.busy="refresh";render();
        await onRestored(state.restoredPreferences||{});state.needsRefresh=false;
      }
    }catch(error){
      if(restored){state.preview=null;state.needsRefresh=true;state.notice="数据已增量导入";state.error=`数据已导入，刷新未完成：${errorText(error)}`;}
      else{state.error=errorText(error);if(invalidToken(error))state.preview=null;}
    }finally{state.busy="";render();}
    return true;
  }
  return {mount,unmount,action};
}
