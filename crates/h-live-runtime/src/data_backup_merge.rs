//! Selection and additive restore. All mutations happen on disposable database snapshots.
#[cfg(test)]
#[path = "data_backup_merge_tests.rs"]
mod tests;
use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn db_error(_: rusqlite::Error) -> String {
    error("备份增量合并失败，当前数据未修改")
}
fn writable(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path).map_err(db_error)?;
    conn.busy_timeout(Duration::from_secs(10))
        .map_err(db_error)?;
    conn.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA temp_store=FILE;").map_err(db_error)?;
    Ok(conn)
}
fn optional_json(directory: &Path, name: &str, default: Value) -> Result<Value> {
    if directory.join(name).exists() {
        read_json(&directory.join(name), MAX_JSON)
    } else {
        Ok(default)
    }
}
fn roster(directory: &Path, config: &Value) -> Result<BTreeMap<String, Value>> {
    let tags = crate::anchor_tags::canonical(&config["anchor_tags"])?;
    let conn = readonly(&directory.join("messages.sqlite3"))?;
    let mut rows = BTreeMap::new();
    let mut statement = conn.prepare("WITH ids AS (SELECT anchor_id FROM anchors UNION SELECT anchor_id FROM quality_registry), m AS (SELECT anchor_id,COUNT(*) n FROM messages GROUP BY anchor_id), s AS (SELECT anchor_id,COUNT(*) n FROM(SELECT anchor_id,session_id FROM quality_registry UNION SELECT anchor_id,session_id FROM sessions) GROUP BY anchor_id) SELECT i.anchor_id,COALESCE(a.anchor_name,(SELECT anchor_name FROM quality_registry q WHERE q.anchor_id=i.anchor_id LIMIT 1),''),COALESCE(a.web_rid,(SELECT web_rid FROM quality_registry q WHERE q.anchor_id=i.anchor_id LIMIT 1),''),COALESCE(m.n,0),COALESCE(s.n,0) FROM ids i LEFT JOIN anchors a USING(anchor_id) LEFT JOIN m USING(anchor_id) LEFT JOIN s USING(anchor_id)").map_err(db_error)?;
    let mapped = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })
        .map_err(db_error)?;
    for row in mapped {
        let (id, name, rid, messages, sessions) = row.map_err(db_error)?;
        rows.insert(id.clone(),json!({"anchor_id":id,"web_rid":rid,"name":name,"messages":messages,"sessions":sessions}));
    }
    let marks = optional_json(
        directory,
        "session-marks.json",
        json!({"format":1,"marks":[]}),
    )?;
    validate_marks(&marks)?;
    for mark in marks["marks"].as_array().unwrap() {
        let id = mark["anchor_id"].as_str().unwrap().to_owned();
        rows.entry(id.clone()).or_insert_with(||json!({"anchor_id":id,"web_rid":mark["room_id"],"name":mark["anchor_name"],"messages":0,"sessions":0}));
    }
    if directory.join("clips.sqlite3").exists() {
        let clips = readonly(&directory.join("clips.sqlite3"))?;
        let mut statement=clips.prepare("SELECT json_extract(payload,'$.anchor_id'),MAX(json_extract(payload,'$.anchor_name')) FROM clip_jobs WHERE json_valid(payload) AND json_extract(payload,'$.anchor_id')<>'' GROUP BY json_extract(payload,'$.anchor_id')").map_err(db_error)?;
        let jobs = statement
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(db_error)?;
        for row in jobs {
            let (id, name) = row.map_err(db_error)?;
            if storage::id(&id) {
                rows.entry(id.clone()).or_insert_with(
                    || json!({"anchor_id":id,"web_rid":"","name":name,"messages":0,"sessions":0}),
                );
            }
        }
    }
    if let Some(anchors) = config["anchors"].as_object() {
        for (rid, name) in anchors {
            let stable = config["anchor_metadata"][rid]["anchor_id"]
                .as_str()
                .filter(|id| storage::id(id));
            let known = stable.map(str::to_owned).or_else(|| {
                let matches: Vec<_> = rows
                    .iter()
                    .filter(|(_, r)| r["web_rid"].as_str() == Some(rid))
                    .map(|(id, _)| id.clone())
                    .collect();
                (matches.len() == 1).then(|| matches[0].clone())
            });
            let pending = format!("room:{rid}");
            // A remembered identity is not a network confirmation of a new
            // monitored room. Pending tags remain a separately selectable scope.
            let id = if tags.get(&pending).is_some() {
                pending
            } else {
                known.unwrap_or(pending)
            };
            rows.entry(id.clone()).or_insert_with(
                || json!({"anchor_id":id,"web_rid":rid,"name":name,"messages":0,"sessions":0}),
            );
        }
    }
    for (id, row) in &mut rows {
        // Historical room aliases can belong to another broadcaster now. Only
        // the stable identity (or an explicitly unresolved room row) owns tags.
        row["tags"] = tags.get(id).cloned().unwrap_or_else(|| json!([]));
    }
    Ok(rows)
}
pub(super) fn anchors(directory: &Path) -> Result<Value> {
    safe_directory(directory)?;
    let config = optional_json(directory, "config.json", json!({}))?;
    Ok(json!({"anchors":roster(directory,&config)?.into_values().collect::<Vec<_>>()}))
}

pub(super) fn filter(directory: &Path, ids: Vec<String>) -> Result<()> {
    if ids.is_empty() || ids.len() > 10_000 {
        return Err(error("请至少选择一位主播"));
    }
    let mut config = read_json(&directory.join("config.json"), MAX_JSON)?;
    let pending_tags = crate::anchor_tags::canonical(&config["anchor_tags"])?;
    let roster = roster(directory, &config)?;
    let mut selected = BTreeSet::new();
    let mut rooms = BTreeSet::new();
    let mut explicit_rooms = BTreeSet::new();
    for id in ids {
        let row = roster.get(&id).ok_or("选中的主播不存在，请刷新后重试")?;
        if let Some(rid) = id.strip_prefix("room:") {
            rooms.insert(rid.to_owned());
            explicit_rooms.insert(rid.to_owned());
        } else {
            selected.insert(id.clone());
        }
        if let Some(rid) = row["web_rid"].as_str().filter(|id| storage::id(id)) {
            rooms.insert(rid.to_owned());
        }
    }
    for (rid, row) in config["anchor_metadata"]
        .as_object()
        .ok_or("备份主播身份无效")?
    {
        if row["anchor_id"]
            .as_str()
            .is_some_and(|a| selected.contains(a))
        {
            rooms.insert(rid.clone());
        }
    }
    rooms.retain(|rid| {
        if pending_tags.get(format!("room:{rid}")).is_some() {
            return explicit_rooms.contains(rid);
        }
        match config["anchor_metadata"][rid]["anchor_id"]
            .as_str()
            .filter(|a| !a.is_empty())
        {
            Some(a) => selected.contains(a),
            None if explicit_rooms.contains(rid) => true,
            None => {
                let owners: Vec<_> = roster
                    .iter()
                    .filter(|(id, row)| {
                        !id.starts_with("room:") && row["web_rid"].as_str() == Some(rid)
                    })
                    .map(|(id, _)| id)
                    .collect();
                owners.len() == 1 && selected.contains(owners[0])
            }
        }
    });
    // A room ID is never interpreted as another person's numeric stable ID.
    let mut conn = writable(&directory.join("messages.sqlite3"))?;
    conn.execute_batch(
        "PRAGMA secure_delete=ON; CREATE TEMP TABLE backup_scope(anchor_id TEXT PRIMARY KEY);",
    )
    .map_err(db_error)?;
    for id in &selected {
        conn.execute("INSERT INTO backup_scope VALUES(?1)", [id])
            .map_err(db_error)?;
    }
    let session_scope=conn.prepare("SELECT session_id FROM(SELECT anchor_id,session_id FROM quality_registry UNION SELECT anchor_id,session_id FROM sessions) GROUP BY session_id HAVING COUNT(DISTINCT anchor_id)=1 AND MAX(anchor_id) IN(SELECT anchor_id FROM backup_scope)").map_err(db_error)?.query_map([],|r|r.get::<_,String>(0)).map_err(db_error)?.collect::<rusqlite::Result<BTreeSet<_>>>().map_err(db_error)?;
    let tx = conn.transaction().map_err(db_error)?;
    tx.execute(
        "DELETE FROM messages WHERE anchor_id NOT IN(SELECT anchor_id FROM backup_scope)",
        [],
    )
    .map_err(db_error)?;
    for table in &expected_shape(true)?.tables {
        if table.name != "messages" && table.columns.iter().any(|c| c.0 == "anchor_id") {
            tx.execute(
                &format!(
                    "DELETE FROM {} WHERE anchor_id NOT IN(SELECT anchor_id FROM backup_scope)",
                    table.name
                ),
                [],
            )
            .map_err(db_error)?;
        }
    }
    tx.execute_batch("DELETE FROM quality_results WHERE message_id NOT IN(SELECT id FROM messages); DELETE FROM activity_rounds WHERE message_row_id NOT IN(SELECT id FROM messages); DELETE FROM activity_aliases WHERE message_row_id NOT IN(SELECT id FROM messages);").map_err(db_error)?;
    tx.commit().map_err(db_error)?;
    clear_statistics(&conn)?;
    conn.execute_batch("DROP TABLE backup_scope; VACUUM;")
        .map_err(db_error)?;
    drop(conn);
    config["anchors"]
        .as_object_mut()
        .ok_or("备份主播列表无效")?
        .retain(|rid, _| rooms.contains(rid));
    config["anchor_metadata"]
        .as_object_mut()
        .ok_or("备份主播身份无效")?
        .retain(|rid, row| {
            rooms.contains(rid)
                && row["anchor_id"]
                    .as_str()
                    .is_none_or(|id| id.is_empty() || selected.contains(id))
        });
    let mut tags = crate::anchor_tags::canonical(&config["anchor_tags"])?;
    tags.as_object_mut().unwrap().retain(|id, _| {
        id.strip_prefix("room:")
            .map_or_else(|| selected.contains(id), |rid| explicit_rooms.contains(rid))
    });
    config["anchor_tags"] = tags;
    config["pc_unsupported_rids"]
        .as_array_mut()
        .unwrap()
        .retain(|v| v.as_str().is_some_and(|rid| rooms.contains(rid)));
    write_json(&directory.join("config.json"), &config)?;
    let mut marks = read_json(&directory.join("session-marks.json"), MAX_JSON)?;
    marks["marks"]
        .as_array_mut()
        .ok_or("打点备份无效")?
        .retain(|v| {
            v["anchor_id"]
                .as_str()
                .is_some_and(|a| selected.contains(a))
        });
    write_json(&directory.join("session-marks.json"), &marks)?;
    let mut habits = read_json(&directory.join("polling-habits.json"), MAX_JSON)?;
    habits["anchors"]
        .as_object_mut()
        .ok_or("开播偏好备份无效")?
        .retain(|a, _| selected.contains(a));
    habits["aliases"]
        .as_object_mut()
        .ok_or("开播偏好备份无效")?
        .retain(|rid, a| rooms.contains(rid) && a.as_str().is_some_and(|a| selected.contains(a)));
    write_json(&directory.join("polling-habits.json"), &habits)?;
    let mut preferences = read_json(&directory.join("preferences.json"), MAX_JSON)?;
    preferences
        .as_object_mut()
        .ok_or("界面偏好备份无效")?
        .retain(|key, _| {
            key.strip_prefix("livescope.replay-offset.")
                .is_none_or(|key| {
                    key.split_once(':')
                        .is_some_and(|(a, _)| selected.contains(a))
                })
        });
    write_json(&directory.join("preferences.json"), &preferences)?;
    let mut clips = writable(&directory.join("clips.sqlite3"))?;
    clips
        .execute_batch("PRAGMA secure_delete=ON;")
        .map_err(db_error)?;
    let tx = clips.transaction().map_err(db_error)?;
    {
        let mut statement = tx
            .prepare("SELECT id,payload FROM clip_jobs")
            .map_err(db_error)?;
        let mut rows = statement.query([]).map_err(db_error)?;
        while let Some(row) = rows.next().map_err(db_error)? {
            let id: String = row.get(0).map_err(db_error)?;
            let payload: String = row.get(1).map_err(db_error)?;
            let value: Value =
                serde_json::from_str(&payload).map_err(|_| error("下载备份记录无效"))?;
            let keep = value["anchor_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map_or_else(
                    || {
                        value["session_id"]
                            .as_str()
                            .is_some_and(|s| session_scope.contains(s))
                    },
                    |a| selected.contains(a),
                );
            if !keep {
                tx.execute("DELETE FROM clip_jobs WHERE id=?1", [id])
                    .map_err(db_error)?;
            }
        }
    }
    tx.commit().map_err(db_error)?;
    clear_statistics(&clips)?;
    clips.execute_batch("VACUUM;").map_err(db_error)?;
    Ok(())
}

fn clear_statistics(conn: &Connection) -> Result<()> {
    // STAT4 samples can contain indexed user text even after live rows are removed.
    for name in ["sqlite_stat1", "sqlite_stat4"] {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
                [name],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        if exists {
            conn.execute(&format!("DELETE FROM {name}"), [])
                .map_err(db_error)?;
        }
    }
    Ok(())
}

fn fill_missing(local: &mut Value, imported: &Value) {
    if let (Some(local), Some(imported)) = (local.as_object_mut(), imported.as_object()) {
        for (key, value) in imported {
            match local.get_mut(key) {
                Some(current) if current.is_object() && value.is_object() => {
                    fill_missing(current, value)
                }
                Some(current) if current.is_null() || current.as_str() == Some("") => {
                    *current = value.clone()
                }
                None => {
                    local.insert(key.clone(), value.clone());
                }
                _ => {}
            }
        }
    }
}
pub(super) fn config(local: &mut Value, imported: &Value) -> Result<()> {
    for key in ["anchors", "anchor_metadata"] {
        if !local[key].is_object() {
            local[key] = json!({});
        }
    }
    let mut compatible = imported.clone();
    for (rid, metadata) in imported["anchor_metadata"]
        .as_object()
        .ok_or("备份主播身份无效")?
    {
        let old = local["anchor_metadata"][rid]["anchor_id"]
            .as_str()
            .unwrap_or("");
        let new = metadata["anchor_id"].as_str().unwrap_or("");
        if !old.is_empty() && !new.is_empty() && old != new {
            compatible["anchors"].as_object_mut().unwrap().remove(rid);
            compatible["anchor_metadata"]
                .as_object_mut()
                .unwrap()
                .remove(rid);
        }
    }
    fill_missing(&mut local["anchors"], &compatible["anchors"]);
    fill_missing(
        &mut local["anchor_metadata"],
        &compatible["anchor_metadata"],
    );
    let mut tags = crate::anchor_tags::canonical(&local["anchor_tags"])?;
    let incoming_tags = crate::anchor_tags::canonical(&imported["anchor_tags"])?;
    for (id, incoming) in incoming_tags.as_object().unwrap() {
        if let Some(rid) = id.strip_prefix("room:") {
            let current = local["anchor_metadata"][rid]["anchor_id"]
                .as_str()
                .unwrap_or("");
            let incoming = imported["anchor_metadata"][rid]["anchor_id"]
                .as_str()
                .unwrap_or("");
            // An unresolved imported room must not relabel a known local
            // person; a reused room with two known owners is also incompatible.
            if !current.is_empty() && current != incoming {
                continue;
            }
        }
        let mut merged = match tags.get(id) {
            Some(existing) => crate::anchor_tags::normalize(existing)?,
            None => Vec::new(),
        };
        for tag in crate::anchor_tags::normalize(incoming)? {
            if merged.len() == 20 {
                break;
            }
            if !merged.contains(&tag) {
                merged.push(tag);
            }
        }
        tags[id] = json!(merged);
    }
    crate::anchor_tags::validate(&tags)?;
    local["anchor_tags"] = tags;
    for key in CONFIG_KEYS {
        if matches!(
            key,
            "anchors" | "anchor_metadata" | "anchor_tags" | "download_naming"
        ) {
            continue;
        }
        if key == "highlight_provider"
            && local["highlight_api_key_encrypted"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        {
            continue;
        }
        if local.get(key).is_none() {
            if let Some(value) = imported.get(key) {
                local[key] = value.clone();
            }
        }
    }
    Ok(())
}

pub(super) fn stage(
    local: &Path,
    imported: &Path,
    staging: &Path,
    local_config: &Value,
) -> Result<Value> {
    let mut added = json!({"messages":0,"anchors":0,"sessions":0,"downloads":0,"marks":0});
    for (name, messages) in [("messages.sqlite3", true), ("clips.sqlite3", false)] {
        let source = local.join(name);
        if source.exists() {
            validate_db(&source, messages)?;
            snapshot(&source, &staging.join(name))?;
            if messages {
                merge_messages(&staging.join(name), &imported.join(name), &mut added)?;
            } else {
                merge_clips(&staging.join(name), &imported.join(name), local, &mut added)?;
            }
        } else {
            copy_atomic(&imported.join(name), &staging.join(name))?;
            if messages {
                let count = validate_db(&staging.join(name), true)?;
                for key in ["messages", "anchors", "sessions"] {
                    added[key] = count[key].clone();
                }
            } else {
                added["downloads"] = validate_db(&staging.join(name), false)?["downloads"].clone();
                rebase_clips(&staging.join(name), local)?;
            }
        }
    }
    let imported_config = read_json(&imported.join("config.json"), MAX_JSON)?;
    write_json(
        &staging.join("config.json"),
        &merge_config(&imported_config, local_config, local)?,
    )?;
    let mut marks = optional_json(local, "session-marks.json", json!({"format":1,"marks":[]}))?;
    validate_marks(&marks)?;
    let incoming = read_json(&imported.join("session-marks.json"), MAX_JSON)?;
    let old = marks["marks"].as_array().unwrap();
    let mut ids: BTreeSet<String> = old
        .iter()
        .filter_map(|m| m["id"].as_str().map(str::to_owned))
        .collect();
    let mut identities: BTreeSet<String> = old.iter().map(mark_identity).collect();
    let old_len = old.len();
    for mark in incoming["marks"].as_array().ok_or("打点备份无效")? {
        if !ids.contains(mark["id"].as_str().unwrap_or(""))
            && identities.insert(mark_identity(mark))
        {
            ids.insert(mark["id"].as_str().unwrap_or("").to_owned());
            marks["marks"].as_array_mut().unwrap().push(mark.clone());
        }
    }
    validate_marks(&marks)?;
    added["marks"] = json!(marks["marks"].as_array().unwrap().len() - old_len);
    write_json(&staging.join("session-marks.json"), &marks)?;
    let mut habits = optional_json(
        local,
        "polling-habits.json",
        json!({"version":1,"aliases":{},"anchors":{}}),
    )?;
    validate_habits(&habits)?;
    let incoming = read_json(&imported.join("polling-habits.json"), MAX_JSON)?;
    fill_missing(&mut habits, &incoming);
    validate_habits(&habits)?;
    write_json(&staging.join("polling-habits.json"), &habits)?;
    // Preserve machine-specific state; only old SQLite sidecars are intentionally discarded.
    for name in ["clips.json", "update-resume.json"] {
        if local.join(name).exists() {
            copy_atomic(&local.join(name), &staging.join(name))?;
        }
    }
    validate_db(&staging.join("messages.sqlite3"), true)?;
    validate_db(&staging.join("clips.sqlite3"), false)?;
    Ok(added)
}
fn mark_identity(mark: &Value) -> String {
    json!([
        mark["anchor_id"],
        mark["session_id"],
        mark["marked_at_epoch"],
        mark["title"],
        mark["note"]
    ])
    .to_string()
}

fn table_columns(table: &TableShape) -> String {
    table
        .columns
        .iter()
        .map(|c| format!("\"{}\"", c.0))
        .collect::<Vec<_>>()
        .join(",")
}
fn merge_messages(path: &Path, incoming: &Path, added: &mut Value) -> Result<()> {
    let mut conn = writable(path)?;
    conn.execute(
        "ATTACH DATABASE ?1 AS incoming",
        [incoming.to_string_lossy().as_ref()],
    )
    .map_err(db_error)?;
    let before: i64 = conn
        .query_row("SELECT COALESCE(MAX(id),0) FROM messages", [], |r| r.get(0))
        .map_err(db_error)?;
    let tx = conn.transaction().map_err(db_error)?;
    // Activity notifications from different machines need not retain the same server message ID.
    // Resolve only authoritative aliases and reliable lottery draw seconds, never titles.
    const DRAW:&str="CASE WHEN json_valid(payload_json) THEN CASE WHEN json_extract(payload_json,'$.activity_kind')='lottery' AND json_type(payload_json,'$.activity_details.draw_at') IN ('integer','real') THEN CASE WHEN json_extract(payload_json,'$.activity_details.draw_at')>=1000000000000 AND json_extract(payload_json,'$.activity_details.draw_at')<253402300800000 THEN CAST(json_extract(payload_json,'$.activity_details.draw_at')/1000 AS INTEGER) WHEN json_extract(payload_json,'$.activity_details.draw_at')>=1 AND json_extract(payload_json,'$.activity_details.draw_at')<253402300800 THEN CAST(json_extract(payload_json,'$.activity_details.draw_at') AS INTEGER) END END END";
    tx.execute_batch(&format!("CREATE TEMP TABLE backup_activity_matches(source_id INTEGER NOT NULL,local_id INTEGER NOT NULL,PRIMARY KEY(source_id,local_id));
      INSERT OR IGNORE INTO backup_activity_matches SELECT i.message_row_id,l.message_row_id FROM incoming.activity_aliases i JOIN main.activity_aliases l USING(anchor_id,session_id,kind,activity_id);
      INSERT OR IGNORE INTO backup_activity_matches SELECT i.id,l.id FROM (SELECT *,({DRAW}) AS backup_draw FROM incoming.messages WHERE message_type='activity') i JOIN (SELECT *,({DRAW}) AS backup_draw FROM main.messages WHERE message_type='activity') l ON l.anchor_id=i.anchor_id AND l.session_id=i.session_id AND l.backup_draw=i.backup_draw WHERE i.backup_draw IS NOT NULL;")).map_err(db_error)?;
    let ambiguous:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM backup_activity_matches GROUP BY source_id HAVING COUNT(*)>1)",[],|r|r.get(0)).map_err(db_error)?;
    if ambiguous {
        return Err(error("备份活动身份存在冲突，当前数据未修改"));
    }
    tx.execute_batch("CREATE TEMP TABLE backup_new_sessions(anchor_id TEXT NOT NULL,session_id TEXT NOT NULL,PRIMARY KEY(anchor_id,session_id));
      INSERT INTO backup_new_sessions SELECT i.anchor_id,i.session_id FROM (SELECT anchor_id,session_id FROM incoming.quality_registry UNION SELECT anchor_id,session_id FROM incoming.sessions) i WHERE NOT EXISTS(SELECT 1 FROM main.quality_registry l WHERE l.anchor_id=i.anchor_id AND l.session_id=i.session_id) AND NOT EXISTS(SELECT 1 FROM main.sessions l WHERE l.anchor_id=i.anchor_id AND l.session_id=i.session_id);
      CREATE TEMP TABLE backup_changed_sessions(anchor_id TEXT NOT NULL,session_id TEXT NOT NULL,PRIMARY KEY(anchor_id,session_id));
      INSERT OR IGNORE INTO backup_changed_sessions SELECT i.anchor_id,i.session_id FROM incoming.quality_manual i WHERE NOT EXISTS(SELECT 1 FROM main.quality_manual l WHERE l.anchor_id=i.anchor_id AND l.session_id=i.session_id AND l.normalized=i.normalized);").map_err(db_error)?;
    for name in ["anchors", "sessions", "quality_registry"] {
        let count = tx
            .execute(
                &format!("INSERT OR IGNORE INTO main.{name} SELECT * FROM incoming.{name}"),
                [],
            )
            .map_err(db_error)?;
        if name == "anchors" {
            added["anchors"] = json!(count);
        }
        for field in if name == "anchors" {
            vec!["anchor_name", "web_rid"]
        } else {
            vec!["anchor_name", "web_rid", "title", "started_at"]
        } {
            let identity = if name == "anchors" {
                "i.anchor_id=l.anchor_id"
            } else {
                "i.anchor_id=l.anchor_id AND i.session_id=l.session_id"
            };
            tx.execute(&format!("UPDATE main.{name} AS l SET {field}=(SELECT i.{field} FROM incoming.{name} i WHERE {identity}) WHERE ({field} IS NULL OR {field}='') AND EXISTS(SELECT 1 FROM incoming.{name} i WHERE {identity} AND i.{field} IS NOT NULL AND i.{field}<>'')"),[]).map_err(db_error)?;
        }
    }
    added["sessions"] = json!(tx
        .query_row("SELECT COUNT(*) FROM backup_new_sessions", [], |r| r
            .get::<_, i64>(0))
        .map_err(db_error)?);
    tx.execute_batch("INSERT OR IGNORE INTO quality_registry(anchor_id,session_id,anchor_name,web_rid,title,first_seen,last_seen,started_at) SELECT anchor_id,session_id,anchor_name,web_rid,title,first_seen,last_seen,started_at FROM sessions;
      INSERT INTO session_monitor_durations SELECT * FROM incoming.session_monitor_durations WHERE 1 ON CONFLICT(anchor_id,session_id) DO UPDATE SET accumulated_seconds=MAX(accumulated_seconds,excluded.accumulated_seconds),last_checkpoint_at=MAX(last_checkpoint_at,excluded.last_checkpoint_at);
      INSERT OR IGNORE INTO quality_settings SELECT * FROM incoming.quality_settings;
      INSERT OR IGNORE INTO quality_manual SELECT * FROM incoming.quality_manual;
      INSERT OR IGNORE INTO audience_minutes SELECT * FROM incoming.audience_minutes;").map_err(db_error)?;
    const TIME:&str="CASE WHEN time_source='server' AND message_time<>'' THEN message_time ELSE received_at END";
    const FIELDS: &str =
        "anchor_id,session_id,sender_id,message_type,content,time_source,payload_json";
    const MATCH:&str="m.anchor_id=i.anchor_id AND m.session_id=i.session_id AND m.sender_id=i.sender_id AND m.message_type=i.message_type AND m.identity_time=i.identity_time AND m.ordinal=i.ordinal AND m.content=i.content AND m.time_source=i.time_source AND m.payload_json=i.payload_json";
    // Missing server IDs are a multiset: preserve independent repeated notifications while only
    // adding the excess source occurrences. Window ordinals keep repeated imports idempotent.
    let local_fallback=format!("SELECT id local_id,{FIELDS},({TIME}) identity_time,ROW_NUMBER() OVER(PARTITION BY {FIELDS},({TIME}) ORDER BY id) ordinal FROM main.messages WHERE message_id=''");
    tx.execute_batch(&format!("CREATE TEMP TABLE backup_fallback_incoming AS SELECT id source_id,{FIELDS},({TIME}) identity_time,ROW_NUMBER() OVER(PARTITION BY {FIELDS},({TIME}) ORDER BY id) ordinal FROM incoming.messages WHERE message_id='' AND id NOT IN(SELECT source_id FROM backup_activity_matches);
      CREATE TEMP TABLE backup_fallback_local AS {local_fallback};
      CREATE INDEX temp.backup_fallback_lookup ON backup_fallback_local(anchor_id,session_id,sender_id,message_type,identity_time,ordinal);
      CREATE TEMP TABLE backup_fallback_matches(source_id INTEGER PRIMARY KEY,local_id INTEGER NOT NULL);
      INSERT INTO backup_fallback_matches SELECT i.source_id,m.local_id FROM backup_fallback_incoming i JOIN backup_fallback_local m ON {MATCH};")).map_err(db_error)?;
    let columns = expected_shape(true)?
        .tables
        .iter()
        .find(|t| t.name == "messages")
        .unwrap()
        .columns
        .iter()
        .filter(|c| c.0 != "id")
        .map(|c| format!("\"{}\"", c.0))
        .collect::<Vec<_>>()
        .join(",");
    let reliable=tx.execute(&format!("INSERT OR IGNORE INTO main.messages({columns}) SELECT {columns} FROM incoming.messages WHERE message_id<>'' AND id NOT IN(SELECT source_id FROM backup_activity_matches) ORDER BY id"),[]).map_err(db_error)?;
    let fallback=tx.execute(&format!("INSERT INTO main.messages({columns}) SELECT {columns} FROM incoming.messages WHERE id IN(SELECT source_id FROM backup_fallback_incoming WHERE source_id NOT IN(SELECT source_id FROM backup_fallback_matches)) ORDER BY id"),[]).map_err(db_error)?;
    added["messages"] = json!(reliable + fallback);
    tx.execute_batch(&format!("CREATE TEMP TABLE backup_message_map(source_id INTEGER PRIMARY KEY,local_id INTEGER NOT NULL);
      INSERT INTO backup_message_map SELECT i.id,m.id FROM incoming.messages i JOIN main.messages m ON m.anchor_id=i.anchor_id AND m.session_id=i.session_id AND m.message_id=i.message_id WHERE i.message_id<>'' AND m.message_id<>'' AND i.id NOT IN(SELECT source_id FROM backup_activity_matches);
      DELETE FROM backup_fallback_local;
      INSERT INTO backup_fallback_local {local_fallback};
      INSERT INTO backup_message_map SELECT i.source_id,m.local_id FROM backup_fallback_incoming i JOIN backup_fallback_local m ON {MATCH};
      INSERT OR IGNORE INTO backup_message_map SELECT source_id,local_id FROM backup_activity_matches;")).map_err(db_error)?;
    let unmapped:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM incoming.messages i WHERE NOT EXISTS(SELECT 1 FROM backup_message_map m WHERE m.source_id=i.id))",[],|r|r.get(0)).map_err(db_error)?;
    if unmapped {
        return Err(error("备份消息身份无法安全合并，当前数据未修改"));
    }
    for name in ["activity_aliases", "activity_rounds", "quality_results"] {
        let table = expected_shape(true)?
            .tables
            .iter()
            .find(|t| t.name == name)
            .unwrap();
        let reference = if name == "quality_results" {
            "message_id"
        } else {
            "message_row_id"
        };
        let select = table
            .columns
            .iter()
            .map(|c| {
                if c.0 == reference {
                    "map.local_id".to_owned()
                } else {
                    format!("i.\"{}\"", c.0)
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        let scope = if name == "quality_results" {
            "WHERE EXISTS(SELECT 1 FROM backup_new_sessions s WHERE s.anchor_id=i.anchor_id AND s.session_id=i.session_id)"
        } else {
            ""
        };
        tx.execute(&format!("INSERT OR IGNORE INTO main.{name}({}) SELECT {select} FROM incoming.{name} i JOIN backup_message_map map ON map.source_id=i.{reference} {scope}",table_columns(table)),[]).map_err(db_error)?;
    }
    for table in &expected_shape(true)?.tables {
        if matches!(
            table.name.as_str(),
            "quality_results" | "quality_registry" | "quality_manual"
        ) || !table.name.starts_with("quality_")
            || !table.columns.iter().any(|c| c.0 == "session_id")
        {
            continue;
        }
        let select = table
            .columns
            .iter()
            .map(|c| {
                if table.name == "quality_jobs" && c.0 == "cursor" {
                    "0".to_owned()
                } else {
                    format!("i.\"{}\"", c.0)
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        tx.execute(&format!("INSERT OR IGNORE INTO main.{}({}) SELECT {select} FROM incoming.{} i WHERE EXISTS(SELECT 1 FROM backup_new_sessions s WHERE s.anchor_id=i.anchor_id AND s.session_id=i.session_id)",table.name,table_columns(table),table.name),[]).map_err(db_error)?;
    }
    tx.execute("INSERT OR IGNORE INTO backup_changed_sessions SELECT anchor_id,session_id FROM messages WHERE id>?1",[before]).map_err(db_error)?;
    // Rebuild only overlapping sessions that received genuinely new evidence. Keeping generations
    // separate prevents source and destination aggregates from being added twice.
    tx.execute_batch("DELETE FROM backup_changed_sessions WHERE EXISTS(SELECT 1 FROM backup_new_sessions n WHERE n.anchor_id=backup_changed_sessions.anchor_id AND n.session_id=backup_changed_sessions.session_id);
      INSERT OR IGNORE INTO quality_gc SELECT r.anchor_id,r.session_id,r.generation FROM quality_registry r JOIN backup_changed_sessions c USING(anchor_id,session_id);
      UPDATE quality_registry SET generation=generation+1,cleared=0 WHERE EXISTS(SELECT 1 FROM backup_changed_sessions c WHERE c.anchor_id=quality_registry.anchor_id AND c.session_id=quality_registry.session_id);
      INSERT INTO quality_jobs(anchor_id,session_id,generation,cursor,state,turn) SELECT r.anchor_id,r.session_id,r.generation,0,'queued',0 FROM quality_registry r JOIN backup_changed_sessions c USING(anchor_id,session_id) WHERE 1 ON CONFLICT(anchor_id,session_id) DO UPDATE SET generation=excluded.generation,cursor=0,state='queued',turn=0;
      ").map_err(db_error)?;
    tx.commit().map_err(db_error)?;
    conn.execute_batch("DETACH DATABASE incoming;")
        .map_err(db_error)?;
    Ok(())
}

fn merge_clips(path: &Path, incoming: &Path, directory: &Path, added: &mut Value) -> Result<()> {
    let portable = tempfile::tempdir_in(directory).map_err(|_| error("无法准备下载记录合并"))?;
    let portable_path = portable.path().join("clips.sqlite3");
    copy_atomic(incoming, &portable_path)?;
    rebase_clips(&portable_path, directory)?;
    let mut conn = writable(path)?;
    conn.execute(
        "ATTACH DATABASE ?1 AS incoming",
        [portable_path.to_string_lossy().as_ref()],
    )
    .map_err(db_error)?;
    let tx = conn.transaction().map_err(db_error)?;
    // The ID and idempotency-key constraints both prefer local records. Local output paths and
    // completed files are untouched; imported active tasks were interrupted before attachment.
    added["downloads"] = json!(tx
        .execute(
            "INSERT OR IGNORE INTO main.clip_jobs SELECT * FROM incoming.clip_jobs",
            []
        )
        .map_err(db_error)?);
    tx.execute(
        "INSERT OR IGNORE INTO clip_history_meta VALUES('legacy_json_v1',?1)",
        [storage::now()],
    )
    .map_err(db_error)?;
    tx.commit().map_err(db_error)?;
    conn.execute_batch("DETACH DATABASE incoming;")
        .map_err(db_error)?;
    Ok(())
}
