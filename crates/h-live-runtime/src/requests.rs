//! Existing desktop IPC contract, now handled directly in Rust.
use crate::{
    maintenance, proxy,
    runtime::{id_list, proxy_snapshot, subscription_snapshot, Job, Runtime, VERSION},
    storage::{self, Result},
};
use futures_util::{stream, StreamExt};
use qingkan_core::{
    archive::{ArchiveDeleteFilter, CONTENT_TYPES},
    export::{self, CancelToken, ExportOptions},
};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::atomic::Ordering};

impl Runtime {
    pub fn request(&self, method: &str, p: Value) -> Result<Value> {
        if !p.is_object() || method.len() > 80 {
            return Err("请求格式无效".into());
        }
        if method == "shutdown" {
            return self.shutdown();
        }
        if method == "cancel_update" {
            self.quiescing.store(false, Ordering::Release);
            return self.resume_update();
        }
        self.ensure_open()?;
        let mutation = !matches!(
            method,
            "init"
                | "poll"
                | "recent_messages"
                | "archive_anchors"
                | "archive_sessions"
                | "quality_sessions"
                | "quality_detail"
                | "quality_settings"
                | "proxy_status"
                | "list_proxy_subscriptions"
                | "list_imported_proxies"
                | "archive_delete_status"
                | "viewer_refresh_status"
                | "update_readiness"
        );
        let _gate = if mutation {
            Some(self.action.lock().map_err(|_| "后台状态不可用")?)
        } else {
            None
        };
        self.ensure_open()?;
        match method {
            "init" => Ok(
                json!({"version":VERSION,"native_version":VERSION,"processing":"rust","anchors":self.anchors(),"archive_anchors":self.shared.archive.list_anchors().map_err(|_|"读取历史主播失败")?,"settings":self.settings(),"metrics":self.shared.metrics(),"cursor":self.shared.state.lock().unwrap().seq}),
            ),
            "poll" => {
                let after = p
                    .get("after")
                    .map(|v| v.as_u64().ok_or("游标无效"))
                    .transpose()?
                    .unwrap_or(0);
                let limit = p
                    .get("limit")
                    .map(|v| {
                        v.as_u64()
                            .filter(|n| (1..=300).contains(n))
                            .ok_or("分页参数无效")
                    })
                    .transpose()?
                    .unwrap_or(300) as usize;
                let (events, cursor, dropped, jobs) = {
                    let s = self.shared.state.lock().unwrap();
                    let events = s
                        .events
                        .iter()
                        .filter(|v| v["seq"].as_u64().unwrap_or(0) > after)
                        .take(limit)
                        .cloned()
                        .collect::<Vec<_>>();
                    let cursor = events
                        .last()
                        .and_then(|v| v["seq"].as_u64())
                        .unwrap_or(s.seq);
                    let dropped = s
                        .events
                        .front()
                        .and_then(|v| v["seq"].as_u64())
                        .unwrap_or(s.seq + 1)
                        .saturating_sub(after.saturating_add(1));
                    let jobs = s.jobs.values().map(|j| j.value.clone()).collect::<Vec<_>>();
                    (events, cursor, dropped, jobs)
                };
                Ok(
                    json!({"events":events,"cursor":cursor,"dropped":dropped,"anchors":self.anchors(),"metrics":self.shared.metrics(),"export_jobs":jobs}),
                )
            }
            "archive_anchors" => serde_json::to_value(
                self.shared
                    .archive
                    .list_anchors()
                    .map_err(|_| "读取历史主播失败")?,
            )
            .map_err(|_| "历史主播格式无效".into()),
            "recent_messages" => self
                .shared
                .archive
                .recent_messages(p)
                .map_err(|_| "读取实时消息失败，请检查筛选条件".into()),
            "quality_sessions" | "quality_detail" | "quality_settings" | "quality_rebuild"
            | "quality_pause" | "quality_configure" | "quality_commands" => {
                if self.shared.state.lock().unwrap().deleting && mutation {
                    return Err("消息删除仍在执行".into());
                }
                let mut v = self
                    .shared
                    .archive
                    .quality_request(
                        method.trim_start_matches("quality_"),
                        p,
                        CancelToken::default(),
                    )
                    .map_err(|_| "分析请求失败，请检查范围和参数")?;
                v["worker"] = json!({"available":true,"retrying":self.shared.quality_error.load(Ordering::Acquire)});
                Ok(v)
            }
            "archive_sessions" | "preview_archive_delete" | "execute_archive_delete" => {
                let mut selection = p.clone();
                selection.as_object_mut().unwrap().remove("limit");
                selection.as_object_mut().unwrap().remove("offset");
                let selection: ArchiveDeleteFilter =
                    serde_json::from_value(selection).map_err(|_| "归档范围无效")?;
                if method == "archive_sessions" {
                    let limit = p["limit"].as_u64().unwrap_or(200) as usize;
                    let offset = p["offset"].as_u64().unwrap_or(0) as usize;
                    return self
                        .shared
                        .archive
                        .list_sessions(&selection, limit, offset)
                        .map_err(|_| "读取场次失败，请检查范围".into());
                }
                let report = self
                    .shared
                    .archive
                    .preview_delete(&selection)
                    .map_err(|_| "请选择有效主播、月份和场次范围")?;
                let reason = self.delete_reason(&selection.anchor_ids);
                if method == "preview_archive_delete" {
                    let mut report = report;
                    report["ready"] = json!(reason.is_empty());
                    report["blocked_reason"] = json!(reason);
                    return Ok(report);
                }
                if !reason.is_empty() {
                    return Err(reason);
                }
                self.delete_job(selection)
            }
            "archive_delete_status" => {
                let s = self.shared.state.lock().unwrap();
                Ok(s.deletions
                    .get(p["job_id"].as_str().unwrap_or(""))
                    .ok_or("找不到此删除任务")?
                    .value
                    .clone())
            }
            "add_anchors" => {
                let raw = if let Some(s) = p["values"].as_str() {
                    s.to_owned()
                } else if let Some(a) = p["values"].as_array() {
                    a.iter()
                        .map(|v| v.as_str().unwrap_or(""))
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    return Err("请输入直播间链接或 ID".into());
                };
                if raw.len() > 200_000 {
                    return Err("导入内容过长".into());
                }
                let raw = raw.replace("https://", "\nhttps://");
                let mut valid = vec![];
                let mut invalid = vec![];
                for token in raw
                    .split(|c: char| {
                        c.is_whitespace() || c == ',' || c == '，' || c == ';' || c == '；'
                    })
                    .filter(|s| !s.is_empty())
                {
                    match storage::room_id(token) {
                        Ok(rid) => {
                            if !valid.contains(&rid) {
                                valid.push(rid)
                            }
                        }
                        Err(_) => invalid.push("无法识别的链接或 ID"),
                    }
                }
                let mut s = self.shared.state.lock().unwrap();
                let mut config = s.config.value.clone();
                let mut added = vec![];
                for rid in valid {
                    if config["anchors"].get(&rid).is_none() {
                        config["anchors"][&rid] = json!(format!("主播 {rid}"));
                        added.push(rid)
                    }
                }
                if config["anchors"].as_object().unwrap().len() > 500 {
                    return Err("最多可配置 500 个监控任务".into());
                }
                s.config.replace(config)?;
                Ok(json!({"added":added,"invalid":invalid}))
            }
            "start" | "stop" | "remove" => {
                let rids = id_list(&p["rids"], true)?;
                {
                    let s = self.shared.state.lock().unwrap();
                    if rids
                        .iter()
                        .any(|r| s.config.value["anchors"].get(r).is_none())
                    {
                        return Err("部分直播间不在列表中".into());
                    }
                }
                if method == "start" {
                    return self.start(rids);
                }
                let result = self.stop(&rids)?;
                if method == "remove" {
                    let mut s = self.shared.state.lock().unwrap();
                    let mut config = s.config.value.clone();
                    for rid in &rids {
                        config["anchors"].as_object_mut().unwrap().remove(rid);
                        if let Some(m) = config["anchor_metadata"].as_object_mut() {
                            m.remove(rid);
                        }
                        if let Some(a) = config["pc_unsupported_rids"].as_array_mut() {
                            a.retain(|v| v != rid)
                        }
                    }
                    s.config.replace(config)?;
                    for rid in &rids {
                        s.rooms.remove(rid);
                    }
                }
                Ok(result)
            }
            "rename" => {
                let rid = storage::room_id(p["rid"].as_str().ok_or("主播 ID 无效")?)?;
                let name = p["name"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty() && s.chars().count() <= 200)
                    .ok_or("名称须为 1 至 200 字符")?
                    .trim();
                let mut s = self.shared.state.lock().unwrap();
                if s.config.value["anchors"].get(&rid).is_none() {
                    return Err("直播间不在列表中".into());
                }
                let mut config = s.config.value.clone();
                config["anchors"][&rid] = json!(name);
                s.config.replace(config)?;
                if let Some(r) = s.rooms.get_mut(&rid) {
                    r.value["name"] = json!(name)
                }
                Ok(json!({"renamed":rid}))
            }
            "set_pc_unsupported" => {
                let rid = storage::room_id(p["rid"].as_str().ok_or("主播 ID 无效")?)?;
                let flag = p["unsupported"].as_bool().ok_or("标记参数无效")?;
                let mut s = self.shared.state.lock().unwrap();
                if s.config.value["anchors"].get(&rid).is_none() {
                    return Err("直播间不在列表中".into());
                }
                let mut config = s.config.value.clone();
                let mut values = config["pc_unsupported_rids"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                values.retain(|v| v != &rid);
                if flag {
                    values.push(json!(rid));
                }
                config["pc_unsupported_rids"] = json!(values);
                s.config.replace(config)?;
                Ok(
                    json!({"rid":rid,"pc_unsupported":flag,"pc_support_source":if flag{"manual"}else{""}}),
                )
            }
            "configure_proxy" => {
                let enabled = p["enabled"].as_bool().ok_or("enabled 须为布尔值")?;
                let mut s = self.shared.state.lock().unwrap();
                if s.rooms.values().any(|r| r.value["active"] == true) {
                    return Err("请先停止监控再切换出口策略".into());
                }
                let source = p["source"].as_str().unwrap_or(&s.pool.source).to_owned();
                if !["public", "imported"].contains(&source.as_str()) {
                    return Err("代理来源无效".into());
                }
                let direct = p
                    .get("include_direct")
                    .map(|v| v.as_bool().ok_or("直连选项无效"))
                    .transpose()?
                    .unwrap_or(s.pool.include_direct);
                let mut config = s.config.value.clone();
                config["proxy_pool"] = json!({"provider":"scdn","schema_version":1,"enabled":enabled,"source":source,"include_direct":direct,"protocol":"https","api_url":"https://proxy.scdn.io/api/get_proxy.php"});
                s.config.replace(config)?;
                if source != s.pool.source {
                    let keys = s
                        .pool
                        .routes
                        .keys()
                        .filter(|k| k.as_str() != "direct")
                        .cloned()
                        .collect::<Vec<_>>();
                    for k in keys {
                        if let Some(r) = s.pool.routes.remove(&k) {
                            r.retired.store(true, Ordering::Release);
                            s.pool.retired.push(r)
                        }
                    }
                    s.pool.candidates.clear();
                }
                s.pool.enabled = enabled;
                s.pool.source = source.clone();
                s.pool.include_direct = direct;
                drop(s);
                maintenance::reconcile(&self.shared);
                Ok(json!({"enabled":enabled,"source":source,"restart_required":true}))
            }
            "proxy_status" => {
                let s = self.shared.state.lock().unwrap();
                Ok(proxy_snapshot(&s))
            }
            "preview_proxies" | "import_proxies" => {
                let text = p["text"].as_str().ok_or("请输入代理信息")?;
                let protocol = p["default_protocol"].as_str().unwrap_or("http");
                let mut s = self.shared.state.lock().unwrap();
                let existing = s.config.value["imported_proxies"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|v| proxy::ProxyRecord::from_value(v, "http"))
                    .collect::<Result<Vec<_>>>()?;
                let (records, preview) = proxy::parse(text, protocol, &existing)?;
                if method == "import_proxies" {
                    let mut all = existing
                        .into_iter()
                        .map(|r| (r.id.clone(), r))
                        .collect::<std::collections::BTreeMap<_, _>>();
                    for record in records {
                        if all.get(&record.id).is_some_and(|r| r != &record) {
                            s.pool.failed.remove(&record.id);
                            s.pool.checked.remove(&record.id);
                        }
                        all.insert(record.id.clone(), record);
                    }
                    if all.len() > 5000 {
                        return Err("最多保存 5000 条代理".into());
                    }
                    let mut config = s.config.value.clone();
                    config["imported_proxies"] = json!(all.into_values().collect::<Vec<_>>());
                    s.config.replace(config)?;
                    drop(s);
                    maintenance::reconcile(&self.shared);
                }
                Ok(preview)
            }
            "list_imported_proxies" => {
                let s = self.shared.state.lock().unwrap();
                let records = s.config.value["imported_proxies"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|v| proxy::ProxyRecord::from_value(v, "http"))
                    .collect::<Result<Vec<_>>>()?;
                let items = records
                    .iter()
                    .map(|r| {
                        r.public(if s.pool.failed.contains(&r.id) {
                            "failed"
                        } else if r.expired() {
                            "expired"
                        } else if s.pool.routes.contains_key(&r.id) {
                            "available"
                        } else {
                            "unchecked"
                        })
                    })
                    .collect::<Vec<_>>();
                Ok(json!({"items":items,"total":items.len(),"source":s.pool.source}))
            }
            "remove_imported_proxies" => {
                let ids = p["ids"]
                    .as_array()
                    .filter(|a| a.len() <= 5000)
                    .ok_or("代理列表无效")?;
                let mut s = self.shared.state.lock().unwrap();
                let mut config = s.config.value.clone();
                let values = config["imported_proxies"]
                    .as_array_mut()
                    .ok_or("代理配置无效")?;
                let before = values.len();
                values.retain(|v| !ids.contains(&v["id"]));
                let removed = before - values.len();
                s.config.replace(config)?;
                drop(s);
                maintenance::reconcile(&self.shared);
                Ok(json!({"removed":removed}))
            }
            "refresh_proxies" => {
                let accepted = self.rt.block_on(async { maintenance::probe(&self.shared) });
                Ok(
                    json!({"accepted":accepted,"refresh":self.shared.state.lock().unwrap().pool.refresh}),
                )
            }
            "list_proxy_subscriptions" => {
                Ok(json!({"items":subscription_snapshot(&self.shared.state.lock().unwrap())}))
            }
            "add_proxy_subscription" => {
                let raw = p["url"]
                    .as_str()
                    .filter(|s| s.len() <= 4096)
                    .ok_or("订阅 URL 无效")?;
                let u = url::Url::parse(raw).map_err(|_| "订阅 URL 无效")?;
                if u.scheme() != "https"
                    || u.port_or_known_default() != Some(443)
                    || !u.username().is_empty()
                    || u.password().is_some()
                    || u.fragment().is_some()
                    || u.host_str().is_none()
                {
                    return Err("订阅仅支持公共 HTTPS 地址".into());
                }
                if let Some(ip) = u.host_str().and_then(|s| s.parse().ok()) {
                    if !proxy::public_ip(ip) {
                        return Err("订阅不接受内网地址".into());
                    }
                }
                let minutes = p["interval_minutes"].as_u64().unwrap_or(60);
                if !(5..=1440).contains(&minutes) {
                    return Err("订阅刷新间隔须为 5 至 1440 分钟".into());
                }
                let id = proxy::digest(u.as_str());
                let mut s = self.shared.state.lock().unwrap();
                let mut config = s.config.value.clone();
                let a = config["proxy_subscriptions"]
                    .as_array_mut()
                    .ok_or("订阅配置无效")?;
                if a.len() >= 50 {
                    return Err("最多保存 50 个订阅".into());
                }
                if a.iter().any(|v| v["id"] == id) {
                    return Err("此订阅已存在".into());
                }
                a.push(json!({"id":id,"url":u.as_str(),"enabled":true,"interval_minutes":minutes,"records":[],"updated_at":""}));
                s.config.replace(config)?;
                drop(s);
                self.rt
                    .block_on(async { maintenance::refresh_subscription(&self.shared, &id) });
                Ok(json!({"id":id}))
            }
            "refresh_proxy_subscription" => {
                let id = p["id"].as_str().ok_or("订阅标识无效")?;
                Ok(
                    json!({"accepted":self.rt.block_on(async{maintenance::refresh_subscription(&self.shared,id)})}),
                )
            }
            "remove_proxy_subscription" => {
                let id = p["id"].as_str().ok_or("订阅标识无效")?;
                let mut s = self.shared.state.lock().unwrap();
                let mut config = s.config.value.clone();
                let a = config["proxy_subscriptions"]
                    .as_array_mut()
                    .ok_or("订阅配置无效")?;
                let before = a.len();
                a.retain(|v| v["id"] != id);
                let removed = before - a.len();
                s.config.replace(config)?;
                s.subscriptions.remove(id);
                if let Some(cancel) = s.subscription_tokens.remove(id) {
                    cancel.cancel();
                }
                drop(s);
                maintenance::reconcile(&self.shared);
                Ok(json!({"removed":removed}))
            }
            "refresh_viewers" => self.refresh_viewers(),
            "viewer_refresh_status" => Ok(self.shared.state.lock().unwrap().refresh.clone()),
            "export" => self.export_job(p, false),
            "quality_export" => self.export_job(p, true),
            "cancel_export" => {
                let s = self.shared.state.lock().unwrap();
                s.jobs
                    .get(p["job_id"].as_str().unwrap_or(""))
                    .ok_or("找不到导出任务")?
                    .cancel
                    .cancel();
                Ok(json!({"cancel_requested":true}))
            }
            "update_readiness" => Ok(self.update_readiness()),
            "prepare_update" => self.prepare_update(&p),
            "cancel_update" => self.resume_update(),
            _ => Err("未知 RPC 方法".into()),
        }
    }
    fn settings(&self) -> Value {
        let s = self.shared.state.lock().unwrap();
        json!({"max_tasks":500,"data_dir":self.shared.directory,"proxy_enabled":s.pool.enabled,"proxy_source":s.pool.source,"proxy_include_direct":s.pool.include_direct,"imported_proxy_count":s.config.value["imported_proxies"].as_array().map(|a|a.len()).unwrap_or(0),"proxy":{"mode":if s.pool.enabled{"pool"}else{"direct"},"configured":true,"supported":true}})
    }
    fn delete_reason(&self, anchors: &[String]) -> String {
        let s = self.shared.state.lock().unwrap();
        if s.deleting {
            return "消息删除仍在执行".into();
        }
        if s.rooms.values().any(|r| {
            r.value["active"] == true
                && (r.value["anchor_id"].as_str().is_none_or(|id| id.is_empty())
                    || anchors.iter().any(|a| r.value["anchor_id"] == a.as_str()))
        }) {
            return "请先停止所选主播及尚未识别身份的任务".into();
        }
        if s.jobs.values().any(|j| j.value["status"] == "running") {
            return "导出仍在进行，请稍后删除".into();
        }
        String::new()
    }
    fn delete_job(&self, selection: ArchiveDeleteFilter) -> Result<Value> {
        let id = format!("{:032x}", rand::random::<u128>());
        let cancel = CancelToken::default();
        {
            let mut s = self.shared.state.lock().unwrap();
            if s.deletions.len() >= 50 {
                s.deletions.retain(|_, j| j.value["status"] == "running");
            }
            s.deleting = true;
            s.deletions.insert(
                id.clone(),
                Job {
                    value: json!({"job_id":id,"status":"running","report":null,"error":null}),
                    cancel: cancel.clone(),
                },
            );
        }
        let shared = self.shared.clone();
        let key = id.clone();
        self.spawn(async move {
            let a = shared.archive.clone();
            let result = tokio::task::spawn_blocking(move || {
                a.execute_delete_cancellable(&selection, move || cancel.is_cancelled())
            })
            .await;
            let mut s = shared.state.lock().unwrap();
            if let Some(job) = s.deletions.get_mut(&key) {
                match result {
                    Ok(Ok(report)) => {
                        job.value["status"] = json!("completed");
                        job.value["report"] = report
                    }
                    _ => {
                        job.value["status"] = json!(if job.cancel.is_cancelled() {
                            "cancelled"
                        } else {
                            "error"
                        });
                        job.value["error"] = json!("删除未完成，未提交的修改已回滚");
                    }
                }
            }
            s.deleting = false;
        });
        Ok(json!({"job_id":id,"accepted":true}))
    }
    fn export_job(&self, p: Value, analysis: bool) -> Result<Value> {
        let anchors = if analysis {
            id_list(&json!([p["anchor_id"]]), false)?
        } else {
            id_list(&p["anchor_ids"], false)?
        };
        let month = p["month"].as_str().unwrap_or("").to_owned();
        if !analysis || !month.is_empty() {
            qingkan_core::archive::month_bounds(&month).map_err(|_| "月份须为 YYYY-MM")?;
        }
        let format = p["format"].as_str().unwrap_or("xlsx").to_owned();
        if !["csv", "xlsx"].contains(&format.as_str()) {
            return Err("仅支持 CSV 或 XLSX".into());
        }
        let output = p["output_path"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() < 32768)
            .ok_or("缺少导出目标路径")?;
        let output = PathBuf::from(output);
        if !output.is_absolute() {
            return Err("导出必须使用完整路径".into());
        }
        let types = if p["types"].is_null() {
            CONTENT_TYPES
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        } else {
            p["types"]
                .as_array()
                .filter(|a| !a.is_empty())
                .ok_or("请选择消息类型")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .filter(|s| CONTENT_TYPES.contains(s))
                        .map(str::to_owned)
                        .ok_or("消息类型无效")
                })
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let id = format!("{:032x}", rand::random::<u128>());
        let cancel = CancelToken::default();
        {
            let mut s = self.shared.state.lock().unwrap();
            if s.deleting {
                return Err("消息删除仍在执行".into());
            }
            if s.jobs
                .values()
                .filter(|j| j.value["status"] == "running")
                .count()
                >= 2
            {
                return Err("最多同时运行两个导出任务".into());
            }
            if s.jobs.len() >= 100 {
                s.jobs.retain(|_, j| j.value["status"] == "running");
            }
            s.jobs.insert(id.clone(),Job{value:json!({"id":id,"job_id":id,"status":"running","kind":if analysis{"session_analysis"}else{"messages"},"anchor_ids":anchors,"report":null,"error":null}),cancel:cancel.clone()});
        }
        let shared = self.shared.clone();
        let key = id.clone();
        let directory = shared.directory.clone();
        self.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                if analysis {
                    export::export_quality(
                        &directory,
                        &anchors[0],
                        &month,
                        &output,
                        &format,
                        &cancel,
                    )
                    .map_err(|_| ())
                } else {
                    export::export_month(
                        &directory,
                        ExportOptions {
                            anchor_ids: anchors,
                            month,
                            output_path: output,
                            format,
                            types,
                        },
                        &cancel,
                    )
                    .map(|r| serde_json::to_value(r).unwrap())
                    .map_err(|_| ())
                }
            })
            .await;
            let mut s = shared.state.lock().unwrap();
            if let Some(job) = s.jobs.get_mut(&key) {
                match result {
                    Ok(Ok(report)) => {
                        job.value["status"] = json!("completed");
                        job.value["report"] = report
                    }
                    _ => {
                        job.value["status"] = json!(if job.cancel.is_cancelled() {
                            "cancelled"
                        } else {
                            "error"
                        });
                        job.value["error"] =
                            json!("导出未完成，请检查路径、空间或参数；原目标文件已保留");
                    }
                }
            }
        });
        Ok(json!({"job_id":id,"id":id}))
    }
    fn refresh_viewers(&self) -> Result<Value> {
        let (id, targets) = {
            let mut s = self.shared.state.lock().unwrap();
            if s.refresh["active"] == true {
                let mut v = s.refresh.clone();
                v["accepted"] = json!(false);
                return Ok(v);
            }
            let id = (s.refresh["id"]
                .as_str()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0)
                + 1)
            .to_string();
            let targets = s
                .rooms
                .iter()
                .filter(|(_, r)| {
                    r.value["active"] == true
                        && r.value["state"] == "connected"
                        && r.value["is_live"] == true
                })
                .filter_map(|(rid, r)| {
                    let route = r
                        .route
                        .clone()
                        .filter(|route| !route.retired.load(Ordering::Acquire))?;
                    Some((
                        rid.clone(),
                        r.generation,
                        route,
                        crate::net::RoomInfo {
                            anchor_id: r.value["anchor_id"].as_str()?.into(),
                            room_id: r.value["session_id"].as_str()?.into(),
                            ..Default::default()
                        },
                        r.cancel.clone(),
                    ))
                })
                .collect::<Vec<_>>();
            s.refresh = json!({"id":id,"active":!targets.is_empty(),"total":targets.len(),"checked":0,"updated":0,"failed":0,"skipped":0});
            (id, targets)
        };
        if !targets.is_empty() {
            let shared = self.shared.clone();
            self.spawn(async move {
                stream::iter(targets)
                    .for_each_concurrent(4, |(rid, generation, route, info, cancel)| {
                        let shared = shared.clone();
                        async move {
                            let result = crate::monitor::refresh_one(
                                shared.clone(),
                                rid,
                                generation,
                                route,
                                info,
                                cancel,
                            )
                            .await;
                            let mut s = shared.state.lock().unwrap();
                            s.refresh["checked"] =
                                json!(s.refresh["checked"].as_u64().unwrap_or(0) + 1);
                            let k = if result.is_ok() { "updated" } else { "failed" };
                            s.refresh[k] = json!(s.refresh[k].as_u64().unwrap_or(0) + 1);
                        }
                    })
                    .await;
                shared.state.lock().unwrap().refresh["active"] = json!(false);
            });
        }
        let mut v = self.shared.state.lock().unwrap().refresh.clone();
        v["accepted"] = json!(true);
        v["id"] = json!(id);
        Ok(v)
    }
}
