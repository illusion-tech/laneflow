use crate::{
    BASE, EXPERIMENT, Result, cache_research, chunk_build, chunk_collector, environment, need, plan,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}
fn metric(run: &Value, window: &str, field: &str) -> Result<f64> {
    let value = run["windows"][window]["step"][field]
        .as_f64()
        .ok_or("split metric")?;
    need(value.is_finite() && value > 0.0, "positive finite metric")?;
    Ok(value)
}

pub(crate) fn summarize(runs: &[Value]) -> Result<Value> {
    need(runs.len() == 24, "24 processes")?;
    let labels: BTreeSet<_> = runs.iter().map(|r| r["label"].as_str()).collect();
    need(
        labels.len() == 24 && !labels.contains(&None),
        "unique process labels",
    )?;
    let mut comparison = json!({});
    for scale in ["10k", "100k"] {
        for window in ["all", "entry", "screen"] {
            let mut rows = json!({});
            for arm in ["base", "candidate"] {
                let selected: Vec<_> = runs
                    .iter()
                    .filter(|r| r["scale"] == scale && r["arm"] == arm)
                    .collect();
                need(selected.len() == 6, "six processes per arm/scale")?;
                let means: Vec<_> = selected
                    .iter()
                    .map(|r| metric(r, window, "mean_ms"))
                    .collect::<Result<_>>()?;
                let p95s: Vec<_> = selected
                    .iter()
                    .map(|r| metric(r, window, "p95_ms"))
                    .collect::<Result<_>>()?;
                rows[arm] = json!({"mean_ms":mean(&means),"average_process_p95_ms":mean(&p95s),"process_means_ms":means,"process_p95_ms":p95s});
            }
            let mut groups = Vec::new();
            let mut max_span = 0.0_f64;
            let mut improved = 0;
            let mut regressed = 0;
            for group in 1..=3 {
                let mut grouped = json!({});
                for arm in ["base", "candidate"] {
                    let selected: Vec<_> = runs
                        .iter()
                        .filter(|r| {
                            r["scale"] == scale
                                && r["arm"] == arm
                                && r["label"]
                                    .as_str()
                                    .is_some_and(|s| s.starts_with(&format!("{scale}-{group}-")))
                        })
                        .collect();
                    need(selected.len() == 2, "two processes per arm/group")?;
                    let values: Vec<_> = selected
                        .iter()
                        .map(|r| metric(r, window, "mean_ms"))
                        .collect::<Result<_>>()?;
                    let m = mean(&values);
                    let span = (values[0] - values[1]).abs() / m * 100.0;
                    max_span = max_span.max(span);
                    grouped[arm] = json!({"mean_ms":m,"within_arm_span_percent":span});
                }
                let base = grouped["base"]["mean_ms"].as_f64().ok_or("base")?;
                let candidate = grouped["candidate"]["mean_ms"]
                    .as_f64()
                    .ok_or("candidate")?;
                improved += usize::from(candidate < base);
                regressed += usize::from(candidate > base);
                groups.push(json!({"group":group,"arms":grouped,"candidate_vs_base_percent":(candidate/base-1.0)*100.0}));
            }
            let delta = (rows["candidate"]["mean_ms"].as_f64().ok_or("candidate")?
                / rows["base"]["mean_ms"].as_f64().ok_or("base")?
                - 1.0)
                * 100.0;
            let direction = if delta.abs() > max_span && improved == 3 {
                "benefit"
            } else if delta.abs() > max_span && regressed == 3 {
                "cost"
            } else {
                "inconclusive"
            };
            comparison[scale][window] = json!({"arms":rows,"groups":groups,"candidate_vs_base_percent":delta,"maximum_within_arm_span_percent":max_span,"candidate_improving_groups":improved,"candidate_regressing_groups":regressed,"split_direction":direction});
        }
    }
    let ten = &comparison["10k"]["screen"]["arms"];
    let p95_regression = (ten["candidate"]["average_process_p95_ms"]
        .as_f64()
        .ok_or("p95")?
        / ten["base"]["average_process_p95_ms"]
            .as_f64()
            .ok_or("p95")?
        - 1.0)
        * 100.0;
    comparison["continue_candidate"] = json!(
        comparison["100k"]["screen"]["split_direction"] == "benefit" && p95_regression <= 2.0
    );
    comparison["ten_k_p95_regression_percent"] = json!(p95_regression);
    Ok(comparison)
}

pub(crate) fn analyze(raw: &Path) -> Result<Value> {
    let mut value = cache_research::analyze_planned_for(raw, EXPERIMENT, plan(), None)?;
    let collector = chunk_collector::verify_running()?;
    for arm in ["base", "candidate"] {
        let source = &value["identity"]["sources"][arm];
        need(
            source["base"] == BASE
                && source["arm"] == arm
                && source["mode"] == "plain"
                && source["protocol"] == EXPERIMENT.protocol
                && source["batch_lanes"] == 1
                && source["motion_split"] == (arm == "candidate")
                && source["build"]["collector"] == collector,
            "split source/collector",
        )?;
        chunk_build::verify_raw(raw, source, &value["identity"]["binaries"][arm]["sha256"])?;
    }
    (plan().validate_builds)(&value["identity"])?;
    for run in value["runs"].as_array().ok_or("split runs")? {
        environment::verify(raw, run["label"].as_str().ok_or("label")?)?;
    }
    value["comparison"] = summarize(value["runs"].as_array().ok_or("split runs")?)?;
    value["collector_verified"] = json!(true);
    value["build_settings_equal"] = json!(true);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(candidate: f64) -> Vec<Value> {
        ["10k", "100k"].into_iter().flat_map(|scale| (1..=3).flat_map(move |group| ["base", "candidate"].into_iter().flat_map(move |arm| (1..=2).map(move |position| {
            let m = if arm == "base" { 10.0 } else { candidate };
            let step = json!({"step":{"mean_ms":m,"p95_ms":m*1.1}});
            json!({"scale":scale,"arm":arm,"label":format!("{scale}-{group}-{position}-{arm}-plain"),"windows":{"all":step,"entry":step,"screen":step}})
        })))).collect()
    }
    #[test]
    fn rejects_missing_duplicate_and_nonfinite_metrics() {
        assert!(summarize(&[]).is_err());
        let mut runs = fixture(10.0);
        runs[1] = runs[0].clone();
        assert!(summarize(&runs).is_err());
        let mut runs = fixture(10.0);
        runs[0]["windows"]["screen"]["step"]["mean_ms"] = json!(0.0);
        assert!(summarize(&runs).is_err());
    }
    #[test]
    fn requires_all_groups_and_difference_above_span() {
        assert_eq!(
            summarize(&fixture(11.0)).unwrap()["100k"]["screen"]["split_direction"],
            "cost"
        );
        assert_eq!(
            summarize(&fixture(9.0)).unwrap()["100k"]["screen"]["split_direction"],
            "benefit"
        );
        assert_eq!(
            summarize(&fixture(10.0)).unwrap()["100k"]["screen"]["split_direction"],
            "inconclusive"
        );
        let mut runs = fixture(11.0);
        runs[12]["windows"]["screen"]["step"]["mean_ms"] = json!(8.0);
        assert_eq!(
            summarize(&runs).unwrap()["100k"]["screen"]["split_direction"],
            "inconclusive"
        );
    }
}
