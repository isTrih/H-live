import test from 'node:test';
import assert from 'node:assert/strict';
import {createStoryController,wheelDistance,validateRelease,validateChangelog,formatReleaseDate} from './site.js';

function fixture(options={}) {
  let now=0,sequence=0;
  const timers=new Map(),changes=[],timerStates=[];
  const controller=createStoryController({count:5,now:()=>now,
    setTimer:(fn,delay)=>{const id=++sequence;timers.set(id,{fn,at:now+delay});return id;},
    clearTimer:id=>timers.delete(id),onChange:state=>changes.push(state),onTimer:state=>timerStates.push(state),...options});
  function advance(ms) {
    const end=now+ms;
    while(true){const next=[...timers.entries()].filter(([,timer])=>timer.at<=end).sort((a,b)=>a[1].at-b[1].at)[0];if(!next)break;now=next[1].at;timers.delete(next[0]);next[1].fn();}
    now=end;
  }
  return {controller,changes,timerStates,timers,advance};
}

test('five seconds of inactivity advances one panel and stops on the download panel',()=>{
  const f=fixture();f.advance(4999);assert.equal(f.controller.state().index,0);
  f.advance(1);assert.equal(f.controller.state().index,1);
  f.advance(15000);assert.equal(f.controller.state().index,4);assert.equal(f.timers.size,0);
  f.advance(60000);assert.equal(f.changes.length,4);assert.equal(f.timerStates.at(-1).running,false);
});
test('a continuous trackpad gesture cannot run through multiple panels',()=>{
  const f=fixture();assert.equal(f.controller.wheel(20),false);f.advance(20);
  assert.equal(f.controller.wheel(25),true);assert.equal(f.controller.state().index,1);
  for(let i=0;i<30;i++){f.advance(40);f.controller.wheel(70);}
  assert.equal(f.controller.state().index,1);
  f.advance(200);f.controller.wheel(-60);assert.equal(f.controller.state().index,0);
});
test('scroll input restarts the five second idle period without queuing automatic jumps',()=>{
  const f=fixture();f.advance(4800);f.controller.wheel(1);f.advance(4999);
  assert.equal(f.controller.state().index,0);f.advance(1);assert.equal(f.controller.state().index,1);
});
test('dialog and hidden-page blocks compose and rearm only after both end',()=>{
  const f=fixture();f.advance(4000);f.controller.block('dialog',true);f.controller.block('hidden',true);
  f.advance(30000);assert.equal(f.controller.state().index,0);
  f.controller.block('dialog',false);f.advance(6000);assert.equal(f.controller.state().index,0);
  f.controller.block('hidden',false);f.advance(4999);assert.equal(f.controller.state().index,0);
  f.advance(1);assert.equal(f.controller.state().index,1);
});
test('pause, resume, reduced motion and disposal do not leave stray autoplay timers',()=>{
  const f=fixture({reducedMotion:true});assert.equal(f.timers.size,0);f.advance(9000);assert.equal(f.changes.length,0);
  f.controller.toggle();f.advance(5000);assert.equal(f.controller.state().index,1);
  f.controller.toggle();f.advance(20000);assert.equal(f.controller.state().index,1);
  f.controller.toggle();f.controller.dispose();assert.equal(f.timers.size,0);f.advance(20000);assert.equal(f.changes.length,1);
});
test('manual navigation is bounded and can return from the final download to the first panel',()=>{
  const f=fixture();assert.equal(f.controller.go(-1),false);f.controller.go(99,'keyboard');assert.equal(f.controller.state().index,4);
  f.controller.go(0,'navigation');assert.equal(f.controller.state().index,0);assert.equal(f.timers.size,1);
  assert.equal(f.controller.go(NaN),false);
});
test('wheel input normalizes mouse lines and page deltas and supports horizontal trackpads',()=>{
  assert.equal(wheelDistance({deltaY:3,deltaMode:1}),48);
  assert.equal(wheelDistance({deltaY:-1,deltaMode:2},900),-900);
  assert.equal(wheelDistance({deltaX:90,deltaY:2}),90);
  assert.equal(wheelDistance({deltaY:NaN}),0);
});
test('download targets must match the signed release version under the configured Site',()=>{
  const base='https://qingkan-live.internal.lilithsites.com/',version='0.4.5';
  const manifest=url=>({version,platforms:{'windows-x86_64':{signature:'signature',url}}});
  const url=`${base}${version}/QingkanLive-${version}-windows-x64-setup.exe`;
  assert.deepEqual(validateRelease(manifest(url),base),{version,url});
  for(const bad of [url.replace('https:','http:'),url.replace('qingkan-live.internal.lilithsites.com','evil.example'),url+'?token=x',url+'#fragment',url.replace('/0.4.5/','/0.4.4/'),url.replace('setup.exe','other.exe')])assert.throws(()=>validateRelease(manifest(bad),base));
  assert.throws(()=>validateRelease({version,platforms:{}},base));
});
test('all changelog bodies survive validation while duplicate versions and malformed data are rejected',()=>{
  const entries=[{version:'0.4.5',title:'氢看直播 0.4.5',body:'# 氢看直播 0.4.5\n\n- 完整记录\n<script>alert(1)</script>'},{version:'0.1.0',title:'首次发布',body:'完整的旧版本说明'}];
  assert.equal(validateChangelog({format:1,entries})[0].body,entries[0].body);
  assert.throws(()=>validateChangelog({format:1,entries:[entries[0],entries[0]]}));
  assert.throws(()=>validateChangelog({format:2,entries}));
  assert.throws(()=>validateChangelog({format:1,entries:[{version:'1.2.3',title:'x',body:'x'.repeat(1024*1024+1)}]}));
});
test('release dates use the recorded calendar day without timezone shifts or invented fallback dates',()=>{
  assert.equal(formatReleaseDate('2026-10-26'),'26/10/26');
  assert.equal(formatReleaseDate('2024-02-29'),'24/02/29');
  assert.equal(formatReleaseDate(null),'日期未记录');
  assert.equal(formatReleaseDate(undefined),'日期未记录');
  for(const date of ['2026-02-30','2026-2-3','<script>','2026-10-26T00:00:00Z',123])assert.throws(()=>formatReleaseDate(date));
  assert.throws(()=>validateChangelog({format:1,entries:[{version:'1.2.3',title:'test',body:'body',date:'2026-02-30'}]}));
});
