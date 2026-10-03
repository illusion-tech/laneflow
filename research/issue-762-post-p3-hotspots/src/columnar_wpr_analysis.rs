//! #814：先验封窗口与计数器失效范围；零丢失或非零 PMC 不能替代有效性验证。
use crate::{Result, need};
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn number(row: &Value, key: &str) -> Result<u64> {
    row[key]
        .as_u64()
        .ok_or_else(|| format!("missing integer {key}").into())
}

pub(crate) fn windows(text: &str) -> Result<Value> {
    let rows = text
        .lines()
        .filter_map(|line| line.strip_prefix("LF814_WPR "))
        .map(serde_json::from_str)
        .collect::<std::result::Result<Vec<Value>, _>>()?;
    need(
        rows.len() == 256,
        "complete 256-tick Windows marker window required",
    )?;
    let mut previous_end = 0;
    let mut pid = 0;
    let mut sum = 0_u128;
    let mut max_width = 0;
    let mut max_error = 0;
    for (i, row) in rows.iter().enumerate() {
        need(
            number(row, "tick")? == i as u64 + 1,
            "missing or reordered Windows tick",
        )?;
        let current_pid = number(row, "pid")?;
        if i == 0 {
            pid = current_pid;
        }
        need(
            pid != 0 && current_pid == pid,
            "mixed Windows process markers",
        )?;
        let a = number(row, "start_lower_filetime")?;
        let b = number(row, "start_upper_filetime")?;
        let c = number(row, "end_lower_filetime")?;
        let d = number(row, "end_upper_filetime")?;
        let step = number(row, "step_ns")?;
        let error = number(row, "clock_error_ns")?;
        need(
            a > 0 && a <= b && b < c && c <= d && previous_end <= a && step > 0,
            "overlapping, reversed or empty Windows window",
        )?;
        need(error <= 100_000, "Windows system clock divergence")?;
        let minimum = u128::from(c - b) * 100;
        let maximum = u128::from(d - a) * 100;
        need(
            u128::from(step) + u128::from(error) >= minimum
                && u128::from(step) <= maximum + u128::from(error),
            "monotonic latency outside anchor bounds",
        )?;
        let width = (b - a)
            .max(d - c)
            .checked_mul(100)
            .ok_or("anchor width overflow")?;
        need(width <= 100_000, "Windows anchor uncertainty too large")?;
        max_width = max_width.max(width);
        max_error = max_error.max(error);
        previous_end = d;
        sum += u128::from(step);
    }
    Ok(
        json!({"schema":"lf814-windows-step-windows-v1","ticks":256,"pid":pid,
        "wall_sum_ns":sum.to_string(),"wall_mean_ns":sum as f64 / 256.0,
        "max_anchor_width_ns":max_width,"max_clock_error_ns":max_error,"windows":rows,
        "marker_windows_valid":true,"etl_clock_alignment_verified":false,"performance_acceptance":false,
        "limits":"FILETIME anchors with monotonic latency; ETL clock alignment and PMU/thread interval validity require independent verification"}),
    )
}

pub(crate) fn status(value: &Value) -> Result<Value> {
    need(
        value["schema"] == "lf814-pmc-status-investigation-v1" && value["failed"] == false,
        "unrecognized or failed native status investigation",
    )?;
    need(
        number(value, "qpc_frequency")? > 0
            && number(value, "events_lost")? == 0
            && number(value, "buffers_lost")? == 0,
        "missing QPC clock or ETW loss",
    )?;
    let cpus = value["cpus"].as_array().ok_or("CPU switch ranges")?;
    let targets = value["target_process_switch_ranges"]
        .as_array()
        .ok_or("target process ranges")?;
    let statuses = value["counter_status"]
        .as_array()
        .ok_or("counter status records")?;
    need(
        !cpus.is_empty() && !targets.is_empty(),
        "no CPU or target evidence",
    )?;
    let mut cpu_ids = BTreeSet::new();
    for cpu in cpus {
        need(cpu_ids.insert(number(cpu, "cpu")?), "duplicate CPU")?;
        need(
            number(cpu, "counter_or_time_reversals")? == 0
                && number(cpu, "thread_continuity_failures")? == 0,
            "counter or thread continuity failure",
        )?;
    }
    let mut invalid_intervals = Vec::new();
    let mut pairs = BTreeSet::new();
    for row in statuses {
        let cpu = number(row, "cpu")?;
        let detected = number(row, "qpc")?;
        need(
            cpu_ids.contains(&cpu),
            "counter status CPU absent from switch evidence",
        )?;
        for counter in row["counters"].as_array().ok_or("counter status list")? {
            let source = number(counter, "source")?;
            let good = number(counter, "last_known_good_qpc")?;
            need(
                [19, 26].contains(&source)
                    && pairs.insert((cpu, source))
                    && good > 0
                    && good <= detected,
                "unknown, duplicate or reversed corruption status",
            )?;
            let affected: Vec<_> = targets
                .iter()
                .filter_map(|target| {
                    let start = number(target, "first_qpc");
                    let end = number(target, "last_qpc");
                    match (start, end) {
                        (Ok(start), Ok(end)) if good < end && detected > start => {
                            Some(target["pid"].clone())
                        }
                        _ => None,
                    }
                })
                .collect();
            invalid_intervals.push(json!({"cpu":cpu,"source":source,
                "last_known_good_qpc":good,"detected_qpc":detected,"overlapping_target_pids":affected}));
        }
    }
    let mut pids = BTreeSet::new();
    for target in targets {
        need(
            number(target, "first_qpc")? > 0
                && number(target, "first_qpc")? < number(target, "last_qpc")?
                && pids.insert(number(target, "pid")?),
            "invalid or duplicate target range",
        )?;
    }
    let overlaps = invalid_intervals.iter().any(|row| {
        row["overlapping_target_pids"]
            .as_array()
            .is_some_and(|p| !p.is_empty())
    });
    Ok(
        json!({"schema":"lf814-wpr-counter-status-check-v1","analysis_only":true,
        "cpu_count":cpu_ids.len(),"status_events":statuses.len(),"counter_status_entries":pairs.len(),
        "counter_status_overlaps_target_ranges":overlaps,"status_allows_target_ranges":!overlaps,
        "uncertified_intervals":invalid_intervals,"performance_acceptance":false,
        "limits":"conservative interval from last known good to detection, based on frozen local WMI layout; no root-cause attribution, no assumption that corruption started exactly at last known good; target process lifetimes are not public step windows; absence of status overlap alone does not certify PMU"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn evidence() -> Value {
        json!({"schema":"lf814-pmc-status-investigation-v1","failed":false,"qpc_frequency":10_000_000,
            "events_lost":0,"buffers_lost":0,"cpus":[{"cpu":0,"counter_or_time_reversals":0,"thread_continuity_failures":0}],
            "target_process_switch_ranges":[{"pid":7,"first_qpc":200,"last_qpc":300}],
            "counter_status":[{"cpu":0,"qpc":400,"counters":[{"source":19,"last_known_good_qpc":100},{"source":26,"last_known_good_qpc":100}]}]})
    }
    #[test]
    fn corruption_detected_after_workload_still_invalidates_unproven_interval() {
        assert_eq!(
            status(&evidence()).unwrap()["status_allows_target_ranges"],
            false
        );
        let mut v = evidence();
        v["counter_status"][0]["counters"][0]["last_known_good_qpc"] = json!(350);
        v["counter_status"][0]["counters"][1]["last_known_good_qpc"] = json!(350);
        assert_eq!(status(&v).unwrap()["status_allows_target_ranges"], true);
        assert_eq!(status(&v).unwrap()["performance_acceptance"], false);
    }
    #[test]
    fn lost_events_counter_reversal_and_duplicate_status_fail_closed() {
        let mut v = evidence();
        v["events_lost"] = json!(1);
        assert!(status(&v).is_err());
        let mut v = evidence();
        v["cpus"][0]["counter_or_time_reversals"] = json!(1);
        assert!(status(&v).is_err());
        let mut v = evidence();
        v["counter_status"][0]["counters"][1]["source"] = json!(19);
        assert!(status(&v).is_err());
        let mut v = evidence();
        v["counter_status"][0]["counters"][1]["last_known_good_qpc"] = json!(401);
        assert!(status(&v).is_err());
    }
    fn markers() -> Vec<Value> {
        (1..=256).map(|tick| { let start = 10_000 + tick * 100;
            json!({"pid":7,"tick":tick,"start_lower_filetime":start,"start_upper_filetime":start+1,
                "end_lower_filetime":start+10,"end_upper_filetime":start+11,"step_ns":1_000,"clock_error_ns":0})
        }).collect()
    }
    fn trace(rows: &[Value]) -> String {
        rows.iter()
            .map(|v| format!("LF814_WPR {v}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn complete_monotonic_markers_preserve_exact_filetime_and_reject_bad_windows() {
        let v = markers();
        assert_eq!(windows(&trace(&v)).unwrap()["ticks"], 256);
        assert!(windows(&trace(&v[..255])).is_err());
        let mut v = markers();
        v[10]["pid"] = json!(8);
        assert!(windows(&trace(&v)).is_err());
        let mut v = markers();
        v[10]["step_ns"] = json!(2_000);
        assert!(windows(&trace(&v)).is_err());
        let mut v = markers();
        v[10]["start_lower_filetime"] = json!(1);
        assert!(windows(&trace(&v)).is_err());
        let mut v = markers();
        v[10]["clock_error_ns"] = json!(100_001);
        assert!(windows(&trace(&v)).is_err());
    }
}
