//! 九种位置/方向配置档组合与 workload 编译计数聚合：manifest 行与 cross-record
//! validator 共用的编译入口。计数只来自编译器只读视图（`GeometryModuleCounts`、
//! `LirTableCounts`、`CompilationMetrics`），不由 harness 自报。

use std::collections::BTreeMap;

use laneflow_compiler::{
    AccessRelationOwner, CompilationOutput, CompilationUnitBuilder, CompileLimits, Compiler,
    CrossSectionRelationOwner, GeometryAccuracyProfile, GeometryDirectionProfile,
    GeometryDocumentInput, GeometryModuleBuilder, JunctionRelationOwner, LirTableCounts,
    SignalRelationOwner, SourceLocationView, SourceRelationRole,
};
use sha2::{Digest as _, Sha256};

/// 完整 `CompilationOutput` 规范编码的域分隔符（UTF-8，NUL 结尾）。
/// 编码 = 域分隔符 || semantic_fingerprint(32B) || lir_record_count(u64le) ||
/// output_logical_bytes(u64le) || compiler_controlled_peak_bytes(u64le) ||
/// diagnostics 条数(u64le) || 53 张 record-counted 表行数（`LirTableCounts::NAMES`
/// 字典序，各 u64le）|| 完整 source-map 规范编码。LIR 逐行内容由编译器计算的
/// semantic_fingerprint 绑定；source-map 的描述符、owner、角色、local index 与来源位置
/// 使用下列显式 length-prefix 编码。manifest 生成器与 cross-record validator 共用本函数。
const COMPLETE_OUTPUT_DIGEST_DOMAIN: &[u8] =
    b"laneflow.geometry-frontend-calibration.complete-output.v2\0";

fn digest_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
    hasher.update(bytes);
}

fn digest_location(hasher: &mut Sha256, location: SourceLocationView<'_>) {
    digest_bytes(hasher, location.source_document_key().as_bytes());
    for value in [
        location.start().line(),
        location.start().column(),
        location.end().line(),
        location.end().column(),
    ] {
        hasher.update(value.to_le_bytes());
    }
}

fn digest_relation_header(
    hasher: &mut Sha256,
    owner_tag: u8,
    owner_ordinal: u32,
    owner_stable_id: &[u8; 16],
    role: SourceRelationRole,
    local_index: u32,
) {
    hasher.update([owner_tag]);
    hasher.update(owner_ordinal.to_le_bytes());
    hasher.update(owner_stable_id);
    hasher.update((role as u16).to_le_bytes());
    hasher.update(local_index.to_le_bytes());
}

macro_rules! digest_stable_sources {
    ($hasher:expr, $tag:literal, $sources:expr) => {{
        let sources = $sources;
        digest_bytes($hasher, $tag);
        $hasher.update(
            u64::try_from(sources.len())
                .unwrap_or(u64::MAX)
                .to_le_bytes(),
        );
        for source in sources {
            $hasher.update(source.ordinal().raw().to_le_bytes());
            $hasher.update(source.stable_id().as_untyped().as_bytes());
            digest_location($hasher, source.primary_source());
            let contributing = source.contributing_sources();
            $hasher.update(
                u64::try_from(contributing.len())
                    .unwrap_or(u64::MAX)
                    .to_le_bytes(),
            );
            for location in contributing {
                digest_location($hasher, location);
            }
        }
    }};
}

macro_rules! digest_relation_tail {
    ($hasher:expr, $source:expr) => {{
        digest_location($hasher, $source.primary_source());
        let contributing = $source.contributing_sources();
        $hasher.update(
            u64::try_from(contributing.len())
                .unwrap_or(u64::MAX)
                .to_le_bytes(),
        );
        for location in contributing {
            digest_location($hasher, location);
        }
    }};
}

fn digest_source_map(output: &CompilationOutput, hasher: &mut Sha256) {
    let source_map = output.source_map_input();
    digest_bytes(hasher, b"source-modules");
    let modules = source_map.source_modules();
    hasher.update(
        u64::try_from(modules.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    for module in modules {
        digest_bytes(hasher, module.authoring_namespace_id().as_bytes());
        digest_bytes(hasher, module.source_language().as_str().as_bytes());
        hasher.update(module.source_document_set_digest());
        hasher.update(module.source_document_set_digest_version().to_le_bytes());
        hasher.update(module.frontend_version().to_le_bytes());
        hasher.update(module.frontend_options_digest());
        digest_bytes(hasher, module.generator_build_id().as_bytes());
        hasher.update(module.parameters_and_inputs_digest());
        match module.random_seed() {
            Some(seed) => {
                hasher.update([1]);
                hasher.update(seed.to_le_bytes());
            }
            None => hasher.update([0]),
        }
        digest_bytes(hasher, module.provenance().as_bytes());
        let imports = module.imports();
        hasher.update(
            u64::try_from(imports.len())
                .unwrap_or(u64::MAX)
                .to_le_bytes(),
        );
        for import in imports {
            digest_bytes(hasher, import.as_bytes());
        }
    }
    digest_bytes(hasher, b"source-module-locations");
    let module_sources = source_map.source_module_sources();
    hasher.update(
        u64::try_from(module_sources.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    for source in module_sources {
        digest_location(hasher, source.primary_source());
    }
    digest_bytes(hasher, b"source-documents");
    let documents = source_map.source_documents();
    hasher.update(
        u64::try_from(documents.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    for document in documents {
        digest_bytes(hasher, document.source_document_key().as_bytes());
        digest_bytes(hasher, document.authoring_namespace_id().as_bytes());
        hasher.update(document.source_document_digest());
        hasher.update(document.source_record_byte_len().to_le_bytes());
        match document.origin().display_source() {
            Some(display) => {
                hasher.update([1]);
                digest_bytes(hasher, display.as_bytes());
            }
            None => hasher.update([0]),
        }
    }

    digest_stable_sources!(hasher, b"lane-edges", source_map.lane_edge_sources());
    digest_stable_sources!(
        hasher,
        b"road-corridors",
        source_map.road_corridor_sources()
    );
    digest_stable_sources!(hasher, b"road-sections", source_map.road_section_sources());
    digest_stable_sources!(
        hasher,
        b"authoring-lanes",
        source_map.authoring_lane_sources()
    );
    digest_stable_sources!(hasher, b"lane-groups", source_map.lane_group_sources());
    digest_stable_sources!(
        hasher,
        b"facility-bands",
        source_map.facility_band_sources()
    );
    digest_stable_sources!(hasher, b"junctions", source_map.junction_sources());
    digest_stable_sources!(hasher, b"movements", source_map.movement_sources());
    digest_stable_sources!(
        hasher,
        b"maneuver-paths",
        source_map.maneuver_path_sources()
    );
    digest_stable_sources!(hasher, b"stop-lines", source_map.stop_line_sources());
    digest_stable_sources!(
        hasher,
        b"maneuver-gates",
        source_map.maneuver_gate_sources()
    );
    digest_stable_sources!(hasher, b"waiting-zones", source_map.waiting_zone_sources());
    digest_stable_sources!(hasher, b"signal-groups", source_map.signal_group_sources());
    digest_stable_sources!(
        hasher,
        b"signal-controllers",
        source_map.signal_controller_sources()
    );
    digest_stable_sources!(hasher, b"signal-phases", source_map.signal_phase_sources());
    digest_stable_sources!(hasher, b"parking-areas", source_map.parking_area_sources());
    digest_stable_sources!(
        hasher,
        b"parking-spaces",
        source_map.parking_space_sources()
    );
    digest_stable_sources!(
        hasher,
        b"participant-classes",
        source_map.participant_class_sources()
    );
    digest_stable_sources!(
        hasher,
        b"vehicle-profiles",
        source_map.vehicle_profile_sources()
    );
    digest_stable_sources!(
        hasher,
        b"canonical-frames",
        source_map.canonical_frame_sources()
    );
    digest_stable_sources!(hasher, b"access-rules", source_map.access_rule_sources());
    digest_stable_sources!(hasher, b"static-routes", source_map.static_route_sources());

    digest_bytes(hasher, b"lane-edge-successors");
    let relations = source_map.lane_edge_successor_sources();
    hasher.update(
        u64::try_from(relations.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    for source in relations {
        digest_relation_header(
            hasher,
            1,
            source.owner_ordinal().raw(),
            source.owner_stable_id().as_untyped().as_bytes(),
            source.role(),
            source.local_index(),
        );
        digest_relation_tail!(hasher, source);
    }

    digest_bytes(hasher, b"cross-section-relations");
    let relations = source_map.cross_section_relation_sources();
    hasher.update(
        u64::try_from(relations.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    for source in relations {
        match source.owner() {
            CrossSectionRelationOwner::RoadCorridor(ordinal, id) => digest_relation_header(
                hasher,
                1,
                ordinal.raw(),
                id.as_untyped().as_bytes(),
                source.role(),
                source.local_index(),
            ),
            CrossSectionRelationOwner::RoadSection(ordinal, id) => digest_relation_header(
                hasher,
                2,
                ordinal.raw(),
                id.as_untyped().as_bytes(),
                source.role(),
                source.local_index(),
            ),
            CrossSectionRelationOwner::AuthoringLane(ordinal, id) => digest_relation_header(
                hasher,
                3,
                ordinal.raw(),
                id.as_untyped().as_bytes(),
                source.role(),
                source.local_index(),
            ),
            CrossSectionRelationOwner::LaneGroup(ordinal, id) => digest_relation_header(
                hasher,
                4,
                ordinal.raw(),
                id.as_untyped().as_bytes(),
                source.role(),
                source.local_index(),
            ),
            _ => unreachable!("new cross-section relation owner requires digest encoding"),
        }
        digest_relation_tail!(hasher, source);
    }

    digest_bytes(hasher, b"junction-relations");
    let relations = source_map.junction_relation_sources();
    hasher.update(
        u64::try_from(relations.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    for source in relations {
        match source.owner() {
            JunctionRelationOwner::Junction(ordinal, id) => digest_relation_header(
                hasher,
                1,
                ordinal.raw(),
                id.as_untyped().as_bytes(),
                source.role(),
                source.local_index(),
            ),
            JunctionRelationOwner::Movement(ordinal, id) => digest_relation_header(
                hasher,
                2,
                ordinal.raw(),
                id.as_untyped().as_bytes(),
                source.role(),
                source.local_index(),
            ),
            JunctionRelationOwner::ManeuverPath(ordinal, id) => digest_relation_header(
                hasher,
                3,
                ordinal.raw(),
                id.as_untyped().as_bytes(),
                source.role(),
                source.local_index(),
            ),
            JunctionRelationOwner::StopLine(ordinal, id) => digest_relation_header(
                hasher,
                4,
                ordinal.raw(),
                id.as_untyped().as_bytes(),
                source.role(),
                source.local_index(),
            ),
            _ => unreachable!("new junction relation owner requires digest encoding"),
        }
        digest_relation_tail!(hasher, source);
    }

    digest_bytes(hasher, b"signal-relations");
    let relations = source_map.signal_relation_sources();
    hasher.update(
        u64::try_from(relations.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    for source in relations {
        match source.owner() {
            SignalRelationOwner::SignalController(ordinal, id) => digest_relation_header(
                hasher,
                1,
                ordinal.raw(),
                id.as_untyped().as_bytes(),
                source.role(),
                source.local_index(),
            ),
            SignalRelationOwner::SignalPhase(ordinal, id) => digest_relation_header(
                hasher,
                2,
                ordinal.raw(),
                id.as_untyped().as_bytes(),
                source.role(),
                source.local_index(),
            ),
            SignalRelationOwner::ManeuverGate(ordinal, id) => digest_relation_header(
                hasher,
                3,
                ordinal.raw(),
                id.as_untyped().as_bytes(),
                source.role(),
                source.local_index(),
            ),
            _ => unreachable!("new signal relation owner requires digest encoding"),
        }
        digest_relation_tail!(hasher, source);
    }

    digest_bytes(hasher, b"access-relations");
    let relations = source_map.access_relation_sources();
    hasher.update(
        u64::try_from(relations.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    for source in relations {
        match source.owner() {
            AccessRelationOwner::ParticipantClass(ordinal, id) => digest_relation_header(
                hasher,
                1,
                ordinal.raw(),
                id.as_untyped().as_bytes(),
                source.role(),
                source.local_index(),
            ),
            AccessRelationOwner::VehicleProfile(ordinal, id) => digest_relation_header(
                hasher,
                2,
                ordinal.raw(),
                id.as_untyped().as_bytes(),
                source.role(),
                source.local_index(),
            ),
            AccessRelationOwner::AccessRule(ordinal, id) => digest_relation_header(
                hasher,
                3,
                ordinal.raw(),
                id.as_untyped().as_bytes(),
                source.role(),
                source.local_index(),
            ),
            _ => unreachable!("new access relation owner requires digest encoding"),
        }
        digest_relation_tail!(hasher, source);
    }

    macro_rules! digest_simple_relations {
        ($tag:literal, $owner_tag:literal, $relations:expr) => {{
            digest_bytes(hasher, $tag);
            let relations = $relations;
            hasher.update(
                u64::try_from(relations.len())
                    .unwrap_or(u64::MAX)
                    .to_le_bytes(),
            );
            for source in relations {
                digest_relation_header(
                    hasher,
                    $owner_tag,
                    source.owner_ordinal().raw(),
                    source.owner_stable_id().as_untyped().as_bytes(),
                    source.role(),
                    source.local_index(),
                );
                digest_relation_tail!(hasher, source);
            }
        }};
    }
    digest_simple_relations!(
        b"parking-relations",
        1,
        source_map.parking_relation_sources()
    );
    digest_simple_relations!(
        b"spatial-relations",
        1,
        source_map.spatial_relation_sources()
    );
    digest_simple_relations!(b"route-relations", 1, source_map.route_relation_sources());
}

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
    digest_source_map(output, &mut hasher);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile_min_fixture(display_source: Option<&str>) -> CompilationOutput {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/min-v1.fixture.json")).unwrap();
        let source = fixture["modules"][0]["source"].as_str().unwrap();
        let limits = CompileLimits::p100_initial_v1();
        let module = GeometryModuleBuilder::new(
            GeometryDocumentInput::new("min.geometry.json", source.as_bytes(), display_source),
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &limits,
        )
        .unwrap()
        .finish()
        .unwrap();
        let mut unit = CompilationUnitBuilder::new(limits);
        unit.add_geometry_module(module).unwrap();
        Compiler::new().compile(unit.build().unwrap()).unwrap()
    }

    #[test]
    fn complete_digest_binds_source_map_display_source() {
        let first = compile_min_fixture(Some("fixtures/min-a.geometry.json"));
        let second = compile_min_fixture(Some("fixtures/min-b.geometry.json"));

        assert_eq!(
            first.metrics().semantic_fingerprint(),
            second.metrics().semantic_fingerprint()
        );
        assert_ne!(
            complete_output_digest(&first),
            complete_output_digest(&second)
        );
    }
}
