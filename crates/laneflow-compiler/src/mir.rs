use std::sync::Arc;

use crate::arena::{ArenaKey, ArenaKeyOverflow, TableRange, TypedArena};
use crate::diagnostic::DiagnosticCollector;
use crate::hir::{HirLaneEdgeKey, HirUnit};
use crate::{CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceSpan};

pub(crate) enum MirModuleTag {}
pub(crate) enum MirLaneEdgeTag {}

pub(crate) type MirModuleKey = ArenaKey<MirModuleTag>;
pub(crate) type MirLaneEdgeKey = ArenaKey<MirLaneEdgeTag>;

pub(crate) struct MirModule {
    pub(crate) authoring_namespace_id: Arc<str>,
    pub(crate) source_document_key: Arc<str>,
    pub(crate) source_span: SourceSpan,
}

pub(crate) struct MirLaneEdgeConnection {
    pub(crate) target: MirLaneEdgeKey,
    pub(crate) source_span: SourceSpan,
}

pub(crate) struct MirLaneEdge {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) length_meters: f64,
    pub(crate) speed_limit_meters_per_second: f64,
    pub(crate) connections: TableRange<MirLaneEdgeConnection>,
    pub(crate) source_span: SourceSpan,
}

pub(crate) struct MirUnit {
    pub(crate) modules: Box<[MirModule]>,
    pub(crate) lane_edges: Box<[MirLaneEdge]>,
    pub(crate) lane_edge_connections: Box<[MirLaneEdgeConnection]>,
    pub(crate) mir_record_count: u64,
    pub(crate) controlled_live_bytes: u64,
}

pub(crate) fn lower_to_mir(
    unit: &CompilationUnit,
    hir: &HirUnit,
) -> Result<MirUnit, DiagnosticBundle> {
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
