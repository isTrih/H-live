import test from "node:test";
import assert from "node:assert/strict";
import {createPlaybackPreferences,normalizePlaybackAudio} from "./playback-preferences.js";

function memoryStorage(){const values=new Map(),writes=[];return {getItem:key=>values.get(key)||null,setItem(key,value){values.set(key,value);writes.push(JSON.parse(value));},values,writes};}

test("new replay audio is audible and read does not write an initialization preference",()=>{
  const storage=memoryStorage(),prefs=createPlaybackPreferences({getStorage:()=>storage});
  assert.deepEqual(prefs.read(),{volume:1,muted:false,lastAudibleVolume:1});
  assert.equal(storage.writes.length,0);
});

test("global audio preference survives controller restart, manual mute and zero-volume unmute",()=>{
  const storage=memoryStorage(),open=()=>createPlaybackPreferences({getStorage:()=>storage});
  const first=open();first.setVolume(.34);first.setMuted(true);
  assert.deepEqual(open().read(),{volume:.34,muted:true,lastAudibleVolume:.34});
  const second=open();second.setMuted(false);second.setVolume(0);
  assert.deepEqual(open().read(),{volume:0,muted:true,lastAudibleVolume:.34});
  const third=open();assert.deepEqual(third.setMuted(false),{volume:.34,muted:false,lastAudibleVolume:.34});
  assert.deepEqual(open().read(),third.read());
  assert.ok(storage.writes.every(value=>Object.keys(value).sort().join(',')==='format,lastAudibleVolume,muted,volume'));
});

test("blocked or corrupt local storage keeps safe audible defaults and memory across modal changes",()=>{
  const denied=createPlaybackPreferences({getStorage(){throw Error('blocked');}});
  denied.setVolume(.43);denied.setMuted(true);
  assert.deepEqual(denied.read(),{volume:.43,muted:true,lastAudibleVolume:.43});
  assert.deepEqual(denied.setMuted(false),{volume:.43,muted:false,lastAudibleVolume:.43});
  for(const value of ['bad-json','{"format":2,"muted":true}','{"format":1,"volume":-3,"muted":"true","lastAudibleVolume":999}']){
    assert.deepEqual(createPlaybackPreferences({getStorage:()=>({getItem:()=>value})}).read(),{volume:1,muted:false,lastAudibleVolume:1});
  }
  assert.deepEqual(normalizePlaybackAudio({volume:0,muted:false,lastAudibleVolume:.2}),{volume:0,muted:true,lastAudibleVolume:.2});
});
