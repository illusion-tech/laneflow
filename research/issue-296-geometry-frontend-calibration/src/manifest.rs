//! §9.2 workload manifest（27 行 = 3 fixture × 9 配置档组合）生成与 Draft 2020-12
//! 校验。行的全部计数与摘要来自编译器只读视图（`WorkloadCounts` /
//! `complete_output_digest`），fixture 身份来自冻结容器文件字节；harness 不自报任何数字。

use std::path::Path;

use laneflow_compiler::{GeometryAccuracyProfile, GeometryDirectionProfile};
use serde_json::{Value, json};

use crate::container::{FixtureModule, decode_container, sha256_hex};
use crate::counts::{
    ACCURACY_PROFILES, DIRECTION_PROFILES, GeometrySource, WorkloadCounts, accuracy_code,
    compile_geometry_workload, complete_output_digest, direction_code,
};

/// manifest 顶层 `schema` 字段常量。
pub const MANIFEST_SCHEMA: &str = "laneflow.geometry-frontend-calibration-workload-manifest";

pub const MIN_WORKLOAD_ID: &str = "LF-COMP-GEOMETRY-MIN-v1";
pub const CORRIDOR_WORKLOAD_ID: &str = "LF-COMP-GEOMETRY-CORRIDOR-v1";
pub const P100_WORKLOAD_ID: &str = "LF-COMP-GEOMETRY-P100-v1";

/// 三个冻结 fixture 的 repo 相对路径（manifest `fixture.path` 登记值）。
pub const MIN_FIXTURE_PATH: &str =
    "research/issue-296-geometry-frontend-calibration/fixtures/min-v1.fixture.json";
pub const CORRIDOR_FIXTURE_PATH: &str =
    "research/issue-296-geometry-frontend-calibration/fixtures/corridor-v1.fixture.json";
pub const P100_FIXTURE_PATH: &str =
    "research/issue-296-geometry-frontend-calibration/fixtures/p100-v1.fixture.json";

/// 一个冻结 fixture 工件：repo 相对路径、字节身份与解码后的模块集合。
pub struct WorkloadFixture {
    pub workload_id: String,
    pub path: String,
    pub byte_length: u64,
    pub sha256: String,
    pub modules: Vec<FixtureModule>,
}

/// 从 repo 根读取并解码 fixture 容器；自报 workload id 与字节身份不符预期即 panic。
#[must_use]
pub fn load_fixture(
    repo_root: &Path,
    relative_path: &str,
    expected_workload_id: &str,
) -> WorkloadFixture {
    let bytes = std::fs::read(repo_root.join(relative_path))
        .unwrap_or_else(|error| panic!("读取 fixture {relative_path} 失败：{error}"));
    let (workload_id, modules) = decode_container(&bytes);
    assert_eq!(
        workload_id, expected_workload_id,
        "fixture {relative_path} 自报 workload id 与登记不符"
    );
    WorkloadFixture {
        workload_id,
        path: relative_path.to_string(),
        byte_length: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        sha256: sha256_hex(&bytes),
        modules,
    }
}

fn hex_32(bytes: &[u8; 32]) -> String {
    let mut hex = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// 生成 27 行 manifest（行序 = workload 登记序 × accuracy 鉴别码 × direction 鉴别码），
/// 序列化为两空格缩进 JSON（与 #308 工件排版约定一致）。
#[must_use]
pub fn build_manifest(fixtures: &[WorkloadFixture]) -> String {
    let mut rows = Vec::with_capacity(27);
    for fixture in fixtures {
        let sources: Vec<GeometrySource<'_>> = fixture
            .modules
            .iter()
            .map(|module| GeometrySource {
                namespace: &fixture.workload_id,
                document_key: &module.source_path,
                source: module.source.as_bytes(),
            })
            .collect();
        for accuracy in ACCURACY_PROFILES {
            for direction in DIRECTION_PROFILES {
                let (output, counts) = compile_geometry_workload(&sources, accuracy, direction);
                rows.push(manifest_row(fixture, accuracy, direction, &counts, &output));
            }
        }
    }
    assert_eq!(rows.len(), 27, "manifest 必须恰好 27 行");
    let manifest = json!({
        "schema": MANIFEST_SCHEMA,
        "schemaVersion": 1,
        "rows": rows,
    });
    serde_json::to_string_pretty(&manifest).expect("manifest 序列化")
}

fn manifest_row(
    fixture: &WorkloadFixture,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    counts: &WorkloadCounts,
    output: &laneflow_compiler::CompilationOutput,
) -> Value {
    let distribution: Vec<Value> = counts
        .absolute_offset_distribution
        .iter()
        .map(|(bits, curve_count)| {
            json!({
                "absoluteOffsetMetersBits": format!("0x{bits:016x}"),
                "curveCount": curve_count,
            })
        })
        .collect();
    let table_counts = counts
        .lir_table_counts
        .as_ref()
        .expect("编译成功后 lir_table_counts 恒为 Some");
    let mut lir_table_counts = serde_json::Map::with_capacity(53);
    for (name, count) in table_counts.entries() {
        lir_table_counts.insert(name.to_string(), json!(count));
    }
    json!({
        "workloadId": fixture.workload_id,
        "fixture": {
            "path": fixture.path,
            "byteLength": fixture.byte_length,
            "sha256": fixture.sha256,
        },
        "accuracyProfileCode": accuracy_code(accuracy),
        "directionProfileCode": direction_code(direction),
        "counts": {
            "moduleCount": counts.module_count,
            "documentCount": counts.document_count,
            "declarationCount": counts.declaration_count,
            "referenceCount": counts.reference_count,
            "relationOccurrenceCount": counts.relation_occurrence_count,
            "lineSegmentCount": counts.line_segment_count,
            "cubicSegmentCount": counts.cubic_segment_count,
            "controlPointCount": counts.control_point_count,
            "offsetCurveCount": counts.offset_curve_count,
            "canonicalPointCount": counts.canonical_point_count,
            "lirRecordCount": counts.lir_record_count,
            "logicalOutputBytes": counts.logical_output_bytes,
        },
        "absoluteOffsetDistribution": distribution,
        "lirTableCounts": Value::Object(lir_table_counts),
        "semanticFingerprint": hex_32(&counts.semantic_fingerprint),
        "completeOutputDigest": hex_32(&complete_output_digest(output)),
    })
}

/// 以 Draft 2020-12 schema 校验 manifest 字节；任何违规即 panic（校准工件必须合法）。
pub fn validate_manifest(schema_bytes: &[u8], manifest_bytes: &[u8]) {
    let schema: Value = serde_json::from_slice(schema_bytes).expect("schema 必须是合法 JSON");
    let instance: Value = serde_json::from_slice(manifest_bytes).expect("manifest 必须是合法 JSON");
    jsonschema::draft202012::validate(&schema, &instance)
        .unwrap_or_else(|error| panic!("manifest 违反 Draft 2020-12 schema：{error}"));
}
