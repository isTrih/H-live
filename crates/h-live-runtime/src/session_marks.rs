//! Live bookmarks keep wall-clock time so a later recording can map them to media time.
use crate::{
    runtime::Runtime,
    storage::{self, Result},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Mutex};

const MAX_MARKS: usize = 10_000;
const MAX_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct SessionMark {
    pub id: String,
    pub anchor_id: String,
    pub session_id: String,
    pub room_id: String,
    pub anchor_name: String,
    pub title: String,
    pub note: String,
    pub marked_at_epoch: f64,
    pub marked_at: String,
    pub created_at: String,
    pub updated_at: String,
    pub broadcast_started_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_key: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct MarkFile {
    format: u32,
    marks: Vec<SessionMark>,
}

pub(crate) struct MarkStore {
    path: PathBuf,
    gate: Mutex<()>,
}

impl MarkStore {
    pub(crate) fn new(directory: PathBuf) -> Self {
        Self {
            path: directory.join("session-marks.json"),
            gate: Mutex::new(()),
        }
    }

    fn load(&self) -> Result<Vec<SessionMark>> {
        let metadata = match std::fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(_) => return Err("打点记录不可读，原文件已保留".into()),
        };
        if metadata.len() > MAX_BYTES {
            return Err("打点记录超出大小限制，原文件已保留".into());
        }
        let file: MarkFile =
            serde_json::from_slice(&std::fs::read(&self.path).map_err(|_| "打点记录不可读")?)
                .map_err(|_| "打点记录格式无效，原文件已保留")?;
        if file.format != 1
            || file.marks.len() > MAX_MARKS
            || file.marks.iter().any(|mark| !valid_mark(mark))
        {
            return Err("打点记录格式无效，原文件已保留".into());
        }
        Ok(file.marks)
    }

    fn save(&self, marks: Vec<SessionMark>) -> Result<()> {
        let bytes =
            serde_json::to_vec(&MarkFile { format: 1, marks }).map_err(|_| "打点记录格式无效")?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("打点记录超出大小限制".into());
        }
        storage::atomic(&self.path, &bytes)
    }

    pub(crate) fn get(&self, id: &str) -> Result<SessionMark> {
        let _gate = self.gate.lock().map_err(|_| "打点记录不可用")?;
        self.load()?
            .into_iter()
            .find(|mark| mark.id == id)
            .ok_or_else(|| "找不到此打点".into())
    }

    fn existing(
        &self,
        anchor: &str,
        session: &str,
        key: Option<&str>,
    ) -> Result<Option<SessionMark>> {
        let Some(key) = key else {
            return Ok(None);
        };
        let _gate = self.gate.lock().map_err(|_| "打点记录不可用")?;
        Ok(self.load()?.into_iter().find(|mark| {
            mark.anchor_id == anchor
                && mark.session_id == session
                && mark.request_key.as_deref() == Some(key)
        }))
    }

    fn insert(&self, mark: SessionMark) -> Result<Value> {
        let _gate = self.gate.lock().map_err(|_| "打点记录不可用")?;
        let mut marks = self.load()?;
        if let Some(existing) = marks.iter().find(|old| {
            mark.request_key.is_some()
                && old.anchor_id == mark.anchor_id
                && old.session_id == mark.session_id
                && old.request_key == mark.request_key
        }) {
            return Ok(json!({"mark": existing, "created": false}));
        }
        if marks.len() >= MAX_MARKS {
            return Err("打点已达 10000 条，请先删除不需要的打点".into());
        }
        marks.push(mark.clone());
        self.save(marks)?;
        Ok(json!({"mark":mark,"created":true}))
    }

    fn list(&self, p: &Value) -> Result<Value> {
        let anchor = optional_identity(p, "anchor_id")?;
        let session = optional_identity(p, "session_id")?;
        let limit = p
            .get("limit")
            .map(|v| {
                v.as_u64()
                    .filter(|n| (1..=1000).contains(n))
                    .ok_or("打点分页参数无效")
            })
            .transpose()?
            .unwrap_or(1000) as usize;
        let offset = p
            .get("offset")
            .map(|v| {
                v.as_u64()
                    .filter(|n| *n <= MAX_MARKS as u64)
                    .ok_or("打点分页参数无效")
            })
            .transpose()?
            .unwrap_or(0) as usize;
        let _gate = self.gate.lock().map_err(|_| "打点记录不可用")?;
        let mut marks: Vec<_> = self
            .load()?
            .into_iter()
            .filter(|mark| {
                anchor.is_none_or(|a| a == mark.anchor_id)
                    && session.is_none_or(|s| s == mark.session_id)
            })
            .collect();
        marks.sort_by(|a, b| {
            a.marked_at_epoch
                .total_cmp(&b.marked_at_epoch)
                .then_with(|| a.id.cmp(&b.id))
        });
        let total = marks.len();
        Ok(
            json!({"marks":marks.into_iter().skip(offset).take(limit).collect::<Vec<_>>(),"total":total,"limit":limit,"offset":offset,"server_epoch":epoch()}),
        )
    }

    fn update(&self, p: &Value) -> Result<Value> {
        let id = mark_id(p)?;
        let title = p
            .get("title")
            .map(|v| clean_text(v, 80, false, "打点标题无效"))
            .transpose()?;
        let note = p
            .get("note")
            .map(|v| clean_text(v, 2000, true, "打点备注无效"))
            .transpose()?;
        if title.is_none() && note.is_none() {
            return Err("请提供打点标题或备注".into());
        }
        let _gate = self.gate.lock().map_err(|_| "打点记录不可用")?;
        let mut marks = self.load()?;
        let mark = marks
            .iter_mut()
            .find(|mark| mark.id == id)
            .ok_or("找不到此打点")?;
        if let Some(title) = title {
            mark.title = if title.is_empty() {
                "直播打点".into()
            } else {
                title
            };
        }
        if let Some(note) = note {
            mark.note = note;
        }
        mark.updated_at = storage::now();
        let mark = mark.clone();
        self.save(marks)?;
        Ok(json!({"mark":mark}))
    }

    fn delete(&self, p: &Value) -> Result<Value> {
        let id = mark_id(p)?;
        let _gate = self.gate.lock().map_err(|_| "打点记录不可用")?;
        let mut marks = self.load()?;
        let before = marks.len();
        marks.retain(|mark| mark.id != id);
        if marks.len() == before {
            return Err("找不到此打点".into());
        }
        self.save(marks)?;
        Ok(json!({"deleted":true,"id":id}))
    }
}

fn epoch() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64 / 1000.
}
fn optional_identity<'a>(p: &'a Value, field: &str) -> Result<Option<&'a str>> {
    p.get(field)
        .map(|v| {
            v.as_str()
                .filter(|s| storage::id(s))
                .ok_or_else(|| "主播或场次 ID 无效".into())
        })
        .transpose()
}
fn mark_id(p: &Value) -> Result<&str> {
    p["id"]
        .as_str()
        .filter(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| "打点 ID 无效".into())
}
fn clean_text(value: &Value, max: usize, multiline: bool, error: &str) -> Result<String> {
    let text = value.as_str().ok_or_else(|| error.to_owned())?.trim();
    if text.chars().count() > max
        || text
            .chars()
            .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\r' | '\t')))
    {
        return Err(error.into());
    }
    Ok(text.into())
}
fn valid_mark(mark: &SessionMark) -> bool {
    mark.id.len() == 32
        && mark.id.bytes().all(|b| b.is_ascii_hexdigit())
        && storage::id(&mark.anchor_id)
        && storage::id(&mark.session_id)
        && storage::id(&mark.room_id)
        && mark.marked_at_epoch.is_finite()
        && mark.marked_at_epoch > 0.
        && mark.title.chars().count() <= 80
        && mark.note.chars().count() <= 2000
        && mark.anchor_name.chars().count() <= 120
        && mark
            .broadcast_started_at
            .is_none_or(|t| t.is_finite() && t > 0.)
        && mark
            .request_key
            .as_ref()
            .is_none_or(|key| valid_request_key(key))
}
fn valid_request_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 80
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

impl Runtime {
    pub(crate) fn mark_request(&self, method: &str, p: Value) -> Result<Value> {
        match method {
            "mark_list" => self.session_marks.list(&p),
            "mark_update" => self.session_marks.update(&p),
            "mark_delete" => self.session_marks.delete(&p),
            "mark_create" => {
                let anchor = optional_identity(&p, "anchor_id")?.ok_or("缺少主播 ID")?;
                let session = optional_identity(&p, "session_id")?.ok_or("缺少直播场次 ID")?;
                let request_key = p
                    .get("request_key")
                    .map(|v| {
                        v.as_str()
                            .filter(|key| valid_request_key(key))
                            .ok_or("打点请求标识无效")
                    })
                    .transpose()?;
                if let Some(mark) = self.session_marks.existing(anchor, session, request_key)? {
                    return Ok(json!({"mark":mark,"created":false}));
                }
                let title = p
                    .get("title")
                    .map(|v| clean_text(v, 80, false, "打点标题无效"))
                    .transpose()?
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "直播打点".into());
                let note = p
                    .get("note")
                    .map(|v| clean_text(v, 2000, true, "打点备注无效"))
                    .transpose()?
                    .unwrap_or_default();
                // Check the current collector identity, never restored archive metadata.
                // Capture time under the same state lock as a possible session rollover.
                let mark = {
                    let state = self.shared.state.lock().map_err(|_| "直播状态不可用")?;
                    let (room_id, room) = state
                        .rooms
                        .iter()
                        .find(|(_, room)| {
                            room.value["anchor_id"] == anchor
                                && room.value["session_id"] == session
                                && room.value["is_live"] == true
                                && room.value["active"] == true
                                && !room.cancel.is_cancelled()
                                && (room.value["state"] == "connected"
                                    || room.value["live_checked_at"]
                                        .as_str()
                                        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
                                        .is_some_and(|t| {
                                            (chrono::Utc::now().timestamp_millis()
                                                - t.timestamp_millis())
                                            .abs()
                                                <= 90_000
                                        }))
                        })
                        .ok_or("当前直播状态或场次已变化，请刷新后再打点")?;
                    let now = storage::now();
                    SessionMark {
                        id: format!("{:032x}", rand::random::<u128>()),
                        anchor_id: anchor.into(),
                        session_id: session.into(),
                        room_id: room_id.clone(),
                        anchor_name: room.value["name"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .take(120)
                            .collect(),
                        title,
                        note,
                        marked_at_epoch: epoch(),
                        marked_at: now.clone(),
                        created_at: now.clone(),
                        updated_at: now,
                        broadcast_started_at: room.value["broadcast_started_at"]
                            .as_f64()
                            .filter(|t| t.is_finite() && *t > 0.),
                        request_key: request_key.map(str::to_owned),
                    }
                };
                self.session_marks.insert(mark)
            }
            _ => Err("不支持的打点操作".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::Room;
    use tokio_util::sync::CancellationToken;

    fn live_room(runtime: &Runtime, session: &str, live: bool) {
        runtime.shared.state.lock().unwrap().rooms.insert("123".into(),Room {
            generation:1,value:json!({"name":"打点测试主播","anchor_id":"456","session_id":session,"is_live":live,"active":true,"state":"connected","broadcast_started_at":epoch()-60.}),
            route:None,cancel:CancellationToken::new(),
        });
    }

    #[test]
    fn live_marks_survive_stop_restart_and_keep_original_wall_clock() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(directory.path().into()).unwrap();
        live_room(&runtime, "789", true);
        let before = epoch();
        let created = runtime.request("mark_create",json!({"anchor_id":"456","session_id":"789","title":"精彩对局","request_key":"click-1","marked_at_epoch":1})).unwrap();
        let mark = &created["mark"];
        assert!(mark["marked_at_epoch"].as_f64().unwrap() >= before);
        assert_eq!(mark["session_id"], "789");
        assert_eq!(mark["anchor_id"], "456");
        assert_eq!(mark["room_id"], "123");
        live_room(&runtime, "789", false);
        assert!(
            runtime
                .request("mark_create", json!({"anchor_id":"456","session_id":"789"}))
                .is_err()
        );
        let duplicate = runtime
            .request(
                "mark_create",
                json!({"anchor_id":"456","session_id":"789","request_key":"click-1"}),
            )
            .unwrap();
        assert_eq!(duplicate["mark"]["id"], mark["id"]);
        assert_eq!(duplicate["created"], false);
        runtime.shutdown().unwrap();
        drop(runtime);
        let reopened = Runtime::open(directory.path().into()).unwrap();
        let listed = reopened
            .request("mark_list", json!({"anchor_id":"456","session_id":"789"}))
            .unwrap();
        assert_eq!(listed["total"], 1);
        assert_eq!(
            listed["marks"][0]["marked_at_epoch"],
            mark["marked_at_epoch"]
        );
        assert!(
            reopened
                .request("mark_create", json!({"anchor_id":"456","session_id":"789"}))
                .is_err(),
            "restored metadata must not permit stale live bookmarks"
        );
        let updated = reopened.request("mark_update",json!({"id":mark["id"],"title":"改名","note":"稍后精选\n对局结尾","marked_at_epoch":1})).unwrap();
        assert_eq!(updated["mark"]["title"], "改名");
        assert_eq!(updated["mark"]["marked_at_epoch"], mark["marked_at_epoch"]);
        assert_eq!(
            reopened
                .request("mark_delete", json!({"id":mark["id"]}))
                .unwrap()["deleted"],
            true
        );
        assert_eq!(
            reopened.request("mark_list", json!({})).unwrap()["total"],
            0
        );
        reopened.shutdown().unwrap();
    }

    #[test]
    fn session_rollover_and_anchor_change_never_retarget_a_live_mark() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(directory.path().into()).unwrap();
        live_room(&runtime, "789", true);
        let first = runtime
            .request("mark_create", json!({"anchor_id":"456","session_id":"789"}))
            .unwrap();
        live_room(&runtime, "790", true);
        assert!(
            runtime
                .request("mark_create", json!({"anchor_id":"456","session_id":"789"}))
                .is_err()
        );
        assert!(
            runtime
                .request("mark_create", json!({"anchor_id":"999","session_id":"790"}))
                .is_err()
        );
        assert_eq!(
            runtime
                .request("mark_list", json!({"session_id":"789"}))
                .unwrap()["marks"][0]["id"],
            first["mark"]["id"]
        );
        assert_eq!(
            runtime
                .request("mark_list", json!({"session_id":"790"}))
                .unwrap()["total"],
            0
        );
        assert!(
            runtime
                .request(
                    "mark_create",
                    json!({"anchor_id":"456","session_id":"790","title":"a\u{0000}b"})
                )
                .is_err()
        );
        assert!(
            runtime
                .request("mark_list", json!({"session_id":790}))
                .is_err()
        );
        runtime.shutdown().unwrap();
    }

    #[test]
    fn corrupt_mark_file_is_preserved_instead_of_replaced_by_new_mark() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(directory.path().into()).unwrap();
        live_room(&runtime, "789", true);
        let path = directory.path().join("session-marks.json");
        std::fs::write(&path, b"broken original file").unwrap();
        assert!(
            runtime
                .request("mark_create", json!({"anchor_id":"456","session_id":"789"}))
                .is_err()
        );
        assert!(runtime.request("mark_list", json!({})).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"broken original file");
        runtime.shutdown().unwrap();
    }
}
