import test from "node:test";
import assert from "node:assert/strict";
import {createSessionTimeline} from "./session-timeline.js";
import {createPlaybackPreferences} from "./playback-preferences.js";
import {createBookmarkDocument} from "./test-support/bookmark-dom.js";

const flush=()=>new Promise(resolve=>setImmediate(resolve));
const anchor={rid:"111",anchor_id:"11",session_id:"21",state:"connected",active:true,is_live:true};
function storage(){const values=new Map();return {getItem:key=>values.get(key)||null,setItem:(key,value)=>values.set(key,value)};}
function createController(prefs,media) {
  return createSessionTimeline({state:{modal:{kind:"detail",rid:"111"}},audioPreferences:prefs,
    bridge:async(method,params)=>method==="session_replay"?{enabled:true,available:true,replay:{play_url:"https://media.xingtu.cn/replay.m3u8",duration_ms:120000}}:method==="session_timeline"?{anchor_id:"11",session_id:params.session_id,start_epoch:1792735200,duration_seconds:120,messages:[],buckets:[],activities:[]}:method==="mark_list"?{marks:[],total:0}:{},
    createMedia:options=>{
      let audio={...options.initialAudio},lastAudible=audio.lastAudibleVolume;
      const player={element:options.element,hasMedia:true,isFullscreen:false,currentTime:0,duration:120,paused:true,
        options,sources:[],setSource:async replay=>{player.sources.push({replay,audio:{...audio}});options.onVolume(audio);},
        speed(){},pause(){},play:async()=>{},destroy(){},
        volume(value){audio={volume:value,muted:value===0};if(value>0)lastAudible=value;options.onVolume(audio);},
        muted(value){audio={volume:!value&&audio.volume===0?lastAudible:audio.volume,muted:value};options.onVolume(audio);},
      };media.push(player);return player;
    },
  });
}

test("audio intent is global across session switches, modal reopen and an application restart",async t=>{
  const previous=globalThis.document,document=createBookmarkDocument();globalThis.document=document;
  const disk=storage(),newPrefs=()=>createPlaybackPreferences({getStorage:()=>disk}),prefs=newPrefs(),media=[];
  let timeline=createController(prefs,media);
  t.after(()=>{timeline.unmount();globalThis.document=previous;});
  timeline.mount(anchor);await flush();await flush();
  assert.deepEqual(media[0].options.initialAudio,{volume:1,muted:false,lastAudibleVolume:1});
  assert.equal(document.querySelector("video").getAttribute("muted"),null);
  timeline.input({id:"timeline-volume",value:"0.24"});
  await timeline.action({dataset:{action:"timeline-mute"}});
  assert.deepEqual(prefs.read(),{volume:.24,muted:true,lastAudibleVolume:.24});
  timeline.selectSession("22");await flush();await flush();
  assert.equal(media.length,1,"changing session preserves the player and its audio state");
  assert.equal(media[0].sources.at(-1).audio.muted,true);
  timeline.unmount();timeline.mount(anchor);await flush();await flush();
  assert.deepEqual(media.at(-1).options.initialAudio,{volume:.24,muted:true,lastAudibleVolume:.24});
  timeline.unmount();timeline=createController(newPrefs(),media);timeline.mount(anchor);await flush();await flush();
  assert.equal(media.at(-1).options.initialAudio.muted,true);
  await timeline.action({dataset:{action:"timeline-mute"}});
  assert.deepEqual(newPrefs().read(),{volume:.24,muted:false,lastAudibleVolume:.24});
  // Opening/status observations may reflect a device or an older native command.
  // They update controls, but must never become a saved user preference.
  media.at(-1).options.onVolume({volume:1,muted:true});
  assert.deepEqual(newPrefs().read(),{volume:.24,muted:false,lastAudibleVolume:.24});
  timeline.unmount();timeline.mount(anchor);await flush();await flush();
  assert.deepEqual(media.at(-1).options.initialAudio,{volume:.24,muted:false,lastAudibleVolume:.24});
  timeline.input({id:"timeline-volume",value:"0"});
  await timeline.action({dataset:{action:"timeline-mute"}});
  assert.deepEqual(newPrefs().read(),{volume:.24,muted:false,lastAudibleVolume:.24});
});
