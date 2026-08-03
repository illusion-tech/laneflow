//! 高层中间表示（HIR）到中层中间表示（MIR）的确定性降级阶段。
//!
//! HIR 已完成模块与符号解析；本阶段不再接受文本引用，而是把模块、车道图边和下游
//! 连接冻结为目标布局中立的连续表。HIR 与 MIR 使用不同的键标记，并通过显式映射表
//! 转换，避免两个碰巧相同的 `u32` 被误认为可跨阶段复用。
//!
//! MIR 仍是 crate 私有编译阶段，不是静态镜像 ABI 或公共制品格式。它保留稳定键、
//! `f64` 交通标量和来源位置；后续 LIR 验证/冻结完成前，调用方不得把这些表视为已验证
//! 发布输出。

use std::sync::Arc;

use crate::arena::{ArenaKey, ArenaKeyOverflow, TableRange, TypedArena};
use crate::diagnostic::DiagnosticCollector;
use crate::hir::{HirLaneEdgeKey, HirUnit};
use crate::{CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceSpan};

/// 区分 MIR 模块表键的零尺寸阶段标记。
pub(crate) enum MirModuleTag {}
/// 区分 MIR 车道图边表键的零尺寸阶段标记。
pub(crate) enum MirLaneEdgeTag {}

/// 仅在当前 `MirUnit` 模块表内有效的致密键。
pub(crate) type MirModuleKey = ArenaKey<MirModuleTag>;
/// 仅在当前 `MirUnit` 车道图边表内有效的致密键。
pub(crate) type MirLaneEdgeKey = ArenaKey<MirLaneEdgeTag>;

/// MIR 中保留的模块身份与来源上下文。
pub(crate) struct MirModule {
    /// 模块稳定 authoring namespace。
    pub(crate) authoring_namespace_id: Arc<str>,
    /// 与机器路径无关的来源文档键。
    pub(crate) source_document_key: Arc<str>,
    /// 模块声明位置。
    pub(crate) source_span: SourceSpan,
}

/// MIR 平坦连接表中的一条有类型车道图边连接。
pub(crate) struct MirLaneEdgeConnection {
    /// 当前 `MirUnit::lane_edges` 中的目标键。
    pub(crate) target: MirLaneEdgeKey,
    /// 原始引用位置，供后续诊断与源映射使用。
    pub(crate) source_span: SourceSpan,
}

/// 已冻结模块归属和连续连接区间的车道图边 MIR 记录。
pub(crate) struct MirLaneEdge {
    /// 拥有声明的 MIR 模块；不能用原始值当作 HIR 模块键。
    pub(crate) module: MirModuleKey,
    /// 模块内稳定键；不由 MIR 致密下标派生。
    pub(crate) stable_key: Arc<str>,
    /// 交通权威长度，单位为米并保持 `f64`。
    pub(crate) length_meters: f64,
    /// 基础道路限速，单位为米每秒并保持 `f64`。
    pub(crate) speed_limit_meters_per_second: f64,
    /// 此边在 `MirUnit::lane_edge_connections` 中的半开连续区间。
    pub(crate) connections: TableRange<MirLaneEdgeConnection>,
    /// 原始声明位置。
    pub(crate) source_span: SourceSpan,
}

/// MIR 阶段成功后一次性冻结的目标布局中立表集合。
///
/// 每个连接区间都落在 `lane_edge_connections` 内，且所有目标键指向本实例的
/// `lane_edges`。`controlled_live_bytes` 只统计 MIR 成功返回后自身拥有的表；阶段峰值
/// 预检还包含 CompilationUnit、HIR 与键映射暂存区。
pub(crate) struct MirUnit {
    pub(crate) modules: Box<[MirModule]>,
    pub(crate) lane_edges: Box<[MirLaneEdge]>,
    pub(crate) lane_edge_connections: Box<[MirLaneEdgeConnection]>,
    pub(crate) mir_record_count: u64,
    pub(crate) controlled_live_bytes: u64,
}

/// 将已解析 HIR 降级为连续 MIR 表，并显式重映射全部阶段键。
///
/// # Errors
///
/// 当 MIR 记录、阶段暂存区、编译器控制存续字节或 `u32` 表边界超过所选配置档时，
/// 返回资源诊断且不返回部分 MIR。输入 HIR 只能由 `build_hir` 成功产生，因此本函数不
/// 重复执行文本符号解析。
pub(crate) fn lower_to_mir(
    unit: &CompilationUnit,
    hir: &HirUnit,
) -> Result<MirUnit, DiagnosticBundle> {
    // MIR record 指标只计语义车道图边与连接；模块元数据仍计入分配和 live-byte 预检。
    // 在任何阶段表分配前先验证记录、暂存映射和 HIR/MIR 同时存续的峰值。
    let module_count = u64::try_from(hir.modules.len()).unwrap_or(u64::MAX);
    let lane_edge_count = u64::try_from(hir.lane_edges.len()).unwrap_or(u64::MAX);
    let connection_count = u64::try_from(hir.lane_edge_references.len()).unwrap_or(u64::MAX);
    let mir_record_count = lane_edge_count.saturating_add(connection_count);
    let stage_scratch_bytes = requested_bytes::<MirModuleKey>(module_count)
        .saturating_add(requested_bytes::<MirLaneEdgeKey>(lane_edge_count));
    let mir_owned_bytes = requested_bytes::<MirModule>(module_count)
        .saturating_add(requested_bytes::<MirLaneEdge>(lane_edge_count))
        .saturating_add(requested_bytes::<MirLaneEdgeConnection>(connection_count));
    let controlled_live_bytes = unit
        .controlled_live_bytes
        .saturating_add(hir.controlled_live_bytes)
        .saturating_add(mir_owned_bytes)
        .saturating_add(stage_scratch_bytes);
    let primary_span = hir.modules.first().map(|module| module.source_span.clone());
    let stable_key = hir
        .modules
        .first()
        .map(|module| module.authoring_namespace_id.as_ref().into());
    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    for (dimension, observed) in [
        (CompileLimitDimension::MirRecordCount, mir_record_count),
        (
            CompileLimitDimension::StageScratchBytes,
            stage_scratch_bytes,
        ),
        (
            CompileLimitDimension::CompilerControlledLiveBytes,
            controlled_live_bytes,
        ),
    ] {
        if observed > unit.limits.value(dimension) {
            diagnostics.push(Diagnostic::compile_limit_exceeded_at(
                dimension,
                unit.limits.value(dimension),
                observed,
                primary_span.clone(),
                stable_key.clone(),
            ));
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    // 不依赖 HIR/MIR raw key 数值碰巧一致：每次插入都记录显式 stage-to-stage 映射。
    let mut modules = TypedArena::<MirModuleTag, MirModule>::with_capacity(hir.modules.len());
    let mut hir_module_to_mir = Vec::with_capacity(hir.modules.len());
    for module in &hir.modules {
        let mir_key = modules
            .push(MirModule {
                authoring_namespace_id: Arc::clone(&module.authoring_namespace_id),
                source_document_key: Arc::clone(&module.source_document_key),
                source_span: module.source_span.clone(),
            })
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, primary_span.clone()))?;
        hir_module_to_mir.push(mir_key);
    }

    let edge_capacity = usize::try_from(lane_edge_count)
        .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, primary_span.clone()))?;
    let connection_capacity = usize::try_from(connection_count)
        .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, primary_span.clone()))?;
    let mut lane_edges = TypedArena::<MirLaneEdgeTag, MirLaneEdge>::with_capacity(edge_capacity);
    let mut hir_to_mir = Vec::with_capacity(edge_capacity);
    for edge in &hir.lane_edges {
        let module = hir_module_to_mir[edge.module.index()];
        let mir_key = lane_edges
            .push(MirLaneEdge {
                module,
                stable_key: Arc::clone(&edge.stable_key),
                length_meters: edge.length_meters,
                speed_limit_meters_per_second: edge.speed_limit_meters_per_second,
                connections: TableRange::empty(),
                source_span: edge.source_span.clone(),
            })
            .map_err(|overflow| {
                arena_overflow(overflow, &unit.limits, Some(edge.source_span.clone()))
            })?;
        hir_to_mir.push(mir_key);
    }

    // 按 HIR 的规范边顺序追加连接，并以 TableRange 记录每条边的连续片段；这样后续遍历
    // 不需要哈希查找或每边独立分配。
    let mut connections = Vec::with_capacity(connection_capacity);
    for (hir_index, edge) in hir.lane_edges.iter().enumerate() {
        let mir_key = hir_to_mir[hir_index];
        let start = connections.len();
        for reference in &hir.lane_edge_references[edge.successors.as_usize_range()] {
            connections.push(MirLaneEdgeConnection {
                target: mir_key_for_hir(reference.target, &hir_to_mir),
                source_span: reference.source_span.clone(),
            });
        }
        lane_edges.get_mut(mir_key).connections =
            TableRange::try_from_usize(start, connections.len().saturating_sub(start)).map_err(
                |overflow| arena_overflow(overflow, &unit.limits, Some(edge.source_span.clone())),
            )?;
    }

    debug_assert_eq!(modules.len(), hir.modules.len());
    debug_assert_eq!(lane_edges.len(), edge_capacity);
    debug_assert_eq!(connections.len(), connection_capacity);
    Ok(MirUnit {
        modules: modules.into_boxed_slice(),
        lane_edges: lane_edges.into_boxed_slice(),
        lane_edge_connections: connections.into_boxed_slice(),
        mir_record_count,
        controlled_live_bytes: mir_owned_bytes,
    })
}

fn mir_key_for_hir(key: HirLaneEdgeKey, mapping: &[MirLaneEdgeKey]) -> MirLaneEdgeKey {
    mapping[key.index()]
}

fn requested_bytes<T>(count: u64) -> u64 {
    count.saturating_mul(u64::try_from(size_of::<T>()).unwrap_or(u64::MAX))
}

fn arena_overflow(
    _: ArenaKeyOverflow,
    limits: &crate::CompileLimits,
    primary_span: Option<SourceSpan>,
) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
        CompileLimitDimension::MirRecordCount,
        limits.value(CompileLimitDimension::MirRecordCount),
        u64::from(u32::MAX) + 1,
        primary_span,
        None,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::build_hir;
    use crate::{
        CompilationUnitBuilder, CompileLimits, DiagnosticPayload, LaneEdgeInput, LaneEdgeReference,
        SourceModuleHeader, SourceModuleHeaderInput, SyntheticModule, SyntheticModuleBuilder,
    };

    fn module(
        namespace: &str,
        imports: &[&str],
        edges: &[(&str, &[LaneEdgeReference<'_>])],
    ) -> SyntheticModule {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: namespace,
                source_document_key: namespace,
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: Some(42),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .unwrap();
        let mut builder = SyntheticModuleBuilder::new(header, &limits).unwrap();
        for import in imports {
            builder.add_import(import).unwrap();
        }
        for (key, successors) in edges {
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: 12.5,
                    speed_limit_meters_per_second: 13.75,
                    successors,
                })
                .unwrap();
        }
        builder.finish().unwrap()
    }

    fn unit(modules: impl IntoIterator<Item = SyntheticModule>) -> CompilationUnit {
        let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
        for module in modules {
            builder.add_synthetic_module(module).unwrap();
        }
        builder.build().unwrap()
    }

    fn projection(mir: &MirUnit) -> Vec<(String, String, Vec<u32>)> {
        mir.lane_edges
            .iter()
            .map(|edge| {
                (
                    mir.modules[edge.module.index()]
                        .authoring_namespace_id
                        .to_string(),
                    edge.stable_key.to_string(),
                    mir.lane_edge_connections[edge.connections.as_usize_range()]
                        .iter()
                        .map(|connection| connection.target.raw())
                        .collect(),
                )
            })
            .collect()
    }

    #[test]
    fn mir_freezes_resolved_lane_edges_and_flat_connection_ranges() {
        let app_successors = [
            LaneEdgeReference::imported("city/base", "edge-b"),
            LaneEdgeReference::local("edge-c"),
        ];
        let unit = unit([
            module(
                "city/app",
                &["city/base"],
                &[("edge-c", &[]), ("edge-a", &app_successors)],
            ),
            module("city/base", &[], &[("edge-b", &[])]),
        ]);
        let hir = build_hir(&unit).unwrap();
        let mir = lower_to_mir(&unit, &hir).unwrap();

        assert_eq!(mir.modules.len(), 2);
        assert_eq!(mir.lane_edges.len(), 3);
        assert_eq!(mir.lane_edge_connections.len(), 2);
        assert_eq!(mir.mir_record_count, 5);
        assert_eq!(mir.modules[1].source_document_key.as_ref(), "city/app");
        assert_eq!(mir.lane_edges[1].length_meters, 12.5);
        assert_eq!(mir.lane_edges[1].speed_limit_meters_per_second, 13.75);
        assert_eq!(
            mir.lane_edges[1].source_span.source_document_key(),
            "city/app"
        );
        assert_eq!(
            mir.lane_edge_connections[0]
                .source_span
                .source_document_key(),
            "city/app"
        );
        assert_eq!(
            projection(&mir),
            [
                ("city/base".into(), "edge-b".into(), vec![]),
                ("city/app".into(), "edge-a".into(), vec![2, 0]),
                ("city/app".into(), "edge-c".into(), vec![]),
            ]
        );
    }

    #[test]
    fn mir_topology_is_identical_after_declaration_permutation() {
        let successors = [
            LaneEdgeReference::local("edge-c"),
            LaneEdgeReference::local("edge-b"),
        ];
        let left_unit = unit([module(
            "city/a",
            &[],
            &[("edge-a", &successors), ("edge-b", &[]), ("edge-c", &[])],
        )]);
        let right_unit = unit([module(
            "city/a",
            &[],
            &[("edge-c", &[]), ("edge-a", &successors), ("edge-b", &[])],
        )]);
        let left_hir = build_hir(&left_unit).unwrap();
        let right_hir = build_hir(&right_unit).unwrap();
        let left = lower_to_mir(&left_unit, &left_hir).unwrap();
        let right = lower_to_mir(&right_unit, &right_hir).unwrap();

        assert_eq!(projection(&left), projection(&right));
        assert_eq!(left.mir_record_count, right.mir_record_count);
    }

    #[test]
    fn mir_checks_record_scratch_and_live_byte_limits_before_stage_allocation() {
        let successors = [LaneEdgeReference::local("edge-a")];
        let mut unit = unit([module("city/a", &[], &[("edge-a", &successors)])]);
        let hir = build_hir(&unit).unwrap();

        unit.limits = CompileLimits::p100_initial_v1().with_test_pipeline_limits(
            u32::MAX,
            1,
            u32::MAX,
            u32::MAX,
        );
        let record_failure = match lower_to_mir(&unit, &hir) {
            Ok(_) => panic!("MIR record limit must fail closed"),
            Err(diagnostics) => diagnostics,
        };
        assert!(matches!(
            record_failure.diagnostics()[0].payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::MirRecordCount,
                limit: 1,
                observed: 2,
            }
        ));

        unit.limits = CompileLimits::p100_initial_v1().with_test_pipeline_limits(
            u32::MAX,
            u32::MAX,
            0,
            u32::MAX,
        );
        let scratch_failure = match lower_to_mir(&unit, &hir) {
            Ok(_) => panic!("MIR scratch limit must fail closed"),
            Err(diagnostics) => diagnostics,
        };
        assert!(
            scratch_failure
                .diagnostics()
                .iter()
                .any(|diagnostic| matches!(
                    diagnostic.payload(),
                    DiagnosticPayload::CompileLimitExceeded {
                        dimension: CompileLimitDimension::StageScratchBytes,
                        limit: 0,
                        observed,
                    } if *observed > 0
                ))
        );

        let input_live_bytes =
            u32::try_from(unit.controlled_live_bytes + hir.controlled_live_bytes).unwrap();
        unit.limits = CompileLimits::p100_initial_v1().with_test_pipeline_limits(
            u32::MAX,
            u32::MAX,
            u32::MAX,
            input_live_bytes,
        );
        let live_failure = match lower_to_mir(&unit, &hir) {
            Ok(_) => panic!("MIR live byte limit must fail closed"),
            Err(diagnostics) => diagnostics,
        };
        assert!(live_failure.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::CompilerControlledLiveBytes,
                limit,
                observed,
            } if *limit == u64::from(input_live_bytes) && observed > limit
        )));
    }
}
