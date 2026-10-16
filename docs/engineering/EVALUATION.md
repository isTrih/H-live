# 氢看直播 0.2.1 可用性评估

评估日期：2026-10-15，Asia/Shanghai。对象为基于 [eplayn/DouyinLiveMonitor](https://github.com/eplayn/DouyinLiveMonitor) 提交 `12559e18440f939712c14793f4ab2f77a3f9d57c` 改造项目。当前版本为 **0.2.1**。各节标明其验证版本，历史 0.2.0 及更早的安装报告不能代替本版最终产物验收。

## 0.2.1 功能与验收

本版新增红包/福袋活动解析、共享出口与直连均衡、HTTPS URL 订阅、认证代理覆盖更新、按主播/月份/场次清理内容归档、手动「非 PC 支持」标记和首次空主播列表。沿用同一用户数据目录、安装身份与更新公钥；内容库从 v1 事务迁移至 v2，保留原消息并允许活动类型，旧版应用不能打开升级后的库。用户已有主播不删除，不将 HTTP 444 或隐藏网页模板当作自动兼容性结论。归档边界见 [schema v2 说明](ARCHIVE_SCHEMA_V2.md)。

活动功能加入前的源码功能回归 **398 项通过**：Python 326（277 主测试 + 34 后台 + 15 原生适配）、Rust 48、前端 24。详情与限制见 [工作区验收](VALIDATION-2026-10-15.md)、[共享出口](PROXY_GROUPS.md)及[订阅读取边界](SUBSCRIPTION_SECURITY.md)。改版本号前的隔离冻结程序通过 11 项检查、17 次 RPC，包含真实空冷启动、手动标记、异步空库删除和正常退出；报告 `build/diagnostics/proxy-groups-final-frozen/report.json` 记录输入指纹，不能将其 0.2.0 诊断版号当作正式 0.2.1 成品版本。

新增活动支持 8 种协议消息，合成消息已验证实时筛选、空发送人展示及默认导出选项。最终前端 34 项通过，证据 `build/0.2.1-frontend-tests.log`、`build/ui-validation-0.2.1.json`；看板开播优先并按在线人数倒序、未开播默认折叠，去掉排序说明及底部状态栏，内部轻圆角。指定房间 `100000007000` 的直连和 Orem 查询仅 HTTP 200，没有有效房间元数据，未取得 WSS 活动实包；这不属于真实活动接收成功。新增原生及 Python 回归以本轮最终验收汇总为准，旧 398 项不包含这些后加功能。

最终源码回归 **446 项通过**：Python **342**（290 主测试 + 34 后台 + 18 原生适配）、Rust **70**（65 核心 + 5 桌面）、前端 **34**。日志和哈希汇总在 `build/release-validation-0.2.1.json`；原生模块 SHA-256 为 `70f867c2b192c131d00b8c3fac41d6c5b2241061ea36063f37d2026edb0f9d26`。本版活动、schema v2 和同场累计时长包含在此终态回归内，前述 398 项是先前阶段记录，不能与 446 项相加。

冻结后台经真实离线 JSON RPC 默认导出 4 条合成内容、3 张主播表（含空月表），其中包含 1 条活动；另按 `activity` 单独导出 CSV/XLSX 各 1 条，校验中文类型、详细摘要、空发送人和正确主播表。新时长遥测字段存在，后台正常退出；实际打包的原生模块哈希与上述源码回归一致。报告为 `build/frozen-export-check-0.2.1/report.json`。桌面壳→后台自检的 init、poll、代理状态、更新安全门禁、shutdown 和进程退出通过，见 `build/smoke-report-0.2.1.json`。

本机 Windows 10 x64 完成 **0.2.0 → 0.2.1 独立标识 NSIS 隔离升级**。旧版和新版安装后自检均通过，1212 个资源与测试载荷逐文件一致；历史合成 SQLite 库自动迁移到 v2，全部原消息、主播、场次、自增序号、索引及外键保持一致，新增独立时长表。旧 Inno 目录保护返回 2；测试卸载清理自己的程序和注册项、保留独立数据。证据为 `build/nsis-install-check-0.2.1/report.json`、同目录 `database-compatibility.json` 及 `build/0.2.1-nsis-isolated-upgrade.log`。此后只同步最终验收文档生成正式签名包，最终文档和载荷哈希另行核对；这不等于替用户实际安装，也不是线上 CDN 更新演练。

正式安装器为 `dist/installer/QingkanLive-0.2.1-windows-x64-setup.exe`，CDN 上传 ZIP 为 `dist/QingkanLive-0.2.1-cdn.zip`，版本、签名及 SHA-256 见 `build/releases/0.2.1/release-package-report.json`。旧 0.2.0 安装器和 ZIP 保留；本地 `latest.json` 更新至 0.2.1，未上传 CDN，也未改真实用户安装、预览或数据库。schema v2 不能被旧版应用打开，回退需要升级前完整备份。

**本地多任务管理、归档、导出和桌面集成已通过所列验证；新增来源已找到实际收取互动内容的公网代理。** ProxyHub 两枚 HTTP 代理和 ProxyClean 一枚 SOCKS5 代理完成短时真实消息验证，后者使用独立诊断适配器；这些样本未收到 chat 类型。此前 SCDN 的失败样本结论仍成立，但不代表所有公共来源均不可用。不能把本机 300 个模拟任务的容量结果当成 300 路真实直播连接可用。来源对比和证据见 [PROXY_SOURCES_REVIEW.md](PROXY_SOURCES_REVIEW.md)。

## 0.2.0 迁移验收

本版将 SQLite 归档、月导出与 protobuf 解码迁入 PyO3 原生 Rust 扩展，网络采集与调度继续留在 Python。历史安装器、签名密钥、配置目录和数据库格式保持不变。架构见 [ARCHITECTURE.md](ARCHITECTURE.md)，性能方法和数据见 [PERFORMANCE_0.2.0.md](PERFORMANCE_0.2.0.md)。

源码回归 **Python 219 项**通过（主测试 174 + 后台 30 + 协议差分 15），前端 **20 项**通过，Rust **40 项**通过（原生核心 35 + 桌面壳 5）。日志为 `build/tests-python-0.2.0.log`、`build/tests-frontend-0.2.0.log`、`build/tests-rust-0.2.0.log`。差分包括 500 条混合消息、最大 64 位 ID、昵称、时间边界、重复嵌套字段、gzip 拼接及填充、ACK/心跳和结束通知。原先只测试旧 Python 私有实现的部分测试已由跨实际模块接口的 Rust 合同覆盖，不用测试数量下降表示功能删减。

同一 SQLite 结构的新旧双向读写、消息去重、失败事务回滚、排空关闭、快照一致性、导出取消及原文件保留通过。本机 300 个模拟生产者、6 万条消息的 CSV/XLSX 值哈希相等；独立 openpyxl 完整读取两版各 300 工作表、6 万行，确认七列、昵称、长 ID 文本和无公式。证据为 `build/benchmark-0.2.0/archive-export.json` 与 `archive-export-openpyxl.json`。Rust 版本在本轮固定数据 CSV/XLSX 导出约快 3.06/3.34 倍；归档阶段内存略增，两版均未达到持续 6000 条/秒。

最终冻结后台经真实 JSON-RPC 确认 `native_version=0.2.0`、`processing=rust`；导出 3 条合成消息、3 张主播表（含空表）及 CSV，字段、正常退出与独立 openpyxl 读取通过。证据为 `build/frozen-export-check-0.2.0/report.json`、`openpyxl-validation.json`。认证代理导入、DPAPI 加密与跨后台重启保留也以合成数据验证，不访问用户账号或真实代理。

浏览器开发预览核对 300 个模拟主播、开播优先宫格、月度归档七列、默认抖音美好体、设置中 0.2.0 版本及完整更新日志。该检查是前端渲染验证；冻结原生进程、安装器及升级由独立发布验收记录覆盖，不将浏览器预览当作真实直播或原生窗口测试。

## 当前实现

上游已有消息解析与 Tk 界面，主要按单个监控实例工作，并包含流地址提取和 PotPlayer 播放调用，没有独立的视频录制实现。当前版本使用 Tauri / WebView2 桌面界面和 Python stdio 后台和 Rust 原生热路径；已移除播放、流地址提取，不录制视频或音频。

每个房间独立连接、取消、重试和 Cookie 配置。同一出口允许多个任务共享，默认允许直连参加负载均衡；同一任务的元数据、访客凭据、页面和 WebSocket 使用同一租约。严格校验 HTTPS / WSS 证书，禁用环境代理覆盖；关闭直连参与后，缺少可分配代理时等待。关闭代理也可手动检测备用池，不改变直连任务的出口，启用时复用仍有效的候选；切换网络模式需先全部停止。

新增代理导入入口支持 JSON、文本、表格及代理 URL，支持 HTTP CONNECT / SOCKS5 用户名密码认证。SOCKS5 使用远端 DNS，HTTP 与 WS 共享同一认证租约。导入凭据整体由当前 Windows 用户 DPAPI 加密保存，预览和状态只返回地址、协议、认证是否配置。先选择「已导入代理」并检测，再启用；导入本身不启动代理或监控。ProxyHub、ProxyClean 尚未成为自动获取来源。用户提供的 33 枚认证 SOCKS5 本次未通过认证/连通初筛，不能据此声称新增私密代理的真实 WSS 路径已经成功。

公共来源手动检测以 50 个新候选为一批，跳过当前池已有及本次应用运行中已失败的地址；失败记录跨模式切换共享，不跨应用重启保存。批次进行中不叠加新批次，来源不足时有界收尾，50 不表示承诺获得 50 个健康代理。已导入来源每批最多检测 50 个，少于 50 个时以实际数量为准。下述真实 500 候选探测采用当时每次请求 20 个的独立脚本，不能当作新手动批次流程的真实网络验收。

内容保存到 SQLite，运行日志单独按日保存。稳定 `anchor_id` 聚合同一主播，平台实际 `room_id` 表示场次；同场重连不新建场次，有服务端消息 ID 时按主播、场次、消息 ID 去重。月导出为 XLSX / CSV，固定七列：主播、直播间场次、发消息人ID、发消息人名字、消息类型、消息内容、发消息时间。发送人名字取已归档昵称，缺失留空；XLSX 按主播分工作表，CSV 保持平表。未知身份或时间不伪造，发送时间缺失按接收月份归入但发送列留空。旧 JSONL 保留原样，不自动混入新导出。

看板显示主播、直播状态、该真实场次的已累计监控时长与当前在线人数。当前人数仅保留在内存供显示，不写内容库、运行日志或导出，不增加消息计数，不使用累计观看数代替。同场次恢复后续接已保存累计值，中断和停止间隔不计入，新场次另计；多个入口的重叠监控区间不重复累计。检查点独立于聊天内容，强退仅恢复最近保存值，不补造停机区间。真实 `broadcast_started_at` 与归档 `started_at` 仍只接受服务端明确时间，不被累计计时覆盖。

## 0.1.1 增量验证

本次范围为七列导出、按主播拆分 XLSX 工作表、默认文件名及版本更新。全量 Python **215 项通过（185 主测试 + 30 后台测试）**，日志为 `build/release-0.1.1-python-tests.log` 和 `build/release-0.1.1-backend-tests.log`；其中导出/归档定向 40 项（24 + 16）也独立通过。JavaScript **16 项通过**（6 文件命名 + 8 更新流程 + 2 虚拟宫格），语法检查和 Vite 构建通过。浏览器实际选择单/多主播和 XLSX/CSV，核对默认文件名、七列字段、分表/平表提示及传给保存桥接层的名称，证据 `.artifacts/release-0.1.1-ui-validation.md`。预览模拟数据不计作真实导出文件验收。独立 openpyxl 3.1.5 读取合成工作簿，核对 3 张表分别 2/1/0 行、七列、昵称/空昵称、长 ID 与前导零、文本公式保护及筛选范围，报告 `.artifacts/release-0.1.1-openpyxl-validation.json`；这属于文件结构与值的独立检查，未冒充 Excel 桌面视觉验收。

新冻结后台通过真实 JSON-RPC 导出：XLSX 3 条消息分为 3 个主播工作表（含仅表头的空月份表），CSV 3 条消息合为七列平表；昵称、空昵称、长 ID、公式保护和进程正常退出通过。报告为 `build/frozen-export-check-0.1.1/report.json`，独立读取冻结程序产物见 `.artifacts/release-0.1.1-frozen-openpyxl.json`，完整导出记录见 `.artifacts/release-0.1.1-export-validation.md`。测试使用合成数据，零直播网络请求。

0.1.0→0.1.1 工作区隔离 NSIS 升级已通过，两个版本各自的 Rust→冻结后台自检通过；升级后 1250 个资源逐文件匹配，卸载保留独立数据。本轮构建重跑 Rust URL policy 2 项通过，未改动的更新策略沿用前版验证基线。最终安装包版本、图标、签名、篡改拒绝、文档一致性和 SHA-256 记录于 `build/branding-report.json`、`.artifacts/release-signature-verification.json` 与 `build/release-package-report.json`。下列 0.1.0 记录为继承功能的历史基线，不冒充本次重跑。

## 0.1.0 自动化与桌面验证基线

| 范围 | 结果 | 证据或覆盖 |
| --- | --- | --- |
| Python 主测试（含认证导入与房间状态确认） | **176 项通过** | `.artifacts/import-and-room-state-validation.md`；引擎、认证代理导入与路由、监控状态和计时、存储、导出、代理池、竞态和旧界面兼容测试。 |
| Python 后台测试 | **30 项通过** | `.artifacts/update-readiness-validation.md`；JSON RPC、游标、后台操作、代理检测/复用/清理、配置保密、导出取消与退出，以及更新前操作/导出/归档就绪检查。 |
| JavaScript | **10 项通过** | 虚拟宫格几何检查 2 项；更新流程检查 8 项，覆盖状态、后台繁忙与错误处理。 |
| Rust | **5 项通过** | 直播间 URL 限制和更新就绪/退出策略检查。 |
| 冻结后台导出与字段 | **通过** | `build/frozen-export-check/report.json`；XLSX / CSV、代理模式、刷新目标 50，以及 `broadcast_started_at`、`viewer_count`、`duration_started_at`、`duration_source` 字段；未知数值保留 null。 |
| 打包资源 | **通过** | `build/runtime-assets-check.json`；V8、ICU、sign.js 和 121 个 TLS 根证书。 |
| Rust 与冻结后台通信 | **通过** | `build/smoke-report-v2.json`；init/poll/proxy_status/error/shutdown 及子进程退出。 |
| 桌面生命周期 | **通过** | `build/window-lifecycle/report.json`；WebView 和后台启动、单实例与恢复聚焦、唯一后台、正常退出及 Job 清理。 |
| Windows DPI | **通过本机核验** | 同生命周期报告：PerMonitorV2，当前测试显示器 DPI 96。 |

0.1.0 的 Python 验证总计 **206 项通过（176 主测试 + 30 后台测试）**，JavaScript 10 项、Rust 5 项通过。176 项主测试记录于 `.artifacts/import-and-room-state-validation.md`；后台增加更新就绪检查后完整重跑 30 项，记录于 `.artifacts/update-readiness-validation.md`。这些记录是执行结果汇总，并非原始逐项日志。旧 `.artifacts/tests-final.log` 与 `backend-tests-final.log` 对应此前的 169 项回归，不能冒用为本次结果。当前覆盖包括特殊字符凭据、HTTP/WS 一致出口、SOCKS 远端 DNS、TLS 校验、安全错误，以及房间业务拒绝时保留租约、短时确认而不伪造下播。DPAPI 测试在正常 Windows 用户环境执行；受限沙箱无法访问用户密钥，不据此判定生产功能失败。模拟测试证明所覆盖的代码路径，不能替代平台真实协议与网络验证。

已接入 Tauri 签名更新、设置中的手动检查与版本日志。下载前和安装前检查后台是否空闲，安装前阻止新的业务 RPC，并确认归档收尾和后台正常退出；签名不通过时阻止安装。维护者需手动上传 CDN 更新包和清单，线上检查、下载及升级链路仍待上传后验证，不能将离线测试当成线上更新验收。

界面已检查顶部导航、状态配色、搜索筛选、连续虚拟滚动、批量选择、居中详情、消息与日志分离、月导出及默认浏览器打开。此前独立视觉审阅为 7/10，随后已修正操作提示、卡片信息过多和固定区域等问题，新版静态复核为 8/10，见 `.artifacts/ui/visual-review.md`。两款本地字体的默认选择、切换与持久化已通过浏览器检查；原生开发窗口也已启动，见 `build/dev-preview/launch-report.json`。最新截图位于 `.artifacts/ui/`，静态审阅和开发预览中的模拟主播不是生产连接成功的证据。

## 当前真实网络证据

### 新增来源与认证代理复核

2026-10-14 22:54 后，ProxyHub 中国 HTTP 首屏 20 个候选中，5 个完整抖音首页通过、4 个生产房间接口通过；其中两枚固定代理各完成 45 秒 WSS 接收，分别归档 **5 条（3 点赞、2 进入）**和 **4 条（2 点赞、2 进入）**。随后 `119.188.131.55:17981` 再测 90 秒，归档 **17 条（10 点赞、6 进入、1 社交，0 chat）**。HTTP 请求与实际 WS socket 对端证明同代理，证书验证保持开启、无用户登录 Cookie，计数一致且停止后无新增。证据：`.artifacts/proxyhub-live-119.json`、`proxyhub-live-111.json`、`proxyhub-live-chat-confirm.json`。

ProxyClean 的 30 个 SOCKS5 样本中找到一个首页通过地址，经独立本机协议适配器转向固定公网 SOCKS5 上游，45 秒归档 **7 条（3 点赞、4 进入，0 chat）**；该轮有远端 DNS、上游 peer、SOCKS 成功回复及密文字节证据，未降级 TLS 或回退直连。它证明当时该公网节点能传递消息，不等同于后来新增的原生 SOCKS5 认证实现已完成真实账号验收。haipproxy 为自建框架，未发现官方托管 endpoint，未部署或测代理。完整比较见 [代理来源评估](PROXY_SOURCES_REVIEW.md)。

另对用户提供的 **33 枚认证 SOCKS5**进行有界检测：30 枚返回 SOCKS 用户名密码认证拒绝，3 枚超时；首页、metadata、WSS 成功数均为 0。仅内存核对确认字段与 RFC1929 报文一致，独立原生 curl 单节点也返回 SOCKS5 认证拒绝，失败发生于访问抖音之前。可测代理 TCP 载荷合计 **900 字节**；不含 TCP/IP 开销、重传、未读内核缓冲及独立 curl 对照的握手流量，不能视作供应商计费值。未将凭据另存为明文或写入报告。证据：`.artifacts/private-proxy-batch-report.json`、`private-proxy-auth-consistency.json`。

### 2026-10-14 22:00 后的代理专项复核

使用固定 HTTPS 来源另测 300 个不同候选，3 个通过首页检查，但同代理生产房间接口均失败。进一步严格 TLS / Windows Schannel / 原生 SSL 对照复现隧道后的 TLS EOF、证书验证失败或连接超时；没有证据表明应统一把代理 URL 改为 `https://`。两个候选曾成功读取完整首页，但 enter 接口延长到 20 秒仍超时。新 Session 对照在首页就返回 502，不能据此确定连接复用是否相关。

随后排除前述地址，再测 **400 个新候选**，首页超时放宽为 `(5,10)`：350 个不限国家样本中仅 1 个首页通过，50 个 CN 样本无通过；该首页通过地址的生产房间查询也失败，稍后新 Session 对照在首页就证书验证失败。两轮共 700 个不同的 SCDN 候选，没有该来源真实消息成功证据；不同超时条件的结果不合并推算来源整体可用率，也不泛化到其他公共来源。

本轮同时完成 **v2 当前生产代码的直连收消息对照**：22:01:20—22:02:05，公开直播间 `145926919737`，45 秒收到并归档 **109 条内容事件**（89 点赞、16 进入、2 弹幕、2 社交）。计数一致，停止后没有新增，工作线程退出；没有用户登录 Cookie。三个用户指定房间当时均下播。该对照证明当前接收路径可用，不计作公共代理成功。

22:14:49—22:15:34，使用临时本机 CONNECT 转发做独立正对照，45 秒收到并归档 **41 条内容事件，其中 3 条弹幕**。生产 HTTP / WS 调用通过诊断工厂注入同一代理，真实 socket 对端和 CONNECT 转发字节数证明没有静默直连；监听与线程结束后清理。此测试绕过公共池候选获取、仍使用本机公网出口，只证明代理传输代码路径能够收消息，不证明公共池可用或出口隔离。

已修正误导性判定和展示：首页初检只接受 HTTP 200，拒绝重定向；界面「可用」改为「首页通过」，TLS EOF 文案改为描述经代理访问目标站点时连接提前关闭，不直接归因代理自身握手。定向代理池 **21 项测试通过**（含新增的重定向拒绝用例），前端语法检查通过；不把这次定向测试说成全量回归重跑。详细对照见 [代理诊断记录](PROXY_DIAGNOSTIC.md)。

### HTTPS 代理来源与房间查询

来源固定为 `https://proxy.scdn.io/api/get_proxy.php?protocol=https&count=20`。本次显式 `country_code=all` 返回空列表，省略该可选参数才返回候选，生产实现已采用省略方式。候选用 HTTP CONNECT 承载经过证书验证的 HTTPS / WSS；没有关闭 TLS 校验，没有使用用户登录 Cookie。

2026-10-14 **20:48:33—20:50:53**，26 次来源请求检测 **500 个不同地址、265 个 IP**，耗时 140.531 秒。**10 个地址、9 个不同 IP**返回抖音 HTTPS 首页 HTTP 200，其余未通过。检查只读取经 TLS 验证的响应头，不验证完整页面、房间元数据或 WebSocket。报告：`.artifacts/proxy-https-expanded-report.json`。

随后使用 **7 个曾通过首页检查的地址 × 3 个指定房间，共 21 次**生产元数据请求，结果为 **0 次成功**。失败包括超时及 TLS 连接或证书验证错误；部分首页请求成功后房间接口仍超时。由于没有取得有效直播元数据，未进入短时 WebSocket 收消息测试。报告：`.artifacts/proxy-room-probe-report.json`。公共代理可能迅速失效，首页检查通过不能证明实际消息可用或池容量充足。

### 每批 50 候选的真实流程检查

2026-10-14 **21:36:24.775—21:37:01.513**，使用生产代理池连续执行两批手动检测，共 36.75 秒。报告为 `.artifacts/proxy-refresh-50-live-report.json`；这与上面的 500 候选探测是独立样本，不合并计算通过率。

| 批次 | 实际新候选 | 首页通过 / 失败 | 来源请求 | 最终状态 |
| --- | ---: | --- | ---: | --- |
| 第 1 批 | 49 | 1 / 48 | 6 | partial；连续 3 次没有新增候选，明确报告 49/50，未宣称补满。 |
| 第 2 批 | 50 | 0 / 50 | 5 | completed；已检测满 50 个，不表示有 50 个可用代理。 |

峰值并发 12，单次来源请求最多 20 个，来源请求间隔至少 1 秒。没有使用 Cookie，没有启动直播监控任务，TLS 校验保持开启。唯一一次通过仍只是抖音 HTTPS 首页检查，不证明房间元数据或 WebSocket 可用。

这次失败地址重复探测数为 0，但两批的 `skipped_failed` 也均为 0，即来源没有返回需要拒绝的历史失败地址。因此这次真实网络检查**不能单独证明历史失败拒绝路径被触发**；该行为的证据来自自动化单测和跨代理模式的后台集成测试。

### 开播时间与当前人数的字段核验

2026-10-14 **20:59:35—20:59:38**，另以明确直连方式读取三个公开房间，不使用用户登录 Cookie。这是字段诊断，不计作代理成功。

| 入口 `web_rid` | 稳定 `anchor_id` | 返回的实际 `room_id` | 状态 |
| --- | --- | --- | --- |
| 100000000000 | 100000003000 | 7696456021958609673 | 下播，status=4 |
| 100000001000 | 100000004000 | 7696406817731447603 | 下播，status=4 |
| 100000002000 | 100000005000 | 7696455970398309120 | 下播，status=4 |

三个响应均没有房间时间字段，均有 `user_count_str="0"`。离线时人数和开播时长显示「—」；本次没有验证直播期间真实时长或人数更新。代码只接受明确 `start_time`，不把通用 `create_time` 或任务启动时间当开播时间；人数只接受精确当前数，不把「1.2万」猜成精确数。报告：`.artifacts/broadcast-display-fields.json`。

### 历史 v1 直连联测

v1 于 **18:18:23—18:21:28** 进行三个房间免登录直连：监控 180 秒，停止后观察 5 秒，共收到并归档 **193 条旧版事件，其中 24 条弹幕**。分房间为 105 / 24 / 64 条，其中弹幕 12 / 6 / 6 条。归档、回调、计数一致，停止后不新增，线程退出等 12 项检查通过。报告：`.artifacts/soak-result.json`。

该旧版计数含当前观看与房间统计，不能直接当成 2.0 内容条数。它证明当时 v1 的三个真实房间接收路径可用，**不证明新版代理连接、当前房间仍在直播或 2.0 全部新功能已真实联测成功**。

## 本机容量测试

报告 `.artifacts/capacity-report.json`，Windows 10 19045、CPython 3.12.14、16 个逻辑 CPU。测试为 **300 个模拟接收任务 + MonitorManager + SQLite + 并发 CSV 导出**，关闭代理；不包含真实 HTTP、WebSocket、TLS、V8 签名和桌面 WebView。

| 场景 | 结果 |
| --- | --- |
| 3,000 条/秒，持续 60 秒 | 预期与实际入库均 **180,000 条**；有效 2,998.5 条/秒，排空滞后 0.031 秒，提交回调延迟 p95 上界 100ms。 |
| 6,000 条/秒，持续 10 秒 | 预期与实际入库均 **60,000 条**；产生 **6.906 秒**排空积压，不代表可持续承受该速率。 |
| 带负载停止与刷新 | 耗时 5.359 秒；最终 **251,342 条**数据库、回调和快照计数一致，停止后不再增加，全部工作线程退出。 |
| 内存 | 被测 Python 进程峰值 RSS **56.3 MiB**；不包括上述真实连接及 WebView 开销，不是整个桌面应用内存上限。 |
| 数据完整性 | SQLite integrity_check=ok；两阶段主计数精确，导出行数与报告一致，无失败批次。 |

这些结果支持本机模拟归档路径的容量评估，不承诺其他机器、磁盘、网络、代理或平台条件下相同性能。任务配置上限 500，200–300 为设计目标；**没有进行 300 路真实平台连接或 24 小时连续运行验证**。

## 0.2.0 安装与发布验收

完整原生扩展、采集后台和 Tauri 外壳已重新构建。发行载荷中实际包含 `qingkan_native.pyd`，版本 0.2.0，与本轮基准 SHA 一致；冻结代码不含 betterproto、Python 生成的 protobuf 模型或 Tk。`build/runtime-assets-check.json` 验证发行目录自己的原生扩展、V8/ICU、签名脚本和 TLS 证书；`build/frozen-runtime-layout-0.2.0.json` 记录模块清单与原生文件 SHA。Rust → 冻结后台自检确认 `processing=rust`、版本、轮询、更新门禁、正常 shutdown 及子进程退出，见 `build/smoke-report-0.2.0.json`。合成认证代理的预览/导入、DPAPI、去重、删除持久化与三次正常退出见 `build/frozen-proxy-import-tscqyo2p/report.json`，测试没有访问代理或直播网络。

在本机 Windows 10 x64 19045 完成了 **0.1.1 → 0.2.0 独立标识 NSIS 隔离升级**。使用保留的 0.1.1 测试安装器，升级前后分别运行安装后的自检，核对 0.2.0 注册版本及 **1204 个安装资源**的 SHA-256。另将 0.1.1 的合成 SQLite 内容库复制到独立数据目录，升级前后 `anchors`、`sessions`、`messages` 全部行和 user_version 的摘要相同。旧 Inno 目录保护返回退出码 2；卸载清理测试注册和程序，同时保留独立测试数据。证据为 `build/nsis-install-check-0.2.0/report.json`、同目录 `database-compatibility.json`、`baseline-self-test.json`、`installed-self-test.json` 与 `build/nsis-upgrade-0.2.0.log`。

测试使用独立产品名、注册项及主程序名，不修改实际用户安装或关闭预览。验收后按最新要求将外框改为 5% 灰（#F2F2F2）、内层及工具栏/状态栏改为纯白，并重新嵌入前端和签名；最终主程序因此与隔离验收时的哈希不同，采集引擎和安装逻辑保持一致。该外观改动经浏览器截图与实际样式读取验证，见 `build/ui-shell-color-validation-0.2.0.json`，随后重新运行原生外壳离线自检；未为四个颜色值重复整套安装/卸载测试。安装器为 `dist/installer/QingkanLive-0.2.0-windows-x64-setup.exe`；公开 CDN 文件位于 `dist/cdn/qingkan-live/0.2.0/`，上传说明见 [CDN_RELEASE.md](../release/CDN_RELEASE.md)。最终 SHA、签名/篡改检查、版本和 ZIP 逐文件校对以 `build/release-package-report.json` 及其引用报告为准，不能把历史安装器哈希当成本包哈希。

现有公钥已独立验证 0.1.0 与 0.1.1 的原签名，私钥未重新生成，见 `build/signing-key-continuity-0.2.0.json`。两版历史安装器、签名、CDN 版本文件及 ZIP 保留。此处没有执行 CDN 上传或线上检查/下载/安装更新，也未在缺少 WebView2 的干净系统验证引导安装分支。

## 历史安装交付状态（0.1.1）

正式版 **0.1.1** 使用 Tauri 原生 NSIS currentUser 安装及签名更新渠道。目录版入口为 `dist/DouyinLiveMonitor/DouyinLiveMonitor.exe`，保留整个目录；安装器为 `dist/installer/QingkanLive-0.1.1-windows-x64-setup.exe`，同目录提供 `.sig` 与 `.sha256`。CDN 上传目录为 `dist/cdn/qingkan-live/`，说明见 [发布说明](../release/CDN_RELEASE.md)。

0.1.1 沿用 0.1.0 的原生 NSIS 渠道与签名公钥。旧 Inno 试制版需要先关闭，再一次性手动安装当前正式版到新的默认目录。安装器不会静默卸载旧程序，并拒绝覆盖旧 Inno 安装目录；旧 AppId 不用于冒充 NSIS 升级。内部二进制文件名、Tauri identifier 和 `%LOCALAPPDATA%\DouyinLiveMonitor` 数据目录保持兼容，后续 0.1.x 通过同一 NSIS 渠道更新。

0.1.1 在 Windows 10 x64 19045 完成了 **0.1.0→0.1.1 的独立标识 NSIS 隔离升级**：在工作区安装保留的 0.1.0 fixture，确认旧版自检后，以 `/UPDATE` 安装 0.1.1，再验证注册版本、新版自检、1250 个资源 SHA-256 和独立数据保留。旧 Inno 目录保护仍返回退出码 2，卸载后的注册清理与数据保留通过。证据为 `build/nsis-install-check-0.1.1/report.json`、同目录 `baseline-self-test.json` / `installed-self-test.json` 与 `build/nsis-upgrade-0.1.1.log`。测试使用独立产品名、注册标识及主程序名，未修改实际用户安装或关闭预览。正式包仅在此后同步验收文档并重新签名；这不是线上 CDN 下载后安装的端到端验收。

0.1.0 在本机 Windows 10 x64 19045 已完成**独立标识的 NSIS 隔离验收**：使用与正式版相同源资源和安装钩子，仅换测试产品名、注册标识及主程序文件名，安装到工作区。1172 个安装资源与源逐文件 SHA-256 一致；旧安装目录保护返回退出码 2；安装后 Rust→冻结后台自检、同版本 `/UPDATE` 模式重装、卸载注册清理和数据保留均通过。真实用户的 1.0 注册和原生 dev 进程未被修改或关闭。证据为 `build/nsis-install-check/report.json`、`build/nsis-install-check/installed-self-test.json` 和 `build/nsis-install-check.log`。这是同载荷隔离安装及重装验证，不是实际用户升级，也不等同于线上 CDN 版本升级演练。该次验收后仅同步结果及 NSIS 许可文本生成 0.1.0 正式包，其历史报告保留于 `build/releases/0.1.0/build__release-package-report.json`。

0.1.0 冻结后台已通过真实管道导入、DPAPI 加密存储/本用户解密、两次重启读回、删除持久化、无效请求不改配置、update_readiness 与三次正常退出检查，全部使用合成认证代理、零网络。证据为 `build/frozen-proxy-import-oy3wuqtd/report.json`。冻结 CSV/XLSX 导出与 HTTP 200/302 初检策略也通过，0.1.0 历史报告为 `build/releases/0.1.0/build__frozen-export-check__report.json`、`build/frozen-proxy-policy-report.json`。主程序、后台和安装器的 PE ProductVersion/FileVersion 该次均核对为 0.1.0 或 Windows 四段等价值，新图标全部 6 个尺寸已嵌入，历史报告见 `build/releases/0.1.0/build__branding-report.json`。

更新签名使用与生产 updater 一致的 minisign 验证器。最终报告 `.artifacts/release-signature-verification.json` 核对 manifest 版本、下载地址、发布说明、签名和安装器 SHA-256，并要求原包验签成功、单字节篡改被拒绝。签名私钥只在本机被忽略且限制访问的发布目录，不进入分发内容。此 `.sig` 不是 Authenticode 签名；本包仍未配置发行方 Windows 代码签名证书。CDN 上传由用户执行，上传前没有线上检查更新/完整下载更新实测结论。

此前 2.0 Inno 试制版的全新安装、1.0 等效升级、卸载与数据保留报告属于**历史版本证据**，不适用于新 NSIS 安装器哈希。原记录仍保留于 `packaging/install-test-v2-*-result.txt`、相关安装/卸载日志及 `build/release-package-report-503b4acddfd0.json`，不再作为当前正式版的直接升级指南。

包内含 Python 采集环境，无需另装 Python、Node.js、Excel 或播放器。界面需要 WebView2；NSIS 内嵌微软引导程序，缺失运行时才调用并需联网。本机已有 WebView2，缺失运行时的安装分支未在全新系统实测。尚未全面验证所有 Windows 版本或多屏 DPI 组合。

## 剩余限制

- 非官方网页协议及代理受接口、签名、风控、登录条件与网络变化影响；少量公网代理已完成短时互动消息接收，但没有长期稳定性、规模容量或本次私密认证代理 WSS 成功的证据。
- 未完成 24 小时运行、长时间断网、休眠恢复、所有消息类型和登录账号并发的真实验证。
- 只保存运行期间收到并成功入库的内容；不补采历史，不能恢复平台未送达或断线期间的数据。有 ID 的去重不等于完整或恰好一次交付保证。
- 实时显示缓存有界，拥挤时可能跳过早期显示批次；内容库写入失败会暂停对应房间并提示。

保留上游 [MIT 许可](../../LICENSE)。依赖和字体声明见 [THIRD_PARTY_NOTICES.txt](../../THIRD_PARTY_NOTICES.txt)、[THIRD_PARTY_DESKTOP_NOTICES.txt](../../THIRD_PARTY_DESKTOP_NOTICES.txt) 及包内许可证。
