import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp, mkdir, writeFile, readFile, rm, readdir, access} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {buildReleaseNotes, serializeReleaseNotes, recordReleaseDate} from './release_notes_data.mjs';

const run = promisify(execFile);
const repository = fileURLToPath(new URL('../', import.meta.url));
async function fixture(t, version = '0.4.5', files = {}) {
  const root = await mkdtemp(join(tmpdir(), 'qingkan-notes-'));
  t.after(() => rm(root, {recursive:true, force:true}));
  await mkdir(join(root, 'release-notes'));
  await writeFile(join(root, 'package.json'), JSON.stringify({version}));
  for (const [name, body] of Object.entries(files)) await writeFile(join(root, 'release-notes', name), body);
  return root;
}

test('all desktop notes through the current version are sorted numerically and retain their complete original body', async t => {
  const full = '\uFEFF# 氢看直播 0.1.0\r\n\r\n## 第一部分\r\n\r\n' + Array.from({length:90}, (_, index) => `- 完整历史内容 ${index}。\r\n`).join('') + '\r\n## 最后一部分\r\n\r\n保留末段，不截断。\r\n';
  const root = await fixture(t, '0.4.10', {
    '0.4.9.md':'# 氢看直播 0.4.9\n\n- 旧补丁。\n',
    '0.1.0.md':full,
    '0.3.10.md':'# 氢看直播 0.3.10\n\n- 旧功能。\n',
    '0.4.10.md':'# 氢看直播 0.4.10\n\n- 当前完整内容。\n',
    '0.4.11.md':'尚未发布的未来版本',
    'next.md':'未发布草稿',
    'server-0.1.0.md':'服务器独立日志',
    '0.04.1.md':'不是合法版本',
    '0.4.10-copy.md':'不是一个版本文件',
  });
  const result = await buildReleaseNotes(root);
  assert.equal(result.format, 1);assert.equal(result.current_version, '0.4.10');
  assert.deepEqual(result.entries.map(entry => entry.version), ['0.4.10', '0.4.9', '0.3.10', '0.1.0']);
  assert.equal(result.entries.at(-1).body, full.slice(1));
  assert.equal(result.entries.at(-1).title, '氢看直播 0.1.0');
  assert.match(result.entries.at(-1).body, /完整历史内容 89/);
  assert.match(result.entries.at(-1).body, /保留末段，不截断。\r\n$/);
  assert.ok(result.entries.every(entry => entry.date === null && entry.date_status === 'unrecorded' && entry.date_source.kind === 'unrecorded'), 'dates must not be invented from filesystem timestamps');
  await assert.rejects(access(join(root, 'release-site')), {code:'ENOENT'}, 'the imported build function must not mutate the website');
});

test('untrusted Markdown remains plain data and JSON serialization cannot close an inline script-data block', async t => {
  const body = '# <img src=x onerror="alert(1)">\n\n- </script><script>alert(1)</script> & 引号 "\n\n文本\u2028换行\u2029结束\n';
  const root = await fixture(t, '0.4.5', {'0.4.5.md':body});
  const result = await buildReleaseNotes(root), encoded = serializeReleaseNotes(result);
  assert.equal(result.entries[0].body, body);
  assert.doesNotMatch(encoded, /<|>|&|\u2028|\u2029/);
  assert.match(encoded, /\\u003c\/script\\u003e/);
  const decoded = JSON.parse(encoded);
  assert.equal(decoded.entries[0].body, body, 'escaping must preserve the entire original Markdown after JSON parsing');
  assert.equal(decoded.entries[0].date, null);
  assert.equal(decoded.entries[0].date_status, 'unrecorded');
  assert.ok(!('date_source' in decoded.entries[0]), 'private evidence must not be serialized into public data');
});

test('missing current notes, invalid UTF-8 and unreasonable files fail without publishing a partial changelog', async t => {
  const root = await fixture(t, '0.4.5', {'0.1.0.md':'Old complete notes'});
  await assert.rejects(buildReleaseNotes(root), /current version 0\.4\.5.*missing/i);
  await writeFile(join(root, 'release-notes', '0.4.5.md'), Buffer.from([0xc3, 0x28]));
  await assert.rejects(buildReleaseNotes(root), /UTF-8/);
  await writeFile(join(root, 'release-notes', '0.4.5.md'), 'x'.repeat(1024 * 1024 + 1));
  await assert.rejects(buildReleaseNotes(root), /byte limit/);
  await writeFile(join(root, 'release-notes', '0.4.5.md'), '\uFEFF\n\t');
  await assert.rejects(buildReleaseNotes(root), /empty or invalid/);
});

test('CLI writes complete UTF-8 JSON only inside the supplied fixture and preserves an existing output on validation failure', async t => {
  const root = await fixture(t, '0.4.5', {'0.1.0.md':'# 首版\n\n- 全部内容。\n', '0.4.5.md':'# 最新版\n\n- 最新内容。\n'});
  const script = fileURLToPath(new URL('./release_notes_data.mjs', import.meta.url));
  await run(process.execPath, [script, root]);
  const path = join(root, 'release-site', 'changelog.json');
  const bytes = await readFile(path), expected = await buildReleaseNotes(root);
  const actual = JSON.parse(bytes.toString('utf8'));
  assert.equal(actual.current_version, expected.current_version);
  assert.deepEqual(actual.entries.map(entry => [entry.version,entry.title,entry.body,entry.date,entry.date_status]), expected.entries.map(entry => [entry.version,entry.title,entry.body,entry.date,entry.date_status]));
  assert.ok(actual.entries.every(entry => !('date_source' in entry)));
  assert.notEqual(bytes[0], 0xef);
  await writeFile(join(root, 'release-notes', '0.4.5.md'), '');
  await assert.rejects(run(process.execPath, [script, root]));
  assert.deepEqual(await readFile(path), bytes);
  assert.deepEqual(await readdir(join(root, 'release-site')), ['changelog.json']);
});

test('the real repository includes every full desktop release from 0.1.0 through the current package version', async () => {
  const result = await buildReleaseNotes(repository);
  const packageVersion = JSON.parse(await readFile(join(repository, 'package.json'), 'utf8')).version;
  assert.equal(result.current_version, packageVersion);
  assert.equal(result.entries[0].version, packageVersion);
  assert.equal(result.entries.at(-1).version, '0.1.0');
  assert.ok(result.entries.length >= 23, '0.1.0 through 0.4.5 must include all 23 desktop notes');
  assert.ok(result.entries.some(entry => entry.version === '0.4.5'));
  const through = version => {
    const a=version.split('.').map(Number), b=packageVersion.split('.').map(Number);
    for(let index=0;index<3;index++) if(a[index]!==b[index])return a[index]<b[index];
    return true;
  };
  const expected = (await readdir(join(repository, 'release-notes'))).filter(name => /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)\.md$/.test(name) && through(name.slice(0,-3)));
  assert.equal(result.entries.length, expected.length);
  for (const entry of result.entries) assert.equal(entry.body, (await readFile(join(repository, 'release-notes', `${entry.version}.md`), 'utf8')).replace(/^\uFEFF/, ''));
  const version = value => result.entries.find(entry => entry.version === value);
  assert.deepEqual(result.entries.filter(entry => entry.date === null).map(entry => entry.version), ['0.2.1', '0.2.0', '0.1.1', '0.1.0']);
  assert.equal(version('0.3.2').date, '2026-10-20', 'the archived manifest takes precedence over the conflicting CHANGELOG heading');
  assert.equal(version('0.4.0').date, '2026-10-24', 'release days use China time across UTC midnight');
  assert.equal(version('0.4.5').date, '2026-10-26');
  assert.equal(version('0.4.2').date_source.kind, 'manifest');
  assert.match(version('0.4.2').date_source.note, /unsubmitted/);
  assert.ok(result.entries.every(entry => entry.date_source.kind && (entry.date === null || /^\d{4}-\d{2}-\d{2}$/.test(entry.date))));
});

test('public changelog retains all release bodies and dates while local evidence paths and deployment IDs stay private', async () => {
  const local = await buildReleaseNotes(repository);
  assert.ok(local.entries.length >= 23);
  const localEvidence = JSON.stringify(local.entries.map(entry => entry.date_source));
  assert.match(localEvidence, /\.release-secrets/);
  assert.match(localEvidence, /dep_[a-f0-9]+/);
  const encoded = serializeReleaseNotes(local), publicData = JSON.parse(encoded);
  assert.doesNotMatch(encoded, /date_source|\.release-secrets|release-attempts|deployment_id|dep_[a-f0-9]+/);
  assert.equal(publicData.entries.length, local.entries.length);
  for (let index = 0; index < local.entries.length; index++) {
    const entry = local.entries[index], published = publicData.entries[index];
    assert.deepEqual(published, {version:entry.version,title:entry.title,body:entry.body,date:entry.date,date_status:entry.date_status});
    assert.ok(entry.date_source, 'serialization must not mutate the local source records');
  }
  const metadata = JSON.parse(await readFile(join(repository, 'release-notes/release-dates.json'), 'utf8'));
  assert.deepEqual(JSON.parse(serializeReleaseNotes(metadata)), metadata, 'atomic local metadata serialization must preserve the complete evidence');
});

test('matching manifest pub_date supplies China calendar dates and existing release metadata wins over a repack', async t => {
  const root = await fixture(t, '0.4.6', {'0.4.5.md':'# Earlier\n', '0.4.6.md':'# New\n'});
  await mkdir(join(root, 'dist/cdn/qingkan-live'), {recursive:true});
  await mkdir(join(root, 'release-site'));
  await writeFile(join(root, 'dist/cdn/qingkan-live/latest.json'), JSON.stringify({version:'0.4.6', pub_date:'2026-10-26T16:01:00Z'}));
  await writeFile(join(root, 'release-site/latest.json'), JSON.stringify({version:'0.4.5', pub_date:'2026-10-24T16:30:00Z'}));
  let result = await buildReleaseNotes(root);
  assert.deepEqual(result.entries.map(entry => [entry.version, entry.date]), [['0.4.6','2026-10-27'], ['0.4.5','2026-10-25']]);
  assert.equal(result.entries[0].date_source.path, 'dist/cdn/qingkan-live/latest.json');
  await assert.rejects(access(join(root, 'release-notes/release-dates.json')), {code:'ENOENT'}, 'reading fallback manifests must never write metadata');
  await writeFile(join(root, 'release-notes/release-dates.json'), JSON.stringify({format:1,time_zone:'Asia/Shanghai',versions:{'0.4.6':{date:'2026-10-26',source:{kind:'release-notes',path:'release-notes/release-dates.json',value:'2026-10-26'}}}}));
  result = await buildReleaseNotes(root);
  assert.equal(result.entries[0].date, '2026-10-26');
  assert.equal(result.entries[0].date_source.kind, 'release-notes');
  assert.equal(result.entries[1].date, '2026-10-25');
});

test('packaging records a version once, keeps original dates on repack and retains historical dates after the next release', async t => {
  const root = await fixture(t, '0.4.6', {'0.4.5.md':'# Earlier\n', '0.4.6.md':'# New\n'});
  const first = await recordReleaseDate(root, {version:'0.4.5',pub_date:'2026-10-25T16:00:00Z'});
  assert.equal(first.date, '2026-10-26');
  const original = await readFile(join(root, 'release-notes/release-dates.json'));
  const repack = await recordReleaseDate(root, {version:'0.4.5',pub_date:'2026-10-30T00:00:00Z'});
  assert.deepEqual(repack, first);
  assert.deepEqual(await readFile(join(root, 'release-notes/release-dates.json')), original);
  await recordReleaseDate(root, {version:'0.4.6',pub_date:'2026-10-26T16:00:00Z'});
  const result = await buildReleaseNotes(root);
  assert.deepEqual(result.entries.map(entry => entry.date), ['2026-10-27','2026-10-26']);
  assert.equal(result.entries[1].date_source.value, '2026-10-25T16:00:00Z');
  await assert.rejects(recordReleaseDate(root, {version:'0.4.7',pub_date:'2026-02-30T12:00:00Z'}), /calendar date/);
  await assert.rejects(recordReleaseDate(root, {version:'0.4.7',pub_date:'2026-10-26T12:00:00'}), /explicit timezone/);
  assert.equal(Object.keys(JSON.parse(await readFile(join(root, 'release-notes/release-dates.json'), 'utf8')).versions).length, 2);
});

test('date recorder CLI only edits explicit metadata and rejects invalid dates without damaging existing data', async t => {
  const root = await fixture(t, '0.4.5', {'0.4.5.md':'# Latest\n'});
  const script = fileURLToPath(new URL('./release_notes_data.mjs', import.meta.url));
  await writeFile(join(root, 'manifest.json'), JSON.stringify({version:'0.4.5',pub_date:'2026-10-26T06:39:31Z'}));
  await run(process.execPath, [script, '--record-manifest', root, 'manifest.json']);
  const path = join(root, 'release-notes/release-dates.json'), original = await readFile(path);
  assert.equal(JSON.parse(original).versions['0.4.5'].date, '2026-10-26');
  await assert.rejects(access(join(root, 'release-site')), {code:'ENOENT'});
  await writeFile(join(root, 'manifest.json'), JSON.stringify({version:'0.4.6',pub_date:'2026-13-01T12:00:00Z'}));
  await assert.rejects(run(process.execPath, [script, '--record-manifest', root, 'manifest.json']));
  assert.deepEqual(await readFile(path), original);
  await assert.rejects(run(process.execPath, [script, '--record-manifest', root, '../outside.json']));
});

test('invalid explicit date metadata fails clearly instead of inventing a replacement date', async t => {
  const root = await fixture(t, '0.4.5', {'0.4.5.md':'# Latest\n'});
  const path = join(root, 'release-notes/release-dates.json');
  await writeFile(path, JSON.stringify({format:1,time_zone:'Asia/Shanghai',versions:{'0.4.5':{date:'2026-02-29',source:{kind:'release-notes'}}}}));
  await assert.rejects(buildReleaseNotes(root), /calendar date/);
  await writeFile(path, JSON.stringify({format:1,time_zone:'UTC',versions:{}}));
  await assert.rejects(buildReleaseNotes(root), /Unsupported release date metadata/);
  await writeFile(path, JSON.stringify({format:1,time_zone:'Asia/Shanghai',versions:{'0.4.5':{date:null,source:{kind:'manifest'}}}}));
  await assert.rejects(buildReleaseNotes(root), /explicitly unrecorded/);
});
