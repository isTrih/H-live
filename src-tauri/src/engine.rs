//! Owns the private collector process, request ordering and graceful exit.
use crate::{paths::data_directory, update_policy};
use serde_json::{json, Value};
#[cfg(windows)]
use std::os::windows::{io::AsRawHandle, process::CommandExt};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use tauri::Manager;

#[cfg(windows)]
struct ProcessJob(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
unsafe impl Send for ProcessJob {}
#[cfg(windows)]
impl ProcessJob {
    fn attach(child: &Child) -> Result<Self, String> {
        use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err("无法创建后台进程保护".into());
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(job, child.as_raw_handle() as _) == 0
            {
                CloseHandle(job);
                return Err("无法保护后台进程生命周期".into());
            }
            Ok(Self(job))
        }
    }
}
#[cfg(windows)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

pub(crate) struct Engine {
    child: Child,
    stdin: ChildStdin,
    responses: mpsc::Receiver<String>,
    serial: u64,
    #[cfg(windows)]
    _job: ProcessJob,
}

impl Engine {
    pub(crate) fn start(app: &tauri::AppHandle) -> Result<Self, String> {
        let resource = app
            .path()
            .resource_dir()
            .map_err(|_| "无法找到程序资源目录")?;
        Self::start_in(resource)
    }

    pub(crate) fn start_in(resource: PathBuf) -> Result<Self, String> {
        let mut paths = vec![resource.join("engine/DouyinMonitorEngine.exe")];
        if let Ok(executable) = std::env::current_exe() {
            if let Some(parent) = executable.parent() {
                paths.push(parent.join("engine/DouyinMonitorEngine.exe"));
            }
        }
        if cfg!(debug_assertions) {
            paths.push(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engine/DouyinMonitorEngine.exe"),
            );
        }
        let executable = paths
            .into_iter()
            .find(|path| path.is_file())
            .ok_or("消息引擎文件缺失，请重新安装应用")?;
        let data = data_directory()?;
        fs::create_dir_all(&data).map_err(|_| "无法创建数据目录")?;
        let errors = OpenOptions::new()
            .create(true)
            .append(true)
            .open(data.join("engine.log"))
            .map_err(|_| "无法写入引擎诊断日志")?;
        let mut command = Command::new(executable);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(errors))
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUTF8", "1");
        #[cfg(windows)]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW: private stdio only
        let mut child = command
            .spawn()
            .map_err(|_| "无法启动消息引擎，请检查安装文件")?;
        #[cfg(windows)]
        let job = match ProcessJob::attach(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let stdin = child.stdin.take().ok_or("后台输入通道不可用")?;
        let stdout = child.stdout.take().ok_or("后台输出通道不可用")?;
        let (sender, receiver) = mpsc::sync_channel(8);
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if sender.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            responses: receiver,
            serial: 0,
            #[cfg(windows)]
            _job: job,
        })
    }

    pub(crate) fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if self
            .child
            .try_wait()
            .map_err(|_| "无法检查引擎状态")?
            .is_some()
        {
            return Err("消息引擎已退出，请关闭并重新打开应用".into());
        }
        self.serial += 1;
        let id = self.serial;
        let request = json!({"id": id, "method": method, "params": params});
        writeln!(self.stdin, "{request}")
            .and_then(|_| self.stdin.flush())
            .map_err(|_| "无法向消息引擎发送请求")?;
        let timeout = if method == "shutdown" { 35 } else { 30 };
        let line = match self.responses.recv_timeout(Duration::from_secs(timeout)) {
            Ok(line) => line,
            Err(_) => {
                let _ = self.child.kill();
                let _ = self.child.wait();
                return Err("消息引擎响应超时或连接已断开，请重新打开应用".into());
            }
        };
        let response: Value = serde_json::from_str(&line).map_err(|_| "消息引擎返回格式错误")?;
        if response.get("id").and_then(Value::as_u64) != Some(id) {
            return Err("消息引擎响应序号不一致，请重新打开应用".into());
        }
        if response.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(response.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("消息引擎请求失败")
                .to_owned())
        }
    }

    pub(crate) fn shutdown(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.request("shutdown", json!({}));
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                if self.child.try_wait().ok().flatten().is_some() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    pub(crate) fn shutdown_for_update(&mut self) -> Result<(), String> {
        let response = self.request("shutdown", json!({}))?;
        if !update_policy::shutdown_confirmed(&response) {
            return Err("后台工作尚未安全结束，已取消更新安装".into());
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().map_err(|_| "无法确认后台退出")? {
                return if status.success() {
                    Ok(())
                } else {
                    Err("后台退出异常，已取消更新安装".into())
                };
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        Err("后台尚未退出，已取消更新安装".into())
    }
    pub(crate) fn has_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// Offline distribution check, through the same private pipes and Windows Job
// used by the desktop UI. An explicit data directory prevents touching user data.
pub(crate) fn offline_self_test() -> Result<Value, String> {
    if std::env::var_os("DOUYIN_MONITOR_HOME").is_none() {
        return Err("自检须设置独立的 DOUYIN_MONITOR_HOME 数据目录".into());
    }
    let root = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .parent()
        .ok_or("无法确定程序位置")?
        .to_path_buf();
    let mut engine = Engine::start_in(root)?;
    let child_id = engine.child.id();
    let initial = engine.request("init", json!({}))?;
    if initial.get("version").and_then(Value::as_str) != Some(env!("CARGO_PKG_VERSION")) {
        return Err("后台版本不一致".into());
    }
    if initial.get("native_version").and_then(Value::as_str) != Some(env!("CARGO_PKG_VERSION"))
        || initial.get("processing").and_then(Value::as_str) != Some("rust")
    {
        return Err("Rust 消息处理模块未正确加载或版本不一致".into());
    }
    let poll = engine.request("poll", json!({"after": 0}))?;
    if !poll.is_object() {
        return Err("后台轮询结果无效".into());
    }
    let proxy = engine.request("proxy_status", json!({}))?;
    if proxy.get("enabled").and_then(Value::as_bool).is_none() {
        return Err("代理池模块未正确加载".into());
    }
    if engine
        .request("unknown_self_test_method", json!({}))
        .is_ok()
    {
        return Err("后台没有拒绝未知请求".into());
    }
    let ready_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let ready = update_policy::readiness(engine.request("update_readiness", json!({}))?);
        if ready["ready"] == true {
            break;
        }
        if Instant::now() >= ready_deadline {
            return Err("后台更新安装门禁未正确就绪".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let response = engine.request("shutdown", json!({}))?;
    if response.get("all_workers_stopped").and_then(Value::as_bool) != Some(true) {
        return Err("后台工作线程未正常结束".into());
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let exit_code = loop {
        if let Some(status) = engine.child.try_wait().map_err(|error| error.to_string())? {
            break status.code();
        }
        if Instant::now() >= deadline {
            return Err("后台关闭后没有退出".into());
        }
        std::thread::sleep(Duration::from_millis(40));
    };
    if exit_code != Some(0) {
        return Err("后台退出码异常".into());
    }
    Ok(
        json!({"init": true, "poll": true, "proxy_status": true, "unknown_method_rejected": true,
        "update_readiness": true, "native_version": initial["native_version"], "processing": "rust",
        "shutdown": response, "child_exited": true, "child_pid": child_id,
        "windows_job_protection": cfg!(windows), "data_directory": data_directory()?}),
    )
}
