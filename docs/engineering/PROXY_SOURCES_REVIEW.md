# 三个新增代理来源与认证代理评估

2026-10-14，氢看直播。以下为来源核查与独立实测，未切换应用当前内置来源、未安装第三方代理服务，也未更改用户任务。

## 结论

**优先考虑 ProxyHub；ProxyClean 可作后续 SOCKS5 备用源；haipproxy 不适合直接作为当前桌面应用的现成 IP 来源。** 前两者均找到真正收到直播互动消息的公网代理。短时、小样本结果不能保证节点长期存活，也不能证明有足够出口支撑 200–300 路并发。

| 来源 | 实际提供的东西 | 本机结果 | 接入现有应用的差距 |
| --- | --- | --- | --- |
| [ProxyHub 中国 HTTP 列表](https://proxyhub.me/zh/cn-http-proxy-list.html) | 公开网页代理列表 | 首屏 20 个中国 HTTP 地址：5 个完整抖音首页通过、4 个生产房间接口通过；其中 2 个完成真实 WSS 内容接收 | 传输协议兼容现有 HTTP CONNECT；仍需实现网页来源解析、分页、去重和调度。当前只做固定地址诊断，未替换自动来源 |
| [ProxyClean](https://github.com/HankNovic/ProxyClean) | 持续更新的 SOCKS5 文本列表 | 此次文件 100 个唯一地址，抽测前 30 个，1 个完整首页通过；该地址通过独立 SOCKS5 适配器实际收到了互动内容 | 新增源码已支持 HTTP/WS 的 SOCKS5 认证与远端 DNS，并通过离线传输测试；该来源的自动列表获取仍未接入，原真实结果来自诊断适配器 |
| [haipproxy](https://spiderclub.github.io/haipproxy/) | 自建采集、校验、调度和分发框架 | 未找到官方托管列表或公共 endpoint，因此没有可直接取用的候选，本轮未部署 | 需要 Redis、Scrapy、调度器等服务，还要更新旧依赖、抓取规则和抖音专用验证 |

## ProxyHub：当前最有实际依据的候选

22:54:46–22:55:02 直接读取用户提供的中国 HTTP 页面和相关中国 HTTPS 页面，页面列出的检测时间为 22:50。网页搜索缓存较旧，因此实测以此次直接下载的 HTML 为准；原始 HTML 和提取结果已保留。

中国 HTTPS 筛选当时显示“没有完全匹配”，并回退到其他国家/协议的通用列表。本次没有把这些回退地址误当中国 HTTPS 代理，也未纳入样本。实际测试的 20 个地址全部来自中国 HTTP 页。

“HTTP 代理”仍可通过 CONNECT 承载严格 TLS 的 HTTPS / WSS。先测试完整 HTTPS 首页，再使用同一代理调用生产房间查询；随后保持出口一致验证真实消息。参考 [urllib3 官方说明](https://urllib3.readthedocs.io/en/stable/advanced-usage.html#http-and-https-proxies)。

两枚代理的首轮联测：

| 固定公网代理 | 时长 | 入库内容 | 路由证明 |
| --- | ---: | --- | --- |
| `119.188.131.55:17981` | 45 秒 | 5 条：3 点赞、2 进入 | WS socket 对端精确等于该代理 IP/端口 |
| `111.206.4.163:18888` | 45 秒 | 4 条：2 点赞、2 进入 | WS socket 对端精确等于该代理 IP/端口 |

HTTP 首页、enter API 和直播间页面也均经过各自固定代理。两轮 TLSv1.3、证书验证开启、没有用户登录 Cookie。归档、回调与计数一致，停止后无新增，工作线程结束。首轮 45 秒均未收到 chat 类型，不将点赞或进入事件称作聊天弹幕。

随后对 `119.188.131.55:17981` 追加 **90 秒**观察，共归档 **17 条：10 点赞、6 进入、1 社交，0 条 chat**。HTTP 和 WS 仍使用同一公网代理，归档/回调/快照一致，停止后无新增。报告为 `.artifacts/proxyhub-live-chat-confirm.json`。观察时间加长后仍未见聊天事件，不能把这次结果写成已经验证聊天弹幕接收。

## ProxyClean：有可用节点，但需 SOCKS5 传输支持

检查固定提交 [3715089](https://github.com/HankNovic/ProxyClean/commit/3715089a41e1be3b1cdc72307b717c08520a49dd)，数据提交时间为当日 18:46:32。公开树中是数据、README、许可和 API 文档，没有可供部署的完整采集器代码；API 文档也没有直接公布托管域名和自助 Key。现成入口是 [SOCKS5.txt](https://raw.githubusercontent.com/HankNovic/ProxyClean/refs/heads/main/SOCKS5.txt)，实际消费时需处理协议前缀及行尾国家注释。

项目自述验证是 SOCKS5 握手，以及 CONNECT `1.1.1.1:53` / `8.8.8.8:53`，不是抖音检测。此次 30 个样本使用 Windows curl 原生 SOCKS5h、严格证书验证、完整响应检查：1 个首页通过，17 个超时，9 个不受信任证书错误，2 个 TLS 握手失败，1 个代理连接关闭。固定顺序样本不是随机抽样，不推算全库成功率。

唯一通过地址 `175.161.96.22:666` 随后在 22:58:47–22:59:32 接受 45 秒真实连接验证：**7 条内容（3 点赞、4 进入），无聊天弹幕**。独立诊断程序将本机 HTTP CONNECT 转为该固定 SOCKS5 上游，使用远端 DNS，不解密 TLS，不允许直连回退。4 条隧道的实际上游 socket 均为该地址，SOCKS 回复成功；WS 下行 53,727 字节、上行 16,049 字节。目标 TLSv1.3，证书校验保持开启。监听、线程与连接均已清理。

这证明该 SOCKS5 公网节点在测试时能够传递本应用的消息协议；该轮适配器仅存在于诊断脚本。后续源码新增了原生 SOCKS5 认证和远端 DNS 支持、并通过离线路由测试，但此真实结果不能追溯当成新原生传输或认证账号的验收，也不表示应用已自动接入 ProxyClean。

## haipproxy：可以参考设计，不能直接获取现成 IP

[官方仓库](https://github.com/SpiderClub/haipproxy)默认分支最后提交为 [2019-07-16 的 ab30ccf4](https://github.com/SpiderClub/haipproxy/commit/ab30ccf4b1d78e9304c27830006cc5800fe41bb3)，release v0.1 发布于 2018 年。当前依赖锁定 Scrapy 1.5.0、Twisted 17.9.0、requests 2.13.0 等，采集规则也较旧。本次只读审阅了 23 个固定提交文件，没有执行其代码或安装 Redis/Splash/Squid。

README 的“不是 API”说法与较后源码存在差异：源码确有本机 Flask API，但数据仍来自自行部署的 Redis，不是官方公共服务。站点队列与校验器主要针对 httpbin、微博、知乎，未找到抖音房间/WSS 验证。其按目标站点维护质量的思路可参考；整个旧框架目前不适合直接嵌入本桌面应用。

## 用户提供的 33 枚认证 SOCKS5

该批 JSON 明确标注 SOCKS5，测试按该协议进行，未先猜测 HTTP。凭据只在检测进程内使用，没有放入命令行参数、报告或额外凭据文件，也没有修改用户代理配置。初筛并发上限 4、每节点总时限 20 秒、目标响应体最多读取 4,096 字节，并设可测流量上限；只有初筛成功才会最多选择两个继续 metadata 和 45 秒消息测试。

实际结果为 **30 枚 SOCKS 用户名密码认证拒绝、3 枚超时，首页成功 0**，因此没有执行后续房间或 WSS 验证。逐条内存一致性检查确认用户名、密码和端口未被改写，RFC1929 长度和字段编码正确。另用原生 curl 只复核一枚，仍返回 SOCKS5 认证拒绝（退出码 97、HTTP 状态 0、目标响应体 0 字节）。尚未到目标 TLS 阶段，不能用 curl 的 `ssl_verify_result=0` 宣称证书握手完成。

认证拒绝发生在抖音请求之前，不能据此判断直播下播或抖音风控，也不能区分账号、密码、套餐、授权范围等具体原因。该批可测 SOCKS TCP 载荷合计 **900 字节**，没有完整目标 HTTP 或 WS 流量；此数不含 IP/TCP 开销、重传、未读内核缓冲和独立 curl 对照握手，不能当供应商流量账单。所有临时监听、线程和连接已清理。证据为 `.artifacts/private-proxy-batch-report.json`、`private-proxy-auth-consistency.json`。

应用新增「代理池 → 导入代理」入口，支持 JSON/文本/表格、HTTP CONNECT 与 SOCKS5 认证。账号密码由当前 Windows 用户 DPAPI 加密保存，预览只显示是否配置认证；可先选择「已导入代理」检测，再启用。这里的功能实现与上述真实凭据失败分别记录，**尚无该批私密代理成功接收 WSS 内容的证据**。新增网页/公共列表仍未自动集成。

## 测试边界与原始证据

- 所有消息测试使用公开正在直播的 `145926919737`，临时数据目录，严格 TLS，无用户登录 Cookie；没有启动用户配置中的监控任务。
- 固定代理通过诊断工厂注入生产 HTTP/WS Fetcher；管理器快照可能仍显示 direct，这是未启用自动池的元信息。报告中的实际 socket、CONNECT 和上游协议证据用于证明真实网络路径，不能仅靠快照文字判断。
- 这次未切换应用自动来源；此前 scdn 的 700 个候选失败结果仍成立，但不能据此推断其他公共来源也不可用。
- 源码工作区 `.artifacts/` 中的证据：`proxyhub-review.json`、`proxyhub-live-119.json`、`proxyhub-live-111.json`、`proxyhub-live-chat-confirm.json`、`proxyclean-review.json`、`proxyclean-live-socks5.json`、`haipproxy-review.md` / `.json`、`private-proxy-batch-report.json`、`private-proxy-auth-consistency.json`。安装包附本摘要，不附原始临时记录或用户附件。
