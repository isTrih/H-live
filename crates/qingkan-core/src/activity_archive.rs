//! Activity identities and lifecycle projections. Notification IDs are not rounds.
//! A lottery can also share a reliable draw second; red packets never share a
//! campaign/title/amount merely because those fields happen to be the same.
use super::*;

const DRAW: &str = "CASE WHEN json_valid(payload_json) THEN CASE WHEN json_extract(payload_json,'$.activity_kind')='lottery' AND json_type(payload_json,'$.activity_details.draw_at') IN ('integer','real') THEN CASE WHEN json_extract(payload_json,'$.activity_details.draw_at')>=1000000000000 AND json_extract(payload_json,'$.activity_details.draw_at')<253402300800000 THEN CAST(json_extract(payload_json,'$.activity_details.draw_at')/1000 AS INTEGER) WHEN json_extract(payload_json,'$.activity_details.draw_at')>=1 AND json_extract(payload_json,'$.activity_details.draw_at')<253402300800 THEN CAST(json_extract(payload_json,'$.activity_details.draw_at') AS INTEGER) END END END";
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS activity_aliases(anchor_id TEXT NOT NULL,session_id TEXT NOT NULL,kind TEXT NOT NULL,activity_id TEXT NOT NULL,message_row_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,PRIMARY KEY(anchor_id,session_id,kind,activity_id)) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS activity_alias_row ON activity_aliases(message_row_id);
CREATE TABLE IF NOT EXISTS activity_rounds(message_row_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,anchor_id TEXT NOT NULL,session_id TEXT NOT NULL,kind TEXT NOT NULL,ends_at INTEGER,terminal INTEGER NOT NULL,observed_open INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS activity_round_scope ON activity_rounds(anchor_id,session_id,kind);
CREATE INDEX IF NOT EXISTS messages_activity_scope ON messages(anchor_id,session_id,id) WHERE message_type='activity';";

fn kind(p: &Value) -> Option<&str> {
    p["activity_kind"]
        .as_str()
        .filter(|k| matches!(*k, "lottery" | "red_packet"))
}
fn activity_id(p: &Value) -> Option<&str> {
    p["activity_id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
}
fn epoch(v: &Value) -> Option<i64> {
    let n = v
        .as_i64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))?;
    let n = if n >= 1_000_000_000_000 { n / 1000 } else { n };
    (1..253_402_300_800).contains(&n).then_some(n)
}
fn observed(r: &MessageRecord) -> i64 {
    parse_time(if r.message_time.is_empty() {
        &r.received_at
    } else {
        &r.message_time
    })
    .map(|t| t.timestamp_millis())
    .unwrap_or(0)
}
fn terminal(p: &Value) -> bool {
    p["activity_ended"] == true
        || matches!(p["activity_stage"].as_str(), Some("ended" | "draw_result"))
        || p["activity_details"]["remaining_count"].as_str() == Some("0")
        || p["activity_details"]["remaining_count"].as_u64() == Some(0)
}
fn opened(p: &Value) -> bool {
    p["activity_open_seen"] == true
        || matches!(
            p["activity_stage"].as_str(),
            Some("notice" | "preview" | "sync")
        )
}
fn draw(c: &Connection, payload: &str) -> Result<Option<i64>> {
    Ok(c.query_row(
        &format!("SELECT ({DRAW}) FROM (SELECT ? AS payload_json)"),
        [payload],
        |r| r.get(0),
    )?)
}
fn bind(c: &Connection, r: &MessageRecord, p: &Value, row: i64) -> Result<()> {
    let Some(kind) = kind(p) else { return Ok(()) };
    if let Some(id) = activity_id(p) {
        c.execute("INSERT INTO activity_aliases VALUES(?,?,?,?,?) ON CONFLICT(anchor_id,session_id,kind,activity_id) DO UPDATE SET message_row_id=excluded.message_row_id",params![r.anchor_id,r.session_id,kind,id,row])?;
    }
    Ok(())
}
pub(super) fn register(c: &Connection, r: &MessageRecord, payload: &str, row: i64) -> Result<()> {
    if r.message_type != "activity" {
        return Ok(());
    }
    let p: Value = serde_json::from_str(payload)?;
    let Some(kind) = kind(&p) else { return Ok(()) };
    let draw = draw(c, payload)?;
    if activity_id(&p).is_none() && draw.is_none() {
        return Ok(());
    }
    bind(c, r, &p, row)?;
    let end = if kind == "lottery" {
        draw
    } else {
        epoch(&p["activity_details"]["expires_at"])
    };
    let open = opened(&p);
    c.execute("INSERT INTO activity_rounds VALUES(?,?,?,?,?,?,?) ON CONFLICT(message_row_id) DO UPDATE SET ends_at=excluded.ends_at,terminal=excluded.terminal,observed_open=excluded.observed_open",params![row,r.anchor_id,r.session_id,kind,end,terminal(&p),open])?;
    Ok(())
}
pub(super) fn initialize(c: &Connection) -> Result<()> {
    let indexed = c
        .prepare("SELECT 1 FROM sqlite_schema WHERE type='index' AND name='messages_lottery_draw'")?
        .exists([])?;
    let projected = c.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name IN ('activity_rounds','activity_aliases')",[],|r|r.get::<_,i64>(0))? == 2;
    if indexed && projected {
        return Ok(());
    }
    c.execute_batch(SCHEMA)?;
    if !indexed {
        c.execute_batch(&format!("CREATE INDEX messages_lottery_draw ON messages(anchor_id,session_id,({DRAW})) WHERE message_type='activity' AND ({DRAW}) IS NOT NULL"))?;
    }
    let mut cursor = 0;
    loop {
        let rows=c.prepare(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE message_type='activity' AND id>? ORDER BY id LIMIT 128"))?.query_map([cursor],stored_row)?.collect::<std::result::Result<Vec<_>,_>>()?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            cursor = row.id;
            if !merge(c, &row.record, &row.payload_json, Some(row.id))? {
                register(c, &row.record, &row.payload_json, row.id)?;
            }
        }
    }
    if !indexed {
        c.execute_batch("DROP INDEX messages_lottery_draw")?;
        c.execute_batch(&format!("CREATE UNIQUE INDEX messages_lottery_draw ON messages(anchor_id,session_id,({DRAW})) WHERE message_type='activity' AND ({DRAW}) IS NOT NULL"))?;
    }
    Ok(())
}
pub(super) fn merge_existing(c: &Connection, r: &MessageRecord, payload: &str) -> Result<bool> {
    merge(c, r, payload, None)
}
fn merge(c: &Connection, r: &MessageRecord, payload: &str, current: Option<i64>) -> Result<bool> {
    if r.message_type != "activity" {
        return Ok(false);
    }
    let incoming: Value = serde_json::from_str(payload)?;
    let Some(kind) = kind(&incoming) else {
        return Ok(false);
    };
    let mut ids = std::collections::BTreeSet::new();
    if let Some(id) = activity_id(&incoming) {
        if let Some(row)=c.query_row("SELECT message_row_id FROM activity_aliases WHERE anchor_id=? AND session_id=? AND kind=? AND activity_id=?",params![r.anchor_id,r.session_id,kind,id],|r|r.get::<_,i64>(0)).optional()?{ids.insert(row);}
    }
    if let Some(draw) = draw(c, payload)? {
        if let Some(row)=c.query_row(&format!("SELECT id FROM messages WHERE anchor_id=? AND session_id=? AND message_type='activity' AND ({DRAW}) IS NOT NULL AND ({DRAW})=? ORDER BY id LIMIT 1"),params![r.anchor_id,r.session_id,draw],|r|r.get::<_,i64>(0)).optional()?{ids.insert(row);}
    }
    if ids.is_empty() || current.is_some_and(|id| ids.len() == 1 && ids.contains(&id)) {
        return Ok(false);
    }
    if let Some(row) = current {
        ids.insert(row);
    }
    let keep = *ids.first().unwrap();
    let mut records = Vec::new();
    for id in &ids {
        records.push((
            *id,
            c.query_row(
                &format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id=?"),
                [id],
                stored_row,
            )?,
        ));
    }
    let retained = &records[0].1;
    if current.is_none()
        && records.len() == 1
        && retained.record.message_id == r.message_id
        && retained.payload_json == payload
    {
        return Ok(true);
    }
    let mut combined: Value = serde_json::from_str(&retained.payload_json)?;
    combined["activity_ended"] = json!(terminal(&combined));
    combined["activity_updated_at"] = json!(combined["activity_updated_at"]
        .as_i64()
        .unwrap_or_else(|| observed(&retained.record)));
    for (_, row) in records.iter().skip(1) {
        enrich(
            &mut combined,
            &serde_json::from_str(&row.payload_json)?,
            observed(&row.record),
        );
    }
    enrich(&mut combined, &incoming, observed(r));
    // Rebind all known protocol IDs before deleting redundant notifications.
    for (id, row) in &records {
        bind(
            c,
            &retained.record,
            &serde_json::from_str(&row.payload_json)?,
            keep,
        )?;
        if *id != keep {
            c.execute(
                "UPDATE activity_aliases SET message_row_id=? WHERE message_row_id=?",
                params![keep, id],
            )?;
            c.execute("DELETE FROM messages WHERE id=?", [id])?;
            c.execute("DELETE FROM activity_rounds WHERE message_row_id=?", [id])?;
        }
    }
    bind(c, r, &incoming, keep)?;
    let text = summary(&combined, &retained.record.content);
    combined["activity_summary_truncated"] = json!(text.chars().count() > 16000);
    let content: String = text.chars().take(16000).collect();
    if combined.get("content").is_some() {
        combined["content"] = json!(content);
    }
    let bytes = serde_json::to_string(&combined)?;
    if bytes != retained.payload_json || content != retained.record.content {
        c.execute(
            "UPDATE messages SET payload_json=?,content=? WHERE id=?",
            params![bytes, content, keep],
        )?;
        quality::refresh_activity(c, &r.anchor_id, &r.session_id, &bytes)?;
    }
    register(c, &retained.record, &bytes, keep)?;
    Ok(true)
}
fn merge_details(target: &mut Value, source: &Value) {
    match (target, source) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                if !v.is_null() && v.as_str() != Some("") {
                    merge_details(a.entry(k.clone()).or_insert(Value::Null), v);
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            for v in b {
                if a.len() < 100 && !a.contains(v) {
                    a.push(v.clone());
                }
            }
        }
        (a, b) if !b.is_null() => *a = b.clone(),
        _ => (),
    }
}
fn enrich(target: &mut Value, source: &Value, observed: i64) {
    let newer = observed >= target["activity_updated_at"].as_i64().unwrap_or(0);
    let ended = terminal(target) || terminal(source);
    let open = opened(target) || opened(source);
    let dynamic = ["remaining_count", "remaining_amount_raw", "status_code"];
    let saved: Vec<_> = dynamic
        .iter()
        .map(|k| target["activity_details"][k].clone())
        .collect();
    merge_details(&mut target["activity_details"], &source["activity_details"]);
    if !newer {
        for (k, v) in dynamic.iter().zip(saved) {
            if !v.is_null() {
                target["activity_details"][k] = v;
            }
        }
    }
    if newer {
        target["activity_updated_at"] = json!(observed);
        if source["activity_stage"].is_string() {
            target["activity_stage"] = source["activity_stage"].clone();
        }
    }
    target["activity_ended"] = json!(ended);
    target["activity_open_seen"] = json!(open);
}
fn summary(p: &Value, fallback: &str) -> String {
    let d = &p["activity_details"];
    let mut parts = vec![format!(
        "{}活动{}",
        if p["activity_kind"] == "lottery" {
            "福袋"
        } else {
            "红包"
        },
        if terminal(p) { " · 已结束" } else { "" }
    )];
    for (key, label) in [
        ("title", "标题"),
        ("prize_name", "奖品"),
        ("prize_description", "说明"),
        ("prize_count", "奖品数量"),
        ("winner_count", "中奖名额"),
        ("diamond_count", "钻石数量"),
        ("total_count", "红包份数"),
        ("remaining_count", "剩余份数"),
    ] {
        if let Some(v) = d
            .get(key)
            .filter(|v| !v.is_null() && v.as_str() != Some(""))
        {
            parts.push(format!(
                "{label}：{}",
                v.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string())
            ));
        }
    }
    if let Some(conditions) = d["conditions"].as_array() {
        for condition in conditions.iter().take(100) {
            if let Some(text) = condition["content"].as_str().filter(|s| !s.is_empty()) {
                parts.push(format!("参与条件：{text}"));
            }
        }
    }
    for (key, label) in [("draw_at", "开奖时间"), ("expires_at", "结束时间")] {
        if let Some(date) = epoch(&d[key]).and_then(|t| chrono::DateTime::from_timestamp(t, 0)) {
            parts.push(format!(
                "{label}：{}",
                date.with_timezone(&shanghai())
                    .format("%Y-%m-%d %H:%M:%S%:z")
            ));
        }
    }
    if parts.len() == 1 && !terminal(p) {
        fallback.into()
    } else {
        parts.join("；")
    }
}
impl ArchiveStore {
    pub fn activity_summary(&self, anchor: &str, session: &str, now: i64) -> Result<Value> {
        identity(anchor)?;
        identity(session)?;
        let snapshot = self.snapshot()?;
        let mut value = json!({"red_packet":{"total":0,"active_until":null,"uncertain":false},"lottery":{"total":0,"active_until":null,"uncertain":false},"checked_at":now});
        let rows=snapshot.connection.prepare("SELECT kind,COUNT(*),MAX(CASE WHEN terminal=0 AND observed_open=1 AND ends_at>? THEN ends_at END),MAX(terminal=0 AND (ends_at IS NULL OR (observed_open=0 AND ends_at>?))) FROM activity_rounds WHERE anchor_id=? AND session_id=? GROUP BY kind")?.query_map(params![now,now,anchor,session],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,Option<i64>>(2)?,r.get::<_,bool>(3)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        for (kind, total, until, uncertain) in rows {
            value[&kind] = json!({"total":total,"active_until":until,"uncertain":uncertain});
        }
        // Unknown identities cannot safely be collapsed into invented rounds.
        // Content-free temporary status packets are not evidence of an activity.
        let unknown = snapshot.connection.prepare("SELECT DISTINCT json_extract(m.payload_json,'$.activity_kind') FROM messages m WHERE m.anchor_id=? AND m.session_id=? AND m.message_type='activity' AND json_valid(m.payload_json) AND json_extract(m.payload_json,'$.activity_kind') IN ('red_packet','lottery') AND COALESCE(json_extract(m.payload_json,'$.activity_stage'),'')!='status' AND NOT EXISTS(SELECT 1 FROM activity_rounds a WHERE a.message_row_id=m.id)")?.query_map(params![anchor,session], |r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        for kind in unknown
            .into_iter()
            .filter(|k| matches!(k.as_str(), "red_packet" | "lottery"))
        {
            value[&kind]["unidentified"] = json!(true);
        }
        Ok(value)
    }
}
