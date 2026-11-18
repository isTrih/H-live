// Local release transaction. No network, Cargo resolution or dependency updates.
import {readFile, writeFile, rename, mkdir, unlink, readdir, cp, lstat} from 'node:fs/promises';
import {createReadStream} from 'node:fs';
import {createHash, randomUUID} from 'node:crypto';
import {resolve, dirname, join, relative, isAbsolute} from 'node:path';
import {pathToFileURL} from 'node:url';

const stateFile = root => join(root, '.release-secrets/release-state.json');
const digest = value => createHash('sha256').update(value).digest('hex');
async function fileDigest(path) {const hash=createHash('sha256');for await(const chunk of createReadStream(path))hash.update(chunk);return hash.digest('hex');}
const semver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const text = path => readFile(path, 'utf8');
async function optional(path) { try { return await text(path); } catch (error) { if (error.code === 'ENOENT') return null; throw error; } }
async function atomic(path, value) {
  await mkdir(dirname(path), {recursive:true});
  const temporary = `${path}.${randomUUID()}.tmp`;
  await writeFile(temporary, value, {flag:'wx', mode:0o600});
  await rename(temporary, path);
}
async function save(root, state) { await atomic(stateFile(root), JSON.stringify(state, null, 2) + '\n'); }
async function readState(root) {
  const source = await optional(stateFile(root));
  if (source === null) return null;
  const state = JSON.parse(source);
  if (state.format !== 1 || !semver.test(state.version) || !['preparing','prepared','publishing','published','superseded'].includes(state.phase)) throw Error('Unrecognized local release state; retain it and resolve before publishing.');
  return state;
}
function replaceOne(source, pattern, value, label) {
  let count = 0;
  const result = source.replace(pattern, (...args) => { count++; return value(...args); });
  if (count !== 1) throw Error(`Expected one ${label}; found ${count}.`);
  return result;
}
function jsonVersion(source, from, to, label) {
  return replaceOne(source, /("version"\s*:\s*")([^"\r\n]+)(")/, (_, prefix, found, suffix) => {
    if (found !== from) throw Error(`${label} version differs from package.json.`);
    return prefix + to + suffix;
  }, label);
}
function manifestVersion(source, section, from, to, label) {
  return replaceOne(source, new RegExp(`(\\[${section}\\][\\s\\S]*?^version\\s*=\\s*")([^"\\r\\n]+)(")`, 'm'), (_, prefix, found, suffix) => {
    if (found !== from) throw Error(`${label} version differs from package.json.`);
    return prefix + to + suffix;
  }, label);
}
function lockVersions(source, names, from, to, label) {
  const seen = new Set();
  const result = source.replace(/(\[\[package\]\]\r?\nname = "([^"]+)"\r?\nversion = ")([^"\r\n]+)(")/g, (whole, prefix, name, found, suffix) => {
    if (!names.includes(name)) return whole;
    if (found !== from || seen.has(name)) throw Error(`${label}: unexpected local package ${name} version.`);
    seen.add(name);
    return prefix + to + suffix;
  });
  if (seen.size !== names.length) throw Error(`${label}: a required local package is missing.`);
  return result;
}
async function sourcePlan(root, from, to, notes) {
  const edits = [];
  async function edit(path, transform) {
    const before = await text(join(root, path));
    edits.push({path, before, after:transform(before)});
  }
  await edit('package.json', source => jsonVersion(source, from, to, 'package.json'));
  await edit('package-lock.json', source => {
    const parsed = JSON.parse(source);
    if (parsed.version !== from || parsed.packages?.['']?.version !== from) throw Error('npm lockfile root versions differ from package.json.');
    // Only the document and empty-path root package; dependency versions stay untouched.
    let result = jsonVersion(source, from, to, 'npm lockfile');
    result = replaceOne(result, /(""\s*:\s*\{[\s\S]*?"version"\s*:\s*")([^"\r\n]+)(")/, (_, prefix, found, suffix) => {
      if (found !== from) throw Error('npm lockfile root changed unexpectedly.');
      return prefix + to + suffix;
    }, 'npm root package');
    return result;
  });
  await edit('src-tauri/tauri.conf.json', source => jsonVersion(source, from, to, 'Tauri config'));
  await edit('src-tauri/Cargo.toml', source => manifestVersion(source, 'package', from, to, 'desktop Cargo manifest'));
  await edit('crates/Cargo.toml', source => manifestVersion(source, 'workspace\\.package', from, to, 'Rust workspace manifest'));
  await edit('src-tauri/Cargo.lock', source => lockVersions(source, ['douyin-live-monitor','h-live-runtime','qingkan-core'], from, to, 'desktop Cargo lockfile'));
  await edit('crates/Cargo.lock', source => lockVersions(source, ['h-live-runtime','qingkan-core'], from, to, 'workspace Cargo lockfile'));
  await edit('src/update-panel.js', source => replaceOne(source, /((?:from\s+["'])\.\.\/release-notes\/)([^/"']+)(\.md\?raw["'])/, (_, prefix, found, suffix) => {
    if (found !== from) throw Error('Bundled release notes differ from the application version.');
    return prefix + to + suffix;
  }, 'bundled release notes import'));
  const path = `release-notes/${to}.md`;
  const before = await optional(join(root, path));
  if (before !== null && before !== notes) throw Error(`Release notes ${path} already exist. Use -NotesPath to explicitly select them.`);
  // Install notes first so the dev server never sees an import for an absent file.
  edits.unshift({path, before, after:notes});
  return edits;
}
async function recoverPreparation(root, state) {
  for (const edit of state.edits) {
    const path = join(root, edit.path);
    const current = await optional(path);
    if (current === edit.after) continue;
    if (current !== edit.before) throw Error(`Interrupted version preparation conflicts with ${edit.path}. Retain release-state.json and resolve the edit.`);
    await atomic(path, edit.after);
  }
  state.phase = 'prepared';
  delete state.edits;
  await save(root, state);
}
async function validatePrepared(root, state) {
  if (state.website_refresh) throw Error('A website revision is interrupted. Run scripts/refresh_site.ps1 to finish it before ordinary publication.');
  const version = JSON.parse(await text(join(root, 'package.json'))).version;
  if (version !== state.version) throw Error(`Pending release ${state.version} differs from the working tree. Resume that release before changing versions.`);
  // Generate an unchanged plan to validate every version without rewriting files.
  await sourcePlan(root, version, version, await text(join(root, `release-notes/${version}.md`)));
  if (digest(await text(join(root, `release-notes/${version}.md`))) !== state.notes_sha256) throw Error('Pending release notes changed. Finish or explicitly resolve the pending release before replacing them.');
}
async function treeDigest(root, siteDirectory = join(root, 'dist/sites/qingkan-live')) {
  const directory = siteDirectory;
  if (!(await lstat(directory)).isDirectory()) throw Error('Release site must be a regular directory, never a symbolic link.');
  const entries = [];
  async function visit(path) {
    for (const item of (await readdir(path, {withFileTypes:true})).sort((a,b) => a.name.localeCompare(b.name))) {
      const child = join(path, item.name);
      if (item.isDirectory()) await visit(child);
      else if (item.isFile()) entries.push([relative(directory, child).replaceAll('\\','/'), await fileDigest(child)]);
      else throw Error('Release site contains a non-regular file.');
    }
  }
  await visit(directory);
  return digest(JSON.stringify(entries));
}

export async function prepareRelease(root, notesPath = '', bump = 'patch') {
  if (!['patch','minor','major'].includes(bump)) throw Error('Version bump must be patch, minor or major.');
  root = resolve(root);
  let state = await readState(root);
  if (state && !['published','superseded'].includes(state.phase)) {
    if (notesPath) {
      const supplied = (await text(resolve(root, notesPath))).replaceAll('{{version}}', state.version).replaceAll('\r\n','\n').replaceAll('\r','\n');
      if (digest(supplied) !== state.notes_sha256) throw Error('The supplied notes differ from the pending release. Retry with the original notes.');
    }
    if (state.phase === 'preparing') await recoverPreparation(root, state);
    await validatePrepared(root, state);
    if (state.phase === 'publishing' && state.site_sha256 !== await treeDigest(root)) throw Error('Pending Sites files changed. Restore the prepared files before resuming the retained deployment.');
    return {version:state.version, phase:state.phase, reused:true};
  }
  const from = JSON.parse(await text(join(root, 'package.json'))).version;
  const match = semver.exec(from);
  if (!match || !Number.isSafeInteger(Number(match[3])) || Number(match[3]) >= 65535) throw Error('Automatic releases require a stable major.minor.patch version with a patch below 65535.');
  if (state && state.version !== from) throw Error('Working version differs from the last finalized release. Resolve it before automatic versioning.');
  const version = bump === 'major' ? `${Number(match[1])+1}.0.0` : bump === 'minor' ? `${match[1]}.${Number(match[2])+1}.0` : `${match[1]}.${match[2]}.${Number(match[3])+1}`;
  if(version.split('.').some(value=>!Number.isSafeInteger(Number(value))||Number(value)>65535))throw Error('Release version exceeds the Windows installer version range.');
  if (await optional(join(root, `build/sites-publication-${version}.json`)) !== null) throw Error(`Version ${version} already has a publication receipt. Never replace a published version.`);
  const nextPath = join(root, 'release-notes/next.md');
  const candidate = notesPath ? resolve(root, notesPath) : nextPath;
  let notes = await optional(candidate);
  if (notesPath && notes === null) throw Error('The explicit release notes file does not exist.');
  if (notes !== null && !notes.trim()) throw Error('Release notes must not be empty.');
  if (notes === null) notes = `# 氢看直播 ${version}\n\n新版本已发布。\n`;
  notes = notes.replaceAll('{{version}}', version).replaceAll('\r\n','\n').replaceAll('\r','\n');
  state = {format:1, phase:'preparing', from_version:from, version, created_at:new Date().toISOString(), notes_sha256:digest(notes), edits:await sourcePlan(root, from, version, notes)};
  if (candidate === nextPath && await optional(nextPath) !== null) state.next_notes_sha256 = digest(await text(nextPath));
  await save(root, state);
  await recoverPreparation(root, state);
  return {version, phase:state.phase, reused:false};
}
export async function markPublishing(root) {
  const state = await readState(root);
  if (!state || !['prepared','publishing'].includes(state.phase)) throw Error('No prepared release is available.');
  await validatePrepared(root, state);
  const current = await treeDigest(root);
  if (state.site_sha256 && current !== state.site_sha256) throw Error('Prepared Sites files changed during publication.');
  const manifest = JSON.parse(await text(join(root, 'dist/sites/qingkan-live/latest.json')));
  if (manifest.version !== state.version) throw Error('Prepared Sites manifest version differs from the release transaction.');
  state.site_sha256 = current;
  state.phase = 'publishing';
  await save(root, state);
  return {version:state.version, phase:state.phase};
}
export async function repackFailedRelease(root, reportPath) {
  root = resolve(root);
  const state = await readState(root);
  if (!state || state.phase !== 'publishing') throw Error('Only a failed pending publication can be repackaged.');
  await validatePrepared(root, state);
  if (await optional(join(root, `build/sites-publication-${state.version}.json`)) !== null) throw Error('A publication receipt exists; never replace a possibly published version.');
  const config = JSON.parse(await text(join(root, 'release-site/publishing.json')));
  const evidence = await text(resolve(root, reportPath));
  const report = JSON.parse(evidence);
  const deployments = report.deployments ?? (report.deployment ? [report.deployment] : []);
  if (!deployments.length || deployments.some(item => item.site_id !== config.site_id || !item.deployment_id || !['failed','cancelled'].includes(item.phase))) throw Error('Repackaging requires an official report containing only terminal failed or cancelled deployments for this Site. Query pending deployments first.');
  if (deployments.some(item => item.deployment_id === report.current_deployment)) throw Error('The report identifies a failed attempt as the current successful deployment; resolve it before repackaging.');
  const lastTime = Math.max(...deployments.map(item => Date.parse(item.created_at)));
  if (!Number.isFinite(lastTime) || lastTime < Date.parse(state.created_at)) throw Error('The failure report predates this pending release.');
  if (state.site_sha256 !== await treeDigest(root)) throw Error('The retained failed publication files changed. Restore them before creating a repackaging backup.');
  // Retain the immutable previous payload digest and its terminal failure evidence.
  // This explicitly reopens the same unpublished version; ordinary retry still
  // refuses altered content and successful versions can never be reopened.
  const reopenedAt = new Date().toISOString();
  const attemptDirectory = join(root, '.release-secrets/release-attempts', `${state.version}-${reopenedAt.replaceAll(':','-')}`);
  await mkdir(attemptDirectory, {recursive:true});
  await cp(join(root,'dist/sites/qingkan-live'), join(attemptDirectory,'site'), {recursive:true, errorOnExist:true, force:false});
  await atomic(join(attemptDirectory,'failed-deployments.json'), evidence);
  const attempt = {state, failure_report_sha256:digest(evidence), deployments:deployments.map(item=>({deployment_id:item.deployment_id,phase:item.phase})), reopened_at:reopenedAt};
  await atomic(join(attemptDirectory,'release-state.json'), JSON.stringify(attempt, null, 2) + '\n');
  state.phase = 'prepared';
  state.repacked_at = reopenedAt;
  delete state.site_sha256;
  await save(root, state);
  return {version:state.version, phase:state.phase, repacked:true};
}
// Official PostgreSQL timestamps can contain a space, microseconds and +08.
// Compare nanosecond instants so a newer sub-millisecond submission cannot be
// rounded down to the pending transaction's timestamp.
function deploymentInstant(value) {
  if (typeof value !== 'string') return null;
  const match = /^(\d{4}-\d{2}-\d{2})[ T](\d{2}:\d{2}:\d{2})(?:\.(\d{1,9}))?(Z|[+-]\d{2}(?::?\d{2})?)$/.exec(value.trim());
  if (!match) return null;
  const [,day,time,fraction='',zone] = match;
  const digits = zone === 'Z' ? '' : zone.slice(1).replace(':','');
  const hour = digits ? Number(digits.slice(0,2)) : 0;
  const minute = digits.length > 2 ? Number(digits.slice(2)) : 0;
  if (hour > 23 || minute > 59) return null;
  const offset = (zone.startsWith('-') ? -1 : 1) * (hour*60+minute);
  const normalizedZone = zone === 'Z' ? 'Z' : `${zone[0]}${String(hour).padStart(2,'0')}:${String(minute).padStart(2,'0')}`;
  const millis = Date.parse(`${day}T${time}${normalizedZone}`);
  if (!Number.isFinite(millis) || new Date(millis+offset*60000).toISOString().slice(0,19) !== `${day}T${time}`) return null;
  return BigInt(millis)*1_000_000n + BigInt(fraction.padEnd(9,'0') || '0');
}
export async function retireUnsubmittedRelease(root, reportPath) {
  root = resolve(root);
  const state = await readState(root);
  if (!state || state.phase !== 'publishing') throw Error('Only a pending publishing release can be retired as unsubmitted.');
  await validatePrepared(root, state);
  if (await optional(join(root, `build/sites-publication-${state.version}.json`)) !== null) throw Error('A publication receipt exists; never retire a possibly published version as unsubmitted.');
  const config = JSON.parse(await text(join(root, 'release-site/publishing.json')));
  const evidence = await text(resolve(root, reportPath));
  const report = JSON.parse(evidence);
  if (!report || report.site_id !== config.site_id || !Object.hasOwn(report,'next_before_sequence') || report.next_before_sequence !== null) throw Error('Retirement requires the complete official deployment history for this Site, with next_before_sequence explicitly null.');
  if (!Array.isArray(report.deployments) || !report.deployments.length || typeof report.current_deployment !== 'string' || !report.current_deployment) throw Error('Retirement requires a nonempty official deployment history and its current successful deployment.');
  const pendingAt = deploymentInstant(state.created_at);
  const seen = new Set();
  for (const deployment of report.deployments) {
    const created = deploymentInstant(deployment?.created_at);
    if (!deployment || deployment.site_id !== config.site_id || typeof deployment.deployment_id !== 'string' || !deployment.deployment_id || seen.has(deployment.deployment_id) || !['succeeded','failed','cancelled'].includes(deployment.phase)) throw Error('The deployment history contains a mismatched, duplicate, unknown or nonterminal deployment; retain the pending release.');
    if (pendingAt === null || created === null || created > pendingAt) throw Error('A deployment is newer than the pending release or has an unknown date; its submission status must be resolved first.');
    seen.add(deployment.deployment_id);
  }
  const current = report.deployments.find(deployment => deployment.deployment_id === report.current_deployment);
  if (!current || current.phase !== 'succeeded') throw Error('The reported current deployment must match a successful deployment in the complete history.');
  if (!state.site_sha256 || state.site_sha256 !== await treeDigest(root)) throw Error('The retained unsubmitted publication files changed; restore the frozen payload before retiring it.');
  const retiredAt = new Date().toISOString();
  const attemptDirectory = join(root, '.release-secrets/release-attempts', `${state.version}-unsubmitted-${retiredAt.replaceAll(':','-')}-${randomUUID().slice(0,8)}`);
  await mkdir(attemptDirectory, {recursive:true});
  const backupSite = join(attemptDirectory, 'site');
  await cp(join(root,'dist/sites/qingkan-live'), backupSite, {recursive:true, errorOnExist:true, force:false});
  if (state.site_sha256 !== await treeDigest(root, backupSite) || state.site_sha256 !== await treeDigest(root)) throw Error('The frozen payload changed while its retirement backup was being created; retain the pending state and inspect the backup.');
  await atomic(join(attemptDirectory,'unsubmitted-deployments.json'), evidence);
  const reportHash = digest(evidence);
  await atomic(join(attemptDirectory,'release-state.json'), JSON.stringify({state, evidence_sha256:reportHash, current_deployment:report.current_deployment, retired_at:retiredAt}, null, 2) + '\n');
  state.phase = 'superseded';
  state.superseded_at = retiredAt;
  state.superseded_reason = 'unsubmitted';
  state.retirement_backup = relative(root,attemptDirectory).replaceAll('\\','/');
  state.retirement_evidence_sha256 = reportHash;
  await save(root, state);
  // This does not publish, alter version notes or consume next.md. The next
  // ordinary preparation creates a new version from the current working tree.
  return {version:state.version, phase:state.phase, retired:true};
}

async function optionalTreeDigest(root,directory) {
  try{return await treeDigest(root,directory);}catch(error){if(error.code==='ENOENT')return null;throw error;}
}
async function signedDigests(directory,version) {
  const {signedSiteFiles}=await import('./site_bundle.mjs');
  const result={};
  for(const path of signedSiteFiles(version))result[path]=await fileDigest(join(directory,path));
  return result;
}
function sameSigned(actual,expected) {
  if(JSON.stringify(actual)!==JSON.stringify(expected))throw Error('A website revision must preserve the installer, signature, checksum and latest.json bytes.');
}
function revisionDirectory(root,journal) {
  const base=join(root,'.release-secrets/release-attempts');
  if(journal?.format!==1||typeof journal.directory!=='string'||!Number.isSafeInteger(journal.revision)||journal.revision<2||
    !/^[a-f0-9]{64}$/.test(journal.previous_sha256)||!/^[a-f0-9]{64}$/.test(journal.next_sha256))throw Error('Unrecognized retained website revision; keep its files for inspection.');
  const directory=resolve(root,journal.directory),part=relative(base,directory);
  if(!part||part.startsWith('..')||isAbsolute(part)||/[\\/]/.test(part))throw Error('Retained website revision escaped its private attempt directory.');
  return directory;
}
async function finishWebsiteRefresh(root,state) {
  const journal=state.website_refresh,directory=revisionDirectory(root,journal);
  const frozen=join(root,'dist/sites/qingkan-live'),snapshot=join(directory,'site'),staged=join(directory,'staged-site'),displaced=join(directory,'displaced-site');
  if(journal.previous_sha256!==state.site_sha256||await treeDigest(root,snapshot)!==journal.previous_sha256)throw Error('The retained website revision snapshot changed; keep all files for inspection.');
  sameSigned(await signedDigests(snapshot,state.version),journal.signed_sha256);
  let current=await optionalTreeDigest(root,frozen);
  if(current!==journal.next_sha256){
    if(current!==null&&current!==journal.previous_sha256)throw Error('The frozen website differs from both sides of the retained revision; keep all files for inspection.');
    if(await treeDigest(root,staged)!==journal.next_sha256)throw Error('Staged website revision changed; keep its snapshot and original release state.');
    sameSigned(await signedDigests(staged,state.version),journal.signed_sha256);
    if(current===null){
      if(await treeDigest(root,displaced)!==journal.previous_sha256)throw Error('The interrupted website replacement has no intact previous directory.');
    }else{
      // The old frozen payload has already been copied and verified privately.
      // Directory renames do not expose a half-copied installer to the uploader.
      await rename(frozen,displaced);
    }
    try{await rename(staged,frozen);}
    catch(error){
      // If rename failed before installing the new directory, restore the old
      // frozen tree. The retained journal still permits an explicit retry.
      try{if(await optionalTreeDigest(root,frozen)===null)await rename(displaced,frozen);}catch{}
      throw error;
    }
    current=await treeDigest(root,frozen);
  }
  if(current!==journal.next_sha256)throw Error('Website revision installation digest differs from its verified staged tree.');
  sameSigned(await signedDigests(frozen,state.version),journal.signed_sha256);
  state.website_revision=journal.revision;
  state.website_revised_at=journal.created_at;
  state.website_previous_sha256=journal.previous_sha256;
  state.website_backup=journal.directory;
  state.site_sha256=journal.next_sha256;
  delete state.website_refresh;
  await save(root,state);
  return {version:state.version,phase:state.phase,website_revision:state.website_revision,site_sha256:state.site_sha256,backup:state.website_backup,directory:frozen};
}
export async function refreshWebsite(root) {
  root=resolve(root);
  const state=await readState(root);
  if(!state||state.phase!=='publishing')throw Error('A website-only revision requires an existing frozen publishing release. It never builds or increments the application version.');
  if(await optional(join(root,`build/sites-publication-${state.version}.json`))!==null)throw Error('A verified publication receipt already exists. Resolve that publication before revising its website.');
  if(state.website_refresh)return finishWebsiteRefresh(root,state);
  await validatePrepared(root,state);
  const originalState=await text(stateFile(root));
  if(JSON.stringify(JSON.parse(originalState))!==JSON.stringify(state))throw Error('Release state changed while starting the website revision.');
  const frozen=join(root,'dist/sites/qingkan-live');
  if(!state.site_sha256||state.site_sha256!==await treeDigest(root,frozen))throw Error('The frozen website changed; restore its retained digest before requesting a website revision.');
  const signed=await signedDigests(frozen,state.version);
  const revision=(state.website_revision??1)+1;
  if(!Number.isSafeInteger(revision)||revision<2)throw Error('Invalid website revision number.');
  const revisedAt=new Date().toISOString();
  const attempt=join(root,'.release-secrets/release-attempts',`${state.version}-website-r${revision}-${randomUUID()}`);
  await mkdir(attempt,{recursive:true,mode:0o700});
  await atomic(join(attempt,'release-state.json'),originalState);
  await cp(frozen,join(attempt,'site'),{recursive:true,errorOnExist:true,force:false});
  if(await treeDigest(root,join(attempt,'site'))!==state.site_sha256||await treeDigest(root,frozen)!==state.site_sha256)throw Error('The frozen website changed while its private revision snapshot was being saved.');
  // Signed inputs come from the immutable old snapshot, never from rebuilt CDN
  // output or a possibly changed installer. Only the website sources are new.
  const {stageReleaseSite}=await import('./site_bundle.mjs');
  const staged=join(attempt,'staged-site');
  await stageReleaseSite(root,{outputDirectory:staged,artifactDirectory:join(attempt,'site'),allowPending:true});
  sameSigned(await signedDigests(staged,state.version),signed);
  if(await treeDigest(root,frozen)!==state.site_sha256)throw Error('The frozen website changed while the new revision was being staged.');
  if(await text(stateFile(root))!==originalState)throw Error('Release state changed while the new website was being staged; the original snapshot is retained.');
  state.website_refresh={format:1,revision,created_at:revisedAt,directory:relative(root,attempt).replaceAll('\\','/'),previous_sha256:state.site_sha256,next_sha256:await treeDigest(root,staged),signed_sha256:signed};
  // Persist a recovery journal before either rename. A later refresh-site call
  // can finish after an interruption without rebuilding or changing signed data.
  await save(root,state);
  return finishWebsiteRefresh(root,state);
}
export async function completeRelease(root) {
  const state = await readState(root);
  if (!state || state.phase !== 'publishing') throw Error('No publishing release is available.');
  if (state.website_refresh) throw Error('Finish the retained website revision before completing publication.');
  const receipt = JSON.parse(await text(join(root, `build/sites-publication-${state.version}.json`)));
  const config = JSON.parse(await text(join(root, 'release-site/publishing.json')));
  if (!receipt.verified || receipt.version !== state.version || receipt.site_id !== config.site_id || receipt.url !== config.url || !receipt.deployment_id) throw Error('The verified Sites publication receipt does not match the release.');
  if (state.site_sha256 !== await treeDigest(root)) throw Error('Published Sites files changed before release completion.');
  state.phase = 'published';
  state.published_at = new Date().toISOString();
  state.deployment_id = receipt.deployment_id;
  const nextPath = join(root, 'release-notes/next.md');
  const next = await optional(nextPath);
  if (next !== null && digest(next) === state.next_notes_sha256) await unlink(nextPath);
  await save(root, state);
  return {version:state.version, phase:state.phase};
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const cliArgs=process.argv.slice(2);
    const bumpArgs=cliArgs.filter(value=>value.startsWith('--bump='));
    if(bumpArgs.length>1)throw Error('Specify one version bump.');
    const bump=bumpArgs[0]?.slice('--bump='.length)||'patch';
    const [command, root, notesPath, ...extra] = cliArgs.filter(value=>!value.startsWith('--bump='));
    if (!root || extra.length || !['prepare','publishing','complete','repack-failed','retire-unsubmitted','refresh-site'].includes(command) || (notesPath && !['prepare','repack-failed','retire-unsubmitted'].includes(command)) || (['repack-failed','retire-unsubmitted'].includes(command) && !notesPath)) throw Error('Usage: node release_version.mjs prepare ROOT [NOTES_FILE] | publishing ROOT | complete ROOT | refresh-site ROOT | repack-failed ROOT FAILED_DEPLOYMENT_REPORT | retire-unsubmitted ROOT COMPLETE_DEPLOYMENT_HISTORY');
    const result = command === 'prepare' ? await prepareRelease(root, notesPath, bump) : command === 'publishing' ? await markPublishing(root) : command === 'refresh-site' ? await refreshWebsite(root) : command === 'repack-failed' ? await repackFailedRelease(root, notesPath) : command === 'retire-unsubmitted' ? await retireUnsubmittedRelease(root,notesPath) : await completeRelease(root);
    console.log(JSON.stringify(result));
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
