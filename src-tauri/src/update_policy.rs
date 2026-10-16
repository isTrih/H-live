use serde_json::{json, Value};

// Fail closed if an older/malformed backend cannot prove all work is complete.
pub fn readiness(response: Value) -> Value {
    let ready = response.get("ready").and_then(Value::as_bool) == Some(true)
        && response.get("reason").and_then(Value::as_str) == Some("ready");
    let reason = if ready {
        "ready"
    } else {
        match response.get("reason").and_then(Value::as_str) {
            Some("monitoring") => "monitoring",
            Some("exporting") => "exporting",
            _ => "pending",
        }
    };
    json!({"ready": ready, "reason": reason})
}

pub fn shutdown_confirmed(response: &Value) -> bool {
    response.get("closed").and_then(Value::as_bool) == Some(true)
        && response.get("all_workers_stopped").and_then(Value::as_bool) == Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_requires_explicit_matching_confirmation() {
        for value in [
            json!({}),
            json!({"ready": true}),
            json!({"ready": true, "reason": "monitoring"}),
            json!({"ready": "true", "reason": "ready"}),
        ] {
            assert_eq!(readiness(value)["ready"], false);
        }
        assert_eq!(
            readiness(json!({"ready":true,"reason":"ready"}))["ready"],
            true
        );
    }

    #[test]
    fn readiness_does_not_return_backend_details_or_arbitrary_reasons() {
        let value = readiness(json!({"ready":false,"reason":"private URL", "detail":"secret"}));
        assert_eq!(value, json!({"ready":false,"reason":"pending"}));
    }

    #[test]
    fn shutdown_must_confirm_both_service_and_workers() {
        assert!(!shutdown_confirmed(&json!({"closed":true})));
        assert!(!shutdown_confirmed(
            &json!({"closed":true,"all_workers_stopped":false})
        ));
        assert!(shutdown_confirmed(
            &json!({"closed":true,"all_workers_stopped":true})
        ));
    }
}
