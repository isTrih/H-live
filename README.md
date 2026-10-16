# H-live · 氢看直播

Windows 多主播直播消息监控桌面应用，采用 Tauri 2 界面，支持场次归档、互动分析和月度表格导出。

> **当前公开基线为 0.2.4，仍包含 Python 采集后台。纯 Rust 后端属于后续迁移目标，尚未交付，也没有宣称已达到 300 房间同时在线。**

## 免责声明

本项目仅用于学习研究，严禁用于商业谋利、破坏系统、盗取个人信息等行为。 侵权或涉及相关利益请联系原作者。

本项目与抖音、字节跳动不存在官方关联。请尊重平台规则、知识产权与个人信息权益。免责声明见 [DISCLAIMER.md](DISCLAIMER.md)，上游版权与许可原文分别保留，不因项目改名而变更。

## 当前功能

- 多主播监控卡片、在线优先排序、人数刷新、未开播折叠、默认浏览器打开直播间。
- 按主播和真实场次保存消息；停止后恢复同一场次，继续累计有效监控时长，暂停间隔不计入。
- 弹幕、表情、关注与分享、粉丝团、点赞、红包和福袋；同场相同开奖时间的福袋合并保存。消息是否下发取决于平台，不能保证收齐所有事件。
- 场次峰值在线、分钟平均在线、本地规则有效发言统计；重复内容按规则处理，暂不接入 AI。
- CSV/XLSX 月导出、多人按主播分 sheet、保留发送人 ID 与名称；支持按主播、月份、场次删除归档。
- 代理文本、JSON 和 URL 订阅导入；直连与代理共同作为出口，带并发限制、冷却及生命周期维护。
- Tauri 签名更新、每小时检查、确认更新后暂停任务，升级后恢复。

不播放或录制直播流。**已停用新增礼物记录、计数和实时展示；历史礼物仍可查询和导出。** 使用游客监控，无需提供登录 Cookie；当前兼容代码尚保留旧配置接口，后续 Rust 版本会移除登录入口。

## 技术与目录

| 目录 | 职责 |
| --- | --- |
| `src/` | Tauri WebView 界面、虚拟网格、字体与前端测试 |
| `src-tauri/` | Rust 桌面外壳、受限 IPC、单例、更新与窗口管理 |
| `crates/qingkan-core/` | Rust protobuf 解码、受限解压、SQLite 归档、统计与导出 |
| `crates/qingkan-native/` | 当前采集后台调用 Rust 核心的 PyO3 适配层 |
| `python/collector/` | 当前 HTTP/WSS 连接、签名、任务调度、代理和订阅 |
| `tests/`、`backend_tests/`、`native_tests/` | 使用隔离数据的离线验证 |
| `docs/`、`scripts/`、`packaging/` | 设计、构建、发布及许可收集 |

生产包无需用户另装 Python 或 Node；**当前版本仍把 Python/V8 运行库打包进安装器**。Tauri 的界面由 HTML/CSS/JavaScript 驱动，所谓“纯 Rust”迁移指后端不再依赖 Python、Node 或 JS 签名引擎，不是去掉 WebView。

## 构建与测试

目标 Windows 10/11 x64。准备 CPython 3.12 x64、Node.js 24、Rust MSVC 工具链，以及 Visual Studio C++ 桌面工具和 Windows SDK。首次构建需要联网获取锁定依赖；完整步骤见 [构建说明](docs/release/BUILD.md)。

```powershell
python -m venv .venv
.\.venv\Scripts\python.exe -m pip install -r requirements-dev.txt
npm.cmd ci
. .\scripts\rust-env.ps1
cargo fetch --manifest-path src-tauri/Cargo.toml --locked --target x86_64-pc-windows-msvc
cargo fetch --manifest-path crates/Cargo.toml --locked --target x86_64-pc-windows-msvc
.\scripts\build.ps1 -SkipInstaller
.\scripts\test.ps1
```

首次检出生成目录版不需要更新私钥。正式更新包只由持有既有签名密钥的维护者生成，仓库不包含私钥。独立构建者应使用自己的应用标识、数据目录及更新渠道，避免与已安装正式版冲突。开发调试请用 `DOUYIN_MONITOR_HOME` 指向独立测试目录。

## 纯 Rust 迁移

已对两个上游的固定提交完成源码比较。Remastered 的 WSS 原生签名算法可移植为 Rust，因此消息监听不需要引入其 Node 抓流路径。完整方案、出处和验收边界见 [迁移调研](docs/research/remastered-rust-migration-2026-10-16.md) 与 [迁移计划](docs/engineering/RUST_MIGRATION.md)。

计划保留现有 UI、数据库及导出契约，使用 Tokio 异步任务替换 Python HTTP/WSS、心跳、调度与代理层。使用真实参数生成签名，保留 TLS 校验、响应/帧/解压上限、有界队列、取消与停止确认。Rust 无法保证消除平台 HTTP 444 或游客消息限制；性能提升需要相同负载下实测后再公布。

## 来源与许可

- [eplayn/DouyinLiveMonitor](https://github.com/eplayn/DouyinLiveMonitor)：基础实现与协议路线，MIT，版权声明见 [LICENSE](LICENSE)。
- [eplayn/DouyinLiveMonitor-Remastered](https://github.com/eplayn/DouyinLiveMonitor-Remastered)：原生签名迁移参考，MIT 原文见 [上游许可](licenses/upstream/DouyinLiveMonitor-Remastered-MIT.txt)。尚未将其 C 后端作为运行组件。
- 协议字段参考及许可见 [活动协议研究](docs/research/live-activities-protocol.md)。字体原文、来源和校验值位于 `src/fonts/`。
- 其他依赖按各自许可证分发，见 `THIRD_PARTY_NOTICES.txt`、`THIRD_PARTY_DESKTOP_NOTICES.txt`；构建时收集完整依赖许可。

公开仓库不包含用户配置、代理凭据、登录数据、私钥、聊天数据库或现场诊断记录。随仓库提供的活动二进制测试夹具为合成数据，不是直播抓包。新安装用户不预置主播。
