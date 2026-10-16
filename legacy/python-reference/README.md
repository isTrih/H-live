# 0.1.1 Python 行为参考

这三个文件是 0.1.1 的冻结实现，用于原生迁移的差分验证和可复现性能比较，不属于当前应用运行路径，也不会打包。

- `liveMan.py`：旧版协议解码及连接适配器。
- `archive_store.py`：旧版 SQLite 批量归档。
- `message_export.py`：旧版 CSV / XLSX 流式导出。

它们依赖 `python/collector` 内保留的存储配置和测试协议类型。运行参考实现时必须使用独立的合成数据目录，不能导入用户配置或启用实际直播连接。

生产实现位于 `crates/qingkan-core`，通过 `crates/qingkan-native` 和 `python/collector` 的小型适配器接入。参考文件不再接受功能修改。
