import test from "node:test";
import assert from "node:assert/strict";
import {createHighlightSettings} from "./highlight-settings.js";
import {DEFAULT_AI_BASE_URL} from "./ai-defaults.js";

const flush=()=>new Promise(resolve=>setImmediate(resolve));
function fixture(bridge){const root={innerHTML:"",isConnected:true};const notices=[];const controller=createHighlightSettings({bridge,toast:value=>notices.push(value)});return {root,controller,notices};}
const input=(controller,id,value)=>controller.input({id,value});
test("new and blank AI settings prefill Lilith proxy without starting a request",async()=>{
 for(const saved of [{},{base_url:""},{base_url:"   "}]){
  const calls=[];const f=fixture(async method=>{calls.push(method);return saved;});await f.controller.mount(f.root);
  assert.match(f.root.innerHTML,new RegExp(DEFAULT_AI_BASE_URL.replace(/[.*+?^${}()|[\]\\]/g,"\\$&")));assert.deepEqual(calls,["highlight_settings"]);
 }
});
test("settings loads saved model without returning or persisting a key in the form",async()=>{
  const f=fixture(async()=>({base_url:"https://models.example/v1",model:"chat-a",key_configured:true}));
  await f.controller.mount(f.root);assert.match(f.root.innerHTML,/chat-a/);assert.match(f.root.innerHTML,/已配置，留空保留/);
  assert.match(f.root.innerHTML,/type="password"/);assert.doesNotMatch(f.root.innerHTML,/localStorage/);
});
test("settings fields stay disabled until the saved configuration has loaded",async()=>{
 let finish;const f=fixture(()=>new Promise(resolve=>finish=resolve));const mounting=f.controller.mount(f.root);
 assert.match(f.root.innerHTML,/id="highlight-base-url"[^>]*disabled/);assert.match(f.root.innerHTML,/id="highlight-api-key"[^>]*disabled/);assert.match(f.root.innerHTML,/id="highlight-model" disabled/);
 input(f.controller,"highlight-base-url","https://typed.invalid/v1");finish({base_url:"https://saved.example/v1",model:"chat",key_configured:true});await mounting;assert.match(f.root.innerHTML,/https:\/\/saved.example/);assert.doesNotMatch(f.root.innerHTML,/typed.invalid/);
});
test("model discovery saves provider first, keeps an omitted key and preserves chosen model",async()=>{
  const calls=[];const f=fixture(async(method,p)=>{calls.push([method,p]);if(method==="highlight_models")return {models:[{id:'chat-<b>'},{id:"chat-b"}]};return {base_url:p?.base_url||"",model:p?.model||"",key_configured:true};});
  await f.controller.mount(f.root);input(f.controller,"highlight-base-url","https://models.example/v1");input(f.controller,"highlight-api-key","test-secret");
  await f.controller.action({dataset:{action:"highlight-models"}});
  assert.deepEqual(calls.slice(-2).map(c=>c[0]),["highlight_configure","highlight_models"]);assert.equal(calls[1][1].api_key,"test-secret");assert.doesNotMatch(f.root.innerHTML,/test-secret/);assert.match(f.root.innerHTML,/chat-&lt;b&gt;/);
  input(f.controller,"highlight-model","chat-b");await f.controller.action({dataset:{action:"highlight-save"}});
  assert.equal(calls.at(-1)[1].model,"chat-b");assert.equal(Object.hasOwn(calls.at(-1)[1],"api_key"),false);assert.equal(f.notices.length,1);
});
test("failed provider save retains editable draft and late responses cannot repaint unmounted settings",async()=>{
  let finish;const f=fixture(async method=>{if(method==="highlight_settings")return {base_url:"",model:"",key_configured:false};return new Promise(resolve=>finish=resolve);});
  await f.controller.mount(f.root);input(f.controller,"highlight-base-url","https://models.example/v1");input(f.controller,"highlight-api-key","draft-key");input(f.controller,"highlight-model","chat");
  const saving=f.controller.action({dataset:{action:"highlight-save"}});await flush();f.controller.unmount();f.root.innerHTML="closed";finish({key_configured:true});await saving;assert.equal(f.root.innerHTML,"closed");
  const failed=fixture(async method=>{if(method==="highlight_configure")throw Error("保存失败");return {base_url:"",model:"",key_configured:false};});
  await failed.controller.mount(failed.root);input(failed.controller,"highlight-base-url","https://models.example/v1");input(failed.controller,"highlight-api-key","draft-key");input(failed.controller,"highlight-model","chat");await failed.controller.action({dataset:{action:"highlight-save"}});assert.match(failed.root.innerHTML,/draft-key/);assert.match(failed.root.innerHTML,/保存失败/);
});
