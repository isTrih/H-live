"""Offline smoke of proxy import and update readiness in the frozen sidecar.

Uses only synthetic credentials and an isolated data directory. Never requests
proxy refresh, monitoring, or any other operation that initiates network I/O.
"""
from contextlib import contextmanager
from pathlib import Path
import json
import os
import queue
import subprocess
import sys
import tempfile
import threading

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python" / "collector"))
from app_storage import load_config

SECRETS = ("frozen-fixture-http", "synthetic:http/@", "frozen-fixture-socks", "synthetic:socks/@")


def main():
    executable = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / "dist/DouyinMonitorEngine/DouyinMonitorEngine.exe"
    directory = Path(tempfile.mkdtemp(prefix="frozen-proxy-import-", dir=ROOT / "build"))
    data = directory / "data"
    report = {"ok": False, "executable": str(executable), "data_directory": str(data),
              "fixture": "Synthetic credentials only; no monitoring or proxy refresh requested", "checks": []}
    environment = {**os.environ, "DOUYIN_MONITOR_HOME": str(data), "PYTHONIOENCODING": "utf-8"}

    @contextmanager
    def sidecar(round_number):
        with (directory / f"stderr-{round_number}.log").open("w", encoding="utf-8") as errors:
            child = subprocess.Popen([str(executable)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=errors, text=True, encoding="utf-8", env=environment,
                                     creationflags=subprocess.CREATE_NO_WINDOW)
            responses = queue.Queue()
            def receive():
                for line in child.stdout:
                    responses.put(line)
            threading.Thread(target=receive, daemon=True).start()
            serial = 0
            def rpc(method, params=None, expect_ok=True):
                nonlocal serial
                serial += 1
                child.stdin.write(json.dumps({"id": serial, "method": method, "params": params or {}}, ensure_ascii=False) + "\n")
                child.stdin.flush()
                line = responses.get(timeout=40)
                assert not any(secret in line for secret in SECRETS), "Credential in RPC output"
                response = json.loads(line)
                assert response["id"] == serial and response["ok"] is expect_ok, (method, response)
                return response["result"] if expect_ok else response["error"]
            try:
                initialized = rpc("init")
                expected_version = json.loads((ROOT / "package.json").read_text(encoding="utf-8"))["version"]
                assert initialized["version"] == expected_version
                assert initialized["native_version"] == expected_version
                assert initialized["processing"] == "rust"
                yield rpc
                assert rpc("update_readiness")["ready"] is True
                result = rpc("shutdown")
                assert result["closed"] and result["all_workers_stopped"], result
                assert child.wait(timeout=20) == 0
            finally:
                if child.poll() is None:
                    child.kill()
                    child.wait()

    try:
        fixtures = [
            {"ip": "8.8.8.8", "port": 18080, "protocol": "http", "username": SECRETS[0], "password": SECRETS[1], "status": "available"},
            {"ip": "1.1.1.1", "port": 11080, "protocol": "socks5", "username": SECRETS[2], "password": SECRETS[3], "status": "available"},
        ]
        text = json.dumps(fixtures, ensure_ascii=False)
        with sidecar(1) as rpc:
            assert rpc("configure_proxy", {"enabled": False})["enabled"] is False
            preview = rpc("preview_proxies", {"text": text})
            assert (preview["valid"], preview["duplicates"], preview["invalid"]) == (2, 0, 0)
            assert all(row["auth_present"] and row["status"] == "unchecked" for row in preview["items"])
            assert {row["protocol"] for row in preview["items"]} == {"http", "socks5h"}
            assert rpc("list_imported_proxies")["total"] == 0
            report["checks"].append("Preview masks credentials, normalizes protocols, ignores vendor status, and does not persist")
            assert rpc("import_proxies", {"text": text})["added"] == 2
            assert rpc("import_proxies", {"text": text})["added"] == 0
            rows = rpc("list_imported_proxies")["items"]
            assert len(rows) == 2
            assert rpc("proxy_status")["enabled"] is False
            config_path = data / "config.json"
            raw = config_path.read_text(encoding="utf-8")
            config = json.loads(raw)
            assert config.get("imported_proxies_encrypted") and "imported_proxies" not in config
            assert not any(secret in raw for secret in SECRETS), "Plaintext in config"
            restored = load_config(data_dir=data)["imported_proxies"]
            assert [(p["username"], p["password"]) for p in restored] == [(SECRETS[0], SECRETS[1]), (SECRETS[2], SECRETS[3])]
            report["checks"].append("Frozen binary stores DPAPI ciphertext; same Windows user decrypts original synthetic values")
            before = config_path.read_bytes()
            rpc("configure_proxy", {"enabled": False, "source": "invalid-fixture-source"}, expect_ok=False)
            rpc("import_proxies", {"text": "[{broken-fixture-json"}, expect_ok=False)
            assert config_path.read_bytes() == before
            assert rpc("list_imported_proxies")["total"] == 2
            assert rpc("configure_proxy", {"enabled": False, "source": "imported"})["source"] == "imported"
            report["checks"].append("Repeated import deduplicates; rejected requests preserve configuration; import does not enable proxies")
        with sidecar(2) as rpc:
            state = rpc("init")["settings"]
            assert state["proxy_source"] == "imported" and state["proxy_enabled"] is False
            assert state["imported_proxy_count"] == 2
            rows = rpc("list_imported_proxies")["items"]
            assert len(rows) == 2 and all(row["auth_present"] for row in rows)
            assert rpc("configure_proxy", {"enabled": True, "source": "imported"})["enabled"] is True
            assert rpc("update_readiness")["ready"] is True
            assert rpc("configure_proxy", {"enabled": False})["enabled"] is False
            assert rpc("remove_imported_proxies", {"ids": [row["id"] for row in rows]})["removed"] == 2
            assert rpc("list_imported_proxies")["total"] == 0
            report["checks"].append("Restart restores protected records; idle mode switches and deletes succeed without starting network work")
        with sidecar(3) as rpc:
            assert rpc("list_imported_proxies")["total"] == 0
            assert rpc("init")["settings"]["proxy_enabled"] is False
        for path in data.rglob("*"):
            if path.is_file():
                content = path.read_bytes()
                assert not any(secret.encode() in content for secret in SECRETS), "Plaintext in data directory"
        for path in directory.glob("stderr-*.log"):
            assert not any(secret in path.read_text(encoding="utf-8") for secret in SECRETS), "Credential in stderr"
        report["checks"].append("Deletion persists after second restart; three readiness checks, clean shutdowns and zero-exit processes")
        report["ok"] = True
    finally:
        output = directory / "report.json"
        output.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        print(json.dumps(report, ensure_ascii=False, indent=2))
        print(f"Report: {output}")


if __name__ == "__main__":
    main()
