# H-live · 氢看直播

Windows 多主播直播消息监控应用。**0.3.6 使用 Rust 后端 + Tauri 2 界面，不附带 Python、Node、独立 V8 签名引擎或外部签名服务。** 支持监控看板、聊天回放、场次分析和数据导出。

桌面安装包和更新清单默认发布到[莉莉丝 Sites 内网下载页](https://qingkan-live.internal.lilithsites.com/)。运行 `./scripts/release.ps1` 自动递增 patch 版本并发布，失败重跑复用已准备的版本；更新说明可提前写入 `release-notes/next.md`。旧 CDN 永久停留在 0.3.6，只用于老客户端迁移。GitHub Releases 用于独立服务端。当前变化见 [0.4.0 更新日志](release-notes/0.4.0.md)，完整步骤见[发布流程](docs/release/CDN_RELEASE.md)。

Windows 安装器与 CDN 更新文件见 [Releases](https://github.com/isTrih/H-live/releases)。更新清单由维护者手动上传 CDN。

## 免责声明

本项目仅用于学习研究，严禁用于商业谋利、破坏系统、盗取个人信息等行为。 侵权或涉及相关利益请联系原作者。

本项目与抖音、字节跳动不存在官方关联。免责声明见 [DISCLAIMER.md](DISCLAIMER.md)。上游版权与许可证原文保留，不因项目改名而变更。

## 功能

- 多主播异步监控、开播优先并按在线人数倒序、未开播折叠、无限滚动、人数刷新和默认浏览器打开直播间。
- 导航集中为「监控看板」「导出数据」「设置」；实时消息收在看板，代理与诊断收在设置。累计监控时长按变化的数字独立翻页，减少动态效果时直接更新。
- 通过游客 HTTPS/WSS 接收聊天、表情、关注/分享、粉丝团、点赞、红包和福袋。**没有登录 Cookie 输入/调用接口，不新增礼物记录。** 历史礼物仍可查询与导出；进入消息不出现在消息流中。
- 按真实主播与直播场次归档；同场相同开奖时间的福袋合并；暂停后恢复同场次时，继续累计已记录时长，中间停止时间不计入。
- 重启后按已保存身份或归档中的可靠入口 ID 对应恢复历史，未开播也可查看最近场次；不按昵称猜测身份，开播状态仍由平台查询确认。
- 场次峰值在线、分钟平均在线、本地规则有效发言统计及相关比例；可选 OpenAI 兼容模型单次分析本场归档，在时间轴标记互动亮点。
- CSV/XLSX 月导出，包含发送人 ID 和名字；多人按主播分 sheet，单人文件名带主播名；支持按主播、月份或场次删除记录。
- 新安装默认直连、关闭代理池；保留旧配置中明确启用的代理偏好。设置中的代理区默认折叠，代理关闭时不自动拉取公共代理或刷新订阅，仍支持手动检测和刷新。支持代理文本、认证 URL、JSON、CSV/TSV/表格及 HTTPS URL 订阅导入。
- 调试模式默认关闭，常规连接和未开播轮询不进入运行日志队列或磁盘；真实异常继续保存，排查时可在「设置 → 诊断」开启详细日志。
- 单例、macOS 风格顶部窗口按钮、两种字体和签名更新；每小时检查更新，确认后先停监控和完成写入，更新后恢复原任务。

启用社区生态并配置 KEY 后，可播放上游提供的录播，支持暂停、音量调整、全屏及弹幕回放；没有可用录播时继续使用弹幕回放。应用不自行录制音视频。只保存实际收到且成功写入的消息；不能补回断线期间或平台未下发的内容。Rust 不会消除平台 HTTP 444/429、游客下发限制或风控。

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

新增独立 **H-live Server 0.1.1**：Rust + Salvo、Linux 单体容器，复用现有看板和归档功能，支持浏览器管理与持续后台监控。默认使用 [Docker Engine 单容器](docs/server/DOCKER_SINGLE.md)，一个应用容器承载网页、API、采集、SQLite 和导出，同容器自动初始化。SQLite 文件保存在容器外的持久化卷；外部 PostgreSQL 是后续可选后端计划，当前尚未实现。保留早期 Compose 和可选 Podman 方案。使用独立管理认证、任务恢复、代理凭据加密和导出下载；桌面版及 CDN 更新渠道保持独立。部署步骤与验收范围见 [服务端说明](docs/server/README.md)。

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

验证包含核心归档/导出回归、旧版合成数据库与 DPAPI 配置升级、TLS/CONNECT、停止/重启、更新恢复、合成礼物过滤，以及真实游客 WSS 有界观察。性能验证使用 **300 个配置任务、50 个模拟消息连接、51 个出口和一次集中重连**；不等于抖音真实 50 房间并发验收。活动轮次合并、卡片和启动优化见 [活动统计说明](docs/engineering/ACTIVITY_ROUNDS.md)；此前调度及容器发布验收见 [0.3.1 验证](docs/engineering/VALIDATION_0.3.1.md)。

## 来源与许可

- [eplayn/DouyinLiveMonitor](https://github.com/eplayn/DouyinLiveMonitor)：协议和基础实现参考，MIT，见 [LICENSE](LICENSE)。
- [eplayn/DouyinLiveMonitor-Remastered](https://github.com/eplayn/DouyinLiveMonitor-Remastered)：WSS 原生签名实现及测试向量参考，MIT 原文见 [上游许可](licenses/upstream/DouyinLiveMonitor-Remastered-MIT.txt)。Rust 移植没有引入其 Win32 GUI、播放器或 Node 抓流程序。
- 活动协议字段参考和许可见 [协议调研](docs/research/live-activities-protocol.md)。字体 OFL 原文和字节校验清单位于 `src/fonts/`。
- 依赖按各自许可证分发，见 `THIRD_PARTY_DESKTOP_NOTICES.txt` 和发行包 `licenses/`。

公开源码不包含用户配置、代理凭据、登录数据、私钥、聊天数据库、现场抓包或诊断文件。活动二进制测试夹具全部为合成数据；原始 0.2.4 源码仍可从 Git 历史查看。

测试与构建会先运行 `scripts/check_no_python.ps1`，拒绝已跟踪的 Python 源码、字节码、扩展模块、依赖清单、Python shebang 与文档中的 Python 代码块。旧版代码仅保留在 Git 历史中。

