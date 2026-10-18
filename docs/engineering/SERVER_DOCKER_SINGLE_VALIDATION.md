# Docker 单容器验证 · 2026-10-18

默认部署改为 Docker Engine 的一个应用容器，保留既有 Rust 镜像、UI、SQLite schema 和桌面渠道；不需要 Compose、Podman、数据库容器或初始化容器。后续 PostgreSQL 是尚未实现的可选外部存储后端。

## 来源与产物

- [成功 Linux CI](https://github.com/isTrih/H-live/actions/runs/39120659267)，部署源码 `70e59890b8d034dd1553b246513508eaa261d9f5`。
- 环境：GitHub 临时 Ubuntu 24.04 amd64 VM、Docker Engine。测试数据均为合成配置，不开始真实直播间监控。
- 原发行镜像 `h-live-server-0.1.0-linux-amd64.docker.tar.gz`，SHA256 `02181250d1fda167368df810fb629d54453d33fd5f15b0418904c2044300cd05`。
- 新增 `h-live-server-0.1.0-docker-single.tar.gz`，SHA256 `1d6c71980e54d11d66843607ed92cd87fc3a63adb3a96a01ec6d14c513245190`。
- `DOCKER-SINGLE-SHA256SUMS` 覆盖部署包和 `docker-single-validation.json`；下载时再次核验 CI ZIP 与内部清单。
- [部署说明](../server/DOCKER_SINGLE.md)、[发布页](https://github.com/isTrih/H-live/releases/tag/server-v0.1.0)。原镜像、原发行附件及桌面最新版本保持不变。

## 已通过

1. 三份 shell 脚本语法与 ShellCheck，下载原发行镜像并校验后加载。
2. `run.sh` 一次 `docker run` 创建唯一运行容器，没有初始化容器；容器 UID 10001、Rust 为 PID 1，根文件系统拒绝写入，没有 Python/Python3 可执行程序。
3. 同一容器启动时初始化密钥，网页和健康检查正常，未认证请求返回 401，首次主播列表为空。
4. 导入一个合成主播配置但不启动监控，确认 `messages.sqlite3` 已在持久化数据目录创建，并写入独立卷探针文件。
5. SIGTERM 正常停止退出码为 0，日志包含 `All monitoring workers stopped and archives flushed`。
6. 移除并重建应用容器，重新挂载同一对 named volumes；数据库文件和卷探针保留，主播配置恢复且保持手动停止，两个密钥的 SHA256 均不变。
7. 模拟主密钥缺失，再重建容器，启动逻辑拒绝生成替代密钥。

数据卷与容器可写层独立。镜像内 `/bin/sh` 仅负责启动阶段调用已有 Rust `init-secrets` 命令，然后 `exec` Rust 主服务。脚本只创建一个应用容器。密钥卷为读写以支持自动初始化，权限仍为目录 0700 / 文件 0600；不声称只读密钥挂载。

## 未覆盖

未部署到用户服务器，未进行真实房间或 24 小时在线验收；本轮未重启整台宿主机或 Docker daemon，也未注入故障验证 Docker 重启策略，`unless-stopped` 依据 Docker 文档配置。`unhealthy` 本身不会触发 Docker 重启。SQLite 的内容质量、协议、导出与采集正确性沿用现有 [服务端应用验证](SERVER_VALIDATION_0.1.0.md)，本次只验证部署和持久化行为。

外部 PostgreSQL 仍需单独开发存储适配、迁移与回归，不提供尚未实现的数据库连接串设置。当前 SQLite 单机单实例，不允许两个应用容器同时写同一份数据库。

官方依据：[Docker volumes](https://docs.docker.com/engine/storage/volumes/)、[Docker restart policies](https://docs.docker.com/engine/containers/start-containers-automatically/)。
