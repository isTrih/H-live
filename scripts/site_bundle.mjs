// Static website whitelist shared by normal releases and website-only revisions.
import {readFile,mkdir,copyFile,readdir,lstat,rm} from 'node:fs/promises';
import {resolve,join,dirname,relative,isAbsolute} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {renderReleaseSite} from './render_release_site.mjs';

const defaultRoot=fileURLToPath(new URL('../',import.meta.url));
const fontFiles=['fonts/SmileySans-Oblique.woff2','fonts/SmileySans-OFL.txt','fonts/SmileySans-SOURCE.txt'];
export function signedSiteFiles(version){
  if(!/^\d+\.\d+\.\d+$/.test(version))throw Error('Invalid signed release version.');
  const installer=`${version}/QingkanLive-${version}-windows-x64-setup.exe`;
  return ['latest.json',installer,`${installer}.sig`,`${installer}.sha256`];
}
function inside(parent,path){const part=relative(parent,path);return Boolean(part)&&!part.startsWith('..')&&!isAbsolute(part);}
async function regular(path){if(!(await lstat(path)).isFile())throw Error(`Website resource must be a regular file: ${path}`);}
async function noLinks(root,path){
  if(path!==root&&!inside(root,path))throw Error('Website path escaped workspace.');
  let current=root;
  for(const segment of relative(root,path).split(/[\\/]/).filter(Boolean)){
    current=join(current,segment);
    try{if((await lstat(current)).isSymbolicLink())throw Error('Website paths cannot contain symbolic links.');}
    catch(error){if(error.code==='ENOENT')break;throw error;}
  }
}
async function copyRegular(source,target){await regular(source);await mkdir(dirname(target),{recursive:true});await copyFile(source,target);}
async function copyAssets(source,target){
  for(const item of await readdir(source,{withFileTypes:true})){
    if(item.name.startsWith('.'))throw Error('Hidden files are not website image assets.');
    const from=join(source,item.name),to=join(target,item.name);
    if(item.isDirectory())await copyAssets(from,to);
    else if(item.isFile()&&/\.(?:png|jpe?g|webp|gif|avif|svg|ico)$/i.test(item.name))await copyRegular(from,to);
    else throw Error('Only regular image assets can be included in the website assets directory.');
  }
}
export async function stageReleaseSite(root=defaultRoot,{outputDirectory,artifactDirectory,allowPending=false}={}){
  root=resolve(root);
  const output=resolve(outputDirectory||join(root,'dist/sites/qingkan-live'));
  const frozen=join(root,'dist/sites/qingkan-live');
  if(!inside(join(root,'dist/sites'),output)&&!inside(join(root,'.release-secrets/release-attempts'),output))throw Error('Website output must be inside a release staging directory.');
  if(allowPending&&output===frozen)throw Error('Website revisions must stage outside the frozen publication directory.');
  let state;
  try{state=JSON.parse(await readFile(join(root,'.release-secrets/release-state.json'),'utf8'));}catch(error){if(error.code!=='ENOENT')throw error;}
  if(state?.phase==='publishing'&&!allowPending)throw Error('A frozen publication is pending. Use scripts/refresh_site.ps1 for an explicit website-only revision.');
  await noLinks(root,output);
  const rendered=await renderReleaseSite(root);
  const artifacts=resolve(artifactDirectory||join(root,'dist/cdn/qingkan-live'));
  if(!inside(root,artifacts))throw Error('Signed website artifacts escaped workspace.');
  await noLinks(root,artifacts);
  const site=JSON.parse(await readFile(join(root,'release-site/publishing.json'),'utf8'));
  const base=site.url.replace(/\/$/,'')+(site.base_path||'').replace(/\/$/,'')+'/';
  const manifest=JSON.parse(await readFile(join(artifacts,'latest.json'),'utf8'));
  if(manifest.version!==rendered.version||manifest.platforms?.['windows-x86_64']?.url!==`${base}${rendered.version}/QingkanLive-${rendered.version}-windows-x64-setup.exe`)
    throw Error('Site manifest must match the configured signed release.');
  // Resolve and verify every input before any existing output is removed.
  const website=join(root,'release-site');
  const files=['index.html','style.css','site.js','changelog.json','vendor/gsap.min.js'];
  const html=await readFile(join(website,'index.html'),'utf8');
  const css=await readFile(join(website,'style.css'),'utf8');
  const fontReferences=[...(html+'\n'+css).matchAll(/\bfonts\/[^"'()\s<>]+/g)].map(match=>match[0].split(/[?#]/)[0]);
  if(fontReferences.some(path=>!fontFiles.includes(path)))throw Error('Website font references must match the approved local font whitelist.');
  let fontsPresent=false;
  try{const stat=await lstat(join(website,'fonts'));if(!stat.isDirectory())throw Error('Website fonts must be a regular directory.');fontsPresent=true;}
  catch(error){if(error.code!=='ENOENT')throw error;}
  if(fontsPresent||fontReferences.length)files.push(...fontFiles);
  if(html.includes('vendor/ScrollTrigger.min.js'))files.push('vendor/ScrollTrigger.min.js');
  for(const name of files){await noLinks(root,join(website,name));await regular(join(website,name));}
  await noLinks(root,join(website,'assets'));
  for(const name of signedSiteFiles(rendered.version)){await noLinks(root,join(artifacts,name));await regular(join(artifacts,name));}
  // The absolute target was checked above and cannot refer to the workspace root.
  await rm(output,{recursive:true,force:true});await mkdir(output,{recursive:true});
  for(const name of files)await copyRegular(join(website,name),join(output,name));
  await copyAssets(join(website,'assets'),join(output,'assets'));
  for(const name of signedSiteFiles(rendered.version))await copyRegular(join(artifacts,name),join(output,name));
  return {version:rendered.version,directory:output};
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href){
  try{if(process.argv.length>3)throw Error('Usage: node site_bundle.mjs [ROOT]');console.log((await stageReleaseSite(process.argv[2]||defaultRoot)).directory);}
  catch(error){console.error(error.message);process.exitCode=1;}
}
