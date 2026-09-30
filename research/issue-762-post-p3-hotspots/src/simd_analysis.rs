use crate::{
    BASE, EXPERIMENT, Result, cache_research, chunk_build, chunk_collector, environment, need, plan,
};
use serde_json::{Value, json};
use std::path::Path;

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}
fn metric(run: &Value, window: &str, field: &str) -> Result<f64> {
    run["windows"][window]["step"][field]
        .as_f64()
        .ok_or("SIMD metric".into())
}

pub(crate) fn summarize(runs: &[Value]) -> Result<Value> {
    let mut comparison = json!({});
    for scale in ["10k", "100k"] {
        for window in ["all", "entry", "screen"] {
            let mut rows = json!({});
            for arm in ["base", "layout", "candidate"] {
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
                rows[arm] = json!({"mean_ms":mean(&means),"average_process_p95_ms":mean(&p95s),
                    "process_means_ms":means,"process_p95_ms":p95s});
            }
            let mut groups = Vec::new();
            let mut max_span = 0.0_f64;
            let mut improved = 0;
            let mut simd_improved = 0;
            for group in 1..=3 {
                let mut grouped = json!({});
                for arm in ["base", "layout", "candidate"] {
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
                let layout = grouped["layout"]["mean_ms"].as_f64().ok_or("layout")?;
                let candidate = grouped["candidate"]["mean_ms"]
                    .as_f64()
                    .ok_or("candidate")?;
                improved += usize::from(candidate < base);
                simd_improved += usize::from(candidate < layout);
                groups.push(json!({"group":group,"arms":grouped,
                    "candidate_vs_base_percent":(candidate/base-1.0)*100.0,
                    "candidate_vs_layout_percent":(candidate/layout-1.0)*100.0}));
            }
            let base = rows["base"]["mean_ms"].as_f64().ok_or("base")?;
            let layout = rows["layout"]["mean_ms"].as_f64().ok_or("layout")?;
            let candidate = rows["candidate"]["mean_ms"].as_f64().ok_or("candidate")?;
            comparison[scale][window] = json!({"arms":rows,"groups":groups,
                "layout_vs_base_percent":(layout/base-1.0)*100.0,
                "candidate_vs_base_percent":(candidate/base-1.0)*100.0,
                "candidate_vs_layout_percent":(candidate/layout-1.0)*100.0,
                "maximum_within_arm_span_percent":max_span,
                "candidate_improving_groups":improved,"simd_improving_groups":simd_improved});
        }
    }
    let hundred = &comparison["100k"]["screen"];
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
        hundred["candidate_improving_groups"] == 3
            && -hundred["candidate_vs_base_percent"]
                .as_f64()
                .ok_or("delta")?
                > hundred["maximum_within_arm_span_percent"]
                    .as_f64()
                    .ok_or("span")?
            && p95_regression <= 2.0
    );
    comparison["ten_k_p95_regression_percent"] = json!(p95_regression);
    Ok(comparison)
}

pub(crate) fn analyze(raw: &Path) -> Result<Value> {
    let mut value = cache_research::analyze_planned_for(raw, EXPERIMENT, plan(), None)?;
    let collector = chunk_collector::verify_running()?;
    for arm in ["base", "layout", "candidate"] {
        let source = &value["identity"]["sources"][arm];
        need(
            source["base"] == BASE
                && source["arm"] == arm
                && source["mode"] == "plain"
                && source["protocol"] == EXPERIMENT.protocol
                && source["build"]["collector"] == collector,
            "SIMD source/collector",
        )?;
        chunk_build::verify_raw(raw, source, &value["identity"]["binaries"][arm]["sha256"])?;
    }
    (plan().validate_builds)(&value["identity"])?;
    for run in value["runs"].as_array().ok_or("SIMD runs")? {
        environment::verify(raw, run["label"].as_str().ok_or("label")?)?;
    }
    value["comparison"] = summarize(value["runs"].as_array().ok_or("SIMD runs")?)?;
    value["collector_verified"] = json!(true);
    value["build_settings_equal"] = json!(true);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_or_duplicate_group_members_fail_closed() {
        assert!(summarize(&[]).is_err());
        let runs = (0..36)
            .map(|i| {
                json!({"scale":if i<18 {"10k"} else {"100k"},
            "arm":(["base","layout","candidate"][i%3]), "label":"100k-1-1-base-plain",
            "windows":{"all":{"step":{"mean_ms":10.0,"p95_ms":11.0}}}})
            })
            .collect::<Vec<_>>();
        assert!(summarize(&runs).is_err());
    }
}
