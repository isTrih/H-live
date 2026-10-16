//! Local, explainable interaction analysis. Raw messages remain authoritative.
//! One bounded step commits its cursor, classifications and aggregates together.
use super::*;
use crate::export::CancelToken;
use unicode_normalization::UnicodeNormalization;

pub const RULE_VERSION: &str = "interaction-v1";
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS quality_registry(
 anchor_id TEXT NOT NULL,session_id TEXT NOT NULL,anchor_name TEXT NOT NULL,web_rid TEXT NOT NULL,
 title TEXT NOT NULL,first_seen TEXT NOT NULL,last_seen TEXT NOT NULL,started_at TEXT,
 coverage TEXT NOT NULL DEFAULT 'unknown',generation INTEGER NOT NULL DEFAULT 1,
 cleared INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(anchor_id,session_id));
CREATE TABLE IF NOT EXISTS quality_meta(key TEXT PRIMARY KEY,value INTEGER NOT NULL);
INSERT OR IGNORE INTO quality_meta VALUES('cursor',0);
CREATE TABLE IF NOT EXISTS quality_jobs(anchor_id TEXT NOT NULL,session_id TEXT NOT NULL,
 generation INTEGER NOT NULL,cursor INTEGER NOT NULL DEFAULT 0,state TEXT NOT NULL DEFAULT 'queued',
 turn INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(anchor_id,session_id));
CREATE TABLE IF NOT EXISTS quality_gc(anchor_id TEXT NOT NULL,session_id TEXT NOT NULL,generation INTEGER NOT NULL,PRIMARY KEY(anchor_id,session_id,generation));
CREATE TABLE IF NOT EXISTS quality_pending(anchor_id TEXT NOT NULL,session_id TEXT NOT NULL,generation INTEGER NOT NULL,normalized TEXT NOT NULL,start REAL NOT NULL,end REAL NOT NULL,PRIMARY KEY(anchor_id,session_id,generation,normalized,start,end));
CREATE TABLE IF NOT EXISTS quality_settings(anchor_id TEXT PRIMARY KEY,config TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS quality_manual(anchor_id TEXT NOT NULL,session_id TEXT NOT NULL,
 normalized TEXT NOT NULL,content TEXT NOT NULL,PRIMARY KEY(anchor_id,session_id,normalized));
CREATE TABLE IF NOT EXISTS quality_commands(anchor_id TEXT NOT NULL,session_id TEXT NOT NULL,
 generation INTEGER NOT NULL,normalized TEXT NOT NULL,start REAL NOT NULL,end REAL NOT NULL,
 PRIMARY KEY(anchor_id,session_id,generation,normalized,start,end));
CREATE TABLE IF NOT EXISTS quality_results(message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
 generation INTEGER NOT NULL,anchor_id TEXT NOT NULL,session_id TEXT NOT NULL,sender TEXT NOT NULL,
 normalized TEXT NOT NULL,observed REAL NOT NULL,verdict INTEGER NOT NULL,reason TEXT NOT NULL,
 PRIMARY KEY(message_id,generation));
CREATE INDEX IF NOT EXISTS quality_text_window ON quality_results(anchor_id,session_id,generation,normalized,observed);
CREATE INDEX IF NOT EXISTS quality_user_window ON quality_results(anchor_id,session_id,generation,normalized,sender,observed);
CREATE INDEX IF NOT EXISTS quality_valid_text ON quality_results(anchor_id,session_id,generation,normalized,verdict,observed);
CREATE INDEX IF NOT EXISTS quality_reason ON quality_results(anchor_id,session_id,generation,reason,message_id);
CREATE TABLE IF NOT EXISTS quality_users(anchor_id TEXT NOT NULL,session_id TEXT NOT NULL,generation INTEGER NOT NULL,
 sender TEXT NOT NULL,total INTEGER NOT NULL DEFAULT 0,valid INTEGER NOT NULL DEFAULT 0 CHECK(valid>=0),
 PRIMARY KEY(anchor_id,session_id,generation,sender));
CREATE TABLE IF NOT EXISTS quality_groups(anchor_id TEXT NOT NULL,session_id TEXT NOT NULL,generation INTEGER NOT NULL,
 normalized TEXT NOT NULL,total INTEGER NOT NULL DEFAULT 0,users INTEGER NOT NULL DEFAULT 0,last_seen REAL NOT NULL,
 PRIMARY KEY(anchor_id,session_id,generation,normalized));
CREATE TABLE IF NOT EXISTS quality_senders(anchor_id TEXT NOT NULL,session_id TEXT NOT NULL,generation INTEGER NOT NULL,
 normalized TEXT NOT NULL,sender TEXT NOT NULL,last_seen REAL NOT NULL,
 PRIMARY KEY(anchor_id,session_id,generation,normalized,sender));
CREATE INDEX IF NOT EXISTS quality_recent_senders ON quality_senders(anchor_id,session_id,generation,normalized,last_seen);
CREATE TABLE IF NOT EXISTS quality_stats(anchor_id TEXT NOT NULL,session_id TEXT NOT NULL,generation INTEGER NOT NULL,
 analyzed INTEGER NOT NULL DEFAULT 0,valid INTEGER NOT NULL DEFAULT 0 CHECK(valid>=0),unknown_sender INTEGER NOT NULL DEFAULT 0,uncertain INTEGER NOT NULL DEFAULT 0 CHECK(uncertain>=0),
 PRIMARY KEY(anchor_id,session_id,generation));
CREATE INDEX IF NOT EXISTS messages_quality_session ON messages(anchor_id,session_id,message_type,id);
"#;

pub(super) fn initialize(c: &Connection) -> Result<()> {
    c.execute_batch(SCHEMA)?;
    c.execute_batch("INSERT OR IGNORE INTO quality_registry(anchor_id,session_id,anchor_name,web_rid,title,first_seen,last_seen,started_at) SELECT anchor_id,session_id,anchor_name,web_rid,title,first_seen,last_seen,started_at FROM sessions;
      INSERT OR IGNORE INTO quality_registry(anchor_id,session_id,anchor_name,web_rid,title,first_seen,last_seen,coverage) SELECT d.anchor_id,d.session_id,COALESCE(a.anchor_name,d.anchor_id),COALESCE(a.web_rid,''),'',d.last_checkpoint_at,d.last_checkpoint_at,'unknown' FROM session_monitor_durations d LEFT JOIN anchors a ON a.anchor_id=d.anchor_id;")?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Rules {
    pub window_seconds: u32,
    pub self_count: u32,
    pub crowd_count: u32,
    pub crowd_users: u32,
    pub session_count: u32,
    pub session_users: u32,
}
impl Default for Rules {
    fn default() -> Self {
        Self {
            window_seconds: 60,
            self_count: 3,
            crowd_count: 20,
            crowd_users: 10,
            session_count: 50,
            session_users: 20,
        }
    }
}
impl Rules {
    fn validate(&self) -> Result<()> {
        if !(5..=600).contains(&self.window_seconds)
            || !(2..=20).contains(&self.self_count)
            || !(5..=200).contains(&self.crowd_count)
            || !(2..=100).contains(&self.crowd_users)
            || self.crowd_users > self.crowd_count
            || !(10..=1000).contains(&self.session_count)
            || !(2..=500).contains(&self.session_users)
            || self.session_users > self.session_count
        {
            return Err(ArchiveError::Invalid(
                "重复规则超出范围，人数不能大于消息条数".into(),
            ));
        }
        Ok(())
    }
}
/// NFC and conservative width/space comparison. Punctuation, digits, negation,
/// emoji joiners and repeated letters retain their meaning; raw content is untouched.
pub fn normalize(text: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for ch in text.nfc() {
        let ch = if ('\u{ff01}'..='\u{ff5e}').contains(&ch) {
            char::from_u32(ch as u32 - 0xfee0).unwrap()
        } else {
            ch
        };
        if ch.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.extend(ch.to_lowercase());
    }
    out
}
fn id_arg(v: &Value, key: &str) -> Result<String> {
    let s = v
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ArchiveError::Invalid(format!("缺少{key}")))?;
    identity(s)?;
    Ok(s.into())
}
fn page(v: &Value, key: &str, default: usize, max: usize) -> Result<usize> {
    let n = match v.get(key) {
        None => default,
        Some(n) => {
            n.as_u64()
                .ok_or_else(|| ArchiveError::Invalid("分页参数无效".into()))? as usize
        }
    };
    if n > max || (key == "limit" && n == 0) {
        return Err(ArchiveError::Invalid("分页参数超出范围".into()));
    }
    Ok(n)
}
fn rules(c: &Connection, a: &str) -> Result<Rules> {
    let raw: Option<String> = c
        .query_row(
            "SELECT config FROM quality_settings WHERE anchor_id=?",
            [a],
            |r| r.get(0),
        )
        .optional()?;
    Ok(match raw {
        Some(raw) => serde_json::from_str(&raw)?,
        None => Rules::default(),
    })
}
fn enqueue(c: &Connection, a: &str, s: Option<&str>) -> Result<usize> {
    let condition = if s.is_some() {
        "anchor_id=?1 AND session_id=?2"
    } else {
        "anchor_id=?1"
    };
    let values: Vec<String> = std::iter::once(a.to_owned())
        .chain(s.map(str::to_owned))
        .collect();
    c.execute(&format!("INSERT OR IGNORE INTO quality_gc SELECT anchor_id,session_id,generation FROM quality_registry WHERE {condition}"),params_from_iter(&values))?;
    let n = c.execute(
        &format!("UPDATE quality_registry SET generation=generation+1 WHERE {condition}"),
        params_from_iter(&values),
    )?;
    c.execute(&format!("INSERT INTO quality_jobs(anchor_id,session_id,generation,cursor,state,turn) SELECT anchor_id,session_id,generation,0,'queued',0 FROM quality_registry WHERE {condition} ON CONFLICT(anchor_id,session_id) DO UPDATE SET generation=excluded.generation,cursor=0,state='queued',turn=0"),params_from_iter(&values))?;
    Ok(n)
}
pub(super) fn after_delete(c: &Connection) -> Result<()> {
    c.execute_batch("INSERT OR IGNORE INTO quality_gc SELECT r.anchor_id,r.session_id,r.generation FROM quality_registry r JOIN delete_sessions d USING(anchor_id,session_id);
      UPDATE quality_registry SET generation=generation+1,cleared=1 WHERE EXISTS(SELECT 1 FROM delete_sessions d WHERE d.anchor_id=quality_registry.anchor_id AND d.session_id=quality_registry.session_id);
      INSERT INTO quality_jobs(anchor_id,session_id,generation,cursor,state,turn) SELECT r.anchor_id,r.session_id,r.generation,0,'queued',0 FROM quality_registry r JOIN delete_sessions d USING(anchor_id,session_id) WHERE 1 ON CONFLICT(anchor_id,session_id) DO UPDATE SET generation=excluded.generation,cursor=0,state='queued',turn=0;")?;
    Ok(())
}

impl ArchiveStore {
    /// Read operations use one WAL snapshot; bounded writes share deletion's gate.
    pub fn quality_request(&self, action: &str, p: Value, cancel: CancelToken) -> Result<Value> {
        if matches!(action, "sessions" | "detail" | "settings") {
            let snap = self.snapshot()?;
            return read(&snap.connection, action, &p);
        }
        let _permit = match self.inner.mutation_gate.try_read() {
            Ok(v) => v,
            Err(std::sync::TryLockError::WouldBlock) if action == "step" => {
                return Ok(json!({"processed":0,"busy":true}))
            }
            Err(_) => return Err(ArchiveError::Invalid("记录清理正在进行，请稍后再试".into())),
        };
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| ArchiveError::WorkerStopped)?;
        if state.closed {
            return Err(ArchiveError::Closed);
        }
        let mut c = connect(&self.inner.path)?;
        c.busy_timeout(Duration::from_millis(150))?;
        let stop = cancel.clone();
        c.progress_handler(500, Some(move || stop.is_cancelled()))?;
        let result = (|| {
            if cancel.is_cancelled() {
                return Err(ArchiveError::Invalid("分析已取消".into()));
            }
            let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let value = match action {
                "audience_record" => super::audience::record(&tx, &p)?,
                "register" => {
                    let a = id_arg(&p, "anchor_id")?;
                    let s = id_arg(&p, "session_id")?;
                    let name = p["anchor_name"].as_str().unwrap_or(&a);
                    let rid = p["web_rid"].as_str().unwrap_or("");
                    let now = Utc::now()
                        .with_timezone(&shanghai())
                        .to_rfc3339_opts(SecondsFormat::Millis, false);
                    let started = p["started_at"]
                        .as_str()
                        .filter(|v| !v.is_empty())
                        .map(parse_time)
                        .transpose()?
                        .map(|d| d.to_rfc3339_opts(SecondsFormat::Millis, false));
                    tx.execute("INSERT INTO quality_registry(anchor_id,session_id,anchor_name,web_rid,title,first_seen,last_seen,started_at,coverage) VALUES(?,?,?,?,?,?,?,?, 'partial') ON CONFLICT(anchor_id,session_id) DO UPDATE SET anchor_name=excluded.anchor_name,web_rid=excluded.web_rid,title=excluded.title,last_seen=excluded.last_seen,started_at=COALESCE(quality_registry.started_at,excluded.started_at)",params![a,s,name,rid,p["title"].as_str().unwrap_or(""),now,now,started])?;
                    json!({"registered":true})
                }
                "step" => step(&tx, page(&p, "limit", 128, 512)?, &cancel)?,
                "rebuild" => {
                    let a = id_arg(&p, "anchor_id")?;
                    let s = p
                        .get("session_id")
                        .map(|_| id_arg(&p, "session_id"))
                        .transpose()?;
                    json!({"queued":enqueue(&tx,&a,s.as_deref())?})
                }
                "pause" => {
                    let a = id_arg(&p, "anchor_id")?;
                    let s = id_arg(&p, "session_id")?;
                    let n = tx.execute(
                        "UPDATE quality_jobs SET state='paused' WHERE anchor_id=? AND session_id=?",
                        params![a, s],
                    )?;
                    json!({"paused":n>0})
                }
                "configure" => {
                    let a = id_arg(&p, "anchor_id")?;
                    let cfg: Rules = serde_json::from_value(p["rules"].clone())?;
                    cfg.validate()?;
                    tx.execute("INSERT INTO quality_settings VALUES(?,?) ON CONFLICT(anchor_id) DO UPDATE SET config=excluded.config",params![a,serde_json::to_string(&cfg)?])?;
                    json!({"queued":enqueue(&tx,&a,None)?,"rules":cfg})
                }
                "commands" => {
                    let a = id_arg(&p, "anchor_id")?;
                    let s = id_arg(&p, "session_id")?;
                    let list = p["commands"]
                        .as_array()
                        .filter(|v| v.len() <= 50)
                        .ok_or_else(|| ArchiveError::Invalid("最多配置50条本场活动口令".into()))?;
                    tx.execute(
                        "DELETE FROM quality_manual WHERE anchor_id=? AND session_id=?",
                        params![a, s],
                    )?;
                    for v in list {
                        let text = v
                            .as_str()
                            .filter(|v| !v.trim().is_empty() && v.chars().count() <= 512)
                            .ok_or_else(|| ArchiveError::Invalid("口令须为1–512字文本".into()))?;
                        tx.execute(
                            "INSERT OR IGNORE INTO quality_manual VALUES(?,?,?,?)",
                            params![a, s, normalize(text), text],
                        )?;
                    }
                    json!({"queued":enqueue(&tx,&a,Some(&s))?})
                }
                _ => return Err(ArchiveError::Invalid("未知分析操作".into())),
            };
            if cancel.is_cancelled() {
                return Err(ArchiveError::Invalid("分析已取消".into()));
            }
            tx.commit()?;
            Ok(value)
        })();
        c.progress_handler(0, None::<fn() -> bool>)?;
        if !c.is_autocommit() {
            c.execute_batch("ROLLBACK;")?;
        }
        result
    }
}

fn step(c: &Connection, limit: usize, cancel: &CancelToken) -> Result<Value> {
    let begin = Instant::now();
    let cursor: i64 = c.query_row(
        "SELECT value FROM quality_meta WHERE key='cursor'",
        [],
        |r| r.get(0),
    )?;
    let ids = c
        .prepare("SELECT id FROM messages WHERE id>? ORDER BY id LIMIT ?")?
        .query_map(params![cursor, limit as i64], |r| r.get::<_, i64>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut processed = 0;
    for id in ids {
        if cancel.is_cancelled() {
            return Err(ArchiveError::Invalid("分析已取消".into()));
        }
        process(c, id, false)?;
        c.execute("UPDATE quality_meta SET value=? WHERE key='cursor'", [id])?;
        processed += 1;
        if begin.elapsed() > Duration::from_millis(35) {
            break;
        }
    }
    let job:Option<(String,String,i64,i64)>=c.query_row("SELECT anchor_id,session_id,generation,cursor FROM quality_jobs WHERE state='queued' ORDER BY turn,anchor_id,session_id LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    if let Some((a, s, g, cursor)) = job {
        let ids=c.prepare("SELECT id FROM messages WHERE anchor_id=? AND session_id=? AND id>? ORDER BY id LIMIT ?")?.query_map(params![a,s,cursor,limit as i64],|r|r.get::<_,i64>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        if ids.is_empty() {
            c.execute(
                "DELETE FROM quality_jobs WHERE anchor_id=? AND session_id=? AND generation=?",
                params![a, s, g],
            )?;
        } else {
            for id in ids {
                process(c, id, true)?;
                c.execute("UPDATE quality_jobs SET cursor=? WHERE anchor_id=? AND session_id=? AND generation=?",params![id,a,s,g])?;
                processed += 1;
                if begin.elapsed() > Duration::from_millis(70) {
                    break;
                }
            }
            c.execute("UPDATE quality_jobs SET turn=(SELECT COALESCE(MAX(turn),0)+1 FROM quality_jobs) WHERE anchor_id=? AND session_id=?",params![a,s])?;
        }
    }
    // Late activity evidence is applied in bounded batches, independent of ingestion.
    let pending:Option<(String,String,i64,String,f64,f64)>=c.query_row("SELECT p.anchor_id,p.session_id,p.generation,p.normalized,p.start,p.end FROM quality_pending p JOIN quality_registry r USING(anchor_id,session_id,generation) WHERE NOT EXISTS(SELECT 1 FROM quality_jobs j WHERE j.anchor_id=p.anchor_id AND j.session_id=p.session_id AND j.state='paused') LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    if let Some((a, s, g, norm, start, end)) = pending {
        let more = apply_activity(c, &a, &s, g, &norm, start, end)?;
        if !more {
            c.execute("DELETE FROM quality_pending WHERE anchor_id=? AND session_id=? AND generation=? AND normalized=? AND start=? AND end=?",params![a,s,g,norm,start,end])?;
        }
        processed += 1;
    }
    // Reclaim one retired generation via indexed lookups, never scan all live results.
    let retired: Option<(String, String, i64)> = c
        .query_row(
            "SELECT anchor_id,session_id,generation FROM quality_gc LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if let Some((a, s, g)) = retired {
        let mut full = false;
        for table in [
            "quality_results",
            "quality_users",
            "quality_groups",
            "quality_senders",
            "quality_stats",
            "quality_commands",
            "quality_pending",
        ] {
            let removed=c.execute(&format!("DELETE FROM {table} WHERE rowid IN(SELECT rowid FROM {table} WHERE anchor_id=? AND session_id=? AND generation=? LIMIT 64)"),params![a,s,g])?;
            full |= removed == 64;
        }
        if !full {
            c.execute(
                "DELETE FROM quality_gc WHERE anchor_id=? AND session_id=? AND generation=?",
                params![a, s, g],
            )?;
        }
    }
    Ok(json!({"processed":processed,"rule_version":RULE_VERSION}))
}

fn process(c: &Connection, id: i64, rebuilding: bool) -> Result<()> {
    let (a,s,sender,kind,content,received,sent,payload):(String,String,String,String,String,f64,Option<f64>,String)=c.query_row("SELECT anchor_id,session_id,sender_id,message_type,content,received_at_epoch,message_time_epoch,payload_json FROM messages WHERE id=?",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?)))?;
    c.execute("INSERT OR IGNORE INTO quality_registry(anchor_id,session_id,anchor_name,web_rid,title,first_seen,last_seen,started_at) SELECT anchor_id,session_id,anchor_name,web_rid,title,first_seen,last_seen,started_at FROM sessions WHERE anchor_id=? AND session_id=?",params![a,s])?;
    let g: i64 = c.query_row(
        "SELECT generation FROM quality_registry WHERE anchor_id=? AND session_id=?",
        params![a, s],
        |r| r.get(0),
    )?;
    if !rebuilding
        && c.prepare("SELECT 1 FROM quality_jobs WHERE anchor_id=? AND session_id=?")?
            .exists(params![a, s])?
    {
        return Ok(());
    }
    if kind == "activity" {
        activity(c, &a, &s, g, &payload)?;
        return Ok(());
    }
    if kind != "chat" {
        return Ok(());
    }
    if c.prepare("SELECT 1 FROM quality_results WHERE message_id=? AND generation=?")?
        .exists(params![id, g])?
    {
        return Ok(());
    }
    let norm = normalize(&content);
    let prior:Option<f64>=c.query_row("SELECT last_seen FROM quality_groups WHERE anchor_id=? AND session_id=? AND generation=? AND normalized=?",params![a,s,g,norm],|r|r.get(0)).optional()?;
    // Receive-order clock: a backwards wall-clock adjustment cannot put newly
    // processed text before an earlier copy. Never use this as a send timestamp.
    let t = received.max(prior.unwrap_or(received));
    let manual = c
        .prepare(
            "SELECT 1 FROM quality_manual WHERE anchor_id=? AND session_id=? AND normalized=?",
        )?
        .exists(params![a, s, norm])?;
    let automatic = if let Some(time) = sent {
        c.prepare("SELECT 1 FROM quality_commands WHERE anchor_id=? AND session_id=? AND generation=? AND normalized=? AND start<=? AND end>=?")?.exists(params![a,s,g,norm,time,time])?
    } else {
        false
    };
    let uncertain=c.prepare("SELECT 1 FROM quality_commands WHERE anchor_id=? AND session_id=? AND generation=? AND normalized=? AND (start<0 OR ? IS NULL)")?.exists(params![a,s,g,norm,sent])?;
    let reason = if norm.is_empty() {
        "empty"
    } else if manual || automatic {
        "activity_command"
    } else if uncertain {
        "activity_uncertain"
    } else {
        "normal"
    };
    let valid = i64::from(reason == "normal");
    let pending = i64::from(reason == "activity_uncertain");
    c.execute(
        "INSERT INTO quality_results VALUES(?,?,?,?,?,?,?,?,?)",
        params![
            id,
            g,
            a,
            s,
            sender,
            norm,
            t,
            if pending == 1 { -1 } else { valid },
            reason
        ],
    )?;
    c.execute("INSERT INTO quality_stats VALUES(?,?,?,1,?,?,?) ON CONFLICT(anchor_id,session_id,generation) DO UPDATE SET analyzed=analyzed+1,valid=valid+excluded.valid,unknown_sender=unknown_sender+excluded.unknown_sender,uncertain=uncertain+excluded.uncertain",params![a,s,g,valid,i64::from(sender.is_empty()),pending])?;
    if !sender.is_empty() {
        c.execute("INSERT INTO quality_users VALUES(?,?,?,?,1,?) ON CONFLICT(anchor_id,session_id,generation,sender) DO UPDATE SET total=total+1,valid=valid+excluded.valid",params![a,s,g,sender,valid])?;
    }
    c.execute("INSERT INTO quality_groups VALUES(?,?,?,?,1,0,?) ON CONFLICT(anchor_id,session_id,generation,normalized) DO UPDATE SET total=total+1,last_seen=excluded.last_seen",params![a,s,g,norm,t])?;
    if !sender.is_empty() {
        let added = c.execute(
            "INSERT OR IGNORE INTO quality_senders VALUES(?,?,?,?,?,?)",
            params![a, s, g, norm, sender, t],
        )?;
        c.execute("UPDATE quality_senders SET last_seen=? WHERE anchor_id=? AND session_id=? AND generation=? AND normalized=? AND sender=?",params![t,a,s,g,norm,sender])?;
        if added > 0 {
            c.execute("UPDATE quality_groups SET users=users+1 WHERE anchor_id=? AND session_id=? AND generation=? AND normalized=?",params![a,s,g,norm])?;
        }
    }
    let cfg = rules(c, &a)?;
    let (count,users):(u32,u32)=c.query_row("SELECT total,users FROM quality_groups WHERE anchor_id=? AND session_id=? AND generation=? AND normalized=?",params![a,s,g,norm],|r|Ok((r.get(0)?,r.get(1)?)))?;
    if count >= cfg.session_count && users >= cfg.session_users {
        invalidate(c, &a, &s, g, &norm, "session_repeat", None, None)?;
        return Ok(());
    }
    let lo = t - cfg.window_seconds as f64;
    if !sender.is_empty() {
        let count:i64=c.query_row("SELECT COUNT(*) FROM(SELECT 1 FROM quality_results WHERE anchor_id=? AND session_id=? AND generation=? AND normalized=? AND sender=? AND observed>=? AND observed<=? LIMIT ?)",params![a,s,g,norm,sender,lo,t,cfg.self_count],|r|r.get(0))?;
        if count >= cfg.self_count as i64 {
            invalidate(
                c,
                &a,
                &s,
                g,
                &norm,
                "self_repeat",
                Some((&sender, lo, t)),
                None,
            )?;
        }
    }
    if users >= cfg.crowd_users && count >= cfg.crowd_count {
        let recent:i64=c.query_row("SELECT COUNT(*) FROM(SELECT 1 FROM quality_senders WHERE anchor_id=? AND session_id=? AND generation=? AND normalized=? AND last_seen>=? AND last_seen<=? LIMIT ?)",params![a,s,g,norm,lo,t,cfg.crowd_users],|r|r.get(0))?;
        if recent >= cfg.crowd_users as i64 {
            let count:i64=c.query_row("SELECT COUNT(*) FROM(SELECT 1 FROM quality_results WHERE anchor_id=? AND session_id=? AND generation=? AND normalized=? AND observed>=? AND observed<=? LIMIT ?)",params![a,s,g,norm,lo,t,cfg.crowd_count],|r|r.get(0))?;
            if count >= cfg.crowd_count as i64 {
                invalidate(c, &a, &s, g, &norm, "crowd_repeat", None, Some((lo, t)))?;
            }
        }
    }
    Ok(())
}
fn invalidate(
    c: &Connection,
    a: &str,
    s: &str,
    g: i64,
    norm: &str,
    reason: &str,
    user: Option<(&str, f64, f64)>,
    window: Option<(f64, f64)>,
) -> Result<()> {
    let mut filter =
        "anchor_id=? AND session_id=? AND generation=? AND normalized=? AND verdict IN(-1,1)"
            .to_owned();
    let mut args: Vec<rusqlite::types::Value> = vec![
        a.to_owned().into(),
        s.to_owned().into(),
        g.into(),
        norm.to_owned().into(),
    ];
    if let Some((sender, lo, hi)) = user {
        filter.push_str(" AND sender=? AND observed>=? AND observed<=?");
        args.extend([sender.to_owned().into(), lo.into(), hi.into()]);
    }
    if let Some((lo, hi)) = window {
        filter.push_str(" AND observed>=? AND observed<=?");
        args.extend([lo.into(), hi.into()]);
    }
    let changes=c.prepare(&format!("SELECT sender,SUM(verdict=1),SUM(verdict=-1) FROM quality_results WHERE {filter} GROUP BY sender"))?.query_map(params_from_iter(&args),|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
    let (mut total, mut uncertain) = (0, 0);
    for (sender, count, pending) in changes {
        total += count;
        uncertain += pending;
        if !sender.is_empty() {
            c.execute("UPDATE quality_users SET valid=valid-? WHERE anchor_id=? AND session_id=? AND generation=? AND sender=?",params![count,a,s,g,sender])?;
        }
    }
    if total + uncertain > 0 {
        c.execute("UPDATE quality_stats SET valid=valid-?,uncertain=uncertain-? WHERE anchor_id=? AND session_id=? AND generation=?",params![total,uncertain,a,s,g])?;
        let mut update = vec![rusqlite::types::Value::from(reason.to_owned())];
        update.extend(args);
        c.execute(
            &format!("UPDATE quality_results SET verdict=0,reason=? WHERE {filter}"),
            params_from_iter(update),
        )?;
    }
    Ok(())
}

// A merged lottery can acquire participation evidence after its row cursor was
// consumed. Queue that evidence in the current generation without rescanning chat.
pub(super) fn refresh_activity(c: &Connection, a: &str, s: &str, payload: &str) -> Result<()> {
    if let Some(g) = c
        .query_row(
            "SELECT generation FROM quality_registry WHERE anchor_id=? AND session_id=?",
            params![a, s],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    {
        activity(c, a, s, g, payload)?;
    }
    Ok(())
}

fn activity(c: &Connection, a: &str, s: &str, g: i64, payload: &str) -> Result<()> {
    let p: Value = serde_json::from_str(payload)?;
    let d = &p["activity_details"];
    let start = d["start_at"].as_f64().filter(|v| *v > 0.0);
    let end = d["draw_at"].as_f64().filter(|v| *v > 0.0);
    let (start, end) = match (start, end) {
        (Some(a), Some(b)) if b >= a => (a, b),
        _ => (-1.0, -1.0),
    };
    if let Some(conditions) = d["conditions"].as_array() {
        for condition in conditions.iter().take(100) {
            let text = condition["content"].as_str().unwrap_or("");
            // A bare condition string/type code has no proven enum semantics.
            // Only explicit quoted comment instructions qualify automatically.
            let phrase = quoted_comment(text);
            if let Some(phrase) = phrase {
                let norm = normalize(phrase);
                if norm.is_empty() {
                    continue;
                }
                let n = c.execute(
                    "INSERT OR IGNORE INTO quality_commands VALUES(?,?,?,?,?,?)",
                    params![a, s, g, norm, start, end],
                )?;
                if n > 0 {
                    c.execute(
                        "INSERT OR IGNORE INTO quality_pending VALUES(?,?,?,?,?,?)",
                        params![a, s, g, norm, start, end],
                    )?;
                }
            }
        }
    }
    Ok(())
}
fn apply_activity(
    c: &Connection,
    a: &str,
    s: &str,
    g: i64,
    norm: &str,
    start: f64,
    end: f64,
) -> Result<bool> {
    let ids=c.prepare("SELECT q.message_id,q.sender,m.message_time_epoch,q.verdict FROM quality_results q JOIN messages m ON m.id=q.message_id WHERE q.anchor_id=? AND q.session_id=? AND q.generation=? AND q.normalized=? AND ((q.verdict=1 AND (?<0 OR m.message_time_epoch IS NULL OR (m.message_time_epoch>=? AND m.message_time_epoch<=?))) OR (q.verdict=-1 AND ?>=0 AND m.message_time_epoch>=? AND m.message_time_epoch<=?)) LIMIT 128")?.query_map(params![a,s,g,norm,start,start,end,start,start,end],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<f64>>(2)?,r.get::<_,i64>(3)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
    if ids.is_empty() {
        return Ok(false);
    }
    for (id, sender, sent, old) in ids {
        let pending = i64::from(start < 0.0 || sent.is_none());
        let was_valid = i64::from(old == 1);
        let was_pending = i64::from(old == -1);
        c.execute(
            "UPDATE quality_results SET verdict=?,reason=? WHERE message_id=? AND generation=?",
            params![
                if pending == 1 { -1 } else { 0 },
                if pending == 1 {
                    "activity_uncertain"
                } else {
                    "activity_command"
                },
                id,
                g
            ],
        )?;
        c.execute("UPDATE quality_stats SET valid=valid-?,uncertain=uncertain+? WHERE anchor_id=? AND session_id=? AND generation=?",params![was_valid,pending-was_pending,a,s,g])?;
        if !sender.is_empty() {
            c.execute("UPDATE quality_users SET valid=valid-? WHERE anchor_id=? AND session_id=? AND generation=? AND sender=?",params![was_valid,a,s,g,sender])?;
        }
    }
    Ok(true)
}

fn quoted_comment(text: &str) -> Option<&str> {
    let t = text.trim();
    if !["发送评论", "发表评论", "评论内容"]
        .iter()
        .any(|p| t.starts_with(p))
    {
        return None;
    }
    for (left, right) in [('「', '」'), ('“', '”'), ('"', '"'), ('【', '】')] {
        if let Some(i) = t.find(left) {
            let tail = &t[i + left.len_utf8()..];
            if let Some(j) = tail.find(right) {
                return Some(&tail[..j]);
            }
        }
    }
    None
}

fn read(c: &Connection, action: &str, p: &Value) -> Result<Value> {
    let a = id_arg(p, "anchor_id")?;
    if action == "settings" {
        return Ok(json!({"rules":rules(c,&a)?,"rule_version":RULE_VERSION}));
    }
    if action == "sessions" {
        let limit = page(p, "limit", 20, 100)?;
        let offset = page(p, "offset", 0, 1_000_000)?;
        let month = p.get("month").and_then(Value::as_str).unwrap_or("");
        if !month.is_empty() {
            month_bounds(month)?;
        }
        let filter="r.anchor_id=?1 AND (?2='' OR substr(COALESCE(NULLIF(r.started_at,''),r.first_seen),1,7)=?2)";
        let total: i64 = c.query_row(
            &format!("SELECT COUNT(*) FROM quality_registry r WHERE {filter}"),
            params![a, month],
            |r| r.get(0),
        )?;
        let sql=format!("SELECT r.session_id FROM quality_registry r WHERE {filter} ORDER BY (r.session_id=?5) DESC,r.first_seen DESC,r.session_id DESC LIMIT ?3 OFFSET ?4");
        let ids = c
            .prepare(&sql)?
            .query_map(
                params![
                    a,
                    month,
                    limit as i64,
                    offset as i64,
                    p["current_session_id"].as_str().unwrap_or("")
                ],
                |r| r.get::<_, String>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let rows = ids
            .iter()
            .map(|s| summary(c, &a, s))
            .collect::<Result<Vec<_>>>()?;
        return Ok(
            json!({"sessions":rows,"total":total,"next_offset":if offset+limit<total as usize{Some(offset+limit)}else{None},"rules":rules(c,&a)?,"rule_version":RULE_VERSION}),
        );
    }
    let s = id_arg(p, "session_id")?;
    let overview = summary(c, &a, &s)?;
    let g = overview["generation"].as_i64().unwrap();
    let reason = p["reason"].as_str().unwrap_or("");
    if ![
        "",
        "normal",
        "self_repeat",
        "crowd_repeat",
        "session_repeat",
        "activity_command",
        "activity_uncertain",
        "empty",
    ]
    .contains(&reason)
    {
        return Err(ArchiveError::Invalid("消息分类无效".into()));
    }
    let limit = page(p, "limit", 30, 100)?;
    let offset = page(p, "offset", 0, 1_000_000)?;
    let reasons=c.prepare("SELECT reason,COUNT(*) FROM quality_results WHERE anchor_id=? AND session_id=? AND generation=? GROUP BY reason")?.query_map(params![a,s,g],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?)))?.collect::<std::result::Result<std::collections::HashMap<_,_>,_>>()?;
    let total:i64=c.query_row("SELECT COUNT(*) FROM quality_results WHERE anchor_id=? AND session_id=? AND generation=? AND (?='' OR reason=?)",params![a,s,g,reason,reason],|r|r.get(0))?;
    let rows=c.prepare("SELECT m.id,m.sender_id,m.sender_name,m.content,m.message_time,m.received_at,q.reason FROM quality_results q JOIN messages m ON m.id=q.message_id WHERE q.anchor_id=? AND q.session_id=? AND q.generation=? AND (?='' OR q.reason=?) ORDER BY m.id DESC LIMIT ? OFFSET ?")?.query_map(params![a,s,g,reason,reason,limit as i64,offset as i64],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"sender_id":r.get::<_,String>(1)?,"sender_name":r.get::<_,String>(2)?,"content":r.get::<_,String>(3)?,"message_time":r.get::<_,String>(4)?,"received_at":r.get::<_,String>(5)?,"reason":r.get::<_,String>(6)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
    let commands=c.prepare("SELECT content FROM quality_manual WHERE anchor_id=? AND session_id=? ORDER BY content")?.query_map(params![a,s],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
    Ok(
        json!({"overview":overview,"reasons":reasons,"messages":rows,"commands":commands,"total":total,"next_offset":if offset+limit<total as usize{Some(offset+limit)}else{None}}),
    )
}
pub(crate) fn summary(c: &Connection, a: &str, s: &str) -> Result<Value> {
    let mut value=c.query_row("SELECT r.generation,r.anchor_name,r.web_rid,r.title,r.first_seen,r.last_seen,r.started_at,r.coverage,r.cleared,COALESCE(d.accumulated_seconds,0),COALESCE(j.state,''),COALESCE(t.analyzed,0),COALESCE(t.valid,0),COALESCE(t.unknown_sender,0) FROM quality_registry r LEFT JOIN session_monitor_durations d USING(anchor_id,session_id) LEFT JOIN quality_jobs j ON j.anchor_id=r.anchor_id AND j.session_id=r.session_id AND j.generation=r.generation LEFT JOIN quality_stats t ON t.anchor_id=r.anchor_id AND t.session_id=r.session_id AND t.generation=r.generation WHERE r.anchor_id=? AND r.session_id=?",params![a,s],|r|Ok(json!({"anchor_id":a,"session_id":s,"generation":r.get::<_,i64>(0)?,"anchor_name":r.get::<_,String>(1)?,"web_rid":r.get::<_,String>(2)?,"title":r.get::<_,String>(3)?,"first_seen":r.get::<_,String>(4)?,"last_seen":r.get::<_,String>(5)?,"started_at":r.get::<_,Option<String>>(6)?,"coverage":r.get::<_,String>(7)?,"records_cleared":r.get::<_,bool>(8)?,"monitor_seconds":r.get::<_,f64>(9)?,"job_state":r.get::<_,String>(10)?,"analyzed_chat":r.get::<_,i64>(11)?,"valid_messages":r.get::<_,i64>(12)?,"unknown_sender_messages":r.get::<_,i64>(13)?}))).optional()?.ok_or_else(||ArchiveError::Invalid("尚未登记此直播场次".into()))?;
    let g = value["generation"].as_i64().unwrap();
    let raw: i64 = c.query_row(
        "SELECT COUNT(*) FROM messages WHERE anchor_id=? AND session_id=? AND message_type='chat'",
        params![a, s],
        |r| r.get(0),
    )?;
    let (users,valid_users):(i64,i64)=c.query_row("SELECT COUNT(*),COALESCE(SUM(valid>0),0) FROM quality_users WHERE anchor_id=? AND session_id=? AND generation=?",params![a,s,g],|r|Ok((r.get(0)?,r.get(1)?)))?;
    let uncertain:i64=c.query_row("SELECT COALESCE(SUM(uncertain),0) FROM quality_stats WHERE anchor_id=? AND session_id=? AND generation=?",params![a,s,g],|r|r.get(0))?;
    let unprocessed = (raw - value["analyzed_chat"].as_i64().unwrap()).max(0);
    let pending = unprocessed + uncertain;
    value["chat_messages"] = json!(raw);
    value["speakers"] = json!(users);
    value["valid_speakers"] = json!(valid_users);
    value["pending"] = json!(pending);
    value["unprocessed"] = json!(unprocessed);
    value["uncertain"] = json!(uncertain);
    value["analysis_state"] = json!(if value["job_state"] == "paused" {
        "paused"
    } else if unprocessed > 0
        || value["job_state"] == "queued"
        || c.prepare(
            "SELECT 1 FROM quality_pending WHERE anchor_id=? AND session_id=? AND generation=?"
        )?
        .exists(params![a, s, g])?
    {
        "analyzing"
    } else if uncertain > 0 {
        "needs_review"
    } else {
        "ready"
    });
    value["valid_speaker_rate"] = if users > 0 && value["analysis_state"] == "ready" {
        json!(valid_users as f64 / users as f64)
    } else {
        Value::Null
    };
    value["rule_version"] = json!(RULE_VERSION);
    super::audience::summary(c, a, s, &mut value)?;
    Ok(value)
}
