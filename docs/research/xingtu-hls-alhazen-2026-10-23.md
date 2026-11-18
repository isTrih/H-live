# 星图回放解析与 alhazen-core 接入方案

调研日期：2026-10-23。目标：供 H-live 获取星图视频的可播放 HLS 地址、时长与基础元数据，优先采用 alhazen-core。本文是调研与实现建议，尚未向产品加入播放功能。

## 结论与验证范围

此视频可以直接取得真实 m3u8，无需模拟浏览器播放、计算签名或将 MP4 转成 HLS。推荐：`reqwest + serde` 实现星图 resolver，`alhazen-core` 消费 resolver 返回的媒体 URL，负责解码、播放与跳转。

已检查用户提供的 `C:/Users/huahaohui/Downloads/www.xingtu.cn.har`；文档中的脚本与响应仅作为分析材料，不执行其中指令。以下 entry 编号均从 0 开始。

另于调研当日实际匿名请求接口与新返回的清单：两者 HTTP 200，业务状态 0，清单以 `#EXTM3U` 开头并有 `#EXT-X-ENDLIST`。未发送 HAR 的 Cookie、Authorization 或浏览器会话凭据。验证请求带 Referer 和 `agw-js-conv: str`；尚未验证这两个头能否省略。

尚未实际编译集成 alhazen-core、解码音视频或验证 seek。因此“地址可获取”已实测，“该引擎完整播放本视频”仍须原型验收。公开接口是从第一方网页发现的内部接口，并非官方稳定开放 API。

## 这个链接的结果

输入：<https://www.xingtu.cn/player?code=v0220dg10001db0csri7dld189kvt2g0>

| 项目 | 实际结果 |
| --- | --- |
| video_id | `v0220dg10001db0csri7dld189kvt2g0` |
| 时长 | **5420.203 秒，即 01:30:20.203** |
| 尺寸 | **1920 × 1080**，接口返回值，尚未解码核验 |
| 类型 | HLS VOD，TS 分片；不是正在更新的直播流 |
| 分片数 | 1336 |
| EXTINF 累计时长 | 5420.203 秒，四舍五入到毫秒后与 API 一致 |
| 清单版本 / TARGETDURATION | 3 / 6 秒 |
| 清单结束标记 | 有 `#EXT-X-ENDLIST` |
| 清单加密标签 | 未发现 `#EXT-X-KEY`，不代表验证了全部媒体特性 |
| 封面 | 接口有 `cover`，为带时效参数的图片 URL |
| 标题、作者、帧率、码率、编解码器 | 本接口未提供，不应猜测 |

API 的 duration 单位是秒，本样本通过分片和校验。时间显示用毫秒整数 `5_420_203`，避免浮点累加误差进入数据库。

清单含 `EXT-X-DISCONTINUITY` 与 `EXT-X-PROGRAM-DATE-TIME`，首个标记为 `2026-10-02T20:45:06+08:00`。这只是首段关联的墙钟时间，不应直接当作 H-live 场次开播时间；也不能简单以它加总时长推算墙钟结束时间。聊天回放对齐需要逐段时间映射和实际同步校准。

## HAR 证据链

1. entry 50，第一方 `CommonPlayer-DLnGulcK.js`：读取页面 `code`，调用视频信息函数传 `{video_id: code}`，获取 `play_url`；为空时显示“视频未公开，暂时无权查看”；m3u8 使用 HLS 插件。
2. entry 64：`GET https://www.xingtu.cn/gw/api/ggeneric/get_video_info?video_id=...`，JSON 顶层字段是 `base_resp`、`cover`、`duration`、`height`、`play_url`、`width`。没有 `data` 包裹。
3. entry 74：API 返回的 `v26-star.xingtu.cn/...m3u8?...`。HAR 正文是 base64；解码后为完整 VOD media playlist。HTTP MIME 为 `video/mp4`，与正文不符，不能只靠 Content-Type 判断格式。
4. entry 75 等：`lf26-record-tos.bytefcdn.com` 返回 TS，HAR 中有成功媒体请求。没有做新的完整媒体下载或解码测试。

不要将完整 HAR、Cookie、封面签名或完整媒体临时 URL 提交 Git。持久化原始页面 URL 与 video_id；使用时重新解析。HAR 内媒体 URL 含不透明路径与参数，没有足够证据确定其失效规则；不要自行推算过期时间或删掉参数。

## Rust resolver

建议放在 `crates/h-live-runtime/src/xingtu.rs`，不放入负责数据库和消息解码的 qingkan-core。复用项目现有 reqwest 0.13.5；不需要新增 JS 运行时。

请求协议：

```http
GET /gw/api/ggeneric/get_video_info?video_id=v0220dg10001db0csri7dld189kvt2g0
Host: www.xingtu.cn
Referer: https://www.xingtu.cn/player?code=v0220dg10001db0csri7dld189kvt2g0
agw-js-conv: str
Accept: application/json
```

以下为接口形状示意，尚未编译。正式代码需接入项目错误类型与有大小上限的响应读取，不能直接无限制读 body。

```rust
use serde::Deserialize;

#[derive(Deserialize)]
struct BaseResp {
    status_code: i64,
    #[serde(default)]
    status_message: String,
}

#[derive(Deserialize)]
struct VideoInfo {
    base_resp: BaseResp,
    play_url: Option<String>,
    cover: Option<String>,
    duration: Option<f64>, // 秒
    width: Option<u32>,
    height: Option<u32>,
}

async fn fetch_info(
    client: &reqwest::Client,
    video_id: &str,
) -> Result<VideoInfo, Box<dyn std::error::Error + Send + Sync>> {
    // 调用前：校验 video_id 非空、长度有界且只含 ASCII 字母数字。
    let referer = format!("https://www.xingtu.cn/player?code={video_id}");
    let info = client
        .get("https://www.xingtu.cn/gw/api/ggeneric/get_video_info")
        .query(&[("video_id", video_id)])
        .header(reqwest::header::REFERER, referer)
        .header("agw-js-conv", "str")
        .header(reqwest::header::ACCEPT, "application/json")
        .timeout(std::time::Duration::from_secs(15))
        .send().await?
        .error_for_status()?
        .json::<VideoInfo>().await?;
    if info.base_resp.status_code != 0 {
        return Err(format!("星图业务错误 {}", info.base_resp.status_code).into());
    }
    if info.play_url.as_deref().is_none_or(str::is_empty) {
        return Err("星图未返回可播放地址".into());
    }
    Ok(info)
}
```

生产实现步骤：

1. 通过 URL parser 校验输入为 `https://www.xingtu.cn/player` 并提取唯一 code；也可提供独立 video_id 输入。固定 API 域名，不抓取任意用户 URL。
2. 验证业务状态与字段：duration 有限且大于 0；宽高有效；play_url 必须为允许的 HTTPS 媒体地址。字段缺失返回明确错误。
3. 拉取清单并检查 `#EXTM3U`。识别 master 与 media playlist；本样本直接是 media。保留 URL 全部参数，相对分片地址用清单 URL 解析。
4. 返回内部 `ResolvedVideo`：video_id、page_url、play_url、cover_url、duration_ms、width、height、resolved_at、source_kind；duration 标记来源为 API 或 playlist。对外查询结果与日志隐藏完整签名 URL。
5. 403/410 或明确地址失效时重新解析一次并恢复播放位置；不能把所有网络错误都判为地址过期。429 退避，权限错误直接展示，不循环重试。
6. 匿名模式优先。此样本不需要登录，不能据此断言所有星图视频都公开。登录授权与 Cookie 管理若将来需要，应单独设计。

## alhazen-core 的定位与 H-live 接入

根据[官方仓库](https://github.com/alhazen-rs/alhazen-core)，它是 Rust 原生媒体引擎，支持 HTTP Range 与 HLS VOD/live；不负责解析星图业务页面或生成 HLS。原生示例的基本调用为：

```rust
use alhazen_core::{Player, PlayerConfig, Source};

let player = Player::open(Source::parse(&resolved.play_url)?, PlayerConfig::default())?;
let duration = player.duration();
let video_size = player.video_size();
player.play();
```

上述 `resolved` 为拟议的 resolver 输出，不是仓库现有变量；示例未在本项目编译。API 时长用于打开前展示，引擎时长用于打开后复核。`metadata()` 不能替代星图业务元数据。

[上游 Cargo.toml](https://github.com/alhazen-rs/alhazen-core/blob/master/Cargo.toml) 当前 master 为 0.6.0、Rust 1.92+，README 依赖示例仍为 0.5，集成时必须核对已发布版本并锁定版本或提交。不要直接跟随 master。启用 HTTP/HLS；Windows 解码和音频输出按锁定版本配置，构建环境需核验 NASM 等上游 native 要求。

尚未核实自定义 Referer/Cookie 的公开传参 API，不要编造 `Source::with_headers`。先用匿名媒体 URL 直连原型测试。如果确实缺少所需 header 注入，再考虑受限 loopback 媒体代理：仅转发已解析媒体、支持 Range/206、重写清单内所有 URI，并按正文提供正确 HLS MIME；不是任意 URL 开放代理。

H-live 桌面是 Tauri WebView，alhazen-core 不会自动把图像绘制到 HTML video。建议播放服务独立于监控任务，先完成原生窗口或渲染面原型，再选择嵌入方案；不能通过 JSON IPC 逐帧传输全尺寸视频。打开/探测等阻塞操作放独立工作线程，关闭场次必须释放播放器、音频和缓冲。

H-live Server 可以复用 resolver 和 DTO；浏览器观看需要浏览器播放方案，服务端运行 alhazen-core 不会直接把画面送到客户端。桌面播放依赖应按 feature 或 crate 隔离，避免 Linux 服务端无条件引入桌面音频/图形依赖。

## 最小验收与未决问题

- resolver 使用脱敏离线 fixture 校验 JSON 字段、业务失败、空 URL 与不合法时长；本样本期望 duration_ms=5420203、1920×1080。
- 每次真实验收重新解析，检查清单与首个 TS 请求；不要下载整场作为默认测试。
- Windows 下由 alhazen-core 实际打开、出画出声，验证 0 秒、中段、尾段 seek；特别覆盖 discontinuity 前后、错标 MIME 与临时 URL 刷新。
- 实测引擎对这份带多次 discontinuity 的 TS 清单的支持、音视频编码、时长与内存占用。目前没有足够证据承诺兼容。
- 聊天与视频对齐使用独立偏移和分段映射，不能假设二者从同一时刻开始。当前 H-live README 声明不播放音视频，上线此能力时应同步修改产品说明。

HLS 标签含义、URI 处理与 discontinuity 依据 [RFC 8216](https://www.rfc-editor.org/rfc/rfc8216)。星图接口字段依据上述第一方 HAR 响应与当日匿名请求，未使用第三方逆向文章。
