"""Collect license files from the exact Python environment being frozen."""
from importlib.metadata import distribution
from pathlib import Path
import json
import re
import shutil
import sys

ROOT = Path(__file__).resolve().parent.parent
DEST = ROOT / 'packaging' / 'licenses'
if DEST.exists():
    if DEST.resolve() != (ROOT / 'packaging/licenses').resolve() or not DEST.resolve().is_relative_to(ROOT):
        raise RuntimeError('License staging target escaped the project')
    shutil.rmtree(DEST)
DEST.mkdir(parents=True, exist_ok=True)
packages = [
    line.split('==', 1)[0]
    for line in (ROOT / 'requirements.txt').read_text(encoding='utf-8').splitlines()
    if line.strip() and not line.startswith('#')
]
packages.append('pyinstaller')  # redistributable bootloader with its special exception
notices = [
    f'氢看直播 {json.loads((ROOT / "package.json").read_text(encoding="utf-8"))["version"]} - third-party notices',
    '',
    'Modified community distribution based on https://github.com/eplayn/DouyinLiveMonitor',
    'Upstream Copyright (c) 2026 eplayn. License: MIT (see LICENSE).',
    'Not an official Douyin/ByteDance product.',
    '',
    'Complete license texts accompany this application in licenses/ and engine/_internal/licenses/.',
    'The package includes CPython, OpenSSL and Microsoft runtime components.',
    'See Python-LICENSE.txt for their notices and redistribution terms.',
    'MiniRacer includes V8 14.4.258.23 and ICU; see the V8/ICU license files.',
    'The 0.1.x installer and signed update packages use Tauri / NSIS (https://nsis.sourceforge.io/).',
    'Desktop Rust and JavaScript notices: THIRD_PARTY_DESKTOP_NOTICES.txt.',
    'Microsoft WebView2 Evergreen is used under Microsoft runtime license terms.',
    'The installer includes the Microsoft-signed official WebView2 bootstrapper.',
    'https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution',
    '',
]
for name in packages:
    dist = distribution(name)
    license_id = dist.metadata.get('License-Expression') or dist.metadata.get('License', '')
    license_id = license_id.splitlines()[0] if license_id else 'See bundled license text'
    notices.append(f'{dist.metadata["Name"]} {dist.version} - {license_id}')
    copied = 0
    for item in dist.files or []:
        if '.dist-info/' not in str(item).replace('\\', '/'):
            continue
        if not re.match(r'^(LICENSE|COPYING|NOTICE|AUTHORS)', item.name, re.IGNORECASE):
            continue
        output = DEST / f'{name}-{item.name}'
        shutil.copyfile(dist.locate_file(item), output)
        copied += 1
    if not copied:
        raise RuntimeError(f'Missing license files for {name}')
python_license = Path(sys.base_prefix) / 'LICENSE.txt'
shutil.copyfile(python_license, DEST / 'Python-LICENSE.txt')
tk_license = Path(sys.base_prefix) / 'tcl' / 'tk8.6' / 'license.terms'
if tk_license.exists():
    shutil.copyfile(tk_license, DEST / 'Tk-license.terms')
(ROOT / 'THIRD_PARTY_NOTICES.txt').write_text('\n'.join(notices) + '\n', encoding='utf-8')
print(f'Collected licenses for {len(packages)} Python distributions plus interpreter/runtime notices.')
