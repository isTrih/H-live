import test from "node:test";
import assert from "node:assert/strict";
import {operations,parameters} from "./protocol.js";
test("browser bridge only exposes known domain operations",()=>{
 for(const name of ["shutdown","prepare_update","backend_request","open_data_folder","window_action"])assert.equal(operations[name],undefined);
 assert.equal(new Set(Object.values(operations)).size,Object.keys(operations).length);
});
test("browser export uses filename rather than sending a server path",()=>{
 const input={output_path:"直播消息-主播甲-2026-10.xlsx",format:"xlsx"};
 assert.deepEqual(parameters("export",input,"id"),{file_name:input.output_path,format:"xlsx"});
 assert.ok(input.output_path);
 assert.deepEqual(parameters("poll",{after:99},"new-instance"),{after:99,instance:"new-instance"});
});
