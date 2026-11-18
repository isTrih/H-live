import test from "node:test";
import assert from "node:assert/strict";
import {replayTimeMap} from "./replay-sync.js";
import {replayClockMetadata,archiveClockMetadata,playbackClock,clockEpoch,clockAxis} from "./replay-clock.js";
import {setDisplayTimezone} from "./time-display.js";
const epoch=value=>Date.parse(value)/1000;
const span=(start,duration,pdt)=>({offset_seconds:start,duration,program_date_time:pdt});

test("raw PDT clock works before archives arrive and ignores all danmaku calibration",t=>{
  setDisplayTimezone("Asia/Shanghai");
  const start=epoch("2026-10-25T06:00:00Z"),replay={time_mapping:[span(0,120,start)]};
  const original=replayTimeMap(replay,null,900),corrected=replayTimeMap(replay,start-30,-400);
  assert.equal(original.dated,false);assert.equal(original.worldDated,true);
  assert.equal(original.toWorldEpoch(45),start+45);assert.equal(corrected.toWorldEpoch(45),start+45);
  assert.equal(playbackClock(replayClockMetadata(original,45,120),45,120).position,"14:00:45");
  assert.deepEqual(playbackClock(replayClockMetadata(original,45,120),45,120),playbackClock(replayClockMetadata(corrected,45,120),45,120));
  assert.equal(corrected.toArchive(45),-325,"chat correction remains independent of clock display");
});

test("a multi-hour HLS timeline transfers only compact continuous clock runs",()=>{
  const start=epoch("2026-10-25T06:00:00Z");
  const map=replayTimeMap({time_mapping:Array.from({length:5447},(_,index)=>span(index*4,4,start+index*4))},null);
  assert.deepEqual(map.clockWindow(15000),[{start:0,end:21788,epoch:start}]);
  const metadata=replayClockMetadata(map,15000,21788);
  assert.ok(JSON.stringify(metadata).length<400);
  assert.equal(clockEpoch(metadata,15000),start+15000);
});

test("PDT jumps and missing media spans do not create invented wall-clock positions",()=>{
  const map=replayTimeMap({time_mapping:[span(0,10,1000),span(20,10,1040)]},990,60);
  assert.equal(map.toWorldEpoch(10),null);assert.equal(map.toWorldEpoch(15),null);
  assert.equal(map.toWorldEpoch(20),1040);assert.equal(map.toWorldEpoch(30),1050);
  const metadata=replayClockMetadata(map,15,30);
  assert.equal(clockEpoch(metadata,10),null);assert.equal(clockEpoch(metadata,15),null);
  assert.equal(playbackClock(metadata,15,30).note,"时间未对齐");
  const partial=replayClockMetadata(map,5,60);
  assert.equal(playbackClock(partial,5,60).end,"—","a missing recording endpoint cannot become an elapsed value presented as a clock");
  assert.equal(clockAxis({duration:30,toEpoch:time=>map.toWorldEpoch(time)})[2].label,"—");
  const jump=replayTimeMap({time_mapping:[span(0,10,1000),span(10,10,1040)]},990);
  assert.equal(jump.toWorldEpoch(10),1040);
  assert.equal(clockEpoch(replayClockMetadata(jump,9.9,20),10),1040);
});

test("cross-midnight clock labels include dates while playback positions remain media seconds",()=>{
  setDisplayTimezone("Asia/Shanghai");
  const start=epoch("2026-10-25T15:59:30Z"),map=replayTimeMap({time_mapping:[span(0,120,start)]},null);
  const labels=playbackClock(replayClockMetadata(map,0,120),0,120);
  assert.equal(labels.position,"26/10/25 23:59:30");assert.equal(labels.end,"26/10/26 00:01:30");
  assert.equal(labels.duration,"时长 00:02:00");
  const axis=clockAxis({duration:120,toEpoch:time=>map.toWorldEpoch(time)});
  assert.equal(axis[0].label,"26/10/25 23:59:30");assert.equal(axis.at(-1).label,"26/10/26 00:01:30");
  assert.deepEqual(axis.map(tick=>tick.offset),[0,30,60,90,120]);
});

test("display timezone and DST changes alter presentation without changing PDT or media offsets",t=>{
  t.after(()=>setDisplayTimezone("Asia/Shanghai"));
  const start=epoch("2026-11-01T05:30:00Z"),map=replayTimeMap({time_mapping:[span(0,3600,start)]},start);
  setDisplayTimezone("America/New_York");
  const labels=playbackClock(replayClockMetadata(map,0,3600),0,3600);
  assert.equal(labels.position,"01:30:00");assert.equal(labels.end,"01:30:00");
  assert.equal(labels.duration,"时长 01:00:00");
  assert.equal(map.toWorldEpoch(3600),start+3600);assert.equal(map.toMedia(1800),1800);
  setDisplayTimezone("Asia/Tokyo");
  assert.equal(playbackClock(replayClockMetadata(map,0,3600),0,3600).position,"14:30:00");
});

test("undated video stays unaligned even when the chat archive has a known start",()=>{
  setDisplayTimezone("Asia/Shanghai");
  const start=epoch("2026-10-25T06:00:00Z"),map=replayTimeMap({},start,100);
  const labels=playbackClock(replayClockMetadata(map,45,120),45,120);
  assert.equal(labels.position,"00:00:45");assert.equal(labels.end,"00:02:00");assert.equal(labels.note,"时间未对齐");
  assert.equal(playbackClock(archiveClockMetadata(start,120),45,120).position,"14:00:45");
  assert.equal(playbackClock(archiveClockMetadata(null,120),45,120).note,"时间未对齐");
});
