import {validateBackupPreferences,validateBackupAnchors} from "./data-backup.js";

// Desktop backup commands remain in memory; no files or native bridge are used.
export function createPreviewBackup({version="预览",now=()=>new Date().toISOString(),getCounts=()=>({monitored_anchors:4,anchors:3,sessions:12,messages:12486,marks:8,downloads:5}),
  getAnchors=()=>[{anchor_id:"10001",name:"小天（塔塔冒险队）",web_rid:"111001",sessions:5,messages:6230,tags:["塔塔冒险队","重点关注"]},{anchor_id:"10002",name:"末末大王玩万龙觉醒",web_rid:"111002",sessions:4,messages:4016,tags:["万龙觉醒"]},{anchor_id:"10003",name:"氢看演示直播间",web_rid:"111003",sessions:3,messages:2240,tags:[]}]}={}){
  let exported=null,sequence=0;
  const tokens=new Map();
  const summary=()=>({format:1,app_version:version,created_at:now(),counts:{...getCounts()}});
  async function handle(command,params={}){
    if(command==="data_backup_anchors")return {anchors:validateBackupAnchors({anchors:getAnchors()}),preview:true};
    if(command==="data_backup_export"){
      const preferences=validateBackupPreferences(params.preferences),result=summary();
      if(params.anchorIds!==undefined&&params.anchorIds!==null){
        if(Array.isArray(params.anchorIds)&&params.anchorIds.length>10000)throw Error("一次最多选择10000位主播");
        const anchors=validateBackupAnchors({anchors:getAnchors()}),available=new Set(anchors.map(anchor=>anchor.anchor_id));
        if(!Array.isArray(params.anchorIds)||!params.anchorIds.length||params.anchorIds.some(id=>!available.has(id))||new Set(params.anchorIds).size!==params.anchorIds.length)throw Error("请至少选择一位有效主播");
        const selected=new Set(params.anchorIds),chosen=anchors.filter(anchor=>selected.has(anchor.anchor_id));
        result.counts.anchors=chosen.length;result.counts.monitored_anchors=Math.min(chosen.length,result.counts.monitored_anchors??chosen.length);
        for(const key of ["sessions","messages"])result.counts[key]=chosen.reduce((sum,anchor)=>sum+(anchor[key]??0),0);
        for(const key of ["marks","downloads"])result.counts[key]=Math.min(result.counts[key],Math.ceil(result.counts[key]*chosen.length/Math.max(1,anchors.length)));
        for(const key of Object.keys(preferences)){const match=/^livescope\.replay-offset\.([^:]+):/.exec(key);if(match&&!selected.has(match[1]))delete preferences[key];}
      }
      exported={preferences,summary:result};
      return {path:"预览备份（未写入磁盘）",bytes:16384,summary:structuredClone(exported.summary),preview:true};
    }
    if(command==="data_backup_inspect"){
      const token=`preview-backup-${++sequence}`;
      const data=exported?structuredClone(exported):{preferences:{"livescope.appearance.font":"douyin"},summary:summary()};
      tokens.set(token,data);
      return {token,file_name:"氢看直播备份（演示）.zip",summary:structuredClone(data.summary),preview:true};
    }
    if(command==="data_backup_restore"){
      const data=tokens.get(params.token);
      if(!data)throw Error("备份预览已过期，请重新选择");
      tokens.delete(params.token);
      return {restored:true,preferences:structuredClone(data.preferences),rollback_path:"预览回滚副本（未写入磁盘）",preview:true};
    }
    if(command==="data_backup_discard"){tokens.delete(params.token);return {discarded:true,preview:true};}
    return undefined;
  }
  return {handle};
}
