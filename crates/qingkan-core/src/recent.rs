//! Bounded, filtered history for the live message view. The archive remains
//! authoritative when high-volume entry/like events evict the IPC ring.
use super::*;

pub(super) const INDEXES:&str="CREATE INDEX IF NOT EXISTS messages_type_recent ON messages(message_type,id); CREATE INDEX IF NOT EXISTS messages_room_recent ON messages(web_rid,id); CREATE INDEX IF NOT EXISTS messages_room_type_recent ON messages(web_rid,message_type,id);";

fn clip(value: String, bytes: usize) -> String {
    if value.len() <= bytes {
        return value;
    }
    let mut end = bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}
impl ArchiveStore {
    pub fn recent_messages(&self, p: Value) -> Result<Value> {
        let kind = match p.get("type") {
            None => "all",
            Some(v) => v
                .as_str()
                .ok_or_else(|| ArchiveError::Invalid("消息类型无效".into()))?,
        };
        if kind != "all" && !CONTENT_TYPES.contains(&kind) {
            return Err(ArchiveError::Invalid("消息类型无效".into()));
        }
        let rid = match p.get("web_rid") {
            None => "",
            Some(v) => v
                .as_str()
                .ok_or_else(|| ArchiveError::Invalid("直播间入口无效".into()))?,
        };
        if !rid.is_empty() {
            identity(rid)?;
        }
        let limit = match p.get("limit") {
            None => 150,
            Some(v) => v
                .as_u64()
                .filter(|n| (1..=150).contains(n))
                .ok_or_else(|| ArchiveError::Invalid("读取条数应为 1 至 150".into()))?,
        };
        let before = if let Some(before) = p.get("before_id") {
            Some(
                before
                    .as_str()
                    .filter(|s| {
                        !s.is_empty() && s.len() <= 19 && s.bytes().all(|c| c.is_ascii_digit())
                    })
                    .and_then(|s| s.parse::<i64>().ok())
                    .ok_or_else(|| ArchiveError::Invalid("消息游标无效".into()))?,
            )
        } else {
            None
        };
        // Merge bounded index scans. Entries and retired gifts remain in the
        // historical archive but cannot crowd supported realtime categories.
        let kinds: Vec<&str> = if kind == "all" {
            CONTENT_TYPES
                .iter()
                .copied()
                .filter(|s| *s != "member" && *s != "gift")
                .collect()
        } else {
            vec![kind]
        };
        let mut values: Vec<rusqlite::types::Value> = Vec::new();
        let mut branches = Vec::new();
        for kind in kinds {
            let mut clauses = vec!["message_type=?"];
            values.push(kind.to_owned().into());
            if !rid.is_empty() {
                clauses.push("web_rid=?");
                values.push(rid.to_owned().into());
            }
            if let Some(before) = before {
                clauses.push("id<=?");
                values.push(before.into());
            }
            values.push((limit as i64).into());
            branches.push(format!(
                "SELECT id FROM (SELECT id FROM messages WHERE {} ORDER BY id DESC LIMIT ?)",
                clauses.join(" AND ")
            ));
        }
        let filter = format!("id IN ({})", branches.join(" UNION ALL "));
        values.push((limit as i64).into());
        let snapshot = self.snapshot()?;
        let mut query=snapshot.connection.prepare(&format!("SELECT id,anchor_id,anchor_name,web_rid,session_id,sender_id,sender_name,message_type,substr(content,1,4097),length(content),message_time,received_at,CASE WHEN json_valid(payload_json) THEN CASE WHEN json_type(payload_json,'$.activity_kind')='text' THEN json_extract(payload_json,'$.activity_kind') END ELSE NULL END FROM messages WHERE {filter} ORDER BY id DESC LIMIT ?"))?;
        let mut rows = query.query(params_from_iter(values))?;
        let mut messages = Vec::new();
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            let content = clip(row.get(8)?, 4096);
            let original_chars: i64 = row.get(9)?;
            let activity: Option<String> = row.get(12)?;
            let activity = activity
                .filter(|s| s == "lottery" || s == "red_packet")
                .unwrap_or_default();
            let record = json!({"archive_id":row.get::<_,i64>(0)?.to_string(),"anchor_id":row.get::<_,String>(1)?,"anchor_name":clip(row.get(2)?,256),"rid":row.get::<_,String>(3)?,"session_id":row.get::<_,String>(4)?,"sender_id":row.get::<_,String>(5)?,"sender_name":clip(row.get(6)?,256),"message_type":row.get::<_,String>(7)?,"content_truncated":original_chars>content.chars().count() as i64,"content":content,"message_time":row.get::<_,String>(10)?,"received_at":row.get::<_,String>(11)?,"payload":{"activity_kind":activity}});
            bytes += record.to_string().len();
            if bytes > 512 * 1024 {
                break;
            }
            messages.push(record);
        }
        let watermark: i64 =
            snapshot
                .connection
                .query_row("SELECT COALESCE(MAX(id),0) FROM messages", [], |r| r.get(0))?;
        Ok(
            json!({"high_watermark":watermark.to_string(),"messages":messages,"limit":limit,"source":"archive","order":"received_desc"}),
        )
    }
}
