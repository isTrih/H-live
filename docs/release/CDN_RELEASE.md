# 氢看直播：Sites 内网发布

固定发布入口：`./scripts/release.ps1`（PowerShell 7），也可运行 `npm run release`。每次新发布自动将桌面 patch 版本加一，例如 0.3.6 → 0.3.7，依次构建、签名、验签、组装单 HTML 站点并发布到同一个莉莉丝 Sites 站点。无需手动修改版本号。以后不生成或更新旧客户端 CDN 包。独立服务端继续使用 GitHub Releases，版本不随桌面递增。

当前机器从被 Git 忽略的 `.release-secrets/release-local.json` 读取原签名密钥的路径，不复制密钥。其他机器可配置 `QINGKAN_SIGNING_KEY_PATH` 或传入 `-SigningKey`。要额外运行全量测试时加 `-RunTests`；`-SkipBuild` 仅接受已经为本次准备版本构建并通过离线自检的产物。

更新说明优先读取 `-NotesPath <文件>`，其次读取 `release-notes/next.md`，其中 `{{version}}` 会替换为本次版本。没有提供说明时仅写“新版本已发布”，不会复制上一版功能。`next.md` 在发布成功后才移除；发布过程中改写的新内容会保留给下次发布。示例：

```powershell
./scripts/release.ps1
./scripts/release.ps1 -NotesPath ./changes.md
./scripts/release.ps1 -Bump minor
```

默认递增 patch；新功能版本使用 `-Bump minor`（例如 0.3.9 → 0.4.0），主版本使用 `-Bump major`。重试仍复用已有待发布版本。

版本准备同时同步 npm 根包与 lockfile、Tauri 配置、Rust 桌面与共享 workspace 的版本、两个 Cargo lockfile 中的本地包，以及应用内置更新日志。第三方依赖和独立服务端版本保持不变。版本状态保存在被忽略的 `.release-secrets/release-state.json`；同一时间只允许一个发布进程。发布失败后直接重跑同一命令，会复用已经准备的版本，不会再加一。进入 Sites 上传阶段后，重跑还会复用原站点文件和官方请求 ID；不要删除状态、上传请求或改动已经准备的站点文件。只有确认站点发布成功并保存回执后，下次运行才创建下一版本。

配置以 `release-site/publishing.json` 为准。首次部署仅含 HTML，部署成功后才采用返回的实际地址：

- 下载页：<https://qingkan-live.internal.lilithsites.com/>
- 新版检查更新：<https://qingkan-live.internal.lilithsites.com/latest.json>
- 安装包：`/<版本>/QingkanLive-<版本>-windows-x64-setup.exe`

站点为公司内所有人、内网免登录、公网关闭，需要公司网络或 VPN。不把飞书 Cookie 或令牌写进客户端。每次发布沿用原 Site ID、域名和权限。使用已安装的莉莉丝官方 `sites` Skill；换机器先安装该技能并登录 Atlas prod。非默认技能位置可用 `SITES_SKILL_DIR` 指定。官方上传票据与请求保存在受限的 `.c2site/` 目录，不能提交或分享；重试保留原请求，状态未知先查询已有部署。

`render_release_site.mjs` 将品牌图标、官网截图和固定版本 GSAP 内嵌成一个 index.html。`prepare_site.ps1` 只从已验签的发布目录选择当前版 EXE、签名、SHA-256、latest.json 和单个 index.html，不上传工程源码、密钥、日志、旧版本或开发文件。安装器包含完整 libmpv、FFmpeg/ffprobe，客户端无需另行下载组件。

超过官方 MCP 单文件 16 MiB 或 ZIP 45 MiB 限制时，发布脚本自动保留 `dist/sites/qingkan-live` 完整文件夹并显示原站点重新上传地址，不尝试 MCP 大文件上传、不拆分安装包，也不标记发布成功。请使用 Sites 管理页的“重新上传”选择该文件夹。确认站点成功、公开更新清单和签名安装包验证通过后保存正式发布回执，再执行 `node scripts/release_version.mjs complete .` 完成本版本。失败或等待人工上传期间重跑不会递增版本。
如果全部发布尝试已确认失败，且需要修改同一个尚未发布的版本，先通过官方 `c2site_deployment_list`/`c2site_deployment_get` 查询并保存失败记录，再运行 `node scripts/release_version.mjs repack-failed . build/失败记录.json`。该命令要求对应站点的所有提供记录均为 `failed` 或 `cancelled`、时间晚于本次版本准备、且本地不存在发布回执；旧摘要与失败证据保存在 `.release-secrets/release-attempts/`，版本号保持不变。随后重新构建并执行发布流程。状态未知、仍在部署或已有成功回执时不能重打同一版本。

如果旧事务停在大文件上传提示，尚未向 Sites 提交，而当前代码已有下一轮功能，不要直接重跑并把旧冻结安装包当作新功能发布。先通过官方接口读取 Site 当前部署和全部部署历史，翻页直到 `next_before_sequence` 为 `null`，保存完整报告：`{site_id,current_deployment,deployments:[官方部署条目],next_before_sequence:null}`。只有当前部署在报告中且已成功、所有记录均属本站且为终态、全部创建时间不晚于旧事务准备时间、原冻结目录未改动且没有该版本发布回执时，才能运行：

```powershell
node scripts/release_version.mjs retire-unsubmitted . build/完整部署历史.json
./scripts/release.ps1
```

该命令将旧站点载荷、原事务和官方证据完整备份到 `.release-secrets/release-attempts/`，仅把旧事务标记为 `superseded`（未提交的旧版本已被后续工作取代），保留原发行说明和 `next.md`，不会宣称旧版本已发布。下一次正常发布从当前工作版本递增，并重新构建、签名和上传本轮功能，例如未提交的 0.4.2 → 新构建的 0.4.3。历史未读完、存在较新部署或状态未知时必须先调查；不得删除事务文件或伪造成功回执来绕过验证。已成功发布的版本按正式验证与 `complete` 流程收尾，不能使用此命令。

## 旧客户端迁移

0.3.5 及以前的客户端仍读取旧 CDN。旧 CDN 永久停留在 0.3.6，不跟随之后的内网版本变化。一次性迁移包是 `dist/QingkanLive-0.3.6-legacy-cdn.zip`，包含相同签名的 0.3.6 EXE 和指向旧 CDN 的独立 latest.json。只需将其中 0.3.6 目录上传到 `https://assets.checkpoint321.com/lilith/qingkan-live/`，再替换根目录 latest.json。老客户端先更新至 0.3.6，然后从 Sites 内网获取后续更新。

旧 CDN 上传仍由发布者执行；Sites 发布脚本不会修改旧 CDN。仅发布 Sites 不能使尚未迁移的旧客户端发现更新。

`.sig` 是 Tauri 更新完整性签名，不是 Windows Authenticode 签名。沿用原私钥和编译公钥，不改动签名后的 EXE，不生成替代私钥，不上传 `.release-secrets/`。更新安装前保存监听列表并完成归档，NSIS 安装后自动重启；新进程在原生启动阶段初始化后台，恢复监听不依赖窗口显示。
