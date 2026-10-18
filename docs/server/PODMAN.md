# 单体容器部署 · Podman + Quadlet

当前默认方案已按用户选择改为 [Docker Engine 单容器](DOCKER_SINGLE.md)。本文保留可选 Podman 部署方式。

H-live Server 使用一个 Rust 进程，同时提供网页、Salvo API、采集调度、SQLite 归档和 CSV/XLSX 导出。只有 **一个应用容器**，没有初始化容器，不依赖独立数据库、缓存或网页容器。systemd 属于宿主机服务管理，不是额外容器。

此前 Compose 方案用于统一管理配置和数据卷。此可选方案使用无 root 的 Podman + Quadlet，直接复用 server 0.1.0 已发布镜像；部署方式变化不代表采集协议或性能发生变化。

## 准备与安装

需要 Linux amd64、systemd、cgroup v2、Podman **4.9 或以上**（含 Quadlet）、rootless 所需 uidmap 和网络组件。建议专用普通用户及本地 SSD；该用户需要正确的 `/etc/subuid`、`/etc/subgid` 配置及可用的 systemd 用户会话。安装包不自动修改这些系统设置。当前固定使用标准 `~/.config` 目录。

在 [server-v0.1.0 发布页](https://github.com/isTrih/H-live/releases/tag/server-v0.1.0) 下载镜像、`SHA256SUMS`、`h-live-server-0.1.0-podman.tar.gz` 和 `PODMAN-SHA256SUMS`。核对清单中的镜像及 Podman 包哈希；原 Docker 部署包无需下载。镜像是 Docker archive 格式，Podman 可以直接加载，无需重建镜像。

以下命令使用专用普通用户执行，**不要加 sudo**：

```sh
podman load -i h-live-server-0.1.0-linux-amd64.docker.tar.gz
podman tag docker.io/library/h-live-server:0.1.0 localhost/h-live-server:0.1.0
tar -xzf h-live-server-0.1.0-podman.tar.gz
bash packaging/server/podman/install.sh
systemctl --user start h-live.service
curl --fail http://localhost:8080/healthz
```

初始化脚本直接在宿主机生成两份独立的随机 32 字节 Base64 密钥，不启动任何容器、不输出密钥值。已有密钥和配置不覆盖；数据目录非空且密钥缺失时拒绝重新生成，要求恢复原密钥。修改过的 Quadlet/停止配置也不会被脚本覆盖。

管理员执行一次下列命令，将 `YOUR_DEPLOYMENT_USER` 换成上述专用用户名，使其退出登录和服务器重启后仍能运行：

```sh
sudo loginctl enable-linger YOUR_DEPLOYMENT_USER
```

Quadlet 的 `[Install] WantedBy=default.target` 负责用户服务自启动，无需执行 `systemctl --user enable h-live.service`。检查：`loginctl show-user YOUR_DEPLOYMENT_USER -p Linger` 应为 `yes`。

默认仅绑定 `127.0.0.1:8080`，本机访问 `http://localhost:8080`；远程可用 `ssh -L 8080:127.0.0.1:8080 user@server`。管理员在服务器自己的终端读取 `~/.local/share/h-live/secrets/admin-token`，填入网页登录框。不要把令牌发到聊天或提交仓库。

## 数据、配置与边界

| 宿主机路径 | 内容 |
| --- | --- |
| `~/.local/share/h-live/data/` | SQLite、任务状态、日志、导出文件 |
| `~/.local/share/h-live/secrets/` | `master.key` 和 `admin-token`，文件权限 0600 |
| `~/.config/h-live/server.env` | 管理网页 Origin，默认 `http://localhost:8080` |
| `~/.config/containers/systemd/h-live.container` | 单容器 Quadlet 定义 |
| `~/.config/systemd/user/h-live.service.d/10-graceful-stop.conf` | 180 秒优雅停止覆盖配置 |

`keep-id:uid=10001,gid=10001` 将专用宿主用户映射到容器 UID/GID 10001，数据仍归宿主用户，无需递归 chown。数据目录读写、密钥目录只读，根文件系统只读，仅 `/tmp` 使用有界 tmpfs；capabilities 全部去除，禁止获得新权限。`:Z` 仅为这两个应用专用目录设置私有 SELinux 标签，不要改成挂载整个 HOME。

密钥是受权限保护的宿主文件，**并非宿主机静态加密保险箱**。有该用户或 root 权限仍可读取；代理配置在数据目录中使用独立 master.key 进行认证加密。两类目录应分别备份保管。

内存上限 2 GiB、进程数上限 128。不创建 Pod、不启动 sidecar，不依赖 Docker daemon。网络默认 rootless 隔离，不使用 host network 或 privileged。镜像使用 `Pull=never`，先校验并加载固定版本，避免意外拉取同名镜像。

对外提供 HTTPS 时，复用服务器已有反向代理，并把 `server.env` 中的 `H_LIVE_PUBLIC_ORIGIN` 改成精确的 `https://live.example.com`，不带路径。详细 API、身份认证和平台限制见 [服务端说明](README.md)。

## 运维与恢复

```sh
systemctl --user status h-live.service
journalctl --user -u h-live.service -n 100 --no-pager
podman ps
podman exec h-live h-live-server healthcheck
systemctl --user restart h-live.service
systemctl --user stop h-live.service
```

systemd 自动重启异常退出的容器；健康检查持续失败时先停止容器，再由 systemd 重启。`Restart=always` 也覆盖健康停止的正常退出码，但手动 `systemctl --user stop` 会保持停止。正常停止使用 SIGTERM，最多等待 180 秒完成归档；单独的 systemd drop-in 显式指定 `podman stop`，随后保留 Quadlet 的清理步骤。240 秒 systemd 停止超时为清理留出余量。日志归 journald 管理，宿主机应配置保留期限/磁盘限额。

网页添加并开始监控的任务在服务重启后恢复，手动停止的任务不会自行启动。健康检查只说明控制服务就绪，不保证抖音下发消息。仍不使用抖音登录 Cookie、不新增礼物记录。

备份前 `systemctl --user stop h-live.service`，确认 journal 中出现 `All monitoring workers stopped and archives flushed`，且 `systemctl --user show h-live.service -p ExecMainStatus` 为 0，再完整备份 `data/`（包括 WAL、状态和导出）及单独保存的 `secrets/`。备份后重新启动。不要只复制正在写入的数据库。

从旧 Compose 迁移时，先停止原服务并完整备份，将原数据卷内容和**原密钥卷**分别复制到上述宿主目录，以专用普通用户持有目录、目录权限 0700、密钥 0600。数据目录不要再嵌套一层。确认恢复后再启动新服务；不要让两个实例同时访问同一数据库，不要执行 `down -v` 删除原卷。桌面数据库迁移规则沿用服务端说明。

升级时先备份、停止、校验并加载新镜像，再手动将 Quadlet 的 `Image=` 改为新版本，执行 `systemctl --user daemon-reload` 和 `systemctl --user start h-live.service`。已有密钥不变。不启用无人值守拉取镜像升级，数据库降级需要配套备份。桌面 Tauri 更新渠道不受影响。

真实 24 小时与真实 50 个房间的并发验收仍需在目标服务器进行；更换容器工具不会消除平台限流或游客消息下发限制。当前仍是单机单实例，不支持多个副本共享一份 SQLite。
