# 氢看直播：发布到 CDN

正式版本从 0.1.0 开始使用同一个 Tauri NSIS 安装及更新渠道。旧 Inno 试制版需要关闭后，手动把正式版装到新的默认目录；安装器不会静默卸载旧程序或覆盖旧安装目录。本地数据仍在 `%LOCALAPPDATA%\DouyinLiveMonitor`。

将 `dist/cdn/qingkan-live/` 中的版本子目录先上传到：

`https://assets.checkpoint321.com/lilith/qingkan-live/`

当前 0.3.1 的文件最终应为 `0.3.1/QingkanLive-0.3.1-windows-x64-setup.exe`、同名 `.exe.sig` 和 `.exe.sha256`。可直接解压 `dist/QingkanLive-0.3.1-cdn.zip`，先上传版本子目录；确认安装器可下载后，最后上传或原子替换根目录 `latest.json`。应用检查地址固定为 `https://assets.checkpoint321.com/lilith/qingkan-live/latest.json`。建议 latest.json 使用短缓存或 `Cache-Control: no-cache`，版本文件可使用长期缓存。

`.sig` 为 Tauri 更新完整性签名，不是 Windows Authenticode 代码签名。不要修改签名后的 EXE，也不要手工把别的文件名/签名拼进 latest.json。已安装版本与清单中的相同版本比较应显示无更新；0.1.0 / 0.1.1 / 0.2.0 可发现 0.3.1，后续版本必须继续增加 SemVer 版本号。保留已发布版本目录，不覆盖旧版本安装器及 ZIP。

私钥仅在本机 `.release-secrets/qingkan-updater.key`，整个目录已被 Git 忽略。不要上传该目录、生成日志或私钥；安全备份原私钥是后续更新的必要条件。新版本继续使用同一私钥，公钥已经编译进应用。发布产物只包含公钥验证所需的 `.sig`，不包含私钥。

构建：`scripts/build.ps1`；只重新同步文档并生成 NSIS/CDN 目录：`scripts/package_release.ps1`。更新代码或前端后必须先完整构建，不能只重新打包。上传由发布者自行执行，本工具不会调用 CDN 写入接口。
