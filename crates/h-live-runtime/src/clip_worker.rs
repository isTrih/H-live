//! Download selected HLS assets, then run fixed local tools without network input.
use crate::{
    clip_plan::{self, Plan},
    clips::{ClipJob, ClipService, MediaTools},
    community,
    storage::Result,
};
use futures_util::{stream, StreamExt, TryStreamExt};
use serde_json::Value;
use std::{
    ffi::OsString,
    io::{BufRead, Read},
    path::Path,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

fn check(service: &ClipService, cancel: &CancellationToken) -> Result<()> {
    if cancel.is_cancelled() || service.shared.cancel.is_cancelled() {
        return Err("切片已取消".into());
    }
    if service
        .shared
        .state
        .lock()
        .map_err(|_| "社区状态不可用")?
        .config
        .value["community_enabled"]
        != true
    {
        return Err("社区生态已关闭，切片已停止".into());
    }
    Ok(())
}
async fn fetch(
    service: &ClipService,
    cancel: &CancellationToken,
    url: &str,
    range: Option<(u64, u64)>,
) -> Result<Vec<u8>> {
    check(service, cancel)?;
    tokio::select! {biased;_=cancel.cancelled()=>Err("切片已取消".into()),result=community::fetch_media(&service.shared,url,range.map(|r|r.0),range.map(|r|r.1))=>result}
}
async fn source(
    service: &Arc<ClipService>,
    job: &ClipJob,
    cancel: &CancellationToken,
) -> Result<(String, Plan)> {
    check(service, cancel)?;
    service.update(&job.id, |j| {
        j.phase = "resolving".into();
        j.progress = 2.;
    });
    let lookup = community::replay_snapshot(&service.shared, &job.session_id)?;
    let resolved = tokio::select! {biased;_=cancel.cancelled()=>return Err("切片已取消".into()),result=lookup.fetch()=>result?};
    if resolved["available"] != true {
        return Err("此场次尚未同步可下载录播".into());
    }
    let replay = &resolved["replay"];
    if replay["media_kind"] != "hls" {
        return Err("当前原画切片仅支持 HLS VOD，暂不支持 MP4 直链".into());
    }
    let mut url =
        crate::validated_media_url(replay["play_url"].as_str().ok_or("录播缺少媒体地址")?)?;
    let identity = if let Some(id) = replay["video_id"].as_str().filter(|s| !s.is_empty()) {
        format!("xingtu:{id}")
    } else {
        use sha2::{Digest, Sha256};
        let mut stable = url.clone();
        stable.set_query(None);
        format!("hls:{:x}", Sha256::digest(stable.as_str().as_bytes()))
    };
    service.update(&job.id, |j| {
        j.phase = "planning".into();
        j.progress = 5.;
    });
    for _ in 0..3 {
        let bytes = fetch(service, cancel, url.as_str(), None).await?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("HLS 清单超过大小限制".into());
        }
        let text = String::from_utf8(bytes).map_err(|_| "HLS 清单编码无效")?;
        if let Some(next) = clip_plan::variant(&text, &url)? {
            url = next;
            continue;
        }
        let playlist = clip_plan::parse(&text, &url)?;
        return Ok((
            identity,
            if job.mode == "full_session" {
                clip_plan::full_session(&playlist)?
            } else {
                clip_plan::select(
                    &playlist,
                    job.requested_start_seconds,
                    job.requested_end_seconds,
                )?
            },
        ));
    }
    Err("HLS 清单层级超过限制".into())
}
async fn download(
    service: &Arc<ClipService>,
    job: &ClipJob,
    cancel: &CancellationToken,
    plan: &Plan,
    folder: &Path,
) -> Result<()> {
    service.update(&job.id, |j| {
        j.phase = "downloading".into();
        j.progress = 10.;
        j.downloaded_bytes = 0;
        j.total_bytes = None;
    });
    let bytes = Arc::new(AtomicU64::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let total = plan.assets.len();
    stream::iter(plan.assets.iter().cloned()).map(|asset|{
        let bytes=bytes.clone();let completed=completed.clone();let service=service.clone();let cancel=cancel.clone();let id=job.id.clone();let file=folder.join(&asset.file);
        async move{
            let mut attempt=0;
            let data=loop{
                match fetch(&service,&cancel,asset.url.as_str(),asset.range).await{
                    Ok(data)=>break data,
                    Err(error)if attempt<2&&(error.contains("HTTP 5")||error.contains("超时")||error.contains("连接失败"))=>{attempt+=1;tokio::select!{_=cancel.cancelled()=>return Err("切片已取消".into()),_=tokio::time::sleep(Duration::from_millis(250*attempt))=>{}}},
                    Err(error)=>return Err(error),
                }
            };
            let downloaded=bytes.fetch_add(data.len()as u64,Ordering::AcqRel)+data.len()as u64;
            let limit = if job.mode == "full_session" { 64_u64*1024*1024*1024 } else { 512*1024*1024 };
            if downloaded>limit{return Err(if job.mode == "full_session" { "整场录像超过 64 GiB 下载限制" } else { "此切片下载量超过 512 MiB，请缩短选区" }.into());}
            tokio::task::spawn_blocking(move||std::fs::write(file,data).map_err(|_|"写入切片分片失败".to_owned())).await.map_err(|_|"切片写入任务失败")??;
            let done=completed.fetch_add(1,Ordering::AcqRel)+1;
            service.update(&id,|j|{j.downloaded_bytes=bytes.load(Ordering::Acquire);j.progress=10.+55.*done as f64/total as f64;});
            Ok::<_,String>(())
        }
    }).buffer_unordered(2).try_collect::<Vec<_>>().await?;
    service.update(&job.id, |j| {
        j.total_bytes = Some(bytes.load(Ordering::Acquire));
        j.downloaded_bytes = bytes.load(Ordering::Acquire);
        j.progress = 65.;
    });
    std::fs::write(folder.join("input.m3u8"), &plan.local_manifest)
        .map_err(|_| "保存本地切片清单失败")?;
    Ok(())
}

pub(crate) async fn run(
    service: &Arc<ClipService>,
    job: &ClipJob,
    cancel: &CancellationToken,
) -> Result<(f64, f64)> {
    check(service, cancel)?;
    let tools = service.tools()?;
    let cache = service.shared.directory.join("clip-work");
    std::fs::create_dir_all(&cache).map_err(|_| "创建切片缓存失败")?;
    let mut identity = job.source_identity.clone();
    let mut fingerprint = job.source_fingerprint.clone();
    for refresh in 0..=1 {
        let (found, plan) = source(service, job, cancel).await?;
        let resolved;
        let job = if job.mode == "full_session" {
            resolved = service.resolve_full_session(job, plan.duration)?;
            &resolved
        } else {
            job
        };
        if identity.as_ref().is_some_and(|old| old != &found)
            || fingerprint
                .as_ref()
                .is_some_and(|old| old != &plan.fingerprint)
        {
            return Err("录播源或分片结构发生变化，请重新确认选区后创建切片".into());
        }
        identity = Some(found.clone());
        fingerprint = Some(plan.fingerprint.clone());
        service.update(&job.id, |j| {
            j.source_identity = Some(found);
            j.source_fingerprint = Some(plan.fingerprint.clone());
        });
        let folder = tempfile::Builder::new()
            .prefix("clip-")
            .tempdir_in(&cache)
            .map_err(|_| "创建切片临时目录失败")?;
        match download(service, job, cancel, &plan, folder.path()).await {
            Err(error)
                if refresh == 0 && (error.contains("HTTP 403") || error.contains("HTTP 410")) =>
            {
                continue
            }
            Err(error) => return Err(error),
            Ok(()) => {}
        }
        return mux(service, job, cancel, &tools, &plan, folder.path()).await;
    }
    Err("录播地址已失效，请重新尝试".into())
}
fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}
async fn mux(
    service: &Arc<ClipService>,
    job: &ClipJob,
    cancel: &CancellationToken,
    tools: &MediaTools,
    plan: &Plan,
    folder: &Path,
) -> Result<(f64, f64)> {
    check(service, cancel)?;
    let input = folder.join("input.m3u8");
    service.update(&job.id, |j| {
        j.phase = "probing".into();
        j.progress = 67.;
    });
    let mut probe = args(&[
        "-v",
        "error",
        "-protocol_whitelist",
        "file",
        "-allowed_extensions",
        "ALL",
        "-i",
    ]);
    probe.push(input.clone().into());
    // Whole recordings can contain millions of packets. Only inspect the opening
    // GOP; the copy mux below consumes the full manifest without decoding it.
    if job.mode == "full_session" {
        probe.extend(args(&["-read_intervals", "%+10"]));
    }
    probe.extend(args(&[
        "-select_streams",
        "v:0",
        "-show_packets",
        "-show_streams",
        "-show_format",
        "-show_entries",
        "packet=pts_time,flags:stream=codec_name,codec_type:format=start_time,duration",
        "-of",
        "json",
    ]));
    let info = execute(
        service,
        job,
        cancel,
        &tools.ffprobe,
        &probe,
        folder,
        Duration::from_secs(90),
        None,
    )
    .await?;
    let info: Value = serde_json::from_slice(&info).map_err(|_| "关键帧探测未返回有效结果")?;
    let wanted = job.requested_start_seconds - plan.base;
    let key = keyframe(&info, wanted)?;
    let actual_start = plan.base + key;
    if actual_start > job.requested_start_seconds + 0.1 {
        return Err("未找到选区之前可独立解码的关键帧，请提前起点重试".into());
    }
    let duration = job.requested_end_seconds - actual_start;
    if duration <= 0. {
        return Err("实际切片范围无效".into());
    }
    let output = Path::new(&job.output_path);
    if output.exists() {
        return Err("目标文件已存在，不会覆盖".into());
    }
    let partial = tempfile::Builder::new()
        .prefix(".qingkan-clip-")
        .suffix(".mp4.part")
        .tempfile_in(output.parent().ok_or("切片保存目录无效")?)
        .map_err(|_| "无法创建切片输出文件")?
        .into_temp_path();
    service.update(&job.id, |j| {
        j.phase = "muxing".into();
        j.progress = 70.;
        j.actual_start_seconds = Some(actual_start);
        j.actual_end_seconds = Some(job.requested_end_seconds);
    });
    let mut command = args(&[
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-progress",
        "pipe:1",
        "-protocol_whitelist",
        "file",
        "-allowed_extensions",
        "ALL",
        "-ss",
        &format!("{key:.6}"),
        "-i",
    ]);
    command.push(input.into());
    command.extend(args(&[
        "-t",
        &format!("{duration:.6}"),
        "-map",
        "0:v:0",
        "-map",
        "0:a:0?",
        "-c",
        "copy",
    ]));
    if info["streams"]
        .as_array()
        .is_some_and(|s| s.iter().any(|s| s["codec_name"] == "hevc"))
    {
        command.extend(args(&["-tag:v", "hvc1"]));
    }
    command.extend(args(&[
        "-avoid_negative_ts",
        "make_zero",
        "-movflags",
        "+faststart",
        "-f",
        "mp4",
        "-y",
    ]));
    command.push(partial.to_path_buf().into());
    execute(
        service,
        job,
        cancel,
        &tools.ffmpeg,
        &command,
        folder,
        Duration::from_secs(if job.mode == "full_session" {
            7200
        } else {
            600
        }),
        Some(duration),
    )
    .await?;
    service.update(&job.id, |j| {
        j.phase = "validating".into();
        j.progress = 97.;
    });
    let mut verify = args(&[
        "-v",
        "error",
        "-protocol_whitelist",
        "file",
        "-select_streams",
        "v:0",
        "-read_intervals",
        "%+2",
        "-show_frames",
        "-show_streams",
        "-show_format",
        "-show_entries",
        "frame=key_frame,pict_type:stream=codec_name,codec_type:format=duration",
        "-of",
        "json",
    ]);
    verify.push(partial.to_path_buf().into());
    let verified = execute(
        service,
        job,
        cancel,
        &tools.ffprobe,
        &verify,
        folder,
        Duration::from_secs(90),
        None,
    )
    .await?;
    let verified: Value = serde_json::from_slice(&verified).map_err(|_| "切片验证结果无效")?;
    let actual_duration = validate_recording_output(&verified, job)?;
    check(service, cancel)?;
    partial
        .persist_noclobber(output)
        .map_err(|_| "保存切片失败；目标文件可能已存在，原文件已保留")?;
    Ok((actual_start, actual_start + actual_duration))
}
fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|n| n.is_finite())
}
fn keyframe(info: &Value, wanted: f64) -> Result<f64> {
    let packets = info["packets"].as_array().ok_or("未探测到视频关键帧")?;
    let base = number(&info["format"]["start_time"])
        .or_else(|| {
            packets
                .iter()
                .filter_map(|p| number(&p["pts_time"]))
                .reduce(f64::min)
        })
        .ok_or("视频缺少可校准时间戳")?;
    let mut last = None;
    for packet in packets {
        if packet["flags"].as_str().is_some_and(|s| s.contains('K')) {
            if let Some(time) = number(&packet["pts_time"]) {
                let offset = time - base;
                if offset >= -0.05 && offset <= wanted + 0.02 {
                    last = Some(last.map_or(offset.max(0.), |old: f64| old.max(offset.max(0.))));
                }
            }
        }
    }
    last.ok_or_else(|| "选区前置分片未包含可验证关键帧，请提前起点后重试".into())
}
fn validate_output(info: &Value) -> Result<f64> {
    if info["frames"]
        .as_array()
        .and_then(|f| f.first())
        .is_none_or(|f| f["key_frame"] != 1)
    {
        return Err("切片开头无法验证为可独立解码关键帧，未生成正式文件".into());
    }
    number(&info["format"]["duration"])
        .filter(|n| *n > 0. && *n <= 930.)
        .ok_or_else(|| "切片文件时长无效".into())
}
fn validate_recording_output(info: &Value, job: &ClipJob) -> Result<f64> {
    if job.mode != "full_session" {
        return validate_output(info);
    }
    if info["frames"]
        .as_array()
        .and_then(|f| f.first())
        .is_none_or(|f| f["key_frame"] != 1)
    {
        return Err("整场录像开头无法验证为可独立解码关键帧".into());
    }
    let duration = number(&info["format"]["duration"])
        .filter(|n| *n > 0. && *n <= 31. * 86400. + 30.)
        .ok_or("整场录像文件时长无效")?;
    let expected = job.requested_end_seconds;
    if (duration - expected).abs() > (expected * 0.001).clamp(2., 30.) {
        return Err("整场录像输出时长与清单不一致，未生成正式文件".into());
    }
    Ok(duration)
}
async fn execute(
    service: &Arc<ClipService>,
    job: &ClipJob,
    cancel: &CancellationToken,
    executable: &Path,
    arguments: &[OsString],
    folder: &Path,
    timeout: Duration,
    progress_duration: Option<f64>,
) -> Result<Vec<u8>> {
    check(service, cancel)?;
    let mut command = std::process::Command::new(executable);
    command
        .args(arguments)
        .current_dir(folder)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for key in [
        "FFREPORT",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        command.env_remove(key);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "启动媒体工具失败，请重新安装工具组件")?;
    let stdout = child.stdout.take().ok_or("媒体工具输出不可用")?;
    let stderr = child.stderr.take().ok_or("媒体工具错误流不可用")?;
    let clone = service.clone();
    let id = job.id.clone();
    let reader = std::thread::spawn(move || -> Result<Vec<u8>> {
        if let Some(duration) = progress_duration {
            let mut reader = std::io::BufReader::new(stdout);
            let mut line = String::new();
            while reader
                .read_line(&mut line)
                .map_err(|_| "读取封装进度失败")?
                > 0
            {
                if let Some(time) = line
                    .trim()
                    .strip_prefix("out_time_us=")
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|n| n.is_finite())
                {
                    clone.update(&id, |j| {
                        j.progress = (70. + 25. * time / 1_000_000. / duration).clamp(70., 95.)
                    });
                }
                line.clear();
            }
            Ok(vec![])
        } else {
            bounded_pipe(stdout, 4 * 1024 * 1024)
        }
    });
    let errors = std::thread::spawn(move || bounded_pipe(stderr, 256 * 1024));
    let start = Instant::now();
    let mut failure = None;
    let status = loop {
        if let Err(error) = check(service, cancel) {
            failure = Some(error);
            let _ = child.kill();
            break child.wait();
        }
        if start.elapsed() > timeout {
            failure = Some("媒体工具处理超时".into());
            let _ = child.kill();
            break child.wait();
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(_) => {
                failure = Some("媒体工具运行状态不可用".into());
                let _ = child.kill();
                break child.wait();
            }
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
    };
    let output = reader.join().map_err(|_| "读取媒体工具结果失败")?;
    let _ = errors.join();
    if let Some(error) = failure {
        return Err(error);
    }
    if !status.map_err(|_| "回收媒体工具失败")?.success() {
        return Err("媒体工具无法完成此回放的原画切片，请调整选区后重试".into());
    }
    output
}
fn bounded_pipe(mut pipe: impl Read, limit: usize) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    let mut overflow = false;
    loop {
        let read = pipe.read(&mut buffer).map_err(|_| "读取媒体工具输出失败")?;
        if read == 0 {
            break;
        }
        if output.len() + read <= limit {
            output.extend_from_slice(&buffer[..read]);
        } else {
            overflow = true;
        }
    }
    if overflow {
        Err("媒体工具输出超过限制".into())
    } else {
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyframe_selection_reports_actual_earlier_start() {
        let info = serde_json::json!({"format":{"start_time":"10"},"packets":[{"pts_time":"10","flags":"K_"},{"pts_time":"12","flags":"__"},{"pts_time":"14","flags":"K_"}]});
        assert_eq!(keyframe(&info, 3.).unwrap(), 0.);
        assert_eq!(keyframe(&info, 4.5).unwrap(), 4.);
        assert!(keyframe(&serde_json::json!({"packets":[]}), 2.).is_err());
    }
    #[test]
    fn output_requires_decodable_first_keyframe_and_positive_duration() {
        assert!(validate_output(
            &serde_json::json!({"frames":[{"key_frame":0}],"format":{"duration":"5"}})
        )
        .is_err());
        assert_eq!(
            validate_output(
                &serde_json::json!({"frames":[{"key_frame":1}],"format":{"duration":"5.5"}})
            )
            .unwrap(),
            5.5
        );
    }
    #[test]
    fn whole_recording_validation_accepts_long_mp4_but_rejects_truncated_output() {
        let job: ClipJob = serde_json::from_value(serde_json::json!({"mode":"full_session","id":"11111111111111111111111111111111","session_id":"123","title":"整场","status":"running","phase":"validating","progress":97.,"downloaded_bytes":0,"total_bytes":null,"output_path":"whole.mp4","error":null,"requested_start_seconds":0.,"requested_end_seconds":14400.,"actual_start_seconds":null,"actual_end_seconds":null,"attempt":1,"created_at":"2026-10-26T10:00:00+08:00","updated_at":"2026-10-26T10:00:00+08:00"})).unwrap();
        let good = serde_json::json!({"frames":[{"key_frame":1}],"format":{"duration":"14400.1"}});
        assert!(
            validate_output(&good).is_err(),
            "ordinary clips keep the existing duration bound"
        );
        assert_eq!(validate_recording_output(&good, &job).unwrap(), 14400.1);
        assert!(validate_recording_output(
            &serde_json::json!({"frames":[{"key_frame":1}],"format":{"duration":"900"}}),
            &job
        )
        .is_err());
    }

    #[test]
    #[ignore = "requires bundled ffmpeg/ffprobe; synthetic local HLS only, no network"]
    fn whole_recording_mux_preserves_both_sides_of_a_timestamp_discontinuity() {
        struct Protector;
        impl crate::storage::SecretProtector for Protector {
            fn protect(&self, bytes: &[u8]) -> Result<Vec<u8>> {
                Ok(bytes.iter().map(|b| b ^ 0xa5).collect())
            }
            fn unprotect(&self, bytes: &[u8]) -> Result<Vec<u8>> {
                self.protect(bytes)
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let runtime = crate::Runtime::open_with_transport_and_secrets(
            directory.path().into(),
            Arc::new(crate::transport::LiveTransport),
            Arc::new(Protector),
        )
        .unwrap();
        runtime
            .request(
                "community_configure",
                serde_json::json!({"enabled":true,"api_key":"synthetic-local-whole"}),
            )
            .unwrap();
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        runtime
            .configure_media_tools(root.join("packaging/tools/media"))
            .unwrap();
        let tools = runtime.clips.tools().unwrap();
        let source = directory.path().join("source.ts");
        let status = std::process::Command::new(&tools.ffmpeg)
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=64x36:rate=10",
                "-t",
                "4",
                "-pix_fmt",
                "yuv420p",
                "-c:v",
                "libopenh264",
                "-g",
                "10",
                "-f",
                "mpegts",
            ])
            .arg(&source)
            .status()
            .unwrap();
        assert!(status.success());
        let parsed = clip_plan::parse("#EXTM3U\n#EXTINF:4,\nsource.ts\n#EXT-X-DISCONTINUITY\n#EXT-X-PROGRAM-DATE-TIME:2026-10-26T12:00:00+08:00\n#EXTINF:4,\nsource.ts\n#EXT-X-ENDLIST\n", &url::Url::parse("https://v26-star.xingtu.cn/local/index.m3u8").unwrap()).unwrap();
        let plan = clip_plan::full_session(&parsed).unwrap();
        let working = directory.path().join("working");
        std::fs::create_dir(&working).unwrap();
        for asset in &plan.assets {
            std::fs::copy(&source, working.join(&asset.file)).unwrap();
        }
        std::fs::write(working.join("input.m3u8"), &plan.local_manifest).unwrap();
        let created = runtime
            .request(
                "clip_create",
                serde_json::json!({"mode":"full_session","session_id":"123"}),
            )
            .unwrap();
        let original = runtime
            .clips
            .job(created["job_id"].as_str().unwrap())
            .unwrap();
        let job = runtime
            .clips
            .resolve_full_session(&original, plan.duration)
            .unwrap();
        let result = runtime
            .rt
            .block_on(mux(
                &runtime.clips,
                &job,
                &CancellationToken::new(),
                &tools,
                &plan,
                &working,
            ))
            .unwrap();
        assert_eq!(result.0, 0.);
        assert!(
            (result.1 - 8.).abs() < 0.5,
            "both original four-second pieces must be present: {result:?}"
        );
        assert!(Path::new(&job.output_path).is_file());
        runtime.shutdown().unwrap();
    }

    #[test]
    #[ignore = "requires the fixed local media component; launches no network requests"]
    fn cancels_a_running_ffmpeg_and_reaps_it_within_two_seconds() {
        struct Protector;
        impl crate::storage::SecretProtector for Protector {
            fn protect(&self, bytes: &[u8]) -> Result<Vec<u8>> {
                Ok(bytes.iter().map(|b| b ^ 0xa5).collect())
            }
            fn unprotect(&self, bytes: &[u8]) -> Result<Vec<u8>> {
                self.protect(bytes)
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let runtime = crate::Runtime::open_with_transport_and_secrets(
            directory.path().into(),
            Arc::new(crate::transport::LiveTransport),
            Arc::new(Protector),
        )
        .unwrap();
        runtime
            .request(
                "community_configure",
                serde_json::json!({"enabled":true,"api_key":"synthetic-process-cancel"}),
            )
            .unwrap();
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        runtime
            .configure_media_tools(root.join("packaging/tools/media"))
            .unwrap();
        let tools = runtime.clips.tools().unwrap();
        let job = ClipJob {
            mode: "clip".into(),
            id: "11111111111111111111111111111111".into(),
            session_id: "123".into(),
            title: "process cancellation fixture".into(),
            status: "running".into(),
            phase: "muxing".into(),
            progress: 70.,
            downloaded_bytes: 0,
            total_bytes: None,
            output_path: directory.path().join("unused.mp4").to_string_lossy().into(),
            error: None,
            requested_start_seconds: 0.,
            requested_end_seconds: 30.,
            actual_start_seconds: None,
            actual_end_seconds: None,
            attempt: 1,
            created_at: String::new(),
            updated_at: String::new(),
            source: "manual".into(),
            bookmark_id: None,
            request_key: None,
            anchor_id: String::new(),
            anchor_name: String::new(),
            session_date: String::new(),
            source_identity: None,
            source_fingerprint: None,
        };
        let cancel = CancellationToken::new();
        let signal = cancel.clone();
        let started = Instant::now();
        let result = runtime.rt.block_on(async {
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(180)).await;
                signal.cancel();
            });
            execute(
                &runtime.clips,
                &job,
                &cancel,
                &tools.ffmpeg,
                &args(&[
                    "-nostdin",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-re",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=black:s=16x16:r=1",
                    "-t",
                    "30",
                    "-f",
                    "null",
                    "-",
                ]),
                directory.path(),
                Duration::from_secs(5),
                None,
            )
            .await
        });
        assert!(result.unwrap_err().contains("取消"));
        assert!(started.elapsed() < Duration::from_secs(2));
        runtime.shutdown().unwrap();
    }
}
