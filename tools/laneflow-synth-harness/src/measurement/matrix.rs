use std::{
    fs::{self, File},
    path::Path,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};

use serde_json::{Value, json};

use crate::{Error, require, write_json};

use super::runner::hash;

#[derive(Clone, Debug)]
struct Row {
    scale: u32,
    case: &'static str,
    dt: u32,
    percent: u8,
}

impl Row {
    fn input_label(&self) -> String {
        format!("{}-{}-{}", self.scale, self.case, self.dt)
    }
    fn label(&self) -> String {
        format!("{}-p{}", self.input_label(), self.percent)
    }
}

fn rows(stretch: bool) -> Vec<Row> {
    if stretch {
        return vec![Row {
            scale: 100_000,
            case: "W1",
            dt: 16,
            percent: 10,
        }];
    }
    let mut result = Vec::new();
    for (scale, dt) in [(10_000, 16), (100_000, 33)] {
        for case in ["W1", "W2", "W3", "W4"] {
            let percentages: &[u8] = match case {
                "W1" | "W3" => &[1, 10, 50, 100],
                "W2" => &[0],
                _ => {
                    if scale == 10_000 {
                        &[100]
                    } else {
                        &[10]
                    }
                }
            };
            result.extend(percentages.iter().map(|&percent| Row {
                scale,
                case,
                dt,
                percent,
            }));
        }
    }
    result
}

/// 生成同输入的完整窗口语义 oracle；最多四个进程并行，不采纳其计时。
///
/// # Errors
/// 创建输出、启动进程、输入验证、语义检查或读取回执失败时返回错误。
pub fn prepare_oracles(
    semantic_binary: &Path,
    input: &Path,
    implementation: &str,
    output: &Path,
) -> Result<(), Error> {
    fs::create_dir(output)?;
    let mut jobs = Vec::new();
    for row in rows(false).into_iter().chain(rows(true)) {
        let label = row.input_label();
        if !jobs.contains(&label) {
            jobs.push(label);
        }
    }
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel();
    let mut summary = json!({"complete":false,"scope":"semantic_oracles_only_not_latency","implementation_commit":implementation,
        "binary_sha256":hash(&fs::read(semantic_binary)?),"rows":[],"errors":[]});
    write_json(&output.join("summary.json"), &summary)?;
    std::thread::scope(|scope| -> Result<(), Error> {
        for _ in 0..4 {
            let (next, jobs, sender) = (&next, &jobs, sender.clone());
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(label) = jobs.get(index) else { break };
                    let result = (|| -> Result<Value, Error> {
                        let log = File::create(output.join(format!("{label}.log")))?;
                        let target = output.join(label);
                        let status = Command::new(semantic_binary)
                            .arg("run")
                            .arg(input.join(format!("{label}-a")))
                            .arg(implementation)
                            .arg(&target)
                            .stdout(Stdio::null())
                            .stderr(log)
                            .status()?;
                        require(status.success(), || {
                            format!("oracle 子进程失败 {label}: {status}")
                        })?;
                        let report: Value =
                            serde_json::from_slice(&fs::read(target.join("report.json"))?)?;
                        require(report["complete"] == true, || {
                            format!("oracle 未完成 {label}")
                        })?;
                        Ok(json!({"identity":label,"report":report}))
                    })();
                    if sender.send((label.clone(), result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        for (label, result) in receiver {
            match result {
                Ok(row) => summary["rows"].as_array_mut().unwrap().push(row),
                Err(e) => summary["errors"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"identity":label,"error":e.to_string()})),
            }
            write_json(&output.join("summary.json"), &summary)?;
            eprintln!(
                "oracle {label} complete ({}/{})",
                summary["rows"].as_array().unwrap().len(),
                jobs.len()
            );
        }
        Ok(())
    })?;
    require(
        summary["errors"].as_array().unwrap().is_empty()
            && summary["rows"].as_array().unwrap().len() == jobs.len(),
        || "oracle 矩阵不完整".into(),
    )?;
    summary["complete"] = json!(true);
    write_json(&output.join("summary.json"), &summary)
}

/// 串行启动正式三轮或独立 allocation 单轮；stretch 只覆盖十万 W1/16 ms/10%。
///
/// # Errors
/// 输出冲突、输入/子进程失败、身份或三轮语义不一致、I/O 失败时返回错误并保留已完成行。
pub fn matrix(
    input: &Path,
    oracle: &Path,
    implementation: &str,
    environment: &Path,
    output: &Path,
    stretch: bool,
) -> Result<(), Error> {
    fs::create_dir(output)?;
    let binary = std::env::current_exe()?;
    let rows = rows(stretch);
    let rounds = if cfg!(feature = "allocation") { 1 } else { 3 };
    let mut summary = json!({"format":"lf-synth-measurement-matrix-1","complete":false,"implementation_commit":implementation,
        "binary_sha256":hash(&fs::read(&binary)?),"instrumented":cfg!(feature="allocation"),"stretch_observation":stretch,
        "max_concurrent_measurements":1,"fresh_process_rounds":rounds,"execution_order":[],"runs":[],"aggregates":[],"errors":[]});
    write_json(&output.join("summary.json"), &summary)?;
    let mut reports = vec![Vec::new(); rows.len()];
    for round in 1..=rounds {
        for offset in 0..rows.len() {
            let index = (offset + (round - 1) * 7) % rows.len();
            let row = &rows[index];
            let label = format!("{}-r{round}", row.label());
            let target = output.join(&label);
            let log = File::create(output.join(format!("{label}.log")))?;
            let command = vec![
                "run".to_owned(),
                input
                    .join(format!("{}-a", row.input_label()))
                    .display()
                    .to_string(),
                oracle.join(row.input_label()).display().to_string(),
                implementation.to_owned(),
                environment.display().to_string(),
                row.percent.to_string(),
                round.to_string(),
                target.display().to_string(),
            ];
            summary["execution_order"]
                .as_array_mut()
                .unwrap()
                .push(json!({"label":label,"arguments":command}));
            write_json(&output.join("summary.json"), &summary)?;
            eprintln!(
                "measurement {label} start ({}/{})",
                summary["runs"].as_array().unwrap().len() + 1,
                rows.len() * rounds
            );
            let started = std::time::Instant::now();
            let status = Command::new(&binary)
                .args(&command)
                .stdout(Stdio::null())
                .stderr(log)
                .status()?;
            let elapsed = started.elapsed().as_secs_f64();
            if !status.success() {
                summary["errors"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"label":label,"exit":status.to_string(),"elapsed_s":elapsed}));
                write_json(&output.join("summary.json"), &summary)?;
                return Err(Error::Invalid(format!(
                    "正式运行失败 {label}；停止矩阵，保留失败目录"
                )));
            }
            let report: Value = serde_json::from_slice(&fs::read(target.join("report.json"))?)?;
            require(
                report["complete"] == true
                    && report["identity"]["scale"] == row.scale
                    && report["identity"]["case"] == row.case
                    && report["identity"]["fixed_delta_ms"] == row.dt
                    && report["presentation_percent"] == row.percent
                    && report["round"] == round
                    && report["implementation_commit"] == implementation
                    && report["instrumented"] == cfg!(feature = "allocation")
                    && report["manifest"]["binary_sha256"] == summary["binary_sha256"],
                || format!("子进程回执身份不符 {label}"),
            )?;
            if let Some(previous) = reports[index].first() {
                same_semantics(previous, &report)?;
            }
            let capacity = read_capacity_high_water(&output.join(format!("{label}.log")))?;
            summary["runs"].as_array_mut().unwrap().push(json!({"label":label,"elapsed_s":elapsed,
                "report_sha256":hash(&fs::read(target.join("report.json"))?),"statistics":report["statistics"],"capacity_high_water":capacity}));
            reports[index].push(report);
            if reports[index].len() == rounds {
                summary["aggregates"].as_array_mut().unwrap().push(json!({"row":row.label(),"rounds":rounds,
                    "statistics":aggregate(&reports[index])?,"semantics_equal_across_rounds":true}));
            }
            write_json(&output.join("summary.json"), &summary)?;
            eprintln!("measurement {label} complete {elapsed:.1}s");
        }
    }
    summary["complete"] = json!(true);
    write_json(&output.join("summary.json"), &summary)
}

fn same_semantics(a: &Value, b: &Value) -> Result<(), Error> {
    for key in [
        "identity",
        "checkpoints",
        "ordered_events_digest",
        "caller_commands",
        "blocked_commands",
        "completion_records",
        "arrival_records",
    ] {
        require(a[key] == b[key], || format!("三轮语义字段不同：{key}"))?;
    }
    for key in [
        "input_manifest",
        "outer_frame_sha256",
        "selection_sha256",
        "binary_sha256",
    ] {
        require(a["manifest"][key] == b["manifest"][key], || {
            format!("三轮输入不同：{key}")
        })?;
    }
    Ok(())
}

fn aggregate(reports: &[Value]) -> Result<Value, Error> {
    if cfg!(feature = "allocation") {
        return Ok(reports[0]["statistics"].clone());
    }
    require(reports.len() == 3, || "latency 聚合必须恰好三轮".into())?;
    fn merge(values: [&Value; 3], key: &str) -> Result<Value, Error> {
        if let Some(first) = values[0].as_object() {
            let mut result = serde_json::Map::new();
            for name in first.keys() {
                if name == "raw" {
                    continue;
                }
                result.insert(
                    name.clone(),
                    merge([&values[0][name], &values[1][name], &values[2][name]], name)?,
                );
            }
            return Ok(Value::Object(result));
        }
        if values.iter().all(|v| v.is_null()) {
            return Ok(Value::Null);
        }
        let mut ns = values
            .map(|v| v.as_u64())
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| Error::Invalid("聚合字段类型不同".into()))?;
        ns.sort_unstable();
        if key == "samples" {
            require(ns[0] == ns[2], || "三轮样本数不同".into())?;
            return Ok(json!(ns[0]));
        }
        if key == "max_ns" {
            return Ok(json!(ns[2]));
        }
        if ["core_ns", "spatial_adapter_ns", "integrated_ns"].contains(&key) {
            return Ok(json!({"median_ns":ns[1],"worst_ns":ns[2]}));
        }
        Ok(json!(ns[1]))
    }
    merge(
        [
            &reports[0]["statistics"],
            &reports[1]["statistics"],
            &reports[2]["statistics"],
        ],
        "",
    )
}

fn read_capacity_high_water(path: &Path) -> Result<Value, Error> {
    let mut result = serde_json::Map::new();
    for line in fs::read_to_string(path)?.lines() {
        if !line.starts_with("pose-adapter ") && !line.starts_with("pose-spatial ") {
            continue;
        }
        let prefix = line.split_whitespace().next().unwrap();
        for part in line.split_whitespace().skip(1) {
            if let Some((key, value)) = part.split_once('=')
                && (key.ends_with("_cap")
                    || key.ends_with("_size")
                    || key.ends_with("_len")
                    || key == "logical_copy_bytes")
                && let Ok(value) = value.parse::<u64>()
            {
                let key = format!("{prefix}.{key}");
                let old = result.get(&key).and_then(Value::as_u64).unwrap_or(0);
                result.insert(key, json!(old.max(value)));
            }
        }
    }
    Ok(
        json!({"method":"maximum_of_existing_pose_profiling_capacity_counters","fields":result,
        "scope":"capacity_is_retained_storage_not_process_heap_peak"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nominal_rows_and_round_rotation_cover_exactly_twenty_configs() {
        let nominal = rows(false);
        assert_eq!(nominal.len(), 20);
        assert_eq!(rows(true).len(), 1);
        for round in 0..3 {
            let mut indexes: Vec<_> = (0..20).map(|i| (i + round * 7) % 20).collect();
            indexes.sort_unstable();
            assert_eq!(indexes, (0..20).collect::<Vec<_>>());
        }
        assert!(
            nominal
                .iter()
                .all(|r| r.dt == if r.scale == 10_000 { 16 } else { 33 })
        );
    }
    #[cfg(not(feature = "allocation"))]
    #[test]
    fn aggregation_takes_round_medians_and_worst_max_not_pooled_percentiles() {
        let reports:Vec<_>=[(100,8),(1,200),(10,80)].into_iter().map(|(p,max)|json!({"statistics":{"core":{"samples":5,"p95_ns":p,"max_ns":max},"frame_groups":{}}})).collect();
        let result = aggregate(&reports).unwrap();
        assert_eq!(result["core"]["p95_ns"], 10);
        assert_eq!(result["core"]["max_ns"], 200);
        let burst: Vec<_> = [10, 30, 20]
            .into_iter()
            .map(|v| json!({"statistics":{"raw":[],"burst_max":{"core_ns":v}}}))
            .collect();
        let result = aggregate(&burst).unwrap();
        assert_eq!(
            result["burst_max"]["core_ns"],
            json!({"median_ns":20,"worst_ns":30})
        );
    }
}
