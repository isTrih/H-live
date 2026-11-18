import test from "node:test";
import assert from "node:assert/strict";
import {replayProgressData} from "./replay-progress.js";
import {replayTimeMap} from "./replay-sync.js";

test("video progress maps density counts and keeps activity and highlight action indices", () => {
  const data = {
    buckets:[{offset_seconds:30,count:12},{offset_seconds:10,count:3}],
    activities:[{id:"later",offset_seconds:30},{id:"unknown",offset_seconds:null},{id:"earlier",offset_seconds:10}],
  };
  const highlights = [{title:"outside",offset_seconds:2},{title:"inside",offset_seconds:20}];
  const mapped = replayProgressData(data,highlights,replayTimeMap({},1000,5),30);
  assert.deepEqual(mapped.buckets,[{offset_seconds:5,count:3},{offset_seconds:25,count:12}]);
  assert.deepEqual(mapped.activities.map(value=>[value.id,value.offset_seconds]),[["later",25],["unknown",null],["earlier",5]]);
  assert.deepEqual(mapped.highlights,[{title:"inside",offset_seconds:15,highlight_index:1}]);
  assert.equal(data.buckets[0].offset_seconds,30);
  assert.equal(data.activities[1].offset_seconds,null);
  assert.equal(highlights[1].highlight_index,undefined);
});

test("recording gaps drop unmapped density and highlights while retaining activity slots", () => {
  const map = replayTimeMap({time_mapping:[{offset_seconds:0,duration:10,program_date_time:1000},{offset_seconds:10,duration:10,program_date_time:1040}]},990);
  const result = replayProgressData({
    buckets:[{offset_seconds:15,count:2},{offset_seconds:30,count:99},{offset_seconds:55,count:4}],
    activities:[{offset_seconds:30},{offset_seconds:55}],
  },[{offset_seconds:30},{offset_seconds:55}],map,20);
  assert.deepEqual(result.buckets,[{offset_seconds:5,count:2},{offset_seconds:15,count:4}]);
  assert.deepEqual(result.activities,[{offset_seconds:null},{offset_seconds:15}]);
  assert.deepEqual(result.highlights,[{offset_seconds:15,highlight_index:1}]);
});

test("unknown and out-of-range media positions never become markers at zero", () => {
  const offsets = [null,undefined,NaN,-1,0,10,11,Infinity];
  const data = {activities:offsets.map(offset_seconds=>({offset_seconds}))};
  const map = {toMedia:value=>value};
  assert.deepEqual(replayProgressData(data,[],map,10).activities.map(value=>value.offset_seconds),[null,null,null,null,0,10,null,null]);
  assert.ok(replayProgressData(data,[],map,NaN).activities.every(value=>value.offset_seconds===null));
  assert.deepEqual(replayProgressData(),{buckets:[],activities:[],highlights:[]});
});
test("activity interval ends share the media clock and clip to the recording boundary",()=>{
  const result=replayProgressData({activities:[{offset_seconds:20,end_offset_seconds:90}]},[],replayTimeMap({},1000,10),60);
  assert.deepEqual(result.activities,[{offset_seconds:10,end_offset_seconds:60}]);
});

test("density keeps a known chat when its archive bucket starts before the recording",()=>{
  const map=replayTimeMap({time_mapping:[{offset_seconds:0,duration:5,program_date_time:1035}]},1000);
  const result=replayProgressData({
    bucket_seconds:60,buckets:[{offset_seconds:0,count:1}],
    messages:[{message_type:"chat",time_source:"server",offset_seconds:35.5}],
  },[],map,5);
  assert.deepEqual(result.buckets,[{offset_seconds:0,count:1}]);
});

test("density bins individual chat times on the media clock without counting recording gaps",()=>{
  const map=replayTimeMap({time_mapping:[{offset_seconds:0,duration:10,program_date_time:1000},{offset_seconds:10,duration:10,program_date_time:1040}]},990);
  const result=replayProgressData({
    bucket_seconds:10,buckets:[{offset_seconds:10,count:99},{offset_seconds:50,count:99}],
    messages:[
      {message_type:"chat",time_source:"server",offset_seconds:15},
      {message_type:"chat",time_source:"server",offset_seconds:30},
      {message_type:"chat",time_source:"server",offset_seconds:55},
      {message_type:"chat",time_source:"missing",offset_seconds:55},
      {message_type:"chat",time_source:"server",offset_seconds:null},
      {message_type:"gift",time_source:"server",offset_seconds:55},
    ],
  },[],map,20);
  assert.deepEqual(result.buckets,[{offset_seconds:0,count:1},{offset_seconds:10,count:1}]);
});

test("an exact recording-end chat belongs to the final visible media bucket",()=>{
  const result=replayProgressData({bucket_seconds:10,messages:[
    {message_type:"chat",offset_seconds:20},
    {message_type:"chat",offset_seconds:21},
  ]},[],replayTimeMap({},1000),20);
  assert.deepEqual(result.buckets,[{offset_seconds:10,count:1}]);
});
