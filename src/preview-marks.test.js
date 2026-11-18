import test from 'node:test';
import assert from 'node:assert/strict';
import {createPreviewMarks} from './preview-marks.js';
import {createPreviewDownloads} from './preview-downloads.js';
const memory=()=>{const rows=new Map();return {getItem:key=>rows.get(key)||null,setItem:(key,value)=>rows.set(key,value)};};
const anchor={active:true,is_live:true,anchor_id:'456',session_id:'7698991795467832114',rid:'123',name:'桃桃大王',broadcast_started_at:1792896000};

test('preview marks survive reload, retain server time on retry and reject a changed live session',()=>{
  const storage=memory();let epoch=1792896600;
  const options={storage,getAnchor:()=>anchor,now:()=>epoch};
  const marks=createPreviewMarks(options),params={anchor_id:anchor.anchor_id,session_id:anchor.session_id,title:'精彩防守',request_key:'same-click',marked_at_epoch:1};
  const response=marks.handle('mark_create',params);assert.equal(response.mark.marked_at_epoch,epoch);
  epoch+=5;assert.equal(marks.handle('mark_create',params).created,false);
  const restored=createPreviewMarks(options);assert.equal(restored.handle('mark_list',params).marks.length,1);
  restored.handle('mark_update',{id:response.mark.id,note:'这段需要精选'});
  assert.equal(createPreviewMarks(options).get(response.mark.id).note,'这段需要精选');
  assert.throws(()=>restored.handle('mark_create',{...params,session_id:'other',request_key:'new-click'}),/场次/);
});

test('a preview bookmark creates a named download through the same persisted naming contract',async()=>{
  const storage=memory(),getAnchor=()=>anchor,options={storage,getAnchor,now:()=>1792896600};
  const marks=createPreviewMarks(options),mark=marks.handle('mark_create',{anchor_id:anchor.anchor_id,session_id:anchor.session_id,title:'精彩防守'}).mark;
  const downloads=createPreviewDownloads({...options,getMark:id=>marks.get(id),seed:false});
  await downloads.handle('download_naming_save',{root_directory:'C:/氢看下载',directory_template:'{date}/{anchor}',filename_template:'{title}_{start}-{end}.mp4'});
  const response=await downloads.handle('clip_create',{anchor_id:anchor.anchor_id,session_id:anchor.session_id,title:mark.title,source:'bookmark',bookmark_id:mark.id,start_seconds:585,end_seconds:630,request_key:'selected-mark'});
  assert.equal(response.job.source,'bookmark');assert.equal(response.job.bookmark_id,mark.id);
  assert.match(response.job.output_path,/桃桃大王/);assert.match(response.job.output_path,/精彩防守/);
  assert.equal(response.job.simulated,true);
});

test('failed preview storage does not pretend to create or delete a mark',()=>{
  const storage=memory(),marks=createPreviewMarks({storage,getAnchor:()=>anchor});
  const mark=marks.handle('mark_create',{anchor_id:anchor.anchor_id,session_id:anchor.session_id}).mark;
  storage.setItem=()=>{throw Error('storage full');};
  assert.throws(()=>marks.handle('mark_delete',{id:mark.id}),/storage full/);
  assert.equal(marks.get(mark.id).id,mark.id);
});
