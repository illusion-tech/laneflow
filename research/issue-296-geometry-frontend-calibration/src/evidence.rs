//! §9.2 原始执行制品与紧凑证据装配：三进程样本文件 → raw artifact（来源/环境/协议
//! 绑定 + 逐进程逐样本记录）→ 证据（每进程中位数/MAD、中位数的中位数、预算校准写回）。
//! 证据必须能从 raw 独立重算并逐字节比对，且通过证据 schema 的 Draft 2020-12 校验。

use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::{Value, json};

use crate::container::sha256_hex;
use crate::counts::{ACCURACY_PROFILES, DIRECTION_PROFILES, accuracy_code, direction_code};
use crate::environment::environment_json;
use crate::manifest::{CORRIDOR_WORKLOAD_ID, MIN_WORKLOAD_ID, P100_WORKLOAD_ID};
use crate::measure::{BASE_LEVEL_IDS, FORMAL_SAMPLE_COUNT, LEVEL_IDS, PROCESS_COUNT};
use crate::validator::{CONTRACT_PATH, load_contract, read_bound_artifact};

/// raw 制品与证据共用的 schema 常量。
pub const RAW_EXECUTION_SCHEMA: &str = "laneflow.geometry-frontend-calibration-raw-execution";
pub const EVIDENCE_SCHEMA_NAME: &str = "laneflow.geometry-frontend-calibration-evidence";
pub const PROTOCOL_ID: &str = "geometry-frontend-calibration-v1";

/// §9.2 G1 冻结的三个候选预算（精确整数；closure 只接受 exact 数值与证据绑定）。
pub const FULL_COMPILE_BUDGET_NS: u64 = 25_000_000;
pub const COMPILER_CONTROLLED_PEAK_BUDGET_BYTES: u64 = 6_291_456;
pub const COLD_INSTANCE_RETAINED_BUDGET_BYTES: u64 = 0;

const BUDGET_SOURCE: &str = "#296 G1 候选预算（docs/design/geometry-document-frontend.md §9.2）";
const PROCESS_SAMPLES_SCHEMA: &str = "laneflow.geometry-frontend-calibration-process-samples";
const CLOCK_QUANTUM_OBSERVATIONS: usize = 100_000;

/// 奇数长度精确整数样本的中位数与 MAD（median-and-mad-of-exact-integers-v1）。
pub fn median_and_mad(samples: &[u64]) -> (u64, u64) {
    assert!(
        !samples.is_empty() && samples.len() % 2 == 1,
        "样本数必须为正奇数"
    );
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let median = sorted[sorted.len() / 2];
    let mut deviations: Vec<u64> = samples.iter().map(|v| v.abs_diff(median)).collect();
    deviations.sort_unstable();
    (median, deviations[deviations.len() / 2])
}

/// 计时量子：连续 `Instant::now()` 相邻差值的最小正差（100_000 次观察）。
pub fn observe_clock_quantum_ns() -> u64 {
    let mut minimum = u64::MAX;
    let mut previous = Instant::now();
    for _ in 0..CLOCK_QUANTUM_OBSERVATIONS {
        let current = Instant::now();
        let delta = u64::try_from(current.duration_since(previous).as_nanos()).unwrap_or(0);
        if delta > 0 {
            minimum = minimum.min(delta);
        }
        previous = current;
    }
    assert!(minimum != u64::MAX, "计时器未产生正差值");
    minimum
}

fn run_git(repo_root: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("启动 git 失败：{error}"));
    assert!(output.status.success(), "git {:?} 退出失败", args);
    String::from_utf8(output.stdout)
        .expect("git 输出必须是 UTF-8")
        .trim()
        .to_string()
}

fn source_commit(repo_root: &Path) -> String {
    let commit = run_git(repo_root, &["rev-parse", "HEAD"]);
    assert!(
        commit.len() == 40
            && commit
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "source commit 必须是 40 位小写十六进制：{commit}"
    );
    commit
}

/// 测量与装配必须发生在干净工作树（dirty:false 由本检查保证而非自报）。
fn verify_clean_worktree(repo_root: &Path) {
    let status = run_git(
        repo_root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    );
    assert!(status.is_empty(), "工作树必须干净：{status}");
}

fn sha256_file(path: &Path) -> (u64, String) {
    let bytes =
        std::fs::read(path).unwrap_or_else(|error| panic!("读取 {} 失败：{error}", path.display()));
    (
        u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        sha256_hex(&bytes),
    )
}

pub(crate) fn repo_relative(repo_root: &Path, path: &Path) -> String {
    let canonical_root = repo_root.canonicalize().expect("repo 根必须存在");
    let canonical = path
        .canonicalize()
        .unwrap_or_else(|error| panic!("路径 {} 必须存在：{error}", path.display()));
    canonical
        .strip_prefix(&canonical_root)
        .unwrap_or_else(|_| panic!("{} 必须位于 repo 根内", path.display()))
        .to_string_lossy()
        .replace('\\', "/")
}

/// 装配证据 `source` 对象：commit、干净树、Cargo.lock、contract 四件绑定与测量二进制。
fn source_json(repo_root: &Path, contract: &Value, binary: &Value) -> Value {
    let commit = source_commit(repo_root);
    verify_clean_worktree(repo_root);
    let (_, cargo_lock_sha256) = sha256_file(&repo_root.join("Cargo.lock"));
    let contract_bytes = std::fs::read(repo_root.join(CONTRACT_PATH)).expect("读取 contract 失败");
    let bound_sha256 = |key: &str| sha256_hex(&read_bound_artifact(repo_root, contract, key));
    json!({
        "measurementCommit": commit,
        "harnessCommit": commit,
        "dirty": false,
        "cargoLockSha256": cargo_lock_sha256,
        "contractDescriptorSha256": sha256_hex(&contract_bytes),
        "workloadManifestSha256": bound_sha256("workloadManifest"),
        "evidenceSchemaSha256": bound_sha256("evidenceSchema"),
        "referenceMachineDeclarationSha256": bound_sha256("referenceMachineDeclaration"),
        "releaseBinaries": [binary],
    })
}

/// 27 行冻结行序（workload 登记序 × 位置鉴别码 × 方向鉴别码）。
fn expected_row_order() -> Vec<(&'static str, u8, u8)> {
    let mut order = Vec::with_capacity(27);
    for workload_id in [MIN_WORKLOAD_ID, CORRIDOR_WORKLOAD_ID, P100_WORKLOAD_ID] {
        for accuracy in ACCURACY_PROFILES {
            for direction in DIRECTION_PROFILES {
                order.push((
                    workload_id,
                    accuracy_code(accuracy),
                    direction_code(direction),
                ));
            }
        }
    }
    order
}

fn row_u64(row: &Value, key: &str) -> u64 {
    row.get(key)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("行缺少 {key} 字段"))
}

fn row_string<'a>(row: &'a Value, key: &str) -> &'a str {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("行缺少 {key} 字段"))
}

fn level_samples(process_row: &Value, level: &str, base: bool) -> Vec<u64> {
    let holder = if base {
        process_row
            .get("syntheticBase")
            .and_then(|b| b.get("levels"))
            .expect("CORRIDOR 行缺少 syntheticBase.levels")
    } else {
        process_row.get("levels").expect("行缺少 levels")
    };
    let samples: Vec<u64> = holder
        .get(level)
        .and_then(|l| l.get("samplesNs"))
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("行缺少 {level}.samplesNs"))
        .iter()
        .map(|v| v.as_u64().expect("样本必须是 u64"))
        .collect();
    assert_eq!(
        samples.len(),
        FORMAL_SAMPLE_COUNT,
        "每级必须恰好 7 个正式样本"
    );
    samples
}

/// 从三进程样本计算一行的某级：每进程中位数/MAD + 中位数的中位数。
fn level_timing_json(processes: &[Value], row_index: usize, level: &str, base: bool) -> Value {
    let mut medians = Vec::with_capacity(PROCESS_COUNT as usize);
    let mut per_process = Vec::with_capacity(PROCESS_COUNT as usize);
    for process in processes {
        let row = &process
            .get("rows")
            .and_then(Value::as_array)
            .expect("进程缺少 rows")[row_index];
        let (median, mad) = median_and_mad(&level_samples(row, level, base));
        medians.push(median);
        per_process.push(json!({"medianNs": median, "madNs": mad}));
    }
    let (median_of_medians, _) = median_and_mad(&medians);
    json!({
        "perProcess": per_process,
        "medianOfMediansNs": median_of_medians,
    })
}

/// 从 raw 制品字节独立重算紧凑证据（assemble 与 verify 共用；结果必须逐字节稳定）。
pub fn build_evidence(raw_bytes: &[u8], raw_repo_relative_path: &str) -> String {
    let raw: Value = serde_json::from_slice(raw_bytes).expect("raw 制品必须是合法 JSON");
    assert_eq!(
        raw.get("schema").and_then(Value::as_str),
        Some(RAW_EXECUTION_SCHEMA),
        "raw 制品 schema 字段不匹配"
    );
    assert_eq!(
        raw.get("schemaVersion").and_then(Value::as_u64),
        Some(1),
        "不支持的 raw 制品 schemaVersion"
    );
    let processes = raw
        .get("processes")
        .and_then(Value::as_array)
        .expect("raw 缺少 processes 数组");
    assert_eq!(
        processes.len(),
        PROCESS_COUNT as usize,
        "raw 必须恰好含三个进程"
    );
    validate_processes(processes);
    let first_rows = processes[0]
        .get("rows")
        .and_then(Value::as_array)
        .expect("进程缺少 rows 数组");
    assert_eq!(first_rows.len(), 27, "raw 必须恰好含 27 行");

    let mut rows = Vec::with_capacity(27);
    for (row_index, first_row) in first_rows.iter().enumerate() {
        let workload_id = row_string(first_row, "workloadId");
        let mut levels = serde_json::Map::new();
        for level in LEVEL_IDS {
            levels.insert(
                level.to_string(),
                level_timing_json(processes, row_index, level, false),
            );
        }
        let mut row = json!({
            "workloadId": workload_id,
            "accuracyProfileCode": row_u64(first_row, "accuracyProfileCode"),
            "directionProfileCode": row_u64(first_row, "directionProfileCode"),
            "levels": Value::Object(levels),
            "compilerControlledPeakBytes": row_u64(first_row, "compilerControlledPeakBytes"),
            "compilerRetainedCapacityBytes": row_u64(first_row, "compilerRetainedCapacityBytes"),
            "semanticFingerprint": row_string(first_row, "semanticFingerprint"),
            "completeOutputDigest": row_string(first_row, "completeOutputDigest"),
        });
        if workload_id == CORRIDOR_WORKLOAD_ID {
            let mut base_levels = serde_json::Map::new();
            for level in BASE_LEVEL_IDS {
                base_levels.insert(
                    level.to_string(),
                    level_timing_json(processes, row_index, level, true),
                );
            }
            row.as_object_mut().expect("行必须是对象").insert(
                "syntheticBase".to_string(),
                json!({"levels": Value::Object(base_levels)}),
            );
        }
        rows.push(row);
    }

    let observed_wall_clock = rows
        .iter()
        .map(|row| {
            row["levels"]["fullCompile"]["medianOfMediansNs"]
                .as_u64()
                .expect("行缺少中位数的中位数")
        })
        .max()
        .expect("27 行非空");
    let observed_peak = rows
        .iter()
        .map(|row| {
            row["compilerControlledPeakBytes"]
                .as_u64()
                .expect("行缺少峰值")
        })
        .max()
        .expect("27 行非空");
    let observed_retained = rows
        .iter()
        .map(|row| {
            row["compilerRetainedCapacityBytes"]
                .as_u64()
                .expect("行缺少保留容量")
        })
        .max()
        .expect("27 行非空");
    let workload_ids: Vec<&str> =
        [MIN_WORKLOAD_ID, CORRIDOR_WORKLOAD_ID, P100_WORKLOAD_ID].to_vec();
    let budget_entry = |unit: &str, candidate: u64, statistic: &str, observed: u64| {
        json!({
            "unit": unit,
            "candidateBudget": candidate,
            "calibratedBudget": candidate,
            "statistic": statistic,
            "observedValue": observed,
            "supported": observed <= candidate,
            "budgetSource": BUDGET_SOURCE,
            "appliesTo": {
                "workloadIds": workload_ids,
                "accuracyProfileCodes": [1, 2, 3],
                "directionProfileCodes": [1, 2, 3],
            },
        })
    };

    let evidence = json!({
        "schema": EVIDENCE_SCHEMA_NAME,
        "schemaVersion": 1,
        "rawExecution": {
            "schema": RAW_EXECUTION_SCHEMA,
            "schemaVersion": 1,
            "path": raw_repo_relative_path,
            "byteLength": u64::try_from(raw_bytes.len()).unwrap_or(u64::MAX),
            "sha256": sha256_hex(raw_bytes),
        },
        "source": raw.get("source").expect("raw 缺少 source").clone(),
        "environment": raw.get("environment").expect("raw 缺少 environment").clone(),
        "protocol": raw.get("protocol").expect("raw 缺少 protocol").clone(),
        "rows": rows,
        "budgetCalibration": {
            "fullCompileWallClock": budget_entry(
                "ns",
                FULL_COMPILE_BUDGET_NS,
                "perRowMedianOfMediansMax",
                observed_wall_clock,
            ),
            "compilerControlledPeak": budget_entry(
                "bytes",
                COMPILER_CONTROLLED_PEAK_BUDGET_BYTES,
                "perRowExactMax",
                observed_peak,
            ),
            "coldInstanceRetainedCapacity": budget_entry(
                "bytes",
                COLD_INSTANCE_RETAINED_BUDGET_BYTES,
                "perRowExactMax",
                observed_retained,
            ),
        },
    });
    serde_json::to_string_pretty(&evidence).expect("证据序列化")
}

/// 以证据 schema 对证据字节做 Draft 2020-12 校验；任何违规即 panic。
pub fn validate_evidence_document(schema_bytes: &[u8], evidence_bytes: &[u8]) {
    let schema: Value = serde_json::from_slice(schema_bytes).expect("证据 schema 必须是合法 JSON");
    let instance: Value = serde_json::from_slice(evidence_bytes).expect("证据必须是合法 JSON");
    jsonschema::draft202012::validate(&schema, &instance)
        .unwrap_or_else(|error| panic!("证据违反 Draft 2020-12 schema：{error}"));
}

/// 校验三份进程样本文件的结构性一致（schema、进程序号、二进制、冻结行序与跨进程
/// 确定性字段），返回排序后的进程对象与测量二进制绑定。
fn validate_processes(processes: &[Value]) -> Value {
    assert_eq!(
        processes.len(),
        PROCESS_COUNT as usize,
        "必须恰好包含三个进程"
    );
    for process in processes {
        assert_eq!(
            process.get("schema").and_then(Value::as_str),
            Some(PROCESS_SAMPLES_SCHEMA),
            "进程样本 schema 字段不匹配"
        );
        assert_eq!(
            process.get("schemaVersion").and_then(Value::as_u64),
            Some(1),
            "不支持的进程样本 schemaVersion"
        );
    }
    for (expected, process) in (1..=u64::from(PROCESS_COUNT)).zip(processes) {
        assert_eq!(
            row_u64(process, "processIndex"),
            expected,
            "进程序号必须恰好为 1/2/3"
        );
    }
    let binary = processes[0].get("binary").expect("进程缺少 binary").clone();
    for process in &processes[1..] {
        assert_eq!(
            process.get("binary"),
            Some(&binary),
            "三进程必须使用同一测量二进制"
        );
    }

    let order = expected_row_order();
    for (row_index, (workload_id, accuracy, direction)) in order.iter().enumerate() {
        let mut reference: Option<&Value> = None;
        for process in processes {
            let rows = process
                .get("rows")
                .and_then(Value::as_array)
                .expect("进程缺少 rows 数组");
            assert_eq!(rows.len(), 27, "每进程必须恰好 27 行");
            let row = &rows[row_index];
            assert_eq!(row_string(row, "workloadId"), *workload_id, "行序不符");
            assert_eq!(row_u64(row, "accuracyProfileCode"), u64::from(*accuracy));
            assert_eq!(row_u64(row, "directionProfileCode"), u64::from(*direction));
            for level in LEVEL_IDS {
                level_samples(row, level, false);
            }
            if *workload_id == CORRIDOR_WORKLOAD_ID {
                for level in BASE_LEVEL_IDS {
                    level_samples(row, level, true);
                }
            } else {
                assert!(
                    row.get("syntheticBase").is_none(),
                    "MIN/P100 行不得携带 syntheticBase"
                );
            }
            match reference {
                None => reference = Some(row),
                Some(expected_row) => {
                    for key in [
                        "compilerControlledPeakBytes",
                        "compilerRetainedCapacityBytes",
                        "semanticFingerprint",
                        "completeOutputDigest",
                    ] {
                        assert_eq!(
                            row.get(key),
                            expected_row.get(key),
                            "跨进程确定性字段 {key} 漂移"
                        );
                    }
                }
            }
        }
    }
    binary
}

fn load_process_files(repo_root: &Path, paths: &[PathBuf]) -> (Vec<Value>, Value) {
    assert_eq!(
        paths.len(),
        PROCESS_COUNT as usize,
        "必须恰好提供三份进程样本文件"
    );
    let mut processes = Vec::with_capacity(PROCESS_COUNT as usize);
    for path in paths {
        let bytes = std::fs::read(path)
            .unwrap_or_else(|error| panic!("读取进程样本 {} 失败：{error}", path.display()));
        let value: Value = serde_json::from_slice(&bytes).expect("进程样本必须是合法 JSON");
        processes.push(value);
    }
    processes.sort_by_key(|p| row_u64(p, "processIndex"));
    let binary = validate_processes(&processes);
    let executable = std::env::current_exe().expect("读取当前可执行路径失败");
    let (executable_length, executable_sha256) = sha256_file(&executable);
    assert_eq!(
        row_string(&binary, "path"),
        repo_relative(repo_root, &executable),
        "assemble 与 measure 必须使用同一 repo 相对二进制路径"
    );
    assert_eq!(
        row_u64(&binary, "byteLength"),
        executable_length,
        "assemble 与 measure 必须是同一二进制"
    );
    assert_eq!(
        row_string(&binary, "sha256"),
        executable_sha256,
        "assemble 与 measure 必须是同一二进制"
    );

    (processes, binary)
}

/// 正式装配：三进程样本 → raw artifact（写盘）→ 证据（Draft 2020-12 校验后写盘）。
/// 必须 release 构建且在干净工作树运行；打印两件制品的字节身份。
pub fn assemble(
    repo_root: &Path,
    process_files: &[PathBuf],
    raw_output: &Path,
    evidence_output: &Path,
) {
    if cfg!(debug_assertions) {
        panic!("正式 assemble 必须 release 构建");
    }
    let (processes, binary) = load_process_files(repo_root, process_files);
    let contract = load_contract(repo_root);
    let source = source_json(repo_root, &contract, &binary);
    let environment = environment_json(repo_root);
    let protocol = json!({
        "id": PROTOCOL_ID,
        "releaseBuild": true,
        "singleWorkerThread": true,
        "clockQuantumNs": observe_clock_quantum_ns(),
        "processCount": PROCESS_COUNT,
        "warmupCountPerLevel": 1,
        "formalSampleCountPerLevel": FORMAL_SAMPLE_COUNT,
        "levels": LEVEL_IDS,
    });
    let raw = json!({
        "schema": RAW_EXECUTION_SCHEMA,
        "schemaVersion": 1,
        "source": source,
        "environment": environment,
        "protocol": protocol,
        "processes": processes,
    });
    let raw_text = serde_json::to_string_pretty(&raw).expect("raw 序列化");
    std::fs::write(raw_output, &raw_text)
        .unwrap_or_else(|error| panic!("写 raw 制品 {} 失败：{error}", raw_output.display()));

    let raw_relative = repo_relative(repo_root, raw_output);
    let evidence_text = build_evidence(raw_text.as_bytes(), &raw_relative);
    let evidence_schema_bytes = read_bound_artifact(repo_root, &contract, "evidenceSchema");
    validate_evidence_document(&evidence_schema_bytes, evidence_text.as_bytes());
    std::fs::write(evidence_output, &evidence_text)
        .unwrap_or_else(|error| panic!("写证据 {} 失败：{error}", evidence_output.display()));

    println!(
        "{raw_relative}\t{}\t{}",
        raw_text.len(),
        sha256_hex(raw_text.as_bytes())
    );
    let evidence_relative = repo_relative(repo_root, evidence_output);
    println!(
        "{evidence_relative}\t{}\t{}",
        evidence_text.len(),
        sha256_hex(evidence_text.as_bytes())
    );
    eprintln!("raw 与证据装配完成，证据通过 Draft 2020-12 校验");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn committed_raw() -> Value {
        serde_json::from_slice(include_bytes!(
            "../../../docs/reference/geometry-frontend-calibration-raw-execution-v1.json"
        ))
        .unwrap()
    }

    #[test]
    fn raw_rebuild_rejects_reordered_process_rows() {
        let mut raw = committed_raw();
        let rows = raw["processes"][1]["rows"].as_array_mut().unwrap();
        rows.swap(0, 1);
        let bytes = serde_json::to_vec(&raw).unwrap();
        assert!(
            std::panic::catch_unwind(|| build_evidence(&bytes, "raw.json")).is_err(),
            "独立重算不得把不同 workload 的时序按下标混合"
        );
    }

    #[test]
    fn raw_rebuild_rejects_cross_process_deterministic_drift() {
        let mut raw = committed_raw();
        raw["processes"][2]["rows"][0]["compilerControlledPeakBytes"] = json!(u64::MAX);
        let bytes = serde_json::to_vec(&raw).unwrap();
        assert!(
            std::panic::catch_unwind(|| build_evidence(&bytes, "raw.json")).is_err(),
            "独立重算必须复核每个进程的确定性字段"
        );
    }
}
