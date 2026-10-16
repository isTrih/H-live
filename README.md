# H-live · 氢看直播

Windows 多主播直播消息监控应用。**0.3.0 使用 Rust 后端 + Tauri 2 界面，不附带 Python、Node、独立 V8 签名引擎或外部签名服务。** 保留原有顶部菜单、监控网格、字体、场次分析和月度导出界面。

Windows 安装器与 CDN 更新文件见 [Releases](https://github.com/isTrih/H-live/releases)。更新清单由维护者手动上传 CDN。

## 免责声明

本项目仅用于学习研究，严禁用于商业谋利、破坏系统、盗取个人信息等行为。 侵权或涉及相关利益请联系原作者。

本项目与抖音、字节跳动不存在官方关联。免责声明见 [DISCLAIMER.md](DISCLAIMER.md)。上游版权与许可证原文保留，不因项目改名而变更。

## 功能

- 多主播异步监控、开播优先并按在线人数倒序、未开播折叠、无限滚动、人数刷新和默认浏览器打开直播间。
- 通过游客 HTTPS/WSS 接收聊天、表情、关注/分享、粉丝团、点赞、红包和福袋。**没有登录 Cookie 输入/调用接口，不新增礼物记录。** 历史礼物仍可查询与导出；进入消息不出现在实时消息页。
- 按真实主播与直播场次归档；同场相同开奖时间的福袋合并；暂停后恢复同场次时，继续累计已记录时长，中间停止时间不计入。
- 场次峰值在线、分钟平均在线、本地规则有效发言统计及相关比例，不接入 AI。
- CSV/XLSX 月导出，包含发送人 ID 和名字；多人按主播分 sheet，单人文件名带主播名；支持按主播、月份或场次删除记录。
- 代理文本、认证 URL、JSON、CSV/TSV/表格及 HTTPS URL 订阅导入；直连和代理共同作为出口，按出口限制请求、建连与冷却；失败候选跳过，过期/移除的出口逐步退役。
- 单例、macOS 风格顶部窗口按钮、两种字体和签名更新；每小时检查更新，确认后先停监控和完成写入，更新后恢复原任务。

不播放或录制音视频。只保存实际收到且成功写入的消息；不能补回断线期间或平台未下发的内容。Rust 不会消除平台 HTTP 444/429、游客下发限制或风控。

## 技术与目录

| 目录 | 职责 |
| --- | --- |
| `src/` | 现有 WebView 界面、虚拟网格、字体和前端测试 |
| `src-tauri/` | Tauri 桌面窗口、受限 IPC、单例及更新安装流程 |
| `crates/h-live-runtime/` | Rust HTTP/WSS、原生签名、异步调度、出口生命周期、配置、更新恢复及契约测试 |
| `crates/qingkan-core/` | Rust protobuf/受限解压、SQLite 归档、质量规则、在线统计、CSV/XLSX 导出 |
| `docs/` | 使用、构建、数据兼容和来源说明 |
| `scripts/`、`packaging/` | PowerShell 构建、许可收集、打包与验证 |

界面仍由 Tauri WebView 的 HTML/CSS/JavaScript 渲染。Node 是前端构建工具，不随应用打包；Rust 后端不调用 JS 签名引擎，也不启动采集子进程。

## 构建

准备 Windows 10/11 x64、PowerShell 7、Rust MSVC、Node.js 24、Visual Studio C++ 桌面构建工具及 Windows SDK。首次构建需联网获取锁定依赖。

```powershell
npm.cmd ci
. ./scripts/rust-env.ps1
cargo fetch --manifest-path src-tauri/Cargo.toml --locked --target x86_64-pc-windows-msvc
cargo fetch --manifest-path crates/Cargo.toml --locked --target x86_64-pc-windows-msvc
./scripts/test.ps1
./scripts/build.ps1 -SkipInstaller
```

目录版不需要更新签名私钥。签名安装器由持有既有密钥的维护者构建；详见 [构建与发布](docs/release/BUILD.md)、[CDN 更新](docs/release/CDN_RELEASE.md)。不要为现有更新渠道生成替代密钥。独立分发请配置自己的应用标识、数据目录和更新渠道。

## 数据与验证

继续使用 `%LOCALAPPDATA%/DouyinLiveMonitor`，保留应用标识和已有数据库。首次安装列表为空。旧代理配置按原 Windows 用户的 DPAPI 格式读取并保护；旧登录字段不解密、不发送、不对界面暴露。调试请设置独立 `DOUYIN_MONITOR_HOME`。

验证包含核心归档/导出回归、旧版合成数据库与 DPAPI 配置升级、TLS/CONNECT、停止/重启、更新恢复、合成礼物过滤，以及真实游客 WSS 有界观察。性能验证使用 **300 个配置任务、50 个模拟消息连接、51 个出口和一次集中重连**；不等于抖音真实 50 房间并发验收。完整证据范围见 [0.3.0 验证](docs/engineering/VALIDATION_0.3.0.md)。

## 来源与许可

- [eplayn/DouyinLiveMonitor](https://github.com/eplayn/DouyinLiveMonitor)：协议和基础实现参考，MIT，见 [LICENSE](LICENSE)。
- [eplayn/DouyinLiveMonitor-Remastered](https://github.com/eplayn/DouyinLiveMonitor-Remastered)：WSS 原生签名实现及测试向量参考，MIT 原文见 [上游许可](licenses/upstream/DouyinLiveMonitor-Remastered-MIT.txt)。Rust 移植没有引入其 Win32 GUI、播放器或 Node 抓流程序。
- 活动协议字段参考和许可见 [协议调研](docs/research/live-activities-protocol.md)。字体 OFL 原文和字节校验清单位于 `src/fonts/`。
- 依赖按各自许可证分发，见 `THIRD_PARTY_DESKTOP_NOTICES.txt` 和发行包 `licenses/`。

公开源码不包含用户配置、代理凭据、登录数据、私钥、聊天数据库、现场抓包或诊断文件。活动二进制测试夹具全部为合成数据；原始 0.2.4 源码仍可从 Git 历史查看。
