# 单体 Podman 部署验收 · 2026-10-17

目标：一个应用容器承载网页、Salvo API、采集、SQLite 归档和导出；不需要 Compose、初始化容器、独立数据库或网页容器。宿主机 systemd 负责生命周期。应用二进制和数据库 schema 未改变，继续复用 server 0.1.0 镜像；桌面版仍为 0.3.0。

## 产物与来源

- [成功的 Podman Linux CI](https://github.com/isTrih/H-live/actions/runs/39009677731)，部署源码 `2d3a9e4119c7828e1cbb959b9953b153e7a6f008`。
- 环境：GitHub 临时 Ubuntu 24.04 amd64 虚拟机，Podman **4.9.3**，systemd 用户服务，cgroup v2，非 root 用户运行 Podman。
- 复用已发布 `h-live-server-0.1.0-linux-amd64.docker.tar.gz`，SHA256 `02181250d1fda167368df810fb629d54453d33fd5f15b0418904c2044300cd05`；验证后 `podman load`，显式设置 `localhost/h-live-server:0.1.0` 本地标签。
- 新增部署包 `h-live-server-0.1.0-podman.tar.gz`，SHA256 `c6902a5785cea0765ff6e9cc8bec6f4c338945ca36575c7c0cb8b5132c7221e4`。
- 附件 `PODMAN-SHA256SUMS` 校验部署包与 `podman-validation.json`；原服务端发行镜像及原校验清单保持不变。
- [下载与发布页](https://github.com/isTrih/H-live/releases/tag/server-v0.1.0)，[部署说明](../server/PODMAN.md)。

## 已通过

1. Bash 语法和 ShellCheck；正式发行镜像校验、加载、离线标签与 `Pull=never`。
2. 宿主机初始化密钥，不启动初始化容器；重复安装两次，密钥文件哈希不变。
3. 实际 systemd 用户 Quadlet 生成与启动；`podman ps` 仅一个名为 `h-live` 的运行容器，容器用户 UID 10001。
4. 根文件系统和密钥挂载拒绝写入；容器没有 Python/Python3 可执行程序。
5. HTTP 健康检查可用，未认证的数据请求返回 401，首次主播列表为空。
6. 导入一个合成主播配置，**不开始真实监控**；`systemctl --user stop` 正常退出码为 0，journal 包含 `All monitoring workers stopped and archives flushed`。
7. 重启后配置保留且任务仍为手动停止状态。
8. 有数据时移走主密钥，安装脚本拒绝重新生成，原密钥恢复后可继续使用。
9. 对该临时容器执行故障 kill，systemd 自动创建新的容器并恢复健康；主播配置保留、手动停止状态保留。测试结束再正常停止。
10. CI 产物 ZIP 摘要和内部 SHA256 清单下载后再次校验通过。

独立的完整服务端 Linux 回归也通过：[Server Linux](https://github.com/isTrih/H-live/actions/runs/39009131807)，源码 `43c8dfd40fe466cb27c096420bd04049ac9be79c`。之后的生产改动只有 Podman CI 测试用户环境调整；Rust、前端和镜像构建源码未变。原应用与容量证据见 [server 0.1.0 验证](SERVER_VALIDATION_0.1.0.md)。

## 验证边界

- 未部署到用户服务器，未进行真实房间消息采集、真实 50 房间并发或 24 小时持续验收。
- 已设置 linger 与 Quadlet 开机目标，但本轮没有重启整台 CI 虚拟机；目标服务器仍需确认开机恢复。
- 验证了故障 kill 后 systemd 自动重启；没有注入 HTTP 卡死来完整等待三次定时健康检查失败。`HealthOnFailure=stop` 和 `Restart=always` 的组合依据固定 Podman/systemd 官方行为。
- 当前为单机单实例 SQLite；不允许多个副本共享写同一数据库，断线期间的消息无法补采。
- 不改动桌面历史、Tauri 标识或 CDN 更新渠道，不新增抖音登录 Cookie 和礼物记录。

CI 早期尝试创建第二个 Linux 测试用户时，宿主 PAM 把 runner 的 XDG 路径带入新用户 manager，导致生成器读错目录。最终测试直接使用临时 VM 原有普通用户和对应会话，保持真实 rootless 验收；没有向部署包加入 CI 环境改写，也没有关闭 AppArmor 或放宽 HOME 权限。官方依据见 [研究记录](../research/podman-single-container-2026-10-17.md)。
