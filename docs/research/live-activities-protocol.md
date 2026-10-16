# 红包与福袋协议调查（2026-10-15）

## 结论与真实验证边界

0.2.1 新增统一消息类型 `activity`，中文标签“红包/福袋”。目前支持以下八个明确方法的字段投影；本次指定房间的网络探测未能取得可用房间信息，**没有进入 WSS，没有收到真实活动消息**。离线测试使用依据固定协议定义构造的合成 protobuf，不是实收样本，也不证明每种玩法都会向免登录网页连接推送这些消息。

| 方法 | 分类 / 通知阶段 | 读取的详情 |
| --- | --- | --- |
| WebcastLotteryEventMessage | lottery / notice | 活动 ID、原始状态码、开始/开奖/服务器时间 |
| WebcastLotteryEventNewMessage | lottery / notice | 上述字段，以及奖品名称/说明/数量、中奖名额、条件原码及详细条件 |
| WebcastLotteryDrawResultEventMessage | lottery / draw_result | 活动 ID；不读取中奖观众列表和任意 extra |
| WebcastLuckyBoxMessage | red_packet / notice | 红包 ID、标题、钻石数量、发送时间、原始状态、公开描述、明确发送者 |
| WebcastLuckyBoxEndMessage | red_packet / ended | 红包 ID |
| WebcastLuckyBoxRewardMessage | red_packet / reward | 红包 ID、奖励消息类型原码；不读取获奖人/获奖详情 |
| WebcastLuckyBoxTempStatusMessage | red_packet / status | 仅 Common；不存在已定义的红包 ID/奖品详情 |
| WebcastLuckyMoneyMessage | red_packet / notice | 红包 ID、钻石数量、明确发送者 |

普通聊天提到“红包”或“福袋”仍是聊天，不按关键词伪造活动。未验证的其它活动方法仍忽略；没有调用参与、领取、送礼或抽奖接口，没有播放或录制音视频。

## 字段来源和许可

主要字段来源为 [jwwsjlm/douyinlive-proto 的固定提交](https://github.com/jwwsjlm/douyinlive-proto/tree/0d661be57c8f80947054b2ef0cd1ac2fdce3cc4b)，提交通过只读 `git ls-remote` 于本次调查核实。API 查询遇限流，改用公开 Git 引用查询，未使用登录令牌。

- [new_douyin.proto 固定定义](https://github.com/jwwsjlm/douyinlive-proto/blob/0d661be57c8f80947054b2ef0cd1ac2fdce3cc4b/protobuf/new_douyin.proto)：Common 第 102 行；LotteryEvent/Condition/EventNew/DrawResult 第 6366–6433 行；LuckyBox/End/Reward/TempStatus/LuckyMoney 第 6445–6547 行。`Data.LotteryCondition` 与 `Im.LotteryCondition` 的标签不同，本实现使用 **Im 的 1–5 标签**，不混用 Data 定义。
- [MIT 许可证原文](https://github.com/jwwsjlm/douyinlive-proto/blob/0d661be57c8f80947054b2ef0cd1ac2fdce3cc4b/LICENSE)已保存在 `docs/research/licenses/douyinlive-proto-MIT.txt`，随发行保留。许可证 SHA256：`B45CAE8CBF4B812BBE6078026FA73FDA54AF228667AA2DF423D0BE326918AE62`。
- [opedium 的 payload 清单](https://github.com/opedium/douyin-live-proto/blob/master/docs/complete_payload_reference.md)及 [Remember-the-past 的方法映射](https://github.com/Remember-the-past/douyin_proto/blob/main/method%E5%AF%B9%E5%BA%94proto%E5%85%B3%E7%B3%BB.md)能交叉核对部分方法名称，但前者这些活动大多只有 Common、没有采样文件，不能作为详细字段或实收证明。

未执行第三方代码或脚本。独立核对副本在 `build/diagnostics/activity-protocol-source/new_douyin.proto`，SHA256 为 `D00E9D8DD59DF4FC8925172D2A9D5FA9A129A96EE567BDFF2E4A9E74DEEFA2EF`，不作为运行时依赖。

## 事件契约

事件为 `{type: "activity", payload: {...}}`。标准 `sender_id/sender_name/content/message_id/room_id/event_time/time_str/text` 沿用原解码规则；明确 ID 保持字符串，无发送者则空，不使用主播昵称填充。时间只用服务端明确字段，缺失为 0，归档发送时间留空。

新增字段：

- `activity_kind`: `red_packet` 或 `lottery`。
- `activity_stage`: `notice/draw_result/ended/reward/status`，由方法名确定。**不把未定义的数字状态码猜成进行中/已开奖。**
- `activity_method`: 原方法名；`activity_id`: 字符串，未知为空。
- `activity_details`: 各方法已定义的详情；数量使用十进制字符串或 null，防止跨语言损失精度。
- `activity_summary_truncated`: 摘要超过 16000 字符时为 true；避免七列表格单元格无界增长。详情文本单项最多 1024 字符，最多 100 条条件/说明；超量字段不会无限展开。

福袋详情包括 `status_code/start_at/draw_at/server_time/prize_type/prize_name/prize_description/prize_count/winner_count/condition_codes/conditions`。详细条件含 `type/content/remarks/need_diamond_count/min_fans_level`。Unix 时间按现有秒/毫秒规范化，开奖时间写入完整上海时区摘要，不使用接收时间伪造。

红包详情包括 `title/diamond_count/status_code/box_type/sent_at/descriptions` 及明确 BoxMeta 描述。`delay_time` 的来源未注明单位，故只存 `delay_time_raw`，**不据此计算开奖时刻或倒计时**；钻石数量不换算人民币。`box_id_str` 优先于整数 box ID；campaign ID 单独保留，避免混作同一红包 ID。

未知字段、链接、任意业务 JSON、中奖用户 ID/openid/获奖明细均不额外采集。活动内容进入消息流、消息计数和 SQLite `payload_json`；七列 CSV/XLSX 格式不变，类型为“红包/福袋”，详情摘要在内容列。正常 ACK、心跳、控制结束屏障和坏单消息隔离规则保持。

## 本次有界网络探测

脚本 `scripts/diagnostics/probe_live_activities.py` 对 `100000007000` 只做两条已授权路线的一次尝试，不自动重试、不读用户 Cookie、不接触用户配置或数据库。通过本机仅 CONNECT 的预算转发器计量 TLS 密文字节，不解密 TLS，目标白名单仅 live.douyin.com 与现有 webcast WSS 域名；严格校验证书。

| 证据 | UTC 时间 | 结果 | 实测加密流量 |
| --- | --- | --- | --- |
| `build/diagnostics/activities-probe-direct.json` | 07:27:43–07:27:44 | 同直连出口：visitor 200、room_page 200，生产提取未得到可用 room ID，WSS 未启动 | 326147 字节 |
| `build/diagnostics/activities-probe-orem.json` | 07:29:11–07:29:16 | 已授权 Orem：上游 CONNECT 200、visitor 200、room_info 200，生产元数据解析未返回可用对象，WSS 未启动 | 188510 字节 |

第二次上游 socket peer 与配置的代理端点一致。两次 worker、监听、连接及 handler 均确认退出，总计 514657 字节，均未触及单次 16 MiB 上限。HTTP 200 不代表直播间元数据、WSS 或活动成功。报告只保存安全阶段/状态、方法计数，活动样本数均为 0，没有保存其它聊天原文。

## 离线验证

- `crates/qingkan-core/tests/activity_protocol.rs`：9 项，覆盖八方法、长 ID、空发送者/时间、明确零值、原始状态码、开奖秒/毫秒、条件字段、未知字段不误投影、受限重复/摘要、ACK/坏消息/结束屏障及不采集中奖人列表。独立复核发现并修复“忽略的中奖人列表超过 256 项导致结果通知也丢失”：限额仅针对会物化的条件/描述，忽略字段不分配、不按人数截断整个通知；包含测试先失败再通过的记录。
- `native_tests/activity_fixtures.py` 明确标为合成数据；`native_tests/test_activities.py` 验证实际 PyO3 压缩帧入口。
- `tests/test_activity_pipeline.py` 使用禁网护栏和临时数据库验证真实生产回调、manager、去重、场次隔离、SQLite payload 和停止屏障；没有真实用户数据写入。
- 归档 schema v1→v2 迁移和 CSV/XLSX 类型由归档专项测试另外覆盖。最终测试计数以发行验证记录为准。
