import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,mkdir,readFile,writeFile,rm,access} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {resolve,join,dirname} from 'node:path';
import {createHash} from 'node:crypto';
import {renderReleaseSite} from './render_release_site.mjs';
import {stageReleaseSite,signedSiteFiles} from './site_bundle.mjs';

const read=(root,path)=>readFile(join(root,path),'utf8');
const put=async(root,path,value)=>{await mkdir(dirname(join(root,path)),{recursive:true});await writeFile(join(root,path),value);};
async function fixture(t){
  const parent=resolve(tmpdir()),root=await mkdtemp(join(parent,'qingkan-site-fixture-'));
  t.after(async()=>{if(!root.startsWith(join(parent,'qingkan-site-fixture-')))throw Error('Unsafe fixture cleanup.');await rm(root,{recursive:true,force:true});});
  const manifest={version:'0.4.5',platforms:{'windows-x86_64':{url:'https://fixture.internal.example/site/0.4.5/QingkanLive-0.4.5-windows-x64-setup.exe',signature:'signed-original'}}};
  const files={
    'package.json':JSON.stringify({version:'0.4.5'}),
    'release-site/publishing.json':JSON.stringify({site_id:'fixture',url:'https://fixture.internal.example',base_path:'/site'}),
    'release-site/page.html':'<!doctype html><html><head><meta name="release-base" content="@@BASE@@"><link rel="icon" href="@@ICON@@"><link rel="stylesheet" href="style.css"></head><body>@@VERSION@@<img src="@@MASCOT@@"><img src="@@MONITOR@@"><img src="@@REPLAY@@"><img src="@@EXPORT@@"><script>@@GSAP@@</script><script type="module" src="site.js"></script></body></html>',
    'release-site/style.css':'body{color:white}',
    'release-site/site.js':'console.log("site")',
    'release-site/vendor/gsap.min.js':'window.gsap={fixture:true};',
    'release-site/vendor/ScrollTrigger.min.js':'window.ScrollTrigger={};',
    'release-site/assets/monitor.png':'monitor-image','release-site/assets/replay-live.jpg':'replay-image','release-site/assets/export.png':'export-image',
    'src/brand-icon.png':'icon-image','assets/branding/qingkan-app-icon-v2.png':'mascot-image',
    'release-site/.c2site/upload-ticket.json':'never upload this',
    'release-notes/0.4.5.md':'# Latest\n\n- First\n\n## Complete details\n\nParagraph after first bullets.\n',
    'release-notes/0.1.0.md':'# First release\n\nThe entire original release.\n',
    'dist/cdn/qingkan-live/latest.json':JSON.stringify(manifest,null,2)+'\n',
    'dist/cdn/qingkan-live/0.4.5/QingkanLive-0.4.5-windows-x64-setup.exe':'installer original bytes\0\xff',
    'dist/cdn/qingkan-live/0.4.5/QingkanLive-0.4.5-windows-x64-setup.exe.sig':'signed-original\n',
    'dist/cdn/qingkan-live/0.4.5/QingkanLive-0.4.5-windows-x64-setup.exe.sha256':'original checksum\n',
  };
  for(const [path,value]of Object.entries(files))await put(root,path,value);
  return root;
}
test('renderer writes relative assets and complete independent changelog data, without inlined payloads',async t=>{
  const root=await fixture(t),result=await renderReleaseSite(root);
  assert.equal(result.version,'0.4.5');assert.equal(result.changelog_entries,2);
  const html=await read(root,'release-site/index.html');
  for(const path of ['assets/brand-icon.png','assets/mascot.png','assets/monitor.png','assets/replay-live.jpg','assets/export.png','vendor/gsap.min.js','site.js'])assert.ok(html.includes(path));
  assert.doesNotMatch(html,/data:image|@@[A-Z]+@@|window\.gsap/);
  assert.equal(await read(root,'release-site/assets/brand-icon.png'),'icon-image');
  assert.equal(await read(root,'release-site/assets/mascot.png'),'mascot-image');
  const notes=JSON.parse(await read(root,'release-site/changelog.json'));
  assert.equal(notes.entries.length,2);assert.match(notes.entries[0].body,/Paragraph after first bullets/);assert.equal(notes.entries.at(-1).version,'0.1.0');
});
test('renderer rejects embedded scripts, styles and image data before generating a new index',async t=>{
  const root=await fixture(t);
  for(const unsafe of ['<script>window.inline=true</script>','<style>body{color:red}</style>','<img src="data:image/png;base64,abc">']){
    await put(root,'release-site/page.html',`<html>${unsafe}</html>`);
    await assert.rejects(renderReleaseSite(root),/separate local files/);
  }
  await assert.rejects(access(join(root,'release-site/index.html')),{code:'ENOENT'});
});
test('CSS and JavaScript URLs carry stable content hashes that change only with their file bytes',async t=>{
  const root=await fixture(t);
  await put(root,'release-site/page.html','<!doctype html><link href="./style.css?v=stale" rel="stylesheet"><script type="module" src=\'site.js\'></script>');
  await renderReleaseSite(root);
  const hash=async name=>createHash('sha256').update(await readFile(join(root,'release-site',name))).digest('hex').slice(0,12);
  const css=await hash('style.css'),js=await hash('site.js');
  const original=await read(root,'release-site/index.html');
  assert.ok(original.includes(`href="./style.css?v=${css}"`));assert.ok(original.includes(`src='site.js?v=${js}'`));assert.doesNotMatch(original,/stale/);
  await renderReleaseSite(root);assert.equal(await read(root,'release-site/index.html'),original);
  await put(root,'release-site/site.js','console.log("revised same-version website")');await renderReleaseSite(root);
  const updated=await read(root,'release-site/index.html');
  assert.ok(updated.includes(`href="./style.css?v=${css}"`));assert.ok(updated.includes(`src='site.js?v=${await hash('site.js')}'`));assert.ok(!updated.includes(`site.js?v=${js}`));
});
test('normal preparation copies only public resources and preserves all signed bytes',async t=>{
  const root=await fixture(t),result=await stageReleaseSite(root);
  const directory=result.directory;
  for(const file of ['index.html','style.css','site.js','changelog.json','assets/monitor.png','assets/brand-icon.png','assets/mascot.png','vendor/gsap.min.js'])await access(join(directory,file));
  for(const file of ['publishing.json','page.html','.c2site','vendor/ScrollTrigger.min.js'])await assert.rejects(access(join(directory,file)),{code:'ENOENT'});
  for(const file of signedSiteFiles('0.4.5'))assert.deepEqual(await readFile(join(directory,file)),await readFile(join(root,'dist/cdn/qingkan-live',file)));
});
test('ordinary preparation cannot overwrite publishing state and staging cannot escape the workspace',async t=>{
  const root=await fixture(t);await stageReleaseSite(root);
  const index=await read(root,'dist/sites/qingkan-live/index.html');
  await put(root,'.release-secrets/release-state.json',JSON.stringify({phase:'publishing'}));
  await assert.rejects(stageReleaseSite(root),/frozen publication is pending/);
  await assert.rejects(stageReleaseSite(root,{allowPending:true}),/stage outside/);
  await assert.rejects(stageReleaseSite(root,{outputDirectory:join(root,'..','outside')}),/release staging directory/);
  assert.equal(await read(root,'dist/sites/qingkan-live/index.html'),index);
});
test('font packaging copies the exact complete font and license whitelist while excluding unrelated files',async t=>{
  const root=await fixture(t);
  await put(root,'release-site/style.css','@font-face{font-family:heading;src:url("./fonts/SmileySans-Oblique.woff2?v=content")}');
  const allowed={'SmileySans-Oblique.woff2':'wOF2 fixture font','SmileySans-OFL.txt':'fixture OFL license','SmileySans-SOURCE.txt':'fixture upstream source'};
  for(const [file,content]of Object.entries(allowed))await put(root,`release-site/fonts/${file}`,content);
  await put(root,'release-site/fonts/private.txt','never include arbitrary font-directory text');
  await put(root,'release-site/fonts/other.woff2','unapproved font');
  const staged=await stageReleaseSite(root);
  for(const [file,content]of Object.entries(allowed))assert.equal(await readFile(join(staged.directory,'fonts',file),'utf8'),content);
  for(const file of ['private.txt','other.woff2'])await assert.rejects(access(join(staged.directory,'fonts',file)),{code:'ENOENT'});
  const before=await read(root,'dist/sites/qingkan-live/index.html');
  await rm(join(root,'release-site/fonts/SmileySans-OFL.txt'));
  await assert.rejects(stageReleaseSite(root),{code:'ENOENT'});
  assert.equal(await read(root,'dist/sites/qingkan-live/index.html'),before);
});
test('unapproved or missing font references fail before replacing an existing website bundle',async t=>{
  const root=await fixture(t);await stageReleaseSite(root);const before=await read(root,'dist/sites/qingkan-live/index.html');
  await put(root,'release-site/style.css','@font-face{src:url("./fonts/unapproved.woff2")}');
  await assert.rejects(stageReleaseSite(root),/approved local font whitelist/);
  await put(root,'release-site/style.css','@font-face{src:url("./fonts/SmileySans-Oblique.woff2")}');
  await assert.rejects(stageReleaseSite(root),{code:'ENOENT'});
  assert.equal(await read(root,'dist/sites/qingkan-live/index.html'),before);
});
