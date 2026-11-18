import {esc} from "./presentation.js";
import {DEFAULT_DISPLAY_TIMEZONE,formatWorldClock,normalizeDisplayTimezone,setDisplayTimezone} from "./time-display.js";

const commonZones=[
  ["Asia/Shanghai","中国标准时间"],["Asia/Hong_Kong","香港"],["Asia/Singapore","新加坡"],["Asia/Tokyo","东京"],
  ["UTC","协调世界时"],["Europe/London","伦敦"],["Europe/Paris","巴黎"],["America/New_York","纽约"],["America/Los_Angeles","洛杉矶"],["Australia/Sydney","悉尼"],
];
export function createTimeDisplaySettings({bridge,toast=()=>{},onSaved=()=>{}}) {
  let current=null;
  const valid=ctx=>current===ctx&&ctx.root.isConnected!==false;
  const node=(ctx,id)=>ctx.root.querySelector?.(`#${id}`);
  const text=(ctx,id,value)=>{const target=node(ctx,id);if(target)target.textContent=value;};
  function update(ctx) {
    if(!valid(ctx))return;
    const disabled=ctx.loading||ctx.busy||ctx.loadFailed;
    const input=node(ctx,"time-display-timezone");if(input){input.disabled=disabled;input.setAttribute?.("aria-invalid",ctx.invalid?"true":"false");}
    const save=ctx.root.querySelector?.('[data-action="time-display-save"]');if(save)save.disabled=disabled;
    const reload=ctx.root.querySelector?.('[data-action="time-display-reload"]');if(reload)reload.disabled=ctx.loading||ctx.busy;
    text(ctx,"time-display-error",ctx.error);
    text(ctx,"time-display-status",ctx.loading?"正在读取…":ctx.busy?"正在保存…":ctx.notice||(ctx.draft!==ctx.saved?"尚未保存":""));
    text(ctx,"time-display-preview",formatWorldClock(Date.now()/1000,ctx.draft,{date:true}));
  }
  function shell(ctx) {
    return `<div class="time-display-heading"><h3>回看时间</h3></div><label class="time-display-field" for="time-display-timezone">显示时区<input id="time-display-timezone" type="text" list="time-display-timezones" value="${esc(ctx.draft)}" autocomplete="off" spellcheck="false" placeholder="Asia/Shanghai" aria-describedby="time-display-scope"><datalist id="time-display-timezones">${commonZones.map(([value,label])=>`<option value="${esc(value)}">${esc(label)}</option>`).join("")}</datalist></label>
      <p id="time-display-scope" class="time-display-scope">用于回看时间轴与播放时间。</p><div class="time-display-preview"><span>当前时间</span><output id="time-display-preview"></output></div>
      <p id="time-display-error" class="time-display-error" role="alert"></p><div class="time-display-actions"><span id="time-display-status" role="status"></span><div><button class="small quiet" data-action="time-display-reload">重新读取</button><button class="primary" data-action="time-display-save">保存时区</button></div></div>`;
  }
  async function read(ctx,{keepDraft=false}={}) {
    ctx.loading=true;ctx.error="";ctx.invalid=false;ctx.notice="";update(ctx);
    try {
      const result=await bridge("time_display_settings",{}),timezone=normalizeDisplayTimezone(result?.timezone);
      if(!valid(ctx))return false;
      ctx.saved=timezone;setDisplayTimezone(timezone);
      if(!keepDraft){ctx.draft=timezone;const input=node(ctx,"time-display-timezone");if(input)input.value=timezone;}
      ctx.loadFailed=false;return true;
    } catch(error) {if(valid(ctx)){ctx.loadFailed=!ctx.loaded;ctx.error=String(error?.message||error);}return false;}
    finally {if(valid(ctx)){ctx.loading=false;ctx.loaded=!ctx.loadFailed;update(ctx);}}
  }
  async function mount(root=document.querySelector("#time-display-settings")) {
    unmount();if(!root)return;
    const ctx=current={root,saved:DEFAULT_DISPLAY_TIMEZONE,draft:DEFAULT_DISPLAY_TIMEZONE,loaded:false,loading:true,busy:false,loadFailed:false,invalid:false,error:"",notice:""};
    root.innerHTML=shell(ctx);await read(ctx);
  }
  function input(target) {
    const ctx=current;if(target?.id!=="time-display-timezone"||!ctx||!valid(ctx)||ctx.loading||ctx.busy||ctx.loadFailed)return false;
    ctx.draft=target.value;ctx.error="";ctx.notice="";ctx.invalid=false;update(ctx);return true;
  }
  async function action(element) {
    const name=element?.dataset?.action;if(!["time-display-save","time-display-reload"].includes(name))return false;
    const ctx=current;if(!ctx||!valid(ctx)||ctx.loading||ctx.busy)return true;
    if(name==="time-display-reload"){
      const dirty=ctx.draft!==ctx.saved,loaded=await read(ctx,{keepDraft:dirty});
      if(valid(ctx)&&dirty&&loaded){ctx.notice="已读取，保留未保存时区";update(ctx);}return true;
    }
    if(ctx.loadFailed)return true;
    let timezone;
    try{timezone=normalizeDisplayTimezone(ctx.draft);}catch(error){ctx.error=String(error?.message||error);ctx.invalid=true;update(ctx);node(ctx,"time-display-timezone")?.focus?.();return true;}
    ctx.busy=true;ctx.error="";ctx.invalid=false;ctx.notice="";update(ctx);
    try {
      await bridge("time_display_save",{timezone});
      const result=await bridge("time_display_settings",{}),saved=normalizeDisplayTimezone(result?.timezone);
      setDisplayTimezone(saved);try{await onSaved(saved);}catch{/* Persisted preferences remain authoritative if a view refresh fails. */}
      if(!valid(ctx))return true;
      ctx.saved=saved;ctx.draft=saved;const input=node(ctx,"time-display-timezone");if(input)input.value=saved;
      ctx.notice="时区已保存";toast("回看时区已保存");
    } catch(error) {if(valid(ctx))ctx.error=String(error?.message||error);}
    finally {if(valid(ctx)){ctx.busy=false;update(ctx);}}
    return true;
  }
  function unmount(){current=null;}
  return {mount,unmount,input,change:input,action};
}
