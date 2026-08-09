//! §9.2 cross-record validator（manifest 方向）：trusted contract → 四件工件 exact
//! bytes → Draft 2020-12 → 独立 oracle 重编译逐字段比对。manifest 自报值不作 oracle：
//! 全部计数、绝对偏移分布、53 表行数与两个 digest 都由本 validator 从冻结 fixture
//! 重新编译得出；evidence 方向的校验在测量证据存在后启用，复用同一条校验链。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use laneflow_compiler::{GeometryAccuracyProfile, GeometryDirectionProfile, LirTableCounts};
use serde_json::Value;

use crate::container::{decode_container, sha256_hex};
use crate::counts::{
    ACCURACY_PROFILES, DIRECTION_PROFILES, GeometrySource, accuracy_code,
    compile_geometry_workload, complete_output_digest, direction_code,
};
use crate::evidence::{
    self, COLD_INSTANCE_RETAINED_BUDGET_BYTES, COMPILER_CONTROLLED_PEAK_BUDGET_BYTES,
    FULL_COMPILE_BUDGET_NS, RAW_EXECUTION_SCHEMA,
};
use crate::manifest::{
    self, CORRIDOR_FIXTURE_PATH, CORRIDOR_WORKLOAD_ID, MIN_FIXTURE_PATH, MIN_WORKLOAD_ID,
    P100_FIXTURE_PATH, P100_WORKLOAD_ID,
};
use crate::measure::{BASE_LEVEL_IDS, LEVEL_IDS};

/// trusted contract 描述符的 repo 相对路径（整条校验链的起点）。
pub const CONTRACT_PATH: &str = "docs/reference/geometry-frontend-calibration-contract-v1.json";

const CONTRACT_SCHEMA: &str = "laneflow.geometry-frontend-calibration-contract";
const DRAFT_2020_12_META: &str = "https://json-schema.org/draft/2020-12/schema";
const REFERENCE_MACHINE_SCHEMA: &str = "laneflow.geometry-frontend-calibration-reference-machine";

/// 读取并解析 trusted contract，核对自报 schema / schemaVersion（整条校验链的起点）。
pub fn load_contract(repo_root: &Path) -> Value {
    let contract_bytes = std::fs::read(repo_root.join(CONTRACT_PATH)).expect("读取 contract 失败");
    let contract: Value =
        serde_json::from_slice(&contract_bytes).expect("contract 必须是合法 JSON");
    assert_eq!(
        contract.get("schema").and_then(Value::as_str),
        Some(CONTRACT_SCHEMA),
        "contract schema 字段不匹配"
    );
    assert_eq!(
        contract.get("schemaVersion").and_then(Value::as_u64),
        Some(1),
        "不支持的 contract schemaVersion"
    );
    contract
}

/// contract 中一个工件的绑定对象（含 path/byteLength/sha256 字段）。
pub fn contract_artifact<'a>(contract: &'a Value, key: &str) -> &'a Value {
    contract
        .get(key)
        .unwrap_or_else(|| panic!("contract 缺少 {key} 条目"))
}

/// contract 中一个工件的 exact-bytes 绑定（path + byteLength + SHA-256）。
struct ArtifactBinding {
    path: String,
    byte_length: u64,
    sha256: String,
}

/// 从 contract 绑定对象读取一个工件绑定；字段缺失或类型不符即 panic（contract 必须完整）。
fn artifact_binding(entry: &Value, key: &str) -> ArtifactBinding {
    ArtifactBinding {
        path: entry
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("contract {key} 缺少 path 字段"))
            .to_string(),
        byte_length: entry
            .get("byteLength")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| panic!("contract {key} 缺少 byteLength 字段")),
        sha256: entry
            .get("sha256")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("contract {key} 缺少 sha256 字段"))
            .to_string(),
    }
}

/// 从 repo 根读取 contract 绑定工件的 exact bytes 并核对（字节长度 + SHA-256）。
pub fn read_bound_artifact(repo_root: &Path, contract: &Value, key: &str) -> Vec<u8> {
    let binding = artifact_binding(contract_artifact(contract, key), key);
    let bytes = std::fs::read(repo_root.join(&binding.path))
        .unwrap_or_else(|error| panic!("读取工件 {} 失败：{error}", binding.path));
    let actual_length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    assert_eq!(
        actual_length, binding.byte_length,
        "工件 {} 字节长度与 contract 绑定不符",
        binding.path
    );
    let actual_sha256 = sha256_hex(&bytes);
    assert_eq!(
        actual_sha256, binding.sha256,
        "工件 {} SHA-256 与 contract 绑定不符",
        binding.path
    );
    bytes
}

/// 执行完整 manifest 方向校验链；任何一步失败即 panic，全部通过时打印四件工件字节身份。
pub fn validate_manifest_with_contract(repo_root: &Path) {
    // 1. trusted contract：解析并核对自报 schema / schemaVersion。
    let contract = load_contract(repo_root);

    // 2. 四件工件 exact bytes：证据 schema、manifest schema、manifest、参考机声明。
    let evidence_schema_bytes = read_bound_artifact(repo_root, &contract, "evidenceSchema");
    let manifest_schema_bytes = read_bound_artifact(repo_root, &contract, "workloadManifestSchema");
    let manifest_bytes = read_bound_artifact(repo_root, &contract, "workloadManifest");
    let reference_machine_bytes =
        read_bound_artifact(repo_root, &contract, "referenceMachineDeclaration");

    // 证据 schema 与参考机声明在本切片只做结构 smoke；evidence 方向校验随测量证据启用。
    let evidence_schema: Value =
        serde_json::from_slice(&evidence_schema_bytes).expect("evidence schema 必须是合法 JSON");
    assert_eq!(
        evidence_schema.get("$schema").and_then(Value::as_str),
        Some(DRAFT_2020_12_META),
        "evidence schema 必须声明 Draft 2020-12"
    );
    jsonschema::validator_for(&evidence_schema).expect("evidence schema 必须可编译");
    let reference_machine: Value =
        serde_json::from_slice(&reference_machine_bytes).expect("参考机声明必须是合法 JSON");
    assert_eq!(
        reference_machine.get("schema").and_then(Value::as_str),
        Some(REFERENCE_MACHINE_SCHEMA),
        "参考机声明 schema 字段不匹配"
    );
    assert_eq!(
        reference_machine
            .get("schemaVersion")
            .and_then(Value::as_u64),
        Some(1),
        "不支持的参考机声明 schemaVersion"
    );

    // 3. Draft 2020-12 校验 manifest（exact schema bytes，不信任任何自报路径）。
    manifest::validate_manifest(&manifest_schema_bytes, &manifest_bytes);

    // 4. cross-record：行覆盖 → fixture 绑定 → 自洽性 → oracle 重编译。
    let manifest_json: Value =
        serde_json::from_slice(&manifest_bytes).expect("manifest 必须是合法 JSON");
    let rows = manifest_json
        .get("rows")
        .and_then(Value::as_array)
        .expect("manifest 缺少 rows 数组");
    assert_eq!(rows.len(), 27, "manifest 必须恰好 27 行");
    let workloads = [
        (MIN_WORKLOAD_ID, MIN_FIXTURE_PATH),
        (CORRIDOR_WORKLOAD_ID, CORRIDOR_FIXTURE_PATH),
        (P100_WORKLOAD_ID, P100_FIXTURE_PATH),
    ];
    let mut row_index = 0_usize;
    for (workload_id, fixture_path) in workloads {
        let fixture_bytes = std::fs::read(repo_root.join(fixture_path))
            .unwrap_or_else(|error| panic!("读取 fixture {fixture_path} 失败：{error}"));
        check_fixture_binding(rows, row_index, &fixture_bytes, fixture_path);
        let (_, modules) = decode_container(&fixture_bytes);
        let sources: Vec<GeometrySource<'_>> = modules
            .iter()
            .map(|module| GeometrySource {
                namespace: workload_id,
                document_key: &module.source_path,
                source: module.source.as_bytes(),
            })
            .collect();
        for accuracy in ACCURACY_PROFILES {
            for direction in DIRECTION_PROFILES {
                let row = &rows[row_index];
                let label = format!(
                    "{workload_id} 位置{} 方向{}",
                    accuracy_code(accuracy),
                    direction_code(direction)
                );
                check_row_identity(row, &label, workload_id, accuracy, direction);
                check_row_self_consistency(row, &label);
                check_row_against_oracle(row, &label, &sources, accuracy, direction);
                row_index += 1;
            }
        }
    }
    assert_eq!(row_index, 27, "cross-record 必须消费恰好 27 行");

    // 5. 打印四件工件字节身份与结论。
    for key in [
        "evidenceSchema",
        "workloadManifestSchema",
        "workloadManifest",
        "referenceMachineDeclaration",
    ] {
        let binding = artifact_binding(contract_artifact(&contract, key), key);
        println!(
            "{}\t{}\t{}",
            binding.path, binding.byte_length, binding.sha256
        );
    }
    eprintln!("cross-record 验证通过：27 行与独立重编译 oracle 逐字段一致");
}

/// ② 同一 workload 九行 fixture 绑定逐字段相同，且与磁盘上冻结 fixture 字节一致。
fn check_fixture_binding(rows: &[Value], start: usize, fixture_bytes: &[u8], fixture_path: &str) {
    let first = rows[start].get("fixture").expect("行缺少 fixture 绑定");
    let expected_length = u64::try_from(fixture_bytes.len()).unwrap_or(u64::MAX);
    let expected_sha256 = sha256_hex(fixture_bytes);
    assert_eq!(
        first.get("path").and_then(Value::as_str),
        Some(fixture_path),
        "fixture 绑定路径与登记不符"
    );
    assert_eq!(
        first.get("byteLength").and_then(Value::as_u64),
        Some(expected_length),
        "fixture 绑定字节长度与磁盘字节不符"
    );
    assert_eq!(
        first.get("sha256").and_then(Value::as_str),
        Some(expected_sha256.as_str()),
        "fixture 绑定 SHA-256 与磁盘字节不符"
    );
    for offset in 1..9_usize {
        let other = rows[start + offset]
            .get("fixture")
            .expect("行缺少 fixture 绑定");
        assert_eq!(
            other, first,
            "同一 workload 九行 fixture 绑定必须逐字段相同"
        );
    }
}

/// ① 行身份：恰好 3×3×3 唯一行且按冻结行序（workload × 位置鉴别码 × 方向鉴别码）。
fn check_row_identity(
    row: &Value,
    label: &str,
    workload_id: &str,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
) {
    assert_eq!(
        row.get("workloadId").and_then(Value::as_str),
        Some(workload_id),
        "{label} workloadId 与冻结行序不符"
    );
    assert_eq!(
        row.get("accuracyProfileCode").and_then(Value::as_u64),
        Some(u64::from(accuracy_code(accuracy))),
        "{label} accuracyProfileCode 与冻结行序不符"
    );
    assert_eq!(
        row.get("directionProfileCode").and_then(Value::as_u64),
        Some(u64::from(direction_code(direction))),
        "{label} directionProfileCode 与冻结行序不符"
    );
}

/// ③④⑤ 行内自洽：分布桶合计 == offsetCurveCount、53 表合计 == lirRecordCount、
/// lirTableCounts 恰好登记编译器 v1 registry 的 53 个表名（不多不少）。
fn check_row_self_consistency(row: &Value, label: &str) {
    let counts = row.get("counts").expect("行缺少 counts 对象");
    let distribution_sum: u64 = row
        .get("absoluteOffsetDistribution")
        .and_then(Value::as_array)
        .expect("行缺少 absoluteOffsetDistribution 数组")
        .iter()
        .map(|entry| {
            entry
                .get("curveCount")
                .and_then(Value::as_u64)
                .expect("分布桶缺少 curveCount")
        })
        .sum();
    assert_eq!(
        distribution_sum,
        counts
            .get("offsetCurveCount")
            .and_then(Value::as_u64)
            .expect("counts 缺少 offsetCurveCount"),
        "{label} 分布 curveCount 总和必须等于 offsetCurveCount"
    );
    let tables = row
        .get("lirTableCounts")
        .and_then(Value::as_object)
        .expect("行缺少 lirTableCounts 对象");
    let table_sum: u64 = tables
        .values()
        .map(|count| count.as_u64().expect("表计数必须是 u64"))
        .sum();
    assert_eq!(
        table_sum,
        counts
            .get("lirRecordCount")
            .and_then(Value::as_u64)
            .expect("counts 缺少 lirRecordCount"),
        "{label} 53 表行数总和必须等于 lirRecordCount"
    );
    let registered: BTreeSet<&str> = LirTableCounts::NAMES.iter().copied().collect();
    let actual: BTreeSet<&str> = tables.keys().map(String::as_str).collect();
    assert_eq!(
        actual, registered,
        "{label} lirTableCounts 必须恰好登记 53 个 record-counted 表名"
    );
}

/// ⑥ oracle：从冻结 fixture 重新编译该组合，逐字段比对 12 项计数、绝对偏移分布、
/// 53 表行数与两个 digest；manifest 自报值不作 oracle。
fn check_row_against_oracle(
    row: &Value,
    label: &str,
    sources: &[GeometrySource<'_>],
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
) {
    let (output, counts) = compile_geometry_workload(sources, accuracy, direction);
    let counts_json = row.get("counts").expect("行缺少 counts 对象");
    let expected_counts: [(&str, u64); 12] = [
        ("moduleCount", counts.module_count),
        ("documentCount", counts.document_count),
        ("declarationCount", counts.declaration_count),
        ("referenceCount", counts.reference_count),
        ("relationOccurrenceCount", counts.relation_occurrence_count),
        ("lineSegmentCount", counts.line_segment_count),
        ("cubicSegmentCount", counts.cubic_segment_count),
        ("controlPointCount", counts.control_point_count),
        ("offsetCurveCount", counts.offset_curve_count),
        ("canonicalPointCount", counts.canonical_point_count),
        ("lirRecordCount", counts.lir_record_count),
        ("logicalOutputBytes", counts.logical_output_bytes),
    ];
    for (key, expected) in expected_counts {
        assert_eq!(
            counts_json.get(key).and_then(Value::as_u64),
            Some(expected),
            "{label} counts.{key} 与 oracle 重编译不符"
        );
    }

    let mut distribution = BTreeMap::new();
    for entry in row
        .get("absoluteOffsetDistribution")
        .and_then(Value::as_array)
        .expect("行缺少 absoluteOffsetDistribution 数组")
    {
        let bits_hex = entry
            .get("absoluteOffsetMetersBits")
            .and_then(Value::as_str)
            .expect("分布桶缺少 absoluteOffsetMetersBits");
        let bits = u64::from_str_radix(
            bits_hex.strip_prefix("0x").expect("位模式必须带 0x 前缀"),
            16,
        )
        .expect("位模式必须是 16 位十六进制");
        let curve_count = entry
            .get("curveCount")
            .and_then(Value::as_u64)
            .expect("分布桶缺少 curveCount");
        assert!(
            distribution.insert(bits, curve_count).is_none(),
            "{label} 分布存在重复位模式桶"
        );
    }
    assert_eq!(
        distribution, counts.absolute_offset_distribution,
        "{label} 绝对偏移分布与 oracle 重编译不符"
    );

    let tables_json = row
        .get("lirTableCounts")
        .and_then(Value::as_object)
        .expect("行缺少 lirTableCounts 对象");
    let tables = counts
        .lir_table_counts
        .as_ref()
        .expect("编译成功后 lir_table_counts 恒为 Some");
    for (name, expected) in tables.entries() {
        assert_eq!(
            tables_json.get(name).and_then(Value::as_u64),
            Some(expected),
            "{label} lirTableCounts.{name} 与 oracle 重编译不符"
        );
    }

    assert_eq!(
        row.get("semanticFingerprint").and_then(Value::as_str),
        Some(manifest::hex_32(&counts.semantic_fingerprint).as_str()),
        "{label} semanticFingerprint 与 oracle 重编译不符"
    );
    assert_eq!(
        row.get("completeOutputDigest").and_then(Value::as_str),
        Some(manifest::hex_32(&complete_output_digest(&output)).as_str()),
        "{label} completeOutputDigest 与 oracle 重编译不符"
    );
}

/// evidence 制品的 repo 相对路径（§9.2 测量证据写回位置）。
pub const EVIDENCE_PATH: &str = "docs/reference/geometry-frontend-calibration-evidence-v1.json";

/// 执行 evidence 方向校验链：先完成 manifest 方向全链（contract → 四件 exact bytes →
/// Draft 2020-12 → oracle cross-record），再 Draft 2020-12 校验证据，最后 cross-record
/// （raw 绑定 / 来源 / 环境 / 27 行 / 预算校准）并从 raw 独立重算逐字节比对。
pub fn validate_evidence_with_contract(repo_root: &Path) {
    // 1. manifest 方向全链（§9.2：禁止先信任 evidence 自报的 manifest）。
    validate_manifest_with_contract(repo_root);
    let contract = load_contract(repo_root);
    let evidence_schema_bytes = read_bound_artifact(repo_root, &contract, "evidenceSchema");
    let manifest_bytes = read_bound_artifact(repo_root, &contract, "workloadManifest");
    let reference_machine_bytes =
        read_bound_artifact(repo_root, &contract, "referenceMachineDeclaration");

    // 2. evidence Draft 2020-12 校验。
    let evidence_bytes = std::fs::read(repo_root.join(EVIDENCE_PATH)).expect("读取证据失败");
    evidence::validate_evidence_document(&evidence_schema_bytes, &evidence_bytes);
    let evidence: Value = serde_json::from_slice(&evidence_bytes).expect("证据必须是合法 JSON");

    // 3. rawExecution 绑定：磁盘 raw 制品字节 == 证据登记的 byteLength + SHA-256。
    let raw_execution = evidence.get("rawExecution").expect("证据缺少 rawExecution");
    assert_eq!(
        raw_execution.get("schema").and_then(Value::as_str),
        Some(RAW_EXECUTION_SCHEMA),
        "rawExecution schema 字段不匹配"
    );
    assert_eq!(
        raw_execution.get("schemaVersion").and_then(Value::as_u64),
        Some(1),
        "不支持的 rawExecution schemaVersion"
    );
    let raw_path = raw_execution
        .get("path")
        .and_then(Value::as_str)
        .expect("rawExecution 缺少 path");
    let raw_bytes = std::fs::read(repo_root.join(raw_path)).expect("读取 raw 制品失败");
    assert_eq!(
        raw_execution.get("byteLength").and_then(Value::as_u64),
        Some(u64::try_from(raw_bytes.len()).unwrap_or(u64::MAX)),
        "raw 制品字节长度与证据登记不符"
    );
    assert_eq!(
        raw_execution.get("sha256").and_then(Value::as_str),
        Some(sha256_hex(&raw_bytes).as_str()),
        "raw 制品 SHA-256 与证据登记不符"
    );

    // 4. source 绑定：contract 描述符、manifest、证据 schema、参考机声明与 Cargo.lock
    // 的 SHA-256 必须与磁盘字节一致；release 二进制逐个核对 exact bytes。
    let source = evidence.get("source").expect("证据缺少 source");
    let contract_bytes = std::fs::read(repo_root.join(CONTRACT_PATH)).expect("读取 contract 失败");
    let expect_source_sha256 = |key: &str, expected: String| {
        assert_eq!(
            source.get(key).and_then(Value::as_str),
            Some(expected.as_str()),
            "source.{key} 与磁盘字节不符"
        );
    };
    expect_source_sha256("contractDescriptorSha256", sha256_hex(&contract_bytes));
    expect_source_sha256("workloadManifestSha256", sha256_hex(&manifest_bytes));
    expect_source_sha256("evidenceSchemaSha256", sha256_hex(&evidence_schema_bytes));
    expect_source_sha256(
        "referenceMachineDeclarationSha256",
        sha256_hex(&reference_machine_bytes),
    );
    let lock_bytes = std::fs::read(repo_root.join("Cargo.lock")).expect("读取 Cargo.lock 失败");
    expect_source_sha256("cargoLockSha256", sha256_hex(&lock_bytes));
    assert_eq!(
        source.get("measurementCommit"),
        source.get("harnessCommit"),
        "measurement/harness commit 必须一致（同仓库同 commit 测量）"
    );
    for binary in source
        .get("releaseBinaries")
        .and_then(Value::as_array)
        .expect("source 缺少 releaseBinaries")
    {
        let path = binary
            .get("path")
            .and_then(Value::as_str)
            .expect("release 二进制缺少 path");
        let bytes = std::fs::read(path)
            .unwrap_or_else(|error| panic!("读取 release 二进制 {path} 失败：{error}"));
        assert_eq!(
            binary.get("byteLength").and_then(Value::as_u64),
            Some(u64::try_from(bytes.len()).unwrap_or(u64::MAX)),
            "release 二进制 {path} 字节长度不符"
        );
        assert_eq!(
            binary.get("sha256").and_then(Value::as_str),
            Some(sha256_hex(&bytes).as_str()),
            "release 二进制 {path} SHA-256 不符"
        );
    }

    // 5. environment == 参考机声明全部同名字段（declarationRule：与 manifest 无关地核对）。
    let declaration: Value =
        serde_json::from_slice(&reference_machine_bytes).expect("参考机声明必须是合法 JSON");
    let environment = evidence.get("environment").expect("证据缺少 environment");
    for key in [
        "hardwareId",
        "hardwareIdentityScheme",
        "hardwareIdentitySha256",
        "cpu",
        "physicalCoreCount",
        "logicalProcessorCount",
        "physicalMemoryBytes",
        "operatingSystem",
        "operatingSystemBuild",
        "targetTriple",
        "rustc",
        "llvm",
        "powerSource",
        "powerPlan",
        "biosFirmware",
    ] {
        assert_eq!(
            environment.get(key),
            declaration.get(key),
            "environment.{key} 与参考机声明不符"
        );
    }

    // 6. rows：冻结行序、双 digest 与 manifest 行一致、中位数的中位数重算、
    // syntheticBase 恰好出现在 CORRIDOR 行。
    let manifest_json: Value =
        serde_json::from_slice(&manifest_bytes).expect("manifest 必须是合法 JSON");
    let manifest_rows = manifest_json
        .get("rows")
        .and_then(Value::as_array)
        .expect("manifest 缺少 rows 数组");
    let rows = evidence
        .get("rows")
        .and_then(Value::as_array)
        .expect("证据缺少 rows 数组");
    for (index, row) in rows.iter().enumerate() {
        let manifest_row = &manifest_rows[index];
        let label = format!("证据行 {index}");
        for key in [
            "workloadId",
            "accuracyProfileCode",
            "directionProfileCode",
            "semanticFingerprint",
            "completeOutputDigest",
        ] {
            assert_eq!(
                row.get(key),
                manifest_row.get(key),
                "{label} {key} 与 manifest 行不符"
            );
        }
        let levels = row.get("levels").expect("行缺少 levels");
        for level in LEVEL_IDS {
            check_median_of_medians(
                levels
                    .get(level)
                    .unwrap_or_else(|| panic!("行缺少 levels.{level}")),
                &label,
                level,
            );
        }
        let is_corridor =
            row.get("workloadId").and_then(Value::as_str) == Some(CORRIDOR_WORKLOAD_ID);
        assert_eq!(
            row.get("syntheticBase").is_some(),
            is_corridor,
            "{label} syntheticBase 必须恰好出现在 CORRIDOR 行"
        );
        if is_corridor {
            let base_levels = row
                .get("syntheticBase")
                .and_then(|base| base.get("levels"))
                .expect("CORRIDOR 行缺少 syntheticBase.levels");
            for level in BASE_LEVEL_IDS {
                check_median_of_medians(
                    base_levels
                        .get(level)
                        .unwrap_or_else(|| panic!("行缺少 syntheticBase.levels.{level}")),
                    &label,
                    level,
                );
            }
        }
    }

    // 7. budgetCalibration：候选数值 / 统计量 / observedValue / supported / calibratedBudget
    // 全部从证据行重算；closure 只接受 exact 候选数值与证据绑定。
    let full_compile_max = rows
        .iter()
        .map(|row| {
            row["levels"]["fullCompile"]["medianOfMediansNs"]
                .as_u64()
                .expect("行缺少 fullCompile 中位数的中位数")
        })
        .max()
        .expect("27 行非空");
    let peak_max = rows
        .iter()
        .map(|row| {
            row.get("compilerControlledPeakBytes")
                .and_then(Value::as_u64)
                .expect("行缺少编译器控制峰值")
        })
        .max()
        .expect("27 行非空");
    let retained_max = rows
        .iter()
        .map(|row| {
            row.get("compilerRetainedCapacityBytes")
                .and_then(Value::as_u64)
                .expect("行缺少保留容量")
        })
        .max()
        .expect("27 行非空");
    let budget_calibration = evidence
        .get("budgetCalibration")
        .expect("证据缺少 budgetCalibration");
    let cases: [(&str, &str, u64, &str, u64); 3] = [
        (
            "fullCompileWallClock",
            "ns",
            FULL_COMPILE_BUDGET_NS,
            "perRowMedianOfMediansMax",
            full_compile_max,
        ),
        (
            "compilerControlledPeak",
            "bytes",
            COMPILER_CONTROLLED_PEAK_BUDGET_BYTES,
            "perRowExactMax",
            peak_max,
        ),
        (
            "coldInstanceRetainedCapacity",
            "bytes",
            COLD_INSTANCE_RETAINED_BUDGET_BYTES,
            "perRowExactMax",
            retained_max,
        ),
    ];
    for (key, unit, candidate, statistic, observed) in cases {
        let entry = budget_calibration
            .get(key)
            .unwrap_or_else(|| panic!("budgetCalibration 缺少 {key}"));
        assert_eq!(
            entry.get("unit").and_then(Value::as_str),
            Some(unit),
            "{key}.unit 不符"
        );
        assert_eq!(
            entry.get("candidateBudget").and_then(Value::as_u64),
            Some(candidate),
            "{key}.candidateBudget 不符"
        );
        assert_eq!(
            entry.get("calibratedBudget").and_then(Value::as_u64),
            Some(candidate),
            "{key}.calibratedBudget 必须等于 exact 候选数值"
        );
        assert_eq!(
            entry.get("statistic").and_then(Value::as_str),
            Some(statistic),
            "{key}.statistic 不符"
        );
        assert_eq!(
            entry.get("observedValue").and_then(Value::as_u64),
            Some(observed),
            "{key}.observedValue 与证据行重算不符"
        );
        assert_eq!(
            entry.get("supported").and_then(Value::as_bool),
            Some(observed <= candidate),
            "{key}.supported 与 observedValue/候选预算关系不符"
        );
        let applies: Vec<&str> = entry
            .get("appliesTo")
            .and_then(|a| a.get("workloadIds"))
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("{key} 缺少 appliesTo.workloadIds"))
            .iter()
            .map(|id| id.as_str().expect("workloadIds 必须是字符串"))
            .collect();
        assert_eq!(
            applies,
            [MIN_WORKLOAD_ID, CORRIDOR_WORKLOAD_ID, P100_WORKLOAD_ID],
            "{key}.appliesTo.workloadIds 必须覆盖三个 workload"
        );
    }

    // 8. 从 raw 独立重算证据并逐字节比对。
    let recomputed = evidence::build_evidence(&raw_bytes, raw_path);
    assert_eq!(
        recomputed.as_bytes(),
        evidence_bytes.as_slice(),
        "证据必须能从 raw 独立重算且逐字节一致"
    );

    eprintln!(
        "evidence cross-record 验证通过：raw 绑定 / 来源 / 环境 / 27 行 / 预算校准一致，证据可由 raw 重算"
    );
}

/// 重算某级的 `medianOfMediansNs`：必须等于三个进程 `medianNs` 的中位数。
fn check_median_of_medians(level_value: &Value, label: &str, level: &str) {
    let per_process = level_value
        .get("perProcess")
        .and_then(Value::as_array)
        .expect("缺少 perProcess 数组");
    let medians: Vec<u64> = per_process
        .iter()
        .map(|process| {
            process
                .get("medianNs")
                .and_then(Value::as_u64)
                .expect("缺少 medianNs")
        })
        .collect();
    let (expected, _) = evidence::median_and_mad(&medians);
    assert_eq!(
        level_value.get("medianOfMediansNs").and_then(Value::as_u64),
        Some(expected),
        "{label} {level} 中位数的中位数重算不符"
    );
}
