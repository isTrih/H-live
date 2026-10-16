# 0.3.0 架构

```text
WebView：src/main.js + features/*
  │ 受限 IPC：低频请求、有界事件和状态快照
Tauri：src-tauri/src/{lib,backend,desktop,update_policy}.rs
  │ spawn_blocking；关闭和更新必须等待归档完成
h-live-runtime（同进程 Rust）
  ├─ runtime / requests：任务代次、取消、查询、更新恢复
  ├─ monitor / transport / net / signature：HTTPS/WSS、ACK、心跳
  ├─ proxy / maintenance：出口租约、节流、冷却、退役、订阅
  └─ storage：原子配置与 Windows 用户 DPAPI
      │ 有界后台调用与提交确认
qingkan-core
  ├─ protocol：protobuf、限量解压、事件规范化
  ├─ archive：SQLite 单写线程、去重、场次与分钟在线
  ├─ quality：本地有效发言规则与可取消重算
  └─ export：一致快照、CSV/XLSX、临时文件原子替换
```

采用 codebase-design 的深模块原则：外部仅需了解请求、取消和提交契约，网络连接和持久化生命周期由模块内部管理。Transport 是真实网络与离线压测适配器共用的测试 seam；测试适配器只在显式 test-support feature 下编译，默认发行不包含它。

## 并发与资源

固定 4 个 Tokio 工作线程，阻塞工作池最多 16，存储并发 8。每个直播间对应异步任务，不对应独立 OS 线程。每出口共享客户端，最多两个 HTTP 请求和两个建连，建连间隔至少 500 ms；已建立的 WSS 长连接不占用建连名额。

消息提交确认后才增加实时计数。归档队列有背压，实时展示缓存最多 2000 条；界面缓存淘汰不等于数据库丢失。日志独立有界队列并轮转。任务停止会取消网络等待并等待退出，再写入场次时长检查点；同场恢复不计停止间隔。限量帧、解压及协议限制见 core 测试。

## 数据、权限与更新

沿用 schema v3、原数据目录和应用标识，数字 ID 按字符串传递。新礼物丢弃，旧礼物兼容读取；进入消息不显示在实时列表。在线人数存入分钟统计，不混入聊天消息。

没有登录 Cookie 接口。临时游客会话 Cookie 仅服务平台请求；代理凭据不进入日志/快照。订阅仅 HTTPS、公网固定 DNS 解析、无重定向、有大小限制。TLS 默认验证证书；HTTP CONNECT 的代理认证与隧道内平台请求分离。

确认更新先写入不含凭据的恢复意图，停止采集并完成归档，确认线程退出才调用更新器；取消或新版启动后恢复原任务。继续使用原更新公钥。私钥不进仓库与发行文件。

旧 Python/PyO3 实现可在初始 Git 提交中查看，当前运行和构建不使用它。验证范围与未覆盖项见 [验收记录](VALIDATION_0.3.0.md)。
