import test from "node:test";
import assert from "node:assert/strict";
import {sessionDate} from "./session-date.js";
test("session dates use YY/MM/DD in Shanghai, including UTC date crossings",()=>{
  assert.equal(sessionDate("2026-10-25T14:14:00+08:00"),"26/10/25");
  assert.equal(sessionDate("2026-10-24T18:14:00Z",true),"26/10/25 02:14");
  assert.equal(sessionDate(Date.parse("2026-10-25T00:00:00+08:00")/1000),"26/10/25");
  assert.equal(sessionDate(null),"日期未知");
});
