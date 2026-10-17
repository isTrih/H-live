# H-live 纯 Rust / Salvo 常驻服务可行性（2026-10-17）

## 结论与证据边界

**可以在现有 Rust 采集与归档核心上增加独立 Salvo 服务入口，用于无人值守常驻运行。** 这是代码结构与框架能力支持的可行性判断，尚不是已交付的服务器版，也不是已通过真实 50 房间、连续 24 小时验收的结论。部署系统尚待确认；Linux、Windows 服务、Docker 的打包与凭据方案不能混为一套。

本次只读本地源码与官方资料，新增本调研文档；没有启动 HTTP 服务、连接直播平台或开展监控。

## 已有实现事实

| 事实 | 本地证据与影响 |
| --- | --- |
| 采集核心已经独立于桌面壳 | [`h-live-runtime/Cargo.toml`](../../crates/h-live-runtime/Cargo.toml) 不依赖 Tauri；归档由 `qingkan-core` 提供。可新增服务 binary 复用这些 crate，避免复制协议实现。 |
| `Runtime` 是同步外观，内部自有 Tokio runtime | [`runtime.rs`](../../crates/h-live-runtime/src/runtime.rs) 的 `Runtime::open` 建立 4 个异步 worker、最多 16 个 blocking thread；[`requests.rs`](../../crates/h-live-runtime/src/requests.rs) 的 `request` 部分分支及 `shutdown` 调用 `self.rt.block_on`。不能把所有调用直接放进 Salvo async handler。 |
| 停机需要等待采集、后台作业与归档 | `Runtime::shutdown` 会进入 quiescing、取消任务、等待后台任务、保存 metadata，再等待归档关闭；只有完成后返回 `closed=true` 和 `all_workers_stopped=true`。各后台 join 有 30 秒等待、归档 close 有 15 秒等待；这些不是整个停机过程的统一硬上限。 |
| 启动恢复尚不是服务器任务恢复 | `Runtime::open` 调用的是 `resume_update`；不能把桌面更新恢复意图当作通用的开机 / 崩溃重启监控恢复策略。服务器仍需持久化目标运行状态。 |
| 代理配置加密当前依赖 Windows DPAPI | [`storage.rs`](../../crates/h-live-runtime/src/storage.rs) 使用 `CryptProtectData` / `CryptUnprotectData`；非 Windows `crypt` 明确返回错误。导入代理、代理订阅列表只要非空，保存时都会走此加密路径，包括没有用户名密码的条目。Linux 不能直接迁移 Windows 加密配置，也不能声称已支持这些列表的持久化。 |
| Linux 进程资源指标待补齐 | [`process.rs`](../../crates/h-live-runtime/src/process.rs) 的非 Windows 实现返回 RSS、峰值 RSS、CPU 为 `null`；这表示未测量，不表示零资源消耗。 |
| 游客采集范围已有明确约束 | [`README`](../../README.md) 与 [`net.rs`](../../crates/h-live-runtime/src/net.rs) 明确无登录 Cookie 输入接口、不新增礼物记录；游客握手自行获取的 `ttwid` 与用户登录 Cookie 是不同概念。服务器化应保留此边界。 |
| 当前性能证据不是长期真实平台验收 | README 与 [`0.3.0 验证`](../engineering/VALIDATION_0.3.0.md) 区分 300 个配置任务、50 个模拟消息连接和真实游客有界观察。不可由此推导真实 50 房间稳定运行 24 小时。 |

## 官方框架能力

截至调研日，发布包索引显示 **Salvo 1.0.1，发布于 2026-10-04**；固定版本源码声明 **Rust MSRV 1.94、edition 2024**。实施时应锁定依赖并核对当前工具链；不要采用搜索缓存中的 0.95 / 0.96 版本号作为最新结论。[发布包版本](https://docs.rs/crate/salvo/latest)、[v1.0.1 Cargo.toml](https://raw.githubusercontent.com/salvo-rs/salvo/v1.0.1/Cargo.toml)

| 所需能力 | 官方事实 | 本项目拟采用方式（未实现） |
| --- | --- | --- |
| 共享核心实例 | `affix-state` 可把可克隆状态放入 `Depot`，官方示例使用 `Arc<State>`。[Affix State](https://salvo.rs/guide/features/affix-state) | 一个受控的 `Arc<Runtime>` / 服务状态，单实例持有数据目录；最终所有权与销毁放在合适的同步上下文。 |
| 同步与异步桥接 | Tokio `Runtime::block_on` 在异步执行上下文中调用会 panic；`spawn_blocking` 用于可结束的阻塞操作，已启动后不能靠 abort 强行取消。[Runtime](https://docs.rs/tokio/latest/tokio/runtime/struct.Runtime.html#method.block_on)、[spawn_blocking](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html) | Salvo handler 经**有界 `spawn_blocking`** 调用同步 `Runtime::request`；外层 semaphore / 有界命令队列限制排队。HTTP 超时不能冒充后台操作已取消。初始化、shutdown 和最终 drop 也须安排线程与所有权，避免嵌套 runtime。长期采集仍在核心自有 runtime 内运行。 |
| 管理接口鉴权 | Salvo 提供 JWT 中间件与不同 token finder；也允许普通 handler 作为 hoop。[JWT Authentication](https://salvo.rs/guide/features/jwt-auth) | 首版可采用管理 Bearer token 的自定义 hoop；查询、导出、配置写入、SSE 均鉴权。只暴露白名单方法；不能将本地 RPC 的任意方法字符串直接开放到网络。此 token 是本服务的访问凭据，与平台登录 Cookie 无关。 |
| 请求与并发边界 | `size-limiter` 示例提供 `max_size`；`concurrency-limiter` 提供 `max_concurrency`，超出返回 503。[Size Limiter](https://salvo.rs/guide/features/size-limiter)、[Concurrency Limiter](https://salvo.rs/guide/features/concurrency-limiter) | JSON、代理导入、导出分别设大小和并发边界。实现时测试无 Content-Length、chunked body、慢客户端；核心已有参数校验不能取代 HTTP 读取边界。 |
| 事件推送 | Salvo 提供 `SseEvent`、`sse::stream`、`SseKeepAlive`。[SSE](https://salvo.rs/guide/features/sse) | 一个后台轮询 / 事件转发器向有界广播队列发布；每客户端限额、慢客户端断开、缺口提示及快照恢复。官方聊天示例用无界队列，不能直接作为长期运行资源策略。 |
| 优雅退出 | `Server::handle().stop_graceful(Some(Duration))` 停止接新连接并等待在途请求；超时关闭剩余连接。[Graceful Shutdown](https://salvo.rs/guide/topics/graceful-shutdown) | 收到 SIGTERM / Ctrl+C 后停止新写操作、终止 SSE，收敛在途调用，经 blocking 桥执行核心 shutdown，核实归档确认，再退出并在同步上下文销毁核心。Salvo 的 HTTP drain 本身不会替我们关闭采集和 SQLite writer。 |

## 常驻部署建议（方案，尚未配置）

- **Linux 原生进程**：可用 systemd，专用运行用户、`StateDirectory=h-live` 指向持久化状态目录、`Restart=on-failure` / `RestartSec` 控制失败重启，`TimeoutStopSec` 留出实测归档收尾时间。官方规定 `StateDirectory` 在停止后保留；停止超时可导致强制终止。因此停机预算要覆盖 HTTP drain、核心 join 与归档 flush，不能机械套一个很短的默认值。[systemd.exec 官方源码](https://github.com/systemd/systemd/blob/main/man/systemd.exec.xml)、[systemd.service 官方源码](https://github.com/systemd/systemd/blob/main/man/systemd.service.xml)
- **Docker**：数据目录整体挂到持久 volume / bind mount，涵盖 SQLite 与相关侧文件、配置、日志和导出任务状态；配置 `restart: unless-stopped` 及足够的停止宽限时间。Docker stop 默认先发 SIGTERM，宽限期后 SIGKILL；Rust binary 应直接接收停止信号。不要同时叠加 Docker 重启策略和宿主进程管理器来争相管理同一个容器。[Volumes](https://docs.docker.com/engine/storage/volumes/)、[Restart policy](https://docs.docker.com/engine/containers/start-containers-automatically/)、[Container stop](https://docs.docker.com/reference/cli/docker/container/stop/)
- **Linux 凭据前置工作**：抽出 secret-provider 接口，Windows 保留 DPAPI，Linux 从受权限保护的密钥来源提供加密能力；迁移采用显式导入，不能把 DPAPI 密文当作跨机通用格式，也不能静默退回明文。尚未选定实现。若先做无代理核心原型，应明确 Linux 的代理导入 / 订阅持久化未就绪。
- **可观测与恢复**：建议提供存活、就绪、房间连接状态、归档队列、最后写入时间与磁盘余量；重启后按明确保存的监控意图恢复。HTTP 200 只能说明接口存活，不能证明平台消息正在到达。相关指标、恢复策略与报警均需实现和验收。

## 能交付到什么程度

第一阶段可以是单机单实例的 Rust binary：房间管理、启动 / 停止、状态查询、受控导出、鉴权、SSE、数据目录参数、信号退出及部署样例；桌面版继续共用采集核心。Web 管理页面可后续接同一 API，“纯 Rust 服务”无需依赖桌面 WebView，但浏览器页面资产与服务端语言是两回事。

服务器版完成后还需要离线故障恢复与背压测试、真实环境逐级并发、至少 24 小时持续观察，并记录平台 429/444、游客下发范围、代理质量、重连率、消息入库延迟、RSS / 线程趋势、磁盘增长及重启后的数据一致性。**在这些证据出现前，只能承诺常驻部署能力与明确资源边界，不能承诺真实 50 房间 / 24 小时消息完整性。**
