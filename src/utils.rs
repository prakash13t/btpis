pub use crate::adapters::{AdapterDirection, fetch_iflow_adapters};

use chrono::{DateTime, NaiveDateTime, Utc};
use indexmap::IndexMap;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;
use tabled::Tabled;

#[derive(Tabled)]
pub struct DetailRow {
    #[tabled(rename = "Field")]
    pub field: String,
    #[tabled(rename = "Value")]
    pub value: String,
}

pub fn format_timestamp(value: &str) -> String {
    if let Ok(ms) = value.parse::<i64>() {
        if let Some(dt) = DateTime::from_timestamp_millis(ms) {
            return dt.with_timezone(&Utc).format("%m-%d-%Y").to_string();
        }
    }

    if let Ok(seconds) = value.parse::<i64>() {
        if let Some(dt) = DateTime::from_timestamp(seconds, 0) {
            return dt
                .with_timezone(&Utc)
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string();
        }
    }

    if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
        return dt
            .with_timezone(&Utc)
            .format("%Y-%m-%d %H:%M:%S UTC")
            .to_string();
    }

    value.to_string()
}

pub fn summarise_adapters(types: Vec<&str>) -> String {
    let mut counts: IndexMap<&str, usize> = IndexMap::new();
    for t in types {
        *counts.entry(t).or_insert(0) += 1;
    }
    counts
        .iter()
        .map(|(t, n)| {
            if *n > 1 {
                format!("{} {}", n, t)
            } else {
                t.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn parse_json_date(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.starts_with("/Date(") && s.ends_with(")/") {
        let inner = &s[6..s.len() - 2];
        return inner.parse::<i64>().ok();
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    for fmt in &["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(dt.and_utc().timestamp_millis());
        }
    }
    None
}

pub fn format_duration(start_ms: i64, end_ms: i64) -> String {
    let diff_ms = end_ms - start_ms;
    if diff_ms < 0 {
        return "—".to_string();
    }
    let total_secs = diff_ms / 1000;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    let millis = diff_ms % 1000;

    if hours > 0 {
        format!("{}h {:02}m {:02}s", hours, minutes, secs)
    } else if minutes > 0 {
        format!("{}m {:02}.{:03}s", minutes, secs, millis)
    } else if secs > 0 || millis > 0 {
        format!("{}.{:03}s", secs, millis)
    } else {
        "0ms".to_string()
    }
}

pub fn parse_duration_relative(s: &str) -> Option<std::time::Duration> {
    let s = s.trim();
    if s.len() < 2 {
        return None;
    }
    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: u64 = num_str.parse().ok()?;
    match unit {
        "s" => Some(std::time::Duration::from_secs(num)),
        "m" => Some(std::time::Duration::from_secs(num * 60)),
        "h" => Some(std::time::Duration::from_secs(num * 3600)),
        "d" => Some(std::time::Duration::from_secs(num * 86400)),
        _ => None,
    }
}

pub fn start_spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

pub fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
