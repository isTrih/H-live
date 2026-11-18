//! An explicit whole-session snapshot for replay and one-shot model analysis.
//! Unlike recent_messages this never paginates, clips text or drops event types.
use super::*;

impl ArchiveStore {
    pub fn session_timeline(&self, anchor_id: &str, session_id: &str) -> Result<Value> {
        identity(anchor_id)?;
        identity(session_id)?;
        let snapshot = self.snapshot()?;
        let c = &snapshot.connection;
        let summary = quality::summary(c, anchor_id, session_id)?;
        let mut messages = c.prepare("SELECT id,sender_id,sender_name,message_type,content,COALESCE(message_time_epoch,received_at_epoch),message_time,received_at,time_source,payload_json FROM messages WHERE anchor_id=? AND session_id=? ORDER BY COALESCE(message_time_epoch,received_at_epoch),id")?
            .query_map(params![anchor_id,session_id], |r| {
                let payload: String = r.get(9)?;
                Ok(json!({"id":r.get::<_,i64>(0)?.to_string(),"sender_id":r.get::<_,String>(1)?,"sender_name":r.get::<_,String>(2)?,"message_type":r.get::<_,String>(3)?,"content":r.get::<_,String>(4)?,"epoch":r.get::<_,f64>(5)?,"message_time":r.get::<_,String>(6)?,"received_at":r.get::<_,String>(7)?,"time_source":r.get::<_,String>(8)?,"payload":serde_json::from_str::<Value>(&payload).unwrap_or(json!({"invalid_payload":payload}))}))
            })?.collect::<std::result::Result<Vec<_>,_>>()?;
        let mut online = c.prepare("SELECT minute,last_at,mean_count,peak,samples FROM audience_minutes WHERE anchor_id=? AND session_id=? ORDER BY minute")?
            .query_map(params![anchor_id,session_id], |r| Ok(json!({"epoch":r.get::<_,i64>(0)? as f64*60.,"last_at":r.get::<_,f64>(1)?,"mean_count":r.get::<_,f64>(2)?,"peak":r.get::<_,i64>(3)?,"samples":r.get::<_,i64>(4)?})))?
            .collect::<std::result::Result<Vec<_>,_>>()?;
        let epoch = |key: &str| {
            summary[key]
                .as_str()
                .and_then(|s| parse_time(s).ok())
                .map(seconds)
        };
        // Audience bucket boundaries are rounded down; use the actual sample
        // time when establishing the session extent, not an invented earlier start.
        let first = messages
            .iter()
            .filter_map(|m| m["epoch"].as_f64())
            .chain(online.iter().filter_map(|m| m["last_at"].as_f64()))
            .fold(epoch("first_seen").unwrap_or(0.), f64::min);
        let start = epoch("started_at").filter(|s| *s <= first).unwrap_or(first);
        let end = messages
            .iter()
            .map(|m| m["epoch"].as_f64().unwrap())
            .chain(online.iter().map(|m| m["last_at"].as_f64().unwrap()))
            .fold(epoch("last_seen").unwrap_or(start), f64::max)
            .max(start);
        let duration = (end - start).max(0.);
        // At most ~600 area-chart points, while all underlying records survive.
        let bucket_seconds = (duration / 600.).ceil().max(1.) as u64;
        let mut counts = vec![0u64; (duration / bucket_seconds as f64).floor() as usize + 1];
        let mut chat_count = 0;
        let mut untimed_count = 0;
        let mut untimed_chat_count = 0;
        for message in &mut messages {
            let offset = (message["epoch"].as_f64().unwrap() - start).max(0.);
            let timed = message["time_source"] == "server";
            message["observed_offset_seconds"] = json!(offset);
            message["offset_seconds"] = if timed { json!(offset) } else { Value::Null };
            if !timed {
                untimed_count += 1;
            }
            if message["message_type"] == "chat" {
                if timed {
                    counts[(offset / bucket_seconds as f64).floor() as usize] += 1;
                } else {
                    untimed_chat_count += 1;
                }
                chat_count += 1;
            }
        }
        for row in &mut online {
            row["offset_seconds"] = json!((row["epoch"].as_f64().unwrap() - start).max(0.));
        }
        let buckets: Vec<_> = counts
            .iter()
            .enumerate()
            .map(|(i, count)| json!({"offset_seconds":i as u64*bucket_seconds,"count":count}))
            .collect();
        let (activities, unconfirmed_activity_count) =
            timeline_activities(c, anchor_id, session_id, &messages, start, end)?;
        let unpositioned_activity_count = activities
            .iter()
            .filter(|a| a["offset_seconds"].is_null())
            .count();
        Ok(
            json!({"anchor_id":anchor_id,"session_id":session_id,"start_epoch":start,"end_epoch":end,"duration_seconds":duration,"bucket_seconds":bucket_seconds,"buckets":buckets,"total_messages":messages.len(),"chat_count":chat_count,"timed_chat_count":chat_count-untimed_chat_count,"untimed_chat_count":untimed_chat_count,"untimed_message_count":untimed_count,"messages":messages,"activities":activities,"activity_count":activities.len(),"unpositioned_activity_count":unpositioned_activity_count,"unconfirmed_activity_count":unconfirmed_activity_count,"online":online,"summary":summary,"complete":true,"histogram_type":"server_timed_chat","source":"archive"}),
        )
    }
}

// Reuse the archive's canonical identity projection, including merged protocol
// aliases and reliable lottery draw seconds. The full message snapshot above is
// left intact for one-shot analysis, including unconfirmed notifications.
fn timeline_activities(
    c: &Connection,
    anchor_id: &str,
    session_id: &str,
    messages: &[Value],
    start: f64,
    end: f64,
) -> Result<(Vec<Value>, usize)> {
    let rounds = c.prepare("SELECT message_row_id,kind,ends_at,terminal FROM activity_rounds WHERE anchor_id=? AND session_id=?")?
        .query_map(params![anchor_id, session_id], |r| Ok((r.get::<_, i64>(0)?.to_string(), (r.get::<_, String>(1)?, r.get::<_, Option<i64>>(2)?, r.get::<_, bool>(3)?))))?
        .collect::<std::result::Result<std::collections::BTreeMap<_, _>, _>>()?;
    let mut activities = Vec::new();
    let mut unconfirmed = 0;
    for message in messages.iter().filter(|m| m["message_type"] == "activity") {
        let payload = &message["payload"];
        let Some((kind, ends_at, ended)) = message["id"].as_str().and_then(|id| rounds.get(id))
        else {
            if matches!(
                payload["activity_kind"].as_str(),
                Some("lottery" | "red_packet")
            ) && payload["activity_stage"] != "status"
            {
                unconfirmed += 1;
            }
            continue;
        };
        let parse = |field: &str| {
            message[field]
                .as_str()
                .and_then(|s| parse_time(s).ok())
                .map(seconds)
                .filter(|epoch| epoch.is_finite() && *epoch > 0.)
        };
        let server_epoch = (message["time_source"] == "server")
            .then(|| parse("message_time"))
            .flatten();
        let observed_epoch = parse("received_at");
        let epoch = server_epoch.or(observed_epoch);
        let offset = epoch
            .filter(|epoch| *epoch >= start && *epoch <= end)
            .map(|epoch| epoch - start);
        let details = payload
            .get("activity_details")
            .filter(|details| details.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        activities.push(json!({"id":message["id"],"activity_id":payload.get("activity_id").and_then(Value::as_str),"kind":kind,"offset_seconds":offset,"time_source":if server_epoch.is_some(){"server"}else if observed_epoch.is_some(){"observed"}else{"unknown"},"epoch":epoch,"observed_epoch":observed_epoch,"draw_at":if kind=="lottery"{*ends_at}else{None},"expires_at":if kind=="red_packet"{*ends_at}else{None},"details":details,"content":message["content"],"ended":ended}));
    }
    Ok((activities, unconfirmed))
}
