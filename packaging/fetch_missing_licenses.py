"""Fetch missing workspace-level licenses at the exact published crate commits."""
from pathlib import Path
import json
import tomllib
import urllib.request
import urllib.error

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / 'packaging' / 'extra-licenses'
DEST.mkdir(exist_ok=True)
NAMES = {'alloc-stdlib', 'defmt-parser', 'selectors', 'webview2-com', 'webview2-com-macros', 'webview2-com-sys'}
for manifest in (ROOT / 'packaging/tools/cargo-home/registry/src').glob('*/*/Cargo.toml'):
    meta = tomllib.loads(manifest.read_text(encoding='utf-8'))['package']
    if meta['name'] not in NAMES:
        continue
    vcs = json.loads((manifest.parent / '.cargo_vcs_info.json').read_text())
    base = meta['repository'].replace('https://github.com/', 'https://raw.githubusercontent.com/')
    base += '/' + vcs['git']['sha1'] + '/'
    if meta['name'] == 'selectors':
        url = 'https://www.mozilla.org/MPL/2.0/index.txt'
        destination = DEST / (meta['name'] + '-' + meta['version'])
        destination.mkdir(exist_ok=True)
        (destination / 'LICENSE-MPL-2.0.txt').write_bytes(urllib.request.urlopen(url, timeout=20).read())
        (destination / 'SOURCE.txt').write_text(url + '\n' + meta['repository'] + '/tree/' + vcs['git']['sha1'] + '/selectors\n', encoding='utf-8')
        print('selectors', 'MPL-2.0')
        continue
    for name in ('LICENSE', 'LICENSE.txt', 'LICENSE-MIT', 'LICENSE-APACHE', 'LICENSE-MPL-2.0'):
        try:
            data = urllib.request.urlopen(base + name, timeout=20).read()
        except urllib.error.HTTPError as error:
            if error.code == 404:
                continue
            raise
        destination = DEST / (meta['name'] + '-' + meta['version'])
        destination.mkdir(exist_ok=True)
        (destination / name).write_bytes(data)
        (destination / 'SOURCE.txt').write_text(base + name + '\n', encoding='utf-8')
        print(meta['name'], name, len(data))
        break
    else:
        raise RuntimeError(f'No license found: {meta["name"]}')
