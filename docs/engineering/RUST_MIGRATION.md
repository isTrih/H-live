# 0.3.0 纯 Rust 后端迁移

从 0.2.4 的 Tauri + Rust 核心 + Python 采集迁移至进程内 Rust 后端。源码不再包含 Python 采集、PyO3 或独立 JavaScript/V8 签名引擎。Tauri WebView 继续使用已有 HTML/CSS/JavaScript 界面。

## 已完成

1. signature 从同一参数结构生成规范串与 WSS query，保留 Remastered MIT 署名；五组上游向量、随机边界和真实游客 WSS 连接通过。
2. net、transport、monitor 使用 Tokio、reqwest、rustls 与 tokio-tungstenite；HTTP CONNECT / SOCKS5、ACK、心跳、帧与解压限制、停止确认由 Rust 实现。没有关闭证书校验。
3. proxy、maintenance 管理稳定出口租约、请求/建连节流、429/444 冷却、候选排除、过期退役及 HTTPS 订阅。代理首页检测通过与实际 WSS 成功分开计量。
4. runtime 对桌面提供小型同步请求接口；内部有界事件缓存、固定工作线程、有界归档并发，停止时等待采集、工作任务、归档和日志退出。
5. qingkan-core 继续负责 protobuf、SQLite、质量规则和导出；同一主播/场次身份、暂停续时、福袋合并和历史数据契约不变。
6. 旧 DPAPI 代理和订阅读取后继续加密保存；旧登录字段不解密、不暴露、不发送。登录设置与新礼物采集删除；历史礼物读取/导出保留。
7. 构建、许可收集、发行检查改用 PowerShell 和 Rust，不需要 Python。应用标识、产品名、更新公钥和 CDN 清单地址沿用原渠道。

## 验证与限制

详见 [0.3.0 验收](VALIDATION_0.3.0.md)。离线 300 任务/50 连接可验证本机调度、解析与入库，不能证明抖音真实 50 房间长期可用。真实游客观察收到进入事件，没有在观察窗口收到聊天，不将合成聊天测试当作真实聊天证据。HTTP 444/429、游客下发范围和公共代理质量依旧受平台及网络影响；未完成真实大规模长时间与旧版同条件对比。

## 上游取舍

原项目用于理解协议和业务行为；Remastered 用于原生签名参考，没有引入其播放器、抓流、登录、Win32 UI 或每房间心跳线程。完整比较与固定提交链接见 [调研报告](../research/remastered-rust-migration-2026-10-16.md)。
