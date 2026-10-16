//! One lottery row per real anchor, session and known draw second. The same
//! expression backs lookup and uniqueness, including seconds/milliseconds.
use super::*;

const DRAW: &str = "CASE WHEN json_valid(payload_json) THEN CASE WHEN json_extract(payload_json,'$.activity_kind')='lottery' AND json_type(payload_json,'$.activity_details.draw_at') IN ('integer','real') THEN CASE WHEN json_extract(payload_json,'$.activity_details.draw_at')>=1000000000000 AND json_extract(payload_json,'$.activity_details.draw_at')<253402300800000 THEN CAST(json_extract(payload_json,'$.activity_details.draw_at')/1000 AS INTEGER) WHEN json_extract(payload_json,'$.activity_details.draw_at')>=1 AND json_extract(payload_json,'$.activity_details.draw_at')<253402300800 THEN CAST(json_extract(payload_json,'$.activity_details.draw_at') AS INTEGER) END END END";

fn index(unique: bool) -> String {
    format!("CREATE {} INDEX messages_lottery_draw ON messages(anchor_id,session_id,({DRAW})) WHERE message_type='activity' AND ({DRAW}) IS NOT NULL", if unique { "UNIQUE" } else { "" })
}

pub(super) fn initialize(c: &Connection) -> Result<()> {
    if c.prepare("SELECT 1 FROM sqlite_schema WHERE type='index' AND name='messages_lottery_draw'")?
        .exists([])?
    {
        return Ok(());
    }
    // Build a temporary nonunique form first. Consolidate only activity rows in
    // bounded pages; normal chat, timestamps and original server IDs stay intact.
    c.execute_batch(&index(false))?;
    let mut cursor = 0;
    loop {
        let rows = c.prepare(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE message_type='activity' AND id>? ORDER BY id LIMIT 128"))?
            .query_map([cursor], stored_row)?.collect::<std::result::Result<Vec<_>,_>>()?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            cursor = row.id;
            if merge(c, &row.record, &row.payload_json, Some(row.id))? {
                // Earliest row is canonical; every field we retain is merged
                // before deleting a later notification in this same transaction.
                c.execute("DELETE FROM messages WHERE id=?", [row.id])?;
            }
        }
    }
    c.execute_batch("DROP INDEX messages_lottery_draw")?;
    c.execute_batch(&index(true))?;
    Ok(())
}

pub(super) fn merge_existing(c: &Connection, r: &MessageRecord, payload: &str) -> Result<bool> {
    merge(c, r, payload, None)
}

fn merge(c: &Connection, r: &MessageRecord, payload: &str, current: Option<i64>) -> Result<bool> {
    if r.message_type != "activity" {
        return Ok(false);
    }
    let draw: Option<i64> = c.query_row(
        &format!("SELECT ({DRAW}) FROM (SELECT ? AS payload_json)"),
        [payload],
        |row| row.get(0),
    )?;
    let Some(draw) = draw else {
        return Ok(false);
    };
    let existing: Option<(i64,String,String)> = c.query_row(
        &format!("SELECT id,payload_json,content FROM messages WHERE anchor_id=? AND session_id=? AND message_type='activity' AND ({DRAW}) IS NOT NULL AND ({DRAW})=? ORDER BY id LIMIT 1"),
        params![r.anchor_id,r.session_id,draw], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    let Some((id, old, old_content)) = existing else {
        return Ok(false);
    };
    // While initializing the index the current row is itself a candidate.
    if current == Some(id) {
        return Ok(false);
    }
    let mut merged: Value = serde_json::from_str(&old)?;
    let incoming: Value = serde_json::from_str(payload)?;
    // Keep the first envelope's identity/time consistent with its database row.
    // Only activity details are enriched; no unbounded list of raw notifications.
    merge_details(
        &mut merged["activity_details"],
        &incoming["activity_details"],
    );
    merged["activity_details"]["draw_at"] = json!(draw);
    if merged != serde_json::from_str::<Value>(&old)? {
        let full_content = summary(&merged, &old_content);
        let truncated = full_content.chars().count() > 16000;
        let content: String = full_content.chars().take(16000).collect();
        merged["activity_summary_truncated"] = json!(truncated);
        if merged.get("content").is_some() {
            merged["content"] = json!(content);
        }
        let combined = serde_json::to_string(&merged)?;
        c.execute(
            "UPDATE messages SET payload_json=?,content=? WHERE id=?",
            params![combined, content, id],
        )?;
        quality::refresh_activity(c, &r.anchor_id, &r.session_id, &combined)?;
    }
    Ok(true)
}

fn merge_details(target: &mut Value, source: &Value) {
    match (target, source) {
        (Value::Object(a), Value::Object(b)) => {
            for (key, value) in b {
                if !value.is_null() && value.as_str() != Some("") {
                    merge_details(a.entry(key.clone()).or_insert(Value::Null), value);
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            for value in b {
                if a.len() < 100 && !a.contains(value) {
                    a.push(value.clone());
                }
            }
        }
        (a, b) if !b.is_null() => *a = b.clone(),
        _ => (),
    }
}

fn summary(p: &Value, fallback: &str) -> String {
    let d = &p["activity_details"];
    let mut parts = vec!["福袋活动通知".to_string()];
    for (key, label) in [
        ("prize_name", "奖品"),
        ("prize_description", "说明"),
        ("prize_count", "奖品数量"),
        ("winner_count", "中奖名额"),
    ] {
        if let Some(v) = d.get(key).filter(|v| !v.is_null()) {
            let text = v
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v.to_string());
            if !text.is_empty() {
                parts.push(format!("{label}：{text}"));
            }
        }
    }
    if let Some(conditions) = d["conditions"].as_array() {
        for condition in conditions.iter().take(100) {
            if let Some(text) = condition["content"].as_str().filter(|s| !s.is_empty()) {
                parts.push(format!("参与条件：{text}"));
            }
        }
    }
    if let Some(date) = d["draw_at"]
        .as_i64()
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
    {
        parts.push(format!(
            "开奖时间：{}",
            date.with_timezone(&shanghai())
                .format("%Y-%m-%d %H:%M:%S%:z")
        ));
    }
    if parts.len() == 1 {
        fallback.into()
    } else {
        parts.join("；")
    }
}
