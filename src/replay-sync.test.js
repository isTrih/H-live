import test from "node:test";
import assert from "node:assert/strict";
import {replayTimeMap} from "./replay-sync.js";

test("pending recording timing never guesses archive offsets or calibrated wall-clock alignment",()=>{
  const map=replayTimeMap({time_mapping_pending:true,time_mapping:[]},1000,12);
  assert.equal(map.dated,false);assert.equal(map.worldDated,false);
  assert.equal(map.toArchive(0),null);assert.equal(map.toArchive(20),null);
  assert.equal(map.toMedia(20),null);assert.equal(map.toWorldEpoch(0),null);
});
test("PDT maps each span separately and never interpolates across a recording gap",()=>{
  const map=replayTimeMap({time_mapping:[{offset_seconds:0,duration:10,program_date_time:1000},{offset_seconds:10,duration:10,program_date_time:1040}]},990);
  assert.equal(map.toArchive(5),15);assert.equal(map.toArchive(15),55);
  assert.equal(map.toMedia(55),15);assert.equal(map.toMedia(30),null);
});
test("undated recordings use explicit user calibration and preserve unknown times",()=>{
  const map=replayTimeMap({},1000,12);assert.equal(map.dated,false);
  assert.equal(map.toArchive(0),12);assert.equal(map.toMedia(20),8);assert.equal(map.toArchive(NaN),null);
});

test("extrapolated timing is identified without filling a fresh PDT wall-clock gap",()=>{
  const map=replayTimeMap({time_mapping:[
    {offset_seconds:0,duration:10,program_date_time:1000,time_source:"pdt"},
    {offset_seconds:10,duration:10,program_date_time:1010,time_source:"extrapolated",discontinuity:1},
    {offset_seconds:20,duration:10,program_date_time:1040,time_source:"pdt",discontinuity:2},
  ]},990);
  assert.equal(map.estimated,true);
  assert.equal(map.toArchive(15),25);
  assert.equal(map.toMedia(25),15);
  assert.equal(map.toMedia(35),null);
  assert.equal(map.toMedia(55),25);
  assert.equal(replayTimeMap({},990).estimated,false);
});

test("span lookup preserves original precedence for overlapping or unsorted timestamps",()=>{
  const map=replayTimeMap({time_mapping:[
    {offset_seconds:10,duration:20,program_date_time:1020},
    {offset_seconds:0,duration:20,program_date_time:1000},
    {offset_seconds:30,duration:10,program_date_time:1010},
  ]},990);
  assert.equal(map.toArchive(15),35,"the first declared overlapping media span wins");
  assert.equal(map.toMedia(25),15,"the first declared overlapping wall-clock span wins");
  assert.equal(map.toArchive(40),30,"the final declared endpoint remains included");
  assert.equal(map.toMedia(30),10,"an existing span takes precedence over the final endpoint fallback");
});
