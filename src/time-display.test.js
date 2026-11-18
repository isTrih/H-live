import test from "node:test";
import assert from "node:assert/strict";
import {DEFAULT_DISPLAY_TIMEZONE,TIME_DISPLAY_MODE,formatWorldClock,formatWorldTime,getDisplayTimezone,getTimeDisplayPreferences,normalizeDisplayTimezone,setDisplayTimezone,worldDayKey} from "./time-display.js";

test("world-clock display defaults to Shanghai and keeps the 26/10/25 date format",()=>{
  setDisplayTimezone(DEFAULT_DISPLAY_TIMEZONE);
  const epoch=Date.parse("2026-10-24T16:03:04Z")/1000;
  assert.equal(TIME_DISPLAY_MODE,"worldclock");assert.deepEqual(getTimeDisplayPreferences(),{mode:"worldclock",timezone:"Asia/Shanghai"});
  assert.equal(formatWorldTime(epoch),"00:03:04");assert.equal(formatWorldTime(epoch,{date:true}),"26/10/25 00:03:04");
  assert.equal(formatWorldTime(epoch,{seconds:false}),"00:03");assert.equal(worldDayKey(epoch),"2026-10-25");
});

test("timezone changes affect only display preferences and day keys",t=>{
  t.after(()=>setDisplayTimezone(DEFAULT_DISPLAY_TIMEZONE));const epoch=Date.parse("2026-10-24T16:03:04Z")/1000;
  assert.equal(setDisplayTimezone(" UTC "),"UTC");assert.equal(getDisplayTimezone(),"UTC");
  assert.equal(formatWorldTime(epoch,{date:true}),"26/10/24 16:03:04");assert.equal(worldDayKey(epoch),"2026-10-24");
  assert.equal(formatWorldClock(epoch,"Asia/Tokyo"),"01:03:04");assert.equal(getDisplayTimezone(),"UTC");
});

test("IANA validation rejects offsets and invalid names without changing the active timezone",()=>{
  setDisplayTimezone(DEFAULT_DISPLAY_TIMEZONE);
  for(const value of ["","Nope/Nowhere","+08:00","GMT+8",null])assert.throws(()=>normalizeDisplayTimezone(value));
  assert.throws(()=>setDisplayTimezone("Invalid/Zone"));assert.equal(getDisplayTimezone(),DEFAULT_DISPLAY_TIMEZONE);
  assert.equal(normalizeDisplayTimezone("Asia/Tokyo"),"Asia/Tokyo");
  assert.equal(formatWorldTime(null),"—");assert.equal(formatWorldTime(NaN),"—");assert.equal(formatWorldTime(Infinity),"—");assert.equal(worldDayKey(undefined),"");
});

test("world clock follows the timezone daylight-saving boundary instead of applying a fixed offset",()=>{
  const before=Date.parse("2026-03-08T06:59:59Z")/1000,after=before+1;
  assert.equal(formatWorldClock(before,"America/New_York"),"01:59:59");
  assert.equal(formatWorldClock(after,"America/New_York"),"03:00:00");
});
