//! Bounded compressed-packet cache. GPU/decoder surfaces are separate budgets.
//! mpv owns all I/O: seek cancels obsolete HLS input and Player::drop joins it.
//! No disk cache or detached fetcher survives a source change or signed URL.

/// At 8 Mbit/s the forward budget holds about 96 seconds. The time limit also
/// bounds low-bitrate read-ahead; the byte limits bound high-bitrate recordings.
pub(crate) const OPTIONS: &[(&str, &str)] = &[
    ("cache", "yes"),
    ("cache-secs", "120"),
    ("cache-on-disk", "no"),
    ("demuxer-thread", "yes"),
    ("demuxer-max-bytes", "96MiB"),
    ("demuxer-max-back-bytes", "16MiB"),
    ("demuxer-readahead-secs", "120"),
    ("demuxer-hysteresis-secs", "60"),
    ("demuxer-hysteresis-bytes", "32MiB"),
    ("cache-pause", "yes"),
    ("cache-pause-wait", "2"),
    ("cache-pause-initial", "yes"),
];

/// FFmpeg keeps one current and one next segment input per active HLS playlist.
/// This avoids relying on HTTP-version auto-detection, without a worker pool.
pub(crate) const HLS_OPTIONS: &str = "http_persistent=1,http_multiple=1";
pub(crate) const FORWARD_BYTES: u64 = 96 * 1024 * 1024;
pub(crate) const BACK_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const AHEAD_SECONDS: u64 = 120;

// Stable libmpv client.h ABI (MPV_FORMAT_NODE). Only numeric cache fields are
// copied; arbitrary metadata/URLs never leave the native player in diagnostics.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) union NodeData {
    pub flag: i32,
    pub integer: i64,
    pub number: f64,
    pub list: *mut NodeList,
}
#[repr(C)]
pub(crate) struct Node { pub data: NodeData, pub format: i32 }
#[repr(C)]
pub(crate) struct NodeList {
    pub count: i32,
    pub values: *mut Node,
    pub keys: *mut *mut std::ffi::c_char,
}
impl Default for Node {
    fn default() -> Self { Self { data: NodeData { integer: 0 }, format: 0 } }
}

/// `node` is an owned result from mpv_get_property and remains alive until this
/// function returns. The caller must then call mpv_free_node_contents exactly once.
pub(crate) unsafe fn cache_fields(node: &Node) -> serde_json::Value {
    use std::ffi::CStr;
    let mut fields = serde_json::Map::new();
    if node.format != 8 || node.data.list.is_null() { return fields.into(); }
    let list = &*node.data.list;
    if !(0..=64).contains(&list.count) || list.values.is_null() || list.keys.is_null() { return fields.into(); }
    for index in 0..list.count as usize {
        let pointer = *list.keys.add(index);
        if pointer.is_null() { continue; }
        let key = CStr::from_ptr(pointer).to_bytes();
        let output = match key {
            b"fw-bytes" => "forward_bytes",
            b"total-bytes" => "total_bytes",
            b"raw-input-rate" => "input_bytes_per_second",
            b"underrun" => "underrun",
            b"eof-cached" => "eof_cached",
            _ => continue,
        };
        let value = &*list.values.add(index);
        let value = match (key, value.format) {
            (b"underrun" | b"eof-cached", 3) => Some(serde_json::json!(value.data.flag != 0)),
            (b"fw-bytes" | b"total-bytes" | b"raw-input-rate", 4) if value.data.integer >= 0 => Some(serde_json::json!(value.data.integer)),
            _ => None,
        };
        if let Some(value) = value { fields.insert(output.into(), value); }
    }
    fields.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policy_bounds_both_time_and_bytes_without_disk_spill() {
        let option = |key| OPTIONS.iter().find(|(name, _)| *name == key).unwrap().1;
        assert_eq!(option("cache-on-disk"), "no");
        assert_eq!(option("demuxer-max-bytes"), format!("{}MiB", FORWARD_BYTES / 1024 / 1024));
        assert_eq!(option("demuxer-max-back-bytes"), format!("{}MiB", BACK_BYTES / 1024 / 1024));
        assert_eq!(option("cache-secs"), AHEAD_SECONDS.to_string());
        assert_eq!(option("demuxer-readahead-secs"), AHEAD_SECONDS.to_string());
        assert!(FORWARD_BYTES + BACK_BYTES <= 112 * 1024 * 1024);
        assert_eq!(option("cache-pause-wait"), "2");
        assert_eq!(HLS_OPTIONS, "http_persistent=1,http_multiple=1");
    }
    #[test]
    fn cache_diagnostics_only_copy_whitelisted_typed_nonnegative_fields() {
        use std::ffi::CString;
        let labels: Vec<_> = ["fw-bytes", "total-bytes", "underrun", "eof-cached", "private-url"]
            .into_iter().map(|key| CString::new(key).unwrap()).collect();
        let mut keys: Vec<_> = labels.iter().map(|key| key.as_ptr() as *mut _).collect();
        let mut values = [
            Node {data:NodeData{integer:1234},format:4},
            Node {data:NodeData{integer:-1},format:4},
            Node {data:NodeData{flag:1},format:3},
            Node {data:NodeData{number:f64::NAN},format:5},
            Node {data:NodeData{integer:9876},format:4},
        ];
        let mut list = NodeList { count: values.len() as i32, values: values.as_mut_ptr(), keys: keys.as_mut_ptr() };
        let node = Node {data:NodeData{list:&mut list},format:8};
        assert_eq!(unsafe { cache_fields(&node) }, serde_json::json!({"forward_bytes":1234,"underrun":true}));
    }
}
