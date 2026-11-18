// Complete desktop changelog data. Markdown stays data; renderers must escape it.
import {readFile, readdir, lstat, mkdir, writeFile, rename, unlink} from 'node:fs/promises';
import {randomUUID} from 'node:crypto';
import {resolve, join, dirname, relative, isAbsolute} from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';
import {TextDecoder} from 'node:util';

const stableVersion = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const noteFilename = /^((?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*))\.md$/;
const MAX_FILE_BYTES = 1024 * 1024;
const MAX_TOTAL_BYTES = 16 * 1024 * 1024;
const MAX_ENTRIES = 1000;
const DATE_METADATA = 'release-notes/release-dates.json';
const DATE_TIME_ZONE = 'Asia/Shanghai';
const dateFormatter = new Intl.DateTimeFormat('en-CA', {timeZone:DATE_TIME_ZONE, year:'numeric', month:'2-digit', day:'2-digit'});
const defaultRoot = fileURLToPath(new URL('../', import.meta.url));

function parts(version) {
  if (typeof version !== 'string' || version.length > 64 || !stableVersion.test(version)) {
    throw Error('Release changelog requires a stable major.minor.patch version.');
  }
  return version.split('.').map(BigInt);
}
function compare(a, b) {
  const left = parts(a), right = parts(b);
  for (let index = 0; index < 3; index++) {
    if (left[index] !== right[index]) return left[index] > right[index] ? 1 : -1;
  }
  return 0;
}
async function source(path, label, maximum = MAX_FILE_BYTES) {
  const stat = await lstat(path);
  if (!stat.isFile() || stat.isSymbolicLink()) throw Error(`${label} must be a regular file.`);
  if (stat.size > maximum) throw Error(`${label} exceeds the changelog byte limit.`);
  const bytes = await readFile(path);
  if (bytes.byteLength > maximum) throw Error(`${label} exceeds the changelog byte limit.`);
  let text;
  try { text = new TextDecoder('utf-8', {fatal:true, ignoreBOM:true}).decode(bytes); }
  catch { throw Error(`${label} is not valid UTF-8.`); }
  return {text:text.replace(/^\uFEFF/, ''), bytes:bytes.byteLength};
}
function title(body, version) {
  const first = body.split(/\r?\n/).find(line => line.trim());
  const heading = first?.match(/^#\s+(.+?)(?:\s+#+)?\s*$/);
  return heading ? heading[1] : `v${version}`;
}

function calendarDate(value) {
  if (typeof value !== 'string' || !/^\d{4}-\d{2}-\d{2}$/.test(value)) throw Error('Release date must be YYYY-MM-DD.');
  const time = Date.parse(`${value}T00:00:00Z`);
  if (!Number.isFinite(time) || new Date(time).toISOString().slice(0, 10) !== value) throw Error('Release date is not a real calendar date.');
  return value;
}
function manifestDate(value) {
  if (typeof value !== 'string' || !/^\d{4}-\d{2}-\d{2}T(?:[01]\d|2[0-3]):[0-5]\d:[0-5]\d(?:\.\d{1,9})?(?:Z|[+-](?:[01]\d|2[0-3]):[0-5]\d)$/.test(value)) {
    throw Error('Release pub_date must be an ISO timestamp with an explicit timezone.');
  }
  calendarDate(value.slice(0, 10));
  const time = Date.parse(value);
  if (!Number.isFinite(time)) throw Error('Release pub_date is invalid.');
  const values = Object.fromEntries(dateFormatter.formatToParts(time).map(part => [part.type, part.value]));
  return calendarDate(`${values.year}-${values.month}-${values.day}`);
}
function object(value) { return value && typeof value === 'object' && !Array.isArray(value); }
function dateRecord(record) {
  if (!object(record) || !object(record.source)) throw Error('Release date metadata requires a source for every record.');
  if (record.date !== null) calendarDate(record.date);
  for (const [key, value] of Object.entries(record.source)) {
    if (typeof value !== 'string' || value.length > 2048 || value.includes('\0')) throw Error(`Invalid release date source ${key}.`);
  }
  if (!record.source.kind || (record.date === null && record.source.kind !== 'unrecorded')) throw Error('Unknown release dates must be explicitly unrecorded.');
  if (record.date !== null && record.source.kind === 'unrecorded') throw Error('A recorded release date needs dated evidence.');
  if (record.source.kind === 'manifest' && record.date !== manifestDate(record.source.value)) throw Error('Release date differs from its manifest evidence.');
  return {date:record.date, source:{...record.source}};
}
async function optionalJson(root, path) {
  try { return JSON.parse((await source(join(root, path), path)).text); }
  catch (error) { if (error.code === 'ENOENT') return null; throw error; }
}
async function dateMetadata(root) {
  const value = await optionalJson(root, DATE_METADATA);
  if (value === null) return {format:1, time_zone:DATE_TIME_ZONE, versions:{}};
  if (!object(value) || value.format !== 1 || value.time_zone !== DATE_TIME_ZONE || !object(value.versions) || Object.keys(value.versions).length > MAX_ENTRIES) {
    throw Error('Unsupported release date metadata.');
  }
  const versions = {};
  for (const [version, record] of Object.entries(value.versions)) { parts(version); versions[version] = dateRecord(record); }
  return {format:1, time_zone:DATE_TIME_ZONE, versions};
}
function fromManifest(manifest, path) {
  if (!object(manifest)) throw Error('Release manifest must be an object.');
  parts(manifest.version);
  return {version:manifest.version, date:manifestDate(manifest.pub_date), source:{kind:'manifest', path, field:'pub_date', value:manifest.pub_date}};
}
async function dates(root) {
  const metadata = await dateMetadata(root);
  // The newly packaged manifest exists before site rendering; the installed feed
  // is the fallback for website-only revisions. Never use file mtime or today.
  for (const path of ['dist/cdn/qingkan-live/latest.json', 'release-site/latest.json']) {
    const manifest = await optionalJson(root, path);
    if (manifest === null) continue;
    if (manifest.pub_date == null) continue; // Older manifests may not have recorded a date.
    const record = fromManifest(manifest, path);
    if (metadata.versions[record.version]?.date == null) metadata.versions[record.version] = {date:record.date, source:record.source};
  }
  return metadata.versions;
}
async function writeAtomic(path, data) {
  await mkdir(dirname(path), {recursive:true});
  const temporary = `${path}.${randomUUID()}.tmp`;
  try {
    await writeFile(temporary, serializeReleaseNotes(data), {encoding:'utf8', flag:'wx'});
    await rename(temporary, path);
  } finally {
    await unlink(temporary).catch(error => { if (error.code !== 'ENOENT') throw error; });
  }
}

/** Save the first declared date before freezing a newly packaged release. */
export async function recordReleaseDate(root, manifest, path = 'dist/cdn/qingkan-live/latest.json') {
  root = resolve(root);
  const record = fromManifest(manifest, path), metadata = await dateMetadata(root);
  const validated = dateRecord(record);
  if (metadata.versions[record.version]?.date != null) return metadata.versions[record.version];
  if (Object.keys(metadata.versions).length >= MAX_ENTRIES && !(record.version in metadata.versions)) throw Error('Too many release dates.');
  metadata.versions[record.version] = validated;
  await writeAtomic(join(root, DATE_METADATA), metadata);
  return metadata.versions[record.version];
}

/** Read every published desktop release through package.json's version. Never writes files. */
export async function buildReleaseNotes(root = defaultRoot) {
  root = resolve(root);
  const packageSource = await source(join(root, 'package.json'), 'package.json');
  const current = JSON.parse(packageSource.text).version;
  parts(current);
  const directory = join(root, 'release-notes');
  const names = await readdir(directory);
  const releases = names.map(name => ({name, match:name.match(noteFilename)}))
    .filter(entry => entry.match && compare(entry.match[1], current) <= 0)
    .map(entry => ({name:entry.name, version:entry.match[1]}))
    .sort((a, b) => compare(b.version, a.version));
  if (releases.length > MAX_ENTRIES) throw Error('The changelog contains too many releases.');
  if (!releases.some(entry => entry.version === current)) throw Error(`Release notes for current version ${current} are missing.`);
  const entries = [];
  const recordedDates = await dates(root);
  let total = 0;
  for (const release of releases) {
    const content = await source(join(directory, release.name), release.name);
    total += content.bytes;
    if (total > MAX_TOTAL_BYTES) throw Error('The complete changelog exceeds the total byte limit.');
    if (!content.text.trim() || content.text.includes('\0')) throw Error(`${release.name} contains empty or invalid release notes.`);
    const record = recordedDates[release.version] || {date:null, source:{kind:'unrecorded', note:'No release date has been recorded.'}};
    entries.push({version:release.version, title:title(content.text, release.version), body:content.text,
      date:record.date, date_status:record.date === null ? 'unrecorded' : 'recorded', date_source:record.source});
  }
  return {format:1, current_version:current, entries};
}

/** Public changelog projection; local date metadata retains its complete evidence. */
export function serializeReleaseNotes(data) {
  const projected = Array.isArray(data?.entries) ? {...data, entries:data.entries.map(({date_source, ...entry}) => entry)} : data;
  return JSON.stringify(projected, null, 2).replace(/[<>&\u2028\u2029]/g, character => ({
    '<':'\\u003c', '>':'\\u003e', '&':'\\u0026', '\u2028':'\\u2028', '\u2029':'\\u2029',
  })[character]) + '\n';
}

async function cli(root) {
  const data = await buildReleaseNotes(root);
  const target = join(root, 'release-site', 'changelog.json');
  await writeAtomic(target, data);
  process.stdout.write(`${target}\n`);
}

async function recordCli(root, path) {
  if (!path) throw Error('Usage: release_notes_data.mjs --record-manifest ROOT MANIFEST');
  root = resolve(root);
  const target = resolve(root, path), local = relative(root, target);
  if (!local || local.startsWith('..') || isAbsolute(local)) throw Error('Release date manifest must be inside the project.');
  const manifest = JSON.parse((await source(target, 'Release manifest')).text);
  const record = await recordReleaseDate(root, manifest, local.replaceAll('\\', '/'));
  process.stdout.write(`${manifest.version} ${record.date}\n`);
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  (process.argv[2] === '--record-manifest' ? recordCli(process.argv[3] || defaultRoot, process.argv[4]) : cli(resolve(process.argv[2] || defaultRoot))).catch(error => {
    process.stderr.write(`${error?.message || error}\n`);
    process.exitCode = 1;
  });
}
