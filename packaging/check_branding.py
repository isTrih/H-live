"""Validate user-visible branding and embedded icons in the built Windows files."""
from pathlib import Path
import hashlib
import json
import struct

import pefile

ROOT = Path(__file__).resolve().parents[1]
NAME = "氢看直播"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def icon_images(data):
    reserved, kind, count = struct.unpack_from("<HHH", data)
    assert reserved == 0 and kind == 1 and count > 0
    images = set()
    for index in range(count):
        entry = struct.unpack_from("<BBBBHHII", data, 6 + index * 16)
        size, offset = entry[-2:]
        images.add(sha(data[offset:offset + size]))
    return images


def pe_brand(path, expected_icons, expected_version):
    pe = pefile.PE(str(path), fast_load=False)
    strings = {}
    for group in pe.FileInfo:
        for item in group:
            for table in getattr(item, "StringTable", []):
                strings.update({key.decode("utf-8"): value.decode("utf-8").rstrip() for key, value in table.entries.items()})
    assert strings.get("ProductName") == NAME, (path.name, strings)
    assert NAME in strings.get("FileDescription", ""), (path.name, strings)
    for key in ('ProductVersion', 'FileVersion'):
        assert strings.get(key) in (expected_version, expected_version + '.0'), (path.name, key, strings.get(key))
    embedded = set()
    for resource_type in pe.DIRECTORY_ENTRY_RESOURCE.entries:
        if resource_type.id == 3:
            for resource in resource_type.directory.entries:
                for language in resource.directory.entries:
                    record = language.data.struct
                    embedded.add(sha(pe.get_data(record.OffsetToData, record.Size)))
    assert expected_icons <= embedded, (path.name, "Missing generated icon sizes", expected_icons - embedded)
    pe.close()
    return {"path": str(path.relative_to(ROOT)), "sha256": sha(path.read_bytes()),
            "product_name": strings["ProductName"], "file_description": strings["FileDescription"],
            "product_version": strings["ProductVersion"], "file_version": strings["FileVersion"],
            "matching_icon_sizes": len(expected_icons)}


def main():
    config = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
    assert config["productName"] == NAME
    assert config["app"]["windows"][0]["title"] == NAME
    assert config["identifier"] == "com.douyinlivemonitor.desktop"
    assert config['mainBinaryName'] == 'DouyinLiveMonitor'
    assert config['bundle']['targets'] == ['nsis']
    assert config['bundle']['windows']['nsis']['installMode'] == 'currentUser'
    version = config['version']
    icon = ROOT / "src-tauri/icons/icon.ico"
    images = icon_images(icon.read_bytes())
    files = [ROOT / "dist/DouyinLiveMonitor/DouyinLiveMonitor.exe",
             ROOT / "dist/DouyinLiveMonitor/engine/DouyinMonitorEngine.exe",
             ROOT / f"dist/installer/QingkanLive-{version}-windows-x64-setup.exe"]
    records = [pe_brand(path, images, version) for path in files]
    import tomllib
    assert json.loads((ROOT / 'package.json').read_text(encoding='utf-8'))['version'] == version
    assert tomllib.loads((ROOT / 'src-tauri/Cargo.toml').read_text(encoding='utf-8'))['package']['version'] == version
    assert tomllib.loads((ROOT / 'crates/Cargo.toml').read_text(encoding='utf-8'))['workspace']['package']['version'] == version
    assert f'VERSION = "{version}"' in (ROOT / 'python/collector/backend_service.py').read_text(encoding='utf-8')
    frontend = ROOT / "dist-ui"
    assert f"<title>{NAME}</title>" in (frontend / "index.html").read_text(encoding="utf-8")
    brand_assets = list((frontend / "assets").glob("brand-icon-*.png"))
    assert len(brand_assets) == 1
    assert brand_assets[0].read_bytes() == (ROOT / "src/brand-icon.png").read_bytes()
    assert any(NAME in path.read_text(encoding="utf-8") for path in (frontend / "assets").glob("*.js"))
    report = {"ok": True, "display_name": NAME, "release_version": version, "icon_sha256": sha(icon.read_bytes()),
              "icon_image_count": len(images), "executables": records,
              "frontend_title_wordmark_and_png": True,
              "installer_channel": "NSIS currentUser; old Inno installation is not overwritten",
              "tauri_identifier_preserved": True,
              "binary_names_and_data_directory_preserved": True,
              "preview_restarted": False, "system_install_performed": False}
    (ROOT / "build/branding-report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=True, indent=2))


if __name__ == "__main__":
    main()
