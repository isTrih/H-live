import test from "node:test";
import assert from "node:assert/strict";
import {collectBrowserCompatibility,createCompatibilityChecks,redactCompatibilityReport} from "./compatibility-checks.js";

test("capability reporting keeps API estimates separate and releases its temporary GPU context",async()=>{
  let released=0;
  const gl={RENDERER:1,VENDOR:2,MAX_TEXTURE_SIZE:3,getParameter:key=>key===3?8192:"GPU renderer",getExtension:name=>name==="WEBGL_lose_context"?{loseContext(){released++;}}:null};
  const report=await collectBrowserCompatibility({document:{fullscreenEnabled:true,createElement:kind=>kind==="canvas"?{getContext:()=>gl}:{canPlayType:()=>"maybe",requestVideoFrameCallback(){}}},window:{MediaSource:{isTypeSupported:()=>true},AudioContext:function(){}},navigator:{mediaCapabilities:{decodingInfo:async()=>({supported:true,smooth:true,powerEfficient:false})}}});
  assert.equal(report.video_codecs.hevc.decoding.file.power_efficient,false);
  assert.equal(report.video_codecs.avc.can_play_type,"maybe");assert.equal(report.webgl.max_texture_size,8192);assert.equal(released,1);
  assert.match(report.estimate_note,/不能证明/);assert.equal(report.apis.request_video_frame_callback,true);
});
test("copied reports remove credentials, signed links, user identity and local paths",()=>{
  const result=redactCompatibilityReport({token:"private",username:"person",nested:{message:"Bearer abc-private",path:"C:\\Users\\person\\app\\ffmpeg.exe",media:"https://cdn.example/video?token=private"},version:"0.4.0"});
  const text=JSON.stringify(result);assert.doesNotMatch(text,/person|abc-private|cdn\.example|ffmpeg\.exe|"private"/);assert.match(text,/0\.4\.0/);
});
test("mounting stays idle until a manual check, then copies only the redacted native and browser report",async()=>{
  const calls=[],copied=[],root={innerHTML:"",isConnected:true};
  const controller=createCompatibilityChecks({invoke:async name=>{calls.push(name);return {os:"Windows",token:"private-native"};},collectBrowser:async()=>({webgl:{renderer:"GPU"}}),navigator:{clipboard:{writeText:async value=>copied.push(value)}}});
  controller.mount(root);assert.deepEqual(calls,[]);
  await controller.action({dataset:{action:"compatibility-run"}});assert.deepEqual(calls,["compatibility_report"]);
  await controller.action({dataset:{action:"compatibility-copy"}});assert.equal(copied.length,1);assert.doesNotMatch(copied[0],/private-native/);assert.doesNotMatch(root.innerHTML,/private-native/);assert.match(root.innerHTML,/已复制脱敏报告/);controller.unmount();
});
