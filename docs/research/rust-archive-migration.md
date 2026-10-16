# 0.2.0 Rust 归档与月导出迁移

审阅日期：2026-10-15。采用 `mattpocock-skills:codebase-design` 的 deep module / interface / seam / adapter 原则。现阶段保留已验证的 Python HTTP、WebSocket、代理、JS/V8 签名与调度；把 SQLite 写入和月导出作为一个 Rust module，经同进程 PyO3 adapter 使用。

## Interface 与实现位置

`crates/qingkan-core/src/archive.rs` 管理 schema、事务、有界批写、提交回执及只读快照；`export.rs` 管理月筛选、七列映射、CSV 与流式 XLSX、取消及原子替换。`crates/qingkan-native/src/archive_bindings.rs` 只做输入转换、错误映射和 `Python::detach`。Python 保留原模块名的薄 adapter，使 monitor/backend 无需了解 SQLite 游标或 ZIP/XML。

`ArchiveStore.record_message(record, title, started_at) -> bool` 必须在事务提交后返回；false 仅表示已有服务端消息 ID。数据库失败须唤醒该批全部等待者，不能提前发送 UI 成功事件。一个有界 writer 队列（默认 4096）、一个 SQLite 写连接，最多 256 条 / 4ms 成批；单条仍为进程内函数调用，不引入逐条 stdio RPC、pickle 或 Python/Rust 数据库双写。输入复制成 Rust-owned 数据后再 detach，等待队列/提交期间不持 GIL。性能需实测，不能从语言更换推断倍数。

导出调用一次进入 Rust，数据库行不逐条回到 Python。取消用原生 AtomicBool token，backend 直接持有兼容 set/is_set 的 token；旧 threading.Event 只在兼容 adapter 使用一个短生命周期 watcher。导出线程仍由 backend 的现有任务管理器拥有，最多两个并发任务和更新安装 readiness 合同不变。

## 必须保留的合同

- 直接打开现有 `messages.sqlite3`，保持 user_version=1、anchors/sessions/messages 字段和索引；拒绝未来 schema，不复制或隐式导入旧 JSONL。
- WAL、foreign_keys=ON、synchronous=NORMAL、busy_timeout=10s；消息、主播及场次元数据同事务。唯一键 `(anchor_id,session_id,message_id)` 仅对非空 message_id 生效；长 ID 始终 TEXT。
- 真主播 ID 与真实场次 ID 必须存在，七种内容类型入库，统计/运行日志排除。未知发送时间保持空、time_source=missing；月份按收到时间回退归属，并在报告计数。
- 月界限为固定 UTC+08:00，与现实现一致；起点含、终点不含。每次导出先 BEGIN 并读取以固定同一快照，名字查询和所有主播查询共享该连接；同时采集写入不改变导出内容。
- 固定七列：主播、直播间场次、发消息人ID、发消息人名字、消息类型、消息内容、发消息时间。行中原昵称不修改；sheet 使用快照内最新主播名。
- XLSX 按稳定主播 ID 分 sheet；选中无当月消息者仍有表头；31 UTF-16 单元、非法字符、History、大小写同名、超行续表、`_xNNNN_` 字面量保护、七列文本样式和无公式均保留。CSV 合并时间排序、UTF-8 BOM/CRLF，并为公式前缀加单引号。CSV 中长 ID 的字节完整不代表 Excel 自动打开时不会自行转数字，应继续提示以文本导入。
- 临时文件与目标同目录，流式写入、flush/fsync，完成并再次检查取消后原子替换。失败/取消保留旧目标，清理临时文件。拒绝 XLSX 无法保真表示的控制字符和过长单元格，建议 CSV。
- close(timeout) 停止接收新记录、排空已接收队列，超时返回 false；不能为达超时丢已接收记录。metrics 必须包括在途事务 pending_requests，供 updater 门禁使用。

## 依赖与技术选择

已在 Cargo.lock 固定并完成本机构建：PyO3 0.29.3、rusqlite 0.40.2（bundled）、chrono 0.4.45、crossbeam-channel 0.5.17、serde_json 1.0.154、zip 4.6.1、tempfile 3.27.0、thiserror 2.0.21。采用 zip 直接流式写已知 OOXML 模板，而非引入工作簿内存模型和不同字符串规则。unicode-casefold 0.2.0 的表仅 Unicode 9，因此先以 Rust 当前 Unicode lowercase 处理较新字母，再完整 casefold；已覆盖 ß/SS 与较新 Georgian Mtavruli 大小写，不只做 ASCII lowercase。

PyO3 官方明确 Rust 长操作应 detach，使其他 Python 线程继续执行：[PyO3 performance](https://pyo3.rs/main/performance)。SQLite Rust 接口采用 [rusqlite](https://docs.rs/rusqlite/latest/rusqlite/)，不改变磁盘格式。上述第三方依赖的版本最终以根 Cargo.lock 为准。

## 验证与风险

现有 `tests/test_archive.py` 16 项和 `tests/test_export.py` 24 项描述 0.1.1 合同；保留结果型场景，将 monkeypatch Python 私有 writer/XML 函数的测试迁成 Rust module interface 测试，不伪造旧私有结构。重点覆盖：旧数据库双向读取、20 producer 400条、重复/空ID、事务失败整批回滚并唤醒、close与锁竞争、同快照边写边导、取消/目标占用、名字同名与历史改名、七列长ID/名字、UTF-16边界、跨月和缺发送时间。

复用 `.artifacts/release-0.1.1-export-validation.md`、`build/frozen-export-check-0.1.1/report.json` 和独立 openpyxl 检查作为输出语义基线；发布前重新运行冻结 backend 的离线导出验收。容量脚本只能说明本机模拟吞吐，需同数据与配置比较 CPU/RSS/吞吐，不能声称 500 个真实抖音连接。

PyO3 `.pyd` 必须纳入 PyInstaller hidden import/binary 及许可证收集；导入失败必须明确失败，不静默回退旧高频实现。CPython ABI、MSVC 运行库、原生 DLL 查找需在干净安装测试。Rust FFI panic/线程异常必须映射成明确错误并确保等待者退出。长期导出持读快照可能让 WAL 增长，这与既有语义相同，应保留取消和双任务上限。

## 已实施和验证（0.2.0）

归档、导出、PyO3 adapter 已实现。生产 Python adapter 位于 `python/collector/`；诊断读取仍可用只读 SQLite snapshot，整次生产导出与所有写入都在 Rust 中完成。Python `ArchiveStore.record_message` 的 JSON 输入在 detach 后解析；一个提交回执保持原来的停止/回调合同，未增加进程通信或双写。

Rust 新测试：归档 9 项、导出 12 项。原 Python 结果型场景 31 项通过；9 项依赖私有 writer/XML monkeypatch 的旧测试由 Rust 中失败回滚、关闭竞态、流式取消、超行与固定快照场景替换。另有 `tests/test_native_archive_compat.py`：0.1.1 写入 → Rust 续写 → 0.1.1 重读且 integrity_check=ok；原生取消保留旧文件；极端 close timeout 返回 ValueError，不越过 FFI 触发 panic。

复核发现并修复两项性能开销：SQLite 三条写语句复用 prepared statement；XLSX 每行构造有界缓冲后一次送入压缩流，避免每个 XML 片段触发压缩调用。关闭等待仅接受 0..86400 秒；旧正常 15 秒行为不变。完整安装器/冻结 sidecar 验收由发布流程继续进行。

## 本机模拟性能结果

可复现命令：`.venv/Scripts/python.exe scripts/diagnostics/benchmark_native_archive.py`。每个实现独立进程，300 个同步 Python producer，30,000 条以 3000/s 排程、另 30,000 条以 6000/s 排程；期间各有一次 CSV 快照导出，最后导出相同 60,000 行的 CSV 和 300-sheet XLSX。包含 Python JSON → PyO3 → Rust → SQLite 提交开销，不包含真实平台连接、TLS、V8 签名或 WebView。脚本仅使用合成消息，无联网。

证据：`build/benchmark-0.2.0/archive-export.json`。旧版与新版都精确归档 60,000 条，integrity_check=ok；每次并发导出实际行数等于报告数；最终 CSV/XLSX 的规范化逻辑 SHA-256 分别完全相同，未比较 ZIP 时间戳或同时间跨主播消息的任意插入次序。

| 指标 | 0.1.1 Python 参考实现 | 0.2.0 Rust adapter |
| --- | ---: | ---: |
| 3000/s 排程实际吞吐 | 3000/s | 3013/s |
| 3000/s 平均提交等待 | 36.57ms | 23.87ms |
| 3000/s CPU 秒（含并发导出） | 4.92s | 4.38s |
| 6000/s 短突发排空吞吐 | 3607/s | 4516/s |
| 6000/s CPU 秒（含并发导出） | 9.28s | 8.16s |
| 300 producer 阶段进程峰值 RSS | 46.45MiB | 50.44MiB |
| 固定 60,000 行 CSV | 3.090s | 1.008s |
| 固定 60,000 行 / 300 sheets XLSX | 5.089s | 1.525s |

这是单机单轮结果，受系统其他工作与文件缓存影响。两版都没有达到持续 6000/s；新实现没有降低该场景内存峰值，不能据此声称 300/500 个真实直播间容量。报告中最终 XLSX 的 process peak 字段受到先前比较读取 CSV 分配的影响，因此内存比较仅使用 producer 阶段，未把比较器峰值归因于导出器；后续脚本已将全部逻辑比较移到计时和内存采样之后。

首轮未优化结果保留在 `.artifacts/native-archive-benchmark.json`，其 Rust 6000/s 排空吞吐和 XLSX 比旧版慢。测量揭示 cached prepare / 整行压缩缓冲的重要性，以上最终结果来自这两项修复后的 release pyd（SHA-256 `de7cda7a71063bca9e79cd5f6e0e08cf8ee40f498faf68c48f95f2a3c7006e3d`）。

独立 openpyxl 完整读取两份性能测试工作簿，各 60,000 行 / 300 sheets；七列、长 ID 字符串、观众名字与无公式检查通过，证据 `build/benchmark-0.2.0/archive-export-openpyxl.json`。最终冻结 sidecar 的离线导出验收报告为 `build/frozen-export-check-0.2.0/report.json`（native_version=0.2.0、processing=rust）；再次用 openpyxl 检查该最终产物，3 个 sheet 的行数为 2/1/0，主播名字/观众名字/长 ID 原文全部符合 fixture，证据 `build/frozen-export-check-0.2.0/openpyxl-validation.json`。
