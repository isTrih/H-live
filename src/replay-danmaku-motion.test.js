import test from "node:test";
import assert from "node:assert/strict";
import {danmakuProgress,syncDanmakuMotion,isCurrentReplaySnapshot} from "./replay-danmaku-motion.js";

test("repeated fixed snapshots never rewind a moving comment at 250ms IPC boundaries",()=>{
  let motion=null,previousX=Infinity;
  for(let now=0;now<=3000;now+=10){
    if(now%250===0)motion=syncDanmakuMotion(motion,{left:"65%"},{now,paused:false,revision:1});
    const x=1000-danmakuProgress(motion,now)*1200;
    assert.ok(x<=previousX+.00001,`comment jumped right at ${now}ms: ${previousX} -> ${x}`);previousX=x;
  }
  assert.ok(previousX<350,"a fixed transport snapshot does not freeze the local animation");
});
test("50ms source sampling and jittered 250ms IPC stay continuous",()=>{
  let motion=null,last=0,maxJump=0;
  for(let now=0;now<=3000;now+=10){
    if(now%250===0){const sampled=Math.max(0,Math.floor((now-(now%500?45:0))/50)*50);const p=.2+sampled/12000;motion=syncDanmakuMotion(motion,{left:`${(1-p)*100}%`},{now,paused:false,revision:1});}
    const p=danmakuProgress(motion,now);if(now){assert.ok(p>=last-.000001,`backwards at ${now}`);maxJump=Math.max(maxJump,p-last);}last=p;
  }
  assert.ok(maxJump<.002,"a transport refresh is no larger than a normal animation step");
});
test("pause freezes once and resume continues without returning to a stale source sample",()=>{
  let motion=syncDanmakuMotion(null,{left:"65%"},{now:0,paused:false,revision:1});
  const atPause=danmakuProgress(motion,240);
  motion=syncDanmakuMotion(motion,{left:"65%"},{now:250,paused:true,revision:1});
  assert.ok(motion.progress>=atPause);assert.equal(danmakuProgress(motion,3000),motion.progress);
  motion=syncDanmakuMotion(motion,{left:"65%"},{now:3000,paused:false,revision:1});
  assert.ok(danmakuProgress(motion,3200)>atPause);
});
test("a short explicit seek resets a surviving comment once, then stale percentages cannot pull it back",()=>{
  let motion=syncDanmakuMotion(null,{left:"60%"},{now:0,paused:false,revision:1});
  motion=syncDanmakuMotion(motion,{left:"59%"},{now:250,paused:true,revision:2});
  assert.ok(Math.abs(motion.progress-.41)<1e-10);
  motion=syncDanmakuMotion(motion,{left:"60%"},{now:500,paused:true,revision:2});assert.ok(Math.abs(motion.progress-.41)<1e-10);
  motion=syncDanmakuMotion(motion,{left:"59%"},{now:750,paused:false,revision:2});assert.ok(Math.abs(motion.progress-.41)<1e-10);
});
test("late packets from before a seek or an older transport sample are discarded",()=>{
  const latest={sequence:14,danmakuRevision:3};
  assert.equal(isCurrentReplaySnapshot({sequence:13,danmakuRevision:3},latest),false);
  assert.equal(isCurrentReplaySnapshot({sequence:16,danmakuRevision:2},latest),false);
  assert.equal(isCurrentReplaySnapshot({sequence:15,danmakuRevision:3},latest),true);
});
