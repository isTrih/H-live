//! Bounded status-query priority queue and a local, disposable broadcast-time cache.
//! This never throttles a connected message socket or treats network errors as offline.
use crate::storage;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    sync::{oneshot, Notify, OwnedSemaphorePermit, Semaphore},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

const DAY: i64 = 86_400;
const SLOT: i64 = 1_800;
const RETENTION_DAYS: i64 = 28;
const MAX_ANCHORS: usize = 10_000;

#[derive(Default, Serialize, Deserialize)]
struct History {
    version: u8,
    aliases: BTreeMap<String, String>,
    anchors: BTreeMap<String, BTreeMap<u8, i64>>,
}
struct Habits {
    data: History,
    revision: u64,
    saved: u64,
    writable: bool,
}
struct Waiting {
    id: u64,
    rid: String,
    known_live: bool,
    offline: u32,
    since: Instant,
    reply: oneshot::Sender<QueryPermit>,
}
pub(crate) struct Polling {
    path: PathBuf,
    habits: Mutex<Habits>,
    queue: Mutex<Vec<Waiting>>,
    sequence: AtomicU64,
    slots: Arc<Semaphore>,
    changed: Arc<Notify>,
}

fn local_time(now: i64) -> (i64, u8) {
    let local = now + 8 * 3600;
    (local.div_euclid(DAY), (local.rem_euclid(DAY) / SLOT) as u8)
}
fn preferred(history: &History, rid: &str, now: i64) -> bool {
    let Some(slots) = history
        .aliases
        .get(rid)
        .and_then(|a| history.anchors.get(a))
    else {
        return false;
    };
    let (day, slot) = local_time(now);
    [slot, (slot + 47) % 48, (slot + 1) % 48]
        .iter()
        .any(|slot| {
            slots
                .get(slot)
                .is_some_and(|seen| *seen <= day && day - *seen < RETENTION_DAYS)
        })
}
fn offline_delay(count: u32) -> u64 {
    30u64
        .saturating_mul(1u64 << count.saturating_sub(1).min(5))
        .min(600)
}
fn delay(history: &History, rid: &str, count: u32, now: i64) -> u64 {
    if preferred(history, rid, now) {
        return 30;
    }
    let base = offline_delay(count);
    let boundary = SLOT - (now + 8 * 3600).rem_euclid(SLOT);
    if boundary < base as i64 && preferred(history, rid, now + boundary) {
        boundary.max(1) as u64
    } else {
        base
    }
}
fn rank(waiter: &Waiting, history: &History, now: i64) -> (u8, u64) {
    // Aged work is promoted ahead of new work; the sequence provides FIFO fairness.
    let priority = if waiter.since.elapsed() >= Duration::from_secs(60) {
        0
    } else if waiter.known_live {
        1
    } else if preferred(history, &waiter.rid, now) {
        2
    } else if waiter.offline == 0 {
        3
    } else {
        4 + waiter.offline.min(6) as u8
    };
    (priority, waiter.id)
}

pub(crate) struct QueryPermit {
    slot: Option<OwnedSemaphorePermit>,
    changed: Arc<Notify>,
}
impl Drop for QueryPermit {
    fn drop(&mut self) {
        self.slot.take();
        self.changed.notify_one();
    }
}
struct WakeOnDrop(Arc<Notify>);
impl Drop for WakeOnDrop {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}
impl Polling {
    pub fn open(directory: &std::path::Path) -> Self {
        let path = directory.join("polling-habits.json");
        // Unrecognized/corrupt caches never stop monitoring and are never overwritten.
        let loaded = if path.exists() {
            std::fs::metadata(&path)
                .ok()
                .filter(|m| m.len() <= 4 * 1024 * 1024)
                .and_then(|_| std::fs::read(&path).ok())
                .and_then(|b| serde_json::from_slice::<History>(&b).ok())
                .filter(|h| {
                    h.version == 1
                        && h.aliases.len() <= MAX_ANCHORS
                        && h.anchors.len() <= MAX_ANCHORS
                        && h.aliases
                            .iter()
                            .all(|(r, a)| storage::id(r) && storage::id(a))
                        && h.anchors.iter().all(|(a, s)| {
                            storage::id(a) && s.len() <= 48 && s.keys().all(|k| *k < 48)
                        })
                })
        } else {
            Some(History {
                version: 1,
                ..History::default()
            })
        };
        let writable = loaded.is_some();
        Self {
            path,
            habits: Mutex::new(Habits {
                data: loaded.unwrap_or_default(),
                revision: 0,
                saved: 0,
                writable,
            }),
            queue: Mutex::new(Vec::new()),
            sequence: AtomicU64::new(0),
            slots: Arc::new(Semaphore::new(8)),
            changed: Arc::new(Notify::new()),
        }
    }
    pub fn observe_live(&self, rid: &str, anchor: &str, now: i64) {
        if !storage::id(rid) || !storage::id(anchor) {
            return;
        }
        let mut h = self.habits.lock().unwrap();
        if (!h.data.aliases.contains_key(rid) && h.data.aliases.len() >= MAX_ANCHORS)
            || (!h.data.anchors.contains_key(anchor) && h.data.anchors.len() >= MAX_ANCHORS)
        {
            return;
        }
        let (day, slot) = local_time(now);
        let alias_changed = h.data.aliases.get(rid).is_none_or(|a| a != anchor);
        let slots = h.data.anchors.entry(anchor.to_owned()).or_default();
        let changed = slots.get(&slot) != Some(&day);
        if changed {
            slots.insert(slot, day);
            slots.retain(|_, d| *d <= day && day - *d < RETENTION_DAYS);
        }
        if alias_changed {
            h.data.aliases.insert(rid.to_owned(), anchor.to_owned());
        }
        if changed || alias_changed {
            h.revision += 1;
        }
    }
    pub fn next_delay(&self, rid: &str, count: u32, now: i64) -> u64 {
        delay(&self.habits.lock().unwrap().data, rid, count, now)
    }
    pub fn flush(&self) -> storage::Result<()> {
        let (bytes, revision) = {
            let h = self.habits.lock().unwrap();
            if !h.writable || h.saved == h.revision {
                return Ok(());
            }
            (
                serde_json::to_vec(&h.data).map_err(|_| "开播偏好序列化失败")?,
                h.revision,
            )
        };
        storage::atomic(&self.path, &bytes)?;
        self.habits.lock().unwrap().saved = revision;
        Ok(())
    }
    pub async fn acquire(
        &self,
        rid: &str,
        known_live: bool,
        offline: u32,
        cancel: &CancellationToken,
    ) -> Option<QueryPermit> {
        let id = self.sequence.fetch_add(1, Ordering::Relaxed);
        let (reply, received) = oneshot::channel();
        self.queue.lock().unwrap().push(Waiting {
            id,
            rid: rid.to_owned(),
            known_live,
            offline,
            since: Instant::now(),
            reply,
        });
        let _wake = WakeOnDrop(self.changed.clone());
        self.changed.notify_one();
        tokio::select! { biased; _ = cancel.cancelled() => None, result = received => result.ok() }
    }
    // One coordinator wakes the chosen task only; no all-room wakeups or repeated queue scans.
    fn dispatch(&self) {
        let mut queue = self.queue.lock().unwrap();
        queue.retain(|w| !w.reply.is_closed());
        let h = self.habits.lock().unwrap();
        let now = chrono::Utc::now().timestamp();
        while !queue.is_empty() {
            let Ok(slot) = self.slots.clone().try_acquire_owned() else {
                break;
            };
            let index = queue
                .iter()
                .enumerate()
                .min_by_key(|(_, w)| rank(w, &h.data, now))
                .unwrap()
                .0;
            let waiter = queue.swap_remove(index);
            let _ = waiter.reply.send(QueryPermit {
                slot: Some(slot),
                changed: self.changed.clone(),
            });
        }
    }
    pub async fn run(&self, cancel: &CancellationToken) {
        loop {
            tokio::select! { biased; _ = cancel.cancelled() => break,
            _ = self.changed.notified() => {}, _ = tokio::time::sleep(Duration::from_secs(1)) => {} }
            self.dispatch();
        }
        self.queue.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(hour: i64, minute: i64) -> i64 {
        20_000 * DAY + (hour - 8) * 3600 + minute * 60
    }
    #[test]
    fn offline_backoff_caps_at_ten_minutes() {
        assert_eq!(
            (1..=9).map(offline_delay).collect::<Vec<_>>(),
            [30, 60, 120, 240, 480, 600, 600, 600, 600]
        );
    }
    #[test]
    fn preference_covers_observed_window_and_midnight_then_expires() {
        let dir = tempfile::tempdir().unwrap();
        let p = Polling::open(dir.path());
        p.observe_live("123", "456", at(0, 5));
        assert_eq!(p.next_delay("123", 20, at(23, 45)), 30);
        assert_eq!(p.next_delay("123", 20, at(0, 45)), 30);
        assert_eq!(p.next_delay("123", 20, at(2, 0)), 600);
        assert_eq!(p.next_delay("123", 20, at(0, 5) + 28 * DAY), 600);
        assert_eq!(p.next_delay("999", 20, at(0, 5)), 600);
        p.flush().unwrap();
        let loaded = Polling::open(dir.path());
        assert_eq!(loaded.next_delay("123", 20, at(0, 5)), 30);
        loaded.observe_live("789", "456", at(0, 5));
        assert_eq!(loaded.next_delay("789", 20, at(0, 45)), 30);
    }
    #[test]
    fn wait_is_shortened_at_preferred_window_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let p = Polling::open(dir.path());
        p.observe_live("123", "456", at(20, 0));
        assert_eq!(p.next_delay("123", 9, at(19, 25)), 300);
        assert_eq!(p.next_delay("123", 9, at(19, 30)), 30);
    }
    #[test]
    fn unsupported_cache_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("polling-habits.json");
        let original = br#"{"version":99,"aliases":{},"anchors":{}}"#;
        std::fs::write(&path, original).unwrap();
        let p = Polling::open(dir.path());
        p.observe_live("123", "456", at(12, 0));
        p.flush().unwrap();
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
    #[tokio::test]
    async fn live_reconnect_precedes_offline_and_cancel_does_not_leak_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let p = Arc::new(Polling::open(dir.path()));
        let cancel = CancellationToken::new();
        let scheduler = p.clone();
        let stop = cancel.clone();
        let worker = tokio::spawn(async move { scheduler.run(&stop).await });
        let mut held = Vec::new();
        for _ in 0..8 {
            held.push(p.acquire("1", false, 0, &cancel).await.unwrap());
        }
        let q = p.clone();
        let c = cancel.child_token();
        let offline = tokio::spawn(async move { q.acquire("2", false, 9, &c).await });
        tokio::task::yield_now().await;
        let q = p.clone();
        let c = cancel.child_token();
        let live = tokio::spawn(async move { q.acquire("3", true, 0, &c).await });
        tokio::task::yield_now().await;
        held.pop();
        let live_permit = tokio::time::timeout(Duration::from_secs(2), live)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!offline.is_finished());
        offline.abort();
        let _ = offline.await;
        drop(live_permit);
        drop(held);
        cancel.cancel();
        worker.await.unwrap();
        assert_eq!(p.slots.available_permits(), 8);
        assert!(p.queue.lock().unwrap().is_empty());
    }
    #[test]
    fn learned_window_and_aging_have_priority_and_fifo_ties() {
        let mut h = History::default();
        h.aliases.insert("2".into(), "3".into());
        let (day, slot) = local_time(at(20, 0));
        h.anchors.insert("3".into(), BTreeMap::from([(slot, day)]));
        let mut cold = Waiting {
            id: 0,
            rid: "1".into(),
            known_live: false,
            offline: 9,
            since: Instant::now(),
            reply: oneshot::channel().0,
        };
        let hot = Waiting {
            id: 1,
            rid: "2".into(),
            known_live: false,
            offline: 9,
            since: Instant::now(),
            reply: oneshot::channel().0,
        };
        assert!(rank(&hot, &h, at(20, 0)) < rank(&cold, &h, at(20, 0)));
        cold.since -= Duration::from_secs(61);
        assert!(rank(&cold, &h, at(20, 0)) < rank(&hot, &h, at(20, 0)));
    }
}
