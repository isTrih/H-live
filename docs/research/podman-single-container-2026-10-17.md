# 单体 rootless Podman / Quadlet 核对（2026-10-17）

范围：单个 H-live 容器、Podman ≥ 4.9、Ubuntu 24.04；仅查阅官方文档与固定 **v4.9.3** 源码，不运行容器或部署。Ubuntu Noble 官方包页列出 4.9.3 系列，因此不能把 Podman 5.x 新增 Quadlet key 直接写入最低兼容模板。[Ubuntu 包](https://packages.ubuntu.com/noble/podman)

## 核对结论

| 计划 | 结论 / 必要条件 | 官方依据 |
| --- | --- | --- |
| rootless Quadlet | 支持；用户定义放在 `~/.config/containers/systemd/*.container`，由 `systemctl --user` 管理。必须有 cgroup v2；另需 subordinate UID/GID、user session 与 rootless 网络工具。版本号不代表这些主机条件已满足。 | [4.9.3 Quadlet 文档](https://github.com/containers/podman/blob/v4.9.3/docs/source/markdown/podman-systemd.unit.5.md)、[rootless 教程](https://github.com/containers/podman/blob/v4.9.3/docs/tutorials/rootless_tutorial.md) |
| `UserNS=keep-id:uid=10001,gid=10001` | 4.9.3 已支持。将**运行 Podman 的宿主用户及主组**映射为容器 10001:10001；不是要求宿主真的创建 UID 10001。容器明确 `User=10001`、`Group=10001` 与映射一致。 | [userns 选项](https://github.com/containers/podman/blob/v4.9.3/docs/source/markdown/options/userns.container.md) |
| 宿主 `~/.local/share/h-live/{data,secrets}` | 由该 rootless 用户创建并拥有，目录建议 0700，key 文件 0600；映射后的容器 UID 10001 可按 owner 权限访问。因此**不需要 init container/chown 容器**，也不要把宿主文件 chown 为 10001。已有他人 / root 所有的文件需先修正 owner。 | [rootless 教程](https://github.com/containers/podman/blob/v4.9.3/docs/tutorials/rootless_tutorial.md)、[挂载 ownership](https://github.com/containers/podman/blob/v4.9.3/docs/source/markdown/options/volume.md) |
| data `:Z`、secrets `:ro,Z` | 支持；`ro` 约束容器写入，`Z` 是 SELinux 私有重标记，**不改变 Unix owner / mode**。只标记专用子目录，不能标记整个 home；同一路径不能再给别的容器以独立私有标签共享。即使 ro 挂载，Podman 启动时仍可能修改宿主 SELinux 标签。 | [volume 选项](https://github.com/containers/podman/blob/v4.9.3/docs/source/markdown/options/volume.md) |
| `ReadOnlyTmpfs=false` + 显式 `/tmp` tmpfs | 4.9.3 支持；与 `ReadOnly=true` 搭配会取消默认 `/run`、`/tmp`、`/var/tmp` 可写 tmpfs，且 `/dev`、`/dev/shm` 也只读。再设 `Tmpfs=/tmp:rw,nosuid,nodev,noexec,size=256m,mode=1777` 可只开放有界临时目录。应用若写 `/run` 或依赖可写 `/dev/shm`，需单独适配 / 验证。 | [read-only-tmpfs](https://github.com/containers/podman/blob/v4.9.3/docs/source/markdown/options/read-only-tmpfs.md)、[tmpfs](https://github.com/containers/podman/blob/v4.9.3/docs/source/markdown/options/tmpfs.md) |
| `PodmanArgs=--stop-timeout=180` | 4.9.3 支持；`PodmanArgs` 传入 `podman run`，`--stop-timeout` 写入容器停止宽限。4.9.3 的 supported key 集合**没有 `[Container] StopTimeout=`**，未知 key 会令生成失败。 | [Quadlet 实现](https://github.com/containers/podman/blob/v4.9.3/pkg/systemd/quadlet/quadlet.go)、[stop-timeout](https://github.com/containers/podman/blob/v4.9.3/docs/source/markdown/options/stop-timeout.md) |
| linger + 开机恢复 | `loginctl enable-linger 用户` 使 user manager 开机启动且登出后保留；Quadlet 用 `[Install] WantedBy=default.target`，不能对生成的 service 使用普通 `systemctl enable`。 | [systemd v255 loginctl](https://github.com/systemd/systemd/blob/v255/man/loginctl.xml)、[Quadlet 文档](https://github.com/containers/podman/blob/v4.9.3/docs/source/markdown/podman-systemd.unit.5.md) |

Ubuntu 24.04 使用 AppArmor，并限制非特权 user namespace；`:Z` 不能解决 AppArmor 拒绝。应使用发行版包与适用的 Podman profile；如果安装了自编译到不同路径的 Podman，需要核对允许的 executable 路径及日志。不能仅因返回 permission denied 就关闭全机 AppArmor / userns 限制。[Ubuntu 24.04 官方 release notes](https://documentation.ubuntu.com/release-notes/24.04/)、[Ubuntu AppArmor](https://documentation.ubuntu.com/security/security-features/privilege-restriction/apparmor/)

`keep-id` 在 4.9 使用用户的整段 subuid/subgid 范围，会影响同一用户同时使用 `--userns=auto` 的其他容器；本方案是单容器，但安装说明不应擅自重写已有 `/etc/subuid` / `/etc/subgid`。[userns 选项](https://github.com/containers/podman/blob/v4.9.3/docs/source/markdown/options/userns.container.md)

## HealthOnFailure、退出码与重启

4.9.3 `HealthOnFailure=stop` 最终调用 `Container.Stop()`，使用容器的 StopTimeout；`kill` 明确调用 **SIGKILL**，没有归档收尾窗口。`Stop()` 是先停止信号、宽限结束后才 SIGKILL，应用仍须正确处理 SIGTERM。[health 选项](https://github.com/containers/podman/blob/v4.9.3/docs/source/markdown/options/health-on-failure.md)、[health 实现](https://github.com/containers/podman/blob/v4.9.3/libpod/healthcheck.go)、[Container.Stop](https://github.com/containers/podman/blob/v4.9.3/libpod/container_api.go)

**`HealthOnFailure=stop` + `Restart=on-failure` 不能保证健康失败后重启**：应用收到 SIGTERM 后若正常返回 0，systemd 看见成功退出，不满足 on-failure。此处采用 **`Restart=always` + `RestartSec=5`**，可在 health-stop 之后的正常退出时重新拉起；用户主动 `systemctl --user stop h-live.service` 属于显式停止，不会因此反复启动。仍受 systemd 启动频率限制，不能承诺无限重试。[systemd v255 service manual](https://github.com/systemd/systemd/blob/v255/man/systemd.service.xml)

健康检查应检测进程卡死 / 核心不可用，不能把单房间离线、暂时无弹幕、平台 429/444 简单当作整个服务必须重启；否则会制造持续重启。健康判据与故障注入需由 H-live 实际测试验证。

## 4.9.3 generator 的停止命令与覆盖顺序

固定源码 `ConvertContainer` 先复制原 `.container` 的 section，再追加：

```text
ExecStop=/usr/bin/podman rm -v -f -i --cidfile=%t/%N.cid
ExecStopPost=-/usr/bin/podman rm -v -f -i --cidfile=%t/%N.cid
```

实际 Podman 绝对路径由发行版构建决定，上面是 Ubuntu 常规安装路径。**`rm -f` 在这里不等于立即 SIGKILL**：CLI 未显式设置 `--time` 时，删除逻辑先用 `c.StopTimeout()` 调用 `c.stop(time)`；因此 `PodmanArgs=--stop-timeout=180` 会作用于默认 ExecStop 的 running-container 收尾路径。[generator](https://github.com/containers/podman/blob/v4.9.3/pkg/systemd/quadlet/quadlet.go)、[rm CLI](https://github.com/containers/podman/blob/v4.9.3/cmd/podman/containers/rm.go)、[remove 实现](https://github.com/containers/podman/blob/v4.9.3/libpod/runtime_ctr.go)

必须另设 **`[Service] TimeoutStopSec=240`**，为 180 秒容器停机与清理留出余量；否则 systemd 自己可能先超时结束 stop command。180 秒到期仍可被强杀，这只是有限停机预算。[systemd v255 service manual](https://github.com/systemd/systemd/blob/v255/man/systemd.service.xml)

默认语义已经支持上述宽限；若项目为了跨版本清晰性要显式调用 `podman stop`，**不要只在 `.container` 的 `[Service]` 中添加 ExecStop**：4.9.3 会在它后面再次追加默认 rm。应在 `~/.config/systemd/user/h-live.service.d/10-stop.conf` 使用标准 systemd drop-in：

```ini
[Service]
ExecStop=
ExecStop=/usr/bin/podman stop --ignore --time=180 h-live
TimeoutStopSec=240
```

这里 `h-live` 必须匹配 `[Container] ContainerName=h-live`；也可用匹配生成器的 `--cidfile=%t/%N.cid`。drop-in 在生成的主 unit 后解析，空 ExecStop 先清空原列表，再设新命令；不清空 ExecStopPost 即保留生成清理。安装 / 升级后须用 `systemctl --user cat h-live.service` 核对最终合并结果。[systemd v255 unit drop-ins](https://github.com/systemd/systemd/blob/v255/man/systemd.unit.xml)、[service command-list 规则](https://github.com/systemd/systemd/blob/v255/man/systemd.service.xml)

## 离线镜像导入

4.9.3 官方 `podman load` 同时支持 `docker-archive`、`oci-archive`，`--input` 支持压缩文件，官方示例直接使用 `fedora.tar.gz`。因此 `docker save` / 对应 docker-archive 产物经 gzip 压缩后可用：

```sh
podman load --input h-live-image.tar.gz
```

这是镜像 archive，不能以 `podman import` 替代；`import` 面向 `export` 的容器根文件系统。必须以实际运行 Quadlet 的同一个非 root 用户导入，确保进入其 rootless image storage，加载后核对镜像架构与确切 tag，Quadlet `Image=` 匹配本地 tag、`Pull=never` 才不会再尝试远端拉取。压缩包与展开层都消耗宿主空间；4.9.3 load 默认临时目录 `/var/tmp`，必要时显式配置宿主 TMPDIR。这与容器内 `/tmp` tmpfs 无关。[官方 load 手册](https://github.com/containers/podman/blob/v4.9.3/docs/source/markdown/podman-load.1.md)

以上是资料核对阶段的官方支持范围与源代码结论。后续 Ubuntu 24.04 实机结果另见 [Podman 验证记录](../engineering/SERVER_PODMAN_VALIDATION.md)，未覆盖的故障场景也在该记录中列明。

## CI 实测：手动 dry-run 成功，但 user unit not-found

后续 Ubuntu 24.04 GitHub runner 测试出现：新建 `hlive-ci` 用户、启用 linger、启动其 user manager 后，rootless image load 成功；Quadlet 带显式 `QUADLET_UNIT_DIRS` 的 dry-run 能输出完整 service，但 daemon-reload 后 `LoadState=not-found`。

**此次诊断证据指向 CI 环境污染，不是 Quadlet 语法或打包缺失**：runner 上实际 user-generator 存在；user manager 的 HOME 为 `/home/hlive-ci`，但 XDG_CONFIG_HOME 仍为 `/home/runner/.config`，journal 明确记录 Quadlet 访问后一路径被拒绝。DBus 子进程还继承了原 runner UID 的 XDG_RUNTIME_DIR。只有实际日志与环境组合才足以作此判断。

官方来源与诊断推论：

- Ubuntu Noble [podman 文件列表](https://packages.ubuntu.com/noble/amd64/podman/filelist) 包含 `/lib/systemd/user-generators/podman-user-generator`、system-generator 与 `/usr/libexec/podman/quadlet`；[4.9.3 Makefile](https://github.com/containers/podman/blob/v4.9.3/Makefile) 将 user-generator 设为指向 quadlet 的链接。仍需核对具体机器上的 `readlink -e`、执行权限与更高优先级目录是否屏蔽该 generator。
- [4.9.3 main.go](https://github.com/containers/podman/blob/v4.9.3/cmd/quadlet/main.go) 用执行文件 basename 包含 `user` 判断用户模式，再以 `os.UserConfigDir()` 发现 `containers/systemd`；有 QUADLET_UNIT_DIRS 时则直接走覆盖路径。[Go UserConfigDir](https://github.com/golang/go/blob/go1.20.14/src/os/file.go) 在 Linux 优先 XDG_CONFIG_HOME，缺省才用 HOME/.config。因此显式目录 dry-run 成功，**不能证明 manager 的默认目录发现正确**。
- systemd v255 [manager.c](https://github.com/systemd/systemd/blob/v255/src/core/manager.c) 在 user manager 初始化时将自身 `environ` 复制到 `transient_environment`；`build_generator_environment` 从该 transient 环境构建 generator 环境。另一方面 [dbus-manager.c](https://github.com/systemd/systemd/blob/v255/src/core/dbus-manager.c) 的 SetEnvironment / UnsetEnvironment 修改的是 `client_environment`。正常服务的 effective environment 合并两者，但 generator 只取 transient。因此 **`systemctl --user set-environment` 后 daemon-reload 并不足以修正本例 generator 的污染路径**；`show-environment` 显示新值也不能单独证明它已生效于 generator。
- systemd v255 [path-lookup.c](https://github.com/systemd/systemd/blob/v255/src/basic/path-lookup.c) 的 user generator 目录包括 `/run/systemd/user-generators`、`/etc/systemd/user-generators`、`/usr/local/lib/systemd/user-generators` 与发行版目录；SYSTEMD_GENERATOR_PATH 可改写发现范围。[generator 手册](https://github.com/systemd/systemd/blob/v255/man/systemd.generator.xml) 规定高优先级同名文件 / `/dev/null` 链接可屏蔽 generator。故通用排查应区分「没执行 generator」「读错源目录」「生成失败」「生成结果不在 manager UnitPath」，不能把它们全归因于同一故障。

曾尝试仅在 CI 为第二用户的 `user@UID.service` 包装启动环境，但实测 manager 启动超时，**没有构成成功修复，最终方案已移除该实验**。这里不推测未确认的超时原因，也不建议生产用户照搬。systemd 原始配置依据：[user@ unit](https://github.com/systemd/systemd/blob/v255/units/user@.service.in)、[exec 环境顺序](https://github.com/systemd/systemd/blob/v255/man/systemd.exec.xml)。

最终 CI 直接使用临时 VM 原有的非 root runner 用户及匹配的 systemd 会话，开始前确认应用目录和 unit 均不存在，所有 Podman 操作不加 sudo。无需修改 PAM、user@ ExecStart 或宿主安全策略，即可正常 daemon-reload 和加载 Quadlet。**[CI 39009677731](https://github.com/isTrih/H-live/actions/runs/39009677731) 已通过**：Podman 4.9.3，source `2d3a9e4119c7828e1cbb959b9953b153e7a6f008`，实测一个运行容器、UID 10001、只读挂载、鉴权、重复初始化不改密钥、缺密钥拒绝重建、停止归档、重启持久化及故障 kill 后自动恢复。完整范围和未测项见验证记录；生产安装脚本不包含上述 CI 环境实验。
