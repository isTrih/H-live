# 官网完整更新日志

`scripts/release_notes_data.mjs` 读取 `package.json` 和 `release-notes/`。只收录不高于当前版本的 `主版本.次版本.补丁版本.md`，按版本数字降序排列；排除 `next.md`、`server-*` 和其他文件。

渲染脚本可调用 `await buildReleaseNotes(root)`，返回 `{format:1,current_version,entries:[{version,title,body,date,date_status,date_source}]}`。`body` 保留完整 Markdown 和换行，只删除文件开头的 UTF-8 BOM。函数本身不写文件。

每条记录都有 `date`：有依据时是 `YYYY-MM-DD`，缺少依据时是 `null`，且 `date_status` 为 `unrecorded`，页面应显示「日期未记录」。有日期时 `date_status` 为 `recorded`；这表示记录了日期依据，不保证该版本曾成功上线。`date_source` 说明具体依据；例如 `publication-verification` 是成功验证日期，`manifest` 是发行清单声明日期。不能把构建准备、文件修改时间或当前日期当作发布日期。

日期优先使用 `release-notes/release-dates.json` 的逐版记录。其次读取 `dist/cdn/qingkan-live/latest.json`，最后读取 `release-site/latest.json`，仅将有效清单的 `pub_date` 用于清单自己的版本，不会套用到其他日志。所有带时间的日期统一转换为 `Asia/Shanghai` 的日历日期，避免 UTC 时间跨午夜时错一天。旧清单没有 `pub_date` 时保持未记录；无效日期会终止生成。

`release-dates.json` 保留了 23 版日志的证据路径、字段、原始时间和必要说明。0.1.0、0.1.1、0.2.0、0.2.1 的日期暂未记录；包含这些日志的 0.2.4 基线提交不能证明更早版本的发布时间。0.3.2 的旧 CDN ZIP 清单写的是 2026-10-20，优先于 CHANGELOG 的 10/19。0.4.0 采用手动上传后清单的中国日期 10/24。0.4.2 留存清单也声明 10/24，但其发布尝试标记为 `unsubmitted`，没有成功站点回执，因此来源说明明确标注此边界。

未来 `package_release.ps1` 在写完签名清单、冻结发布载荷前，运行 `node scripts/release_notes_data.mjs --record-manifest <项目目录> <清单路径>`，将该版首次声明日期保留到日期文件。已有日期永不被重新打包时间覆盖；维护者也可以事先在日期文件中记录有依据的日期。此步骤不修改版本、不更改已冻结载荷。清单路径必须在传入项目目录内。`recordReleaseDate(root, manifest, sourcePath)` 也可供其他发布脚本复用。

运行 `node scripts/release_notes_data.mjs [项目目录]` 会原子写入该目录的 `release-site/changelog.json`。网站渲染器应使用 `serializeReleaseNotes(data)` 序列化；此函数会从公开 `entries` 中删除 `date_source`，避免暴露本地快照路径和部署 ID，保留全部日期与正文。本地 `buildReleaseNotes` 返回的来源、`release-dates.json` 的完整证据均不受影响。日志应当作文本或经过转义的 Markdown 渲染，不要直接赋给 `innerHTML`。

单份日志不超过 1 MiB，全部日志不超过 16 MiB、1000 个版本。无当前版本日志、无效 UTF-8 或超限文件会终止生成，保留已有输出。

验证：`node --test scripts/release_notes_data.test.mjs`。覆盖完整正文、日期依据、北京时间跨日、已记录日期优先、重打包不改日期、跨版本历史保留、非法日期和缺失日期。测试只在临时目录写入，不改动真实站点或发布版本。
