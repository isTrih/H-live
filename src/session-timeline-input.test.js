import test from "node:test";
import assert from "node:assert/strict";
import {createSessionTimeline} from "./session-timeline.js";
import {createBookmarkDocument} from "./test-support/bookmark-dom.js";

const flush=()=>new Promise(resolve=>setImmediate(resolve));
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};};
const anchor={rid:"111",anchor_id:"11",session_id:"21",name:"主播",state:"connected",active:true,is_live:true};
const archive=duration=>({anchor_id:"11",session_id:"21",start_epoch:1792735200,duration_seconds:duration,bucket_seconds:60,buckets:[{offset_seconds:60,count:1}],messages:[{id:"m",message_type:"chat",time_source:"server",offset_seconds:65,content:"观众追问"}],chat_count:1,total_messages:1,activities:[]});
const action=(name,mark)=>({dataset:{action:name,...(mark?{mark}:{})}});
async function fixture(t){
  const previous=globalThis.document,document=createBookmarkDocument();globalThis.document=document;
  const gates={},calls=[],marks=Array.from({length:35},(_,index)=>({id:`mark-${index}`,anchor_id:"11",session_id:"21",marked_at_epoch:1792735210+index*5,title:`精彩片段 ${index}`,note:`已有备注 ${index}`}));
  const timeline=createSessionTimeline({state:{modal:{kind:"detail",rid:"111"}},createMedia:({element})=>({element,hasMedia:false,isFullscreen:false,currentTime:0,duration:0,setSource:async()=>{},speed(){},muted(){},pause(){},play:async()=>{},destroy(){}}),bridge:async(method,params)=>{
    calls.push([method,params]);if(gates[method]){const pending=gates[method];delete gates[method];return pending.promise;}
    if(method==="session_timeline")return {...archive(300),session_id:params.session_id};
    if(method==="mark_list"){const selected=marks.filter(mark=>mark.session_id===params.session_id&&mark.anchor_id===params.anchor_id);return {marks:structuredClone(selected),total:selected.length};}
    if(method==="session_replay")return {enabled:false,available:false};
    if(method==="mark_create"){const mark={id:`created-${calls.length}`,anchor_id:params.anchor_id,session_id:params.session_id,marked_at_epoch:1792735400,title:params.title||"直播打点",note:""};marks.push(mark);return {mark,created:true};}
    if(method==="mark_update"){const mark=marks.find(mark=>mark.id===params.id);Object.assign(mark,params);return {mark:{...mark}};}
    if(method==="mark_delete"){const index=marks.findIndex(mark=>mark.id===params.id);if(index>=0)marks.splice(index,1);return {deleted:true};}
    return {};
  }});
  document.addEventListener("input",event=>timeline.input(event.target));
  t.after(()=>{timeline.unmount();globalThis.document=previous;});
  timeline.mount(anchor);await flush();await flush();
  return {timeline,document,gates,calls,marks,root:document.querySelector("#session-timeline"),modal:document.querySelector(".modal-body")};
}
function beginTyping(f,selector,value,{composing=false,dispatchInput=true}={}){
  const input=f.root.querySelector(selector);assert.ok(input,`${selector} exists in the actual rendered timeline`);input.focus();
  if(composing)input.dispatchEvent({type:"compositionstart",bubbles:true,data:"精"});
  input.value=value;input.setSelectionRange(2,4,"forward");input.scrollTop=17;
  if(dispatchInput)input.dispatchEvent({type:"input",bubbles:true,isComposing:composing,data:composing?"精彩":null,inputType:composing?"insertCompositionText":"insertText"});
  f.modal.scrollTop=410;const list=f.root.querySelector(".bookmark-list");assert.ok(list);list.scrollTop=215;
  return {input,list,value,detachments:input.detachments,composing};
}
function assertUninterrupted(f,snapshot){
  assert.equal(f.root.querySelector(`#${snapshot.input.id}`)===snapshot.input,true,"the live input node must not be replaced");
  assert.equal(snapshot.input.isConnected,true,"the input must stay in the document");
  assert.equal(snapshot.input.detachments,snapshot.detachments,"the input must never detach, even if reinserted later");
  assert.equal(f.document.activeElement,snapshot.input,"focus remains on the same node");assert.equal(snapshot.input.value,snapshot.value);
  assert.deepEqual([snapshot.input.selectionStart,snapshot.input.selectionEnd,snapshot.input.selectionDirection],[2,4,"forward"]);
  if(snapshot.composing){assert.equal(snapshot.input.composing,true,"Chinese IME composition must remain active");assert.notEqual(snapshot.input.compositionCancelled,true);}
  assert.equal(f.modal.scrollTop,410,"modal-body scroll position is preserved");
  assert.equal(f.root.querySelector(".bookmark-list")===snapshot.list,true,"bookmark list stays mounted");assert.equal(snapshot.list.scrollTop,215,"bookmark-list scroll position is preserved");
  assert.equal(snapshot.input.scrollTop,17,"textarea internal scroll is preserved");
}

test("timeline archive refresh keeps a composing live-bookmark title connected and preserves both scroll positions",async t=>{
  const f=await fixture(t),snapshot=beginTyping(f,"#bookmark-title","精彩互动还在输入",{composing:true});
  const next=deferred();f.gates.session_timeline=next;const reading=f.timeline.action(action("timeline-refresh"));
  await flush();next.resolve(archive(310));await reading;
  assert.ok(f.calls.filter(([method])=>method==="session_timeline").length>=2);assertUninterrupted(f,snapshot);
});

test("mark-list refresh keeps the actual edit-note textarea and its Chinese composition intact",async t=>{
  const f=await fixture(t);await f.timeline.action(action("timeline-mark-edit","mark-0"));
  const snapshot=beginTyping(f,"#bookmark-edit-note","第一行\n观众连问的精彩片段",{composing:true}),next=deferred();f.gates.mark_list=next;
  const reading=f.timeline.refreshBookmarks();await flush();next.resolve({marks:[...f.marks,{...f.marks[0],id:"new-mark",title:"刚补充的打点"}],total:36});await reading;
  assertUninterrupted(f,snapshot);
});

test("an earlier save response preserves a newer composing edit draft instead of remounting it",async t=>{
  const f=await fixture(t);await f.timeline.action(action("timeline-mark-edit","mark-0"));
  const submitted=f.root.querySelector("#bookmark-edit-note");submitted.value="已经提交的备注";submitted.dispatchEvent({type:"input",bubbles:true});
  const pending=deferred();f.gates.mark_update=pending;const saving=f.timeline.action(action("timeline-mark-save"));await flush();
  const snapshot=beginTyping(f,"#bookmark-edit-note","上一版保存期间继续输入的新备注",{composing:true});
  pending.resolve({mark:{...f.marks[0],note:"已经提交的备注"}});await saving;
  assertUninterrupted(f,snapshot);
});

test("a provisional IME value before its input event is never overwritten by a background refresh",async t=>{
  const f=await fixture(t);await f.timeline.action(action("timeline-mark-edit","mark-0"));
  const snapshot=beginTyping(f,"#bookmark-edit-note","正在组合但input尚未触发",{composing:true,dispatchInput:false});
  await f.timeline.refreshBookmarks();assertUninterrupted(f,snapshot);
});

test("save completion cannot close an editor after a new composition starts before its input event",async t=>{
  const f=await fixture(t);await f.timeline.action(action("timeline-mark-edit","mark-0"));
  const pending=deferred();f.gates.mark_update=pending;const saving=f.timeline.action(action("timeline-mark-save"));await flush();
  const snapshot=beginTyping(f,"#bookmark-edit-note","仍在组合的下一版备注",{composing:true,dispatchInput:false});
  pending.resolve({mark:{...f.marks[0]}});await saving;assertUninterrupted(f,snapshot);
});

test("adding and clearing a bookmark load error never moves the list containing an active composition",async t=>{
  const f=await fixture(t);await f.timeline.action(action("timeline-mark-edit","mark-0"));
  const snapshot=beginTyping(f,"#bookmark-edit-note","刷新失败时还在输入",{composing:true}),failed=deferred();f.gates.mark_list=failed;
  const loading=f.timeline.refreshBookmarks();await flush();failed.reject(Error("临时连接中断"));await loading;
  assert.ok(f.root.querySelector(".bookmark-error"));assertUninterrupted(f,snapshot);
  await f.timeline.refreshBookmarks();assert.equal(f.root.querySelector(".bookmark-error"),null);assertUninterrupted(f,snapshot);
});

function playerSnapshot(f){return [".timeline-player","video",".timeline-controls",".timeline-danmaku",'[data-action="timeline-play"]'].map(selector=>{const element=f.root.querySelector(selector);assert.ok(element);return {selector,element,detachments:element.detachments};});}
function assertPlayerUntouched(f,snapshot){for(const item of snapshot){assert.equal(f.root.querySelector(item.selector)===item.element,true,`${item.selector} is unchanged by bookmark-only updates`);assert.equal(item.element.detachments,item.detachments,`${item.selector} never detaches`);}}

test("creating a bookmark adds its timeline flag immediately and does not touch the player",async t=>{
  const f=await fixture(t),snapshot=playerSnapshot(f),before=f.root.querySelectorAll(".timeline-bookmark-markers .timeline-bookmark").length;
  const input=f.root.querySelector("#bookmark-title");input.value="刚才精彩的回应";input.dispatchEvent({type:"input",bubbles:true});
  await f.timeline.action(action("timeline-mark-create"));
  const created=f.marks.at(-1);assert.ok(created.id.startsWith("created-"));
  assert.ok(f.root.querySelector(`.timeline-bookmark-markers [data-mark="${created.id}"]`));
  assert.equal(f.root.querySelectorAll(".timeline-bookmark-markers .timeline-bookmark").length,before+1);
  assert.equal(f.root.querySelector("#bookmark-title").value,"");assertPlayerUntouched(f,snapshot);
});

test("ordinary save closes the editor and shows the persisted note while keeping the player mounted",async t=>{
  const f=await fixture(t);await f.timeline.action(action("timeline-mark-edit","mark-0"));const snapshot=playerSnapshot(f);
  const note=f.root.querySelector("#bookmark-edit-note");note.value="已确认保存的新备注";note.dispatchEvent({type:"input",bubbles:true});
  f.root.querySelector('[data-action="timeline-mark-save"]').focus();await f.timeline.action(action("timeline-mark-save"));
  assert.equal(f.root.querySelector("#bookmark-edit-note"),null);assert.equal(f.root.querySelector("#bookmark-edit-title"),null);
  assert.ok(f.root.querySelector('[data-bookmark-id="mark-0"]').textContent.includes("已确认保存的新备注"));assertPlayerUntouched(f,snapshot);
});

test("checkbox updates and deletion retain the correct keyed rows without repainting the player",async t=>{
  const f=await fixture(t),snapshot=playerSnapshot(f),first=f.root.querySelector('[data-bookmark-id="mark-0"]'),second=f.root.querySelector('[data-bookmark-id="mark-1"]');
  const checkbox=first.querySelector('[data-bookmark-select="mark-0"]');checkbox.checked=true;f.timeline.change(checkbox);
  assert.equal(f.root.querySelector('[data-bookmark-id="mark-0"]'),first);assert.equal(checkbox.checked,true);assert.ok(first.classList.contains("selected"));
  await f.timeline.action(action("timeline-mark-delete","mark-0"));await f.timeline.action(action("timeline-mark-delete-confirm","mark-0"));
  assert.equal(f.root.querySelector('[data-bookmark-id="mark-0"]'),null);assert.equal(f.root.querySelector('[data-bookmark-id="mark-1"]'),second);assertPlayerUntouched(f,snapshot);
});

test("removing an earlier row in a refreshed list never detaches the composing editor in a surviving row",async t=>{
  const f=await fixture(t);await f.timeline.action(action("timeline-mark-edit","mark-1"));
  const snapshot=beginTyping(f,"#bookmark-edit-note","相邻打点消失时继续组合",{composing:true});
  f.marks.splice(0,1);await f.timeline.refreshBookmarks();
  assert.equal(f.root.querySelector('[data-bookmark-id="mark-0"]'),null);assertUninterrupted(f,snapshot);
});

test("switching sessions intentionally replaces the old editor and ignores the previous session refresh",async t=>{
  const f=await fixture(t);await f.timeline.action(action("timeline-mark-edit","mark-0"));
  const snapshot=beginTyping(f,"#bookmark-edit-note","仅属于旧场次的草稿",{composing:true}),pending=deferred();f.gates.mark_list=pending;
  const reading=f.timeline.refreshBookmarks();await flush();f.timeline.selectSession("22");await flush();
  assert.equal(snapshot.input.isConnected,false,"switching sessions intentionally discards the old view");
  assert.equal(f.root.querySelector("#bookmark-edit-note"),null);assert.equal(f.root.querySelectorAll("[data-bookmark-id]").length,0);
  pending.resolve({marks:f.marks,total:f.marks.length});await reading;
  assert.equal(f.root.querySelectorAll("[data-bookmark-id]").length,0,"a late previous-session list must not leak into the new session");
  f.timeline.selectSession("21");await flush();assert.equal(f.root.querySelector("#bookmark-edit-note"),null);assert.equal(f.root.querySelector("#bookmark-title").value,"");
});
