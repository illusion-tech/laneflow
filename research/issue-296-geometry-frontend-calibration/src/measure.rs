//! §9.1 四级测量边界与 §9.2 三进程采样协议：每行每级 1 次未计时预热 + 7 次正式计时
//! 样本，三个独立进程各自产出一份样本文件。fullCompile 样本同时核对零诊断、语义指纹
//! 与完整输出 digest 和 manifest 行一致（防漂移护栏），并记录编译器控制峰值与冷实例
//! 保留容量。单模块前端存续/峰值与后端峰值全部来自编译器只读视图；harness 只按
//! builders/modules 的实际所有权生命周期累计模块值，不自报分配数字。

use std::hint::black_box;
use std::path::Path;
use std::time::{Duration, Instant};

use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, GeometryAccuracyProfile,
    GeometryDirectionProfile, GeometryDocumentInput, GeometryModuleBuilder,
};
use serde_json::{Value, json};

use crate::container::sha256_hex;
use crate::corridor::CorridorModel;
use crate::counts::{
    ACCURACY_PROFILES, DIRECTION_PROFILES, GeometrySource, accuracy_code, complete_output_digest,
    direction_code,
};
use crate::evidence::repo_relative;
use crate::manifest::{
    self, CORRIDOR_FIXTURE_PATH, CORRIDOR_WORKLOAD_ID, MIN_FIXTURE_PATH, MIN_WORKLOAD_ID,
    P100_FIXTURE_PATH, P100_WORKLOAD_ID, WorkloadFixture, load_fixture,
};
use crate::twin::{build_corridor_twin, harvest_geometry_output};

/// 每级正式计时样本数（§9.1：七个正式样本）。
pub const FORMAL_SAMPLE_COUNT: usize = 7;
/// 独立测量进程数（§9.1：三个独立进程）。
pub const PROCESS_COUNT: u8 = 3;

/// 四级测量边界 id（与证据 schema `protocol.levels` 元组逐位一致）。
pub const LEVEL_IDS: [&str; 4] = [
    "geometryParseBuild",
    "geometryNumericFreeze",
    "commonAdmission",
    "fullCompile",
];

/// Synthetic base 报告的边界子集（§9.1：base 只用于分离 Geometry 固有解析/细分成本）。
pub const BASE_LEVEL_IDS: [&str; 2] = ["commonAdmission", "fullCompile"];

const PROCESS_SAMPLES_SCHEMA: &str = "laneflow.geometry-frontend-calibration-process-samples";

const TRAFFIC_FIXTURE: &str =
    include_str!("../../../examples/data/v0.10-signalized-corridor.laneflow.json");

/// manifest 行中冻结的两个 digest（measure 的防漂移护栏期望值）。
struct ExpectedDigests {
    semantic_fingerprint: String,
    complete_output_digest: String,
}

/// 从 manifest 读取全部 27 行的期望 digest，按 `(workloadId, 位置码, 方向码)` 索引。
fn load_expected_digests(
    repo_root: &Path,
) -> std::collections::BTreeMap<(String, u8, u8), ExpectedDigests> {
    let manifest_path =
        repo_root.join("docs/reference/geometry-frontend-calibration-workload-manifest-v1.json");
    let bytes =
        std::fs::read(&manifest_path).unwrap_or_else(|error| panic!("读取 manifest 失败：{error}"));
    let manifest: Value = serde_json::from_slice(&bytes).expect("manifest 必须是合法 JSON");
    let mut expected = std::collections::BTreeMap::new();
    for row in manifest
        .get("rows")
        .and_then(Value::as_array)
        .expect("manifest 缺少 rows 数组")
    {
        let workload_id = row
            .get("workloadId")
            .and_then(Value::as_str)
            .expect("行缺少 workloadId")
            .to_string();
        let accuracy = u8::try_from(
            row.get("accuracyProfileCode")
                .and_then(Value::as_u64)
                .expect("行缺少 accuracyProfileCode"),
        )
        .expect("位置鉴别码必须在 1..=3");
        let direction = u8::try_from(
            row.get("directionProfileCode")
                .and_then(Value::as_u64)
                .expect("行缺少 directionProfileCode"),
        )
        .expect("方向鉴别码必须在 1..=3");
        let digests = ExpectedDigests {
            semantic_fingerprint: row
                .get("semanticFingerprint")
                .and_then(Value::as_str)
                .expect("行缺少 semanticFingerprint")
                .to_string(),
            complete_output_digest: row
                .get("completeOutputDigest")
                .and_then(Value::as_str)
                .expect("行缺少 completeOutputDigest")
                .to_string(),
        };
        assert!(
            expected
                .insert((workload_id, accuracy, direction), digests)
                .is_none(),
            "manifest 行键重复"
        );
    }
    assert_eq!(expected.len(), 27, "manifest 必须恰好 27 行");
    expected
}

fn sources_of(fixture: &WorkloadFixture) -> Vec<GeometrySource<'_>> {
    fixture
        .modules
        .iter()
        .map(|module| GeometrySource {
            namespace: &fixture.workload_id,
            document_key: &module.source_path,
            source: module.source.as_bytes(),
        })
        .collect()
}

fn source_namespace(source: &str) -> String {
    serde_json::from_str::<Value>(source)
        .expect("fixture module source 必须是合法 JSON")
        .get("module")
        .and_then(|module| module.get("namespace"))
        .and_then(Value::as_str)
        .expect("fixture module source 缺少 module.namespace")
        .to_string()
}

fn build_builders(
    sources: &[GeometrySource<'_>],
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    limits: &CompileLimits,
) -> BuiltGeometryModules {
    let mut builders = Vec::with_capacity(sources.len());
    let mut live_builder_bytes = 0_u64;
    let mut parse_build_peak_controlled_live_bytes = 0_u64;
    for source in sources {
        let builder = GeometryModuleBuilder::new(
            GeometryDocumentInput::new(source.document_key, source.source, None),
            accuracy,
            direction,
            limits,
        )
        .unwrap_or_else(|diagnostics| panic!("geometry 模块构造失败：{diagnostics:?}"));
        parse_build_peak_controlled_live_bytes = parse_build_peak_controlled_live_bytes.max(
            live_builder_bytes.saturating_add(builder.parse_build_peak_controlled_live_bytes()),
        );
        live_builder_bytes = live_builder_bytes.saturating_add(builder.controlled_live_bytes());
        builders.push(builder);
    }
    BuiltGeometryModules {
        builders,
        parse_build_peak_controlled_live_bytes,
    }
}

struct BuiltGeometryModules {
    builders: Vec<GeometryModuleBuilder>,
    parse_build_peak_controlled_live_bytes: u64,
}

struct FinishedGeometryModules {
    modules: Vec<laneflow_compiler::GeometryModule>,
    frontend_peak_controlled_live_bytes: u64,
}

fn finish_modules(built: BuiltGeometryModules) -> FinishedGeometryModules {
    let builders = built.builders;
    let mut pending_builder_bytes = builders.iter().fold(0_u64, |total, builder| {
        total.saturating_add(builder.controlled_live_bytes())
    });
    let mut finished_module_bytes = 0_u64;
    let mut frontend_peak_controlled_live_bytes =
        pending_builder_bytes.max(built.parse_build_peak_controlled_live_bytes);
    let mut modules = Vec::with_capacity(builders.len());
    for builder in builders {
        pending_builder_bytes =
            pending_builder_bytes.saturating_sub(builder.controlled_live_bytes());
        let module = builder
            .finish()
            .unwrap_or_else(|diagnostics| panic!("geometry 模块 finish 失败：{diagnostics:?}"));
        frontend_peak_controlled_live_bytes = frontend_peak_controlled_live_bytes.max(
            pending_builder_bytes
                .saturating_add(finished_module_bytes)
                .saturating_add(module.counts().frontend_peak_controlled_live_bytes()),
        );
        finished_module_bytes =
            finished_module_bytes.saturating_add(module.counts().controlled_live_bytes());
        modules.push(module);
    }
    FinishedGeometryModules {
        modules,
        frontend_peak_controlled_live_bytes,
    }
}

/// 边界 1：从借用原始 bytes 到 `GeometryModuleBuilder`（含一次 SHA-256 与有界解析，
/// 不含 numeric freeze）。
fn level_parse_build(
    sources: &[GeometrySource<'_>],
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    limits: &CompileLimits,
) -> Duration {
    let started = Instant::now();
    let builders = build_builders(sources, accuracy, direction, limits);
    let elapsed = started.elapsed();
    black_box(&builders.builders);
    elapsed
}

/// 边界 2：对已构造 builder 执行 `finish`（builder 构造在计时区外）。
fn level_numeric_freeze(
    sources: &[GeometrySource<'_>],
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    limits: &CompileLimits,
) -> Duration {
    let builders = build_builders(sources, accuracy, direction, limits);
    let started = Instant::now();
    let modules = finish_modules(builders).modules;
    let elapsed = started.elapsed();
    black_box(&modules);
    elapsed
}

/// 边界 3：对已构造 `GeometryModule` 执行 `add_geometry_module + build`（不归因 #315；
/// 模块构造与 freeze 在计时区外）。
fn level_common_admission(
    sources: &[GeometrySource<'_>],
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    limits: &CompileLimits,
) -> Duration {
    let modules = finish_modules(build_builders(sources, accuracy, direction, limits)).modules;
    let started = Instant::now();
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    for module in modules {
        unit.add_geometry_module(module)
            .unwrap_or_else(|diagnostics| panic!("共同 admission 失败：{diagnostics:?}"));
    }
    let unit = unit.build().expect("编译单元构造失败");
    let elapsed = started.elapsed();
    black_box(&unit);
    elapsed
}

/// 边界 4：原始 bytes 到 `CompilationOutput`（含 HIR/MIR/LIR 与 source-map，真实产品
/// 成本；冷 `Compiler` 实例构造计入边界）。返回墙钟、输出与冷实例保留容量。
fn level_full_compile(
    sources: &[GeometrySource<'_>],
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    limits: &CompileLimits,
) -> (Duration, laneflow_compiler::CompilationOutput, u64, u64) {
    let started = Instant::now();
    let finished = finish_modules(build_builders(sources, accuracy, direction, limits));
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    for module in finished.modules {
        unit.add_geometry_module(module)
            .unwrap_or_else(|diagnostics| panic!("共同 admission 失败：{diagnostics:?}"));
    }
    let mut compiler = Compiler::new();
    let output = compiler
        .compile(unit.build().expect("编译单元构造失败"))
        .expect("fullCompile 样本必须可编译");
    let elapsed = started.elapsed();
    let retained = compiler.retained_capacity_bytes();
    let pipeline_peak = finished
        .frontend_peak_controlled_live_bytes
        .max(output.metrics().compiler_controlled_peak_bytes());
    (elapsed, output, retained, pipeline_peak)
}

/// Synthetic base 边界 3：孪生构造在计时区外，只对 `add_synthetic_module + build` 计时。
fn base_common_admission(
    model: &CorridorModel,
    namespace: &str,
    document_key: &str,
    limits: &CompileLimits,
    harvest: &crate::twin::Harvest,
) -> Duration {
    let twin = build_corridor_twin(model, namespace, document_key, limits, harvest);
    let started = Instant::now();
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    unit.add_synthetic_module(twin)
        .expect("孪生模块进入编译单元");
    let unit = unit.build().expect("编译单元构造失败");
    let elapsed = started.elapsed();
    black_box(&unit);
    elapsed
}

/// Synthetic base 边界 4：孪生构造 + admission + 完整 compile 全链路计时。
fn base_full_compile(
    model: &CorridorModel,
    namespace: &str,
    document_key: &str,
    limits: &CompileLimits,
    harvest: &crate::twin::Harvest,
) -> Duration {
    let started = Instant::now();
    let twin = build_corridor_twin(model, namespace, document_key, limits, harvest);
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    unit.add_synthetic_module(twin)
        .expect("孪生模块进入编译单元");
    let mut compiler = Compiler::new();
    let output = compiler
        .compile(unit.build().expect("编译单元构造失败"))
        .expect("base fullCompile 必须可编译");
    let elapsed = started.elapsed();
    assert!(
        output.diagnostics().is_empty(),
        "base fullCompile 必须零诊断"
    );
    black_box(&output);
    elapsed
}

/// 单级样本集合：1 次未计时预热 + 7 次正式计时（纳秒，精确整数）。
struct LevelSamples {
    warmup_ns: u64,
    samples_ns: [u64; FORMAL_SAMPLE_COUNT],
}

impl LevelSamples {
    fn to_json(&self) -> Value {
        json!({
            "warmupNs": [self.warmup_ns],
            "samplesNs": self.samples_ns,
        })
    }
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).expect("单样本墙钟必须放进 u64 纳秒")
}

/// 测量一行（fixture × 位置档 × 方向档）的全部四级样本与 base 样本（CORRIDOR）。
fn measure_row(
    fixture: &WorkloadFixture,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    expected: &ExpectedDigests,
    corridor_model: Option<&CorridorModel>,
) -> Value {
    let label = format!(
        "{} 位置{} 方向{}",
        fixture.workload_id,
        accuracy_code(accuracy),
        direction_code(direction)
    );
    let limits = CompileLimits::p100_initial_v1();
    let sources = sources_of(fixture);

    let parse_build = measure_level(|| level_parse_build(&sources, accuracy, direction, &limits));
    let numeric_freeze =
        measure_level(|| level_numeric_freeze(&sources, accuracy, direction, &limits));
    let common_admission =
        measure_level(|| level_common_admission(&sources, accuracy, direction, &limits));

    // fullCompile：1 预热 + 7 正式，逐样本护栏（零诊断、双 digest 与 manifest 行一致、
    // 峰值与保留容量跨样本恒定）。
    let mut full_compile_warmup = 0_u64;
    let mut full_compile_samples = [0_u64; FORMAL_SAMPLE_COUNT];
    let mut peak_bytes: Option<u64> = None;
    let mut retained_bytes: Option<u64> = None;
    let mut last_output = None;
    for index in 0..=FORMAL_SAMPLE_COUNT {
        let (elapsed, output, retained, peak) =
            level_full_compile(&sources, accuracy, direction, &limits);
        assert!(
            output.diagnostics().is_empty(),
            "{label} fullCompile 必须零诊断"
        );
        assert_eq!(
            manifest::hex_32(&output.metrics().semantic_fingerprint()),
            expected.semantic_fingerprint,
            "{label} 语义指纹与 manifest 行不符"
        );
        assert_eq!(
            manifest::hex_32(&complete_output_digest(&output)),
            expected.complete_output_digest,
            "{label} 完整输出 digest 与 manifest 行不符"
        );
        match peak_bytes {
            None => peak_bytes = Some(peak),
            Some(previous) => assert_eq!(previous, peak, "{label} 编译器控制峰值跨样本漂移"),
        }
        match retained_bytes {
            None => retained_bytes = Some(retained),
            Some(previous) => assert_eq!(previous, retained, "{label} 冷实例保留容量跨样本漂移"),
        }
        if index == 0 {
            full_compile_warmup = nanos(elapsed);
        } else {
            full_compile_samples[index - 1] = nanos(elapsed);
        }
        last_output = Some(output);
    }
    let full_compile = LevelSamples {
        warmup_ns: full_compile_warmup,
        samples_ns: full_compile_samples,
    };

    // Synthetic base（仅 CORRIDOR）：收获取自本行组合的几何输出，孪生构造语义无时钟。
    let synthetic_base = corridor_model.map(|model| {
        let harvest =
            harvest_geometry_output(last_output.as_ref().expect("fullCompile 已产生输出"));
        let namespace = source_namespace(&fixture.modules[0].source);
        let document_key = &fixture.modules[0].source_path;
        let base_admission = measure_level(|| {
            base_common_admission(model, &namespace, document_key, &limits, &harvest)
        });
        let base_compile =
            measure_level(|| base_full_compile(model, &namespace, document_key, &limits, &harvest));
        json!({
            "levels": {
                "commonAdmission": base_admission.to_json(),
                "fullCompile": base_compile.to_json(),
            }
        })
    });

    let mut row = json!({
        "workloadId": fixture.workload_id,
        "accuracyProfileCode": accuracy_code(accuracy),
        "directionProfileCode": direction_code(direction),
        "levels": {
            "geometryParseBuild": parse_build.to_json(),
            "geometryNumericFreeze": numeric_freeze.to_json(),
            "commonAdmission": common_admission.to_json(),
            "fullCompile": full_compile.to_json(),
        },
        "compilerControlledPeakBytes": peak_bytes.expect("fullCompile 已记录峰值"),
        "compilerRetainedCapacityBytes": retained_bytes.expect("fullCompile 已记录保留容量"),
        "semanticFingerprint": expected.semantic_fingerprint,
        "completeOutputDigest": expected.complete_output_digest,
    });
    if let Some(base) = synthetic_base {
        row.as_object_mut()
            .expect("行必须是对象")
            .insert("syntheticBase".to_string(), base);
    }
    row
}

/// 对单级执行 1 次未计时预热 + 7 次正式计时。
fn measure_level(mut run: impl FnMut() -> Duration) -> LevelSamples {
    let warmup = run();
    let mut samples = [0_u64; FORMAL_SAMPLE_COUNT];
    for sample in &mut samples {
        *sample = nanos(run());
    }
    LevelSamples {
        warmup_ns: nanos(warmup),
        samples_ns: samples,
    }
}

/// 执行一个完整测量进程：27 行（或 `--only-workload` 子集）× 四级 ×（1 预热 + 7 样本），
/// 写进程样本 JSON 并打印测量二进制字节身份。
pub fn measure_process(
    repo_root: &Path,
    process_index: u8,
    only_workload: Option<&str>,
    output_path: &Path,
) {
    assert!(
        (1..=PROCESS_COUNT).contains(&process_index),
        "processIndex 必须在 1..={PROCESS_COUNT}"
    );
    let expected = load_expected_digests(repo_root);
    let fixtures = [
        load_fixture(repo_root, MIN_FIXTURE_PATH, MIN_WORKLOAD_ID),
        load_fixture(repo_root, CORRIDOR_FIXTURE_PATH, CORRIDOR_WORKLOAD_ID),
        load_fixture(repo_root, P100_FIXTURE_PATH, P100_WORKLOAD_ID),
    ];
    let traffic: Value =
        serde_json::from_str(TRAFFIC_FIXTURE).expect("traffic fixture 必须是合法 JSON");
    let corridor_model = CorridorModel::parse(&traffic);

    let mut rows = Vec::with_capacity(27);
    for fixture in &fixtures {
        if let Some(only) = only_workload
            && fixture.workload_id != only
        {
            continue;
        }
        for accuracy in ACCURACY_PROFILES {
            for direction in DIRECTION_PROFILES {
                let key = (
                    fixture.workload_id.clone(),
                    accuracy_code(accuracy),
                    direction_code(direction),
                );
                let expected = expected.get(&key).expect("manifest 必须覆盖该行");
                let corridor =
                    (fixture.workload_id == CORRIDOR_WORKLOAD_ID).then_some(&corridor_model);
                rows.push(measure_row(
                    fixture, accuracy, direction, expected, corridor,
                ));
                eprintln!(
                    "测量完成 {} 位置{} 方向{}",
                    fixture.workload_id,
                    accuracy_code(accuracy),
                    direction_code(direction)
                );
            }
        }
    }
    if only_workload.is_none() {
        assert_eq!(rows.len(), 27, "正式测量必须覆盖 27 行");
    }

    let executable = std::env::current_exe().expect("读取当前可执行路径失败");
    let binary_bytes = std::fs::read(&executable)
        .unwrap_or_else(|error| panic!("读取测量二进制 {} 失败：{error}", executable.display()));
    let binary = json!({
        "path": repo_relative(repo_root, &executable),
        "byteLength": u64::try_from(binary_bytes.len()).unwrap_or(u64::MAX),
        "sha256": sha256_hex(&binary_bytes),
    });
    let report = json!({
        "schema": PROCESS_SAMPLES_SCHEMA,
        "schemaVersion": 1,
        "processIndex": process_index,
        "binary": binary,
        "rows": rows,
    });
    let serialized = serde_json::to_string_pretty(&report).expect("进程样本序列化");
    std::fs::write(output_path, &serialized)
        .unwrap_or_else(|error| panic!("写进程样本 {} 失败：{error}", output_path.display()));
    eprintln!(
        "进程 {process_index} 测量完成：{} 行 → {}",
        report["rows"].as_array().map_or(0, Vec::len),
        output_path.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_base_namespace_comes_from_geometry_document() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let fixture = load_fixture(&repo_root, CORRIDOR_FIXTURE_PATH, CORRIDOR_WORKLOAD_ID);

        assert_eq!(
            source_namespace(&fixture.modules[0].source),
            "calibration/geometry/corridor"
        );
        assert_ne!(
            source_namespace(&fixture.modules[0].source),
            fixture.workload_id
        );
    }

    #[test]
    fn frontend_peak_aggregates_all_simultaneously_live_builders_and_modules() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let fixture = load_fixture(&repo_root, P100_FIXTURE_PATH, P100_WORKLOAD_ID);
        let sources = sources_of(&fixture);
        let built = build_builders(
            &sources,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &CompileLimits::p100_initial_v1(),
        );
        assert!(built.builders.len() > 1);
        let initial_builder_bytes = built.builders.iter().fold(0_u64, |total, builder| {
            total.saturating_add(builder.controlled_live_bytes())
        });
        assert!(built.parse_build_peak_controlled_live_bytes >= initial_builder_bytes);

        let finished = finish_modules(built);
        let final_module_bytes = finished.modules.iter().fold(0_u64, |total, module| {
            total.saturating_add(module.counts().controlled_live_bytes())
        });

        assert!(finished.frontend_peak_controlled_live_bytes >= initial_builder_bytes);
        assert!(finished.frontend_peak_controlled_live_bytes >= final_module_bytes);
    }
}
