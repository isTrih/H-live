# -*- mode: python ; coding: utf-8 -*-
from pathlib import Path
from PyInstaller.utils.hooks import collect_all, collect_data_files

root = Path(SPECPATH).resolve().parent
collector = root / 'python' / 'collector'
if not (collector / 'qingkan_native.pyd').is_file():
    raise RuntimeError('Build the required native module with scripts/build_native.ps1 first.')
mini_data, mini_binaries, mini_imports = collect_all('py_mini_racer')
datas = mini_data + collect_data_files('certifi') + [
    (str(collector / 'sign.js'), '.'),
    (str(root / 'LICENSE'), '.'),
    (str(root / 'THIRD_PARTY_NOTICES.txt'), '.'),
    (str(root / 'packaging' / 'licenses'), 'licenses'),
]
a = Analysis(
    [str(collector / 'backend_service.py')], pathex=[str(collector)],
    binaries=mini_binaries, datas=datas,
    hiddenimports=mini_imports + ['qingkan_native', 'socks', 'urllib3.contrib.socks', 'python_socks.sync'],
    excludes=['tkinter', '_tkinter', 'pytest', 'unittest', 'pip', 'setuptools', 'betterproto', 'protobuf'],
    noarchive=False,
)
pyz = PYZ(a.pure)
# Console subsystem supplies private stdin/stdout handles. The Rust owner spawns
# this executable with CREATE_NO_WINDOW; it never exposes a console to the user.
exe = EXE(pyz, a.scripts, [], exclude_binaries=True, name='DouyinMonitorEngine',
          debug=False, strip=False, upx=False, console=True,
          version=str(root / 'packaging' / 'version_info.txt'),
          icon=str(root / 'src-tauri' / 'icons' / 'icon.ico'), uac_admin=False)
coll = COLLECT(exe, a.binaries, a.datas, strip=False, upx=False, name='DouyinMonitorEngine')
