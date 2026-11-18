// Publish the existing Site using the installed official Sites transport helpers.
import {readFile,mkdir,unlink,writeFile,stat,readdir} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {resolve,join} from 'node:path';
import {homedir} from 'node:os';
const root=resolve(fileURLToPath(new URL('..',import.meta.url)));
const config=JSON.parse(await readFile(join(root,'release-site/publishing.json'),'utf8'));
const version=JSON.parse(await readFile(join(root,'package.json'),'utf8')).version;
const content=join(root,'dist/sites/qingkan-live');
const files=[];
async function inspect(directory){for(const entry of await readdir(directory,{withFileTypes:true})){const path=join(directory,entry.name);if(entry.isDirectory())await inspect(path);else if(entry.isFile())files.push(await stat(path));else throw Error('Only regular release files can be published.');}}
await inspect(content);
// The full installer uses the supported browser folder-upload workflow. Never
// attempt an oversized MCP upload, split the installer, or mark it published.
if(files.some(file=>file.size>16*1024*1024)||files.reduce((sum,file)=>sum+file.size,0)>44*1024*1024){
  console.log(JSON.stringify({status:'browser_upload_required',site_id:config.site_id,url:config.url,folder:content,publish_page:`https://www.lilithsites.com/publish?site=${encodeURIComponent(config.site_id)}`}));
  console.log('安装包超过 MCP 单文件限制。请在原站点管理页选择“重新上传”，上传上述完整文件夹。确认部署、更新清单及安装包验证通过后，再保存发布回执并完成当前版本；重跑不会递增版本。');
  process.exit(2);
}
const skill=process.env.SITES_SKILL_DIR||join(homedir(),'.codex/skills/sites');
const working=join(root,'release-site');process.chdir(working);
process.env.ATLAS_AI_GATEWAY_ENV=config.gateway_env;process.env.C2SITE_GATEWAY_SERVICE=config.gateway_service;
const {callTool}=await import(pathToFileURL(join(skill,'scripts/gateway.mjs')));
const site=await callTool('c2site_site_get',{site_id:config.site_id});
if(site.site.status!=='active'||site.url!==config.url||site.base_path!==config.base_path)throw Error('Site identity, status or returned URL changed. Verify before publishing.');
const privateDirectory=join(working,'.c2site');await mkdir(privateDirectory,{recursive:true,mode:0o700});
if(process.platform==='win32'){
  const identity=execFileSync('whoami',['/user','/fo','csv','/nh'],{encoding:'utf8'});const sid=identity.match(/S-1-5-21-[\d-]+/)?.[0];if(!sid)throw Error('Cannot determine upload ticket owner');
  execFileSync('icacls',[privateDirectory,'/inheritance:r','/grant:r',`*${sid}:(OI)(CI)F`,'*S-1-5-18:(OI)(CI)F'],{stdio:'pipe'});
}
const run=(file,args=[])=>execFileSync(process.execPath,[join(skill,'scripts',file),...args],{stdio:'inherit',env:process.env});
run('bind-site.mjs',[config.site_id]);run('pack-static.mjs',[config.site_id,content]);run('create-revision.mjs',[config.site_id,'--static']);
const ticketPath=join(privateDirectory,'upload-ticket.json'),ticket=JSON.parse(await readFile(ticketPath,'utf8'));
if(ticket.upload)execFileSync('pwsh',['-NoProfile','-File',join(root,'scripts/upload_sites.ps1'),'-Source','.c2site/static.zip','-Ticket','.c2site/upload-ticket.json'],{stdio:'inherit'});
const request=`.c2site/releases/${version}-${ticket.sha256.slice(0,12)}.json`;run('create-deployment.mjs',[config.site_id,ticket.revision_id,request]);
let result=JSON.parse(await readFile(`${request}.result.json`,'utf8'));const deploymentId=result.deployment.deployment_id;let after=0;
for(let attempt=0;attempt<90;attempt++){
  result=await callTool('c2site_deployment_get',{deployment_id:deploymentId,after});after=result.next_cursor??after;console.log(`Sites ${result.deployment.phase}`);
  if(result.deployment.phase==='succeeded'){
    if(result.deployment.url!==config.url)throw Error('Unexpected publication URL');await unlink(ticketPath);
    const base=config.url+config.base_path+'/';const response=await fetch(`${base}latest.json`,{redirect:'error',cache:'no-store'});
    if(!response.ok||!(response.headers.get('content-type')||'').includes('json'))throw Error('Internal update feed requires company/VPN access without a login redirect.');
    const manifest=await response.json(),expected=JSON.parse(await readFile(join(content,'latest.json'),'utf8'));
    if(JSON.stringify(manifest)!==JSON.stringify(expected))throw Error('Published update manifest differs from verified local release.');
    await writeFile(join(root,'release-site/latest.json'),JSON.stringify(manifest,null,2));await mkdir(join(root,'build'),{recursive:true});
    await writeFile(join(root,`build/sites-publication-${version}.json`),JSON.stringify({site_id:config.site_id,deployment_id:deploymentId,url:config.url,version,update_feed:`${base}latest.json`,verified:true},null,2));
    console.log(`Published ${config.url}`);break;
  }
  if(['failed','cancelled'].includes(result.deployment.phase))throw Error(`Sites publication ${result.deployment.phase}: ${deploymentId}`);
  if(attempt===89)throw Error(`Publication pending: ${deploymentId}. Query this deployment before retrying.`);
  await new Promise(done=>setTimeout(done,5000));
}
