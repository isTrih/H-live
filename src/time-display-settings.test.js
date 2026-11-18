import test from "node:test";
import assert from "node:assert/strict";
import {createTimeDisplaySettings} from "./time-display-settings.js";
import {DEFAULT_DISPLAY_TIMEZONE,getDisplayTimezone,setDisplayTimezone} from "./time-display.js";

function fixture(handler,onSaved=()=>{}) {
  const nodes=new Map(["time-display-timezone","time-display-error","time-display-status","time-display-preview"].map(id=>[`#${id}`,{id,value:"",textContent:"",disabled:false,setAttribute(key,value){this[key]=value;},focus(){this.focused=true;}}]));
  const root={isConnected:true,innerHTML:"",querySelector:selector=>nodes.get(selector)},calls=[],notices=[];
  const panel=createTimeDisplaySettings({bridge:async(method,params)=>{calls.push([method,params]);return handler(method,params);},onSaved,toast:text=>notices.push(text)});
  return {root,panel,calls,notices,field:id=>nodes.get(`#${id}`)};
}
const action=name=>({dataset:{action:name}});

test("timezone settings read the stored preference and save only after a read-back",async t=>{
  let saved="Asia/Shanghai";const updates=[];
  const f=fixture(async(method,params)=>{if(method==="time_display_save")saved=params.timezone;return {timezone:saved};},timezone=>updates.push(timezone));
  t.after(()=>{f.panel.unmount();setDisplayTimezone(DEFAULT_DISPLAY_TIMEZONE);});await f.panel.mount(f.root);
  const input=f.field("time-display-timezone");input.value=" UTC ";f.panel.input(input);
  assert.equal(getDisplayTimezone(),"Asia/Shanghai");
  await f.panel.action(action("time-display-save"));
  assert.deepEqual(f.calls.map(([method])=>method),["time_display_settings","time_display_save","time_display_settings"]);
  assert.deepEqual(updates,["UTC"]);assert.equal(input.value,"UTC");assert.equal(getDisplayTimezone(),"UTC");assert.equal(f.notices.length,1);
});

test("invalid IANA input stays editable, focuses the field and never reaches the save endpoint",async t=>{
  const f=fixture(async()=>({timezone:"Asia/Shanghai"}));t.after(()=>f.panel.unmount());await f.panel.mount(f.root);
  const input=f.field("time-display-timezone");input.value="Wrong/Somewhere";f.panel.input(input);await f.panel.action(action("time-display-save"));
  assert.equal(input.value,"Wrong/Somewhere");assert.equal(input["aria-invalid"],"true");assert.equal(input.focused,true);assert.equal(f.calls.length,1);assert.equal(f.notices.length,0);
});

test("failed save preserves the draft and does not change cached display timezone",async t=>{
  const f=fixture(async method=>{if(method==="time_display_save")throw Error("存储不可写");return {timezone:"Asia/Shanghai"};});
  t.after(()=>f.panel.unmount());await f.panel.mount(f.root);
  const input=f.field("time-display-timezone");input.value="Europe/London";f.panel.change(input);await f.panel.action(action("time-display-save"));
  assert.equal(input.value,"Europe/London");assert.equal(getDisplayTimezone(),"Asia/Shanghai");assert.match(f.field("time-display-error").textContent,/存储不可写/);assert.equal(f.notices.length,0);
});

test("reload preserves an unsaved timezone and a failed read-back never announces success",async t=>{
  let reads=0,failReadback=false;
  const f=fixture(async method=>{if(method==="time_display_settings"){reads++;if(failReadback)throw Error("读取失败");}return {timezone:"Asia/Shanghai"};});
  t.after(()=>f.panel.unmount());await f.panel.mount(f.root);
  const input=f.field("time-display-timezone");input.value="Asia/Tokyo";f.panel.input(input);await f.panel.action(action("time-display-reload"));
  assert.equal(input.value,"Asia/Tokyo");assert.match(f.field("time-display-status").textContent,/保留未保存/);
  failReadback=true;await f.panel.action(action("time-display-save"));
  assert.equal(reads,3);assert.equal(input.value,"Asia/Tokyo");assert.equal(f.notices.length,0);assert.match(f.field("time-display-error").textContent,/读取失败/);
});

test("closing the settings page ignores an old read and does not replace another page",async()=>{
  let finish;const f=fixture(()=>new Promise(resolve=>{finish=resolve;}));
  const mounting=f.panel.mount(f.root);f.panel.unmount();f.root.innerHTML="另一个页面";finish({timezone:"UTC"});await mounting;
  assert.equal(f.root.innerHTML,"另一个页面");assert.equal(getDisplayTimezone(),"Asia/Shanghai");
});
