//! 九种位置/方向配置档组合与 workload 编译计数聚合：manifest 行与 cross-record
//! validator 共用的编译入口。计数只来自编译器只读视图（`GeometryModuleCounts`、
//! `LirTableCounts`、`CompilationMetrics`），不由 harness 自报。

use std::collections::BTreeMap;

use laneflow_compiler::{
    CompilationOutput, CompilationUnitBuilder, CompileLimits, Compiler, GeometryAccuracyProfile,
    GeometryDirectionProfile, GeometryDocumentInput, GeometryModuleBuilder, LirTableCounts,
};
use sha2::{Digest as _, Sha256};

/// 完整 `CompilationOutput` 规范编码的域分隔符（UTF-8，NUL 结尾）。
/// 编码 = 域分隔符 || semantic_fingerprint(32B) || lir_record_count(u64le) ||
/// output_logical_bytes(u64le) || compiler_controlled_peak_bytes(u64le) ||
/// diagnostics 条数(u64le) || 53 张 record-counted 表行数（`LirTableCounts::NAMES`
/// 字典序，各 u64le）。LIR 逐行内容由编译器计算的 semantic_fingerprint 绑定，
/// 表基数与指标显式列入；manifest 生成器与 cross-record validator 共用本函数。
const COMPLETE_OUTPUT_DIGEST_DOMAIN: &[u8] =
    b"laneflow.geometry-frontend-calibration.complete-output.v1\0";

/// 位置误差配置档全集合；鉴别码 1..=3 与 manifest `accuracyProfileCode` 一致。
pub const ACCURACY_PROFILES: [GeometryAccuracyProfile; 3] = [
    GeometryAccuracyProfile::Fine2Cm,
    GeometryAccuracyProfile::Balanced5Cm,
    GeometryAccuracyProfile::Compact10Cm,
];

/// 方向跳变配置档全集合；鉴别码 1..=3 与 manifest `directionProfileCode` 一致。
pub const DIRECTION_PROFILES: [GeometryDirectionProfile; 3] = [
    GeometryDirectionProfile::Smooth1Deg,
    GeometryDirectionProfile::Balanced2Deg,
    GeometryDirectionProfile::Compact5Deg,
];

/// 位置配置档鉴别码（枚举判别式即冻结编码）。
#[must_use]
pub const fn accuracy_code(profile: GeometryAccuracyProfile) -> u8 {
    profile as u8
}

/// 方向配置档鉴别码（枚举判别式即冻结编码）。
#[must_use]
pub const fn direction_code(profile: GeometryDirectionProfile) -> u8 {
    profile as u8
}

/// 一个 geometry 源模块：`（命名空间, 文档键, 来源字节）`。
pub struct GeometrySource<'a> {
    pub namespace: &'a str,
    pub document_key: &'a str,
    pub source: &'a [u8],
}

/// 一次 workload 编译聚合的只读计数（manifest 行的唯一数据来源）。
#[derive(Clone, Debug)]
pub struct WorkloadCounts {
    pub module_count: u64,
    pub document_count: u64,
    pub declaration_count: u64,
    pub reference_count: u64,
    pub relation_occurrence_count: u64,
    pub line_segment_count: u64,
    pub cubic_segment_count: u64,
    pub control_point_count: u64,
    pub offset_curve_count: u64,
    pub canonical_point_count: u64,
    pub lir_record_count: u64,
    pub logical_output_bytes: u64,
    pub semantic_fingerprint: [u8; 32],
    /// 编译成功后填充；构造期恒为 `None`，读取方只消费 `Some`。
    pub lir_table_counts: Option<LirTableCounts>,
    /// 跨模块聚合的 |中心偏移| 位模式 → 曲线数。
    pub absolute_offset_distribution: BTreeMap<u64, u64>,
}

/// 以给定配置档编译一组 geometry 模块并聚合计数；编译失败即 panic（fixture 必须可编译）。
pub fn compile_geometry_workload(
    modules: &[GeometrySource<'_>],
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
) -> (CompilationOutput, WorkloadCounts) {
    let limits = CompileLimits::p100_initial_v1();
    let mut counts = WorkloadCounts {
        module_count: 0,
        document_count: 0,
        declaration_count: 0,
        reference_count: 0,
        relation_occurrence_count: 0,
        line_segment_count: 0,
        cubic_segment_count: 0,
        control_point_count: 0,
        offset_curve_count: 0,
        canonical_point_count: 0,
        lir_record_count: 0,
        logical_output_bytes: 0,
        semantic_fingerprint: [0; 32],
        lir_table_counts: None,
        absolute_offset_distribution: BTreeMap::new(),
    };
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    for source in modules {
        let module = GeometryModuleBuilder::new(
            GeometryDocumentInput::new(source.document_key, source.source, None),
            accuracy,
            direction,
            &limits,
        )
        .unwrap_or_else(|diagnostics| {
            panic!(
                "geometry 模块 {} 构造失败：{diagnostics:?}",
                source.namespace
            )
        })
        .finish()
        .unwrap_or_else(|diagnostics| {
            panic!(
                "geometry 模块 {} finish 失败：{diagnostics:?}",
                source.namespace
            )
        });
        let module_counts = module.counts();
        counts.module_count += 1;
        counts.document_count += u64::try_from(module.source_documents().len()).unwrap_or(u64::MAX);
        counts.declaration_count += module_counts.declaration_count();
        counts.reference_count += module_counts.reference_count();
        counts.relation_occurrence_count += module_counts.relation_occurrence_count();
        counts.line_segment_count += module_counts.line_segment_count();
        counts.cubic_segment_count += module_counts.cubic_segment_count();
        counts.control_point_count += module_counts.control_point_count();
        counts.offset_curve_count += module_counts.offset_curve_count();
        counts.canonical_point_count += module_counts.canonical_point_count();
        for bucket in module_counts.absolute_offset_distribution() {
            *counts
                .absolute_offset_distribution
                .entry(bucket.absolute_offset_meters_bits())
                .or_insert(0) += bucket.curve_count();
        }
        unit.add_geometry_module(module)
            .unwrap_or_else(|diagnostics| {
                panic!(
                    "geometry 模块 {} 进入编译单元失败：{diagnostics:?}",
                    source.namespace
                )
            });
    }
    let output = Compiler::new()
        .compile(unit.build().expect("编译单元构造失败"))
        .expect("geometry workload 必须可编译");
    counts.lir_record_count = output.metrics().lir_record_count();
    counts.logical_output_bytes = output.metrics().output_logical_bytes();
    counts.semantic_fingerprint = output.metrics().semantic_fingerprint();
    counts.lir_table_counts = Some(output.lir().lir_table_counts());
    (output, counts)
}

/// 计算完整 `CompilationOutput` 规范编码的 SHA-256（编码规则见域分隔符常量注释）。
/// 校准 fixture 必须编译零诊断；成功路径残留任何诊断都直接 panic。
#[must_use]
pub fn complete_output_digest(output: &CompilationOutput) -> [u8; 32] {
    assert!(
        output.diagnostics().is_empty(),
        "校准 workload 编译必须零诊断"
    );
    let metrics = output.metrics();
    let mut hasher = Sha256::new();
    hasher.update(COMPLETE_OUTPUT_DIGEST_DOMAIN);
    hasher.update(metrics.semantic_fingerprint());
    for value in [
        metrics.lir_record_count(),
        metrics.output_logical_bytes(),
        metrics.compiler_controlled_peak_bytes(),
        0_u64, // diagnostics 条数；上面的断言冻结为零
    ] {
        hasher.update(value.to_le_bytes());
    }
    for (_, count) in output.lir().lir_table_counts().entries() {
        hasher.update(count.to_le_bytes());
    }
    hasher.finalize().into()
}
