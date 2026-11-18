//! Labels belong to confirmed anchors. A room key is only a temporary assignment.
use crate::{
    runtime::{id_list, Runtime},
    storage::{self, Result},
};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub(crate) const MAX_TAGS: usize = 20;
pub(crate) const MAX_TAG_CHARS: usize = 24;

pub(crate) fn normalize(value: &Value) -> Result<Vec<String>> {
    let values = value
        .as_array()
        .filter(|a| a.len() <= 200)
        .ok_or("标签须为字符串数组，每位主播最多 20 个标签")?;
    let mut seen = BTreeSet::new();
    let mut tags = Vec::new();
    for value in values {
        let raw = value
            .as_str()
            .filter(|s| s.len() <= 4096)
            .ok_or("标签须为字符串")?;
        let tag = raw.trim();
        if tag.is_empty()
            || tag.chars().count() > MAX_TAG_CHARS
            || tag
                .chars()
                .any(|c| c.is_control() || matches!(c, ',' | '，'))
        {
            return Err("标签须为 1 至 24 个字符，不能包含逗号或控制字符".into());
        }
        if seen.insert(tag.to_owned()) {
            tags.push(tag.to_owned());
        }
        if tags.len() > MAX_TAGS {
            return Err("每位主播最多 20 个标签".into());
        }
    }
    Ok(tags)
}

pub(crate) fn canonical(value: &Value) -> Result<Value> {
    if value.is_null() {
        return Ok(json!({}));
    }
    let rows = value
        .as_object()
        .filter(|o| o.len() <= 10_000)
        .ok_or("主播标签配置无效，原数据已保留")?;
    let mut output = json!({});
    for (key, value) in rows {
        let valid = if let Some(rid) = key.strip_prefix("room:") {
            storage::id(rid)
        } else {
            // Historical archives can contain opaque stable identities (including sec_uid).
            // These are exact JSON keys, never room IDs, filesystem paths or SQL fragments.
            !key.is_empty()
                && key.encode_utf16().count() <= 160
                && !key.chars().any(|c| c < ' ' || c == '\u{7f}')
        };
        if !valid {
            return Err("主播标签身份无效，原数据已保留".into());
        }
        let tags = normalize(value)?;
        // Explicit empty rows are valid and remain meaningful to additive import.
        output[key] = json!(tags);
    }
    Ok(output)
}
pub(crate) fn validate(value: &Value) -> Result<()> {
    canonical(value).map(|_| ())
}

pub(crate) fn tags(config: &Value, rid: &str, anchor: &str) -> Vec<String> {
    // A pending label is visible on its monitored room until a real identity is observed.
    let pending = format!("room:{rid}");
    let value = if storage::id(rid)
        && config["anchors"].get(rid).is_some()
        && config["anchor_tags"].get(&pending).is_some()
    {
        &config["anchor_tags"][&pending]
    } else if storage::id(anchor) {
        &config["anchor_tags"][anchor]
    } else {
        &Value::Null
    };
    if value.is_null() {
        Vec::new()
    } else {
        normalize(value).unwrap_or_default()
    }
}

pub(crate) fn assign(config: &mut Value, rid: &str, anchor: &str, tags: &[String]) {
    if !config["anchor_tags"].is_object() {
        config["anchor_tags"] = json!({});
    }
    let pending = format!("room:{rid}");
    let key = if storage::id(anchor) {
        anchor.to_owned()
    } else {
        pending.clone()
    };
    config["anchor_tags"][&key] = json!(tags);
    if storage::id(anchor) {
        config["anchor_tags"]
            .as_object_mut()
            .unwrap()
            .remove(&pending);
    }
}

pub(crate) fn confirm(config: &mut Value, rid: &str, anchor: &str) {
    if !storage::id(anchor) {
        return;
    }
    let pending = format!("room:{rid}");
    if let Some(tags) = config["anchor_tags"]
        .as_object_mut()
        .and_then(|o| o.remove(&pending))
    {
        // A newly resolved entrance adopts an existing person's labels. Pending edits made
        // before identity was known cannot replace that person's confirmed assignment.
        // Presence matters: an explicit empty list is an intentional clear.
        if config["anchor_tags"].get(anchor).is_none() {
            config["anchor_tags"][anchor] = tags;
        }
    }
}

impl Runtime {
    pub(crate) fn set_anchor_tags(&self, p: &Value) -> Result<Value> {
        let rids = id_list(&p["rids"], false)?;
        let tags = normalize(&p["tags"])?;
        let mut state = self.shared.state.lock().map_err(|_| "后台状态不可用")?;
        if rids
            .iter()
            .any(|rid| state.config.value["anchors"].get(rid).is_none())
        {
            return Err("部分直播间不在列表中".into());
        }
        let mut config = state.config.value.clone();
        let mut assigned_anchors = BTreeSet::new();
        for rid in &rids {
            let pending = format!("room:{rid}");
            let pending_unconfirmed = config["anchor_tags"].get(&pending).is_some()
                && !state
                    .rooms
                    .get(rid)
                    .is_some_and(|room| room.value["anchor_identity_confirmed"] == true);
            let anchor = if pending_unconfirmed {
                String::new()
            } else {
                state
                    .rooms
                    .get(rid)
                    .and_then(|room| room.value["anchor_id"].as_str())
                    .filter(|id| storage::id(id))
                    .or_else(|| {
                        config["anchor_metadata"][rid]["anchor_id"]
                            .as_str()
                            .filter(|id| storage::id(id))
                    })
                    .unwrap_or("")
                    .to_owned()
            };
            assign(&mut config, rid, &anchor, &tags);
            if storage::id(&anchor) {
                assigned_anchors.insert(anchor);
            }
        }
        // Another entrance may already have confirmed this same person but not yet flushed
        // its older pending tags. A deliberate edit must remain the latest assignment.
        for (rid, room) in &state.rooms {
            if room.value["anchor_identity_confirmed"] == true
                && room.value["anchor_id"]
                    .as_str()
                    .is_some_and(|anchor| assigned_anchors.contains(anchor))
            {
                config["anchor_tags"]
                    .as_object_mut()
                    .unwrap()
                    .remove(&format!("room:{rid}"));
            }
        }
        state.config.replace(config)?;
        Ok(json!({"updated":rids,"tags":tags}))
    }

    pub(crate) fn archive_anchor_rows(&self) -> Result<Value> {
        let anchors = self
            .shared
            .archive
            .list_anchors()
            .map_err(|_| "读取历史主播失败")?;
        let mut anchors = serde_json::to_value(anchors).map_err(|_| "历史主播格式无效")?;
        let state = self.shared.state.lock().map_err(|_| "后台状态不可用")?;
        for row in anchors.as_array_mut().ok_or("历史主播格式无效")? {
            let anchor = row["anchor_id"].as_str().unwrap_or("");
            // Archive rows never fall back to a room label: the same entrance can change owner.
            let tags = state.config.value["anchor_tags"]
                .get(anchor)
                .map(normalize)
                .transpose()?
                .unwrap_or_default();
            row["tags"] = json!(tags);
        }
        Ok(anchors)
    }
}

#[cfg(test)]
#[path = "anchor_tags_tests.rs"]
mod tests;
