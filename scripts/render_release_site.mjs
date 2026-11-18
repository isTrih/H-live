import {readFile,writeFile,mkdir,copyFile} from 'node:fs/promises';
import {resolve,join,dirname} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';
import {buildReleaseNotes,serializeReleaseNotes} from './release_notes_data.mjs';

const defaultRoot=fileURLToPath(new URL('../',import.meta.url));
const attr=value=>String(value).replaceAll('&','&amp;').replaceAll('"','&quot;').replaceAll('<','&lt;').replaceAll('>','&gt;');
export async function renderReleaseSite(root=defaultRoot){
  root=resolve(root);
  const load=path=>readFile(join(root,path),'utf8');
  let html=await load('release-site/page.html');
  const site=JSON.parse(await load('release-site/publishing.json'));
  const version=JSON.parse(await load('package.json')).version;
  if(!/^\d+\.\d+\.\d+$/.test(version))throw Error('Invalid release website version.');
  const base=site.url.replace(/\/$/,'')+(site.base_path||'').replace(/\/$/,'')+'/';
  const substitutions={
    '@@VERSION@@':version,'@@BASE@@':attr(base),
    '@@ICON@@':'assets/brand-icon.png','@@MASCOT@@':'assets/mascot.png',
    '@@MONITOR@@':'assets/monitor.png','@@REPLAY@@':'assets/replay-live.jpg','@@EXPORT@@':'assets/export.png',
    '@@SIGNAL@@':Array.from({length:60},(_,i)=>`<i style="--h:${15+Math.round((Math.sin(i*.43)**2*.6+Math.cos(i*.17)**2*.4)*80)}%"></i>`).join(''),
  };
  // Accept the previous vendor placeholders while producing external scripts.
  html=html.replace(/<script>\s*@@GSAP@@\s*<\/script>/g,'<script src="vendor/gsap.min.js" defer></script>')
    .replace(/<script>\s*@@SCROLL@@\s*<\/script>/g,'<script src="vendor/ScrollTrigger.min.js" defer></script>');
  for(const [token,value]of Object.entries(substitutions))html=html.replaceAll(token,()=>value);
  const resourceHashes={};
  for(const name of ['style.css','site.js'])resourceHashes[name]=createHash('sha256').update(await readFile(join(root,'release-site',name))).digest('hex').slice(0,12);
  html=html.replace(/(\b(?:href|src)\s*=\s*)(["'])((?:\.\/)?(?:style\.css|site\.js))(?:\?[^"']*)?\2/g,
    (_,prefix,quote,path)=>`${prefix}${quote}${path}?v=${resourceHashes[path.replace(/^\.\//,'')]}${quote}`);
  if(/@@[A-Z]+@@/.test(html))throw Error('Unresolved release page asset.');
  if(/data:image\//i.test(html)||/<style\b/i.test(html)||/<script\b(?![^>]*\bsrc\s*=)[^>]*>/i.test(html))
    throw Error('Release website images, styles and scripts must use separate local files.');
  for(const [source,target]of [['src/brand-icon.png','release-site/assets/brand-icon.png'],['assets/branding/qingkan-app-icon-v2.png','release-site/assets/mascot.png']]){
    await mkdir(dirname(join(root,target)),{recursive:true});await copyFile(join(root,source),join(root,target));
  }
  const notes=await buildReleaseNotes(root);
  await writeFile(join(root,'release-site/changelog.json'),serializeReleaseNotes(notes));
  await writeFile(join(root,'release-site/index.html'),html);
  return {version,index:join(root,'release-site/index.html'),changelog_entries:notes.entries.length};
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href){
  try{const result=await renderReleaseSite(process.argv[2]||defaultRoot);console.log(`Rendered release website v${result.version} with separate local assets (${result.changelog_entries} changelog entries).`);}
  catch(error){console.error(error.message);process.exitCode=1;}
}
