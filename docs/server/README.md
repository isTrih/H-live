# H-live Server 0.1.0 · Linux 单体容器

**默认部署：[Docker Engine 单容器](DOCKER_SINGLE.md)**。网页、API、采集、SQLite 和导出在一个应用容器中运行，同容器自动初始化，无需 Compose 或额外初始化容器。SQLite 文件默认保存在容器外的持久化卷；PostgreSQL 是后续可选后端计划，当前尚未实现。下文保留早期 Docker Compose 部署说明；[Podman + Quadlet](PODMAN.md) 作为可选方案。

独立 Rust + Salvo 服务进程，复用桌面 0.3.0 的采集与归档核心。浏览器关闭后，服务器仍继续监控。保留看板、实时消息、代理及 HTTPS 订阅、场次分析、有效发言与在线指标、归档删除、CSV/XLSX 导出。桌面版继续保留。

不使用抖音登录 Cookie，不新增礼物记录，不播放或录制音视频。管理端的登录令牌仅用于保护 H-live，与抖音账号无关。首次启动没有内置主播。

## 启动

需要 Linux amd64、Docker Engine 和 Compose v2。建议先准备 2–4 核、4 GB 内存和持久化 SSD，再按实际归档量调整；这是初始配置建议，不是实测容量保证。单机单实例，不要让多个服务同时写同一份 SQLite。

源码构建：

```sh
git clone https://github.com/isTrih/H-live.git
cd H-live
docker compose build server
docker compose --profile tools run --rm init
docker compose up -d --wait server
```

若使用发行包，先校验 SHA256SUMS，加载镜像，再在解压的部署目录运行初始化和启动命令，不需要安装 Rust、Node 或桌面环境：

```sh
docker load -i h-live-server-0.1.0-linux-amd64.docker.tar.gz
tar -xzf h-live-server-0.1.0-deploy.tar.gz
docker compose --profile tools run --rm init
docker compose up -d --wait server
```

初始化只创建缺失的随机主密钥和管理令牌，已有文件不会覆盖。数据与密钥使用两个独立 Docker 卷；主服务只读挂载密钥。容器以 UID/GID 10001 运行，根文件系统只读，去掉 Linux capabilities。

默认只发布 `127.0.0.1:8080`。本机访问 `http://localhost:8080`；远程服务器可以先使用 SSH 隧道：

```sh
ssh -L 8080:127.0.0.1:8080 user@your-server
```

管理员在自己的服务器终端读取管理令牌，填入网页登录框。不要把它发到聊天、工单、Git 仓库或截图中：

```sh
docker compose --profile tools run --rm --entrypoint cat init /run/h-live-secrets/admin-token
```

令牌不会写入浏览器 localStorage，登录后使用 HttpOnly、SameSite=Strict 会话；有效期 12 小时，服务重启或退出管理端会失效。登录每分钟最多尝试 10 次，最多保留 16 个会话。首次页面出现前需登录，全部消息、代理和下载接口都需要认证。

## HTTPS 与配置

对外访问时使用已有的 HTTPS 反向代理，转发到 `127.0.0.1:8080`，并在部署目录的 `.env` 写入管理端的精确 Origin：

```dotenv
H_LIVE_PUBLIC_ORIGIN=https://live.example.com
```

执行 `docker compose up -d --wait server` 应用配置。Origin 不带路径；除 localhost/127.0.0.1 外拒绝 HTTP Origin。不要把默认管理端端口直接开放到公网。反向代理需保留原始 Host/Origin，不缓存 `/api/`，单个请求体最大 1100 KiB，并设置适当的超时。首版不依赖跨域访问，不返回宽松 CORS 响应头。

源码直接运行时可配置：

| 环境变量 | 默认值 | 用途 |
| --- | --- | --- |
| `H_LIVE_BIND` | `0.0.0.0:8080` | 容器内监听地址 |
| `H_LIVE_PUBLIC_ORIGIN` | `http://localhost:8080` | 浏览器管理端唯一 Origin |
| `H_LIVE_DATA_DIR` | `/var/lib/h-live` | 数据、日志、任务状态、导出 |
| `H_LIVE_SECRETS_DIR` | `/run/h-live-secrets` | `master.key` 与 `admin-token` |
| `H_LIVE_UI_DIR` | `/opt/h-live/ui` | 专用网页构建目录 |
| `H_LIVE_HEALTH_ADDR` | `127.0.0.1:8080` | 容器内健康探测地址 |

Compose 的 `H_LIVE_LISTEN_IP`、`H_LIVE_PORT` 仅控制宿主机端口映射。默认内存上限 2 GiB、进程数上限 128、停止宽限期 180 秒。不要随意设置更短的强制停止时间。

## 运行与恢复

在网页添加主播、点击开始。目标任务先写入 `.server/state.json`，再启动采集；异常退出或正常升级后恢复这些任务，手动停止/删除的任务不会自行重新开启。同一真实场次沿用已有累计监控时长，中断间隔不计入。

每个房间仍通过游客 HTTPS 查询状态、WSS 收消息；直连和代理均为出口，可分配多个任务，共享限速与冷却。管理网页每秒轮询有界事件缓存，数据库归档独立于浏览器；服务实例重启后重置事件游标，实时消息通过已归档查询恢复。首版未提供 SSE 或多用户权限体系。

导出由服务器创建随机文件名，只能写入专用 `exports` 目录。网页显示下载链接，保留七天，最多保留 100 条导出记录；这不会自动删除原始聊天历史。服务重启后已完成文件仍可下载，未完成的导出标为中断，需要重新创建。不会接受浏览器指定的服务器文件路径。

```sh
docker compose ps
docker compose logs --tail 100 server
docker compose exec -T server h-live-server healthcheck
docker compose stop server
docker compose up -d --wait server
```

`/healthz` 只表示控制进程就绪，不证明房间正在开播、代理可用或抖音确实下发聊天。看板可观察归档队列、最近消息、每出口负载及冷却、Linux 自身内存/CPU；磁盘余量和宿主机告警请交给服务器监控系统。运行日志轮转，不记录 API 请求体、管理令牌或代理密码。

## 备份和桌面历史迁移

初版采用短暂停机备份，保证 SQLite、WAL、任务状态和导出文件一致。先停止服务，确认日志出现 `All monitoring workers stopped and archives flushed` 且退出码为 0，再对数据卷做整卷备份；完成后启动。不要仅复制正在写入的 `messages.sqlite3`，也不要运行 `docker compose down -v` 删除持久化卷。

密钥卷另行离线备份、独立保管。丢失 `master.key` 后无法解密代理/订阅配置；不要用重新初始化覆盖原密钥。管理令牌遗失时，在停机后通过受控运维流程单独轮换 `admin-token`，保留主密钥不变。新令牌必须是随机 32 字节的 Base64 编码，Unix 权限 0600 或 0400。

桌面历史迁移时，退出原桌面程序，备份后将 `messages.sqlite3` 及存在的 `-wal`/`-shm` 整组复制到停止状态的服务端数据卷，并使其归属 UID/GID 10001。保持同一 schema，不做历史数据清空。主播列表可从 UI 重新导入；Windows DPAPI 代理密文不能在 Linux 解密，需要重新导入代理。不要复制桌面的更新恢复文件、登录字段或整个 AppData 目录。服务端和桌面不能同时打开同一份数据库。

## 升级与限制

服务端通过 Docker 镜像升级，独立使用 `server-v0.1.0` 版本线；不会覆盖桌面 0.3.0 的 CDN 清单或触发 Tauri 热更新。先备份数据和密钥，再停止旧容器、加载新镜像、启动并确认任务恢复。数据库升级后的降级需要恢复配套备份，不能只替换旧镜像。

300 个配置任务、约 50 个同时开播是目标规模。模拟连接测试不能替代真实平台验收；尚未取得实际服务器 24 小时、真实 50 房间同时接收的验证结果。平台 444/429、游客限制、网络中断和代理故障仍会导致缺失，停机期间不能补采，也不会伪造消息。

## API

管理端使用显式领域接口，接口列表见 `crates/h-live-server/src/http.rs` 的 `ENDPOINTS`。认证使用 `Authorization: Bearer <admin-token>`；浏览器则使用同源会话和写操作 CSRF 标头。调用方必须控制令牌保密。JSON 查询和操作均使用 POST，`GET /api/v1/session` 检查会话，`GET /api/v1/files/{id}` 下载已完成导出。

不提供桌面通用 `backend_request`、任意文件读取、任意导出路径、远程关机或执行命令接口。浏览器中的退出仅退出管理端，不停止服务器监控。

本项目仅用于学习研究，严禁用于商业谋利、破坏系统、盗取个人信息等行为。侵权或涉及相关利益请联系原作者。完整声明见项目 `DISCLAIMER.md`。
