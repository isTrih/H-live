use std::path::PathBuf;

pub(crate) fn data_directory() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("DOUYIN_MONITOR_HOME") {
        return Ok(PathBuf::from(path));
    }
    std::env::var_os("LOCALAPPDATA")
        .map(|path| PathBuf::from(path).join("DouyinLiveMonitor"))
        .ok_or_else(|| "无法确定本地数据目录".to_string())
}
