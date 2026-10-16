//! Read only this process's RSS and CPU counters; never inspect other apps.
use serde_json::{json, Value};
#[cfg(windows)]
pub fn counters() -> Value {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::{
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::{GetCurrentProcess, GetProcessTimes},
        },
    };
    unsafe {
        let process = GetCurrentProcess();
        let mut memory: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        memory.cb = std::mem::size_of_val(&memory) as u32;
        let ok = GetProcessMemoryInfo(process, &mut memory, memory.cb) != 0;
        let mut creation: FILETIME = std::mem::zeroed();
        let mut exit: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        let cpu = GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) != 0;
        let ticks = |v: FILETIME| ((v.dwHighDateTime as u64) << 32) | v.dwLowDateTime as u64;
        json!({"rss_mb":if ok{Some(memory.WorkingSetSize as f64/1048576.)}else{None},"peak_rss_mb":if ok{Some(memory.PeakWorkingSetSize as f64/1048576.)}else{None},"cpu_seconds":if cpu{Some((ticks(kernel)+ticks(user))as f64/10_000_000.)}else{None}})
    }
}
#[cfg(not(windows))]
pub fn counters() -> Value {
    json!({"rss_mb":null,"peak_rss_mb":null,"cpu_seconds":null})
}
