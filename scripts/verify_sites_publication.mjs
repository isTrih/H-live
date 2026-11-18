// Finish a browser-folder publication using official deployment state and the
// exact signed local payload. No browser session or upload credentials needed.
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {resolve,join} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {homedir} from 'node:os';
const root=resolve(fileURLToPath(new URL('..',import.meta.url)));
const config=JSON.parse(await readFile(join(root,'release-site/publishing.json'),'utf8'));
const {version}=JSON.parse(await readFile(join(root,'package.json'),'utf8'));
const directory=join(root,'dist/sites/qingkan-live');
const expected=JSON.parse(await readFile(join(directory,'latest.json'),'utf8'));
if(expected.version!==version)throw Error('Local site and application versions differ.');
process.env.ATLAS_AI_GATEWAY_ENV=config.gateway_env;
process.env.C2SITE_GATEWAY_SERVICE=config.gateway_service;
const skill=process.env.SITES_SKILL_DIR||join(homedir(),'.codex/skills/sites');
const {callTool}=await import(pathToFileURL(join(skill,'scripts/gateway.mjs')));
const site=await callTool('c2site_site_get',{site_id:config.site_id});
if(site.site.status!=='active'||site.url!==config.url||site.base_path!==config.base_path)throw Error('Unexpected Site identity or status.');
const deploymentId=site.site.current_deployment;
if(!deploymentId)throw Error('Site has no active deployment.');
const result=await callTool('c2site_deployment_get',{deployment_id:deploymentId,after:0});
if(result.deployment.phase!=='succeeded')throw Error('Current deployment has not succeeded.');
const base=config.url+config.base_path+'/';
async function readPublic(url){
  const response=await fetch(url,{redirect:'error',cache:'no-store',signal:AbortSignal.timeout(120000)});
  if(!response.ok)throw Error(`Published file returned HTTP ${response.status}.`);
  return response;
}
const manifest=await (await readPublic(base+'latest.json')).json();
if(JSON.stringify(manifest)!==JSON.stringify(expected))throw Error('Published update feed differs from verified local payload.');
const page=Buffer.from(await (await readPublic(base)).arrayBuffer());
const localPage=await readFile(join(directory,'index.html'));
const digest=bytes=>createHash('sha256').update(bytes).digest('hex');
if(digest(page)!==digest(localPage))throw Error('Published HTML differs from the local release page.');
const platform=manifest.platforms['windows-x86_64'];
const expectedUrl=`${base}${version}/QingkanLive-${version}-windows-x64-setup.exe`;
if(platform.url!==expectedUrl)throw Error('Unexpected installer download URL.');
const filename=`QingkanLive-${version}-windows-x64-setup.exe`;
const expectedHash=(await readFile(join(directory,version,filename+'.sha256'),'utf8')).trim().split(/\s+/)[0];
const response=await readPublic(expectedUrl);const hash=createHash('sha256');let bytes=0;
for await(const chunk of response.body){bytes+=chunk.length;hash.update(chunk);}
const actualHash=hash.digest('hex');
if(actualHash!==expectedHash)throw Error('Published installer hash differs from the signed local artifact.');
const receipt={site_id:config.site_id,deployment_id:deploymentId,url:config.url,version,update_feed:base+'latest.json',verified:true,installer_bytes:bytes,installer_sha256:actualHash,html_sha256:digest(page),verified_at:new Date().toISOString()};
await mkdir(join(root,'build'),{recursive:true});
await writeFile(join(root,`build/sites-publication-${version}.json`),JSON.stringify(receipt,null,2)+'\n');
await writeFile(join(root,'release-site/latest.json'),JSON.stringify(manifest,null,2)+'\n');
console.log(JSON.stringify(receipt,null,2));
