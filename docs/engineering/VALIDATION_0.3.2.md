# 0.3.2 本地桌面发版验证

2026-10-20 完成本地安装包与 CDN 热更新包；本次没有创建桌面 GitHub Release，也没有上传 CDN。GitHub Releases 保留给独立服务端，服务端版本仍为 0.1.1。

## 本地产物

| 文件 | 字节 | SHA256 |
| --- | ---: | --- |
| `dist/installer/QingkanLive-0.3.2-windows-x64-setup.exe` | 9,716,011 | `1e5d29be68e1b00737320fcc67eafb3c18db699d6963049f5ac0478ad0339ae6` |
| `dist/QingkanLive-0.3.2-cdn.zip` | 10,239,064 | `51c30c7d4662ee7b5825cc3c71b16776095623f21bfe7d4f2b945342e35c5619` |

ZIP 仅含版本目录内的安装器、签名、SHA256，以及根目录 `latest.json`、`UPLOAD-INSTRUCTIONS.md`，五项内容与原文件逐项哈希一致。先上传 `0.3.2/`，最后替换 CDN 根目录清单。此前本地保存的 0.3.0 / 0.3.1 安装器、签名及 CDN 包哈希均未改变。

## 验收

- 60 项前端、141 项 Rust 工作区及 5 项桌面测试通过；发行构建锁定依赖，正式程序离线自检通过，版本均为 0.3.2。
- 用原公钥验证安装器签名成功，内存中单字节篡改被拒绝；清单版本、下载地址、更新日志、签名和安装器副本一致。
- 保留原应用标识、二进制名称、更新公钥、HTTPS CDN 地址及用户数据目录。正式运行包没有 Python、Node、独立 V8 或旧 engine sidecar。
- 首次构建在 NSIS 阶段中断，重试复用已标记的 NSIS 二进制。检查未知 bundle 占位标记为 0，NSIS 标记为 2；原签名配置没有更换。
- 使用独立 `QingkanWorkspaceTest` 产品身份，验证全新安装、同版本重装、0.3.1 → 0.3.2 升级、已安装程序自检、633 项资源文件哈希，以及卸载后保留数据。用户实际安装与预览未改变。
- 原始用户 CSV、聊天库、代理凭据、签名私钥和诊断日志均不进入发行包或源码提交。更新签名与 Windows Authenticode 证书是不同机制。

本地汇总报告为 `dist/windows-validation-0.3.2.json`。活动合并、卡片、翻页计时及合成性能的范围见 [ACTIVITY_ROUNDS.md](ACTIVITY_ROUNDS.md)，详细用户更新日志见 [0.3.2](../../release-notes/0.3.2.md)。本次没有新增真实平台批量或 24 小时监控验收。
