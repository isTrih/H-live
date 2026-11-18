// hls.js Loader<T> backed by bounded native IPC. No URL or credential logging.
const queues = new WeakMap();
const now = () => globalThis.performance?.now?.() ?? Date.now();
function stats() {
  return {aborted:false,loaded:0,retry:0,total:0,chunkCount:0,bwEstimate:0,
    loading:{start:0,first:0,end:0},parsing:{start:0,end:0},buffering:{start:0,first:0,end:0}};
}
function pump(queue) {
  while (queue.active < 2 && queue.pending.length) {
    const job = queue.pending.shift();
    if (!job.live()) continue;
    queue.active++;
    Promise.resolve().then(() => {if (job.live()) return job.run();})
      .catch(() => {}) // A consumer callback must not produce an unhandled rejection.
      .finally(() => {queue.active--;pump(queue);});
  }
}
function request(context) {
  if (typeof context?.url !== "string" || !context.url) throw Error("invalid request");
  const start = context.rangeStart ?? 0, end = context.rangeEnd ?? 0;
  if (start === 0 && end === 0) return {url:context.url};
  if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start < 0 || end <= start) throw Error("invalid range");
  // hls.js and the native command both use an exclusive rangeEnd.
  return {url:context.url,rangeStart:start,rangeEnd:end};
}
function bytes(value) {
  if (value instanceof ArrayBuffer) return value;
  if (ArrayBuffer.isView(value)) return value.buffer.slice(value.byteOffset,value.byteOffset+value.byteLength);
  throw Error("invalid media response");
}
function safeError(error) {
  const source = typeof error === "string" ? error : typeof error?.message === "string" ? error.message : "";
  const code = Number(/\bHTTP[\s:_-]*([1-5]\d{2})\b/i.exec(source)?.[1]) || 0;
  return {code,text:code?`录播媒体请求失败（HTTP ${code}）。`:"录播媒体请求失败，请重试。"};
}

export function createReplayLoader(fetchMedia) {
  if (typeof fetchMedia !== "function") throw Error("A native media fetch function is required.");
  let queue = queues.get(fetchMedia);
  if (!queue) {queue={active:0,pending:[]};queues.set(fetchMedia,queue);}
  return class ReplayLoader {
    constructor() {this.context=null;this.stats=stats();this.callbacks=null;this.job=null;this.destroyed=false;}
    load(context, _config, callbacks) {
      if (this.destroyed || this.context) throw Error("Media loader can only be used once.");
      this.context=context;this.callbacks=callbacks;this.stats.loading.start=now();
      const job=this.job={cancelled:false,finished:false};
      job.live=()=>!this.destroyed&&!job.cancelled&&!job.finished&&this.job===job;
      job.run=async()=>{
        let buffer, data;
        try {
          buffer=bytes(await fetchMedia(request(context)));
          if (buffer.byteLength > 16*1024*1024) throw Error("media response exceeds limit");
          data=context.responseType==="arraybuffer"?buffer:new TextDecoder().decode(buffer);
          if (context.responseType==="json") data=JSON.parse(data);
        } catch(error) {
          if (!job.live()) return;
          this.stats.loading.end=now();job.finished=true;
          callbacks.onError(safeError(error),context,null,this.stats);
          return;
        }
        if (!job.live()) return;
        const completed=now();
        this.stats.loading.first=this.stats.loading.end=completed;
        this.stats.loaded=this.stats.total=buffer.byteLength;
        this.stats.bwEstimate=buffer.byteLength*8000/Math.max(1,completed-this.stats.loading.start);
        if (context.responseType!=="json") callbacks.onProgress?.(this.stats,context,data,null);
        if (!job.live()) return;
        job.finished=true;
        callbacks.onSuccess({url:context.url,data,code:context.rangeEnd?206:200},this.stats,context,null);
      };
      queue.pending.push(job);pump(queue);
    }
    abort() {
      const job=this.job;
      if (!job || job.cancelled || job.finished) return;
      job.cancelled=true;this.stats.aborted=true;
      const callbacks=this.callbacks;this.callbacks=null;
      callbacks?.onAbort?.(this.stats,this.context,null);
    }
    destroy() {
      if (this.destroyed) return;
      this.destroyed=true;
      if (this.job&&!this.job.finished) {this.job.cancelled=true;this.stats.aborted=true;}
      this.callbacks=null;this.context=null;
    }
    getCacheAge() {return null;}
    getResponseHeader() {return null;}
  };
}
