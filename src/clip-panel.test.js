import test from "node:test";
import assert from "node:assert/strict";
import {createClipPanel,clipJobMarkup,clipJobDay,filterClipJobs,groupClipJobs} from "./clip-panel.js";

function fixture(bridge) {
  const root={innerHTML:"",isConnected:true},calls=[],notices=[];
  const panel=createClipPanel({bridge:async(method,params)=>{calls.push([method,params]);return bridge(method,params);},invoke:async(...args)=>calls.push(args),toast:(...args)=>notices.push(args)});
  return {root,panel,calls,notices};
}
const job=(status="running")=>({id:"job-a",session_id:"session-a",title:"观众追问<script>",status,phase:"downloading",progress:null,downloaded_bytes:1024,requested_start_seconds:10,requested_end_seconds:40});
test("whole-session tasks display an unresolved duration honestly and use the whole-session source in search",()=>{
  const whole={...job("queued"),mode:"full_session",source:"full_session",requested_start_seconds:0,requested_end_seconds:0};
  assert.match(clipJobMarkup(whole),/等待确认完整时长/);assert.doesNotMatch(clipJobMarkup(whole),/00:00:00 —/);
  assert.equal(filterClipJobs([whole],{search:"整场直播"}).length,1);
  assert.match(clipJobMarkup({...whole,requested_end_seconds:14400}),/04:00:00/);
});
test("unknown progress stays indeterminate; completed results show actual boundaries and a safe folder action",()=>{
  const running=clipJobMarkup(job());
  assert.match(running,/<progress[^>]*max="100"\s*>/);
  assert.doesNotMatch(running,/value="(?:90|100)"/);
  assert.match(running,/观众追问&lt;script&gt;/);assert.doesNotMatch(running,/<script>/);
  const completed=clipJobMarkup({...job("completed"),actual_start_seconds:8,actual_end_seconds:40,output_path:'C:\\clips\\a.mp4'});
  assert.match(completed,/实际片段 00:00:08/);assert.match(completed,/clip-job-folder/);assert.doesNotMatch(completed,/clip-job-cancel|<progress/);
});
test("cancel and retry read authoritative job state instead of reporting an optimistic completed file",async t=>{
  let status="running";
  const f=fixture(async method=>{
    if(method==="clip_tools")return {available:true};
    if(method==="clip_cancel"){status="cancelled";return {};}
    if(method==="clip_retry"){status="queued";return {};}
    return {jobs:[job(status)]};
  });t.after(()=>f.panel.unmount());
  await f.panel.mount(f.root);
  await f.panel.action({dataset:{action:"clip-job-cancel",job:"job-a"}});
  assert.ok(f.calls.some(([method,p])=>method==="clip_cancel"&&p.job_id==="job-a"));
  assert.match(f.root.innerHTML,/已取消/);assert.match(f.root.innerHTML,/clip-job-retry/);
  await f.panel.action({dataset:{action:"clip-job-retry",job:"job-a"}});
  assert.match(f.root.innerHTML,/等待下载/);assert.doesNotMatch(f.root.innerHTML,/打开文件夹/);
});
test("opening a completed job uses the backend result path and never opens an unfinished file",async t=>{
  const f=fixture(async method=>method==="clip_tools"?{available:false}:{jobs:[{...job("completed"),output_path:"C:/clips/result.mp4"},job("running")]});
  t.after(()=>f.panel.unmount());await f.panel.mount(f.root);
  assert.match(f.root.innerHTML,/媒体工具未准备好/);
  await f.panel.action({dataset:{action:"clip-job-folder",job:"job-a"}});
  assert.ok(f.calls.some(([method,p])=>method==="open_clip_folder"&&p.path==="C:/clips/result.mp4"&&p.jobId==="job-a"));
});
test("leaving the jobs page does not cancel downloads and ignores late refresh results",async()=>{
  let finish;
  const f=fixture(method=>method==="clip_tools"?{available:true}:new Promise(resolve=>{finish=resolve;}));
  const mounting=f.panel.mount(f.root);f.panel.unmount();f.root.innerHTML="another page";finish({jobs:[job()]});await mounting;
  assert.equal(f.root.innerHTML,"another page");assert.equal(f.calls.filter(([method])=>method==="clip_cancel").length,0);
  const before=f.calls.length;await f.panel.tick(true);assert.equal(f.calls.length,before);
});

test("download tasks group by Shanghai creation day and search metadata without hiding undated legacy tasks",()=>{
  const rows=[{...job("completed"),id:"a",created_at:"2026-10-24T11:30:00+08:00",anchor:"桃桃大王",output_path:"D:/切片/追问.mp4",source:"bookmark"},{...job("failed"),id:"b",created_at:"2026-10-25T11:30:00+08:00",anchor_id:"12345",source:"highlight"},{...job(),id:"legacy"}];
  assert.equal(clipJobDay(rows[0]),"2026-10-24");
  assert.equal(groupClipJobs(rows).length,3);assert.equal(groupClipJobs(rows).at(-1).day,"");
  assert.deepEqual(filterClipJobs(rows,{search:"桃桃",date:"2026-10-24",status:"completed"}).map(j=>j.id),["a"]);
  assert.deepEqual(filterClipJobs(rows,{search:"追问.mp4"}).map(j=>j.id),["a"]);
  assert.deepEqual(filterClipJobs(rows,{search:"12345",status:"failed"}).map(j=>j.id),["b"]);
  assert.deepEqual(filterClipJobs(rows,{search:"直播打点"}).map(j=>j.id),["a"]);
  assert.equal(filterClipJobs(rows,{status:"active"}).length,1);
  assert.equal(clipJobDay({created_at:"2026-10-24T16:01:00Z"}),"2026-10-25");
  assert.match(clipJobMarkup({...job(),created_at:"2026-10-24T16:01:00Z"}),/>00:01<\/time>/);
});

const history=(jobs,{total=jobs.length,page=0,active_count=0,limit=30}={})=>({jobs,total,page,limit,active_count,has_more:(page+1)*limit<total,history:true});
const settle=async()=>{await Promise.resolve();await Promise.resolve();};

test("server history requests pages and keeps the current page during polling",async t=>{
  const rows=Array.from({length:65},(_,index)=>({...job("completed"),id:`history-${index}`,title:`片段 ${index}`,created_at:"2026-10-24T16:01:00Z"}));
  const f=fixture(async(method,p)=>method==="clip_tools"?{available:true}:history(rows.slice(p.page*30,(p.page+1)*30),{total:65,page:p.page,active_count:4}));
  t.after(()=>f.panel.unmount());await f.panel.mount(f.root);
  assert.deepEqual(f.calls.find(([method])=>method==="clip_jobs")[1],{date:"",status:"",search:"",page:0,limit:30});
  assert.match(f.root.innerHTML,/1–30 \/ 65 个任务/);assert.match(f.root.innerHTML,/26\/10\/25/);
  await f.panel.action({dataset:{action:"clip-jobs-next"}});assert.match(f.root.innerHTML,/31–60 \/ 65 个任务/);
  await f.panel.tick(true);assert.equal(f.calls.filter(([method])=>method==="clip_jobs").at(-1)[1].page,1);
  await f.panel.action({dataset:{action:"clip-jobs-next"}});assert.match(f.root.innerHTML,/61–65 \/ 65 个任务/);
  assert.match(f.root.innerHTML,/data-action="clip-jobs-next" disabled/);
  f.panel.change({id:"clip-jobs-status",value:"completed"});await settle();
  const last=f.calls.filter(([method])=>method==="clip_jobs").at(-1)[1];assert.equal(last.page,0);assert.equal(last.status,"completed");
});

test("search debounces to one request containing the newest text",async t=>{
  t.mock.timers.enable({apis:["setTimeout"]});
  const f=fixture(async method=>method==="clip_tools"?{available:true}:history([]));
  t.after(()=>f.panel.unmount());await f.panel.mount(f.root);
  f.panel.input({id:"clip-jobs-search",value:"桃"});f.panel.input({id:"clip-jobs-search",value:"桃桃"});
  assert.equal(f.calls.filter(([method])=>method==="clip_jobs").length,1);
  t.mock.timers.tick(219);await settle();assert.equal(f.calls.filter(([method])=>method==="clip_jobs").length,1);
  t.mock.timers.tick(1);await settle();
  const requests=f.calls.filter(([method])=>method==="clip_jobs");assert.equal(requests.length,2);assert.equal(requests[1][1].search,"桃桃");
});

test("a response for an old filter cannot replace a newer server page",async t=>{
  let finishOld,reads=0;
  const f=fixture((method,p)=>{
    if(method==="clip_tools")return {available:true};reads++;
    if(reads===2)return new Promise(resolve=>{finishOld=resolve;});
    return history([{...job("completed"),title:p.search?"新的服务端匹配结果":"初始内容"}]);
  });
  t.after(()=>f.panel.unmount());await f.panel.mount(f.root);
  const oldRead=f.panel.tick(true);f.panel.input({id:"clip-jobs-search",value:"服务器已匹配"});await f.panel.tick(true);
  finishOld(history([{...job("completed"),title:"过期响应内容"}]));await oldRead;
  assert.match(f.root.innerHTML,/新的服务端匹配结果/);assert.doesNotMatch(f.root.innerHTML,/过期响应内容|初始内容/);
});

test("server filtering and polling keep search DOM and selection intact",async t=>{
  const nodes=new Map(),root={isConnected:true,set innerHTML(value){this._html=value;for(const id of ["clip-jobs-results","clip-jobs-notice","clip-jobs-count","clip-jobs-search","clip-jobs-date","clip-jobs-status"])nodes.set(`#${id}`,{id,innerHTML:"",value:"",selectionStart:0,selectionEnd:0});},get innerHTML(){return this._html;},querySelector:selector=>nodes.get(selector)};
  const panel=createClipPanel({bridge:async method=>method==="clip_tools"?{available:true}:history([{...job(),title:"远端命中",created_at:"2026-10-25T00:00:00+08:00"}],{total:1,active_count:1})});
  t.after(()=>panel.unmount());await panel.mount(root);
  const search=nodes.get("#clip-jobs-search");search.value="远端";search.selectionStart=1;search.selectionEnd=2;panel.input(search);await panel.tick(true);
  assert.equal(nodes.get("#clip-jobs-search"),search);assert.equal(search.value,"远端");assert.equal(search.selectionStart,1);assert.equal(search.selectionEnd,2);
  assert.match(nodes.get("#clip-jobs-results").innerHTML,/远端命中/);assert.equal(nodes.get("#clip-jobs-count").textContent,"1 个任务 · 1 个处理中");
});

test("refresh and filter updates preserve the existing search input and its selection",async t=>{
  const nodes=new Map(),root={isConnected:true,_html:"",set innerHTML(value){this._html=value;for(const id of ["clip-jobs-results","clip-jobs-notice","clip-jobs-count","clip-jobs-search","clip-jobs-date","clip-jobs-status"])nodes.set(`#${id}`,{id,innerHTML:"",value:"",selectionStart:0,selectionEnd:0});},get innerHTML(){return this._html;},querySelector(selector){return nodes.get(selector);}};
  let calls=0;
  const panel=createClipPanel({bridge:async method=>method==="clip_tools"?{available:true}:{jobs:[{...job(),id:`job-${++calls}`,anchor:"桃桃大王",created_at:"2026-10-25T11:30:00+08:00"}]}});
  t.after(()=>panel.unmount());await panel.mount(root);
  const search=nodes.get("#clip-jobs-search");search.value="桃桃";search.selectionStart=1;search.selectionEnd=2;
  panel.input(search);await panel.tick(true);
  assert.equal(nodes.get("#clip-jobs-search"),search);assert.equal(search.value,"桃桃");assert.equal(search.selectionStart,1);assert.equal(search.selectionEnd,2);
  assert.match(nodes.get("#clip-jobs-results").innerHTML,/桃桃大王/);
  await panel.action({dataset:{action:"clip-jobs-clear"}});assert.equal(search.value,"");
});

