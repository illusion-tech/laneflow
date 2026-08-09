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
use crate::manifest::{
    self, CORRIDOR_FIXTURE_PATH, CORRIDOR_WORKLOAD_ID, MIN_FIXTURE_PATH, MIN_WORKLOAD_ID,
    P100_FIXTURE_PATH, P100_WORKLOAD_ID,
};

/// trusted contract 描述符的 repo 相对路径（整条校验链的起点）。
pub const CONTRACT_PATH: &str = "docs/reference/geometry-frontend-calibration-contract-v1.json";

const CONTRACT_SCHEMA: &str = "laneflow.geometry-frontend-calibration-contract";
const DRAFT_2020_12_META: &str = "https://json-schema.org/draft/2020-12/schema";
const REFERENCE_MACHINE_SCHEMA: &str = "laneflow.geometry-frontend-calibration-reference-machine";

/// contract 中一个工件的 exact-bytes 绑定（path + byteLength + SHA-256）。
struct ArtifactBinding {
    path: String,
    byte_length: u64,
    sha256: String,
}

/// 从 contract JSON 读取一个工件绑定；字段缺失或类型不符即 panic（contract 必须完整）。
fn artifact_binding(contract: &Value, key: &str) -> ArtifactBinding {
    let entry = contract
        .get(key)
        .unwrap_or_else(|| panic!("contract 缺少 {key} 条目"));
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

/// 从 repo 根读取工件 exact bytes 并核对 contract 绑定（字节长度 + SHA-256）。
fn read_bound_artifact(repo_root: &Path, binding: &ArtifactBinding) -> Vec<u8> {
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

    // 2. 四件工件 exact bytes：证据 schema、manifest schema、manifest、参考机声明。
    let evidence_schema_bytes =
        read_bound_artifact(repo_root, &artifact_binding(&contract, "evidenceSchema"));
    let manifest_schema_bytes = read_bound_artifact(
        repo_root,
        &artifact_binding(&contract, "workloadManifestSchema"),
    );
    let manifest_bytes =
        read_bound_artifact(repo_root, &artifact_binding(&contract, "workloadManifest"));
    let reference_machine_bytes = read_bound_artifact(
        repo_root,
        &artifact_binding(&contract, "referenceMachineDeclaration"),
    );

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
        let binding = artifact_binding(&contract, key);
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
