"""Independently verify the public CDN release files; never read signing secrets."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile
import time
import tomllib

ROOT = Path(__file__).resolve().parents[1]
CDN_BASE = "https://assets.checkpoint321.com/lilith/qingkan-live/"


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--report", default=".artifacts/release-signature-verification.json")
    args = parser.parse_args()
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", args.version):
        parser.error("--version must be a release SemVer, for example 0.1.0")
    started = time.monotonic()
    result = {"version": args.version, "checked_at": datetime.now(timezone.utc).isoformat(),
              "private_key_read": False, "network_used": False, "checks": {}}
    try:
        cdn = ROOT / "dist/cdn/qingkan-live"
        filename = f"QingkanLive-{args.version}-windows-x64-setup.exe"
        artifact = cdn / args.version / filename
        signature = artifact.with_suffix(".exe.sig")
        manifest = json.loads((cdn / "latest.json").read_text(encoding="utf-8-sig"))
        config = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text(encoding="utf-8-sig"))
        npm = json.loads((ROOT / "package.json").read_text(encoding="utf-8-sig"))
        cargo = tomllib.loads((ROOT / "src-tauri/Cargo.toml").read_text(encoding="utf-8-sig"))
        native = tomllib.loads((ROOT / "crates/Cargo.toml").read_text(encoding="utf-8-sig"))
        platform = manifest["platforms"]["windows-x86_64"]
        checks = result["checks"]
        checks["versions_match"] = all(value == args.version for value in
            (manifest["version"], config["version"], npm["version"], cargo["package"]["version"], native["workspace"]["package"]["version"]))
        checks["endpoint_matches"] = config["plugins"]["updater"]["endpoints"] == [CDN_BASE + "latest.json"]
        checks["download_url_matches"] = platform["url"] == CDN_BASE + args.version + "/" + filename
        checks["manifest_signature_matches_file"] = platform["signature"] == signature.read_text(encoding="utf-8-sig").strip()
        checks["release_notes_match"] = manifest["notes"].strip() == (ROOT / f"release-notes/{args.version}.md").read_text(encoding="utf-8-sig").strip()
        published = datetime.fromisoformat(manifest["pub_date"].replace("Z", "+00:00"))
        checks["publication_date_has_timezone"] = published.tzinfo is not None
        checksum = digest(artifact)
        checksum_line = artifact.with_suffix(".exe.sha256").read_text(encoding="ascii").strip().split()
        checks["sha256_file_matches"] = len(checksum_line) == 2 and checksum_line == [checksum, filename]
        checks["installer_copy_matches"] = digest(ROOT / "dist/installer" / filename) == checksum
        verifier = ROOT / "build/signature-check/release/qingkan-signature-check.exe"
        # Derive the public file from the exact key embedded in the application.
        # This path is deliberately separate from .release-secrets.
        with tempfile.TemporaryDirectory(prefix="release-public-check-", dir=ROOT / "build") as temp:
            public_key = Path(temp) / "updater-public-key.pub"
            public_key.write_text(config["plugins"]["updater"]["pubkey"], encoding="ascii")
            verified = subprocess.run([str(verifier), str(public_key), str(signature), str(artifact)],
                                      capture_output=True, text=True, timeout=120, check=False)
            result["signature_verification"] = json.loads(verified.stdout)
            checks["signature_valid"] = verified.returncode == 0 and result["signature_verification"]["signature_valid"] is True
            checks["tampered_artifact_rejected"] = result["signature_verification"]["tampered_rejected"] is True
            checks["verifier_hash_matches"] = result["signature_verification"]["sha256"] == checksum
        result.update(artifact=str(artifact), bytes=artifact.stat().st_size, sha256=checksum,
                      manifest_url=CDN_BASE + "latest.json", download_url=platform["url"])
        result["ok"] = all(checks.values())
    except Exception as error:
        # No raw file contents or exception strings are included in this report.
        result.update(ok=False, error_type=type(error).__name__)
    result["elapsed_seconds"] = round(time.monotonic() - started, 3)
    report = ROOT / args.report
    report.parent.mkdir(parents=True, exist_ok=True)
    report.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(result, ensure_ascii=False))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
