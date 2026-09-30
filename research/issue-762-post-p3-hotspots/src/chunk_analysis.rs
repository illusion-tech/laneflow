use crate::{
    BASE, EXPERIMENT, Result, STAGES, cache_research, chunk_build, environment, io, need, plan,
};
use cache_research::DetailProtocol;
use serde_json::{Value, json};
use std::{fs, path::Path};

fn integer(value: &Value) -> Result<u64> {
    value
        .as_u64()
        .ok_or_else(|| "expected unsigned integer".into())
}
fn array(value: &Value, len: usize) -> Result<Vec<u64>> {
    let values = value.as_array().ok_or("expected array")?;
    need(values.len() == len, "array length")?;
    values.iter().map(integer).collect()
}
fn validate_stages(rows: &[Value], ticks: &[Value]) -> Result<()> {
    need(rows.len() == 256 && ticks.len() == 256, "row count")?;
    for (i, row) in rows.iter().enumerate() {
        need(
            row["tick"] == i + 1 && ticks[i]["tick"] == i + 1,
            "tick sequence",
        )?;
        integer(&ticks[i]["N_active"])?;
        let step = integer(&row["step_ns"])?;
        let times = array(&row["stages_ns"], 12)?;
        let calls = array(&row["calls"], 12)?;
        need(
            step > 0 && calls.iter().all(|&n| n == 1),
            "stage calls/time",
        )?;
        need(
            times[..10].iter().map(|&n| u128::from(n)).sum::<u128>() <= u128::from(step)
                && u128::from(times[10]) + u128::from(times[11]) <= u128::from(times[3]),
            "stage nesting",
        )?;
    }
    Ok(())
}

const METRICS: [&str; 9] = [
    "dispatch",
    "max_chunk_elapsed",
    "summed_chunk_elapsed",
    "first_start",
    "last_two_chunk_end_gap",
    "after_last_end",
    "max_worker_elapsed",
    "min_worker_elapsed",
    "worker_elapsed_spread",
];
struct Derived {
    times: [u64; 9],
    worker_elapsed: [u64; 4],
    participating: u64,
    overlap: u64,
}
fn derive(row: &Value, tick: usize, factor: u64, motion_ns: u64) -> Result<Derived> {
    need(row["tick"] == tick, "chunk tick")?;
    let workload = integer(&row["workload"])?;
    let size = integer(&row["chunk_size"])?;
    let dispatch = integer(&row["dispatch_ns"])?;
    let target = factor.checked_mul(4).ok_or("target overflow")?;
    need(
        workload >= 1_024
            && size == workload.div_ceil(target)
            && dispatch > 0
            && dispatch <= motion_ns,
        "chunk workload/clock",
    )?;
    let chunks = row["chunks"].as_array().ok_or("chunks")?;
    need(
        chunks.len() as u64 == target && workload.div_ceil(size) == target,
        "chunk count",
    )?;
    let mut cursor = 0_u64;
    let mut elapsed = Vec::new();
    let mut intervals: [Vec<(u64, u64)>; 4] = Default::default();
    let mut endpoints = Vec::new();
    let mut ends = Vec::new();
    let mut starts = Vec::new();
    let mut worker_elapsed = [0_u64; 4];
    for chunk in chunks {
        let c = array(chunk, 6)?;
        let [start, len, began, ended, worker, done] = c.as_slice() else {
            unreachable!()
        };
        need(
            *start == cursor
                && *len == size.min(workload - cursor)
                && *len > 0
                && *began < *ended
                && *ended <= dispatch
                && *worker < 4
                && *done == 1,
            "chunk partition/completion/clock",
        )?;
        cursor = cursor.checked_add(*len).ok_or("partition overflow")?;
        let span = *ended - *began;
        elapsed.push(span);
        starts.push(*began);
        ends.push(*ended);
        let worker = usize::try_from(*worker)?;
        worker_elapsed[worker] = worker_elapsed[worker]
            .checked_add(span)
            .ok_or("worker overflow")?;
        intervals[worker].push((*began, *ended));
        // 同时刻先结束再开始；只衡量墙钟区间重叠，不推断 CPU 同时执行。
        endpoints.push((*began, 1_i64));
        endpoints.push((*ended, -1_i64));
    }
    need(cursor == workload, "partition coverage")?;
    for intervals in &mut intervals {
        intervals.sort_unstable();
        need(
            intervals.windows(2).all(|pair| pair[0].1 <= pair[1].0),
            "worker overlap",
        )?;
    }
    endpoints.sort_unstable();
    let mut depth = 0_i64;
    let mut peak = 0_i64;
    for (_, event) in endpoints {
        depth += event;
        need((0..=4).contains(&depth), "interval depth")?;
        peak = peak.max(depth);
    }
    need(depth == 0, "interval balance")?;
    ends.sort_unstable();
    let max_worker = *worker_elapsed.iter().max().ok_or("workers")?;
    let min_worker = *worker_elapsed.iter().min().ok_or("workers")?;
    Ok(Derived {
        times: [
            dispatch,
            *elapsed.iter().max().ok_or("elapsed")?,
            elapsed.iter().try_fold(0_u64, |sum, &n| {
                sum.checked_add(n).ok_or("elapsed overflow")
            })?,
            *starts.iter().min().ok_or("starts")?,
            ends[ends.len() - 1] - ends[ends.len() - 2],
            dispatch - ends[ends.len() - 1],
            max_worker,
            min_worker,
            max_worker - min_worker,
        ],
        worker_elapsed,
        participating: intervals.iter().filter(|v| !v.is_empty()).count() as u64,
        overlap: u64::try_from(peak)?,
    })
}
fn stats(mut values: Vec<u64>) -> Value {
    values.sort_unstable();
    json!({"mean_ms":values.iter().map(|&v|u128::from(v)).sum::<u128>() as f64 / values.len() as f64 / 1e6,
        "p95_ms":values[(values.len()*95).div_ceil(100)-1] as f64 / 1e6})
}
fn counts(values: Vec<u64>) -> Value {
    json!({"mean":values.iter().sum::<u64>() as f64 / values.len() as f64,
        "min":values.iter().min(),"max":values.iter().max()})
}
fn float(value: &Value) -> Result<f64> {
    let number = value.as_f64().ok_or("number")?;
    need(number.is_finite() && number > 0.0, "positive finite number")?;
    Ok(number)
}
fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}
fn delta(base: f64, candidate: f64) -> f64 {
    (candidate / base - 1.0) * 100.0
}
fn summarize_plain(runs: &[Value]) -> Result<Value> {
    let mut summary = json!({});
    for scale in ["10k", "100k"] {
        let rows: Vec<_> = runs.iter().filter(|r| r["scale"] == scale).collect();
        need(rows.len() == 12, "plain scale count")?;
        for window in ["all", "entry", "screen"] {
            let mut groups = Vec::new();
            let mut base_means = Vec::new();
            let mut candidate_means = Vec::new();
            let mut base_p95s = Vec::new();
            let mut candidate_p95s = Vec::new();
            let mut max_span = 0_f64;
            for (i, group) in rows.as_chunks::<4>().0.iter().enumerate() {
                let read = |arm: &str, field: &str| -> Result<Vec<f64>> {
                    let values: Vec<_> = group
                        .iter()
                        .filter(|r| r["arm"] == arm)
                        .map(|r| float(&r["windows"][window]["step"][field]))
                        .collect::<Result<_>>()?;
                    need(values.len() == 2, "quartet balance")?;
                    Ok(values)
                };
                let b = read("base", "mean_ms")?;
                let c = read("candidate", "mean_ms")?;
                let bp = read("base", "p95_ms")?;
                let cp = read("candidate", "p95_ms")?;
                let span = |v: &[f64]| (v[0] - v[1]).abs() / mean(v) * 100.0;
                max_span = max_span.max(span(&b)).max(span(&c));
                groups.push(json!({"group":i+1,"order":group.iter().map(|r|r["arm"].clone()).collect::<Vec<_>>(),
                    "base_mean_ms":mean(&b),"candidate_mean_ms":mean(&c),"mean_delta_pct":delta(mean(&b),mean(&c)),
                    "base_p95_ms":mean(&bp),"candidate_p95_ms":mean(&cp),"p95_delta_pct":delta(mean(&bp),mean(&cp)),
                    "base_within_arm_span_pct":span(&b),"candidate_within_arm_span_pct":span(&c)}));
                base_means.extend(b);
                candidate_means.extend(c);
                base_p95s.extend(bp);
                candidate_p95s.extend(cp);
            }
            let mean_delta = delta(mean(&base_means), mean(&candidate_means));
            let p95_delta = delta(mean(&base_p95s), mean(&candidate_p95s));
            let directions = groups
                .iter()
                .filter(|g| g["mean_delta_pct"].as_f64().is_some_and(|d| d < 0.0))
                .count();
            summary[scale][window] = json!({"groups":groups,
                "base_run_means_ms":base_means,"candidate_run_means_ms":candidate_means,
                "base_run_p95s_ms":base_p95s,"candidate_run_p95s_ms":candidate_p95s,
                "base_mean_ms":mean(&base_means),"candidate_mean_ms":mean(&candidate_means),
                "mean_delta_pct":mean_delta,"p95_delta_pct":p95_delta,
                "max_within_quartet_arm_span_pct":max_span,"improving_quartets":directions,
                "passes_screening_rule":directions == 3 && mean_delta <= -2.0 && -mean_delta > max_span && p95_delta <= 2.0});
        }
    }
    summary["adopt_candidate"] = json!(
        summary["10k"]["screen"]["passes_screening_rule"] == true
            && summary["100k"]["screen"]["passes_screening_rule"] == true
    );
    Ok(summary)
}

fn compare_modes(detail: &Value, plain: &Value) -> Result<()> {
    need(
        plain["identity"]["mode"] == "plain"
            && detail["identity"]["mode"] == "detail"
            && plain["identity"]["inputs"] == detail["identity"]["inputs"],
        "plain/detail input or mode mismatch",
    )?;
    for arm in ["base", "candidate"] {
        chunk_build::compatible(
            &detail["identity"]["sources"][arm]["build"],
            &plain["identity"]["sources"][arm]["build"],
        )?;
    }
    let references: Vec<_> = plain["runs"]
        .as_array()
        .ok_or("plain runs")?
        .iter()
        .filter(|r| r["scale"] == "100k")
        .collect();
    let diagnostics = detail["runs"].as_array().ok_or("detail runs")?;
    need(
        references.len() == 12 && diagnostics.len() == 6,
        "cross-mode run count",
    )?;
    for run in diagnostics {
        need(run["scale"] == "100k", "detail scale")?;
        for reference in &references {
            for field in ["traffic", "initial_counts", "final_counts"] {
                let valid_shape = if field == "traffic" {
                    run[field].is_object()
                } else {
                    run[field]
                        .as_array()
                        .is_some_and(|v| v.len() == 3 && v.iter().all(|n| n.as_u64().is_some()))
                };
                need(
                    valid_shape && run[field] == reference[field],
                    &format!("plain/detail {field} mismatch"),
                )?;
            }
        }
    }
    Ok(())
}

pub(crate) fn analyze(raw: &Path, plain_raw: Option<&Path>) -> Result<Value> {
    let identity = io::read_json(&raw.join("identity.json"))?;
    let mode = identity["mode"].as_str().ok_or("mode")?;
    need(
        (mode == "detail") == plain_raw.is_some(),
        "detail requires plain raw evidence; plain forbids a reference",
    )?;
    let plain = if let Some(reference) = plain_raw {
        need(
            io::read_json(&reference.join("identity.json"))?["mode"] == "plain",
            "reference must be plain",
        )?;
        Some(analyze(reference, None)?)
    } else {
        None
    };
    let mut output = cache_research::analyze_planned_for(
        raw,
        EXPERIMENT,
        plan(mode)?,
        Some(DetailProtocol {
            stages: &STAGES,
            validate: validate_stages,
        }),
    )?;
    for (arm, factor) in [("base", 2), ("candidate", 4)] {
        let source = &identity["sources"][arm];
        need(
            source["base"] == BASE
                && source["protocol"] == EXPERIMENT.protocol
                && source["arm"] == arm
                && source["mode"] == mode
                && source["chunks_per_worker"] == factor,
            "source protocol",
        )?;
        chunk_build::verify_raw(raw, source, &identity["binaries"][arm]["sha256"])?;
    }
    chunk_build::validate_pair(&identity)?;
    let collector = crate::chunk_collector::verify_running()?;
    need(
        identity["head"] == collector["source_git_head"]
            && identity["tree"] == collector["source_git_tree"]
            && identity["sources"]["base"]["build"]["collector"] == collector,
        "raw collector differs from verified executable/source",
    )?;
    output["build_settings_equal"] = json!(true);
    output["collector_verified"] = json!(true);
    output["cargo_configuration_absent"] = json!(true);
    let runs = output["runs"].as_array_mut().ok_or("runs")?;
    for run in runs.iter_mut() {
        let label = run["label"].as_str().ok_or("label")?;
        environment::verify(raw, label)?;
        let log = fs::read_to_string(raw.join(format!("{label}.stderr")))?;
        let chunks: Vec<Value> = log
            .lines()
            .filter_map(|s| s.strip_prefix("LFP5 "))
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        if mode == "plain" {
            need(chunks.is_empty(), "plain chunk timers")?;
            continue;
        }
        need(chunks.len() == 256, "chunk report count")?;
        let stages: Vec<Value> = log
            .lines()
            .filter_map(|s| s.strip_prefix("LF762 "))
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let factor = if run["arm"] == "base" { 2 } else { 4 };
        let derived: Vec<_> = chunks
            .iter()
            .enumerate()
            .map(|(i, row)| derive(row, i + 1, factor, integer(&stages[i]["stages_ns"][4])?))
            .collect::<Result<_>>()?;
        for (window, start, end) in [("all", 0, 256), ("entry", 0, 64), ("screen", 64, 256)] {
            let part = &derived[start..end];
            for (i, name) in METRICS.iter().enumerate() {
                run["windows"][window]["p5"][*name] =
                    stats(part.iter().map(|r| r.times[i]).collect());
            }
            run["windows"][window]["p5"]["participating_workers"] =
                counts(part.iter().map(|r| r.participating).collect());
            run["windows"][window]["p5"]["peak_interval_overlap"] =
                counts(part.iter().map(|r| r.overlap).collect());
            run["windows"][window]["p5"]["worker_elapsed"] = json!(
                (0..4)
                    .map(|worker| stats(part.iter().map(|r| r.worker_elapsed[worker]).collect()))
                    .collect::<Vec<_>>()
            );
        }
    }
    if mode == "plain" {
        output["plain_comparison"] = summarize_plain(runs)?;
    }
    output["timing_scope"] = json!(
        "chunk elapsed includes scheduling/preemption; sum is neither CPU time nor wall time; interval overlap is not CPU simultaneity; diagnostic-only"
    );
    if let Some(plain) = plain {
        compare_modes(&output, &plain)?;
        output["plain_reference"] = json!({"identity_sha256":io::sha(&plain_raw.ok_or("plain reference")?.join("identity.json"))?,
            "files_sha256":chunk_build::value_sha(&plain["files"])?,"collector_head":plain["identity"]["head"],
            "compared_plain_runs":12,"compared_detail_runs":6,
            "fields":["traffic","initial_counts","final_counts"],"matches":true});
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identical_drift_in_both_diagnostic_arms_is_rejected_against_plain() {
        let run = json!({"scale":"100k","traffic":{"ticks.jsonl":"t","commands.jsonl":"c","events.jsonl":"e"},
            "initial_counts":[75_000,25_000,0],"final_counts":[70_752,25_200,4_048]});
        let build = json!({"inherited_environment":{},"rustc":"rustc 1.98.0 host","cargo":"cargo 1.98.0","native_toolchain":crate::chunk_native::fixture(&std::env::temp_dir()),"collector":crate::chunk_collector::fixture(&std::env::temp_dir()),"command":{"environment":{}}});
        let sources = json!({"base":{"build":build},"candidate":{"build":build}});
        let plain = json!({"identity":{"mode":"plain","inputs":{"frozen":"input"},"sources":sources},"runs":vec![run.clone();12]});
        let detail = json!({"identity":{"mode":"detail","inputs":{"frozen":"input"},"sources":sources},"runs":vec![run;6]});
        compare_modes(&detail, &plain).unwrap();
        for field in ["traffic", "initial_counts", "final_counts"] {
            let mut bad = detail.clone();
            for run in bad["runs"].as_array_mut().unwrap() {
                run[field] = if field == "traffic" {
                    json!({"identical_change_in_both_arms":1})
                } else {
                    json!([1, 2, 3])
                };
            }
            assert!(compare_modes(&bad, &plain).is_err(), "{field}");
        }
        let mut bad = detail.clone();
        bad["identity"]["inputs"]["frozen"] = json!("other");
        assert!(compare_modes(&bad, &plain).is_err());
        assert!(compare_modes(&detail, &detail).is_err());
        let mut bad = detail.clone();
        for arm in ["base", "candidate"] {
            bad["identity"]["sources"][arm]["build"]["inherited_environment"]["CARGO_PROFILE_RELEASE_LTO"] =
                json!("true");
        }
        assert!(compare_modes(&bad, &plain).is_err());
    }
    fn batch() -> Value {
        json!({"tick":1,"workload":8_000,"chunk_size":1_000,"dispatch_ns":30,
            "chunks":(0..8).map(|i|vec![i*1_000,1_000,1+(i/4)*10,9+(i/4)*10,i%4,1]).collect::<Vec<_>>()})
    }
    #[test]
    fn valid_partition_and_elapsed_quantities() {
        let d = derive(&batch(), 1, 2, 40).unwrap();
        assert_eq!(d.times, [30, 8, 64, 1, 0, 11, 16, 16, 0]);
        assert_eq!(d.overlap, 4);
        assert_eq!(d.participating, 4);
    }
    #[test]
    fn corrupt_chunk_records_fail_closed() {
        for (field, value) in [
            ("tick", json!(0)),
            ("workload", json!(7_999)),
            ("chunk_size", json!(0)),
            ("dispatch_ns", json!(41)),
            ("chunks", json!([])),
        ] {
            let mut bad = batch();
            bad[field] = value;
            assert!(derive(&bad, 1, 2, 40).is_err(), "{field}");
        }
        for (index, value) in [
            (0, json!(1)),
            (1, json!(999)),
            (2, json!(-1)),
            (3, json!(31)),
            (4, json!(4)),
            (5, json!(0)),
        ] {
            let mut bad = batch();
            bad["chunks"][0][index] = value;
            assert!(derive(&bad, 1, 2, 40).is_err(), "{index}");
        }
        let mut bad = batch();
        bad["chunks"][4][2] = json!(8);
        assert!(derive(&bad, 1, 2, 40).is_err());
    }
    #[test]
    fn corrupt_stage_shapes_and_nesting_fail_closed() {
        let rows: Vec<_> = (1..=256)
            .map(|tick| {
                json!({"tick":tick,"step_ns":100,
            "stages_ns":vec![0;12],"calls":vec![1;12]})
            })
            .collect();
        let ticks: Vec<_> = (1..=256)
            .map(|tick| json!({"tick":tick,"N_active":8_000}))
            .collect();
        validate_stages(&rows, &ticks).unwrap();
        for (field, value) in [
            ("step_ns", json!(-1)),
            ("calls", json!(vec![1; 11])),
            ("stages_ns", json!([0, 0, 0, 0, 101, 0, 0, 0, 0, 0, 0, 0])),
        ] {
            let mut bad = rows.clone();
            bad[0][field] = value;
            assert!(validate_stages(&bad, &ticks).is_err());
        }
    }
    #[test]
    fn screening_requires_repeatable_gain_beyond_within_arm_span() {
        let mut runs = Vec::new();
        for scale in ["10k", "100k"] {
            for group in 0..3 {
                let arms = if group == 1 {
                    ["candidate", "base", "base", "candidate"]
                } else {
                    ["base", "candidate", "candidate", "base"]
                };
                for arm in arms {
                    let ms = if arm == "base" { 10.0 } else { 9.5 };
                    let mut run = json!({"arm":arm,"scale":scale,"windows":{}});
                    for window in ["all", "entry", "screen"] {
                        run["windows"][window]["step"] = json!({"mean_ms":ms,"p95_ms":ms});
                    }
                    runs.push(run);
                }
            }
        }
        assert_eq!(summarize_plain(&runs).unwrap()["adopt_candidate"], true);
        runs[0]["windows"]["screen"]["step"]["mean_ms"] = json!(11.0);
        assert_eq!(summarize_plain(&runs).unwrap()["adopt_candidate"], false);
        runs[0]["windows"]["screen"]["step"]["mean_ms"] = json!(10.0);
        runs[2]["windows"]["screen"]["step"]["mean_ms"] = json!(11.0);
        assert_eq!(summarize_plain(&runs).unwrap()["adopt_candidate"], false);
    }
}
