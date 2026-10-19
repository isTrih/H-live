# 氢看直播 0.3.1：纯 Rust 后端构建

目标 Windows 10/11 x64。需要 PowerShell 7、Rust MSVC 工具链、Node.js 24、Visual Studio Build Tools C++ 桌面工具与 Windows SDK。Node 仅构建界面，不进入运行包；不再需要 CPython、PyO3、PyInstaller 或 V8。

```powershell
npm.cmd ci
. ./scripts/rust-env.ps1
cargo fetch --manifest-path src-tauri/Cargo.toml --locked --target x86_64-pc-windows-msvc
cargo fetch --manifest-path crates/Cargo.toml --locked --target x86_64-pc-windows-msvc
./scripts/test.ps1
./scripts/build.ps1 -SkipInstaller
```

`rust-env.ps1` 查找系统 MSVC 和 SDK，仅设置当前进程环境；Cargo 缓存在被 Git 忽略的 `packaging/tools/cargo-home`。产品使用两个锁定 workspace；独立的签名校验工具也保留自己的锁文件。完整构建依次执行 Vite、实际依赖许可证收集、Tauri Rust 编译、目录版和实际可执行程序的独立离线自检。

测试中的 DPAPI 保护仅用合成凭据，需以当前 Windows 用户运行；TLS 测试只监听回环地址并临时生成自签证书，不写系统证书库。`test-support` 仅供模拟网络测试与 benchmark，默认发行构建不启用。

目录版位于 `dist/DouyinLiveMonitor`。正式签名发布在目录构建通过后运行：

```powershell
./scripts/package_release.ps1 -SigningKey C:/安全目录/qingkan-updater.key
./scripts/verify_release.ps1
./scripts/package_cdn.ps1
```

参数是原私钥的文件路径，不是密钥正文。保留原更新公钥与 CDN 渠道，不能为绕过缺失密钥重新生成一把私钥。私钥不能提交仓库或放入任何发行包。独立 fork 应更换应用标识、更新渠道及自己的签名配置。

`verify_release.ps1` 只读取公开公钥、签名和发行文件，核对版本、清单、SHA256、安装器副本和无解释器载荷，并在内存中翻转一个字节确认篡改会被拒绝；它不读取私钥。首次需按锁文件获取 `packaging/signature-check/Cargo.toml` 的依赖。`package_cdn.ps1` 检查 ZIP 仅含五个指定文件，并逐项核对解压数据哈希。

隔离安装测试：先运行 `packaging/test_nsis.ps1` 验证全新安装及卸载；再传入工作区内保存的 `QingkanWorkspaceTest_旧版本_x64-setup.exe` 和 `-BaselineVersion` 验证升级。只接受独立测试产品名，不能传入用户实际安装器。旧库逐值升级与导出兼容由 core 测试及 runtime 的 `upgrade_contract` example 独立验证。

产物包括 NSIS 安装器及 `.sig`/`.sha256`，以及 `dist/cdn/qingkan-live` 的 `latest.json` 和版本目录。CDN 手动上传步骤见 [CDN_RELEASE.md](CDN_RELEASE.md)。应用更新完整性签名不等于 Windows Authenticode 发行证书。

应用保留 `com.douyinlivemonitor.desktop`、`DouyinLiveMonitor.exe` 和 `%LOCALAPPDATA%/DouyinLiveMonitor` 以兼容升级。新安装不预置直播间。GUI 调用进程内 Rust，不再附带 engine 目录或解释器；安装/升级测试必须使用独立测试标识和目录，不覆盖用户真实安装。

离线模拟性能：

```powershell
cargo run --release --manifest-path crates/Cargo.toml --target-dir src-tauri/target -p h-live-runtime --features test-support --example benchmark -- build/performance.json
```

真实网络诊断只接收命令行传入的目标，最多观察 120 秒，报告只含聚合计数：

```powershell
cargo run --release --manifest-path crates/Cargo.toml --target-dir src-tauri/target -p h-live-runtime --example probe -- 直播间ID 60 build/probe.json
```

合成二进制夹具可通过 `cargo run --manifest-path crates/Cargo.toml -p qingkan-core --example generate_activity_fixtures` 再生成。所有测试和报告留在独立 `build/` 数据范围，不接触真实用户库。
