import test from "node:test";
import assert from "node:assert/strict";
import { createAutomaticUpdates, UPDATE_INTERVAL_MS, automaticUpdatesEnabled, saveAutomaticUpdates } from "./automatic-updates.js";
function fixture(check, state = () => ({})) {
  let next, delay, cleared = 0; const notices = [];
  const controller = createAutomaticUpdates({check,getState:state,onAvailable:r=>notices.push(r.version),setTimer:(fn,ms)=>{next=fn;delay=ms;return 1;},clearTimer:()=>{cleared++;next=null;}});
  return {controller,notices,fire:async()=>{const fn=next;next=null;await fn();},delay:()=>delay,cleared:()=>cleared};
}
test("automatic checks run after startup and every hour, notify once per version, and never install",async()=>{
  let calls=0;const f=fixture(async()=>{calls++;return {status:"available",version:"0.2.3"};});
  f.controller.start();f.controller.start();assert.equal(f.delay(),1000);await f.fire();assert.equal(calls,1);assert.equal(f.delay(),UPDATE_INTERVAL_MS);
  await f.fire();assert.equal(calls,2);assert.deepEqual(f.notices,["0.2.3"]);f.controller.stop();assert.equal(f.cleared(),1);
});
test("busy or retained update artifacts are not replaced by an automatic check",async()=>{
  let calls=0;let state={busy:true};const f=fixture(async()=>{calls++;return {};},()=>state);f.controller.start();await f.fire();
  state={canInstall:true};await f.fire();assert.equal(calls,0);assert.equal(f.delay(),UPDATE_INTERVAL_MS);f.controller.stop();
});
test("stop during an in-flight check prevents late notifications and retries survive failure",async()=>{
  let resolve;const f=fixture(()=>new Promise(r=>resolve=r));f.controller.start();const pending=f.fire();f.controller.stop();resolve({status:"available",version:"x"});await pending;assert.deepEqual(f.notices,[]);
  const failed=fixture(async()=>{throw Error("offline");});failed.controller.start();await failed.fire();assert.equal(failed.delay(),UPDATE_INTERVAL_MS);failed.controller.stop();
});
test("automatic update preference defaults on and persists an explicit disable",()=>{
  const map=new Map();const storage={getItem:k=>map.get(k),setItem:(k,v)=>map.set(k,v)};
  assert.equal(automaticUpdatesEnabled(storage),true);saveAutomaticUpdates(false,storage);assert.equal(automaticUpdatesEnabled(storage),false);
  assert.equal(automaticUpdatesEnabled({getItem:()=>{throw Error();}}),true);
});
