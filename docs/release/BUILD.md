# 氢看直播 0.2.4：Windows 构建与发布

目标为 Windows 10/11 x64。Tauri 2 / Rust / WebView2 运行界面，独立 Python 后台运行网络采集、签名、调度及代理池，同进程 qingkan_native.pyd 以 Rust 处理 SQLite 归档、月导出和 protobuf 解码；两者使用私有 UTF-8 JSON 管道，没有本地 HTTP 服务。成品包含 Python、V8、SQLite、证书与代理依赖，无需用户另装 Python、Node.js、Excel 或播放器。

## 工具与构建

已验证工具：CPython 3.12.14、Node.js 24、Rust 1.98.0 MSVC、MSVC 14.51、微软 SDK 10.0.26100.7627。新机器先安装 Visual Studio Build Tools 的「使用 C++ 的桌面开发」工作负载，包含 MSVC x64 和 Windows 10/11 SDK，再安装 Rust MSVC 工具链。`scripts/rust-env.ps1` 优先发现系统 Windows Kits SDK，只设置当前进程环境；本机已有的 `packaging/tools/windows-sdk-*` NuGet SDK 仅作后备，不是干净检出的前置文件。Cargo 缓存位于工作区 `packaging/tools/cargo-home`。首次 NSIS 打包由官方 Tauri CLI 下载工具到本地构建目录。

```powershell
python -m venv .venv
.\.venv\Scripts\python.exe -m pip install -r requirements-build.txt
npm.cmd ci
. .\scripts\rust-env.ps1
cargo fetch --manifest-path src-tauri/Cargo.toml --locked --target x86_64-pc-windows-msvc
cargo fetch --manifest-path crates/Cargo.toml --locked --target x86_64-pc-windows-msvc
.\scripts\build.ps1 -SkipInstaller
```

完成前置安装与依赖获取后，build.ps1 执行依赖/V8检查、PyO3 原生扩展构建与版本检查、两个 Cargo workspace 的许可收集、PyInstaller、Vite、Rust 离线锁定构建、完整目录版、自有后台离线自检，最后生成签名 NSIS 安装器及 CDN 文件。-SkipInstaller 只生成目录版；-SkipEngine 只用于明确复用本轮已冻结的后台，Python 源码变更后必须重新冻结。正常构建不会生成或覆盖图标。发布图标为 assets/branding/qingkan-app-icon-v2.png 的尺寸转换产物；旧版源图保留但不发布。

首次检出按上例先生成不需要签名私钥的目录版。只有持有原更新密钥的维护者才运行不带 `-SkipInstaller` 的完整发布，或在目录版完成后运行 `scripts/package_release.ps1`；不要为绕过缺失私钥而生成一把新密钥替代现有更新渠道。

`scripts/build_native.ps1` 使用当前 `.venv` 的 CPython 构建 `crates/qingkan-native`，输出到 `src-tauri/target`，复制为 `python/collector/qingkan_native.pyd`。这是必需的原生扩展，缺失或版本不一致会停止构建；没有 Python 静默回退。`packaging/backend.spec` 的生产入口是 `python/collector/backend_service.py`，不打包旧 Tk、betterproto 或 Python 生成的 protobuf 模型。

开发测试安装 `requirements-dev.txt`，其中 betterproto 仅用于历史 protobuf fixture 与差分验证。统一运行 `scripts/test.ps1`；可用 `-Suite python/frontend/rust` 选择测试集合，首次 Python 测试会先构建原生扩展。该入口包含 `tests/`、`backend_tests/`、`native_tests/` 和递归前端测试，不包含 `legacy/tk/tests/`。源码后台可在项目根设置 `$env:PYTHONPATH = (Resolve-Path python/collector).Path` 后运行 `.venv/Scripts/python.exe python/collector/backend_service.py`，请为手工调试设置独立 `DOUYIN_MONITOR_HOME`。

发布文档以 `packaging/release-resources.json` 为唯一白名单，保留与仓库一致的相对层级。staging 只重建明确的 `src-tauri/bundle-docs` 和 `bundle-licenses`，不删除历史 `dist/installer`、CDN 版本目录或 ZIP。两份 Cargo.lock 的 Windows 实际运行及构建依赖均收集许可证，字体仍按原始字节校验。

字体以原始 OTF 嵌入，构建校验 src/fonts/font-manifest.json 中的 SHA-256。完整 OFL 原文、来源与清单随 licenses/fonts/ 分发。紧凑图文控件使用各字体独立的 CSS 光学度量；字体轮廓、正文与大数字度量不变。

## 安装与更新渠道

正式版统一使用 Tauri 原生 NSIS currentUser 安装器，以便 updater 验签后安装同一格式。默认安装在当前用户目录内的新「氢看直播」目录，不要求管理员权限。DouyinLiveMonitor.exe、DouyinMonitorEngine.exe、Tauri identifier com.douyinlivemonitor.desktop 及数据目录 %LOCALAPPDATA%\DouyinLiveMonitor 保持兼容。

此前 1.0/2.0 Inno 试制版不直接进入原生 OTA 渠道。请关闭旧版，再一次性手动安装当前正式版到默认的新目录；安装器不会静默卸载旧版，也会拒绝覆盖旧 Inno 安装目录。旧 legacy/inno/installer.iss 仅为历史兼容材料，正常发布不调用它。旧 Inno AppId 未被修改或用于 NSIS 冒充升级。卸载不删除独立的数据目录。

WebView2 引导程序由 Tauri 嵌入安装器，目标机缺少运行时时需联网安装；已有运行时可离线安装。安装器未配置发行方 Authenticode 证书，Windows 可能显示未知发布者；Tauri .sig 用于应用内更新完整性验证，两者不是同一种签名。

产物：

- dist/DouyinLiveMonitor/：完整目录版，保留 engine、licenses 与文档。
- dist/installer/QingkanLive-0.2.4-windows-x64-setup.exe 及 .sig、.sha256。
- dist/cdn/qingkan-live/：可手动上传的 latest.json、版本子目录和说明。

发布说明见 [CDN_RELEASE.md](CDN_RELEASE.md)。私钥只保存在已被 Git 忽略且限制 ACL 的 .release-secrets/，不能上传或放入安装包。后续更新必须使用相同密钥。仅同步文档后重打包可运行 scripts/package_release.ps1；代码或前端改变后应先完整构建。

## 验证边界

build/smoke-report-0.2.4.json 是当前构建的 Rust→冻结后台离线自检：验证版本、init/poll、update_readiness、错误响应、shutdown 和子进程退出，使用独立 DOUYIN_MONITOR_HOME。它不会联网抓直播，也不代表 WebView 视觉验收。

packaging/test_frozen_exports.py 验证真实冻结程序的 CSV/XLSX、长 ID、中文 emoji、公式保护与缺失值；packaging/test_frozen_proxy_import.py 使用合成认证节点验证导入、DPAPI 和重启读取，不使用用户的真实代理资料。check_frozen_proxy_policy.py 从实际冻结程序提取模块并模拟 HTTP 200/302/TLS 响应，不发网络请求。check_branding.py 检查 PE 名称与各尺寸图标、前端资源、版本与兼容标识。最终报告记录当前安装器哈希，不能用历史 Inno 验收冒充新 NSIS 完整验收。

历史 test_installer.ps1、legacy fixture、窗口生命周期与 DPI 报告保留作此前版本证据。当前用户预览及真实 1.0 安装不可被测试关闭或覆盖；新 NSIS 安装测试使用独立标识、工作区目录与测试二进制文件名，避免按同名进程干扰用户预览。卸载验收保留测试数据并确认测试注册清理。

## 开发预览

运行 npm.cmd run dev 后可用 scripts/start_dev_preview.ps1 打开原生预览；它连接已有 127.0.0.1:1420，使用正常用户数据，不自动启动监控。构建不会关闭已经运行的预览。用户允许更新预览时才用 -Restart，该选项走正常关闭并等待归档完成，不强制结束未退出的进程。

## 许可与来源

上游 MIT 保留于 LICENSE。Python、Rust、JavaScript 与运行库通知在 THIRD_PARTY_NOTICES.txt、THIRD_PARTY_DESKTOP_NOTICES.txt 和 licenses/。新增 PySocks、python-socks、Tauri updater 与字体许可均包含。缺失 crate 原文按发布包记录的固定官方提交补齐在 packaging/extra-licenses/；构建不会从任意来源执行脚本。

官方参考：[Tauri Windows 安装器](https://v2.tauri.app/distribute/windows-installer/)、[Tauri updater](https://v2.tauri.app/plugin/updater/)、[微软 WebView2 部署](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution)。
