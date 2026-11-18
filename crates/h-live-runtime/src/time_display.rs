//! Display timezone only: archived timestamps and clip media seconds stay unchanged.
use crate::{runtime::Runtime, storage::Result};
use serde_json::{Value, json};

const DEFAULT_TIMEZONE: &str = "Asia/Shanghai";

// The UI verifies supported zones through Intl. The native boundary accepts only
// a bounded IANA identifier shape, without adding a timezone database dependency.
fn valid_timezone(timezone: &str) -> bool {
    !timezone.is_empty()
        && timezone.len() <= 64
        && timezone.is_ascii()
        && timezone.split('/').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'+'))
                && part.bytes().any(|c| c.is_ascii_alphabetic())
        })
}

impl Runtime {
    pub(crate) fn time_display_request(&self, method: &str, p: Value) -> Result<Value> {
        match method {
            "time_display_settings" => {
                let state = self.shared.state.lock().map_err(|_| "时间显示设置不可用")?;
                let timezone = state.config.value["time_display"]["timezone"]
                    .as_str()
                    .filter(|zone| valid_timezone(zone))
                    .unwrap_or(DEFAULT_TIMEZONE);
                Ok(json!({"timezone":timezone}))
            }
            "time_display_save" => {
                let timezone = p["timezone"]
                    .as_str()
                    .filter(|zone| valid_timezone(zone))
                    .ok_or("时区格式无效，请使用 Asia/Shanghai 或其他 IANA 时区")?;
                let mut state = self.shared.state.lock().map_err(|_| "时间显示设置不可用")?;
                let mut config = state.config.value.clone();
                config["time_display"] = json!({"timezone":timezone});
                state.config.replace(config)?;
                Ok(json!({"timezone":timezone}))
            }
            _ => Err("不支持的时间显示操作".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timezone_defaults_to_china_and_survives_restart_without_changing_existing_data() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(directory.path().into()).unwrap();
        assert_eq!(
            runtime.request("time_display_settings", json!({})).unwrap(),
            json!({"timezone":"Asia/Shanghai"})
        );
        let original_config = runtime.shared.state.lock().unwrap().config.value.clone();
        let saved = runtime
            .request("time_display_save", json!({"timezone":"America/New_York"}))
            .unwrap();
        assert_eq!(saved, json!({"timezone":"America/New_York"}));
        let mut after_config = runtime.shared.state.lock().unwrap().config.value.clone();
        after_config.as_object_mut().unwrap().remove("time_display");
        assert_eq!(
            after_config, original_config,
            "a display preference must not change collector or media configuration"
        );
        runtime.shutdown().unwrap();
        drop(runtime);
        let reopened = Runtime::open(directory.path().into()).unwrap();
        assert_eq!(
            reopened
                .request("time_display_settings", json!({}))
                .unwrap(),
            saved
        );
        assert_eq!(
            reopened
                .request("time_display_save", json!({"timezone":"UTC"}))
                .unwrap(),
            json!({"timezone":"UTC"})
        );
        assert_eq!(
            reopened
                .request("time_display_save", json!({"timezone":"Etc/GMT+8"}))
                .unwrap(),
            json!({"timezone":"Etc/GMT+8"})
        );
        reopened.shutdown().unwrap();
    }

    #[test]
    fn invalid_timezone_requests_never_overwrite_the_saved_preference_or_file() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(directory.path().into()).unwrap();
        runtime
            .request("time_display_save", json!({"timezone":"Asia/Tokyo"}))
            .unwrap();
        let path = directory.path().join("config.json");
        let before = std::fs::read(&path).unwrap();
        for timezone in [
            json!(null),
            json!(9),
            json!(""),
            json!(" Asia/Shanghai"),
            json!("Asia/Shanghai "),
            json!("Asia//Shanghai"),
            json!("/Asia/Shanghai"),
            json!("Asia/Shanghai/"),
            json!("../Asia/Shanghai"),
            json!("Asia\\Shanghai"),
            json!("Asia/上 海"),
            json!("UTC\n"),
            json!("UTC+08:00"),
            json!("A".repeat(65)),
        ] {
            assert!(
                runtime
                    .request("time_display_save", json!({"timezone":timezone}))
                    .is_err()
            );
            assert_eq!(
                runtime.request("time_display_settings", json!({})).unwrap()["timezone"],
                "Asia/Tokyo"
            );
            assert_eq!(std::fs::read(&path).unwrap(), before);
        }
        assert!(runtime.request("time_display_save", json!({})).is_err());
        runtime.shutdown().unwrap();
    }
}
