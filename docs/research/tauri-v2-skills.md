# Tauri v2 技能研究与安装记录

观察时间：2026-10-15 13:10–13:14，Asia/Shanghai（05:10–05:14 UTC）。由主任务委派的后台研究代理执行，使用已读取的 mattpocock `research` 流程和 Codex 官方 `skill-installer`。本轮项目内只新增此记录；未修改生产源码、依赖或应用配置。

## 安装结果

已安装 **1 个技能 `tauri-v2`**，下一个对话回合可使用：

- 来源：[pedronauck/skills 的 tauri-v2](https://github.com/pedronauck/skills/tree/0422940cea5d9960b3697016d1cb28c8ed02b030/skills/curated/tauri-v2)。
- 固定提交：`0422940cea5d9960b3697016d1cb28c8ed02b030`。
- 全局位置：`~\.codex\skills\tauri-v2`。
- 入口：`~\.codex\skills\tauri-v2\SKILL.md`。
- 版本声明：1.0.1；技能 README 声明 MIT。目录最近提交为 2026-09-14 19:12:26 UTC；正文自述最后核验 API 日期为 2026-04-02，两者含义不同。[目录提交记录](https://api.github.com/repos/pedronauck/skills/commits?sha=0422940cea5d9960b3697016d1cb28c8ed02b030&path=skills/curated/tauri-v2&per_page=1)、[固定 README](https://github.com/pedronauck/skills/blob/0422940cea5d9960b3697016d1cb28c8ed02b030/skills/curated/tauri-v2/README.md)。

已用官方 helper 的 `--ref` 固定提交安装，退出码 0，输出确认目标路径。安装后校验目录内 8 个普通 Markdown 文件的 SHA-256 与安装前读到的内容一致，无脚本、二进制或符号链接。没有运行技能中的命令示例或任何第三方安装脚本。

```powershell
.\.venv\Scripts\python.exe '~\.codex\skills\.system\skill-installer\scripts\install-skill-from-github.py' --repo pedronauck/skills --ref 0422940cea5d9960b3697016d1cb28c8ed02b030 --path skills/curated/tauri-v2 --dest '~\.codex\skills' --method download
```

## 星数与候选比较

以下是 **GitHub 整个仓库**的实时 `stargazers_count`，不是单个技能星数，也不是 Tauri 官方背书。已通过 GitHub 官方 API 查询；星数会变化，搜索范围不是全站穷举。

| 仓库 | 观察星数 | 维护状态 / 内容核查 | 决定 |
| --- | ---: | --- | --- |
| [midudev/autoskills](https://api.github.com/repos/midudev/autoskills) | 6,950 | 未归档；最近提交 2026-10-14。收录相同 `tauri-v2` SKILL 和参考文件 | 未安装：根许可与技能页声明不同，采用另一处同内容来源。 |
| [xiaolai/vmark](https://api.github.com/repos/xiaolai/vmark) | 841 | 未归档；最近提交 2026-10-13。包含通用 tauri-app-dev 及 VMark 专用桥接/MCP 技能 | 未安装：项目专用路径、伴随工作流较多，当前不需叠加。 |
| [partme-ai/full-stack-skills](https://api.github.com/repos/partme-ai/full-stack-skills) | 695 | 未归档；最近提交 2026-10-06。当前树中 Tauri 部分为规划文档，独立技能转移到另一仓库 | 不把总仓库星数嫁接给拆分后的技能仓库。 |
| [pedronauck/skills](https://api.github.com/repos/pedronauck/skills) | 637 | 未归档；最近提交 2026-09-14。明确 Tauri v2 的命令、IPC、权限、插件、sidecar 与更新参考 | **安装 1 个 `tauri-v2`**。 |
| [peditx/tauri-skills](https://api.github.com/repos/peditx/tauri-skills) | 55 | 未归档；最近提交 2026-07-27。含 Python sidecar、桌面客户端和其他扩展模板 | 未安装：非高星候选，部分客户端材料含自签 TLS 绕过主题，不适合本项目直接套用。未执行。 |
| [dchuk/claude-code-tauri-skills](https://api.github.com/repos/dchuk/claude-code-tauri-skills) | 36 | 未归档；最近提交 2026-01-20，39 个 Tauri 专项目录 | 未安装：维护与覆盖未构成相较已选技能的必要补充。 |
| [full-stack-skills/tauri-skills](https://api.github.com/repos/full-stack-skills/tauri-skills) | 20 | 未归档；最近推送 2026-10-03，明确 Tauri v2 的独立集合 | 未安装：不能把其称为千星 Tauri 技能库。 |
| [sickn33/agentic-awesome-skills](https://api.github.com/repos/sickn33/agentic-awesome-skills) | 47,617 | 未归档；固定树未发现路径名含 tauri 的技能 | 未因总星数高而安装无直接对应内容的整套集合。 |

637 星属于此次明确 Tauri 技能候选中的相对高关注来源，并非“万星 Tauri 技能”。没有为凑够两个安装重复技能。`midudev/autoskills` 固定提交为 `b4489cbdc36ba8c9de8f4dd844bab0b6b559579f`：其 [根 LICENSE](https://github.com/midudev/autoskills/blob/b4489cbdc36ba8c9de8f4dd844bab0b6b559579f/LICENSE) 是 CC BY-NC 4.0，而 [技能 README](https://github.com/midudev/autoskills/blob/b4489cbdc36ba8c9de8f4dd844bab0b6b559579f/packages/autoskills/skills-registry/tauri-v2/README.md) 写 MIT。本轮不裁定该混合许可的适用范围，也不将其当无条件 MIT 软件分发。两处主 SKILL 的 Git blob 均为 `d6bfd9fead96bf26cff2e1ce6e725d6c1c9c82d9`，参考目录 Git tree 均为 `b820208494f0e7b76e1b40d192b5c20fdbd0e64e`；内容相同不等于同等许可声明。

## 安全与正确性审阅

已完整读取拟安装目录的 8 份文档。未发现要求读取凭据、上传项目数据、隐藏执行、覆盖用户指令或安装持久化组件的内容。内嵌 shell/Rust/JS 为示例，仅审阅未运行；技能不会因复制文件自动安装插件或改变应用权限。

这是社区参考，**不能代替当前官方文档和编译测试**。发现以下应校正的具体点，应用时必须遵循本项目要求与官方事实：

1. 技能笼统称所有操作默认禁止。实际注册的自定义应用命令默认可供应用各窗口调用，若要限制需配置 `AppManifest::commands` 与权限；能力模型也不能替代 Rust 代码里的校验。[Tauri capabilities](https://v2.tauri.app/security/capabilities/)。
2. updater 参考包含 `active`、`dialog` 旧式配置，并将更新验签与操作系统代码签名混为宽泛表述。当前 v2 配置结构不包含这两项；应使用已锁定插件版本的 schema、HTTPS、公钥和签名产物。安装包 Authenticode 与 Tauri 更新签名分别验收。[当前插件配置源码](https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/updater/src/config.rs)、[官方 updater 文档](https://v2.tauri.app/plugin/updater/)。
3. 示例把 command 参数的 camelCase 转换泛化到所有结构体。结构体字段仍应显式确定 Serde 命名，不能依赖隐式转换。[官方命令参数说明](https://v2.tauri.app/develop/calling-rust/)、[Serde 字段命名](https://serde.rs/attr-rename.html)。
4. sidecar 示例使用 `args: true`，能力示例含较宽的 HOME/远程 URL 范围；本项目不采用任意参数、任意路径或远程页面的权限。只开放实际需要的命令/路径；Rust 控制固定后台程序和结构化参数。[已审阅 sidecar 参考](https://github.com/pedronauck/skills/blob/0422940cea5d9960b3697016d1cb28c8ed02b030/skills/curated/tauri-v2/references/advanced-runtime-reference.md)、[官方 sidecar 说明](https://v2.tauri.app/develop/sidecar/)。
5. 文档中的 token 节省比例、零错误和“Production Tested”标题未提供可复现实验，不作为本项目性能或可用性证据。未编译或运行其全部示例。

## 对本项目 0.2.0 的架构约束

以下为结合项目需求提出的建议，不是声称技能已经替我们完成实现：

- 将协议解码、归档与导出放入可独立测试的 Rust 库；Tauri 壳负责窗口、权限、更新和 IPC，Python/PyO3 保留明确接口边界。不要仅为移动端模板把全部业务堆入一个 lib.rs。
- CPU 解压/解码和磁盘写入不得阻塞 WebView 或主线程；共享状态锁仅覆盖状态修改，不跨 I/O 和长期任务持锁。[Tauri 异步命令](https://v2.tauri.app/develop/calling-rust/)。
- 高频内容优先批量交付或有界 Channel，低频状态可用事件；前端保持有界缓存和虚拟宫格。Tauri 官方明确普通事件并非高吞吐通道。[Rust → Frontend 与 Channels](https://v2.tauri.app/develop/calling-frontend/)。
- IPC 契约显式定义字段、错误码和版本。用户 ID / room_id / msg_id 以字符串跨 JSON，未知时间留空；Rust 与旧 Python 用同一 protobuf 样本差分，不能把快解释为正确。
- 网络出口、认证和 TLS 继续由单一房间租约控制，凭据不传到 UI 状态、日志或错误原文。性能迁移不能改变真实场次、去重、ack、心跳和停止后不再入库的语义。
- 更新沿用 HTTPS、签名验证、安装前二次就绪检查、归档完成与后台退出门禁；新增 Rust 动态库与 Python 绑定必须作为同一兼容版本交付。插件升级以锁文件和项目生成 schema 为准，不直接运行技能的 `@latest` 示例。

## 安装文件校验

相对目录 `~\.codex\skills\tauri-v2`；全部为普通文件，无 symlink。

| 文件 | 字节 | SHA-256 |
| --- | ---: | --- |
| SKILL.md | 16552 | `86b21fe8d43992247891f28249c615cd2284307b0ca539f7a2a3b82698974bb9` |
| README.md | 7654 | `724c4d70bebb77efa7f51ac1e46bf571aebdb0e76dd078e5b8397e3abcb35e0b` |
| references/README.md | 1889 | `ae87229efecfd2a5dcc9a59ab030f30738eea9b82a3948ba6ec536190b4ec784` |
| references/advanced-runtime-reference.md | 5079 | `69e5ec9329ac82b5442aa4a2cda30df7db81578ef290c46c88525e18bcb91db4` |
| references/capabilities-reference.md | 8816 | `aff274cc3d8731f4faa4bafe892b281edaf9f3e2dc5a2509c6d610819062157e` |
| references/ipc-patterns.md | 12227 | `49c5dceb42c31b27b0f86c48267b06d8a4d62e0f18c737ecc32473e76f0e8b13` |
| references/plugin-reference.md | 8835 | `0025c75c704300046570b43269135a3c0d0b622f4e411606cbd8477ab681db52` |
| references/updater-distribution-reference.md | 4256 | `9bb41e04950c9fe1a908c1a0e4559a1475d765c76446196302c716253a7f476d` |
