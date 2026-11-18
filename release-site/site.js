const IDLE_MS = 5000;

export function wheelDistance(event, viewportHeight = 800) {
  const delta = Math.abs(event.deltaX || 0) > Math.abs(event.deltaY || 0) ? event.deltaX : event.deltaY;
  return Number.isFinite(delta) ? delta * (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? viewportHeight : 1) : 0;
}

export function createStoryController({count, initial = 0, reducedMotion = false, now = () => performance.now(), setTimer = setTimeout, clearTimer = clearTimeout, onChange = () => {}, onTimer = () => {}}) {
  if (!Number.isInteger(count) || count < 1) throw Error('Invalid story length');
  let index = Math.max(0, Math.min(count - 1, initial)), automatic = !reducedMotion, timer = null, disposed = false;
  let lastWheel = -Infinity, wheelSum = 0, consumed = false, lockedUntil = 0;
  const blocks = new Set();
  const state = () => ({index, count, automatic, blocked: blocks.size > 0, atEnd: index === count - 1});
  function arm() {
    if (timer !== null) clearTimer(timer);
    timer = null;
    const running = !disposed && automatic && !blocks.size && index < count - 1;
    onTimer({...state(), running, delay: IDLE_MS});
    if (running) timer = setTimer(() => { timer = null; go(index + 1, 'automatic'); }, IDLE_MS);
  }
  function go(target, source = 'user') {
    if (disposed || !Number.isInteger(target)) return false;
    const next = Math.max(0, Math.min(count - 1, target)), previous = index;
    index = next;
    if (index !== previous) onChange({...state(), previous, source});
    arm();
    return index !== previous;
  }
  function wheel(delta) {
    if (disposed || blocks.size || !Number.isFinite(delta) || delta === 0) return false;
    arm();
    const time = now();
    if (time - lastWheel > 180) { wheelSum = 0; consumed = false; }
    lastWheel = time;
    if (consumed || time < lockedUntil) return false;
    wheelSum += delta;
    if (Math.abs(wheelSum) < 42) return false;
    consumed = true;
    lockedUntil = time + (reducedMotion ? 180 : 650);
    return go(index + Math.sign(wheelSum), 'wheel');
  }
  function block(reason, blocked) { blocked ? blocks.add(reason) : blocks.delete(reason); arm(); }
  function toggle() { automatic = !automatic; arm(); return automatic; }
  function dispose() { disposed = true; if (timer !== null) clearTimer(timer); timer = null; }
  arm();
  return {state, go, wheel, block, toggle, activity: arm, dispose};
}

export function validateRelease(release, base) {
  if (!release || !/^\d+\.\d+\.\d+(?:[-+][\w.-]+)?$/.test(release.version)) throw Error('Invalid release');
  const platform = release.platforms?.['windows-x86_64'];
  if (typeof platform?.signature !== 'string' || !platform.signature.trim() || platform.signature.length > 16384) throw Error('Missing signature');
  const origin = new URL(base), url = new URL(platform.url);
  const expected = new URL(`${release.version}/QingkanLive-${release.version}-windows-x64-setup.exe`, origin);
  if (origin.protocol !== 'https:' || url.href !== expected.href || url.username || url.password || url.search || url.hash) throw Error('Unexpected download location');
  return {version: release.version, url: url.href};
}

export function validateChangelog(data) {
  if (data?.format !== 1 || !Array.isArray(data.entries) || data.entries.length < 1 || data.entries.length > 1000) throw Error('Invalid changelog');
  let bytes = 0;
  const seen = new Set();
  for (const entry of data.entries) {
    if (!/^\d+\.\d+\.\d+(?:[-+][\w.-]+)?$/.test(entry?.version) || seen.has(entry.version) || typeof entry.body !== 'string' || entry.body.length > 1024 * 1024 || typeof entry.title !== 'string' || entry.title.length > 300) throw Error('Invalid changelog entry');
    formatReleaseDate(entry.date);
    seen.add(entry.version); bytes += entry.body.length;
  }
  if (bytes > 16 * 1024 * 1024) throw Error('Changelog too large');
  return data.entries;
}

export function formatReleaseDate(date) {
  if (date == null) return '日期未记录';
  if (typeof date !== 'string' || !/^\d{4}-\d{2}-\d{2}$/.test(date) || !Number.isFinite(Date.parse(`${date}T00:00:00Z`)) || new Date(`${date}T00:00:00Z`).toISOString().slice(0,10) !== date) throw Error('Invalid release date');
  return date.slice(2).replaceAll('-','/');
}

function appendInline(root, text) {
  const pattern = /(\*\*[^*\n]+\*\*|`[^`\n]+`)/g;
  let offset = 0;
  for (const match of text.matchAll(pattern)) {
    root.append(document.createTextNode(text.slice(offset,match.index)));
    const strong = match[0].startsWith('**'), node = document.createElement(strong ? 'strong' : 'code');
    node.textContent = match[0].slice(strong ? 2 : 1,strong ? -2 : -1); root.append(node);
    offset = match.index + match[0].length;
  }
  root.append(document.createTextNode(text.slice(offset)));
}
function appendMarkdown(root, body, title) {
  const lines = body.replace(/^\uFEFF/, '').split(/\r?\n/);
  if (lines[0]?.replace(/^#\s+/, '').trim() === title.trim()) lines.shift();
  let list = null, code = null, paragraph = null;
  for (const line of lines) {
    if (/^\s*```/.test(line)) {
      if (code) code = null;
      else { code = document.createElement('pre'); root.append(code); }
      list = paragraph = null; continue;
    }
    if (code) { code.textContent += `${line}\n`; continue; }
    if (!line.trim()) { list = paragraph = null; continue; }
    const heading = /^(#{1,6})\s+(.+)$/.exec(line), item = /^\s*(?:[-*+]\s+|\d+[.)]\s+)(.*)$/.exec(line);
    if (heading) {
      const node = document.createElement(`h${Math.min(5, heading[1].length + 2)}`);
      node.textContent = heading[2]; root.append(node); list = paragraph = null;
    } else if (item) {
      const kind = /^\s*\d/.test(line) ? 'ol' : 'ul';
      if (!list || list.tagName.toLowerCase() !== kind) { list = document.createElement(kind); root.append(list); }
      const node = document.createElement('li'); appendInline(node,item[1]); list.append(node); paragraph = null;
    } else {
      list = null;
      if (!paragraph) { paragraph = document.createElement('p'); root.append(paragraph); }
      else paragraph.append(document.createElement('br'));
      appendInline(paragraph,line);
    }
  }
}

async function fetchJson(path, limit = 4 * 1024 * 1024) {
  const abort = new AbortController(), timer = setTimeout(() => abort.abort(), 15000);
  try {
    const response = await fetch(new URL(path, import.meta.url), {cache:'no-store', credentials:'omit', redirect:'error', signal:abort.signal});
    if (!response.ok || Number(response.headers.get('content-length') || 0) > limit) throw Error('Unavailable');
    if (!response.body) throw Error('Unavailable');
    const reader = response.body.getReader(), chunks = []; let length = 0;
    for (;;) {
      const {done,value} = await reader.read(); if (done) break;
      length += value.byteLength;
      if (length > limit) { await reader.cancel(); throw Error('Too large'); }
      chunks.push(value);
    }
    const merged = new Uint8Array(length); let offset = 0;
    for (const chunk of chunks) { merged.set(chunk,offset); offset += chunk.byteLength; }
    return JSON.parse(new TextDecoder().decode(merged));
  } finally { clearTimeout(timer); }
}

function startPage() {
  const track = document.querySelector('#story-track'), slides = [...document.querySelectorAll('[data-slide]')];
  if (!track || !slides.length) return;
  const reduced = matchMedia('(prefers-reduced-motion: reduce)'), motion = window.gsap;
  const dots = [...document.querySelectorAll('[data-go-slide]')], dialog = document.querySelector('#changelog-dialog');
  const opener = document.querySelector('#open-changelog'), content = document.querySelector('#changelog-content');
  const current = document.querySelector('#story-current'), status = document.querySelector('#story-status');
  const visited = new Set();
  let controller, tween, logsLoaded = false, logsLoading = false;
  const ambient = new Map();
  const requestedHash=location.hash==='#download'?'#download-slide':location.hash;
  let initial = slides.findIndex(slide => `#${slide.id}` === requestedHash); if (initial < 0) initial = 0;
  function reveal(index) {
    if (visited.has(index)) return;
    visited.add(index);
    if (reduced.matches || !motion) return;
    const slide = slides[index], direction = index === 0 ? 1 : -1;
    const elements = [...slide.querySelectorAll('[data-reveal]')].filter(element => !element.querySelector('.headline-row'));
    motion.from(elements, {y:16, opacity:0, duration:.5, delay:.1, stagger:.065, ease:'power3.out', clearProps:'opacity,transform'});
    motion.from(slide.querySelectorAll('.headline-row>span,.headline-row>em'), {yPercent:105, rotation:3*direction, duration:.7, stagger:.085, ease:'power3.out', clearProps:'transform'});
    const picture = slide.querySelector('.product-shot');
    if (picture) motion.from(picture, {x:32, opacity:.3, scale:.975, duration:.65, ease:'power3.out', clearProps:'opacity,transform'});
    const mascot = slide.querySelector('.float-layer');
    if (mascot) motion.from(mascot.parentElement, {x:24, opacity:0, duration:.7, ease:'power3.out', clearProps:'opacity,transform'});
  }
  function ambientState(state) {
    if (!motion) return;
    if (!reduced.matches && !ambient.has(state.index)) {
      const slide = slides[state.index], animation = motion.timeline({paused:true,repeat:-1,yoyo:true});
      const mascot = slide.querySelector('.float-layer'), bars = slide.querySelectorAll('.hero-wave i');
      if (mascot) animation.to(mascot,{y:-9,duration:2.8,ease:'sine.inOut'},0);
      if (bars.length) animation.fromTo(bars,{scaleY:index=>.5+(index%4)*.1},{scaleY:1,duration:1.2,stagger:.045,ease:'sine.inOut'},0);
      ambient.set(state.index,animation);
    }
    for (const [index,animation] of ambient) {
      if (reduced.matches || state.blocked || index !== state.index) animation.pause();
      else animation.play();
    }
  }
  function present({index, source = 'initial'}) {
    const focused = document.activeElement;
    const scrollHint = document.querySelector('.story-controls>.scroll-hint');
    if (scrollHint) scrollHint.hidden = index === slides.length - 1;
    for (let n = 0; n < slides.length; n++) {
      const active = n === index;
      slides[n].classList.toggle('is-active', active); slides[n].inert = !active;
      slides[n].setAttribute('aria-hidden', String(!active));
    }
    dots.forEach(dot => dot.setAttribute('aria-current', Number(dot.dataset.goSlide) === index ? 'step' : 'false'));
    if (current) current.textContent = String(index + 1).padStart(2, '0');
    if (status) status.textContent = `${index + 1} / ${slides.length} · ${slides[index].getAttribute('aria-label') || slides[index].dataset.title || ''}`;
    tween?.kill();
    const x = -index * window.innerWidth;
    if (motion && !reduced.matches && source !== 'initial') tween = motion.to(track, {x, duration:.55, ease:'power3.inOut', overwrite:true});
    else { if (motion) motion.set(track,{x}); else track.style.transform = `translate3d(${x}px,0,0)`; }
    reveal(index);
    if (controller) ambientState(controller.state());
    if (source === 'keyboard' && slides.some(slide => slide.contains(focused))) slides[index].focus({preventScroll:true});
    if (source !== 'initial') { try { history.replaceState(null,'',`#${slides[index].id}`); } catch {} }
  }
  function timerState(state) {
    document.body.classList.toggle('auto-paused', !state.automatic || state.blocked || state.atEnd);
    ambientState(state);
  }
  controller = createStoryController({count:slides.length,initial,reducedMotion:reduced.matches,onChange:present,onTimer:timerState});
  document.querySelector('#story-count').textContent = String(slides.length).padStart(2,'0');
  present({...controller.state(),source:'initial'});
  function inDialog() { return Boolean(document.querySelector('dialog[open]')); }
  function canScroll(target, delta) {
    const pane = target?.closest?.('[data-scroll-pane]');
    return Boolean(pane && pane.scrollHeight > pane.clientHeight + 1 && (delta < 0 ? pane.scrollTop > 1 : pane.scrollTop + pane.clientHeight < pane.scrollHeight - 1));
  }
  document.addEventListener('wheel', event => {
    if (event.ctrlKey || inDialog()) return;
    const delta = wheelDistance(event,innerHeight);
    if (Math.abs(event.deltaY || 0) >= Math.abs(event.deltaX || 0) && canScroll(event.target,delta)) { controller.activity(); return; }
    if (delta) { event.preventDefault(); controller.wheel(delta); }
  }, {passive:false});
  for (const dot of dots) dot.addEventListener('click', event => {event.preventDefault();controller.go(Number(dot.dataset.goSlide),'navigation');});
  document.addEventListener('keydown', event => {
    if (inDialog() || event.ctrlKey || event.metaKey || event.altKey || event.target.closest?.('input,textarea,select,[contenteditable="true"]')) return;
    if (event.key === ' ' && event.target.matches('body,main,section')) {event.preventDefault();controller.toggle();return;}
    const index = controller.state().index;
    const target = ['ArrowRight','ArrowDown','PageDown'].includes(event.key) ? index + 1 : ['ArrowLeft','ArrowUp','PageUp'].includes(event.key) ? index - 1 : event.key === 'Home' ? 0 : event.key === 'End' ? slides.length - 1 : null;
    if (target !== null) {event.preventDefault();controller.go(target,'keyboard');}
    else controller.activity();
  });
  let touch = null;
  document.addEventListener('touchstart', event => {
    if (inDialog()) {touch=null;return;}
    controller.block('touch',true);
    if (event.touches.length !== 1) {touch=null;return;}
    touch = {x:event.touches[0].clientX,y:event.touches[0].clientY,target:event.target,scrolling:false}; controller.activity();
  },{passive:true});
  document.addEventListener('touchmove', event => {
    if (!touch || inDialog() || event.touches.length !== 1) return;
    const dx = touch.x - event.touches[0].clientX, dy = touch.y - event.touches[0].clientY;
    if (!touch.axis && Math.max(Math.abs(dx),Math.abs(dy)) > 8) touch.axis = Math.abs(dx) > Math.abs(dy) ? 'x' : 'y';
    if (touch.axis !== 'x' && dy !== 0 && canScroll(touch.target,dy)) {touch.scrolling=true;return;}
    if (!touch.scrolling) event.preventDefault();
  },{passive:false});
  document.addEventListener('touchend', event => {
    if (event.touches.length) return;
    controller.block('touch',false);
    if (!touch || touch.scrolling || inDialog()) {touch=null;return;}
    const end = event.changedTouches[0]; if (!end) return;
    const dx=touch.x-end.clientX, dy=touch.y-end.clientY, distance=Math.abs(dx)>Math.abs(dy)?dx:dy;
    if (Math.abs(distance)>48) controller.go(controller.state().index+Math.sign(distance),'touch');
    touch=null;
  },{passive:true});
  document.addEventListener('touchcancel', () => {touch=null;controller.block('touch',false);},{passive:true});
  document.addEventListener('pointerdown', () => controller.block('pointer',true),{passive:true});
  window.addEventListener('pointerup', () => controller.block('pointer',false),{passive:true});
  window.addEventListener('pointercancel', () => controller.block('pointer',false),{passive:true});
  window.addEventListener('blur', () => {controller.block('pointer',false);controller.block('touch',false);controller.block('window',true);});
  window.addEventListener('focus', () => controller.block('window',false));
  document.addEventListener('visibilitychange', () => controller.block('hidden',document.hidden));
  controller.block('hidden',document.hidden);
  reduced.addEventListener?.('change', event => { if (event.matches && controller.state().automatic) controller.toggle(); present({...controller.state(),source:'initial'}); });
  window.addEventListener('resize', () => { tween?.kill(); const x=-controller.state().index*innerWidth; if(motion)motion.set(track,{x});else track.style.transform=`translate3d(${x}px,0,0)`; });

  function logMessage(message, retry = false) {
    content.replaceChildren(); const p=document.createElement('p');p.className='changelog-message';p.textContent=message;content.append(p);
    if(retry){const button=document.createElement('button');button.type='button';button.textContent='重新读取';button.addEventListener('click',loadLogs);content.append(button);}
  }
  async function loadLogs() {
    if (logsLoading || logsLoaded) return;
    logsLoading = true; logMessage('正在读取更新日志…');
    try {
      const entries=validateChangelog(await fetchJson('./changelog.json',16*1024*1024));
      const fragment=document.createDocumentFragment();
      for(const entry of entries){
        const article=document.createElement('article');article.className='changelog-entry';
        const heading=document.createElement('h2');heading.textContent=entry.title || `v${entry.version}`;article.append(heading);
        if(entry.version===document.querySelector('#version')?.dataset.version){const tag=document.createElement('span');tag.className='changelog-latest';tag.textContent='最新版本';heading.append(tag);}
        const date=document.createElement(entry.date?'time':'span');date.className='changelog-date';date.textContent=formatReleaseDate(entry.date);
        if(entry.date)date.dateTime=entry.date;
        article.append(date);
        appendMarkdown(article,entry.body,entry.title);fragment.append(article);
      }
      content.replaceChildren(fragment); logsLoaded=true;
      const count=document.querySelector('#changelog-count');if(count)count.textContent=`${entries.length} 个版本`;
    } catch { logMessage('更新日志暂时无法读取。',true); }
    finally { logsLoading=false; }
  }
  opener?.addEventListener('click', () => {
    controller.block('dialog',true);opener.setAttribute('aria-expanded','true');
    if(typeof dialog.showModal==='function')dialog.showModal();else dialog.setAttribute('open','');
    document.querySelector('#close-changelog')?.focus({preventScroll:true});void loadLogs();
  });
  const closeLogs=()=>{if(typeof dialog.close==='function')dialog.close();else{dialog.removeAttribute('open');controller.block('dialog',false);opener?.focus({preventScroll:true});}};
  document.querySelector('#close-changelog')?.addEventListener('click',closeLogs);
  dialog?.addEventListener('click',event=>{if(event.target===dialog)closeLogs();});
  dialog?.addEventListener('close',()=>{opener?.setAttribute('aria-expanded','false');controller.block('dialog',false);opener?.focus({preventScroll:true});});

  let releaseBusy=false;
  async function loadRelease() {
    if(releaseBusy)return;releaseBusy=true;
    const links=[...document.querySelectorAll('[data-release-download]')],label=document.querySelector('#download-label'),message=document.querySelector('#status');
    for(const link of links){link.removeAttribute('href');link.setAttribute('aria-disabled','true');link.tabIndex=-1;}
    label.textContent='正在读取版本';
    try{
      const base=document.querySelector('meta[name="release-base"]')?.content.trim();
      const release=validateRelease(await fetchJson('./latest.json',262144),base);
      for(const link of links){link.href=release.url;link.setAttribute('aria-disabled','false');link.removeAttribute('tabindex');}
      const version=document.querySelector('#version');version.textContent=`最新版本 v${release.version}`;version.dataset.version=release.version;
      label.textContent='下载 Windows 版';message.textContent='公司网络 / VPN 内下载与更新';
    }catch{
      label.textContent='暂时无法下载';message.textContent='请连接公司网络 / VPN 后重试。';
      const retry=document.createElement('button');retry.type='button';retry.className='retry';retry.textContent='重试';retry.addEventListener('click',loadRelease);message.append(retry);
    }finally{releaseBusy=false;}
  }
  void loadRelease();
  window.addEventListener('pagehide',event=>{controller.block('page-cache',true);if(!event.persisted){controller.dispose();for(const animation of ambient.values())animation.kill();}tween?.kill();});
  window.addEventListener('pageshow',event=>{if(event.persisted){controller.block('page-cache',false);controller.block('hidden',document.hidden);present({...controller.state(),source:'initial'});}});
}

if (typeof document !== 'undefined') startPage();
