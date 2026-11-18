import {esc,icon} from "./presentation.js";

const fields={"download-root-directory":"root_directory","download-directory-template":"directory_template","download-filename-template":"filename_template"};
const draftOf=value=>({root_directory:String(value?.root_directory||""),directory_template:String(value?.directory_template||""),filename_template:String(value?.filename_template||"")});
const same=(a,b)=>Object.keys(draftOf()).every(key=>a[key]===b[key]);
export function insertDownloadVariable(value,start,end,key) {
  const text=String(value||""),from=Number.isInteger(start)?Math.max(0,Math.min(text.length,start)):text.length,to=Number.isInteger(end)?Math.max(from,Math.min(text.length,end)):from;
  const token=`{${key}}`;
  return {value:text.slice(0,from)+token+text.slice(to),cursor:from+token.length};
}

export function createDownloadSettings({bridge,invoke,toast=()=>{}}) {
  let current=null;
  const valid=ctx=>current===ctx&&ctx.root.isConnected!==false;
  const query=(ctx,selector)=>ctx.root.querySelector?.(selector);
  const updateText=(ctx,id,text)=>{const node=query(ctx,`#${id}`);if(node)node.textContent=text;};
  function shell(ctx) {
    return `<div class="download-settings-heading"><h3>录像下载</h3><span>目录与命名</span></div>
      <div class="download-root-field"><label for="download-root-directory">下载位置</label><div><input id="download-root-directory" type="text" autocomplete="off" spellcheck="false" value="${esc(ctx.draft.root_directory)}"><button data-action="download-choose-directory">${icon("folder")}选择目录</button></div></div>
      <div class="download-rule-fields"><label>文件夹规则<input id="download-directory-template" type="text" autocomplete="off" spellcheck="false" placeholder="{date}/{anchor}" value="${esc(ctx.draft.directory_template)}"><small>留空时直接保存到下载位置；用 / 分层。</small></label><label>文件名规则<input id="download-filename-template" type="text" autocomplete="off" spellcheck="false" placeholder="{title}_{start}-{end}.mp4" value="${esc(ctx.draft.filename_template)}"></label></div>
      <details class="download-variable-list"><summary>插入变量</summary><div id="download-variables"></div></details>
      <div class="download-path-preview"><span>路径预览</span><output id="download-path-preview" class="selectable"></output><p id="download-preview-error" role="alert"></p></div>
      <p id="download-settings-error" class="download-settings-error" role="alert"></p>
      <div class="download-settings-actions"><span id="download-settings-status" role="status"></span><div><button data-action="download-settings-reload" class="small quiet">重新读取</button><button data-action="download-settings-save" class="primary">保存规则</button></div></div>`;
  }
  function controls(ctx) {
    if(!valid(ctx))return;
    const disabled=ctx.loading||ctx.busy||ctx.loadFailed;
    for(const id of Object.keys(fields)){const input=query(ctx,`#${id}`);if(input)input.disabled=disabled;}
    for(const action of ["download-settings-save","download-choose-directory"]){const button=query(ctx,`[data-action="${action}"]`);if(button)button.disabled=disabled;}
    const reload=query(ctx,'[data-action="download-settings-reload"]');if(reload)reload.disabled=ctx.loading||ctx.busy;
    for(const button of ctx.root.querySelectorAll?.('[data-action="download-variable"]')||[])button.disabled=disabled;
    updateText(ctx,"download-settings-status",ctx.loading?"正在读取…":ctx.busy?"正在保存…":ctx.notice||(!same(ctx.draft,ctx.saved)?"尚未保存":""));
    updateText(ctx,"download-settings-error",ctx.error);
    updateText(ctx,"download-preview-error",ctx.previewError);
  }
  function writeDraft(ctx) {for(const [id,key]of Object.entries(fields)){const input=query(ctx,`#${id}`);if(input)input.value=ctx.draft[key];}}
  function setVariables(ctx,variables=[]) {
    ctx.variables=variables.filter(item=>/^[a-z][a-z0-9_]*$/.test(item?.key));
    const node=query(ctx,"#download-variables");
    if(node)node.innerHTML=ctx.variables.map(item=>`<button type="button" data-action="download-variable" data-variable="${esc(item.key)}" title="${esc(`${item.label||item.key} · ${item.example||""}`)}"><span>${esc(item.label||item.key)}</span><code>{${esc(item.key)}}</code></button>`).join("");
  }
  async function preview(ctx) {
    if(!valid(ctx)||ctx.loading||ctx.loadFailed)return;
    const revision=++ctx.previewRevision,payload={...ctx.draft};
    updateText(ctx,"download-path-preview","正在预览…");
    try {
      const result=await bridge("download_naming_preview",payload);
      if(!valid(ctx)||revision!==ctx.previewRevision)return;
      ctx.previewError="";ctx.preview=result;
      updateText(ctx,"download-path-preview",result?.output_path||result?.relative_path||"—");
    } catch(error) {
      if(!valid(ctx)||revision!==ctx.previewRevision)return;
      ctx.previewError=String(error?.message||error);
      updateText(ctx,"download-path-preview","—");
    }
    if(valid(ctx))controls(ctx);
  }
  function schedulePreview(ctx) {clearTimeout(ctx.previewTimer);ctx.previewRevision++;ctx.previewError="";updateText(ctx,"download-path-preview","正在预览…");ctx.previewTimer=setTimeout(()=>void preview(ctx),280);ctx.previewTimer?.unref?.();}
  async function read(ctx,{keepDraft=false}={}) {
    ctx.loading=true;ctx.error="";controls(ctx);
    try {
      const response=await bridge("download_naming_settings",{});
      if(!valid(ctx))return;
      ctx.saved=draftOf(response?.settings);ctx.defaults=response?.defaults;
      if(!keepDraft){ctx.draft={...ctx.saved};writeDraft(ctx);}
      setVariables(ctx,response?.variables);
      ctx.loadFailed=false;
      if(!keepDraft&&response?.preview){ctx.preview=response.preview;updateText(ctx,"download-path-preview",response.preview.output_path||response.preview.relative_path||"—");}
      return true;
    } catch(error) {
      if(valid(ctx)){ctx.loadFailed=!ctx.saved.root_directory;ctx.error=String(error?.message||error);}
      return false;
    } finally {if(valid(ctx)){ctx.loading=false;controls(ctx);}}
  }
  async function mount(root=document.querySelector("#download-settings")) {
    unmount();if(!root)return;
    const ctx=current={root,saved:draftOf(),draft:draftOf(),variables:[],loading:true,busy:false,loadFailed:false,error:"",previewError:"",notice:"",previewRevision:0,previewTimer:null,lastField:"download-filename-template"};
    root.innerHTML=shell(ctx);
    ctx.focus=event=>{if(["download-directory-template","download-filename-template"].includes(event.target?.id))ctx.lastField=event.target.id;};
    ctx.pointer=event=>{if(event.target?.closest?.('[data-action="download-variable"]'))event.preventDefault();};
    root.addEventListener?.("focusin",ctx.focus);root.addEventListener?.("pointerdown",ctx.pointer);
    await read(ctx);
  }
  function input(target) {
    const ctx=current,key=fields[target?.id];
    if(!key||!ctx||!valid(ctx)||ctx.loading||ctx.busy||ctx.loadFailed)return false;
    ctx.draft[key]=target.value;ctx.notice="";ctx.error="";
    if(key!=="root_directory")ctx.lastField=target.id;
    schedulePreview(ctx);controls(ctx);return true;
  }
  async function action(element) {
    const name=element?.dataset?.action;
    if(!["download-settings-save","download-settings-reload","download-choose-directory","download-variable"].includes(name))return false;
    const ctx=current;if(!ctx||!valid(ctx)||ctx.loading||ctx.busy)return true;
    if(name==="download-settings-reload"){
      // Re-read authoritative settings without discarding an in-progress naming rule.
      const dirty=!same(ctx.draft,ctx.saved),loaded=await read(ctx,{keepDraft:dirty});
      if(valid(ctx)&&dirty&&loaded){ctx.notice="已读取，保留未保存规则";controls(ctx);await preview(ctx);}
      return true;
    }
    if(ctx.loadFailed)return true;
    if(name==="download-variable"){
      const key=element.dataset.variable;if(!ctx.variables.some(item=>item.key===key))return true;
      const field=query(ctx,`#${ctx.lastField}`),draftKey=fields[ctx.lastField];
      const inserted=insertDownloadVariable(ctx.draft[draftKey],field?.selectionStart,field?.selectionEnd,key);
      ctx.draft[draftKey]=inserted.value;if(field){field.value=inserted.value;field.focus?.();field.setSelectionRange?.(inserted.cursor,inserted.cursor);}
      ctx.notice="";schedulePreview(ctx);controls(ctx);return true;
    }
    if(name==="download-choose-directory"){
      try {const result=await invoke("choose_download_directory",{});if(!valid(ctx))return true;const path=typeof result==="string"?result:result?.path;if(path){ctx.draft.root_directory=path;writeDraft(ctx);ctx.notice="";controls(ctx);await preview(ctx);}}
      catch(error){if(valid(ctx)){ctx.error=String(error?.message||error);controls(ctx);}}
      return true;
    }
    if(!ctx.draft.filename_template.trim()){ctx.error="请输入文件名规则";controls(ctx);query(ctx,"#download-filename-template")?.focus?.();return true;}
    clearTimeout(ctx.previewTimer);ctx.previewRevision++;ctx.busy=true;ctx.error="";ctx.notice="";controls(ctx);
    try {
      await bridge("download_naming_save",{...ctx.draft});
      // Only a successful read-back replaces the draft with the persisted rules.
      const response=await bridge("download_naming_settings",{});
      if(!valid(ctx))return true;
      ctx.saved=draftOf(response?.settings);ctx.draft={...ctx.saved};writeDraft(ctx);setVariables(ctx,response?.variables);
      ctx.previewError="";ctx.preview=response?.preview;
      updateText(ctx,"download-path-preview",response?.preview?.output_path||response?.preview?.relative_path||"—");
      ctx.notice="规则已保存";toast("下载规则已保存");
    } catch(error) {if(valid(ctx))ctx.error=String(error?.message||error);}
    finally {if(valid(ctx)){ctx.busy=false;controls(ctx);}}
    return true;
  }
  function unmount() {
    if(current){clearTimeout(current.previewTimer);current.root.removeEventListener?.("focusin",current.focus);current.root.removeEventListener?.("pointerdown",current.pointer);}
    current=null;
  }
  return {mount,unmount,input,change:input,action};
}
