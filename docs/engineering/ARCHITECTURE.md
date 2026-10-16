# 0.2.0 架构与迁移边界

0.2.0 将高频消息解码、SQLite 写入和整月导出迁入 Rust。网络采集仍由现有 Python 适配器负责，原生扩展在采集进程内加载，不新增逐条消息跨进程往返。应用继续使用现有数据目录、配置加密方式、数据库结构和更新公钥。

本次采用 mattpocock `codebase-design` 的深模块设计方法：接口描述调用者需要的能力，复杂实现与生命周期留在模块内部；测试通过实际接口验证结果，避免测试私有线程、SQL 写法或辅助函数名。

## 运行路径

```text
WebView：src/main.js + features/monitor/model.js
  │ 低频命令 / 有界状态与消息批次
Tauri：src-tauri/src/lib.rs
  ├─ desktop：窗口、系统对话框、受限直播间 URL
  ├─ backend：命令排序、关闭及更新门禁
  └─ engine：私有 stdio RPC、超时、Windows Job 进程保护
      │ 一个采集进程，固定入口
Python：python/collector/backend_service.py
  ├─ monitor：任务状态、重连、真实主播与场次、日志
  ├─ proxy_pool / proxy_import：代理租约、检测、凭据
  ├─ liveMan：HTTP/WSS、签名、心跳与事件回调
  │    └─ qingkan_native.decode_frame → qingkan-core::protocol
  ├─ archive_store → qingkan_native.ArchiveStore → qingkan-core::archive
  └─ message_export → qingkan_native.export_month → qingkan-core::export
```

## 模块接口

| 模块 | 对外接口 | 隐藏的实现 |
| --- | --- | --- |
| `qingkan-core::protocol` | 二进制帧 → ACK、顺序事件；心跳帧 | gzip、protobuf 字段、时间和 ID 归一化、帧大小限制 |
| `qingkan-core::archive` | 打开、写消息并确认提交、读快照、关闭 | 单写线程、有界队列、批量事务、SQLite 连接、去重 |
| `qingkan-core::export` | 月份/主播/类型/目标路径/取消令牌 → 结果报告 | 单快照、CSV 转义、XLSX 工作表命名和切分、临时文件、原子替换 |
| `qingkan-native` | PyO3 扩展及 Python 异常 | GIL 释放、JSON 边界、Rust/Python 类型转换 |
| `engine` | 启动、串行请求、正常退出、更新前退出 | 子进程句柄、Windows Job、管道、响应编号与超时 |
| `features/monitor/model.js` | 排序筛选、状态、时长、人数展示值 | 状态判断规则；不依赖 DOM 或 Tauri |

`qingkan-core` 不依赖 Python、WebView 或 Tauri。`qingkan-native` 是适配器，不复制领域逻辑。两个 Cargo 工作区分别服务桌面壳与消息原生模块，以保留现有 Tauri 构建产物位置。

## 并发与数据约束

- 解压、解码、数据库等待及文件导出在 PyO3 中释放 GIL。解码结果以一帧一个 JSON 数组交付，兼容现有 Python 回调；这仍有序列化成本，性能报告计入该成本。
- 每个消息库一个写线程，默认队列 4096、批次上限 256、聚合等待 4 ms。写入返回成功意味着事务已经提交；队列满时等待，不丢弃内容。错误传播到任务管理器。
- 月导出使用单个只读快照，流式处理，临时文件与目标在同一目录；完整写入并同步后才替换目标。取消或失败保留原文件。界面导出任务持有原生原子取消令牌。
- 前端只接收有界缓存中的展示消息。数据库归档与界面刷新独立，实时显示不能代替持久化成功确认。
- Tauri 后台命令通过 `spawn_blocking` 执行。引擎锁串行保护一对请求/响应管道；它不是数据库或前端渲染锁。退出、更新必须等待后台完成归档并退出，才释放进程保护。
- 数字 ID 始终按字符串传递。主播 ID、直播入口和实际场次分别保存。当前观众人数仅为展示状态，不进入内容归档。缺失发送时间、发送人 ID 或名字保持缺失。
- 结束通知会停止该场连接；即使消费回调抛错也会执行停止，避免已结束的连接继续存活。

## 兼容与发布

数据库仍为 `user_version=1`，0.1.1 与 0.2.0 读写同一结构，无破坏性迁移。Cookie 和代理凭据继续使用当前 Windows 用户的 DPAPI。程序启动时核对采集服务与原生模块版本，禁止静默退回旧 Python 实现；安装版自检再次核对 `native_version` 与 `processing=rust`。

正式构建先生成 `qingkan_native.pyd`，再冻结采集进程，最后构建 Tauri 与签名更新包。用户无需安装 Python 或 Rust。更新仍使用 `https://assets.checkpoint321.com/lilith/qingkan-live/latest.json`，由维护者手动上传公共发布文件；私钥不进入安装包或 CDN 压缩包。

## 目录职责

| 路径 | 用途 |
| --- | --- |
| `src/` | 当前 WebView 界面、展示模型和前端测试 |
| `src-tauri/` | 桌面壳、权限、更新和窗口策略 |
| `crates/` | 独立原生核心和 PyO3 适配器 |
| `python/collector/` | 当前网络采集与配置适配层 |
| `tests/`、`backend_tests/`、`native_tests/` | 采集、后台 RPC 与旧新协议差分测试 |
| `packaging/`、`scripts/` | 打包配置、发布检查、构建、诊断与基准入口 |
| `docs/`、`release-notes/` | 使用、构建、工程决策、研究证据与逐版本更新日志 |
| `legacy/tk/`、`legacy/inno/` | 上代界面和安装配置，排除正式构建 |
| `legacy/python-reference/` | 冻结的 0.1.1 差分/基准参考，排除正式构建 |
| `build/`、`dist/`、`.artifacts/` | 忽略提交的中间产物、发行文件和诊断证据 |

## 继续保留 Python 的原因

HTTP/WSS、签名 V8 适配、代理池与任务管理依赖已经验证的协议和退出行为。本次没有以未经验证的全新网络栈替换它们。它们多数时间等待网络，当前性能迁移优先处理本机 CPU 与磁盘热点。后续若继续迁移，需沿相同接口保留代理出口、认证、TLS、ACK、心跳、重连、去重及停止语义，并补充真实连接验证。

离线基准只能验证本机处理能力；不能据此保证抖音或公共代理支持 300 个稳定的实时连接。测试与实测结果见 `PERFORMANCE_0.2.0.md` 及发布验收记录。
