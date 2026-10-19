# Docker 单容器部署

默认方案采用 **Docker Engine + 一个应用容器**，使用现有 server 0.1.1 镜像。网页、Salvo API、采集、SQLite、归档分析和导出都由同一个 Rust 服务承载。无需 Compose、Podman、独立网页容器或初始化容器。

默认数据库为 SQLite，`messages.sqlite3` 及其 WAL、任务状态、导出文件保存在**容器外的 Docker 持久化卷**中。删除/替换应用容器不删除该卷。SQLite 是文件数据库，没有独立 SQLite 服务容器。未来可以增加连接外部 PostgreSQL 的可选存储后端；**当前尚未实现 PostgreSQL**，不是可直接填写连接串启用的功能。

## 启动

需要 Linux amd64、Docker Engine 和 Bash，Docker 已配置开机启动。当前 Windows 开发主机无需安装 Docker。服务端镜像已含网页和 Rust 程序，运行时不需要 Rust、Node 或 Python。

从 [server-v0.1.1 发布页](https://github.com/isTrih/H-live/releases/tag/server-v0.1.1) 下载：

- `h-live-server-0.1.1-linux-amd64.docker.tar.gz`：本版 Linux amd64 镜像。
- `h-live-server-0.1.1-docker-single.tar.gz`：本版单容器部署脚本。
- `SHA256SUMS`：镜像、部署脚本、构建信息和验收报告的统一校验清单。

校验对应文件的 SHA256 后执行：

```sh
docker load -i h-live-server-0.1.1-linux-amd64.docker.tar.gz
tar -xzf h-live-server-0.1.1-docker-single.tar.gz
bash packaging/server/docker-single/run.sh
```

`run.sh` 执行一次 `docker run`，创建一个容器 `h-live`。容器首次启动时自动初始化两份独立随机密钥，然后 `exec` 为 Rust 服务，使 Rust 成为 PID 1。初始化与常驻服务使用**同一个容器、同一 UID 10001**，不额外创建初始化容器。后续启动校验已有密钥，不覆盖；已有数据但缺密钥时拒绝重建密钥。

默认发布 `127.0.0.1:8080`，本机访问 `http://localhost:8080`；远程服务器先使用 SSH 隧道：

```sh
ssh -L 8080:127.0.0.1:8080 user@your-server
```

管理员在服务器终端读取管理令牌填入网页登录框，不要将令牌提交到仓库或发给别人：

```sh
docker exec h-live cat /run/h-live-secrets/admin-token
```

同名容器已存在时脚本拒绝覆盖，使用 `docker start h-live` 恢复原容器。脚本不会删除已有容器或持久化卷。

## 持久化与安全

| Docker 卷 | 容器内位置 | 内容 |
| --- | --- | --- |
| `h-live-data` | `/var/lib/h-live` | SQLite、WAL、日志、任务、导出 |
| `h-live-secrets` | `/run/h-live-secrets` | `master.key`、`admin-token` |

这两个是宿主 Docker 管理的 named volumes，和容器可写层分离；可用 `docker volume inspect h-live-data` 查询宿主挂载位置。镜像首次拷贝卷目录的 UID/GID 10001，避免使用 root 初始化。不要删除卷，也不要在运行中单独拷贝数据库文件。

根文件系统只读，去掉全部 capabilities，禁止获取新权限；内存上限 2 GiB、进程数上限 128，`/tmp` 为 128 MiB tmpfs，运行日志按 10 MiB × 3 轮转。数据卷读写；**密钥卷也需要读写，以支持同容器初始化**，目录权限 0700、文件 0600。它不是只读密钥挂载或宿主加密保险箱。代理配置使用该主密钥认证加密，备份时分开保管数据与密钥。

继续不使用抖音登录 Cookie、不新增礼物记录，首次列表为空。管理端认证与同源保护不变。

## 配置

首次创建前可设置：

```sh
H_LIVE_PUBLIC_ORIGIN=https://live.example.com \
  bash packaging/server/docker-single/run.sh
```

公网使用已有 HTTPS 反向代理转发至宿主 `127.0.0.1:8080`；Origin 不带路径。反向代理规则与 API 说明见 [服务端说明](README.md)。不需要增加数据库或反向代理容器才能通过 SSH 隧道使用。

脚本支持 `H_LIVE_CONTAINER_NAME`（默认 `h-live`）、`H_LIVE_PORT`（默认 `8080`）、`H_LIVE_DATA_VOLUME`（默认 `h-live-data`）、`H_LIVE_SECRET_VOLUME`（默认 `h-live-secrets`）。更改宿主端口后要同步 Origin，例如 `H_LIVE_PORT=8081 H_LIVE_PUBLIC_ORIGIN=http://localhost:8081`。名称仅接受普通 Docker 名称，不将路径误当成卷名。

## 停止、备份与升级

```sh
docker ps
docker logs --tail 100 h-live
docker exec h-live h-live-server healthcheck
docker stop --time 180 h-live
docker start h-live
```

使用 `unless-stopped` 策略，在 Docker daemon 恢复或进程意外退出时恢复服务；手动停止后保持停止。Docker 的 `unhealthy` 状态本身不会自动重启容器；应用控制线程退出时，现有 Rust watchdog 会结束服务，交给 Docker 重启。健康检查仅证明管理服务就绪，不保证抖音下发消息。

备份前停止容器，确认日志出现 `All monitoring workers stopped and archives flushed` 且 `docker inspect h-live --format '{{.State.ExitCode}}'` 为 0，再完整备份两个卷；包括数据库 WAL、任务状态和导出，密钥另行保管。备份完成后再启动。不要运行 `docker volume rm`、卷清理命令或 `compose down -v` 删除持久化数据。

替换容器时先备份并停止、移除**应用容器本身**（不删除卷），再用相同的卷名运行部署脚本；已有数据库和密钥继续保留。未来镜像版本升级也使用这一流程，配套使用该版本部署脚本。单机单实例，不能让两个容器同时写同一份 SQLite。

从旧 Compose 迁移时，先停止原服务，查询实际卷名（通常为 `h-live_data`、`h-live_secrets`，可能有自定义项目名前缀），备份后指定这**一对原数据卷与原密钥卷**：

```sh
H_LIVE_DATA_VOLUME=h-live_data H_LIVE_SECRET_VOLUME=h-live_secrets \
  bash packaging/server/docker-single/run.sh
```

不要只挂载旧数据卷却生成新密钥。桌面历史迁移和平台限制沿用服务端说明。尚未在用户服务器完成真实房间连续 24 小时验收。
