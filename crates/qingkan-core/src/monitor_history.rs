//! Recover known monitor identities without treating a past session as currently live.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// An exact room entrance associated with a real anchor and its latest known session.
/// Multiple rows for the same entrance deliberately preserve identity ambiguity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MonitorHistory {
    pub web_rid: String,
    pub anchor_id: String,
    pub session_id: String,
    pub anchor_name: String,
}

impl ArchiveStore {
    /// Read only session metadata from a single archive snapshot. This includes
    /// zero-message and cleared sessions without scanning the message history.
    pub fn monitor_history(&self) -> Result<Vec<MonitorHistory>> {
        let snapshot = self.snapshot()?;
        let mut query = snapshot.connection.prepare(
            "SELECT web_rid,anchor_id,session_id,anchor_name,first_seen,last_seen,2 FROM quality_registry
             UNION ALL
             SELECT web_rid,anchor_id,session_id,anchor_name,first_seen,last_seen,1 FROM sessions",
        )?;
        let mut rows = query.query([])?;
        let mut aliases = BTreeSet::new();
        let mut latest = BTreeMap::new();
        while let Some(row) = rows.next()? {
            let rid: String = row.get(0)?;
            let anchor: String = row.get(1)?;
            let session: String = row.get(2)?;
            let priority: i64 = row.get(6)?;
            if identity(&rid).is_err() || identity(&anchor).is_err() || identity(&session).is_err()
            {
                continue;
            }
            aliases.insert((rid, anchor.clone()));
            let name: String = row.get(3)?;
            let first: String = row.get(4)?;
            let last: String = row.get(5)?;
            let rank = (
                parse_time(&last).ok(),
                parse_time(&first).ok(),
                session.clone(),
                priority,
                name.clone(),
            );
            let candidate = latest
                .entry(anchor)
                .or_insert_with(|| (rank.clone(), session.clone(), name.clone()));
            if rank > candidate.0 {
                *candidate = (rank, session, name);
            }
        }
        Ok(aliases
            .into_iter()
            .filter_map(|(web_rid, anchor_id)| {
                latest
                    .get(&anchor_id)
                    .map(|(_, session_id, anchor_name)| MonitorHistory {
                        web_rid,
                        anchor_id: anchor_id.clone(),
                        session_id: session_id.clone(),
                        anchor_name: anchor_name.clone(),
                    })
            })
            .collect())
    }
}
