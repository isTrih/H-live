import test from 'node:test';
import assert from 'node:assert/strict';
import {proxyAssignments} from './proxy-assignment.js';
const anchors=Array.from({length:300},(_,i)=>({rid:String(i),name:`主播${i}`}));
const leases=anchors.map(a=>({rid:a.rid,endpoint:'192.0.2.1:8080'}));
test('300 assignments render only the requested page and retain anchor names',()=>{
  const r=proxyAssignments(leases,anchors,'',2);
  assert.equal(r.items.length,50);assert.equal(r.items[0].name,'主播100');
  assert.equal(r.total,300);assert.equal(r.pages,6);
});
test('search runs before paging and shrinking results clamps the page',()=>{
  const r=proxyAssignments(leases,anchors,'主播299',5);
  assert.equal(r.page,0);assert.equal(r.total,1);assert.equal(r.items[0].rid,'299');
  assert.equal(proxyAssignments(leases,anchors,'192.0.2.1').total,300);
  assert.equal(proxyAssignments(leases,anchors,'不存在').first,0);
});
