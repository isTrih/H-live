import test from "node:test";
import assert from "node:assert/strict";
import {createRecentMessages} from "./recent-messages.js";

test("rare types read persisted matches rather than the entry-flood cache",async()=>{
 const archived=["gift","activity","emoji","fansclub","social"].map((type,i)=>({archive_id:String(i+1),message_type:type,content:`target-${type}`}));
 const ipc=Array.from({length:1500},()=>({message_type:"member"})).slice(-1200);
 assert.equal(ipc.filter(m=>m.message_type==="gift").length,0);
 const filter={active:true,type:"gift",web_rid:"111"};const calls=[];
 const controller=createRecentMessages({getFilter:()=>filter,bridge:async(method,p)=>{calls.push([method,p]);return {messages:archived.filter(m=>m.message_type===p.type)};}});
 for(const type of ["gift","activity","emoji","fansclub","social"]){filter.type=type;await controller.refresh();assert.equal(controller.snapshot().rows[0].content,`target-${type}`);}
 assert.equal(calls.length,5);assert.equal(calls[0][1].limit,150);
});
test("changing filters discards slow replies and cannot show rows from previous type",async()=>{
 let complete;const filter={active:true,type:"gift"};
 const controller=createRecentMessages({getFilter:()=>filter,bridge:async(_m,p)=>p.type==="gift"?new Promise(r=>complete=r):{messages:[{message_type:"activity"}]}});
 const old=controller.refresh();filter.type="activity";assert.deepEqual(controller.snapshot().rows,[]);
 await controller.refresh();complete({messages:[{message_type:"gift"}]});await old;
 assert.equal(controller.snapshot().rows[0].message_type,"activity");
});
test("pause and inactive pages do not poll; deletion invalidates in-flight old data",async()=>{
 let complete;let calls=0;const filter={active:true,type:"all",paused:true};
 const controller=createRecentMessages({getFilter:()=>filter,bridge:async()=>{calls++;return new Promise(r=>complete=r);}});
 await controller.refresh();assert.equal(calls,0);filter.paused=false;
 const pending=controller.refresh();controller.invalidate();complete({messages:[{content:"deleted"}]});await pending;
 assert.deepEqual(controller.snapshot().rows,[]);filter.active=false;await controller.refresh();assert.equal(calls,1);
});
test("failed refresh keeps prior matching rows, explains failure and retries without overlap",async()=>{
 let tick=10000,calls=0;const filter={active:true,type:"gift"};
 const controller=createRecentMessages({getFilter:()=>filter,now:()=>tick,bridge:async()=>{calls++;if(calls===2)throw Error("读取失败");return {messages:[{content:"gift"}]};}});
 await controller.refresh();await controller.refresh();assert.equal(calls,1);tick+=2000;await controller.refresh();
 assert.equal(controller.snapshot().error,"读取失败");assert.equal(controller.snapshot().rows[0].content,"gift");
 tick+=2000;await controller.refresh();assert.equal(controller.snapshot().error,"");assert.equal(calls,3);
});

test("paused display can switch types without including events received after pause",async()=>{
 const filter={active:true,type:"all",paused:false};const rows=[{archive_id:"1",message_type:"gift"},{archive_id:"2",message_type:"member"}];const calls=[];
 const controller=createRecentMessages({getFilter:()=>filter,bridge:async(_m,p)=>{calls.push(p);return {high_watermark:String(rows.length),messages:rows.filter(r=>(p.type==="all"||p.type===r.message_type)&&(!p.before_id||BigInt(r.archive_id)<=BigInt(p.before_id))).slice().reverse()};}});
 await controller.refresh();filter.paused=true;controller.setPaused(true);
 rows.push({archive_id:"3",message_type:"gift"});filter.type="gift";await controller.refresh();
 assert.deepEqual(controller.snapshot().rows.map(r=>r.archive_id),["1"]);assert.equal(calls.at(-1).before_id,"2");
 await controller.refresh();assert.equal(calls.length,2);
 filter.paused=false;controller.setPaused(false);await controller.refresh();
 assert.deepEqual(controller.snapshot().rows.map(r=>r.archive_id),["3","1"]);assert.equal(calls.at(-1).before_id,undefined);
});
