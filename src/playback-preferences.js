// Local audio intent only. Decoder status and media URLs never enter storage.
const KEY="qingkan.playback-audio.v1";
const defaults=()=>({volume:1,muted:false,lastAudibleVolume:1});
const volume=value=>typeof value==="number"&&Number.isFinite(value)&&value>=0&&value<=1;

export function normalizePlaybackAudio(value) {
  const audio=defaults();
  if(!value||typeof value!=="object")return audio;
  if(volume(value.volume))audio.volume=value.volume;
  if(volume(value.lastAudibleVolume)&&value.lastAudibleVolume>0)audio.lastAudibleVolume=value.lastAudibleVolume;
  else if(audio.volume>0)audio.lastAudibleVolume=audio.volume;
  audio.muted=typeof value.muted==="boolean"?value.muted:audio.volume===0;
  if(audio.volume===0)audio.muted=true;
  return audio;
}

export function createPlaybackPreferences({getStorage=()=>globalThis.localStorage}={}) {
  let current=null;
  function read() {
    if(!current){
      current=defaults();
      try{const saved=JSON.parse(getStorage()?.getItem(KEY)||"null");if(saved?.format===1)current=normalizePlaybackAudio(saved);}catch{}
    }
    return {...current};
  }
  function commit(next) {
    const previous=read();current=normalizePlaybackAudio(next);
    if(Object.keys(current).some(key=>current[key]!==previous[key]))
      try{getStorage()?.setItem(KEY,JSON.stringify({format:1,...current}));}catch{}
    return {...current};
  }
  return {
    read,
    setVolume(value){
      const audio=read();if(!Number.isFinite(value))return audio;
      const amount=Math.min(1,Math.max(0,value));
      return commit({...audio,volume:amount,muted:amount===0,lastAudibleVolume:amount>0?amount:audio.lastAudibleVolume});
    },
    setMuted(value){
      const audio=read();if(typeof value!=="boolean")return audio;
      return commit({...audio,muted:value,volume:!value&&audio.volume===0?audio.lastAudibleVolume:audio.volume});
    },
  };
}

export const playbackPreferences=createPlaybackPreferences();
