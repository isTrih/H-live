"""Summarize this release's evidence without fixed test counts or signing secrets."""
from datetime import datetime, timezone
from pathlib import Path
import argparse
import hashlib
import json

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read(relative):
    return json.loads((ROOT / relative).read_text(encoding="utf-8-sig"))


def main():
    version = read("package.json")["version"]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--validation", default=f"build/release-validation-{version}.json",
                        help="Current source/UI counts and evidence, prepared from actual completed checks")
    args = parser.parse_args()
    validation = read(args.validation)
    assert validation["ok"] is True and validation["version"] == version
    for section in ("source_validation", "ui_validation"):
        assert validation[section]["passed"] is True
        for relative in validation[section]["logs"]:
            assert (ROOT / relative).is_file(), relative
        for relative, expected in validation[section].get("log_sha256", {}).items():
            assert digest(ROOT / relative) == expected, relative
    installer = ROOT / f"dist/installer/QingkanLive-{version}-windows-x64-setup.exe"
    checksum = digest(installer)
    branding = read("build/branding-report.json")
    signature = read(".artifacts/release-signature-verification.json")
    cdn_zip = read("build/cdn-zip-report.json")
    install_path = f"build/nsis-install-check-{version}/report.json"
    installation = read(install_path)
    assert all(item["ok"] for item in (branding, signature, cdn_zip, installation))
    assert branding["release_version"] == signature["version"] == cdn_zip["version"] == installation["version"] == version
    assert signature["sha256"] == cdn_zip["installer_sha256"] == branding["executables"][-1]["sha256"] == checksum
    payload = ROOT / "dist/DouyinLiveMonitor"
    staged = ROOT / "src-tauri/bundle-docs"
    documents = {}
    for path in sorted(staged.rglob("*")):
        if path.is_file():
            relative = path.relative_to(staged)
            expected = digest(ROOT / relative)
            assert expected == digest(path) == digest(payload / relative), relative
            documents[relative.as_posix()] = expected
    license_count = 0
    license_root = ROOT / "packaging/licenses"
    for path in license_root.rglob("*"):
        if path.is_file():
            relative = path.relative_to(license_root)
            assert digest(path) == digest(ROOT / "src-tauri/bundle-licenses" / relative) == digest(payload / "licenses" / relative), relative
            license_count += 1
    evidence = {
        "runtime_assets": "build/runtime-assets-check.json",
        "native_sidecar_rpc": f"build/smoke-report-{version}.json",
        "frozen_exports": f"build/frozen-export-check-{version}/report.json",
        "branding_and_all_pe_versions": "build/branding-report.json",
        "signature_manifest_and_tamper_rejection": ".artifacts/release-signature-verification.json",
        "same_updater_signing_key": f"build/signing-key-continuity-{version}.json",
        "cdn_zip_contents": "build/cdn-zip-report.json",
        "isolated_version_upgrade": install_path,
        **validation.get("additional_checks", {}),
    }
    checks = {}
    for name, relative in evidence.items():
        result = read(relative)
        assert result.get("ok", result.get("passed")) is True, relative
        checks[name] = {"passed": True, "report": relative, "report_sha256": digest(ROOT / relative)}
    runtime = read(f"build/smoke-report-{version}.json")
    assert runtime["version"] == runtime["checks"]["native_version"] == version and runtime["checks"]["processing"] == "rust"
    frozen = read(f"build/frozen-export-check-{version}/report.json")
    assert frozen["native_version"] == version and frozen["processing"] == "rust"
    native = payload / "engine/_internal/qingkan_native.pyd"
    assert native.is_file()
    assert digest(native) == validation["source_validation"]["native_sha256"]
    protected_path = f"build/protected-release-baseline-{version}.json"
    protected = read(protected_path)
    for relative, expected in protected["files"].items():
        assert digest(ROOT / relative) == expected, relative
    report = {
        "ok": True, "display_name": "氢看直播", "version": version,
        "validated_at": datetime.now(timezone.utc).isoformat(),
        "current_validation_scope": validation["scope"],
        "checks": checks,
        "source_validation": validation["source_validation"],
        "ui_validation": validation["ui_validation"],
        "installer": {"path": str(installer), "file_name": installer.name, "size_bytes": installer.stat().st_size,
                      "sha256": checksum, "format": "Tauri NSIS currentUser", "updater_signature_verified": True,
                      "windows_authenticode_signed": False},
        "cdn_zip": cdn_zip,
        "gui_sha256": digest(payload / "DouyinLiveMonitor.exe"),
        "engine_sha256": digest(payload / "engine/DouyinMonitorEngine.exe"),
        "native_extension_sha256": digest(native),
        "pe_versions": [{key: item[key] for key in ("path", "product_version", "file_version")} for item in branding["executables"]],
        "document_consistency": {"source_equals_staged_equals_directory_release": True,
                                 "file_count": len(documents), "sha256": documents},
        "license_consistency": {"source_equals_staged_equals_directory_release": True, "file_count": license_count},
        "installation_validation": {"report": install_path, "result": installation,
            "scope": validation.get("installation_scope", "Workspace-only fixture uses a separate product/registry/binary name and the production payload. Exact tested versions, resource hashes, upgrade and uninstall/data retention are recorded in the referenced report; final artifact hashes and signatures are verified separately."),
            "real_user_installation_changed": False, "live_cdn_update_tested": False},
        "data_and_identity": {"identifier": "com.douyinlivemonitor.desktop", "data_directory": "%LOCALAPPDATA%/DouyinLiveMonitor",
                              "signing_key_regenerated": False, "user_preview_restarted": False},
        "previous_releases": {"baseline": protected_path, "file_count": len(protected["files"]), "sha256_unchanged": True},
        "limits": validation.get("limits", []) + [
            "CDN upload and online update chain remain user-operated and unverified here.",
            "No Windows Authenticode certificate; updater minisign integrity signature is verified.",
            "Python/V8 remain for networking/signing/orchestration; this release does not remove the Python runtime."],
    }
    output = ROOT / "build/release-package-report.json"
    content = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    output.write_text(content, encoding="utf-8")
    archived = ROOT / f"build/releases/{version}/release-package-report.json"
    archived.parent.mkdir(parents=True, exist_ok=True)
    archived.write_text(content, encoding="utf-8")
    print(json.dumps({"ok": True, "version": version, "installer_sha256": checksum,
                      "installer_bytes": installer.stat().st_size, "documents": len(documents),
                      "license_files": license_count, "report": str(output)}, ensure_ascii=False))


if __name__ == "__main__":
    main()
