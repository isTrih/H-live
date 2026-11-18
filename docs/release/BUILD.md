# 氢看直播：纯 Rust 后端构建

目标 Windows 10/11 x64。需要 PowerShell 7、Rust MSVC 工具链、Node.js 24、Visual Studio Build Tools C++ 桌面工具与 Windows SDK。Node 仅构建界面，不进入运行包；不再需要 CPython、PyO3、PyInstaller 或 V8。

桌面版本在本地构建签名安装包，默认通过 `./scripts/release.ps1` 自动递增 patch 版本并发布到莉莉丝 Sites 内网；失败重跑复用原版本。旧客户端 CDN 永久停留在 0.3.6，之后不再生成旧渠道包。详见 [发布流程](CDN_RELEASE.md)。GitHub Releases 保留给独立服务端 `server-v*` 发布线。

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

目录版位于 `dist/DouyinLiveMonitor`。正式发布直接运行以下命令，自动准备新版本、构建、签名并上传：

```powershell
./scripts/release.ps1 -SigningKey C:/安全目录/qingkan-updater.key
```

参数是原私钥的文件路径，不是密钥正文。保留原更新公钥与 CDN 渠道，不能为绕过缺失密钥重新生成一把私钥。私钥不能提交仓库或放入任何发行包。独立 fork 应更换应用标识、更新渠道及自己的签名配置。

`verify_release.ps1` 只读取公开公钥、签名和发行文件，核对版本、清单、SHA256、安装器副本和无解释器载荷，并在内存中翻转一个字节确认篡改会被拒绝；它不读取私钥。首次需按锁文件获取 `packaging/signature-check/Cargo.toml` 的依赖。

隔离安装测试：先运行 `packaging/test_nsis.ps1` 验证全新安装及卸载；再传入工作区内保存的 `QingkanWorkspaceTest_旧版本_x64-setup.exe` 和 `-BaselineVersion` 验证升级。只接受独立测试产品名，不能传入用户实际安装器。旧库逐值升级与导出兼容由 core 测试及 runtime 的 `upgrade_contract` example 独立验证。

产物包括 NSIS 安装器及 `.sig`/`.sha256`，以及 `dist/cdn/qingkan-live` 的 `latest.json` 和版本目录。此目录为内网站点的本地暂存输入；正式上传目录是 `dist/sites/qingkan-live`。内网发布流程及旧客户端一次性迁移步骤见 [CDN_RELEASE.md](CDN_RELEASE.md)。应用更新完整性签名不等于 Windows Authenticode 发行证书。

应用保留 `com.douyinlivemonitor.desktop`、`DouyinLiveMonitor.exe` 和 `%LOCALAPPDATA%/DouyinLiveMonitor` 以兼容升级。新安装不预置直播间，代理池与调试模式默认关闭；升级保留已明确保存的代理启用偏好。常规轮询不写运行日志，异常记录与调试日志使用最多两份、各 8 MiB 的轮转文件。GUI 调用进程内 Rust，不再附带 engine 目录或解释器；安装/升级测试必须使用独立测试标识和目录，不覆盖用户真实安装。

离线模拟性能：

```powershell
cargo run --release --manifest-path crates/Cargo.toml --target-dir src-tauri/target -p h-live-runtime --features test-support --example benchmark -- build/performance.json
```

真实网络诊断只接收命令行传入的目标，最多观察 120 秒，报告只含聚合计数：

```powershell
cargo run --release --manifest-path crates/Cargo.toml --target-dir src-tauri/target -p h-live-runtime --example probe -- 直播间ID 60 build/probe.json
```

合成二进制夹具可通过 `cargo run --manifest-path crates/Cargo.toml -p qingkan-core --example generate_activity_fixtures` 再生成。所有测试和报告留在独立 `build/` 数据范围，不接触真实用户库。

0.4.0 的媒体组件由 `scripts/prepare_media_tools.ps1` 按 `packaging/media-tools.json` 和 `packaging/native-player.json` 固定下载并校验 SHA256。可先运行一次，再用 `-Offline` 从缓存准备。安装器与目录发行版都包含 `media-tools/`；播放使用 libmpv 的 D3D11 渲染及自动硬件解码，切片使用 FFmpeg/ffprobe 直接封装，不从用户 PATH 查找程序。许可证、构建参数和源代码出处随包保存。开发用 `QINGKAN_MEDIA_TOOLS_DIR` 可显式指定工具目录。

Windows 原生播放器的实际画面回归（需要公司网络和根目录 `.env` 中的 `INTERNAL_API_TOKEN`）：

```powershell
node scripts/build_replay_diagnostic.mjs
./scripts/verify_native_replay.ps1 -Stage playing -Complete
./scripts/verify_native_replay.ps1 -Stage paused -Complete
./scripts/verify_native_replay.ps1 -Stage fullscreen -Complete
```

该脚本只启动独立目录中的诊断实例，每个状态保留十多秒，捕获该实例客户区并检查四块图像区域，同时核对单份控件、暂停、跳转、音量和全屏。截图与报告保存在 `build/native-visible-*`。媒体解码截图及硬件解码状态不能单独证明屏幕上的画面可见；像素检查依赖当前固定彩色录像片段，场景变更后需目视复核。
