//! Offline adapter, compiled only with test-support. No external network access.
use crate::{
    net::{Error, RoomInfo, Visitor},
    proxy::Route,
    transport::{MessageSocket, Transport},
};
use futures_util::{future::BoxFuture, FutureExt};
use std::{
    io::Write,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub struct Counters {
    pub active: AtomicUsize,
    pub peak: AtomicUsize,
    pub connections: AtomicUsize,
    pub frames: AtomicUsize,
    pub force_disconnect_through: AtomicUsize,
}
pub struct Simulation {
    pub live_rooms: u64,
    pub interval_ms: u64,
    pub reconnect_after: u64,
    pub counters: Arc<Counters>,
}
impl Default for Simulation {
    fn default() -> Self {
        Self {
            live_rooms: 50,
            interval_ms: 200,
            reconnect_after: 0,
            counters: Arc::new(Counters::default()),
        }
    }
}
impl Transport for Simulation {
    fn visitor<'a>(
        &'a self,
        r: &'a Route,
        c: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Visitor, Error>> {
        async move {
            let _p = r.admit(false, c).await.map_err(|_| Error::Cancelled)?;
            Ok(Visitor {
                cookie: "synthetic-visitor".into(),
                did: "777".into(),
            })
        }
        .boxed()
    }
    fn room<'a>(
        &'a self,
        r: &'a Route,
        _: &'a Visitor,
        id: &'a str,
        c: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<RoomInfo, Error>> {
        async move {
            let _p = r.admit(false, c).await.map_err(|_| Error::Cancelled)?;
            let number = id
                .parse::<u64>()
                .map_err(|_| Error::Invalid("合成 ID 无效"))?;
            Ok(RoomInfo {
                nickname: format!("合成主播 {id}"),
                is_live: number <= self.live_rooms,
                title: "合成场次".into(),
                anchor_id: (100000 + number).to_string(),
                room_id: (200000 + number).to_string(),
                viewer_count: Some(100 + number),
                started_at: None,
            })
        }
        .boxed()
    }
    fn connect<'a>(
        &'a self,
        r: Arc<Route>,
        _: &'a Visitor,
        id: &'a str,
        c: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Box<dyn MessageSocket>, Error>> {
        async move {
            let _p = r.admit(true, c).await.map_err(|_| Error::Cancelled)?;
            let active = self.counters.active.fetch_add(1, Ordering::AcqRel) + 1;
            self.counters.peak.fetch_max(active, Ordering::AcqRel);
            let number = self.counters.connections.fetch_add(1, Ordering::AcqRel) + 1;
            Ok(Box::new(SyntheticSocket {
                room: id.parse().unwrap(),
                count: 0,
                connection: number as u64,
                reconnect: self.reconnect_after,
                interval: tokio::time::interval(Duration::from_millis(self.interval_ms)),
                counters: self.counters.clone(),
            }) as Box<dyn MessageSocket>)
        }
        .boxed()
    }
}
struct SyntheticSocket {
    room: u64,
    count: u64,
    connection: u64,
    reconnect: u64,
    interval: tokio::time::Interval,
    counters: Arc<Counters>,
}
impl Drop for SyntheticSocket {
    fn drop(&mut self) {
        self.counters.active.fetch_sub(1, Ordering::AcqRel);
    }
}
impl MessageSocket for SyntheticSocket {
    fn read_frame<'a>(
        &'a mut self,
        c: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<Vec<u8>>, Error>> {
        async move{tokio::select!{biased;_=c.cancelled()=>return Err(Error::Cancelled),_=self.interval.tick()=>{}}if self.counters.force_disconnect_through.load(Ordering::Acquire)>=self.connection as usize{return Err(Error::Closed)}self.count+=1;if self.reconnect>0&&self.count>self.reconnect{return Err(Error::Closed)}self.counters.frames.fetch_add(1,Ordering::AcqRel);let id=self.connection*1_000_000+self.count;let common=[number(2,id),number(3,self.room),number(4,1800000000+self.count)].concat();let user=[number(1,9000+self.count%20),text(3,"合成观众")].concat();
        let chat=[blob(1,&common),blob(2,&user),text(3,&format!("正常互动问题 {}",self.count))].concat();
        let gift=[blob(1,&[number(2,id+500_000),number(3,self.room)].concat()),number(6,1),blob(7,&user),blob(15,&text(16,"合成礼物"))].concat();
        Ok(Some(frame(&[("WebcastChatMessage",chat),("WebcastGiftMessage",gift)])))
    }.boxed()
    }
    fn send_frame<'a>(
        &'a mut self,
        _: Vec<u8>,
        c: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<(), Error>> {
        async move {
            if c.is_cancelled() {
                Err(Error::Cancelled)
            } else {
                Ok(())
            }
        }
        .boxed()
    }
}
fn varint(mut n: u64) -> Vec<u8> {
    let mut out = vec![];
    while n > 127 {
        out.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    out.push(n as u8);
    out
}
fn number(tag: u64, n: u64) -> Vec<u8> {
    [varint(tag << 3), varint(n)].concat()
}
fn blob(tag: u64, b: &[u8]) -> Vec<u8> {
    [varint(tag << 3 | 2), varint(b.len() as u64), b.to_vec()].concat()
}
fn text(tag: u64, s: &str) -> Vec<u8> {
    blob(tag, s.as_bytes())
}
fn frame(messages: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut response = messages
        .iter()
        .flat_map(|(method, payload)| blob(1, &[text(1, method), blob(2, payload)].concat()))
        .collect::<Vec<_>>();
    response.extend(text(5, "synthetic-ack"));
    response.extend(number(9, 1));
    let mut zip = flate2::write::GzEncoder::new(vec![], flate2::Compression::default());
    zip.write_all(&response).unwrap();
    blob(8, &zip.finish().unwrap())
}

/// Only compiled with test-support; all exits are consumed by the fake adapter.
pub fn add_fake_exits(runtime: &crate::Runtime, count: u16) -> Result<(), String> {
    let mut state = runtime.shared.state.lock().unwrap();
    state.pool.enabled = true;
    state.pool.source = "imported".into();
    for number in 1..=count {
        let record = crate::proxy::ProxyRecord::from_line(
            &format!("8.8.{}.{}:8080", number / 250, (number % 250) + 1),
            "http",
        )?;
        let route = Route::new(Some(record))?;
        state.pool.routes.insert(route.id.clone(), route);
    }
    Ok(())
}
