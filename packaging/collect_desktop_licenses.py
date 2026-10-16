"""Preserve license texts from locked, downloaded Rust and JS dependencies.

Build dependencies may also be included to keep redistribution notices complete.
"""
from pathlib import Path
import hashlib
import json
import os
import re
import shutil
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / 'packaging' / 'licenses' / 'desktop'
if DEST.exists():
    if DEST.resolve() != (ROOT / 'packaging/licenses/desktop').resolve() or not DEST.resolve().is_relative_to(ROOT):
        raise RuntimeError('License staging target escaped the project')
    shutil.rmtree(DEST)
DEST.mkdir(parents=True, exist_ok=True)
lock_paths = ('src-tauri/Cargo.lock', 'crates/Cargo.lock')
locked_packages = set()
for lock_path in lock_paths:
    locked = tomllib.loads((ROOT / lock_path).read_text(encoding='utf-8'))
    locked_packages.update((item['name'], item['version']) for item in locked['package'] if item.get('source', '').startswith('registry+'))
# Lockfiles include other operating systems. Require every dependency in both
# actual Windows build graphs, rather than downloading unrelated platform code.
packages = set()
cargo_env = {**os.environ, 'CARGO_HOME': str(ROOT / 'packaging/tools/cargo-home')}
for manifest, selection in (('src-tauri/Cargo.toml', []), ('crates/Cargo.toml', ['-p', 'qingkan-native'])):
    tree = subprocess.run(['cargo', 'tree', '--manifest-path', str(ROOT / manifest),
        '--target', 'x86_64-pc-windows-msvc', '--locked', '--offline', '--edges', 'normal,build',
        '--prefix', 'none', '--format', '{p}', *selection], env=cargo_env, check=True,
        capture_output=True, text=True, encoding='utf-8').stdout
    for line in tree.splitlines():
        match = re.match(r'^(\S+) v(\S+)', line)
        if match and match.groups() in locked_packages:
            packages.add(match.groups())
release_version = json.loads((ROOT / 'package.json').read_text(encoding='utf-8'))['version']
registry = ROOT / 'packaging' / 'tools' / 'cargo-home' / 'registry' / 'src'
notices = [f'氢看直播 {release_version} - desktop and native dependencies', '',
           'Includes Windows runtime and build-time packages from both resolved dependency graphs.',
           'License texts: licenses/desktop/. src-tauri/Cargo.lock, crates/Cargo.lock and package-lock.json pin versions.', '']
missing = []
found = set()
for manifest in sorted(registry.glob('*/*/Cargo.toml')):
    metadata = tomllib.loads(manifest.read_text(encoding='utf-8'))['package']
    name, version = metadata['name'], metadata['version']
    if (name, version) not in packages:
        continue
    found.add((name, version))
    directory = manifest.parent
    files = [path for path in directory.iterdir() if path.is_file() and path.name.lower().startswith(('license', 'licence', 'copying', 'notice', 'authors', 'copyright'))]
    if metadata.get('license-file'):
        files.append(directory / metadata['license-file'])
    if not files:
        files = [path for path in directory.rglob('*') if path.is_file() and path.name.lower().startswith(('license', 'licence', 'copying'))][:8]
    if not files:
        extra = ROOT / 'packaging' / 'extra-licenses' / f'{name}-{version}'
        files = list(extra.glob('*')) if extra.exists() else []
    if not files:
        missing.append(f'{name} {version}')
    target = DEST / f'{name}-{version}'
    target.mkdir(exist_ok=True)
    for index, path in enumerate(dict.fromkeys(files)):
        if path.is_file():
            shutil.copyfile(path, target / f'{index}-{path.name}')
    notices.append(f'{name} {version}: {metadata.get("license", "See license text")}')
    vcs_file = directory / '.cargo_vcs_info.json'
    repository = metadata.get('repository', '')
    if repository and vcs_file.exists():
        revision = json.loads(vcs_file.read_text(encoding='utf-8')).get('git', {}).get('sha1', '')
        notices.append(f'  Unmodified source: {repository}/tree/{revision}')
for name in ('@tauri-apps/api', '@tauri-apps/plugin-updater'):
    directory = ROOT / 'node_modules' / name
    metadata = json.loads((directory / 'package.json').read_text(encoding='utf-8'))
    target = DEST / f'js-{name.replace("/", "-")}-{metadata["version"]}'
    target.mkdir(exist_ok=True)
    files = [path for path in directory.iterdir() if path.is_file() and path.name.lower().startswith(('license', 'notice', 'copying'))]
    if not files:
        missing.append(name)
    for path in files:
        shutil.copyfile(path, target / path.name)
    notices.append(f'{name} {metadata["version"]}: {metadata.get("license", "See license text")}')
font_source = ROOT / 'src' / 'fonts'
font_manifest = json.loads((font_source / 'font-manifest.json').read_text(encoding='utf-8'))
for asset in font_manifest['assets']:
    asset_path = ROOT / asset['file']
    if hashlib.sha256(asset_path.read_bytes()).hexdigest() != asset['sha256']:
        raise RuntimeError(f'Font or license asset changed: {asset["file"]}')
font_target = ROOT / 'packaging' / 'licenses' / 'fonts'
font_target.mkdir(parents=True, exist_ok=True)
shutil.copytree(font_source / 'licenses', font_target / 'licenses', dirs_exist_ok=True)
shutil.copyfile(font_source / 'font-manifest.json', font_target / 'font-manifest.json')
shutil.copytree(ROOT / 'packaging/extra-licenses/nsis-3.11', DEST / 'nsis-3.11', dirs_exist_ok=True)
notices.append('NSIS 3.11 installer stubs and compiler notices: licenses/desktop/nsis-3.11/COPYING')
activity_license = ROOT / 'docs/research/licenses/douyinlive-proto-MIT.txt'
protocol_target = DEST / 'douyinlive-proto'
protocol_target.mkdir(exist_ok=True)
shutil.copyfile(activity_license, protocol_target / 'LICENSE-MIT.txt')
notices.extend(['douyinlive-proto activity protobuf definitions: MIT (copyright retained in licenses/desktop/douyinlive-proto/LICENSE-MIT.txt)',
                '  Source: https://github.com/jwwsjlm/douyinlive-proto/tree/0d661be57c8f80947054b2ef0cd1ac2fdce3cc4b',
                '  Only referenced activity field definitions are used; the complete upstream project is not bundled.'])
notices.extend(['', 'Bundled unmodified fonts (SIL Open Font License 1.1):',
                'Douyin Sans Bold and ChillRoundM. Copyright, licenses and source notices:',
                'licenses/fonts/licenses/. Original file hashes: licenses/fonts/font-manifest.json.',
                'The project MIT license does not apply to these fonts.'])
(ROOT / 'THIRD_PARTY_DESKTOP_NOTICES.txt').write_text('\n'.join(notices) + '\n', encoding='utf-8')
missing.extend(f'{name} {version} (not downloaded)' for name, version in sorted(packages - found))
if missing:
    raise RuntimeError('License text review required: ' + ', '.join(missing))
print(f'Collected desktop dependency notices: {len(notices) - 5} entries.')
