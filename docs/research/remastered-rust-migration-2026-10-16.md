# Remastered 对纯 Rust 后端迁移的可用性（2026-10-16）

## 结论与范围

Remastered **确实将网页 WSS 所需的 `signature` 做成了纯 C 算法**，不是仅实现同名的 HTTP `X-Bogus` 参数。其 `compute_signature` 将指定 13 个参数拼接、求 MD5 后传入 `xbogus_generate`，再把结果追加到 WSS 的 `signature` 参数。该路径无需 Node、JS 引擎或外部签名服务，算法规模很小，可以直接用安全 Rust 重写。这能消除当前 Python/MiniRacer 签名路径的运行时依赖；尚未因此证明新 Rust 网络后端已经能连接，仍需离线兼容与真实游客 WSS 验收。[固定提交 fetcher.c](https://github.com/eplayn/DouyinLiveMonitor-Remastered/blob/c00775f2fff7457f2f5903d7c788a7a996dc3b1f/src/fetcher.c#L190)

**建议复用签名算法及测试向量，重写连接、调度和资源边界；不直接搬运 C 后端或 Win32 GUI。** 上游 GUI 实际仅维护一个选中房间的 WSS；每 50 主播轮换游客凭据是分组策略，不是 50 并发限制。没有发现其达到本项目 200–300 任务、约 50 同时开播、长期监听目标的测试材料。迁移应沿用本项目已有归档、协议及场次隔离能力，而不是回退到上游文本 JSONL。[gui.c](https://github.com/eplayn/DouyinLiveMonitor-Remastered/blob/c00775f2fff7457f2f5903d7c788a7a996dc3b1f/src/gui.c#L405)

本次需求已明确**不使用登录 Cookie、不记录礼物、不播放音视频**。这里只研究匿名弹幕路线；登录、礼物、抓直播源、IPTV、播放器均不应进入迁移范围。

## 来源与下载边界

- 上游：[eplayn/DouyinLiveMonitor-Remastered](https://github.com/eplayn/DouyinLiveMonitor-Remastered)。GitHub API 树读取固定为提交 **`c00775f2fff7457f2f5903d7c788a7a996dc3b1f`**，访问日期 **2026-10-16**。
- 只下载 43 个文本文件到 `build/upstream-review/Remastered/`，没有 `.git`、二进制、依赖安装或用户运行配置。`SOURCE_MANIFEST.json` 记录仓库、提交、下载 UTC、每文件 Git blob、字节数和 SHA256，可核验下表内容。
- **没有编译或执行任何上游 C、Node、JS 或测试程序**。本报告的算法结论来自源码；测试向量是上游已有材料，并非本次成功运行记录。只用本地 .NET MD5 计算了空输入的二次摘要常量，没有执行下载代码。
- 没有读取用户配置、登录凭据、数据库、私钥，没有调用外部签名服务或新增直播请求。`node_scraper/bdms.js`、`tools/sign_folded.js` 不在该提交文件树中；本次没有另外下载这些第三方 SDK。

| 本地相对下载路径 | 调研用途 |
| --- | --- |
| `src/xbogus.c`、`include/xbogus.h`、`src/md5.c` | 纯函数签名、随机包装、MD5 |
| `test/test_xbogus.c`、`tools/verify.mjs`、`tools/ground_truth.mjs` | 固定向量及作者对拍方式；只读 |
| `src/fetcher.c`、`src/http.c`、`src/enter.c` | 实际签名调用链、WSS/HTTP、房间解析 |
| `src/protobuf.c`、`src/gzip.c`、`src/messages.c` | 信封、解压、字段投影与 ACK |
| `src/gui.c`、`src/main.c`、`src/json.c` | 并发、取消、明文配置、归档及播放器启动 |
| `CMakeLists.txt`、`LICENSE` | 实际编译依赖与许可 |
| `capture.mjs`、`node_scraper/a_bogus.mjs`、`node_scraper/bdms_shim.mjs` | 确认另一路 Node 采集与 WSS 的区别 |

## WSS 签名的准确算法

### 1. 先对实际连接参数建立规范输入

参数名按以下**固定顺序**，每项为 `key=value`，用英文逗号连接；不存在的值保留空字符串。不能改成排序后的整个 query，也不能只对 room ID 求摘要。

```text
live_id,aid,version_code,webcast_sdk_version,room_id,sub_room_id,sub_channel_id,did_rule,user_unique_id,device_platform,device_type,ac,identity
```

上游的实际值来自 `compute_signature` 中的固定模板，例如 `live_id=1`、`aid=6383`、`version_code=180800`、`webcast_sdk_version=1.0.14-beta.0`、`did_rule=3`、`device_platform=web`、`identity=audience`，其余 ID 须与 URL 对应。对 UTF-8 字节求 `MD5` 得到 16 字节摘要 `D`；C 接口先转成 32 位 hex，又立即解码回来。Rust 可直接传 `[u8; 16]`，避免无意义编解码。[fetcher.c 的建 URL 与签名](https://github.com/eplayn/DouyinLiveMonitor-Remastered/blob/c00775f2fff7457f2f5903d7c788a7a996dc3b1f/src/fetcher.c#L169)

上游 C 把 `user_unique_id` 等常量写了两份，容易签名/URL 脱节。Rust 应从同一类型化参数对象同时生成 query 和规范串；在兼容测试中核对 13 个字段和空值。原 Python 固定提交的 split 逻辑、当前 Python `parse_qsl` 对转义和含等号值的行为不同；当前这 13 个参数通常是简单 ASCII，但不要把通用解析差异隐藏在迁移里。

### 2. 纯函数变换

定义 `M(x)` 为 MD5 的 **16 字节原始摘要**，`h` 为 0/1，`b8` 和 `r` 为单字节：

1. `E = M(M(empty))`。本次用 .NET 独立计算得 `59adb24ef3cdbe0297f05b395827453f`，因此 `E[14]=0x45`、`E[15]=0x3f`。
2. `Q = M(D)`，不是 `MD5` 作用在 32 个 hex 字符上。
3. 九字节 `B = [1, 0, 1, 14, E[14], E[15], Q[14], Q[15], b8]`。
4. 尾部追加上述九字节的逐字节 XOR，得到十字节 `P`。
5. 使用单字节密钥 `[r]` 对 `P` 做标准 RC4 KSA+PRGA，不丢弃开头密钥流，得到十字节 `C`。
6. 十二字节 `R = [0x40 | ((h & 1) << 4), r] || C`。
7. 对 `R` 按每三字节/四个六位索引做 Base64，使用下列 S1 字母表的前 64 个字符。输出恰好 16 个 ASCII 字符，无 `=` padding。C 中结尾的 `=` 不会在此长度使用。

```text
Dkdpgh4ZKsQB80/Mfvw36XI1R25-WUAlEi7NLboqYTOPuzmFjJnryx9HVGcaStCe=
```

这是 [`xbogus_compute`](https://github.com/eplayn/DouyinLiveMonitor-Remastered/blob/c00775f2fff7457f2f5903d7c788a7a996dc3b1f/src/xbogus.c#L76) 的全部数据通路。它**不读取时间、UA、Cookie、网络或机器环境**；环境相关字节在该实现固定为 `envcode=1`、`ubcode=14`、`initialized=0`、`bogusIndex=1`。这里的 MD5/RC4 是平台协议兼容变换，不能用它为凭据或配置提供安全保护。

### 3. 随机、时钟与状态

[`xbogus_generate`](https://github.com/eplayn/DouyinLiveMonitor-Remastered/blob/c00775f2fff7457f2f5903d7c788a7a996dc3b1f/src/xbogus.c#L129) 依次用 `rand()&1`、`rand()&255`、`rand()&255` 产生 `h,b8,r`；CLI/GUI 在启动时用时间与 PID 播种。因此**纯函数无时间依赖，随机包装有进程播种依赖**，不能笼统称整条调用链确定性。

`tools/verify.mjs` 中作者的 JS 模型对应 `floor(100*Math.random())&1`、`floor(255*Math.random())&255` 两次：后两个随机值范围为 0–254，而 C 包装能取 255。现有零随机向量不能证明这一分布差异有无服务端影响。Rust 可保留一个显式随机参数的纯函数，并以单独包装提供运行时随机值；对兼容原 JS 路线，可按 JS 范围生成，测试另覆盖 255 的确定性边界。不要复用全局 C `rand` 或为每连接复用 JS context，因为作者对拍会为每个输入建立新 context，使 `bogusIndex` 始终为 1。[verify.mjs](https://github.com/eplayn/DouyinLiveMonitor-Remastered/blob/c00775f2fff7457f2f5903d7c788a7a996dc3b1f/tools/verify.mjs)

上游 WSS 中还有独立的时间/设备标识字段，但其 `build_wss` 直接硬编码了旧 cursor、2024 年毫秒时间与 did，不能误以为这些是签名算法要求。新实现应让 URL 与签名共用真实参数，避免把旧 URL 模板全部照抄。

### 4. 已有固定向量及验收方式

以下五组全部使用 `h=0,b8=0,r=0`，逐字取自 [test/test_xbogus.c](https://github.com/eplayn/DouyinLiveMonitor-Remastered/blob/c00775f2fff7457f2f5903d7c788a7a996dc3b1f/test/test_xbogus.c#L32)：

| 32 位 MD5 hex 输入 | 16 字符输出 |
| --- | --- |
| `b5f7f2b3e491718deb69195be3284b1b` | `fDpl4KiMGE7XTRYb` |
| `51b61920a08ead94f29e425711c5ba5c` | `fDpl4KiMGEN6PVTm` |
| `00000000000000000000000000000000` | `fDpl4KiMGEihqVPM` |
| `ffffffffffffffffffffffffffffffff` | `fDpl4KiMGEi/CKON` |
| `0123456789abcdef0123456789abcdef` | `fDpl4KiMGENs2KOV` |

作者另有 MD5 空串、`abc`、fox 三组标准向量。随机包装的测试实际只检查长度；虽然注释说长度与字符集，代码没有检查字符集。上述材料未覆盖非零随机输入、无效 hex、规范串到最终签名的整链，也没有服务端握手断言。

迁移建议：先把五向量和一组真实参数规范串用作 Rust 回归，增加非零 `h,b8,r`、RC4 单字节 key=0/255 和非法输入边界。需要扩大与原 JS 的对拍时，应固定随机序列、每输入创建新 isolate 并比对相同中间值；作者 `verify.mjs` 依赖仓库之外的 `../../sign.js`，不能把下载目录当成可直接运行的完整验证环境。**本研究未运行它。** 离线通过后仍应进行不带登录 Cookie、仅有游客凭据的真实 WSS 验证，证明连通与聊天/在线等消息链路；礼物不在验收项中。

## 运行时依赖：WSS 原生，抓源另有 Node

[CMakeLists.txt](https://github.com/eplayn/DouyinLiveMonitor-Remastered/blob/c00775f2fff7457f2f5903d7c788a7a996dc3b1f/CMakeLists.txt) 的 `dlm_core` 链接 C 模块与 zlib，CLI/GUI 再链接 WinHTTP 等 Windows 库；没有启动 Node 或 JS 签名服务。

另一路 [`capture.mjs`](https://github.com/eplayn/DouyinLiveMonitor-Remastered/blob/c00775f2fff7457f2f5903d7c788a7a996dc3b1f/capture.mjs) 导入 [`a_bogus.mjs`](https://github.com/eplayn/DouyinLiveMonitor-Remastered/blob/c00775f2fff7457f2f5903d7c788a7a996dc3b1f/node_scraper/a_bogus.mjs#L38)：后者读取未随仓库提供的 `bdms.js`，通过 Node `vm` 和浏览器 shim 执行并拦截 fetch，串行生成 `a_bogus`。它用于 feed/分类直播源采集，**不是 C WSS `signature` 的实现**。其注释中的 SM3/环境/时间说明不足以构成完整纯 Rust `a_bogus` 实现；这不是本次已解决的问题，也不需要为当前不抓直播源的迁移引入它。

## 应重写和不应继承的实现

| 方面 | 源码观察 | 迁移处理 |
| --- | --- | --- |
| TLS 与目标 | `http_get`、WSS 使用 WinHTTP；相关 C 文件未发现 `SECURITY_FLAG_IGNORE_*` 或关闭证书校验选项。HTTP helper 接受普通 HTTP/HTTPS URL，缺少统一的目标主机策略。 | 不应误报上游关闭 TLS。Rust 保持正常证书与主机名校验，限定平台 HTTPS/WSS 目标；代理配置不能替换信任语义。 |
| HTTP 资源 | 每请求新建 session/connection，响应体 `realloc` 增长无总量上限；部分分配或读失败结束循环后仍返回成功，状态码需调用者判断。 | 共享客户端连接池；设置连接/请求/读取时限、重定向/状态处理和响应大小上限，部分响应报错。 |
| WSS 参数/解析 | `build_wss` 固定 did/cursor/旧时间；room ID 用搜索 `roomId` 后 10 字符内的数字；host/path 固定数组 `memcpy` 缺少长度检查。 | 类型化、校验 ID 和 query；沿用本项目受控解析与错误分类，不复用裸指针或旧设备模板。 |
| 帧与解压 | `ws_recv_binary` 从 256 KiB 起倍增无总上限；UTF-8/close 也可能返回到二进制解析；`gzip_decompress` 输出无总上限。 | 有界消息/分片/解压/字段预算；明确处理帧类型与 close，保留现有 Rust 受限解码器。 |
| 协议与 ACK | 手写 protobuf 投影；回调执行完才 ACK；ACK 临时数组 8192，超长直接不发；仅初始握手三次重试，连接断后没有完整重连循环。 | 继续使用已有 Rust 协议/ACK/坏消息隔离；持久化通过有界队列异步处理，按状态重连并设退避。 |
| 调度与性能 | GUI 全局一个 `g_fetch_thread`；选中房间停止旧连接再启动新连接。批量检测对所有主播一次性 `CreateThread`，每 50 人仅共享 ttwid；线程句柄未在此循环回收。每 WSS 另开一条心跳线程。 | 单一异步运行时，受限检测/建连并发及统一限流；每房独立场次，选择 UI 不控制已有房间生命周期。 |
| 取消与共享状态 | `stop_fetch` 等待 15 秒后不检查等待结果便关闭线程句柄并重置全局状态；stop 使用 `volatile int`。 | 取消 token、明确 join/退出确认和代次隔离；超时不能视为已退出。 |
| 配置与登录 | CLI/GUI 将 Cookie 明文序列化进 config；`capture.mjs` 有直接覆写 anchors 的路径。 | 按用户要求无登录凭据入口/转发/存储；不要运行迁移或导入脚本触碰用户原配置。游客 ttwid 仅作匿名会话用途。 |
| 进程启动 | `main.c`/`gui.c` 为播放器用 `CreateProcessA` 加拼接命令行；未发现 `cmd.exe`/PowerShell 外壳签名调用。 | 不引入播放器或任意外部程序启动。它不是“后台偷偷运行 Node”的证据。 |
| UI/归档 | UI 队列 4096、历史 50000 有界是可借鉴思路；每消息写 JSONL 并开关文件。手写转义仅覆盖引号/反斜线，控制字符可能破坏 JSONL；缺少原消息结构。 | 保留本项目批量 SQLite 归档及结构化事件/导出；Tauri 端按批推送、UI 只保留视窗与有限缓存，丢弃展示不能冒充归档完整。 |
| 字段与时间 | `messages.c` 用统一字段 15/33 猜事件时间，缺失回退系统时间；礼物/播放字段与新需求冲突。 | 保留明确字段来源和“未知”语义，不用接收时间伪装服务端时间；不移植礼物处理。 |

以上安全/性能判断来自固定提交的 [http.c](https://github.com/eplayn/DouyinLiveMonitor-Remastered/blob/c00775f2fff7457f2f5903d7c788a7a996dc3b1f/src/http.c#L61)、[fetcher.c](https://github.com/eplayn/DouyinLiveMonitor-Remastered/blob/c00775f2fff7457f2f5903d7c788a7a996dc3b1f/src/fetcher.c#L117)、[gzip.c](https://github.com/eplayn/DouyinLiveMonitor-Remastered/blob/c00775f2fff7457f2f5903d7c788a7a996dc3b1f/src/gzip.c#L6)、[gui.c](https://github.com/eplayn/DouyinLiveMonitor-Remastered/blob/c00775f2fff7457f2f5903d7c788a7a996dc3b1f/src/gui.c#L492)、[messages.c](https://github.com/eplayn/DouyinLiveMonitor-Remastered/blob/c00775f2fff7457f2f5903d7c788a7a996dc3b1f/src/messages.c#L46)。这些是源码风险分析，没有实施攻击、故障注入或运行上游程序。

## 与当前项目的衔接

当前 `python/collector/liveMan.py` 的 `_generate_signature` 仍创建 MiniRacer 并执行 `get_sign`；`qingkan-core` 已有 Rust 协议/归档能力。最小迁移接缝是先让 Rust 提供等价签名纯函数，再让 Rust 网络任务直接消费已有协议解码结果和归档服务；Tauri 负责命令、状态快照和批量 UI 事件。不要为替换签名重写已验证的导出与场次契约。

移除运行时 Python/JS 的完整验收还需检查：Tauri 进程不再生成/管理 Python sidecar、签名/HTTP/WSS/调度都在 Rust、发行包不含解释器/JS 签名 SDK、停止和退出可回收全部任务、50 在线负载中的队列/内存/CPU/丢弃计数有实测。Remastered 的源码可以指导实现，但没有提供这些结果，本报告不把预期性能收益写成实测收益。

## 许可证与归属

[Remastered LICENSE](https://github.com/eplayn/DouyinLiveMonitor-Remastered/blob/c00775f2fff7457f2f5903d7c788a7a996dc3b1f/LICENSE) 标为 **MIT，Copyright (c) 2026 eplayn**，要求复制或实质部分中保留版权及许可声明。下载原文 SHA256 为 `E4060ABE0E016E48BDEF5AE10C0A3E6FB3C50407E22CC219E94425A73B227248`。若移植 `xbogus.c` 的实现或测试，应在本项目相应代码和发行 notices 中明确此来源并保留许可原文。

作者将算法称为从 byted_acrawler 1.0.0.53 逆向还原；这份 MIT 声明不能代替未提供的网页 SDK 自身许可。不要复制/打包 `bdms.js` 或 `sign_folded.js`；本次也未获取它们。原项目、现有 proto、归档及其它第三方依赖的已有归属仍分别保留，不能因为新 C 仓库标 MIT 就统一改写其许可。
