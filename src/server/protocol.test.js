import test from "node:test";
import assert from "node:assert/strict";
import {operations,parameters} from "./protocol.js";
test("browser bridge only exposes known domain operations",()=>{
 for(const name of ["shutdown","prepare_update","backend_request","open_data_folder","window_action","data_backup_export","data_backup_inspect","data_backup_restore","data_backup_discard"])assert.equal(operations[name],undefined);
 assert.equal(new Set(Object.values(operations)).size,Object.keys(operations).length);
});

test('replay timezone and live marks have explicit routes without exposing download filesystem RPC',()=>{
 assert.equal(operations.time_display_settings,'replays/time-display');
 assert.equal(operations.time_display_save,'replays/time-display/save');
 for(const action of ['list','create','update','delete'])assert.equal(operations[`mark_${action}`],`marks/${action}`);
 for(const method of ['clip_job','clip_jobs','download_naming_save','choose_download_directory'])assert.equal(operations[method],undefined);
 assert.deepEqual(parameters('mark_create',{anchor_id:'86341165764',session_id:'7698991795467832114'}),{anchor_id:'86341165764',session_id:'7698991795467832114'});
});
test("diagnostic preferences use an explicit server settings route",()=>{
 assert.equal(operations.save_settings,"settings");
 assert.deepEqual(parameters("save_settings",{debug_logging:true}),{debug_logging:true});
});
test("browser export uses filename rather than sending a server path",()=>{
 const input={output_path:"直播消息-主播甲-2026-10.xlsx",format:"xlsx"};
 assert.deepEqual(parameters("export",input,"id"),{file_name:input.output_path,format:"xlsx"});
 assert.ok(input.output_path);
 assert.deepEqual(parameters("poll",{after:99},"new-instance"),{after:99,instance:"new-instance"});
});
test("session replay and global AI operations have explicit authenticated server routes",()=>{
 assert.equal(operations.session_timeline,"analysis/timeline");
 for(const action of ["settings","configure","models","analyze","status","cancel"])assert.equal(operations[`highlight_${action}`],`highlights/${action}`);
 assert.deepEqual(parameters("highlight_analyze",{anchor_id:"11",session_id:"101"}),{anchor_id:"11",session_id:"101"});
});
test("community replay operations retain their explicit admin routes and session identifiers",()=>{
 assert.equal(operations.community_settings,"community/settings");
 assert.equal(operations.community_configure,"community/configure");
 assert.equal(operations.session_replay,"replays/session");
 assert.equal(operations.session_replay_timing,"replays/session/timing");
 assert.deepEqual(parameters("session_replay",{session_id:"101"}),{session_id:"101"});
 assert.deepEqual(parameters("session_replay_timing",{session_id:"101",play_url:"https://v6-star.xingtu.cn/a.m3u8?synthetic=1"}),{session_id:"101",play_url:"https://v6-star.xingtu.cn/a.m3u8?synthetic=1"});
});
