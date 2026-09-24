//! Flexible JSON reading helpers. Provider endpoints change over time;
//! everything here falls back to "no window" instead of panicking.

use serde_json::Value;

use super::Window;

pub fn num(v: &Value, keys: &[&str]) -> Option<f64> {
    let obj = v.as_object()?;
    keys.iter().find_map(|k| match obj.get(*k)? {
        Value::Number(n) => n.as_f64().filter(|x| x.is_finite()),
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|x| x.is_finite()),
        _ => None,
    })
}

pub fn string(v: &Value, keys: &[&str]) -> Option<String> {
    let obj = v.as_object()?;
    keys.iter().find_map(|k| obj.get(*k)?.as_str().filter(|s| !s.is_empty()).map(str::to_string))
}

pub fn clamp_pct(x: f64) -> u8 {
    if !x.is_finite() {
        return 0;
    }
    x.round().clamp(0.0, 100.0) as u8
}

/// Converts a reset time to unix seconds: ISO text, seconds or milliseconds.
pub fn reset_at(v: &Value, keys: &[&str]) -> Option<i64> {
    let obj = v.as_object()?;
    for key in keys {
        match obj.get(*key) {
            Some(Value::String(s)) if !s.is_empty() => {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                    return Some(dt.timestamp());
                }
                if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                    return d.and_hms_opt(0, 0, 0).map(|t| t.and_utc().timestamp());
                }
                if let Ok(n) = s.parse::<f64>() {
                    return Some(unix(n, key));
                }
            }
            Some(Value::Number(n)) => {
                if let Some(f) = n.as_f64().filter(|f| *f > 0.0) {
                    return Some(unix(f, key));
                }
            }
            _ => {}
        }
    }
    None
}

fn unix(n: f64, key: &str) -> i64 {
    if key.ends_with("Ms") || n > 1e12 { (n / 1000.0) as i64 } else { n as i64 }
}

/// Window label from a provider key: five_hour → 5H, seven_day → WEEK.
pub fn normalize_label(key: &str) -> String {
    let k: String = key.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if k.contains("fivehour") || k == "hourly" || k == "5h" || k == "primary" || k.contains("session") {
        "5H".into()
    } else if k.contains("sevenday") || k.contains("week") || k == "7d" || k == "secondary" {
        "WEEK".into()
    } else if k.contains("month") {
        "MONTH".into()
    } else if k.contains("daily") || k == "day" {
        "DAY".into()
    } else {
        key.to_uppercase().chars().take(8).collect()
    }
}

/// Label from a duration (Codex windows are named by duration).
pub fn label_for_duration(seconds: f64) -> String {
    if seconds >= 28.0 * 86_400.0 {
        "MONTH".into()
    } else if seconds >= 6.0 * 86_400.0 {
        "WEEK".into()
    } else if seconds >= 22.0 * 3600.0 {
        "DAY".into()
    } else if seconds >= 4.0 * 3600.0 {
        "5H".into()
    } else if seconds >= 60.0 {
        format!("{}M", (seconds / 60.0).round().max(1.0))
    } else {
        "WINDOW".into()
    }
}

const RESET_KEYS: &[&str] =
    &["resets_at", "resetsAt", "until", "reset_date", "resetsAtMs", "resetTime", "reset_time", "quota_reset_date"];

/// Converts a single window-shaped object into a `Window`. Supported shapes:
/// `{utilization}`, `{usedPercent}`, `{percent_remaining}`, `{used, limit}`,
/// `{remaining, entitlement}`; optional name/duration and reset time.
pub fn window_from(v: &Value, fallback: &str) -> Option<Window> {
    let obj = v.as_object()?;
    if obj.get("unlimited").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let label = if let Some(l) = string(v, &["label", "name", "window"]) {
        normalize_label(&l)
    } else if let Some(s) = num(v, &["durationSeconds", "duration"]) {
        label_for_duration(s)
    } else if let Some(m) = num(v, &["windowDurationMins", "windowDurationMinutes", "durationMins"]) {
        label_for_duration(m * 60.0)
    } else {
        fallback.to_string()
    };
    let resets_at = reset_at(v, RESET_KEYS);
    if let Some(direct) = num(v, &["utilization", "used_percent", "usedPercent", "percent"]) {
        return Some(Window { label, used: clamp_pct(direct), resets_at });
    }
    if let Some(rem) = num(v, &["percent_remaining", "percentRemaining", "remaining_percent"]) {
        return Some(Window { label, used: clamp_pct(100.0 - rem), resets_at });
    }
    if let Some(frac) = num(v, &["remainingFraction", "remaining_fraction"]) {
        return Some(Window { label, used: clamp_pct((1.0 - frac) * 100.0), resets_at });
    }
    let limit = num(v, &["limit", "maximum", "total", "max", "entitlement"]).filter(|l| *l > 0.0)?;
    let used =
        if let Some(u) = num(v, &["used", "current"]) { u } else { limit - num(v, &["remaining", "available"])? };
    Some(Window { label, used: clamp_pct(used / limit * 100.0), resets_at })
}

/// Windows under the given keys; otherwise it looks inside containers
/// such as `rateLimits`/`limits`. At most `max` windows.
pub fn extract_windows(root: &Value, keys: &[&str], max: usize) -> Vec<Window> {
    let mut out: Vec<Window> = Vec::new();
    let add = |w: Window, out: &mut Vec<Window>| {
        if out.len() < max && !out.iter().any(|x| x.label == w.label) {
            out.push(w);
        }
    };
    for k in keys {
        if let Some(w) = root.get(*k).and_then(|v| window_from(v, &normalize_label(k))) {
            add(w, &mut out);
        }
    }
    if !out.is_empty() {
        return out;
    }
    for container in ["rateLimits", "rate_limits", "limits", "windows", "quotas", "usage", "quota"] {
        match root.get(container) {
            Some(Value::Array(items)) => {
                for item in items {
                    if let Some(w) = window_from(item, "WINDOW") {
                        add(w, &mut out);
                    }
                }
            }
            Some(obj @ Value::Object(map)) => {
                if let Some(w) = window_from(obj, &normalize_label(container)) {
                    add(w, &mut out);
                } else {
                    for (k, child) in map {
                        if let Some(w) = window_from(child, &normalize_label(k)) {
                            add(w, &mut out);
                        }
                    }
                }
            }
            _ => {}
        }
        if !out.is_empty() {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn window_shapes() {
        let w = window_from(&json!({"utilization": 71.4, "resets_at": "2026-09-22T18:00:00Z"}), "5H").unwrap();
        assert_eq!(w.used, 71);
        assert_eq!(
            w.resets_at,
            Some(chrono::DateTime::parse_from_rfc3339("2026-09-22T18:00:00Z").unwrap().timestamp())
        );
        let w = window_from(&json!({"usedPercent": 18, "windowDurationMins": 300, "resetsAt": 1_779_459_394}), "X")
            .unwrap();
        assert_eq!((w.label.as_str(), w.used, w.resets_at), ("5H", 18, Some(1_779_459_394)));
        let w = window_from(&json!({"percent_remaining": 25.0, "entitlement": 300}), "PREMIUM").unwrap();
        assert_eq!(w.used, 75);
        let w = window_from(&json!({"remaining": 30, "entitlement": 300}), "P").unwrap();
        assert_eq!(w.used, 90);
        assert!(window_from(&json!({"unlimited": true, "entitlement": 0}), "P").is_none());
        assert!(window_from(&json!({"foo": 1}), "P").is_none());
    }

    #[test]
    fn nested_containers() {
        let v = json!({"rateLimits": {"primary": {"usedPercent": 10, "windowDurationMins": 300},
                                      "secondary": {"usedPercent": 40, "windowDurationMins": 10080}}});
        let ws = extract_windows(&v, &["fiveHour", "weekly"], 2);
        assert_eq!(ws.len(), 2);
        assert_eq!(ws[0].label, "5H");
        assert_eq!(ws[1].label, "WEEK");
        assert_eq!(ws[1].used, 40);
    }
}
