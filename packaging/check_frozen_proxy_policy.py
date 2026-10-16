"""Offline policy checks against proxy_pool bytecode embedded in the engine."""
from pathlib import Path
import hashlib
import importlib.util
import json
import sys
import types
import unittest
from unittest.mock import Mock

from PyInstaller.archive.readers import CArchiveReader

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python" / "collector"))


def main():
    executable = ROOT / "dist/DouyinLiveMonitor/engine/DouyinMonitorEngine.exe"
    archive = CArchiveReader(str(executable))
    pyz_name = next(name for name, entry in archive.toc.items() if entry[-1] == "z")
    embedded_code = archive.open_embedded_archive(pyz_name).extract("proxy_pool")
    module = types.ModuleType("proxy_pool")
    module.__file__ = str(executable) + "/proxy_pool.py"
    sys.modules["proxy_pool"] = module
    exec(embedded_code, module.__dict__)

    spec = importlib.util.spec_from_file_location("frozen_proxy_tests", ROOT / "tests/test_proxy_pool.py")
    tests = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(tests)
    names = [
        "test_init_has_no_network_or_threads_until_demand",
        "test_https_probe_keeps_certificate_checks_and_explicit_proxy",
        "test_redirect_is_not_a_successful_homepage_or_assignable_lease",
    ]
    suite = unittest.TestSuite(tests.ProxyPoolTests(name) for name in names)
    class FrozenErrorTextTests(unittest.TestCase):
        def test_tls_failure_wording_describes_the_observed_stage(self):
            for detail, expected in (
                ("CERTIFICATE_VERIFY_FAILED", "经代理访问目标站点时，TLS 证书验证失败"),
                ("UNEXPECTED_EOF", "经代理访问目标站点时，TLS 连接被提前关闭"),
                ("WRONG_VERSION_NUMBER", "代理隧道未返回有效的 TLS 响应"),
            ):
                pool = module.ProxyPool(validator=Mock(side_effect=module.requests.exceptions.SSLError(detail)))
                try:
                    result = pool._check("8.8.8.8:80")
                    self.assertFalse(result[0])
                    self.assertEqual(result[3], expected)
                finally:
                    pool.shutdown(3)
    suite.addTest(FrozenErrorTextTests("test_tls_failure_wording_describes_the_observed_stage"))
    names.append("test_tls_failure_wording_describes_the_observed_stage")
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    report = {
        "ok": result.wasSuccessful(),
        "engine_path": str(executable),
        "engine_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
        "module_source": "proxy_pool bytecode extracted from the distributable engine executable",
        "dependency_source": "project build environment; network mocked",
        "checks": names,
        "tests_run": result.testsRun,
        "network_requests": 0,
    }
    output = ROOT / "build/frozen-proxy-policy-report.json"
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=True, indent=2))
    if not result.wasSuccessful():
        raise SystemExit(1)


if __name__ == "__main__":
    main()
