//! Bounded HLS VOD plans. Remote URLs stay in memory and never enter tool arguments.
use crate::{storage::Result, validated_media_url};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use url::Url;

#[derive(Clone)]
pub(crate) struct Asset {
    pub url: Url,
    pub range: Option<(u64, u64)>,
    pub file: String,
}
#[derive(Clone)]
pub(crate) struct Segment {
    pub start: f64,
    pub duration: f64,
    pub sequence: u64,
    pub continuity: u64,
    pub asset: Asset,
    pub init: Option<Asset>,
    pub pdt: Option<String>,
}
pub(crate) struct Playlist {
    pub segments: Vec<Segment>,
    pub fingerprint: String,
}
pub(crate) struct Plan {
    pub assets: Vec<Asset>,
    pub local_manifest: String,
    pub base: f64,
    pub fingerprint: String,
    pub duration: f64,
}

pub(crate) fn attributes(raw: &str) -> Result<BTreeMap<String, String>> {
    let mut fields = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    for (index, byte) in raw.bytes().enumerate() {
        if byte == b'"' {
            quoted = !quoted;
        } else if byte == b',' && !quoted {
            fields.push(&raw[start..index]);
            start = index + 1;
        }
    }
    if quoted {
        return Err("HLS 属性引号无效".into());
    }
    fields.push(&raw[start..]);
    let mut result = BTreeMap::new();
    for field in fields {
        let Some((key, value)) = field.split_once('=') else {
            continue;
        };
        result.insert(key.trim().into(), value.trim().trim_matches('"').into());
    }
    Ok(result)
}
pub(crate) fn variant(text: &str, base: &Url) -> Result<Option<Url>> {
    if !text.trim_start_matches('\u{feff}').starts_with("#EXTM3U") {
        return Err("回放未返回 HLS 清单".into());
    }
    for line in text.lines() {
        if let Some(raw) = line.strip_prefix("#EXT-X-MEDIA:") {
            let a = attributes(raw)?;
            if a.get("TYPE").is_some_and(|s| s == "AUDIO") && a.contains_key("URI") {
                return Err("此回放使用独立音轨，当前原画切片暂不支持".into());
            }
        }
    }
    let mut pending = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with("#EXT-X-STREAM-INF:") {
            pending = true;
        } else if pending && !line.is_empty() && !line.starts_with('#') {
            return Ok(Some(validated_media_url(
                base.join(line).map_err(|_| "HLS 子清单地址无效")?.as_str(),
            )?));
        }
    }
    if pending {
        return Err("HLS 主清单缺少视频流".into());
    }
    Ok(None)
}
fn byte_range(raw: &str, previous: Option<u64>) -> Result<(u64, u64)> {
    let (length, offset) = raw
        .split_once('@')
        .map(|(n, o)| (n, Some(o)))
        .unwrap_or((raw, None));
    let length = length
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0 && *n <= 16 * 1024 * 1024)
        .ok_or("HLS 字节范围长度无效")?;
    let start = match offset {
        Some(value) => value.parse::<u64>().map_err(|_| "HLS 字节范围偏移无效")?,
        None => previous.ok_or("HLS 隐式字节范围缺少前一个范围")?,
    };
    Ok((start, start.checked_add(length).ok_or("HLS 字节范围溢出")?))
}
pub(crate) fn parse(text: &str, base: &Url) -> Result<Playlist> {
    if !text.lines().any(|line| line.trim() == "#EXT-X-ENDLIST") {
        return Err("仅支持已结束的 HLS 回放，当前清单仍在直播".into());
    }
    if text.contains("#EXT-X-I-FRAMES-ONLY") {
        return Err("不支持仅关键帧预览清单".into());
    }
    let mut segments = Vec::new();
    let mut duration = None;
    let mut range = None;
    let mut init = None;
    let mut sequence = 0_u64;
    let mut continuity = 0_u64;
    let mut elapsed = 0.;
    let mut pdt = None;
    let mut previous: Option<(String, u64)> = None;
    for line in text.lines().map(str::trim) {
        if let Some(raw) = line.strip_prefix("#EXTINF:") {
            duration = Some(
                raw.split(',')
                    .next()
                    .unwrap_or("")
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite() && *n > 0. && *n < 86400.)
                    .ok_or("HLS 分片时长无效")?,
            );
        } else if let Some(raw) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            sequence = raw.parse().map_err(|_| "HLS 媒体序号无效")?;
        } else if let Some(raw) = line.strip_prefix("#EXT-X-DISCONTINUITY-SEQUENCE:") {
            continuity = raw.parse().map_err(|_| "HLS 连续性序号无效")?;
        } else if line == "#EXT-X-DISCONTINUITY" {
            continuity = continuity.checked_add(1).ok_or("HLS 连续性序号溢出")?;
            pdt = None;
        } else if let Some(raw) = line.strip_prefix("#EXT-X-KEY:") {
            if attributes(raw)?.get("METHOD").is_none_or(|s| s != "NONE") {
                return Err("暂不支持加密或 DRM 回放切片".into());
            }
        } else if line.starts_with("#EXT-X-SESSION-KEY:") {
            return Err("暂不支持加密回放切片".into());
        } else if let Some(raw) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            range = Some(raw.to_owned());
        } else if let Some(raw) = line.strip_prefix("#EXT-X-MAP:") {
            let a = attributes(raw)?;
            let uri = a.get("URI").ok_or("HLS 初始化段缺少地址")?;
            let url =
                validated_media_url(base.join(uri).map_err(|_| "HLS 初始化段地址无效")?.as_str())?;
            let range = a
                .get("BYTERANGE")
                .map(|raw| byte_range(raw, None))
                .transpose()?;
            init = Some(Asset {
                url,
                range,
                file: format!("init_{}.mp4", segments.len()),
            });
        } else if let Some(raw) = line.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:") {
            chrono::DateTime::parse_from_rfc3339(raw).map_err(|_| "HLS 墙钟时间无效")?;
            pdt = Some(raw.to_owned());
        } else if line == "#EXT-X-GAP" {
            return Err("回放包含缺失分片，无法保证切片完整".into());
        } else if !line.is_empty() && !line.starts_with('#') {
            let duration = duration.take().ok_or("HLS 分片缺少时长")?;
            if segments.len() >= 100_000 || elapsed + duration > 31. * 86400. {
                return Err("HLS 回放超出处理范围".into());
            }
            let url =
                validated_media_url(base.join(line).map_err(|_| "HLS 分片地址无效")?.as_str())?;
            let prior = previous
                .as_ref()
                .filter(|(old, _)| old == url.as_str())
                .map(|(_, end)| *end);
            let segment_range = range
                .take()
                .map(|raw| byte_range(&raw, prior))
                .transpose()?;
            previous = segment_range.map(|(_, end)| (url.to_string(), end));
            let asset = Asset {
                url,
                range: segment_range,
                file: format!(
                    "segment_{:06}.{}",
                    segments.len(),
                    if init.is_some() { "m4s" } else { "ts" }
                ),
            };
            segments.push(Segment {
                start: elapsed,
                duration,
                sequence,
                continuity,
                asset,
                init: init.clone(),
                pdt: pdt.clone(),
            });
            if let Some(value) = &pdt {
                let wall = chrono::DateTime::parse_from_rfc3339(value)
                    .map_err(|_| "HLS 墙钟时间无效")?
                    .checked_add_signed(chrono::Duration::milliseconds(
                        (duration * 1000.).round() as i64
                    ))
                    .ok_or("HLS 墙钟时间超出范围")?;
                pdt = Some(wall.to_rfc3339());
            }
            elapsed += duration;
            sequence = sequence.checked_add(1).ok_or("HLS 媒体序号溢出")?;
        }
    }
    if segments.is_empty() {
        return Err("HLS 清单没有媒体分片".into());
    }
    let mut digest = Sha256::new();
    for segment in &segments {
        let mut url = segment.asset.url.clone();
        url.set_query(None);
        digest.update(url.as_str());
        digest.update(segment.duration.to_bits().to_le_bytes());
        digest.update(segment.continuity.to_le_bytes());
        if let Some(range) = segment.asset.range {
            digest.update(range.0.to_le_bytes());
            digest.update(range.1.to_le_bytes());
        }
        if let Some(init) = &segment.init {
            let mut url = init.url.clone();
            url.set_query(None);
            digest.update(url.as_str());
            if let Some((start, end)) = init.range {
                digest.update(start.to_le_bytes());
                digest.update(end.to_le_bytes());
            }
        }
    }
    Ok(Playlist {
        segments,
        fingerprint: format!("{:x}", digest.finalize()),
    })
}
pub(crate) fn select(playlist: &Playlist, start: f64, end: f64) -> Result<Plan> {
    let segments = &playlist.segments;
    let first = segments
        .iter()
        .position(|s| s.start + s.duration > start)
        .ok_or("切片起点超出回放时长")?;
    let last = segments
        .iter()
        .rposition(|s| s.start < end)
        .ok_or("切片终点无效")?;
    if end > segments.last().unwrap().start + segments.last().unwrap().duration + 0.1 {
        return Err("切片终点超出回放时长".into());
    }
    if segments[first..=last]
        .iter()
        .any(|s| s.continuity != segments[first].continuity)
    {
        return Err("选区跨越录播时间中断，请分成多个切片".into());
    }
    let mut begin = first;
    for _ in 0..2 {
        if begin > 0 && segments[begin - 1].continuity == segments[first].continuity {
            begin -= 1;
        } else {
            break;
        }
    }
    if last - begin + 1 > 512 {
        return Err("切片需要过多分片，请缩短选区".into());
    }
    build(playlist, begin, last)
}

/// Full recordings use the actual VOD manifest, never the locally monitored window.
pub(crate) fn full_session(playlist: &Playlist) -> Result<Plan> {
    if playlist.segments.is_empty() {
        return Err("HLS 清单没有媒体分片".into());
    }
    build(playlist, 0, playlist.segments.len() - 1)
}

fn build(playlist: &Playlist, begin: usize, last: usize) -> Result<Plan> {
    let segments = &playlist.segments;
    let target = segments[begin..=last]
        .iter()
        .map(|s| s.duration.ceil() as u64)
        .max()
        .unwrap_or(6);
    let mut manifest=format!("#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-TARGETDURATION:{target}\n#EXT-X-MEDIA-SEQUENCE:{}\n#EXT-X-DISCONTINUITY-SEQUENCE:{}\n",segments[begin].sequence,segments[begin].continuity);
    let mut assets = Vec::new();
    let mut current_init = None;
    let mut continuity = segments[begin].continuity;
    for segment in &segments[begin..=last] {
        if segment.continuity != continuity {
            manifest.push_str("#EXT-X-DISCONTINUITY\n");
            continuity = segment.continuity;
            current_init = None;
        }
        if let Some(init) = &segment.init {
            if current_init.as_ref() != Some(&init.file) {
                manifest.push_str(&format!(
                    "#EXT-X-MAP:URI=\"{}\"{}\n",
                    init.file,
                    init.range
                        .map(|(s, e)| format!(",BYTERANGE=\"{}@0\"", e - s))
                        .unwrap_or_default()
                ));
                if !assets.iter().any(|a: &Asset| a.file == init.file) {
                    assets.push(init.clone());
                }
                current_init = Some(init.file.clone());
            }
        }
        if let Some(pdt) = &segment.pdt {
            manifest.push_str(&format!("#EXT-X-PROGRAM-DATE-TIME:{pdt}\n"));
        }
        manifest.push_str(&format!("#EXTINF:{:.6},\n", segment.duration));
        if let Some((s, e)) = segment.asset.range {
            manifest.push_str(&format!("#EXT-X-BYTERANGE:{}@0\n", e - s));
        }
        manifest.push_str(&segment.asset.file);
        manifest.push('\n');
        assets.push(segment.asset.clone());
    }
    manifest.push_str("#EXT-X-ENDLIST\n");
    Ok(Plan {
        assets,
        local_manifest: manifest,
        base: segments[begin].start,
        fingerprint: playlist.fingerprint.clone(),
        duration: segments[begin..=last].iter().map(|s| s.duration).sum(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn base() -> Url {
        Url::parse("https://v26-star.xingtu.cn/test/index.m3u8?signature=fixture").unwrap()
    }
    #[test]
    fn xingtu_vod_download_accepts_douyincdn_segments_without_changing_signatures() {
        let segment = "https://lr11.douyincdn.com/recording/fixture.ts?signature=synthetic%2Fvalue&expires=123";
        let text = format!("#EXTM3U\n#EXTINF:4,\n{segment}\n#EXT-X-ENDLIST\n");
        let parsed = parse(&text, &base()).expect("星图回放的抖音 CDN 分片应能创建下载计划");
        let full = full_session(&parsed).unwrap();
        let clip = select(&parsed, 0., 3.).unwrap();
        for plan in [full, clip] {
            assert_eq!(plan.assets.len(), 1);
            assert_eq!(plan.assets[0].url.as_str(), segment);
            assert!(!plan.local_manifest.contains("https:"));
            assert!(!plan.local_manifest.contains("signature"));
        }
    }
    #[test]
    fn plan_limits_downloads_and_rebases_byte_ranges_without_remote_urls() {
        let text="#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:10\n#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"100@0\"\n#EXTINF:4,\n#EXT-X-BYTERANGE:1000@100\nall.mp4\n#EXTINF:4,\n#EXT-X-BYTERANGE:1200\nall.mp4\n#EXTINF:4,\nlast.m4s\n#EXT-X-ENDLIST\n";
        let parsed = parse(text, &base()).unwrap();
        assert_eq!(parsed.segments[1].asset.range, Some((1100, 2300)));
        let plan = select(&parsed, 5., 7.).unwrap();
        assert_eq!(plan.assets.len(), 3);
        assert!(!plan.local_manifest.contains("https:"));
        assert!(plan.local_manifest.contains("BYTERANGE:1200@0"));
    }
    #[test]
    fn rejects_live_encrypted_external_audio_and_cross_discontinuity() {
        assert!(parse("#EXTM3U\n#EXTINF:4,\na.ts", &base()).is_err());
        assert!(parse(
            "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key\"\n#EXTINF:4,\na.ts\n#EXT-X-ENDLIST",
            &base()
        )
        .is_err());
        assert!(variant(
            "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,URI=\"audio.m3u8\"",
            &base()
        )
        .is_err());
        let parsed = parse(
            "#EXTM3U\n#EXTINF:4,\na.ts\n#EXT-X-DISCONTINUITY\n#EXTINF:4,\nb.ts\n#EXT-X-ENDLIST",
            &base(),
        )
        .unwrap();
        assert!(select(&parsed, 3., 5.).is_err());
        assert!(select(&parsed, 4., 7.).is_ok());
    }
    #[test]
    fn full_session_uses_all_vod_segments_including_discontinuities_and_real_duration() {
        let mut text =
            "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:40\n#EXT-X-MAP:URI=\"init-a.mp4\"\n".to_owned();
        for number in 0..600 {
            if number == 300 {
                text.push_str("#EXT-X-DISCONTINUITY\n#EXT-X-MAP:URI=\"init-b.mp4\"\n#EXT-X-PROGRAM-DATE-TIME:2026-10-25T16:00:00+08:00\n");
            }
            text.push_str(&format!("#EXTINF:20.25,\n{number}.m4s\n"));
        }
        text.push_str("#EXT-X-ENDLIST\n");
        let parsed = parse(&text, &base()).unwrap();
        let plan = full_session(&parsed).unwrap();
        assert_eq!(plan.base, 0.);
        assert_eq!(plan.duration, 12_150.);
        assert_eq!(plan.assets.len(), 602);
        assert_eq!(plan.local_manifest.matches("#EXTINF:").count(), 600);
        assert_eq!(
            plan.local_manifest
                .matches("#EXT-X-DISCONTINUITY\n")
                .count(),
            1
        );
        assert_eq!(plan.local_manifest.matches("#EXT-X-MAP:").count(), 2);
        assert!(!plan.local_manifest.contains("https:"));
        assert!(!plan.local_manifest.contains("signature"));
        assert!(
            select(&parsed, 6000., 6200.).is_err(),
            "ordinary clip crossing a recording break stays rejected"
        );
    }
}
