# 原生录播缓存与 HLS 预取

2026-10-24。使用项目当前捆绑的 libmpv 实测；不改变 D3D11 GPU 解码、原生视频图层或弹幕实现。

## 结果与实现

旧配置的网络缓存受 32MiB 前向字节预算限制；`demuxer-readahead-secs=10` 并不代表网络流只缓存 10 秒，因为 mpv 的网络 `cache-secs` 可以覆盖它。新策略把压缩包缓存调整为前向 96MiB、后向 16MiB，同时把预取目标限制为 120 秒；两个上限先到者生效。缓存耗尽时积累约 2 秒再恢复，启动及跳转也采用这个较小门槛，无需等满 120 秒。[固定 mpv 版本的选项文档](https://raw.githubusercontent.com/mpv-player/mpv/0fae46954/DOCS/man/options.rst)

继续使用 mpv 自带的 demux 线程。HLS 显式启用 HTTP 持久连接及 `http_multiple=1`；FFmpeg 维护当前分段和下一分段输入，并在跳转、关闭时释放旧输入。没有额外下载线程池。实际并发按活跃音视频播放列表计算，并非整个媒体固定只有两个 HTTP 请求。[FFmpeg HLS 源码](https://raw.githubusercontent.com/FFmpeg/FFmpeg/n8.0/libavformat/hls.c)

关闭磁盘缓存；预算针对压缩包队列，包含少量单包越界和估计开销，不等于进程总内存上限。解码帧、GPU 纹理和网络库还有各自开销。mpv 的磁盘缓存会持续追加，字节预算仅约束其内存元数据，不适合作为这里的有界缓存。填满后按 60 秒或 32MiB 的低水位恢复补充，减少持续唤醒。

播放器关闭或切换源时仍由 `mpv_terminate_destroy` 终止其 I/O。缓存仅属于当前播放器，未持久化签名 URL，也未加入自动重连循环；URL 失效仍须通过现有重试流程重新解析。增加缓存不能修复已失效的鉴权链接。

## 可重复验证

本地测试源为 100 秒、8Mbps 的合成 HLS。先暂停并填满缓存，然后让 HTTP 源返回 503，以 20 倍速播放；要求在源不可用时仍能跨过第 45 秒。该测试验证网络缓存连续性，不验证 GPU 画面。

| 策略 | 前向缓存 | 缓存时长 | 断源后最大播放位置 | 45 秒目标 | 关闭耗时 |
| --- | ---: | ---: | ---: | --- | ---: |
| 原 32MiB | 33,564,864 B | 32.267 s | 35.667 s | 未通过 | <1 ms |
| 新 96MiB | 100,666,464 B | 97.100 s | 45.117 s | 通过 | 16 ms |

脱敏原始结果：`build/cache-baseline.json`、`build/cache-production.json`。调试探针仅保留于忽略的 `build/verify_replay_cache.py`，不进入源码或发行载荷。

真实场次 `7698991795467832114`：新策略 2.375 秒取得约 120 秒前向内容，34,803,696 B；原策略 2.282 秒取得 115.6 秒，33,555,120 B。新策略缓存内跳转 20 秒耗时 63ms，缓存外跳转 6000 秒耗时 1328ms，返回 15 秒耗时 608ms，均成功；关闭 32ms。解码器报告 1280×720。该源码率较低，主要命中时间上限；结果不证明真实偶发卡顿已经复现或全部消失。网络时延波动也使单次跳转耗时不适合用于宣称提速。原始结果：`build/cache-real-production.json`、`build/cache-real-baseline.json`。

`cargo check --manifest-path src-tauri/Cargo.toml --offline --locked` 通过；`replay_cache::tests` 验证时间/字节预算和诊断字段白名单。缓存诊断通过 `native_replay_status.cache` 和设置兼容性报告返回，仅包含数字/布尔值，不返回媒体地址或任意 demux 元数据。[libmpv 节点 ABI](https://raw.githubusercontent.com/mpv-player/mpv/0fae46954/include/mpv/client.h)、[mpv 缓存属性](https://mpv.io/manual/master/#properties)
