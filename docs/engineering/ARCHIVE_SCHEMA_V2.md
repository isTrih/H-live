# 活动消息与 SQLite schema v2

0.2.1 将 `messages.message_type` 的允许值从七类扩展为八类，新增 `activity`，界面和导出名称为「红包/福袋」。`content` 保存事实摘要；活动种类、活动 ID 和已核实的细节保存在原有 `payload_json`，不新增推测字段。未知发送人 ID、名字及服务端时间仍为空。导出保持七列，通过「消息内容」呈现摘要；结构化详情保留在数据库中。

同版新增独立 `session_monitor_durations` 表，按真实主播 ID 与场次 ID 保存已累计监控秒数。它没有指向内容表的外键，因此没有聊天时也可保存计时，清理聊天也不清理计时。`last_checkpoint_at` 仅用于诊断，不用来补算离线时间；保存只允许非负有限值，迟到检查点不能使累计秒数倒退。尚未发行的早期工作区 v2 若缺少该表，会在独立事务中补建；原消息表不再次迁移。

## 升级边界

- 新建内容库直接使用 `PRAGMA user_version=2`。旧 v1 库第一次由 0.2.1 打开时迁移，v2 再次打开不重复修改结构。
- 保留消息 ID、全部字段值、主播与空场次、去重索引、月份索引、外键和 `sqlite_sequence` 自增高水位。历史最大 ID 即使已删除，也不因迁移而重新使用。
- 0.2.0 及更早的应用拒绝打开已迁移的 v2 内容库。不能只换回旧安装器来降级；如需回退，应在升级前关闭应用、等待后台退出并备份完整数据目录，再使用与旧版本匹配的备份。
- 升级会复制一次消息表，需要相应的临时磁盘空间及处理时间。磁盘/约束/锁等错误会让打开失败并保留原库，不忽略错误、清空数据或自动建立替代空库。未来 schema 版本及未知 v1 扩展列明确拒绝。

## 事务和取消

使用 SQLite 官方的建新表、复制、删除旧表、重命名流程，所有数据、索引与版本更新均在一个 `BEGIN IMMEDIATE` 事务内。外键暂时在该专用连接上关闭，避免旧表删除触发入站级联；提交前执行 `foreign_key_check`，完成或失败后恢复外键检查。视图和触发器保存原 SQL 并在事务中重建，复制数据时不重放业务触发器。[SQLite ALTER TABLE 文档](https://www.sqlite.org/lang_altertable.html#making_other_kinds_of_table_schema_changes)

迁移保留 `sqlite_sequence` 的原始值或原本没有记录的状态，避免仅复制存活行导致自增序号回退。[SQLite AUTOINCREMENT 文档](https://www.sqlite.org/autoinc.html)

Rust 的 `open_cancellable` 及导出路径可以在 SQL 执行边界观察取消。取消或失败后先停用 SQLite progress handler，再确保回滚完成。取消以事务实际观察到为准；已提交的迁移不撤回。普通后台启动构造目前不接用户取消令牌，不能宣称点击关窗即中断正在进行的启动迁移。

既有归档删除门禁、写入屏障和删除取消行为保持不变。活动类型参与同样的月份/主播/场次交集、快照、去重及删除；此次迁移不提供自动删除操作。

## 验证

`crates/qingkan-core/tests/archive_migration.rs` 使用独立保存的历史 v1 SQL，覆盖有数据/空库、新建、重复打开、跨月、ID/索引/触发器/视图/外键/自增序号、复制中取消、约束与外键错误回滚、未来版本及未知扩展列。`archive_activity.rs` 与 `tests/test_activity_archive.py` 验证活动详情、空发送人、CSV/XLSX、去重和范围删除；`test_native_archive_compat.py` 验证旧 Python 版生成的库向前迁移和旧应用的拒绝降级行为。`monitor_duration.rs` 与 `test_monitor_duration_storage.py` 验证时长检查点的幂等、乱序并发、正常重启、非法输入、无消息也持久化和清理聊天后保留。所有数据均为工作区临时合成记录，不读取或改写用户真实数据库。
