"""Check only this application's source versions, without loading native code."""
import ast
import json
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def main():
    def read(name):
        return (ROOT / name).read_text(encoding="utf-8-sig")
    def json_file(name):
        return json.loads(read(name))
    def toml(name):
        return tomllib.loads(read(name))
    version = json_file("package.json")["version"]
    assert re.fullmatch(r"\d+\.\d+\.\d+", version)
    npm_lock = json_file("package-lock.json")
    tauri = json_file("src-tauri/tauri.conf.json")
    values = {
        "package": version, "npm_lock": npm_lock["version"],
        "npm_lock_root": npm_lock["packages"][""]["version"], "tauri": tauri["version"],
        "desktop_cargo": toml("src-tauri/Cargo.toml")["package"]["version"],
        "native_workspace": toml("crates/Cargo.toml")["workspace"]["package"]["version"],
    }
    for path, names in (("src-tauri/Cargo.lock", ("douyin-live-monitor",)),
                        ("crates/Cargo.lock", ("qingkan-core", "qingkan-native"))):
        packages = {entry["name"]: entry for entry in toml(path)["package"] if entry["name"] in names}
        for name in names:
            values[f"locked_{name}"] = packages[name]["version"]
    tree = ast.parse(read("python/collector/backend_service.py"))
    values["backend"] = next(ast.literal_eval(node.value) for node in tree.body if isinstance(node, ast.Assign)
                             and any(isinstance(target, ast.Name) and target.id == "VERSION" for target in node.targets))
    values["preview"] = re.search(r'\bversion:\s*"([\d.]+)"', read("src/preview.js")).group(1)
    info = read("packaging/version_info.txt")
    for name in ("FileVersion", "ProductVersion"):
        values[f"engine_{name}"] = re.search(rf"StringStruct\('{name}', '([\d.]+)'\)", info).group(1)
    expected_tuple = tuple(map(int, version.split("."))) + (0,)
    for name in ("filevers", "prodvers"):
        assert ast.literal_eval(re.search(rf"{name}=(\([^)]*\))", info).group(1)) == expected_tuple
    assert all(value == version for value in values.values()), values
    for name in ("qingkan-core", "qingkan-native"):
        assert toml(f"crates/{name}/Cargo.toml")["package"]["version"]["workspace"] is True
    assert f'../release-notes/{version}.md?raw' in read("src/update-panel.js")
    assert (ROOT / f"release-notes/{version}.md").is_file()
    assert tauri["identifier"] == "com.douyinlivemonitor.desktop"
    report = {"ok": True, "version": version, "source_versions": values,
              "scope": "Source/configuration only; PE and bundled native versions are checked after the build"}
    output = ROOT / f"build/source-versions-{version}.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False))


if __name__ == "__main__":
    main()
