import test from 'node:test';import assert from 'node:assert/strict';
import {createViewerRefresh} from './viewer-refresh.js';
test('refresh is non-overlapping and progress cannot regress on a stale poll',async()=>{
 let finish,calls=0;const notices=[];const c=createViewerRefresh({bridge:()=>{calls++;return new Promise(r=>finish=r)},notify:s=>notices.push(s)});
 const pending=c.start();await c.start();assert.equal(calls,1);assert.equal(c.snapshot().disabled,true);
 finish({id:'2',accepted:true,active:true,total:50,checked:0});await pending;
 c.update({id:'2',active:true,total:50,checked:25});c.update({id:'1',active:false,total:1,checked:1});c.update({id:'2',active:true,total:50,checked:0});assert.equal(c.snapshot().label,'刷新 25/50');
 c.update({id:'2',active:false,total:50,checked:50,updated:49,failed:1});assert.equal(c.snapshot().disabled,false);assert.match(notices[0],/49.*1/);
});
test('refresh failure re-enables the button and preserves a retry path',async()=>{
 const notices=[];let calls=0;const c=createViewerRefresh({bridge:async()=>{if(++calls===1)throw Error('读取失败');return {id:'1',accepted:true,active:false,total:0}},notify:s=>notices.push(s)});
 await c.start();assert.equal(c.snapshot().disabled,false);assert.equal(notices[0],'读取失败');await c.start();assert.match(notices[1],/暂无/);
});
test('a poll finishing before the start reply produces only one completion notice',async()=>{
 let finish;const notices=[];const c=createViewerRefresh({bridge:()=>new Promise(r=>finish=r),notify:s=>notices.push(s)});
 const pending=c.start();c.update({id:'1',active:true,total:1,checked:0});
 const done={id:'1',active:false,total:1,checked:1,updated:1,accepted:true};c.update(done);finish(done);await pending;
 assert.equal(notices.length,1);
});
