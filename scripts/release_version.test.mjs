import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp, mkdir, writeFile, readFile, rm, access, rename, readdir} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {join, resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {prepareRelease, markPublishing, completeRelease, repackFailedRelease, retireUnsubmittedRelease,refreshWebsite} from './release_version.mjs';
import {stageReleaseSite,signedSiteFiles} from './site_bundle.mjs';

async function fixture(t) {
  const parent = resolve(tmpdir());
  const root = await mkdtemp(join(parent, 'qingkan-release-fixture-'));
  t.after(async () => {
    if (!root.startsWith(join(parent, 'qingkan-release-fixture-'))) throw Error('Fixture cleanup escaped its temporary directory.');
    await rm(root, {recursive:true, force:true});
  });
  const files = {
    'package.json': JSON.stringify({name:'h-live', version:'0.3.6'}),
    'package-lock.json': JSON.stringify({version:'0.3.6', packages:{'':{version:'0.3.6'}, 'node_modules/example':{version:'0.3.6'}}}),
    'src-tauri/tauri.conf.json': '{"version":"0.3.6"}',
    'src-tauri/Cargo.toml': '[package]\nname = "douyin-live-monitor"\nversion = "0.3.6"\n',
    'crates/Cargo.toml': '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "0.3.6"\n',
    'src-tauri/Cargo.lock': lock(['douyin-live-monitor','h-live-runtime','qingkan-core','field-offset']),
    'crates/Cargo.lock': lock(['h-live-runtime','qingkan-core','field-offset']) + '\n[[package]]\nname = "h-live-server"\nversion = "0.1.1"\n',
    'src/update-panel.js': 'import currentNotes from "../release-notes/0.3.6.md?raw";\n',
    'release-notes/0.3.6.md': 'previous feature: never copy this',
    'release-site/publishing.json': JSON.stringify({site_id:'fixture-site',url:'https://fixture.internal.example'}),
  };
  for (const [path, content] of Object.entries(files)) await put(root,path,content);
  return root;
}
function lock(names) { return names.map(name => `[[package]]\nname = "${name}"\nversion = "0.3.6"\n`).join('\n'); }
async function put(root, path, value) { await mkdir(join(root,path,'..'),{recursive:true}); await writeFile(join(root,path),value); }
const read = (root,path) => readFile(join(root,path),'utf8');
async function stage(root, version) {
  await put(root,'dist/sites/qingkan-live/latest.json',JSON.stringify({version}));
  await put(root,'dist/sites/qingkan-live/index.html','<!doctype html>fixture');
  await markPublishing(root);
}
async function receipt(root, version) {
  await put(root,`build/sites-publication-${version}.json`,JSON.stringify({version,site_id:'fixture-site',url:'https://fixture.internal.example',deployment_id:'fixture-deployment',verified:true}));
}

test('minor release advances to the next feature version and retries retain that version',async t=>{
  const root=await fixture(t);
  assert.equal((await prepareRelease(root,'','minor')).version,'0.4.0');
  assert.deepEqual(await prepareRelease(root),{version:'0.4.0',phase:'prepared',reused:true});
  await stage(root,'0.4.0');await receipt(root,'0.4.0');await completeRelease(root);
  assert.equal((await prepareRelease(root)).version,'0.4.1');
});

test('one release advances all owned versions once, retries reuse it, success permits the next patch', async t => {
  const root = await fixture(t);
  await put(root,'release-notes/next.md','# 氢看直播 {{version}}\n\n- 本次修复。\n');
  assert.deepEqual(await prepareRelease(root),{version:'0.3.7',phase:'prepared',reused:false});
  assert.deepEqual(await prepareRelease(root),{version:'0.3.7',phase:'prepared',reused:true});
  const npm = JSON.parse(await read(root,'package-lock.json'));
  assert.equal(npm.version,'0.3.7');
  assert.equal(npm.packages[''].version,'0.3.7');
  assert.equal(npm.packages['node_modules/example'].version,'0.3.6');
  assert.match(await read(root,'src-tauri/Cargo.lock'),/name = "field-offset"\nversion = "0\.3\.6"/);
  assert.match(await read(root,'crates/Cargo.lock'),/name = "h-live-server"\nversion = "0\.1\.1"/);
  assert.match(await read(root,'src/update-panel.js'),/0\.3\.7\.md/);
  assert.match(await read(root,'release-notes/0.3.7.md'),/氢看直播 0\.3\.7/);
  await stage(root,'0.3.7');
  assert.deepEqual(await prepareRelease(root),{version:'0.3.7',phase:'publishing',reused:true});
  await receipt(root,'0.3.7');
  await completeRelease(root);
  await assert.rejects(access(join(root,'release-notes/next.md')), {code:'ENOENT'});
  assert.equal((await prepareRelease(root)).version,'0.3.8');
  assert.doesNotMatch(await read(root,'release-notes/0.3.8.md'),/本次修复|previous feature/);
});

test('explicit notes take precedence and are never consumed; a newly edited next.md survives publication',async t => {
  const root = await fixture(t);
  await put(root,'release-notes/next.md','planned next release');
  await put(root,'changes.md','explicit {{version}}');
  await prepareRelease(root,'changes.md');
  assert.equal((await prepareRelease(root,'changes.md')).reused,true);
  assert.equal(await read(root,'release-notes/0.3.7.md'),'explicit 0.3.7');
  await stage(root,'0.3.7');
  await receipt(root,'0.3.7');
  await completeRelease(root);
  assert.equal(await read(root,'changes.md'),'explicit {{version}}');
  assert.equal(await read(root,'release-notes/next.md'),'planned next release');
});

test('inconsistent source versions fail before any version is changed',async t => {
  const root = await fixture(t);
  await put(root,'src-tauri/Cargo.toml','[package]\nversion = "0.3.5"\n');
  await assert.rejects(prepareRelease(root),/differs from package.json/);
  assert.equal(JSON.parse(await read(root,'package.json')).version,'0.3.6');
  await assert.rejects(access(join(root,'.release-secrets/release-state.json')),{code:'ENOENT'});
});

test('pending publication rejects altered payloads and does not advance the version',async t => {
  const root = await fixture(t);
  await prepareRelease(root);
  await stage(root,'0.3.7');
  await put(root,'dist/sites/qingkan-live/index.html','changed after upload');
  await assert.rejects(prepareRelease(root),/Pending Sites files changed/);
  assert.equal(JSON.parse(await read(root,'package.json')).version,'0.3.7');
  await assert.rejects(completeRelease(root),{code:'ENOENT'});
});

test('a terminal failed deployment can reopen the same version without discarding the previous digest',async t=>{
  const root=await fixture(t);await prepareRelease(root);await stage(root,'0.3.7');
  const before=JSON.parse(await read(root,'.release-secrets/release-state.json'));
  await put(root,'build/failed.json',JSON.stringify({current_deployment:'old-success',deployments:[{site_id:'fixture-site',deployment_id:'failed-attempt',phase:'failed',created_at:new Date().toISOString()}]}));
  assert.deepEqual(await repackFailedRelease(root,'build/failed.json'),{version:'0.3.7',phase:'prepared',repacked:true});
  assert.equal((await prepareRelease(root)).version,'0.3.7');
  await put(root,'dist/sites/qingkan-live/index.html','lean rebuilt payload');
  await markPublishing(root);
  assert.notEqual(JSON.parse(await read(root,'.release-secrets/release-state.json')).site_sha256,before.site_sha256);
});

test('repackaging rejects a pending deployment, wrong site and a possibly published version',async t=>{
  const root=await fixture(t);await prepareRelease(root);await stage(root,'0.3.7');
  const deployment={site_id:'fixture-site',deployment_id:'retained-attempt',phase:'running',created_at:new Date().toISOString()};
  await put(root,'build/failed.json',JSON.stringify({deployment}));
  await assert.rejects(repackFailedRelease(root,'build/failed.json'),/terminal failed/);
  deployment.phase='failed';deployment.site_id='another-site';
  await put(root,'build/failed.json',JSON.stringify({deployment}));
  await assert.rejects(repackFailedRelease(root,'build/failed.json'),/terminal failed/);
  deployment.site_id='fixture-site';await put(root,'build/failed.json',JSON.stringify({deployment}));
  await receipt(root,'0.3.7');
  await assert.rejects(repackFailedRelease(root,'build/failed.json'),/publication receipt exists/);
  assert.equal(JSON.parse(await read(root,'.release-secrets/release-state.json')).phase,'publishing');
});

test('interrupted multi-file preparation rolls forward without discarding conflicting edits',async t => {
  const root = await fixture(t);
  await prepareRelease(root);
  const statePath = '.release-secrets/release-state.json';
  const state = JSON.parse(await read(root,statePath));
  state.phase = 'preparing';
  state.edits = [{path:'package.json',before:'old',after:await read(root,'package.json')}];
  await put(root,statePath,JSON.stringify(state));
  assert.equal((await prepareRelease(root)).reused,true);
  state.edits[0].before = 'another old value';
  await put(root,statePath,JSON.stringify(state));
  await put(root,'package.json','conflicting work');
  await assert.rejects(prepareRelease(root),/preparation conflicts/);
  assert.equal(await read(root,'package.json'),'conflicting work');
});

async function unsubmittedReport(root) {
  const state=JSON.parse(await read(root,'.release-secrets/release-state.json'));
  return {site_id:'fixture-site',current_deployment:'previous-success',next_before_sequence:null,deployments:[
    {site_id:'fixture-site',deployment_id:'previous-success',phase:'succeeded',created_at:new Date(Date.parse(state.created_at)-60000).toISOString()},
  ]};
}

test('an unsubmitted frozen release is retired with evidence and the next build gets a new version',async t=>{
  const root=await fixture(t);
  const next='# Feature {{version}}\n\nNew bookmarks and timezone.\n';
  await put(root,'release-notes/next.md',next);
  await prepareRelease(root);await stage(root,'0.3.7');
  const before=JSON.parse(await read(root,'.release-secrets/release-state.json'));
  const notes=await read(root,'release-notes/0.3.7.md');
  const report=JSON.stringify(await unsubmittedReport(root),null,2)+'\n';
  await put(root,'build/history.json',report);
  assert.deepEqual(await retireUnsubmittedRelease(root,'build/history.json'),{version:'0.3.7',phase:'superseded',retired:true});
  const state=JSON.parse(await read(root,'.release-secrets/release-state.json'));
  assert.equal(state.superseded_reason,'unsubmitted');assert.equal(state.site_sha256,before.site_sha256);
  assert.equal(state.deployment_id,undefined);assert.equal(state.published_at,undefined);
  assert.deepEqual(JSON.parse(await read(root,`${state.retirement_backup}/release-state.json`)).state,before);
  assert.equal(await read(root,`${state.retirement_backup}/unsubmitted-deployments.json`),report);
  assert.equal(await read(root,`${state.retirement_backup}/site/index.html`),'<!doctype html>fixture');
  assert.equal(await read(root,'release-notes/next.md'),next);
  assert.equal(await read(root,'release-notes/0.3.7.md'),notes);
  assert.equal(JSON.parse(await read(root,'package.json')).version,'0.3.7');
  await assert.rejects(completeRelease(root),/No publishing release/);
  assert.deepEqual(await prepareRelease(root),{version:'0.3.8',phase:'prepared',reused:false});
  assert.match(await read(root,'release-notes/0.3.8.md'),/New bookmarks and timezone/);
  await assert.rejects(markPublishing(root),/manifest version differs/,'retirement cannot relabel the old frozen payload as the new release');
});

test('retirement rejects incomplete, newer, unknown or mismatched official history without changing state',async t=>{
  const root=await fixture(t);await prepareRelease(root);await stage(root,'0.3.7');
  const state=await read(root,'.release-secrets/release-state.json');
  const mutateCases=[
    report=>{delete report.next_before_sequence;},
    report=>{report.next_before_sequence=123;},
    report=>{report.site_id='another-site';},
    report=>{report.deployments=[];},
    report=>{report.current_deployment='not-in-history';},
    report=>{report.deployments[0].phase='failed';},
    report=>{report.deployments[0].site_id='another-site';},
    report=>{report.deployments[0].phase='running';},
    report=>{report.deployments[0].phase='unknown';},
    report=>{delete report.deployments[0].created_at;},
    report=>{report.deployments[0].created_at='2026-02-31T00:00:00Z';},
    report=>{report.deployments[0].created_at=new Date(Date.parse(JSON.parse(state).created_at)+1).toISOString();},
    report=>{report.deployments.push({...report.deployments[0]});},
  ];
  for(const mutate of mutateCases){
    const report=await unsubmittedReport(root);mutate(report);
    await put(root,'build/history.json',JSON.stringify(report));
    await assert.rejects(retireUnsubmittedRelease(root,'build/history.json'));
    assert.equal(await read(root,'.release-secrets/release-state.json'),state);
  }
  await put(root,'build/history.json',JSON.stringify(await unsubmittedReport(root)));
  await receipt(root,'0.3.7');
  await assert.rejects(retireUnsubmittedRelease(root,'build/history.json'),/publication receipt exists/);
  assert.equal(await read(root,'.release-secrets/release-state.json'),state);
});

test('retirement accepts official SQL microsecond timestamps and still rejects one newer microsecond',async t=>{
  const root=await fixture(t);await prepareRelease(root);await stage(root,'0.3.7');
  const state=JSON.parse(await read(root,'.release-secrets/release-state.json'));
  state.created_at='2026-10-23T17:42:03.406Z';await put(root,'.release-secrets/release-state.json',JSON.stringify(state));
  const report=await unsubmittedReport(root);
  report.deployments[0].created_at='2026-10-24 01:42:03.406001+08';
  await put(root,'build/history.json',JSON.stringify(report));
  await assert.rejects(retireUnsubmittedRelease(root,'build/history.json'),/newer/);
  report.deployments[0].created_at='2026-10-24 01:17:10.236372+08';
  await put(root,'build/history.json',JSON.stringify(report));
  assert.equal((await retireUnsubmittedRelease(root,'build/history.json')).phase,'superseded');
});

test('retirement refuses changed frozen artifacts and releases that have not entered publishing',async t=>{
  const root=await fixture(t);await prepareRelease(root);
  await put(root,'build/history.json',JSON.stringify(await unsubmittedReport(root)));
  await assert.rejects(retireUnsubmittedRelease(root,'build/history.json'),/pending publishing/);
  await stage(root,'0.3.7');
  await put(root,'dist/sites/qingkan-live/index.html','changed after freeze');
  await assert.rejects(retireUnsubmittedRelease(root,'build/history.json'),/frozen payload/);
  assert.equal(JSON.parse(await read(root,'.release-secrets/release-state.json')).phase,'publishing');
});

async function websiteFixture(t){
  const root=await fixture(t);await prepareRelease(root);
  const version='0.3.7',installer=`${version}/QingkanLive-${version}-windows-x64-setup.exe`;
  const resources={
    'release-site/page.html':'<!doctype html><link rel="stylesheet" href="style.css"><img src="@@ICON@@"><img src="@@MASCOT@@"><script src="vendor/gsap.min.js" defer></script><script type="module" src="site.js"></script><p>old website @@VERSION@@</p>',
    'release-site/style.css':'body{color:green}', 'release-site/site.js':'console.log("external site")',
    'release-site/vendor/gsap.min.js':'window.gsap={};', 'release-site/assets/monitor.png':'original screenshot',
    'src/brand-icon.png':'icon','assets/branding/qingkan-app-icon-v2.png':'mascot',
    'dist/cdn/qingkan-live/latest.json':JSON.stringify({version,platforms:{'windows-x86_64':{url:`https://fixture.internal.example/${installer}`,signature:'fixture-signature'}}},null,2)+'\n',
    [`dist/cdn/qingkan-live/${installer}`]:'signed installer bytes\0',
    [`dist/cdn/qingkan-live/${installer}.sig`]:'fixture-signature\n',
    [`dist/cdn/qingkan-live/${installer}.sha256`]:'signed-checksum\n',
  };
  for(const [path,value]of Object.entries(resources))await put(root,path,value);
  await stageReleaseSite(root);await markPublishing(root);
  return root;
}
test('website-only revisions snapshot a frozen release and keep signed artifacts, versions and notes unchanged',async t=>{
  const root=await websiteFixture(t),before=JSON.parse(await read(root,'.release-secrets/release-state.json'));
  const versions=await read(root,'package.json'),notes=await read(root,'release-notes/0.3.7.md');
  await put(root,'release-site/page.html',(await read(root,'release-site/page.html')).replace('old website','new website'));
  await put(root,'dist/cdn/qingkan-live/0.3.7/QingkanLive-0.3.7-windows-x64-setup.exe','accidental rebuilt installer ignored');
  const result=await refreshWebsite(root),after=JSON.parse(await read(root,'.release-secrets/release-state.json'));
  assert.equal(result.version,'0.3.7');assert.equal(result.website_revision,2);assert.equal(after.phase,'publishing');
  assert.notEqual(after.site_sha256,before.site_sha256);assert.equal(after.website_previous_sha256,before.site_sha256);
  assert.deepEqual(JSON.parse(await read(root,`${result.backup}/release-state.json`)),before);
  assert.match(await read(root,`${result.backup}/site/index.html`),/old website/);assert.match(await read(root,'dist/sites/qingkan-live/index.html'),/new website/);
  for(const file of signedSiteFiles('0.3.7'))assert.deepEqual(await readFile(join(root,'dist/sites/qingkan-live',file)),await readFile(join(root,result.backup,'site',file)));
  assert.equal(await read(root,'package.json'),versions);assert.equal(await read(root,'release-notes/0.3.7.md'),notes);
  assert.deepEqual(await prepareRelease(root),{version:'0.3.7',phase:'publishing',reused:true});
  await receipt(root,'0.3.7');await completeRelease(root);
});
test('failed website staging leaves the original frozen site and state intact with a private snapshot',async t=>{
  const root=await websiteFixture(t),before=await read(root,'.release-secrets/release-state.json'),html=await read(root,'dist/sites/qingkan-live/index.html');
  await rm(join(root,'release-site/site.js'));
  await assert.rejects(refreshWebsite(root),{code:'ENOENT'});
  assert.equal(await read(root,'.release-secrets/release-state.json'),before);assert.equal(await read(root,'dist/sites/qingkan-live/index.html'),html);
  const attempts=await readdir(join(root,'.release-secrets/release-attempts'));assert.equal(attempts.length,1);
  assert.equal(await read(root,`.release-secrets/release-attempts/${attempts[0]}/release-state.json`),before);
});
test('website refresh rejects modified frozen data or an existing publication receipt before a state change',async t=>{
  const root=await websiteFixture(t),before=await read(root,'.release-secrets/release-state.json');
  await receipt(root,'0.3.7');await assert.rejects(refreshWebsite(root),/receipt already exists/);
  await rm(join(root,'build/sites-publication-0.3.7.json'));
  await put(root,'dist/sites/qingkan-live/index.html','changed after freezing');
  await assert.rejects(refreshWebsite(root),/frozen website changed/);assert.equal(await read(root,'.release-secrets/release-state.json'),before);
});
test('website revision journal recovers after the old directory was renamed and ordinary retries stay blocked',async t=>{
  const root=await websiteFixture(t);
  await put(root,'release-site/page.html',(await read(root,'release-site/page.html')).replace('old website','new website'));
  const result=await refreshWebsite(root),after=JSON.parse(await read(root,'.release-secrets/release-state.json'));
  const before=JSON.parse(await read(root,`${result.backup}/release-state.json`));
  const signed={};for(const file of signedSiteFiles('0.3.7'))signed[file]=createHash('sha256').update(await readFile(join(root,result.backup,'site',file))).digest('hex');
  before.website_refresh={format:1,revision:2,created_at:after.website_revised_at,directory:result.backup,previous_sha256:before.site_sha256,next_sha256:after.site_sha256,signed_sha256:signed};
  await rename(join(root,'dist/sites/qingkan-live'),join(root,result.backup,'staged-site'));
  await put(root,'.release-secrets/release-state.json',JSON.stringify(before));
  await assert.rejects(prepareRelease(root),/website revision is interrupted/);
  await assert.rejects(completeRelease(root),/retained website revision/);
  const recovered=await refreshWebsite(root);assert.equal(recovered.website_revision,2);assert.equal(recovered.site_sha256,after.site_sha256);
  assert.match(await read(root,'dist/sites/qingkan-live/index.html'),/new website/);
  assert.equal(JSON.parse(await read(root,'.release-secrets/release-state.json')).website_refresh,undefined);
  assert.equal((await prepareRelease(root)).reused,true);
});
