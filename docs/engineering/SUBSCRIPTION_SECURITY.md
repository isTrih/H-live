# HTTPS 代理订阅读取边界

实现位于 `python/collector/proxy_subscription.py`。该模块只读取候选，不持久化配置、不创建自动刷新计划，也不代表候选已经通过代理健康检查。配置与默认每 60 分钟刷新由调用方管理。

## 接口

- `subscription_summary(url)`：纯本地检查 HTTPS、无 userinfo、合法主机名与端口；返回 `host / port / label / has_query`，不解析 DNS，不返回路径或 query。
- `validate_subscription_url(url)`：返回规范化 URL，额外检查当前 DNS 的全部地址均为公网。返回值可能含订阅令牌，只供后台保存，不应回传前端或日志。
- `fetch_subscription(url, stop_event=None)`：重新解析 DNS，返回 `records / count / rejected / duplicates / fetched_at / source`。`source` 采用上述脱敏摘要；`records` 是 `make_record` 规范化结果，显式提供的认证字段只供后台加密保存，不应原样发送到 IPC。
- `SubscriptionError`：固定安全文案；`SubscriptionCancelled` 是其子类。异常不会回显原始 URL、查询令牌、响应正文或第三方异常字符串。

## 网络约束

仅 HTTPS；拒绝 userinfo、fragment、控制字符、本机地址、私网、链路本地、多播及保留地址。域名解析结果只要包含一个非公网地址即拒绝。解析后直接连接已验证的数值 IP，并使用原主机名作为 `Host`、TLS SNI 和证书匹配名；不会在建立 HTTPS 连接时再次解析该域名。优先 IPv4；只连接本次选中的一个公网地址，不自动重试其他地址。

使用 Requests `trust_env=False`、`verify=True`、空代理字典、零重试、禁止重定向。连接与读取分别限制 5 秒，调用者等待总时限为 20 秒；单独 URL/DNS 校验等待上限为 5 秒。最多允许两个一次性后台工作器，取消或超时立即停止调用者等待，旧工作器在下一 I/O 边界退出，退出前继续占用并发槽，避免反复取消产生无限后台请求。操作系统 DNS 或慢速对端可能延迟该工作器退出；工作器为 daemon，不阻塞应用进程退出。这里没有周期性调度线程。

主动请求 `Accept-Encoding: identity`，拒绝服务器仍返回的压缩编码；读取前检查 Content-Length，流式读取再校验总量不超过 2 MiB，不解压未知压缩正文。最多接受 5000 条输入记录，超过限额报错而不截断。文本仅按 UTF-8 解码，不执行远端内容。

## 候选解析

支持 Geonode 的 `{data:[{ip,port,protocols:[...]}]}`、标准代理 JSON 数组以及逐行代理 URL / `IP:端口`。Geonode 优先选 SOCKS5（规范化为 `socks5h`），否则将 `http` / `https` 标签作为 HTTP CONNECT 候选；不支持 SOCKS4。Geonode 公共列表不推断认证字段，不信任 `status`、`upTime` 等供应商健康声明。标准格式仅使用明确的 `username / password` 字段或代理 URL 中的认证。

相同规范化 endpoint 首项生效，后项计入 duplicates。非法条目或不支持的协议计入 rejected；非空但完全无法解析的正文报错，不能被当作成功的空快照覆盖历史列表。明确的空 JSON 数组是合法的零候选结果，其应用策略由调用方决定。

## 验证记录

离线测试位于 `tests/test_proxy_subscription.py`；所有 Session 与 DNS 均使用合成响应。覆盖公网/内网混合 DNS、连接 IP 固定与 TLS 主机名、URL 脱敏、重定向、超限、取消、异常去敏、IPv6、协议选择与去重。

一次真实 Geonode 请求报告：`build/geonode-subscription-live-check.json`。2026-10-15 请求 page=1、limit=500，230,662 字节，返回字段为 `data / limit / page / total`；得到 440 个支持的候选（278 HTTP CONNECT、162 SOCKS5h），60 项被拒绝。未进行这些代理的健康检查，不能由此推断有 440 个可用出口。

原始设计依据为 [urllib3 自定义 SNI 文档](https://urllib3.readthedocs.io/en/stable/advanced-usage.html#custom-sni-hostname) 与 [Requests Transport Adapters](https://requests.readthedocs.io/en/latest/user/advanced/#transport-adapters)，并核对了项目锁定版本 Requests 2.34.2 / urllib3 2.8.0 的连接池接口。
