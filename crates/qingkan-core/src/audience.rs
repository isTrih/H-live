//! Minute buckets retain actual online observations, independently of chat.
use super::*;
pub(super) const SCHEMA:&str="CREATE TABLE IF NOT EXISTS audience_minutes(anchor_id TEXT NOT NULL,session_id TEXT NOT NULL,minute INTEGER NOT NULL,last_at REAL NOT NULL,mean_count REAL NOT NULL CHECK(mean_count>=0),samples INTEGER NOT NULL CHECK(samples>0),peak INTEGER NOT NULL CHECK(peak>=0),PRIMARY KEY(anchor_id,session_id,minute)) WITHOUT ROWID;";
pub(super) fn record(c: &Connection, p: &Value) -> Result<Value> {
    let a = p["anchor_id"]
        .as_str()
        .ok_or_else(|| ArchiveError::Invalid("主播无效".into()))?;
    identity(a)?;
    let s = p["session_id"]
        .as_str()
        .ok_or_else(|| ArchiveError::Invalid("场次无效".into()))?;
    identity(s)?;
    let n = p["viewer_count"]
        .as_i64()
        .filter(|n| (0..=9_007_199_254_740_991).contains(n))
        .ok_or_else(|| ArchiveError::Invalid("在线人数无效".into()))?;
    let t = parse_time(
        p["observed_at"]
            .as_str()
            .ok_or_else(|| ArchiveError::Invalid("采样时间无效".into()))?,
    )?;
    let epoch = t.timestamp_micros() as f64 / 1_000_000.;
    if epoch <= 0. {
        return Err(ArchiveError::Invalid("采样时间无效".into()));
    }
    if !c
        .prepare("SELECT 1 FROM quality_registry WHERE anchor_id=? AND session_id=?")?
        .exists(params![a, s])?
    {
        return Err(ArchiveError::Invalid("尚未登记此直播场次".into()));
    }
    c.execute("INSERT INTO audience_minutes VALUES(?,?,?,?,?,1,?) ON CONFLICT(anchor_id,session_id,minute) DO UPDATE SET peak=MAX(peak,excluded.peak),mean_count=CASE WHEN excluded.last_at>last_at THEN mean_count+(excluded.mean_count-mean_count)/(samples+1) ELSE mean_count END,samples=samples+(excluded.last_at>last_at),last_at=MAX(last_at,excluded.last_at)",params![a,s,t.timestamp().div_euclid(60),epoch,n,n])?;
    Ok(json!({"recorded":true}))
}
pub(super) fn summary(c: &Connection, a: &str, s: &str, value: &mut Value) -> Result<()> {
    let (peak,avg,minutes,last):(Option<i64>,Option<f64>,i64,Option<f64>)=c.query_row("SELECT MAX(peak),AVG(mean_count),COUNT(*),MAX(last_at) FROM audience_minutes WHERE anchor_id=? AND session_id=?",params![a,s],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    value["peak_viewers"] = json!(peak);
    value["minute_avg_viewers"] = json!(avg);
    value["audience_sample_minutes"] = json!(minutes);
    value["audience_last_sample_at"] = json!(last);
    // Clearing chat changes the numerator's coverage while audience buckets
    // remain. Do not present the resulting mismatched scopes as a ratio.
    let ready = value["analysis_state"] == "ready" && value["records_cleared"] != true;
    for (key, numerator) in [
        ("speakers_to_online_rate", "speakers"),
        ("valid_speakers_to_online_rate", "valid_speakers"),
    ] {
        value[key] = if ready {
            avg.filter(|n| *n > 0.)
                .map(|n| json!(value[numerator].as_f64().unwrap_or(0.) / n))
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        };
    }
    Ok(())
}
