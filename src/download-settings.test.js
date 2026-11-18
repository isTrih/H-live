import test from "node:test";
import assert from "node:assert/strict";
import {createDownloadSettings,insertDownloadVariable} from "./download-settings.js";

const initial={root_directory:"D:/直播",directory_template:"{date}/{anchor}",filename_template:"{title}_{start}.mp4"};
const response=settings=>({settings,defaults:initial,variables:[{key:"title",label:"标题",example:"观众追问"},{key:"date",label:"日期",example:"2026-10-25"}],preview:{output_path:`${settings.root_directory}/26/追问.mp4`,relative_path:"26/追问.mp4"}});
function fixture(handler,invoke=async()=>null) {
  const ids=["download-root-directory","download-directory-template","download-filename-template","download-settings-status","download-settings-error","download-preview-error","download-path-preview","download-variables"],nodes=new Map(ids.map(id=>[`#${id}`,{id,value:"",textContent:"",innerHTML:"",disabled:false,selectionStart:0,selectionEnd:0,focus(){this.focused=true;},setSelectionRange(start,end){this.selectionStart=start;this.selectionEnd=end;}}])),calls=[],notices=[];
  const root={innerHTML:"",isConnected:true,querySelector:selector=>nodes.get(selector),querySelectorAll:()=>[],addEventListener(){},removeEventListener(){}};
  const panel=createDownloadSettings({bridge:async(method,params)=>{calls.push([method,params]);return handler(method,params);},invoke,toast:message=>notices.push(message)});
  return {panel,root,nodes,calls,notices,field:id=>nodes.get(`#${id}`)};
}
const action=(name,extra={})=>({dataset:{action:name,...extra}});

test("variables replace only the selected portion of a naming template",()=>{
  assert.deepEqual(insertDownloadVariable("前缀_旧标题.mp4",3,6,"title"),{value:"前缀_{title}.mp4",cursor:10});
  assert.deepEqual(insertDownloadVariable("{date}/",undefined,undefined,"anchor"),{value:"{date}/{anchor}",cursor:15});
});

test("settings save is confirmed by a second authoritative read before reporting success",async t=>{
  let saved={...initial};
  const f=fixture(async(method,params)=>{if(method==="download_naming_settings")return response(saved);if(method==="download_naming_save"){saved={...params,filename_template:params.filename_template+".mp4"};return response(saved);}return {output_path:"D:/预览.mp4"};});
  t.after(()=>f.panel.unmount());await f.panel.mount(f.root);
  const field=f.field("download-filename-template");field.value="{title}_{start}-{end}";f.panel.input(field);
  await f.panel.action(action("download-settings-save"));
  assert.equal(field.value,"{title}_{start}-{end}.mp4");
  assert.deepEqual(f.calls.filter(([method])=>method!=="download_naming_preview").map(([method])=>method),["download_naming_settings","download_naming_save","download_naming_settings"]);
  assert.equal(f.notices.length,1);assert.equal(f.field("download-settings-status").textContent,"规则已保存");
});

test("failed saves preserve the draft and never report a saved rule",async t=>{
  const f=fixture(async method=>{if(method==="download_naming_settings")return response(initial);throw Error("磁盘不可写");});
  t.after(()=>f.panel.unmount());await f.panel.mount(f.root);
  const field=f.field("download-directory-template");field.value="{date}/精选";f.panel.input(field);
  await f.panel.action(action("download-settings-save"));
  assert.equal(field.value,"{date}/精选");assert.equal(f.notices.length,0);assert.match(f.field("download-settings-error").textContent,/磁盘不可写/);
});

test("re-read never overwrites an unsaved rule; variable insertion preserves the target field",async t=>{
  const f=fixture(async method=>method==="download_naming_settings"?response(initial):{output_path:"D:/预览.mp4"});
  t.after(()=>f.panel.unmount());await f.panel.mount(f.root);
  const field=f.field("download-directory-template");field.value="精选/";field.selectionStart=3;field.selectionEnd=3;f.panel.input(field);
  await f.panel.action(action("download-variable",{variable:"date"}));
  assert.equal(field.value,"精选/{date}");assert.equal(field.selectionStart,9);
  await f.panel.action(action("download-settings-reload"));
  assert.equal(field.value,"精选/{date}");assert.match(f.field("download-settings-status").textContent,/保留未保存/);
});

test("directory selection previews the real backend path without saving and ignores a stale preview",async t=>{
  let finishPreview;
  const f=fixture(async method=>method==="download_naming_settings"?response(initial):new Promise(resolve=>{finishPreview=resolve;}),async method=>{assert.equal(method,"choose_download_directory");return "E:/视频";});
  t.after(()=>f.panel.unmount());await f.panel.mount(f.root);
  const picking=f.panel.action(action("download-choose-directory"));await Promise.resolve();await Promise.resolve();
  assert.equal(f.field("download-root-directory").value,"E:/视频");
  const file=f.field("download-filename-template");file.value="更新后的规则.mp4";f.panel.input(file);
  finishPreview({output_path:"E:/过期.mp4"});await picking;
  assert.notEqual(f.field("download-path-preview").textContent,"E:/过期.mp4");
  assert.equal(file.value,"更新后的规则.mp4");assert.ok(!f.calls.some(([method])=>method==="download_naming_save"));
});

test("unmount ignores a settings response for a page that has closed",async()=>{
  let finish;const f=fixture(()=>new Promise(resolve=>{finish=resolve;}));
  const mounting=f.panel.mount(f.root);f.panel.unmount();f.root.innerHTML="其他页面";finish(response(initial));await mounting;
  assert.equal(f.root.innerHTML,"其他页面");
});
