import test from "node:test";
import assert from "node:assert/strict";
import {createReplayLoader} from "./replay-loader.js";

function run(loader,context) {
  const progress=[];
  const done=new Promise(resolve=>loader.load(context,{}, {
    onSuccess:(...args)=>resolve({kind:"success",args,progress}),
    onError:(...args)=>resolve({kind:"error",args,progress}),
    onAbort:(...args)=>resolve({kind:"abort",args,progress}),
    onProgress:(...args)=>progress.push(args),
  }));
  return done;
}
const flush=()=>new Promise(resolve=>setImmediate(resolve));

test("native HLS loader decodes UTF8 playlists and exposes the complete LoaderStats contract",async()=>{
  const requests=[],text="#EXTM3U\n# 中文\n";
  const Loader=createReplayLoader(async request=>{requests.push(request);return new TextEncoder().encode(text);});
  const loader=new Loader(),context={url:"https://media.example/playlist.m3u8",responseType:"text",rangeStart:0,rangeEnd:0};
  const result=await run(loader,context);
  assert.equal(result.kind,"success");
  assert.deepEqual(requests,[{url:context.url}]);
  const [response,stats,returnedContext,network]=result.args;
  assert.equal(response.url,context.url);assert.equal(response.data,text);assert.equal(response.code,200);
  assert.equal(returnedContext,context);assert.equal(network,null);assert.equal(stats,loader.stats);
  assert.equal(stats.loaded,new TextEncoder().encode(text).byteLength);assert.equal(stats.total,stats.loaded);
  assert.deepEqual(Object.keys(stats).sort(),["aborted","buffering","bwEstimate","chunkCount","loaded","loading","parsing","retry","total"].sort());
  assert.ok(stats.loading.end>=stats.loading.start);assert.equal(result.progress.length,1);
  assert.equal(loader.getCacheAge(),null);assert.equal(loader.getResponseHeader("age"),null);
});

test("native HLS loader preserves exclusive byte ranges and sliced Uint8Array payloads",async()=>{
  const requests=[],storage=new Uint8Array([99,1,2,3,99]);
  const Loader=createReplayLoader(async request=>{requests.push(request);return storage.subarray(1,4);});
  const context={url:"https://media.example/segment.ts",responseType:"arraybuffer",rangeStart:20,rangeEnd:23};
  const result=await run(new Loader(),context);
  assert.deepEqual(requests,[{url:context.url,rangeStart:20,rangeEnd:23}]);
  assert.ok(result.args[0].data instanceof ArrayBuffer);
  assert.deepEqual([...new Uint8Array(result.args[0].data)],[1,2,3]);assert.equal(result.args[0].code,206);
});

test("native HLS loader returns safe HTTP diagnostics without echoing signed URLs or keys",async()=>{
  for(const failure of [Error("HTTP 403 https://media.example/?signature=secret"),"network secret-key-value"]){
    const Loader=createReplayLoader(async()=>{throw failure;});
    const result=await run(new Loader(),{url:"https://media.example/segment.ts",responseType:"arraybuffer"});
    assert.equal(result.kind,"error");assert.equal(result.args[0].code,typeof failure==="string"?0:403);
    assert.doesNotMatch(result.args[0].text,/secret|signature|https:/);
  }
});

test("abort and destroy suppress stale native completions while explicit abort satisfies the HLS callback",async()=>{
  let finish;const Loader=createReplayLoader(()=>new Promise(resolve=>{finish=resolve;}));
  const loader=new Loader(),done=run(loader,{url:"https://media.example/segment.ts",responseType:"arraybuffer"});
  await flush();loader.abort();assert.equal((await done).kind,"abort");assert.equal(loader.stats.aborted,true);
  finish(new Uint8Array([1]));await flush();
  let callbacks=0,finishDestroyed;
  const DestroyedLoader=createReplayLoader(()=>new Promise(resolve=>{finishDestroyed=resolve;}));
  const destroyed=new DestroyedLoader();
  destroyed.load({url:"https://media.example/segment.ts",responseType:"arraybuffer"},{},{onSuccess:()=>callbacks++,onError:()=>callbacks++,onAbort:()=>callbacks++});
  await flush();destroyed.destroy();finishDestroyed(new Uint8Array([2]));await flush();
  assert.equal(callbacks,0);assert.equal(destroyed.stats.aborted,true);
});

test("at most two native requests run across loader factories and cancelled queued requests never start",async()=>{
  let active=0,peak=0;const finishes=[];
  const fetchMedia=()=>{active++;peak=Math.max(peak,active);return new Promise(resolve=>finishes.push(()=>{active--;resolve(new ArrayBuffer(1));}));};
  const First=createReplayLoader(fetchMedia),Second=createReplayLoader(fetchMedia);
  const loaders=[new First(),new Second(),new First(),new Second()];
  const pending=loaders.map((loader,index)=>run(loader,{url:`https://media.example/${index}.ts`,responseType:"arraybuffer"}));
  await flush();assert.equal(active,2);loaders[2].abort();
  finishes.shift()();await flush();assert.equal(active,2);assert.equal(finishes.length,2);
  finishes.shift()();finishes.shift()();
  const results=await Promise.all(pending);assert.deepEqual(results.map(item=>item.kind),["success","success","abort","success"]);assert.equal(peak,2);
});
