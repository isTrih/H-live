"""Load the distribution's own V8 DLL/ICU/sign script and TLS certificate store."""
from pathlib import Path
import hashlib
import json
import ssl
import sys

ROOT = Path(__file__).resolve().parents[1]
payload = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / 'dist/DouyinMonitorEngine/_internal'
sys._MEIPASS = str(payload)
sys.path.insert(0, str(payload))
import py_mini_racer
import qingkan_native

assert Path(py_mini_racer.__file__).is_relative_to(payload), 'V8 test imported the development package'
assert Path(qingkan_native.__file__).is_relative_to(payload), 'Native test imported the development extension'
expected_version = json.loads((ROOT / 'package.json').read_text(encoding='utf-8'))['version']
assert qingkan_native.__version__ == expected_version, 'Native module version does not match the release'
script = payload / 'sign.js'
assert hashlib.sha256(script.read_bytes()).digest() == hashlib.sha256((ROOT / 'python/collector/sign.js').read_bytes()).digest()
with py_mini_racer.MiniRacer() as context:
    assert context.eval('1 + 2') == 3
    context.eval(script.read_text(encoding='utf-8'), timeout_sec=5)
    signature = context.call('get_sign', '0123456789abcdef0123456789abcdef', timeout_sec=5)
    assert isinstance(signature, str) and signature
certificates = payload / 'certifi/cacert.pem'
store = ssl.create_default_context(cafile=str(certificates)).cert_store_stats()
assert store['x509_ca'] > 0
report = {'ok': True, 'payload': str(payload), 'packaged_v8_and_icu': True,
          'native_version': qingkan_native.__version__, 'packaged_native_extension': True,
          'packaged_signature_script': True, 'signature_length': len(signature),
          'tls_root_certificates': store['x509_ca'], 'requires_tk': False}
(ROOT / 'build/runtime-assets-check.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
print(json.dumps(report, ensure_ascii=False, indent=2))
