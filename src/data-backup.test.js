import test from "node:test";
import assert from "node:assert/strict";
import {createDataBackup,collectBackupPreferences,validateBackupPreferences,applyBackupPreferences,validateBackupSummary,validateBackupAnchors} from "./data-backup.js";
import {createPreviewBackup} from "./preview-backup.js";
import {UNTAGGED_TAG} from "./anchor-tags.js";

const summary={format:1,app_version:"0.4.3",created_at:"2026-10-26T02:03:04Z",counts:{anchors:3,sessions:8,messages:12345,marks:7,downloads:4}};
const inspected=(token="backup-token")=>({token,file_name:"全部数据.zip",summary});
const button=name=>({dataset:{action:`data-backup-${name}`}});
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};};
function memoryStorage(initial={}){
  const values=new Map(Object.entries(initial));
  return {get length(){return values.size;},key:index=>[...values.keys()][index]??null,getItem:key=>values.get(key)??null,
    setItem:(key,value)=>values.set(key,String(value)),removeItem:key=>values.delete(key),entries:()=>Object.fromEntries(values)};
}
function fixture(handler,{storage=memoryStorage(),onRestored=async()=>{}}={}){
  const listeners=new Map(),root={innerHTML:"",isConnected:true,addEventListener:(name,handler)=>listeners.set(name,handler),removeEventListener:(name,handler)=>{if(listeners.get(name)===handler)listeners.delete(name);}},calls=[],notices=[];
  const controller=createDataBackup({storage,invoke:async(command,params)=>{calls.push([command,params]);return handler(command,params);},toast:(...args)=>notices.push(args),onRestored});
  controller.mount(root);
  return {controller,root,calls,notices,storage,listeners};
}

test("preferences export only the three known local settings and normalize replay calibration values",()=>{
  const storage=memoryStorage({"livescope.appearance.font":"chill","livescope.updates.automatic":"false",
    "livescope.replay-offset.123:456":"-12.50","livescope.replay-offset.1:2":"Infinity",
    "livescope.replay-offset.0:0":"2","livescope.replay-offset.invalid":"8","community_api_key":"secret",
    "qingkan.preview.downloads.v1":"fixture","livescope.other":"unrelated"});
  assert.deepEqual(collectBackupPreferences(storage),{"livescope.appearance.font":"chill","livescope.updates.automatic":"false","livescope.replay-offset.123:456":"-12.5"});
  assert.throws(()=>validateBackupPreferences({"community_api_key":"never"}),/不支持/);
  assert.throws(()=>validateBackupPreferences({"livescope.appearance.font":"untrusted"}),/不支持/);
  assert.throws(()=>validateBackupPreferences({"livescope.replay-offset.1:2":"86401"}),/不支持/);
  assert.throws(()=>validateBackupPreferences({"livescope.replay-offset.1:2":12}),/格式/);
  const before=storage.entries();assert.throws(()=>applyBackupPreferences({"__other__":"value"},storage));assert.deepEqual(storage.entries(),before);
  const final=applyBackupPreferences({"livescope.appearance.font":"douyin","livescope.replay-offset.3:4":"2.0"},storage);
  assert.equal(storage.getItem("livescope.appearance.font"),"chill");assert.equal(storage.getItem("livescope.updates.automatic"),"false");assert.equal(storage.getItem("livescope.replay-offset.123:456"),"-12.50");
  assert.equal(final["livescope.replay-offset.3:4"],"2");assert.equal(final["livescope.appearance.font"],"chill");
  assert.equal(storage.getItem("community_api_key"),"secret");assert.equal(storage.getItem("qingkan.preview.downloads.v1"),"fixture");
});

test("native dialog cancellation has no restore side effects",async()=>{
  const f=fixture(async()=>null);
  await f.controller.action(button("export"));assert.match(f.root.innerHTML,/已取消导出/);
  await f.controller.action(button("inspect"));assert.match(f.root.innerHTML,/已取消选择/);
  await f.controller.action(button("restore"));
  assert.equal(f.calls.some(([command])=>command==="data_backup_restore"),false);
  f.controller.unmount();
});

test("flat native export summaries work and monitored anchors remain distinct from archive anchors",async()=>{
  const nativeSummary={...summary,counts:{...summary.counts,monitored_anchors:9}};
  const f=fixture(async command=>command==="data_backup_export"?{...nativeSummary,path:"C:/backup.zip",bytes:1024}:{...inspected(),summary:nativeSummary});
  await f.controller.action(button("export"));
  assert.match(f.root.innerHTML,/备份已导出/);assert.match(f.root.innerHTML,/C:\/backup.zip/);
  await f.controller.action(button("inspect"));
  assert.match(f.root.innerHTML,/监控主播<\/dt><dd>9/);assert.match(f.root.innerHTML,/归档主播<\/dt><dd>3/);
  assert.equal(validateBackupSummary(summary).counts.monitored_anchors,summary.counts.anchors);
  assert.throws(()=>validateBackupSummary({...summary,counts:{...summary.counts,monitored_anchors:-1}}),/数量/);
  f.controller.unmount();
});

test("a valid preview displays its counts but incremental import requires an explicit confirm action",async()=>{
  let restored=0;
  const f=fixture(async command=>command==="data_backup_inspect"?{...inspected(),file_name:"<script>备份</script>.qkbackup"}:
    command==="data_backup_restore"?{restored:true,preferences:{},before_import_path:"C:/恢复前.qkbackup"}:{discarded:true},
    {onRestored:async()=>{restored++;}});
  await f.controller.action(button("inspect"));
  assert.match(f.root.innerHTML,/增量合并备份，保留已有记录与设置/);assert.match(f.root.innerHTML,/确认增量导入/);
  assert.match(f.root.innerHTML,/26\/10\/26/);assert.match(f.root.innerHTML,/12,345/);assert.match(f.root.innerHTML,/&lt;script&gt;/);
  assert.doesNotMatch(f.root.innerHTML,/<script>/);assert.equal(restored,0);
  assert.equal(f.calls.some(([command])=>command==="data_backup_restore"),false);
  await f.controller.action(button("restore"));assert.equal(restored,1);
  assert.match(f.root.innerHTML,/导入前备份：C:\/恢复前/);
  await f.controller.action(button("restore"));assert.equal(f.calls.filter(([command])=>command==="data_backup_restore").length,1);
  f.controller.unmount();
});

test("duplicate export and restore clicks cannot start overlapping native operations",async()=>{
  const exporting=deferred(),restoring=deferred();let refreshed=0;
  const f=fixture(async command=>command==="data_backup_export"?exporting.promise:command==="data_backup_inspect"?inspected():restoring.promise,
    {onRestored:async()=>{refreshed++;}});
  const first=f.controller.action(button("export"));await f.controller.action(button("export"));
  assert.equal(f.calls.length,1);assert.match(f.root.innerHTML,/正在导出备份/);assert.match(f.root.innerHTML,/data-action="data-backup-inspect" disabled/);
  exporting.resolve({path:"C:/backup.qkbackup",bytes:9,summary});await first;
  await f.controller.action(button("inspect"));
  const restore=f.controller.action(button("restore"));await f.controller.action(button("restore"));await f.controller.action(button("inspect"));
  assert.equal(f.calls.filter(([command])=>command==="data_backup_restore").length,1);
  restoring.resolve({restored:true,preferences:{"livescope.appearance.font":"chill"},rollback_path:"C:/before.qkbackup"});await restore;
  assert.equal(refreshed,1);assert.equal(f.storage.getItem("livescope.appearance.font"),"chill");
  f.controller.unmount();
});

test("inspect errors can retry and cancelling a preview discards its token without restoring",async()=>{
  let attempts=0;
  const f=fixture(async command=>{if(command==="data_backup_inspect"){if(!attempts++)throw Error("备份损坏");return inspected();}return {discarded:true};});
  await f.controller.action(button("inspect"));assert.match(f.root.innerHTML,/备份损坏/);
  await f.controller.action(button("inspect"));assert.match(f.root.innerHTML,/待导入备份/);assert.doesNotMatch(f.root.innerHTML,/备份损坏/);
  await f.controller.action(button("cancel"));
  assert.deepEqual(f.calls.at(-1),["data_backup_discard",{token:"backup-token"}]);assert.doesNotMatch(f.root.innerHTML,/确认增量导入/);
  await f.controller.action(button("restore"));assert.equal(f.calls.some(([command])=>command==="data_backup_restore"),false);
  f.controller.unmount();
});

test("an expired restore token removes confirmation and requires a new preview",async()=>{
  const f=fixture(async command=>{if(command==="data_backup_inspect")return inspected();if(command==="data_backup_restore")throw Error("备份预览已过期，请重新选择");return {};});
  await f.controller.action(button("inspect"));await f.controller.action(button("restore"));
  assert.match(f.root.innerHTML,/过期/);assert.doesNotMatch(f.root.innerHTML,/确认增量导入/);
  await f.controller.action(button("restore"));assert.equal(f.calls.filter(([command])=>command==="data_backup_restore").length,1);
  f.controller.unmount();
});

test("a late inspect response is discarded after leaving the settings page",async()=>{
  const pending=deferred();const f=fixture(async command=>command==="data_backup_inspect"?pending.promise:{});
  const choosing=f.controller.action(button("inspect"));f.controller.unmount();f.root.innerHTML="其他页面";
  pending.resolve(inspected("late-token"));await choosing;
  assert.equal(f.root.innerHTML,"其他页面");assert.deepEqual(f.calls.at(-1),["data_backup_discard",{token:"late-token"}]);
});

test("a committed restore still refreshes when localStorage is unwritable and never retries data restoration",async()=>{
  let refreshed=0;
  const storage=memoryStorage();storage.setItem=()=>{throw Error("quota");};
  const f=fixture(async command=>command==="data_backup_inspect"?inspected():{restored:true,preferences:{"livescope.appearance.font":"chill"},startup_error:"监听连接失败"},
    {storage,onRestored:async()=>{refreshed++;}});
  await f.controller.action(button("inspect"));await f.controller.action(button("restore"));
  assert.equal(refreshed,1);assert.match(f.root.innerHTML,/数据已导入，界面偏好未合并/);assert.match(f.root.innerHTML,/监听未能自动恢复/);
  assert.doesNotMatch(f.root.innerHTML,/确认增量导入/);assert.ok(f.notices.some(([message])=>message.includes("界面偏好未合并")));
  await f.controller.action(button("restore"));assert.equal(f.calls.filter(([command])=>command==="data_backup_restore").length,1);
  f.controller.unmount();
});

test("refresh failure offers only a refresh retry after data has already committed",async()=>{
  let refreshes=0;
  const f=fixture(async command=>command==="data_backup_inspect"?inspected():{restored:true,preferences:{}},
    {onRestored:async()=>{if(!refreshes++)throw Error("刷新失败");}});
  await f.controller.action(button("inspect"));await f.controller.action(button("restore"));
  assert.match(f.root.innerHTML,/数据已导入，刷新未完成/);assert.match(f.root.innerHTML,/data-backup-refresh/);
  await f.controller.action(button("refresh"));assert.equal(refreshes,2);assert.equal(f.calls.filter(([command])=>command==="data_backup_restore").length,1);
  f.controller.unmount();
});

test("preview backup commands remain in memory and tokens are one-use",async()=>{
  const mock=createPreviewBackup({version:"0.4.3",now:()=>summary.created_at,getCounts:()=>summary.counts});
  const exported=await mock.handle("data_backup_export",{preferences:{"livescope.appearance.font":"chill"}});
  assert.equal(exported.preview,true);assert.match(exported.path,/未写入/);
  const preview=await mock.handle("data_backup_inspect");assert.deepEqual(preview.summary,summary);
  assert.match(preview.file_name,/\.zip$/);
  const result=await mock.handle("data_backup_restore",{token:preview.token});assert.equal(result.restored,true);assert.equal(result.preferences["livescope.appearance.font"],"chill");
  await assert.rejects(mock.handle("data_backup_restore",{token:preview.token}),/过期/);
  const cancelled=await mock.handle("data_backup_inspect");await mock.handle("data_backup_discard",{token:cancelled.token});
  await assert.rejects(mock.handle("data_backup_restore",{token:cancelled.token}),/过期/);
  assert.equal(await mock.handle("not-a-backup-command"),undefined);
});

test("incremental preferences keep existing values and roll back all additions if a later write fails",()=>{
  const initial={"livescope.appearance.font":"chill","livescope.replay-offset.1:2":"invalid-old-value","unrelated":"kept"};
  const storage=memoryStorage(initial),write=storage.setItem;let writes=0;
  storage.setItem=(key,value)=>{write(key,value);if(++writes===2)throw Error("quota");};
  assert.throws(()=>applyBackupPreferences({"livescope.appearance.font":"douyin","livescope.updates.automatic":"true","livescope.replay-offset.3:4":"10"},storage),/合并失败/);
  assert.deepEqual(storage.entries(),initial);
  storage.setItem=write;
  const final=applyBackupPreferences({"livescope.replay-offset.1:2":"7","livescope.replay-offset.3:4":"10"},storage);
  assert.equal(storage.getItem("livescope.replay-offset.1:2"),"invalid-old-value");
  assert.deepEqual(final,{"livescope.appearance.font":"chill","livescope.replay-offset.3:4":"10"});
});

test("import refresh receives final effective preferences instead of imported overrides",async()=>{
  let effective;
  const storage=memoryStorage({"livescope.appearance.font":"chill","livescope.replay-offset.5:6":"4","secret":"kept"});
  const f=fixture(async command=>command==="data_backup_inspect"?inspected():{restored:true,preferences:{"livescope.appearance.font":"douyin","livescope.updates.automatic":"false"}},
    {storage,onRestored:async preferences=>{effective=preferences;}});
  await f.controller.action(button("inspect"));await f.controller.action(button("restore"));
  assert.deepEqual(effective,{"livescope.appearance.font":"chill","livescope.replay-offset.5:6":"4","livescope.updates.automatic":"false"});
  assert.equal(storage.getItem("secret"),"kept");f.controller.unmount();
});

test("failed preference merge refreshes with preserved local values",async()=>{
  let effective;const storage=memoryStorage({"livescope.appearance.font":"chill"});
  storage.setItem=()=>{throw Error("quota");};
  const f=fixture(async command=>command==="data_backup_inspect"?inspected():{restored:true,preferences:{"livescope.appearance.font":"douyin","livescope.updates.automatic":"true"}},
    {storage,onRestored:async preferences=>{effective=preferences;}});
  await f.controller.action(button("inspect"));await f.controller.action(button("restore"));
  assert.deepEqual(effective,{"livescope.appearance.font":"chill"});assert.match(f.root.innerHTML,/界面偏好未合并/);f.controller.unmount();
});

const backupAnchors=[{anchor_id:"7698991795467832114",name:"末末大王",sessions:8,messages:12345},{anchor_id:"room:111002",name:"小天 <直播>",sessions:2,messages:45}];
test("anchor list keeps opaque stable IDs, escapes labels and rejects ambiguous duplicates",()=>{
  assert.equal(validateBackupAnchors({anchors:backupAnchors})[1].anchor_id,"room:111002");
  assert.throws(()=>validateBackupAnchors({anchors:[...backupAnchors,backupAnchors[0]]}),/列表无效/);
  assert.throws(()=>validateBackupAnchors({anchors:[{anchor_id:7698991795467832114}]}),/列表无效/);
  assert.throws(()=>validateBackupAnchors({anchors:[{anchor_id:"bad\nvalue"}]}),/列表无效/);
});

test("all-anchor export is default and selected exports block an empty scope",async()=>{
  const f=fixture(async command=>command==="data_backup_anchors"?{anchors:backupAnchors}:{path:"C:/backup.zip",summary});
  await f.controller.action(button("export"));
  assert.deepEqual(f.calls[0],["data_backup_export",{preferences:{},anchorIds:null}]);
  await f.controller.action(button("scope-selected"));
  assert.match(f.root.innerHTML,/小天 &lt;直播&gt;/);
  await f.controller.action(button("export"));
  assert.match(f.root.innerHTML,/请至少选择一位主播/);assert.equal(f.calls.filter(([command])=>command==="data_backup_export").length,1);
  await f.controller.action(button("anchor-all"));await f.controller.action(button("export"));
  assert.deepEqual(f.calls.at(-1),["data_backup_export",{preferences:{},anchorIds:backupAnchors.map(anchor=>anchor.anchor_id)}]);
  await f.controller.action(button("anchor-clear"));await f.controller.action(button("export"));
  assert.match(f.root.innerHTML,/请至少选择一位主播/);assert.equal(f.calls.filter(([command])=>command==="data_backup_export").length,2);f.controller.unmount();
});

test("search and checkbox changes preserve input nodes, selection and export IDs across rerenders and remounts",async()=>{
  const f=fixture(async command=>command==="data_backup_anchors"?{anchors:backupAnchors}:{path:"C:/backup.zip",summary});
  await f.controller.action(button("scope-selected"));
  const initialShell=f.root.innerHTML,list={innerHTML:"",scrollTop:56},count={textContent:""};
  f.root.querySelector=selector=>selector==="#data-backup-anchor-list"?list:selector==="#data-backup-anchor-count"?count:null;
  f.listeners.get("input")({target:{id:"data-backup-anchor-search",value:"小天"}});
  assert.equal(f.root.innerHTML,initialShell);assert.match(list.innerHTML,/小天/);assert.doesNotMatch(list.innerHTML,/末末/);assert.equal(list.scrollTop,56);
  f.listeners.get("change")({target:{dataset:{backupAnchor:"room:111002"},checked:true}});
  assert.equal(f.root.innerHTML,initialShell);assert.equal(count.textContent,"已选择 1 / 2 位主播");
  await f.controller.action(button("export"));
  assert.deepEqual(f.calls.at(-1)[1].anchorIds,["room:111002"]);assert.match(f.root.innerHTML,/value="小天"/);assert.match(f.root.innerHTML,/data-backup-anchor="room:111002" checked/);
  await f.controller.action(button("scope-all"));await f.controller.action(button("scope-selected"));
  assert.match(f.root.innerHTML,/value="小天"/);assert.match(f.root.innerHTML,/data-backup-anchor="room:111002" checked/);
  f.controller.unmount();assert.equal(f.listeners.size,0);f.controller.mount(f.root);
  await f.controller.action(button("export"));assert.deepEqual(f.calls.at(-1)[1].anchorIds,["room:111002"]);f.controller.unmount();
});

test("anchor loading can retry and a late response cannot overwrite a new settings mount",async()=>{
  let attempts=0;
  const loading=deferred(),f=fixture(async command=>{if(command==="data_backup_anchors"){if(!attempts++)throw Error("主播读取失败");return loading.promise;}return {};});
  await f.controller.action(button("scope-selected"));assert.match(f.root.innerHTML,/主播读取失败/);
  const reading=f.controller.action(button("anchors-retry"));f.controller.unmount();f.root.innerHTML="其他页面";
  loading.resolve({anchors:backupAnchors});await reading;assert.equal(f.root.innerHTML,"其他页面");
  f.controller.mount(f.root);assert.doesNotMatch(f.root.innerHTML,/末末大王/);f.controller.unmount();
});

test("preview selected export filters calibration preferences and retains the selected scope summary",async()=>{
  const mock=createPreviewBackup({version:"0.4.5",now:()=>summary.created_at,getCounts:()=>summary.counts,getAnchors:()=>backupAnchors});
  assert.equal((await mock.handle("data_backup_anchors")).anchors[1].anchor_id,"room:111002");
  await assert.rejects(mock.handle("data_backup_export",{preferences:{},anchorIds:[]}),/至少选择/);
  await assert.rejects(mock.handle("data_backup_export",{preferences:{},anchorIds:["unknown"]}),/至少选择/);
  const exported=await mock.handle("data_backup_export",{preferences:{"livescope.appearance.font":"chill","livescope.replay-offset.7698991795467832114:123":"4","livescope.replay-offset.5:6":"7"},anchorIds:["7698991795467832114"]});
  assert.equal(exported.summary.counts.anchors,1);assert.equal(exported.summary.counts.messages,12345);assert.equal(exported.summary.counts.sessions,8);
  const inspected=await mock.handle("data_backup_inspect"),restored=await mock.handle("data_backup_restore",{token:inspected.token});
  assert.deepEqual(restored.preferences,{"livescope.appearance.font":"chill","livescope.replay-offset.7698991795467832114:123":"4"});
});

test("successful incremental import invalidates the roster and prunes only selections that disappeared",async()=>{
  let anchors=backupAnchors;
  const added={anchor_id:"new-imported-anchor",name:"刚导入的主播",sessions:1,messages:8};
  const f=fixture(async command=>{
    if(command==="data_backup_anchors")return {anchors};
    if(command==="data_backup_inspect")return inspected();
    if(command==="data_backup_restore"){anchors=[backupAnchors[1],added];return {restored:true,preferences:{}};}
    return {path:"C:/backup.zip",summary};
  });
  await f.controller.action(button("scope-selected"));await f.controller.action(button("anchor-all"));
  await f.controller.action(button("inspect"));await f.controller.action(button("restore"));
  await f.controller.action(button("export"));assert.match(f.root.innerHTML,/请先读取主播列表/);
  await f.controller.action(button("scope-selected"));
  assert.equal(f.calls.filter(([command])=>command==="data_backup_anchors").length,2);assert.match(f.root.innerHTML,/刚导入的主播/);
  assert.match(f.root.innerHTML,/data-backup-anchor="room:111002" checked/);assert.doesNotMatch(f.root.innerHTML,/data-backup-anchor="new-imported-anchor" checked/);
  await f.controller.action(button("export"));assert.deepEqual(f.calls.at(-1)[1].anchorIds,["room:111002"]);f.controller.unmount();
});

test("remount after import automatically reloads the selected roster",async()=>{
  let reads=0;
  const reloaded=deferred(),f=fixture(async command=>{
    if(command==="data_backup_anchors")return ++reads===1?{anchors:backupAnchors}:reloaded.promise;
    if(command==="data_backup_inspect")return inspected();
    return {restored:true,preferences:{}};
  });
  await f.controller.action(button("scope-selected"));await f.controller.action(button("anchor-all"));
  await f.controller.action(button("inspect"));await f.controller.action(button("restore"));
  f.controller.unmount();f.controller.mount(f.root);assert.equal(reads,2);
  reloaded.resolve({anchors:[backupAnchors[1],{anchor_id:"just-imported",name:"新主播"}]});
  await new Promise(resolve=>setImmediate(resolve));
  assert.match(f.root.innerHTML,/新主播/);assert.match(f.root.innerHTML,/data-backup-anchor="room:111002" checked/);f.controller.unmount();
});

test("a roster read started before import cannot repopulate stale data after commit",async()=>{
  const stale=deferred();let reads=0;
  const f=fixture(async command=>{
    if(command==="data_backup_anchors")return ++reads===1?stale.promise:{anchors:[{anchor_id:"fresh-anchor",name:"导入后的主播"}]};
    if(command==="data_backup_inspect")return inspected();
    return {restored:true,preferences:{}};
  });
  const reading=f.controller.action(button("scope-selected"));
  await f.controller.action(button("inspect"));await f.controller.action(button("restore"));
  stale.resolve({anchors:backupAnchors});await reading;assert.doesNotMatch(f.root.innerHTML,/末末大王/);
  await f.controller.action(button("scope-selected"));assert.match(f.root.innerHTML,/导入后的主播/);assert.equal(reads,2);f.controller.unmount();
});

test("selected scope accepts 10000 anchors, rejects larger select-all and prevents a 10001st checkbox",async()=>{
  let anchors=Array.from({length:10000},(_,index)=>({anchor_id:`anchor-${index}`,name:`主播 ${index}`}));
  const f=fixture(async command=>command==="data_backup_anchors"?{anchors}:{path:"C:/backup.zip",summary});
  await f.controller.action(button("scope-selected"));await f.controller.action(button("anchor-all"));
  await f.controller.action(button("export"));assert.equal(f.calls.at(-1)[1].anchorIds.length,10000);
  anchors=[...anchors,{anchor_id:"anchor-extra",name:"搜索到的额外主播"}];
  await f.controller.action(button("anchors-retry"));await f.controller.action(button("anchor-all"));
  assert.match(f.root.innerHTML,/一次最多选择10000位主播/);
  const list={innerHTML:"",scrollTop:0},count={textContent:""},error={textContent:""};
  f.root.querySelector=selector=>selector==="#data-backup-anchor-list"?list:selector==="#data-backup-anchor-count"?count:selector==="#data-backup-operation-error"?error:null;
  f.listeners.get("input")({target:{id:"data-backup-anchor-search",value:"额外"}});
  assert.match(list.innerHTML,/搜索到的额外主播/);
  const checkbox={dataset:{backupAnchor:"anchor-extra"},checked:true};f.listeners.get("change")({target:checkbox});
  assert.equal(checkbox.checked,false);assert.match(error.textContent,/一次最多选择10000位主播/);assert.ok(f.notices.some(([message])=>message.includes("10000")));
  await f.controller.action(button("export"));assert.equal(f.calls.at(-1)[1].anchorIds.length,10000);assert.equal(f.calls.at(-1)[1].anchorIds.includes("anchor-extra"),false);f.controller.unmount();
});

test("preview scope rejects more than 10000 IDs with the same clear selection limit",async()=>{
  const mock=createPreviewBackup();
  await assert.rejects(mock.handle("data_backup_export",{preferences:{},anchorIds:Array.from({length:10001},(_,index)=>String(index))}),/一次最多选择10000位主播/);
});

test("backup roster normalizes tags, accepts legacy rows and rejects malformed labels",()=>{
  const rows=validateBackupAnchors({anchors:[{anchor_id:"11",name:"主播",tags:[" 精选 ","精选","😀".repeat(24)]},{anchor_id:"12",name:"旧版主播"}]});
  assert.deepEqual(rows[0].tags,["精选","😀".repeat(24)]);assert.deepEqual(rows[1].tags,[]);
  for(const tags of [[null],["\n"],["a\nb"],["x".repeat(25)],Array.from({length:21},(_,i)=>`标签${i}`)]){
    assert.throws(()=>validateBackupAnchors({anchors:[{anchor_id:"11",tags}]}));
  }
});

test("choosing a backup tag replaces older selections and exports only that tag",async()=>{
  const anchors=[{anchor_id:"11",name:"精选主播",tags:["精选"]},{anchor_id:"12",name:"其它主播",tags:["其它"]},{anchor_id:"13",name:"未标记主播"},{anchor_id:"14",name:"精选另一个",tags:["精选"]}];
  const f=fixture(async command=>command==="data_backup_anchors"?{anchors}:{path:"C:/backup.zip",summary});
  await f.controller.action(button("scope-selected"));await f.controller.action(button("anchor-all"));
  const list={innerHTML:"",scrollTop:10};f.root.querySelector=selector=>selector==="#data-backup-anchor-list"?list:null;
  f.listeners.get("change")({target:{id:"data-backup-anchor-tag",value:"精选"}});
  assert.match(list.innerHTML,/精选主播/);assert.doesNotMatch(list.innerHTML,/其它主播|未标记主播/);
  f.listeners.get("change")({target:{dataset:{backupAnchor:"12"},checked:true}});
  await f.controller.action(button("export"));assert.deepEqual(f.calls.at(-1)[1].anchorIds,["11","14"]);
  f.listeners.get("change")({target:{id:"data-backup-anchor-tag",value:"其它"}});
  await f.controller.action(button("export"));assert.deepEqual(f.calls.at(-1)[1].anchorIds,["12"]);
  f.listeners.get("change")({target:{id:"data-backup-anchor-tag",value:UNTAGGED_TAG}});
  await f.controller.action(button("export"));assert.deepEqual(f.calls.at(-1)[1].anchorIds,["13"]);f.controller.unmount();
});

test("backup select current results intersects name search with the active tag",async()=>{
  const anchors=[{anchor_id:"11",name:"同名甲",tags:["精选"]},{anchor_id:"12",name:"同名乙",tags:["其它"]},{anchor_id:"13",name:"另一个",tags:["精选"]}];
  const f=fixture(async command=>command==="data_backup_anchors"?{anchors}:{path:"C:/backup.zip",summary});
  await f.controller.action(button("scope-selected"));
  f.listeners.get("change")({target:{id:"data-backup-anchor-tag",value:"精选"}});
  f.listeners.get("input")({target:{id:"data-backup-anchor-search",value:"同名"}});
  await f.controller.action(button("anchor-all"));await f.controller.action(button("export"));
  assert.deepEqual(f.calls.at(-1)[1].anchorIds,["11"]);f.controller.unmount();
});

test("a removed tag on roster refresh clears the old selection instead of widening export",async()=>{
  let anchors=[{anchor_id:"11",name:"主播",tags:["精选"]}];
  const f=fixture(async command=>command==="data_backup_anchors"?{anchors}:{path:"C:/backup.zip",summary});
  await f.controller.action(button("scope-selected"));
  f.listeners.get("change")({target:{id:"data-backup-anchor-tag",value:"精选"}});
  anchors=[{anchor_id:"11",name:"主播",tags:["其它"]}];
  await f.controller.action(button("anchors-retry"));await f.controller.action(button("export"));
  assert.match(f.root.innerHTML,/请至少选择一位主播/);assert.equal(f.calls.some(([command])=>command==="data_backup_export"),false);f.controller.unmount();
});
