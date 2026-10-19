//! One cancelable async task per room; no per-room OS/heartbeat threads.
use crate::{
    net::{self, RoomInfo},
    proxy::{Lease, Route},
    runtime::Shared,
    storage,
};

use qingkan_core::{
    archive::MessageRecord,
    export::CancelToken,
    protocol::{decode_frame, hb_frame},
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub(crate) async fn pause(seconds: f64, cancel: &CancellationToken) -> bool {
    tokio::select! {biased;_=cancel.cancelled()=>false,_=tokio::time::sleep(Duration::from_secs_f64(seconds.max(0.)))=>true}
}
async fn database<T: Send + 'static>(
    shared: &Shared,
    f: impl FnOnce(qingkan_core::archive::ArchiveStore) -> storage::Result<T> + Send + 'static,
) -> storage::Result<T> {
    let permit = shared
        .storage_slots
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| "归档已停止")?;
    let archive = shared.archive.clone();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        f(archive)
    })
    .await
    .map_err(|_| "归档工作任务异常")?
}
async fn checkpoint(
    shared: &Shared,
    rid: &str,
    generation: u64,
    seconds: f64,
) -> storage::Result<()> {
    let row = shared.row(rid, generation).ok_or("监控已结束")?;
    let a = row["anchor_id"].as_str().unwrap_or("").to_owned();
    let session = row["session_id"].as_str().unwrap_or("").to_owned();
    if !storage::id(&a) || !storage::id(&session) {
        return Ok(());
    }
    database(shared, move |archive| {
        archive
            .save_monitor_duration(&a, &session, seconds)
            .map_err(|_| "累计监控时长保存失败".into())
    })
    .await
}
async fn viewers(
    shared: &Shared,
    rid: &str,
    generation: u64,
    info: &RoomInfo,
) -> storage::Result<()> {
    let Some(count) = info.viewer_count else {
        return Ok(());
    };
    shared.patch(
        rid,
        generation,
        json!({"viewer_count":count,"viewer_updated_at":storage::now()}),
    );
    let p = json!({"anchor_id":info.anchor_id,"session_id":info.room_id,"viewer_count":count,"observed_at":storage::now()});
    database(shared, move |archive| {
        archive
            .quality_request("audience_record", p, CancelToken::default())
            .map(|_| ())
            .map_err(|_| "在线人数保存失败".into())
    })
    .await
}
async fn activities(
    shared: &Shared,
    rid: &str,
    generation: u64,
    info: &RoomInfo,
) -> storage::Result<()> {
    let (anchor, session) = (info.anchor_id.clone(), info.room_id.clone());
    let summary = database(shared, move |archive| {
        archive
            .activity_summary(&anchor, &session, chrono::Utc::now().timestamp())
            .map_err(|_| "活动统计读取失败".into())
    })
    .await?;
    shared.patch(rid, generation, json!({"activities":summary}));
    Ok(())
}
pub(crate) async fn refresh_one(
    shared: Arc<Shared>,
    rid: String,
    generation: u64,
    route: Arc<Route>,
    info: RoomInfo,
    cancel: CancellationToken,
) -> storage::Result<()> {
    let visitor = shared
        .visitor(&route, &cancel)
        .await
        .map_err(|e| e.to_string())?;
    let fresh = shared
        .network
        .room(&route, &visitor, &rid, &cancel)
        .await
        .map_err(|e| e.to_string())?;
    if !fresh.is_live || fresh.room_id != info.room_id || fresh.anchor_id != info.anchor_id {
        return Err("场次已变化或状态待确认，保留原在线数据".into());
    }
    if !shared.row(&rid, generation).is_some_and(|r| {
        r["active"] == true && r["session_id"] == info.room_id && r["anchor_id"] == info.anchor_id
    }) {
        return Err("任务或场次已变化".into());
    }
    viewers(&shared, &rid, generation, &fresh).await
}
pub(crate) async fn run(
    shared: Arc<Shared>,
    rid: String,
    generation: u64,
    cancel: CancellationToken,
) {
    let task_started = Instant::now();
    let mut observed_offline = false;
    let mut lease: Option<Lease> = None;
    let mut visitor = None;
    let mut bound_session = String::new();
    let mut accumulated = 0.;
    let mut failures = 0u32;
    let mut offline_checks = 0u32;
    let mut known_live = false;
    'task: while !cancel.is_cancelled() {
        if lease.as_ref().is_some_and(|l| !l.0.ready()) {
            lease = None;
            visitor = None;
        }
        if lease.is_none() {
            lease = shared.state.lock().unwrap().pool.lease();
            if lease.is_none() {
                shared.status(
                    &rid,
                    generation,
                    "waiting_proxy",
                    "等待可用出口或出口冷却结束",
                );
                if !pause(1., &cancel).await {
                    break;
                }
                continue;
            }
        }
        let route = lease.as_ref().unwrap().0.clone();
        shared.bind_route(&rid, generation, route.clone());
        shared.patch(&rid,generation,json!({"route_id":route.id,"route_kind":if route.proxy.is_some(){"proxy"}else{"direct"},"proxy":route.proxy.as_ref().map(|p|p.endpoint.clone()).unwrap_or_default(),"proxy_status":if route.proxy.is_some(){"assigned"}else{"direct"}}));
        shared.status(
            &rid,
            generation,
            "waiting_connection",
            "等待出口建连名额或出口冷却结束",
        );
        let result:std::result::Result<(),net::Error>=async{
            let query = shared.polling.acquire(&rid,known_live,offline_checks,&cancel).await.ok_or(net::Error::Cancelled)?;
            if visitor.is_none(){visitor=Some(shared.visitor(&route,&cancel).await?)}
            let guest=visitor.as_ref().unwrap();
            let info=shared.network.room(&route,guest,&rid,&cancel).await?;
            drop(query); // The permit bounds status queries, never the long-lived WSS stream.
            if !shared.patch_info(&rid,generation,&info){return Err(net::Error::Cancelled)}
            known_live=info.is_live;
            if !info.is_live {
                observed_offline=true;failures=0;offline_checks=offline_checks.saturating_add(1);
                let now=chrono::Utc::now().timestamp();
                let seconds=shared.polling.next_delay(&rid,offline_checks,now);
                shared.patch(&rid,generation,json!({"offline_checks":offline_checks,"poll_interval_seconds":seconds,"next_check_at":now+seconds as i64}));
                shared.status(&rid,generation,"offline",&format!("尚未开播，{seconds} 秒后再检查"));
                pause(seconds as f64,&cancel).await;return Ok(())
            }
            if !storage::id(&info.anchor_id)||!storage::id(&info.room_id){return Err(net::Error::Invalid("无法确认真实主播或场次，暂停连接并稍后核验"))}
            offline_checks=0;
            shared.polling.observe_live(&rid,&info.anchor_id,chrono::Utc::now().timestamp());
            shared.patch(&rid,generation,json!({"offline_checks":0,"poll_interval_seconds":null,"next_check_at":null}));
            let row=shared.row(&rid,generation).ok_or(net::Error::Cancelled)?;
            let params=json!({"anchor_id":info.anchor_id,"session_id":info.room_id,"anchor_name":row["name"],"web_rid":rid,"title":info.title,"started_at":info.started_at.and_then(|t|chrono::DateTime::from_timestamp(t as i64,0)).map(|t|t.to_rfc3339())});
            database(&shared,move|a|a.quality_request("register",params,CancelToken::default()).map(|_|()).map_err(|_|"场次登记失败".into())).await.map_err(|_|net::Error::Invalid("场次登记失败，已停止任务"))?;
            if bound_session!=info.room_id {
                let (a,s)=(info.anchor_id.clone(),info.room_id.clone());
                accumulated=database(&shared,move|archive|archive.load_monitor_duration(&a,&s).map_err(|_|"累计监控时长读取失败".into())).await.map_err(|_|net::Error::Invalid("累计监控时长读取失败，已停止任务"))?;
                if bound_session.is_empty()&&!observed_offline&&accumulated==0.{accumulated=task_started.elapsed().as_secs_f64();}
                bound_session=info.room_id.clone();
            }
            viewers(&shared,&rid,generation,&info).await.map_err(|_|net::Error::Invalid("在线统计归档失败"))?;
            activities(&shared,&rid,generation,&info).await.map_err(|_|net::Error::Invalid("活动统计读取失败"))?;
            shared.status(&rid,generation,"connecting","正在建立消息连接");
            let mut socket=shared.network.connect(route.clone(),guest,&info.room_id,&cancel).await?;
            let began=Instant::now();shared.patch(&rid,generation,json!({"duration_seconds":accumulated,"duration_running":true,"duration_updated_at":crate::proxy::epoch_ms() as f64/1000.,"duration_source":"monitored"}));
            shared.status(&rid,generation,"connected","连接正常，正在记录消息");failures=0;
            let mut heartbeat=tokio::time::interval(Duration::from_secs(20));heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut checkpoints=tokio::time::interval(Duration::from_secs(5));checkpoints.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut last_read=Instant::now();
            let mut activity_dirty=false;
            let mut activity_refreshed=Instant::now();
            let connection='connection:loop{
                tokio::select!{biased;
                    _=cancel.cancelled()=>break Err(net::Error::Cancelled),
                    _=heartbeat.tick()=>{
                        if last_read.elapsed()>Duration::from_secs(90){break Err(net::Error::Invalid("消息连接长时间无响应"))}
                        match tokio::time::timeout(Duration::from_secs(5),socket.send_frame(hb_frame(),&cancel)).await{Ok(Ok(()))=>{},_=>break Err(net::Error::Network)}
                    },
                    _=checkpoints.tick()=>{
                        if activity_dirty {
                            if activities(&shared,&rid,generation,&info).await.is_err(){break Err(net::Error::Invalid("活动统计读取失败"))}
                            activity_dirty=false;activity_refreshed=Instant::now();
                        }
                        shared.polling.observe_live(&rid,&info.anchor_id,chrono::Utc::now().timestamp());
                        let seconds=accumulated+began.elapsed().as_secs_f64();
                        if checkpoint(&shared,&rid,generation,seconds).await.is_err(){break Err(net::Error::Invalid("监控时长写入失败，已停止任务"))}
                        shared.patch(&rid,generation,json!({"duration_seconds":seconds,"duration_updated_at":crate::proxy::epoch_ms() as f64/1000.}));
                    },
                    received=socket.read_frame(&cancel)=>{
                        let bytes=match received{Ok(Some(bytes))=>bytes,Ok(None)=>{last_read=Instant::now();continue},Err(e)=>break Err(e)};last_read=Instant::now();
                        let decoded=match decode_frame(&bytes,&info.room_id){Ok(frame)=>frame,Err(_)=>break Err(net::Error::Invalid("消息包格式或大小无效，连接已重试"))};
                        if let Some(ack)=decoded.ack{match tokio::time::timeout(Duration::from_secs(5),socket.send_frame(ack,&cancel)).await{Ok(Ok(()))=>{},_=>break Err(net::Error::Network)}}
                        for event in decoded.events {
                            if cancel.is_cancelled(){break}
                            if event.kind=="gift"{continue}
                            let payload=event.payload;
                            if payload["room_id"].as_str().is_some_and(|s|!s.is_empty()&&s!=info.room_id){shared.status(&rid,generation,"error","收到其他场次的消息，已停止此任务");cancel.cancel();break}
                            if event.kind=="stats" {if let Some(n)=payload["viewer_count"].as_u64().filter(|n|*n<=9_007_199_254_740_991){let mut i=info.clone();i.viewer_count=Some(n);if viewers(&shared,&rid,generation,&i).await.is_err(){cancel.cancel();break}}continue}
                            if event.kind=="control"{shared.log(&rid,"平台确认本场直播结束");shared.patch(&rid,generation,json!({"is_live":false,"live_checked_at":storage::now()}));observed_offline=true;break 'connection Ok(())}
                            if !["chat","like","member","social","fansclub","emoji","activity"].contains(&event.kind){continue}
                            if event.kind=="activity" && payload["activity_stage"]=="status" && payload["activity_id"].as_str().unwrap_or("").is_empty(){continue}
                            let Some(row)=shared.row(&rid,generation)else{break};let received=storage::now();
                            let time=payload["event_time"].as_u64().filter(|t|*t>0).and_then(|t|chrono::DateTime::from_timestamp(if t>10_000_000_000{(t/1000) as i64}else{t as i64},0)).map(|t|t.with_timezone(&qingkan_core::archive::shanghai()).to_rfc3339_opts(chrono::SecondsFormat::Millis,false)).unwrap_or_default();
                            let text=|k:&str|payload[k].as_str().unwrap_or("").to_owned();
                            let record=MessageRecord{anchor_id:info.anchor_id.clone(),anchor_name:row["name"].as_str().unwrap_or("").into(),web_rid:rid.clone(),session_id:info.room_id.clone(),sender_id:text("sender_id"),sender_name:text("sender_name"),message_type:event.kind.into(),content:text("content"),message_time:time.clone(),received_at:received.clone(),time_source:if time.is_empty(){"missing"}else{"server"}.into(),message_id:text("message_id"),payload:payload.clone()};
                            let title=info.title.clone();let inserted=database(&shared,move|a|a.record_message(record,title,None).map_err(|_|"消息写入失败".into())).await;
                            match inserted{Ok(true)=>{shared.message(&rid,generation,event.kind,&payload,&time,&received);activity_dirty|=event.kind=="activity"},Ok(false)=>activity_dirty|=event.kind=="activity",Err(_)=>{shared.status(&rid,generation,"error","消息写入失败，已暂停此任务");cancel.cancel();break}}
                        }
                        if activity_dirty && activity_refreshed.elapsed()>=Duration::from_secs(1){
                            if activities(&shared,&rid,generation,&info).await.is_err(){break Err(net::Error::Invalid("活动统计读取失败"))}
                            activity_dirty=false;activity_refreshed=Instant::now();
                        }
                    }
                }
            };
            accumulated+=began.elapsed().as_secs_f64();
            if activity_dirty { let _=activities(&shared,&rid,generation,&info).await; }
            let saved=checkpoint(&shared,&rid,generation,accumulated).await;
            shared.patch(&rid,generation,json!({"duration_seconds":accumulated,"duration_running":false,"duration_updated_at":crate::proxy::epoch_ms() as f64/1000.}));
            drop(socket);if saved.is_err(){shared.status(&rid,generation,"error","累计时长保存失败，请检查磁盘");cancel.cancel();}connection
        }.await;
        if cancel.is_cancelled() {
            break;
        }
        match result {
            Ok(()) => {}
            Err(net::Error::Cancelled) => break,
            Err(error) => {
                failures = failures.saturating_add(1);
                let delay = (5u64.saturating_mul(2u64.saturating_pow(failures.min(5)))).min(120);
                match error {
                    net::Error::Status(429 | 444) => {
                        route.cooldown(30u64.saturating_mul(failures as u64).min(300));
                        shared.invalidate_visitor(&route, visitor.as_ref()).await;
                        visitor = None
                    }
                    net::Error::Network => {
                        if route.proxy.is_some() {
                            route.cooldown(20);
                            lease = None;
                            visitor = None
                        }
                    }
                    net::Error::Unavailable => {
                        shared.invalidate_visitor(&route, visitor.as_ref()).await;
                        visitor = None;
                    }
                    _ => {}
                }
                let live = shared
                    .row(&rid, generation)
                    .is_some_and(|r| r["is_live"] == true);
                shared.status(
                    &rid,
                    generation,
                    if matches!(error, net::Error::Unavailable) && live {
                        "suspected_offline"
                    } else {
                        "retrying"
                    },
                    &format!("{error}，{delay} 秒后重试"),
                );
                if !pause(delay as f64, &cancel).await {
                    break 'task;
                }
            }
        }
    }
    drop(lease);
    shared.patch(
        &rid,
        generation,
        json!({"active":false,"duration_running":false}),
    );
    if shared
        .row(&rid, generation)
        .is_some_and(|r| r["state"] != "error")
    {
        shared.status(&rid, generation, "stopped", "监控已停止");
    }
}
