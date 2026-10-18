# 文档范围

`user/` 是使用说明；`release/` 是构建和更新渠道说明；`engineering/` 中带版本号或日期的文件属于历史设计与验收笔记；`research/` 记录固定来源的调研。

公开副本中的直播间、主播示例 ID 已匿名化，示例不是推荐监控对象。历史笔记提及的 `build/`、诊断、日志和真实抓包不随源码公开；不能把这些链接当作本仓库附带的可复现实测证据。活动二进制测试夹具为合成数据，可通过 Rust `qingkan-core` 的 `generate_activity_fixtures` example 重建。

当前产品范围以根目录 README 为准：0.3.0 继续停用新增礼物采集，仅保留历史记录。旧版本文档提及的礼物采集、登录排查或下一步账号对照不属于现行需求。

0.3.0 已将采集、签名、代理和调度迁移至 Rust，见 [迁移计划与结果](engineering/RUST_MIGRATION.md)。0.2.x 文档中的 Python/V8 描述仅适用于历史版本。

Rust + Salvo 独立服务端 0.1.0 已完成 Linux amd64 构建与容器验收；默认 [Docker Engine 单容器](server/DOCKER_SINGLE.md)，SQLite 文件存于容器外持久化卷，外部 PostgreSQL 是后续计划。保留早期 Compose 及可选 Podman 部署。尚未部署到用户服务器。当前功能和步骤见 [服务端说明](server/README.md)，证据与边界见 [应用验证](engineering/SERVER_VALIDATION_0.1.0.md) 与 [Podman 实机验证](engineering/SERVER_PODMAN_VALIDATION.md)，设计依据见 [实施方案](engineering/SERVER_PLAN.md) 与 [官方资料调研](research/salvo-server-feasibility-2026-10-17.md)。
