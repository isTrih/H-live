"""Package only the current public CDN release and verify every archived byte."""
from pathlib import Path
import hashlib
import json
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    version = json.loads((ROOT / "package.json").read_text(encoding="utf-8"))["version"]
    cdn = ROOT / "dist/cdn/qingkan-live"
    name = f"QingkanLive-{version}-windows-x64-setup.exe"
    manifest = json.loads((cdn / "latest.json").read_text(encoding="utf-8"))
    assert manifest["version"] == version
    assert manifest["platforms"]["windows-x86_64"]["url"] == f"https://assets.checkpoint321.com/lilith/qingkan-live/{version}/{name}"
    paths = [cdn / "latest.json", cdn / "UPLOAD-INSTRUCTIONS.md"]
    paths += [cdn / version / (name + suffix) for suffix in ("", ".sig", ".sha256")]
    assert all(path.is_file() for path in paths)
    digest = lambda value: hashlib.sha256(value).hexdigest()
    installer_hash = digest(paths[2].read_bytes())
    assert installer_hash == paths[4].read_text(encoding="ascii").split()[0]
    assert manifest["platforms"]["windows-x86_64"]["signature"] == paths[3].read_text(encoding="utf-8").strip()
    output = ROOT / f"dist/QingkanLive-{version}-cdn.zip"
    expected = {}
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
        for path in paths:
            relative = path.relative_to(cdn.parent).as_posix()
            data = path.read_bytes()
            archive.writestr(relative, data)
            expected[relative] = digest(data)
    with zipfile.ZipFile(output) as archive:
        assert set(archive.namelist()) == set(expected)
        assert archive.testzip() is None
        assert all(digest(archive.read(name)) == value for name, value in expected.items())
    report = {"ok": True, "version": version, "archive": str(output), "bytes": output.stat().st_size,
              "sha256": digest(output.read_bytes()), "installer_sha256": installer_hash,
              "entries": expected, "private_key_included": False,
              "scope": "Five explicitly selected public release files; extracted contents match original hashes"}
    (ROOT / "build/cdn-zip-report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
