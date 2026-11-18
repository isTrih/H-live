import test from "node:test";
import assert from "node:assert/strict";
import {createPreviewDownloads} from "./preview-downloads.js";
import {filterClipJobs,groupClipJobs} from "./clip-panel.js";

const memory=()=>{const values=new Map();return {getItem:key=>values.get(key)||null,setItem:(key,value)=>values.set(key,value)};};
const epoch=Date.parse("2026-10-25T12:00:00+08:00");
const request={session_id:"7698991795467832114",anchor_id:"123456",start_seconds:120,end_seconds:165,title:"观众追问",source:"manual",session_date:"2026-10-24"};
test("whole-session preview uses the VOD length, saved naming rules and the existing cancel/retry queue",async()=>{
  const f=createPreviewDownloads({storage:memory(),seed:false,now:()=>epoch,getReplay:()=>({available:true,ended:true,duration_seconds:14400})});
  await f.handle("download_naming_save",{directory_template:"{source}",filename_template:"{session_id}_{duration}"});
  const {job_id,job}=await f.handle("clip_create",{session_id:request.session_id,mode:"full_session",request_key:"whole:one"});
  assert.equal(job.requested_start_seconds,0);assert.equal(job.requested_end_seconds,14400);assert.equal(job.mode,"full_session");
  assert.match(job.output_path,/整场\/7698991795467832114_14400\.mp4$/);
  assert.equal((await f.handle("clip_cancel",{job_id})).job.status,"cancelled");
  assert.equal((await f.handle("clip_retry",{job_id})).job.mode,"full_session");
  assert.equal((await f.handle("clip_create",{session_id:request.session_id,mode:"full_session",request_key:"whole:one"})).job_id,job_id);
  await assert.rejects(f.handle("clip_create",{session_id:request.session_id,mode:"full_session",end_seconds:60}),/不接受切片起止/);
});
test("whole-session preview refuses unfinished or missing VOD and leaves the queue empty",async()=>{
  for(const replay of [{available:false},{available:true,ended:false,duration_seconds:100}]){
    const f=createPreviewDownloads({storage:memory(),seed:false,getReplay:()=>replay});
    await assert.rejects(f.handle("clip_create",{session_id:request.session_id,mode:"full_session"}),/尚未同步完成/);
    assert.equal((await f.handle("clip_jobs")).total,0);
  }
});

test("naming settings persist across preview reloads; previews never mutate saved rules",async()=>{
  const storage=memory(),first=createPreviewDownloads({storage,seed:false,now:()=>epoch});
  const draft={root_directory:"D:/视频",directory_template:"{date}/{anchor}",filename_template:"{title}_{start}-{end}"};
  const preview=await first.handle("download_naming_preview",{...draft,context:{anchor:"桃桃/大王",title:"观众:追问"}});
  assert.equal(preview.relative_path,"2026-10-25/桃桃_大王/观众_追问_00-02-00-00-02-30.mp4");
  assert.equal((await first.handle("download_naming_settings")).settings.directory_template,"");
  const saved=await first.handle("download_naming_save",draft);
  assert.deepEqual(saved.settings,draft);assert.ok(saved.variables.some(item=>item.key==="download_date"));assert.ok(saved.root_available);
  const reloaded=createPreviewDownloads({storage,seed:false});
  assert.deepEqual((await reloaded.handle("download_naming_settings")).settings,draft);
  assert.equal(saved.simulated,true);
});

test("preview templates reject traversal and unknown variables without overwriting persisted rules",async()=>{
  const f=createPreviewDownloads({storage:memory(),seed:false});
  for(const directory_template of ["../精选","/绝对路径","A//B","{unknown}"]){await assert.rejects(f.handle("download_naming_save",{directory_template}));}
  await assert.rejects(f.handle("download_naming_preview",{filename_template:"{title"}));
  assert.equal((await f.handle("download_naming_settings")).settings.directory_template,"");
  const windowsRoot=await f.handle("download_naming_preview",{root_directory:"C:/"});
  assert.match(windowsRoot.output_path,/^C:\/\d/);
});

test("created tasks use saved rules, retain source metadata and survive page reload",async()=>{
  const storage=memory(),f=createPreviewDownloads({storage,seed:false,now:()=>epoch,getAnchor:()=>({anchor_id:"123456",name:"桃桃大王"})});
  await f.handle("download_naming_save",{directory_template:"{date}/{anchor}",filename_template:"{title}_{source}"});
  const created=await f.handle("clip_create",{...request,request_key:"same-click"});
  assert.equal(created.job.preview,true);assert.equal(created.job.simulated,true);
  assert.match(created.job.output_path,/2026-10-24\/桃桃大王\/观众追问_手动\.mp4$/);
  assert.equal(created.job.created_at,"2026-10-25T04:00:00.000Z");
  assert.equal((await f.handle("clip_create",{...request,request_key:"same-click"})).job_id,created.job_id);
  await assert.rejects(f.handle("clip_create",{...request,end_seconds:200,request_key:"same-click"}),/片段已变化/);
  const reloaded=createPreviewDownloads({storage,seed:false,now:()=>epoch+2000});
  const {jobs}=await reloaded.handle("clip_jobs");assert.equal(jobs.length,1);assert.equal(jobs[0].anchor_name,"桃桃大王");assert.equal(jobs[0].status,"running");
  assert.ok(jobs[0].progress>0);assert.ok(!Object.hasOwn(jobs[0],"_started"));assert.ok(!Object.hasOwn(jobs[0],"_request_key"));
});

test("cancel, retry and simulated progress retain original creation date",async()=>{
  let tick=epoch;const f=createPreviewDownloads({storage:memory(),seed:false,now:()=>tick});
  const {job_id,job}=await f.handle("clip_create",request);tick+=1000;
  assert.equal((await f.handle("clip_cancel",{job_id})).job.status,"cancelled");
  tick+=86400000;const retried=await f.handle("clip_retry",{job_id});assert.equal(retried.job.attempt,2);assert.equal(retried.job.created_at,job.created_at);
  tick+=16000;const completed=(await f.handle("clip_jobs")).jobs[0];assert.equal(completed.status,"completed");assert.equal(completed.progress,100);assert.equal(completed.simulated,true);assert.equal(completed.actual_start_seconds,120);
});

test("bookmark tasks require a matching mark, then preserve its identity and naming source",async()=>{
  const mark={id:"fedcba9876543210fedcba9876543210",session_id:request.session_id,anchor_id:request.anchor_id};
  const f=createPreviewDownloads({storage:memory(),seed:false,getMark:id=>id===mark.id?mark:null});
  const result=await f.handle("clip_create",{...request,source:"bookmark",bookmark_id:mark.id});
  assert.equal(result.job.source,"bookmark");assert.equal(result.job.bookmark_id,mark.id);
  await assert.rejects(f.handle("clip_create",{...request,session_id:"999",source:"bookmark",bookmark_id:mark.id}),/不属于此场次/);
});

test("default task fixtures provide date, status, name, ID and source search coverage",async()=>{
  const f=createPreviewDownloads({storage:memory(),now:()=>epoch});
  const {jobs,preview,simulated}=await f.handle("clip_jobs");assert.equal(preview,true);assert.equal(simulated,true);assert.equal(groupClipJobs(jobs).length,2);
  assert.equal(filterClipJobs(jobs,{status:"failed",search:"互动亮点"}).length,1);
  assert.equal(filterClipJobs(jobs,{search:"测试主播乙",date:"2026-10-24"}).length,1);
  assert.equal(filterClipJobs(jobs,{search:"7698991795467832114"}).length,1);
  assert.equal(filterClipJobs(jobs,{search:"-120000-01234567.mp4"}).length,1);
});

test("preview handler does not intercept unrelated commands or accept invalid clip ranges",async()=>{
  const f=createPreviewDownloads({storage:memory(),seed:false});assert.equal(await f.handle("session_timeline",{}),undefined);
  for(const end_seconds of [NaN,Infinity,119,120,1021])await assert.rejects(f.handle("clip_create",{...request,end_seconds}));
  assert.equal((await f.handle("clip_jobs")).jobs.length,0);
});

test("a failed storage write cannot report a successful save",async()=>{
  const f=createPreviewDownloads({storage:{getItem:()=>null,setItem:()=>{throw Error("存储不可写");}},seed:false});
  await assert.rejects(f.handle("download_naming_save",{directory_template:"{date}"}),/存储不可写/);
  assert.equal((await f.handle("download_naming_settings")).settings.directory_template,"");
});

test("preview history retains more than 100 tasks and applies server-style date, status and search pagination",async()=>{
  const storage=memory();let current=epoch;
  const f=createPreviewDownloads({storage,seed:false,now:()=>current});
  for(let index=0;index<115;index++){
    current=epoch+Math.floor(index/40)*86400000;
    await f.handle("clip_create",{...request,title:`检索片段 ${index}`,source:index%2?"highlight":"manual"});
  }
  const first=await f.handle("clip_jobs",{page:0,limit:30});assert.equal(first.total,115);assert.equal(first.jobs.length,30);assert.equal(first.has_more,true);assert.equal(first.active_count,35);assert.equal(first.history,true);
  const last=await f.handle("clip_jobs",{page:999,limit:30});assert.equal(last.page,3);assert.equal(last.jobs.length,25);assert.equal(last.has_more,false);
  const dated=await f.handle("clip_jobs",{date:"2026-10-25",status:"completed",limit:30,page:1});assert.equal(dated.total,40);assert.equal(dated.jobs.length,10);assert.equal(dated.active_count,35);
  const specific=await f.handle("clip_jobs",{search:"检索片段 103",status:"active"});assert.equal(specific.total,1);assert.equal(specific.jobs[0].source,"highlight");
  const reloaded=createPreviewDownloads({storage,seed:false,now:()=>current});const persisted=await reloaded.handle("clip_jobs",{date:"2026-10-25",limit:100});assert.equal(persisted.total,40);assert.equal(persisted.jobs.length,40);
});

test("preview history dates use Shanghai midnight even when timestamps are UTC",async()=>{
  const f=createPreviewDownloads({storage:memory(),seed:false,now:()=>Date.parse("2026-10-24T16:00:01Z")});
  await f.handle("clip_create",request);
  assert.equal((await f.handle("clip_jobs",{date:"2026-10-25"})).total,1);
  assert.equal((await f.handle("clip_jobs",{date:"2026-10-24"})).total,0);
});
