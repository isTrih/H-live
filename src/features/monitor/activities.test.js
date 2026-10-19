import {test} from 'node:test';
import assert from 'node:assert/strict';
import {activityView} from './activities.js';
const anchor = {active:true,state:'connected',activities:{red_packet:{total:3,active_until:1060,uncertain:false},lottery:{total:2,active_until:null,uncertain:true}}};
test('packet and bag have independent totals and lifetimes',()=>{
 assert.equal(activityView(anchor,'red_packet',1000).state,'active');
 assert.equal(activityView(anchor,'red_packet',1060).state,'idle');
 assert.equal(activityView(anchor,'lottery',1000).state,'unknown');
 assert.equal(activityView(anchor,'lottery',1000).count,'2');
});
test('unverified and stopped activities cannot appear active',()=>{
 assert.equal(activityView({},'red_packet',1000).count,'—');
 assert.equal(activityView({...anchor,active:false},'red_packet',1000).state,'unknown');
 assert.equal(activityView({...anchor,state:'retrying'},'red_packet',1000).state,'unknown');
});
test('unidentified notifications show a lower bound without inventing rounds',()=>{
 const a={...anchor,activities:{red_packet:{total:3,unidentified:true}}};
 assert.equal(activityView(a,'red_packet',1000).count,'≥3');
 assert.equal(activityView(a,'red_packet',1000).state,'unknown');
 a.activities.red_packet.total=0;
 assert.equal(activityView(a,'red_packet',1000).count,'—');
});
