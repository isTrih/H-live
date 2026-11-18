import {esc} from "./presentation.js";
import {DEFAULT_AI_BASE_URL} from "./ai-defaults.js";

export function createHighlightSettings({bridge,toast=()=>{}}) {
  let current=null;
  const valid=ctx=>current===ctx&&ctx.root.isConnected;
  function render(ctx) {
    if(!valid(ctx))return;
    ctx.root.innerHTML=`<div class="highlight-settings-heading"><h3>亮点分析模型</h3><span>所有主播共用</span></div>
      <p class="highlight-settings-description">将本场已归档消息、在线与活动信息拼成一次请求，寻找值得回看的互动。</p>
      <div class="highlight-settings-fields"><label>API 地址<input id="highlight-base-url" type="url" autocomplete="off" spellcheck="false" placeholder="https://your-provider.com/v1" value="${esc(ctx.draft.base_url)}" ${ctx.busy||ctx.loading?"disabled":""}></label>
      <label>API Key<input id="highlight-api-key" type="password" autocomplete="off" spellcheck="false" placeholder="${ctx.saved.key_configured?"已配置，留空保留":"输入 API Key"}" value="${esc(ctx.draft.api_key)}" ${ctx.busy||ctx.loading?"disabled":""}></label>
      <label>当前模型<select id="highlight-model" ${ctx.busy||ctx.loading?"disabled":""}><option value="">请先获取模型</option>${[...new Set([ctx.draft.model,...ctx.models.map(m=>m.id)].filter(Boolean))].map(id=>`<option value="${esc(id)}" ${ctx.draft.model===id?"selected":""}>${esc(id)}</option>`).join("")}</select></label></div>
      ${ctx.error?`<p class="highlight-settings-error" role="alert">${esc(ctx.error)}</p>`:""}
      <div class="highlight-settings-actions"><span role="status">${ctx.loading?"正在读取配置…":ctx.busy?"正在连接…":ctx.notice||"支持 OpenAI 兼容接口"}</span><button data-action="highlight-models" ${ctx.busy||ctx.loading?"disabled":""}>获取模型</button><button class="primary" data-action="highlight-save" ${ctx.busy||ctx.loading?"disabled":""}>保存配置</button></div>`;
  }
  async function mount(root=document.querySelector("#highlight-settings")) {
    if(!root)return;
    const ctx=current={root,saved:{key_configured:false},draft:{base_url:DEFAULT_AI_BASE_URL,model:"",api_key:""},models:[],busy:false,loading:true,error:"",notice:""};render(ctx);
    try{const saved=await bridge("highlight_settings");if(!valid(ctx))return;ctx.saved=saved;ctx.draft={...ctx.draft,base_url:saved.base_url?.trim()||DEFAULT_AI_BASE_URL,model:saved.model||""};}
    catch(e){if(valid(ctx))ctx.error=String(e?.message||e);}
    finally{if(valid(ctx)){ctx.loading=false;render(ctx);}}
  }
  function input(target){const ctx=current;if(!ctx||!valid(ctx)||ctx.loading)return;const key={"highlight-base-url":"base_url","highlight-api-key":"api_key","highlight-model":"model"}[target.id];if(key)ctx.draft[key]=target.value;}
  async function action(el){
    const method=el.dataset.action;if(!["highlight-save","highlight-models"].includes(method))return false;
    const ctx=current;if(!ctx||!valid(ctx)||ctx.busy)return true;
    ctx.draft.base_url=ctx.draft.base_url.trim();
    if(!ctx.draft.base_url){ctx.error="请输入 API 地址";render(ctx);return true;}
    if(method==="highlight-save"&&!ctx.draft.model){ctx.error="请获取并选择模型";render(ctx);return true;}
    ctx.busy=true;ctx.error="";ctx.notice="";render(ctx);
    try{
      const settings=await bridge("highlight_configure",{base_url:ctx.draft.base_url,model:ctx.draft.model,...(ctx.draft.api_key?{api_key:ctx.draft.api_key}:{})});
      if(!valid(ctx))return true;
      ctx.saved=settings;ctx.draft.api_key="";
      if(method==="highlight-models"){
        const result=await bridge("highlight_models");if(!valid(ctx))return true;
        ctx.models=result.models||[];if(!ctx.models.length)throw Error("接口未返回可用模型，请检查地址与 Key");
        if(!ctx.models.some(m=>m.id===ctx.draft.model))ctx.draft.model=ctx.models[0].id;
        ctx.notice=`已获取 ${ctx.models.length} 个模型，请选择后保存`;
      }else{ctx.notice="配置已保存";toast("亮点分析模型已保存");}
    }catch(e){if(valid(ctx))ctx.error=String(e?.message||e);}
    finally{if(valid(ctx)){ctx.busy=false;render(ctx);}}
    return true;
  }
  function unmount(){if(current)current.draft.api_key="";current=null;}
  return {mount,input,change:input,action,unmount};
}
