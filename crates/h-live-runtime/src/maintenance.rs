//! Proxy health/retirement and encrypted HTTPS subscriptions share the same state.
use crate::{
    net,
    proxy::{self, ProxyRecord, Route},
    runtime::{records, Shared},
    storage::{self, Result},
};
use futures_util::{stream, StreamExt};
use serde_json::json;
use std::{
    collections::HashSet,
    sync::{atomic::Ordering, Arc},
};

fn spawn(shared: &Arc<Shared>, f: impl std::future::Future<Output = ()> + Send + 'static) {
    let mut jobs = shared.jobs.lock().unwrap();
    jobs.retain(|j| !j.is_finished());
    jobs.push(tokio::spawn(f));
}
pub(crate) async fn tick(shared: &Arc<Shared>, tick: u64) {
    {
        let mut s = shared.state.lock().unwrap();
        s.pool
            .retired
            .retain(|r| r.load.load(Ordering::Acquire) > 0);
        let stale = s
            .pool
            .routes
            .iter()
            .filter(|(id, r)| {
                r.proxy.is_some()
                    && (r.proxy.as_ref().is_some_and(ProxyRecord::expired)
                        || s.pool
                            .verified
                            .get(*id)
                            .is_some_and(|t| proxy::epoch_ms().saturating_sub(*t) > 300_000))
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in stale {
            if let Some(route) = s.pool.routes.remove(&id) {
                route.retired.store(true, Ordering::Release);
                s.pool.retired.push(route);
            }
        }
    }
    let subscriptions = {
        let s = shared.state.lock().unwrap();
        s.config.value["proxy_subscriptions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v["enabled"] != false)
            .filter(|v| {
                let elapsed = v["updated_at"]
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds())
                    .unwrap_or(i64::MAX);
                elapsed >= v["interval_minutes"].as_i64().unwrap_or(60) * 60
            })
            .filter_map(|v| v["id"].as_str().map(str::to_owned))
            .collect::<Vec<_>>()
    };
    for id in subscriptions {
        refresh_subscription(shared, &id);
    }
    let enabled = shared.state.lock().unwrap().pool.enabled;
    if enabled && tick % 300 == 1 {
        probe(shared);
    }
}
pub(crate) fn refresh_subscription(shared: &Arc<Shared>, id: &str) -> bool {
    let nonce = format!("{:032x}", rand::random::<u128>());
    let (url, cancel) = {
        let mut s = shared.state.lock().unwrap();
        if s.subscriptions
            .get(id)
            .is_some_and(|v| v["state"] == "refreshing")
        {
            return false;
        }
        let Some(sub) = s.config.value["proxy_subscriptions"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|v| v["id"] == id)
        else {
            return false;
        };
        let Some(url) = sub["url"].as_str() else {
            return false;
        };
        let url = url.to_owned();
        s.subscriptions.insert(
            id.into(),
            json!({"state":"refreshing","error":"","nonce":nonce}),
        );
        let cancel = shared.cancel.child_token();
        if let Some(old) = s.subscription_tokens.insert(id.into(), cancel.clone()) {
            old.cancel();
        }
        (url, cancel)
    };
    let shared_clone = shared.clone();
    let id = id.to_owned();
    spawn(shared, async move {
        let result = async {
            let text = proxy::fetch_subscription(&url, &cancel).await?;
            let (records, _) = proxy::parse(&text, "http", &[])?;
            if records.is_empty() {
                return Err("订阅没有有效代理".into());
            }
            Ok::<_, String>(records)
        }
        .await;
        let s2 = shared_clone.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let mut s = s2.state.lock().unwrap();
            if s2.cancel.is_cancelled()
                || s.subscriptions.get(&id).is_none_or(|v| v["nonce"] != nonce)
            {
                return;
            }
            let mut config = s.config.value.clone();
            let Some(sub) = config["proxy_subscriptions"]
                .as_array_mut()
                .and_then(|a| a.iter_mut().find(|v| v["id"] == id))
            else {
                return;
            };
            match result {
                Ok(records) => {
                    sub["records"] = json!(records);
                    sub["updated_at"] = json!(storage::now());
                    match s.config.replace(config) {
                        Ok(()) => {
                            s.subscriptions
                                .insert(id.clone(), json!({"state":"idle","error":""}));
                        }
                        Err(_) => {
                            s.subscriptions.insert(
                                id.clone(),
                                json!({"state":"error","error":"订阅保存失败，旧候选已保留"}),
                            );
                        }
                    }
                }
                Err(_) => {
                    s.subscriptions.insert(
                        id.clone(),
                        json!({"state":"error","error":"订阅刷新失败，旧候选已保留"}),
                    );
                }
            }
            drop(s);
            reconcile(&s2);
        })
        .await;
    });
    true
}
pub(crate) fn reconcile(shared: &Shared) {
    let mut s = shared.state.lock().unwrap();
    let Ok(records) = records(&s.config.value) else {
        return;
    };
    if s.pool.source != "imported" {
        return;
    }
    let current = records
        .into_iter()
        .map(|r| (r.id.clone(), r))
        .collect::<std::collections::BTreeMap<_, _>>();
    let changed = s
        .pool
        .candidates
        .iter()
        .filter(|(id, old)| current.get(*id) != Some(*old))
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for id in changed {
        s.pool.failed.remove(&id);
        s.pool.checked.remove(&id);
        s.pool.verified.remove(&id);
    }
    let retired = s
        .pool
        .routes
        .iter()
        .filter(|(_, r)| {
            r.proxy
                .as_ref()
                .is_some_and(|p| current.get(&p.id) != Some(p))
        })
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for id in retired {
        if let Some(route) = s.pool.routes.remove(&id) {
            route.retired.store(true, Ordering::Release);
            s.pool.retired.push(route);
        }
        s.pool.failed.remove(&id);
        s.pool.checked.remove(&id);
    }
    s.pool.candidates = current;
}
pub(crate) fn probe(shared: &Arc<Shared>) -> bool {
    if shared
        .proxy_busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    {
        let mut s = shared.state.lock().unwrap();
        s.pool.refresh = json!({"active":true,"state":"running","target":50,"total":50,"checked":0,"passed":0,"failed":0});
        s.pool.last_error.clear();
    }
    let clone = shared.clone();
    spawn(shared, async move {
        let source = clone.state.lock().unwrap().pool.source.clone();
        let result:Result<Vec<ProxyRecord>>=async{
            if source=="imported"{return clone.records()}
            let http=proxy::client(None)?;let mut found=vec![];let mut seen=HashSet::new();
            for count in [20,20,10]{
                let response=tokio::select!{biased;_=clone.cancel.cancelled()=>return Err("任务已取消".into()),r=http.get(format!("https://proxy.scdn.io/api/get_proxy.php?protocol=https&count={count}")).send()=>r.map_err(|_|"代理来源获取失败")?};
                let raw=net::body(response,512*1024,&clone.cancel).await.map_err(|e|e.to_string())?;
                let text=String::from_utf8(raw).map_err(|_|"代理来源格式无效")?;
                let(records,_)=proxy::parse(&text,"http",&[])?;for record in records{if seen.insert(record.id.clone()){found.push(record)}}
            }Ok(found)
        }.await;
        match result {
            Err(_) => {
                clone.state.lock().unwrap().pool.last_error = "代理来源获取失败，请稍后重试".into();
            }
            Ok(records) => {
                let candidates = {
                    let mut s = clone.state.lock().unwrap();
                    if s.pool.source == source && source != "imported" {
                        for r in records {
                            s.pool.candidates.insert(r.id.clone(), r);
                        }
                    }
                    let candidates = s
                        .pool
                        .candidates
                        .values()
                        .filter(|r| {
                            !r.expired()
                                && !s.pool.failed.contains(&r.id)
                                && !s.pool.routes.contains_key(&r.id)
                        })
                        .take(50)
                        .cloned()
                        .collect::<Vec<_>>();
                    s.pool.checking = candidates.len();
                    s.pool.refresh["total"] = json!(candidates.len());
                    s.pool.refresh["target"] = json!(candidates.len());
                    candidates
                };
                let futures = stream::iter(candidates).map(|record| {
                    let shared = clone.clone();
                    let source = source.clone();
                    async move {
                        let retained = shared.state.lock().unwrap().pool.retired.iter()
                            .find(|r| r.proxy.as_ref() == Some(&record)).cloned();
                        let route = retained.map(Ok).unwrap_or_else(|| Route::new(Some(record.clone())));
                        let passed = match &route {
                            Ok(route) => {
                                let response = tokio::select! {
                                    biased;
                                    _ = shared.cancel.cancelled() => None,
                                    r = route.client.get("https://live.douyin.com/").send() => r.ok()
                                };
                                response.is_some_and(|r| r.status() == 200)
                            }
                            Err(_) => false,
                        };
                        apply_probe_result(&shared, &source, &record, passed.then(|| route.unwrap()));
                    }
                }).buffer_unordered(10);
                futures.collect::<Vec<_>>().await;
            }
        }
        {
            let mut s = clone.state.lock().unwrap();
            s.pool.refresh["active"] = json!(false);
            s.pool.refresh["state"] = json!("completed");
            s.pool.checking = 0;
            if s.pool.source == "public" {
                let failed = s.pool.failed.clone();
                s.pool.candidates.retain(|id, _| !failed.contains(id));
            }
        }
        clone.proxy_busy.store(false, Ordering::Release);
    });
    true
}

// A probe may finish after an import, credential change or source switch.
// Neither its success nor its failure may mutate the replacement candidate.
fn apply_probe_result(
    shared: &Shared,
    source: &str,
    record: &ProxyRecord,
    route: Option<std::sync::Arc<Route>>,
) {
    let mut s = shared.state.lock().unwrap();
    s.pool.checking = s.pool.checking.saturating_sub(1);
    if shared.cancel.is_cancelled()
        || s.pool.source != source
        || s.pool.candidates.get(&record.id) != Some(record)
    {
        return;
    }
    s.pool.checked.insert(record.id.clone());
    s.pool.refresh["checked"] = json!(s.pool.refresh["checked"].as_u64().unwrap_or(0) + 1);
    let key = if let Some(route) = route {
        // Health TTL expiry must not create a second admission pool for an
        // unchanged endpoint while its existing WSS leases are still alive.
        route.retired.store(false, Ordering::Release);
        s.pool
            .retired
            .retain(|r| !std::sync::Arc::ptr_eq(r, &route));
        s.pool.verified.insert(record.id.clone(), proxy::epoch_ms());
        s.pool.routes.insert(record.id.clone(), route);
        "passed"
    } else {
        s.pool.failed.insert(record.id.clone());
        "failed"
    };
    s.pool.refresh[key] = json!(s.pool.refresh[key].as_u64().unwrap_or(0) + 1);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_probe_cannot_poison_changed_credentials_or_resurrect_removed_proxy() {
        let d = tempfile::tempdir().unwrap();
        let runtime = crate::Runtime::open(d.path().into()).unwrap();
        let old = ProxyRecord::from_line("http://old:secret@8.8.8.8:8080", "http").unwrap();
        let new = ProxyRecord::from_line("http://new:changed@8.8.8.8:8080", "http").unwrap();
        {
            let mut s = runtime.shared.state.lock().unwrap();
            s.pool.source = "imported".into();
            s.pool.candidates.insert(new.id.clone(), new.clone());
        }
        apply_probe_result(&runtime.shared, "imported", &old, None);
        assert!(!runtime
            .shared
            .state
            .lock()
            .unwrap()
            .pool
            .failed
            .contains(&new.id));
        runtime.shared.state.lock().unwrap().pool.candidates.clear();
        apply_probe_result(
            &runtime.shared,
            "imported",
            &old,
            Some(Route::new(Some(old.clone())).unwrap()),
        );
        assert!(!runtime
            .shared
            .state
            .lock()
            .unwrap()
            .pool
            .routes
            .contains_key(&old.id));
        runtime.shutdown().unwrap();
    }

    #[test]
    fn successful_recheck_reuses_live_lease_and_admission_pool() {
        let d = tempfile::tempdir().unwrap();
        let runtime = crate::Runtime::open(d.path().into()).unwrap();
        let record = ProxyRecord::from_line("8.8.8.8:8080", "http").unwrap();
        let route = Route::new(Some(record.clone())).unwrap();
        route.retired.store(true, Ordering::Release);
        route.load.store(2, Ordering::Release);
        {
            let mut s = runtime.shared.state.lock().unwrap();
            s.pool.source = "imported".into();
            s.pool.candidates.insert(record.id.clone(), record.clone());
            s.pool.retired.push(route.clone());
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert!(rt
            .block_on(route.admit(false, &tokio_util::sync::CancellationToken::new()))
            .is_err());
        apply_probe_result(&runtime.shared, "imported", &record, Some(route.clone()));
        {
            let s = runtime.shared.state.lock().unwrap();
            assert!(std::sync::Arc::ptr_eq(&s.pool.routes[&record.id], &route));
            assert!(s.pool.retired.is_empty());
            assert_eq!(route.load.load(Ordering::Acquire), 2);
            assert!(route.ready());
        }
        runtime.shutdown().unwrap();
    }
}
