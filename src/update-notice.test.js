import test from "node:test";
import assert from "node:assert/strict";
import {createUpdateNotice} from "./update-notice.js";
test("available update opens a dialog once per version, deferring while another dialog is open", () => {
  let open = false; const notices=[];
  const notice=createUpdateNotice({canOpen:()=>open,show:value=>notices.push(value.version)});
  const update={status:"available",canInstall:true,version:"0.3.7"};
  notice.observe(update);assert.deepEqual(notices,[]);
  open=true;notice.flush();notice.observe(update);notice.flush();assert.deepEqual(notices,["0.3.7"]);
  notice.observe({...update,version:"0.3.8"});assert.deepEqual(notices,["0.3.7","0.3.8"]);
});
test("a newer check clears a deferred stale update and failed checks do not open a dialog",()=>{
  let open=false;const notices=[];
  const notice=createUpdateNotice({canOpen:()=>open,show:v=>notices.push(v.version)});
  notice.observe({status:"available",canInstall:true,version:"0.3.7"});
  notice.observe({status:"checking"});open=true;notice.flush();
  notice.observe({status:"error",version:"0.3.7",canInstall:true});assert.deepEqual(notices,[]);
});
