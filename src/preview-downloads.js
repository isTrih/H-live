// Development preview only. All jobs, paths and transfers are simulated; no files or APIs are accessed.
const KEY="qingkan-preview-downloads-v1";
const DEFAULTS={root_directory:"C:/氢看下载",directory_template:"",filename_template:"{session_id}-{start_ms}-{job_id8}.mp4"};
const VARIABLES=[
  ["anchor","主播名称","小天"],["anchor_id","主播 ID","123456"],["session_id","场次 ID","7698991795467832114"],["title","切片标题","精彩互动"],
  ["date","场次日期","2026-10-25"],["year","年","2026"],["month","月","10"],["day","日","25"],["download_date","下载任务日期","2026-10-25"],
  ["start","片段起点","00-02-00"],["end","片段终点","00-02-30"],["start_ms","起点毫秒","120000"],["end_ms","终点毫秒","150000"],["duration","片段秒数","30"],
  ["job_id","任务 ID","0123456789abcdef0123456789abcdef"],["job_id8","任务短 ID","01234567"],["source","来源","打点"],["bookmark_id","打点 ID","0123456789abcdef0123456789abcdef"],
].map(([key,label,example])=>({key,label,example}));
const clone=value=>JSON.parse(JSON.stringify(value));
const storageDefault=()=>{try{return globalThis.localStorage;}catch{return null;}};
const dayOf=raw=>{
  if(/^\d{4}-\d{2}-\d{2}$/.test(String(raw||""))){const date=new Date(`${raw}T00:00:00Z`);if(Number.isFinite(+date)&&date.toISOString().slice(0,10)===raw)return raw;}
  const date=new Date(raw);if(!Number.isFinite(+date))throw Error("日期无效");return new Date(+date+8*3600000).toISOString().slice(0,10);
};
const clock=seconds=>[Math.floor(seconds/3600),Math.floor(seconds/60)%60,Math.floor(seconds)%60].map(n=>String(n).padStart(2,"0")).join("-");
const invalid=/[\u0000-\u001f\u007f<>:"/\\|?*]/;
const reserved=value=>/^(CON|PRN|AUX|NUL|COM[1-9¹²³]|LPT[1-9¹²³])(?:\.|$)/i.test(value);
const sanitize=value=>{
  const clean=String(value??"").slice(0,80).replace(/[\u0000-\u001f\u007f<>:"/\\|?*]/g,"_").trim().replace(/[. ]+$/,"");
  return clean&&!/^\.{1,2}$/.test(clean)?clean:"未命名";
};
function component(template,values) {
  let result="",offset=0;
  for(const match of template.matchAll(/\{([^{}]*)\}/g)){
    const literal=template.slice(offset,match.index);if(/[{}]/.test(literal)||invalid.test(literal))throw Error("命名规则包含无效字符");
    if(!Object.hasOwn(values,match[1]))throw Error(`未知命名变量：{${match[1]}}`);
    result+=literal+sanitize(values[match[1]]);offset=match.index+match[0].length;
  }
  const tail=template.slice(offset);if(/[{}]/.test(tail)||invalid.test(tail))throw Error("命名规则包含无效字符");
  result=(result+tail).trim().replace(/[. ]+$/,"");if(!result||/^\.{1,2}$/.test(result))throw Error("目录或文件名不能为空或为 ..");
  if(reserved(result))result=`_${result}`;if(result.length>200)throw Error("目录或文件名过长，请缩短命名规则");return result;
}
function example(){return {anchor:"小天",anchor_id:"123456",session_id:"7698991795467832114",title:"精彩互动",session_date:"2026-10-25",created_at:"2026-10-25T12:00:00+08:00",start_seconds:120,end_seconds:150,job_id:"0123456789abcdef0123456789abcdef",source:"bookmark",bookmark_id:"0123456789abcdef0123456789abcdef"};}
function render(settings,context=example()) {
  const start=context.start_seconds,end=context.end_seconds;
  if(!Number.isFinite(start)||start<0||!Number.isFinite(end)||end<=start)throw Error("片段终点须晚于起点");
  const date=dayOf(context.session_date),downloadDate=dayOf(context.created_at),values={...context,date,year:date.slice(0,4),month:date.slice(5,7),day:date.slice(8,10),download_date:downloadDate,start:clock(start),end:clock(end),start_ms:String(Math.round(start*1000)),end_ms:String(Math.round(end*1000)),duration:String(Math.round((end-start)*1000)/1000),job_id8:String(context.job_id).slice(0,8),source:({manual:"手动",highlight:"亮点",bookmark:"打点",full_session:"整场"})[context.source]||"手动",bookmark_id:context.bookmark_id||"无打点"};
  const directory=settings.directory_template.replaceAll("\\","/");
  if(directory.startsWith("/")||directory.endsWith("/")||directory.includes("//"))throw Error("目录规则须为相对目录，使用 / 分隔");
  const parts=directory?directory.split("/"):[];if(parts.length>8)throw Error("最多支持 8 层子目录");
  if(directory.length>1024||settings.filename_template.length>512)throw Error("命名规则过长");
  const folders=parts.map(part=>component(part,values));let filename=component(settings.filename_template,values);if(!filename.toLowerCase().endsWith(".mp4"))filename+=".mp4";
  const relative_path=[...folders,filename].join("/"),base=settings.root_directory.replaceAll("\\","/").replace(/([^:])\/$/,"$1");
  if(!(/^[a-z]:\//i.test(base)||base.startsWith("/"))||/[\u0000-\u001f]/.test(base))throw Error("请选择完整的下载目录");
  return {relative_path,output_path:`${base}${base.endsWith("/")?"":"/"}${relative_path}`,preview:true};
}
const publicJob=job=>{const {_started,_request_key,_fingerprint,...result}=job;return {...result,preview:true,simulated:true};};

export function createPreviewDownloads({getAnchor=()=>null,getMark=()=>null,getReplay=()=>({available:true,ended:true,duration_seconds:7200}),storage=storageDefault(),now=()=>Date.now(),seed=true}={}) {
  const stamp=()=>new Date(now()).toISOString();
  let state={format:1,settings:{...DEFAULTS},jobs:[]};
  try {
    const saved=JSON.parse(storage?.getItem(KEY)||"null");
    if(saved?.format===1&&saved.settings&&Object.keys(DEFAULTS).every(key=>typeof saved.settings[key]==="string")){
      render(saved.settings);state={format:1,settings:{...saved.settings},jobs:Array.isArray(saved.jobs)?saved.jobs.filter(job=>typeof job?.id==="string"&&/^[a-f0-9]{32}$/i.test(job.id)&&["queued","running","completed","failed","cancelled"].includes(job.status)):[]};
    }
  }catch{/* Corrupt preview storage does not affect the actual application. */}
  function persist(next){storage?.setItem(KEY,JSON.stringify(next));state=next;}
  if(seed&&!state.jobs.length){
    state.jobs=[
      {id:"0123456789abcdef0123456789abcdef",session_id:"7698991795467832114",anchor_id:"100000003001",anchor_name:"测试主播乙",title:"观众追问操作技巧",source:"bookmark",bookmark_id:"fedcba9876543210fedcba9876543210",status:"completed",phase:"validated",progress:100,downloaded_bytes:18432000,total_bytes:18432000,attempt:1,requested_start_seconds:120,requested_end_seconds:165,actual_start_seconds:120,actual_end_seconds:165,created_at:new Date(now()-86400000).toISOString(),updated_at:new Date(now()-86400000).toISOString(),session_date:dayOf(new Date(now()-86400000).toISOString()),error:null,preview:true,simulated:true},
      {id:"123456789abcdef0123456789abcdef0",session_id:"7696456021958609675",anchor_id:"100000003002",anchor_name:"测试主播丙",title:"连招配合与回应",source:"highlight",status:"failed",phase:"resolving",progress:0,downloaded_bytes:0,total_bytes:null,attempt:1,requested_start_seconds:450,requested_end_seconds:490,created_at:stamp(),updated_at:stamp(),session_date:dayOf(stamp()),error:"预览任务：暂无录播，演示重试入口。",output_path:"",preview:true,simulated:true},
    ];
    for(const job of state.jobs)if(job.status==="completed")job.output_path=render(state.settings,{...job,anchor:job.anchor_name,start_seconds:job.requested_start_seconds,end_seconds:job.requested_end_seconds,job_id:job.id}).output_path;
  }
  const settingsResponse=()=>({settings:{...state.settings},defaults:{...DEFAULTS},variables:clone(VARIABLES),root_available:true,preview:render(state.settings),simulated:true});
  const settingsOf=params=>{const next={...state.settings};for(const key of Object.keys(DEFAULTS))if(Object.hasOwn(params,key)){if(typeof params[key]!=="string")throw Error("命名设置必须为文本");next[key]=params[key].trim();}return next;};
  async function handle(method,params={}) {
    if(!["download_naming_settings","download_naming_save","download_naming_preview","clip_tools","clip_create","clip_jobs","clip_cancel","clip_retry"].includes(method))return undefined;
    if(method==="download_naming_settings")return settingsResponse();
    if(method==="download_naming_save"||method==="download_naming_preview"){
      const settings=settingsOf(params),context=params.context;
      if(context!==undefined&&(!context||typeof context!=="object"||Array.isArray(context)))throw Error("预览内容无效");
      const preview=render(settings,{...example(),...(context||{})});
      if(method==="download_naming_preview")return preview;
      persist({...state,settings});return settingsResponse();
    }
    if(method==="clip_tools")return {available:true,mode:"copy",full_session_available:true,preview:true,simulated:true};
    if(method==="clip_jobs"){
      const jobs=state.jobs.map(job=>{
        if(!["queued","running"].includes(job.status))return job;
        const progress=Math.max(0,Math.min(100,(now()-(job._started??now()))/150));
        return {...job,progress,status:progress===100?"completed":progress<8?"queued":"running",phase:progress<12?"planning":progress<85?"downloading":progress===100?"validated":"muxing",downloaded_bytes:Math.round(progress*250000),updated_at:stamp(),...(progress===100?{actual_start_seconds:job.requested_start_seconds,actual_end_seconds:job.requested_end_seconds,total_bytes:25000000}:{}),preview:true,simulated:true};
      });
      if(jobs.some((job,index)=>job!==state.jobs[index]))persist({...state,jobs});
      const active_count=jobs.filter(job=>["queued","running"].includes(job.status)).length;
      const date=String(params.date||""),status=String(params.status||""),search=String(params.search||"").trim().toLocaleLowerCase();
      if(date&&dayOf(date)!==date)throw Error("下载日期无效");
      if(!["","active","queued","running","completed","failed","cancelled"].includes(status))throw Error("任务状态无效");
      const sources={manual:"手动切片",highlight:"互动亮点",bookmark:"直播打点",full_session:"整场直播"};
      const matching=jobs.filter(job=>(!date||dayOf(job.created_at)===date)&&(!status||(status==="active"?["queued","running"].includes(job.status):job.status===status))&&(!search||[job.title,job.anchor_name,job.anchor_id,job.session_id,job.id,job.output_path,job.bookmark_id,job.source,sources[job.source||"manual"]].some(value=>String(value||"").toLocaleLowerCase().includes(search))))
        .sort((a,b)=>(Date.parse(b.created_at)||0)-(Date.parse(a.created_at)||0)||String(b.id).localeCompare(String(a.id)));
      const limit=Number.isInteger(params.limit)?Math.max(1,Math.min(100,params.limit)):30,total=matching.length,page=Math.min(Number.isInteger(params.page)?Math.max(0,params.page):0,Math.max(0,Math.ceil(total/limit)-1));
      return {jobs:matching.slice(page*limit,(page+1)*limit).map(publicJob),total,active_count,page,limit,has_more:(page+1)*limit<total,history:true,preview:true,simulated:true};
    }
    if(method==="clip_cancel"||method==="clip_retry"){
      const index=state.jobs.findIndex(job=>job.id===params.job_id);if(index<0)throw Error("任务不存在");let job=state.jobs[index];
      if(method==="clip_cancel"&&["queued","running"].includes(job.status))job={...job,status:"cancelled",phase:"cancelled",updated_at:stamp()};
      if(method==="clip_retry"&&["failed","cancelled"].includes(job.status))job={...job,status:"queued",phase:"resolving",progress:0,downloaded_bytes:0,total_bytes:null,error:null,_started:now(),attempt:job.attempt+1,updated_at:stamp()};
      persist({...state,jobs:state.jobs.map((old,i)=>i===index?job:old)});return {job_id:job.id,job:publicJob(job),preview:true,simulated:true};
    }
    const mode=params.mode??"clip",full=mode==="full_session";
    if(!["clip","full_session"].includes(mode))throw Error("下载模式无效");
    if(full&&(Object.hasOwn(params,"start_seconds")||Object.hasOwn(params,"end_seconds")||params.bookmark_id))throw Error("整场下载不接受切片起止时间或打点");
    const replay=full?await getReplay(params):null;
    if(full&&(!replay?.available||replay?.ended!==true||!Number.isFinite(replay.duration_seconds)||replay.duration_seconds<=0))throw Error("整场录像尚未同步完成");
    const start=full?0:params.start_seconds,end=full?replay.duration_seconds:params.end_seconds;
    if(typeof params.session_id!=="string"||!/^\d+$/.test(params.session_id))throw Error("场次 ID 无效");
    if(!Number.isFinite(start)||!Number.isFinite(end)||start<0||end-start<.1||end-start>(full?31*86400:900))throw Error("下载时长无效");
    const title=String(params.title||"直播切片").trim();if([...title].length>120||/[\u0000-\u001f\u007f]/.test(title))throw Error("切片标题无效");
    const source=full?"full_session":params.source||"manual";if(!["manual","highlight","bookmark","full_session"].includes(source)||!full&&source==="full_session")throw Error("切片来源无效");
    const anchor=await getAnchor(params)||{},mark=params.bookmark_id?await getMark(params.bookmark_id):null;
    if(params.bookmark_id&&(!mark||mark.session_id!==params.session_id||params.anchor_id&&mark.anchor_id!==params.anchor_id))throw Error("打点不属于此场次");
    const fingerprint=JSON.stringify([mode,params.session_id,start,end,source,params.bookmark_id||""]);
    if(params.request_key){const existing=state.jobs.find(job=>job._request_key===params.request_key);if(existing){if(existing._fingerprint!==fingerprint)throw Error("请求标识对应的片段已变化");return {job_id:existing.id,job:publicJob(existing),preview:true,simulated:true};}}
    const id=crypto.randomUUID().replaceAll("-",""),created=stamp(),sessionDate=dayOf(params.session_date||created);
    const job={id,mode,session_id:params.session_id,anchor_id:params.anchor_id||anchor.anchor_id||mark?.anchor_id||"",anchor_name:params.anchor_name||anchor.name||anchor.anchor_name||"主播",title,source,bookmark_id:params.bookmark_id||null,session_date:sessionDate,status:"queued",phase:"resolving",progress:0,downloaded_bytes:0,total_bytes:null,attempt:1,requested_start_seconds:start,requested_end_seconds:end,actual_start_seconds:null,actual_end_seconds:null,error:null,created_at:created,updated_at:created,_started:now(),_request_key:params.request_key||null,_fingerprint:fingerprint,preview:true,simulated:true};
    const path=render(state.settings,{...job,anchor:job.anchor_name,start_seconds:start,end_seconds:end,job_id:id}).output_path;
    let output=path;for(let index=2;state.jobs.some(existing=>existing.output_path?.toLowerCase()===output.toLowerCase());index++)output=path.replace(/\.mp4$/i,` (${index}).mp4`);job.output_path=output;
    persist({...state,jobs:[job,...state.jobs]});
    return {job_id:id,job:publicJob(job),preview:true,simulated:true};
  }
  return {handle};
}
