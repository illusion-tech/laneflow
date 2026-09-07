use std::alloc::System;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Instant;

use laneflow_compiler::{
    CompilationOutput, CompileLimits, PortableDiffBase, PortableEmissionProvenance,
    check_portable_candidate, emit_portable_candidate_to_staging,
};
use laneflow_format::{FormatLimits, RegistryCheckedFieldValue, ValueCheckedObjectView};
use laneflow_static_contract::{
    EntityKind, PortableFieldType, PortableObjectKind, portable_object_schema,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};
use serde::Serialize;
use stats_alloc::{Stats, StatsAlloc};

use crate::{
    Result, Scale, UrbanConfig, catalog, compile_source, generate_source, source, validation,
};

#[derive(Debug, Serialize)]
pub struct TableCount {
    pub section: String,
    pub table: String,
    pub rows: u32,
    pub chunks: u32,
    pub max_chunk_rows: u32,
    pub max_chunk_bytes: u64,
    pub nested_rows: BTreeMap<String, u64>,
}

#[derive(Debug, Serialize)]
pub struct FileDigest {
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Serialize)]
pub struct Manifest {
    pub manifest_version: u32,
    pub generator: String,
    pub scale: String,
    pub cells: usize,
    pub tiles: u32,
    pub tile_columns: u32,
    pub nominal_individuals: u32,
    pub fixed_step_ms: u64,
    pub compile_limits: String,
    pub format_limits: String,
    pub shared_build_limits: BTreeMap<String, u64>,
    pub network_revision: String,
    pub files: BTreeMap<String, FileDigest>,
    pub source_declarations: BTreeMap<String, u64>,
    pub source_references: BTreeMap<String, u64>,
    pub source_import_edges: Vec<String>,
    pub lir_counts: BTreeMap<String, u64>,
    pub lfca_tables: Vec<TableCount>,
    pub lfsm_tables: Vec<TableCount>,
    pub lir_logical_records: u64,
    pub compiler_output_logical_bytes: u64,
    pub compiler_controlled_peak_bytes: u64,
    pub shared_headless_retained_bytes: u64,
    pub shared_spatial_retained_bytes: u64,
    pub resolved_policy_gate_rows: u64,
    pub resolved_policy_stream_rows: u64,
    pub hir_mir_counts: String,
    pub lane_length_mm: u64,
    pub road_alignment_length_mm: u64,
    pub bounds_min_meters: [f32; 3],
    pub bounds_max_meters: [f32; 3],
    pub installed_entities: BTreeMap<String, u32>,
    pub route_categories: BTreeMap<String, u64>,
    pub route_edge_occurrences: u64,
    pub route_conflict_occurrences: u64,
    pub appearance_catalog: String,
    pub parking_capacity_by_tile: BTreeMap<String, u64>,
    pub checks: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct Measurements {
    phases: BTreeMap<String, PhaseMeasurement>,
    memory_note: String,
}

#[derive(Serialize)]
struct PhaseMeasurement {
    elapsed_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    allocation_calls: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    allocated_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    requested_live_delta_bytes: Option<i128>,
}

struct Recorder<'a> {
    allocator: Option<&'a StatsAlloc<System>>,
    start: Instant,
    heap_start: Option<Stats>,
    phases: BTreeMap<String, PhaseMeasurement>,
}

impl<'a> Recorder<'a> {
    fn new(allocator: Option<&'a StatsAlloc<System>>) -> Self {
        Self {
            allocator,
            start: Instant::now(),
            heap_start: allocator.map(StatsAlloc::stats),
            phases: BTreeMap::new(),
        }
    }
    fn begin(&mut self) {
        self.start = Instant::now();
        self.heap_start = self.allocator.map(StatsAlloc::stats);
    }
    fn finish(&mut self, name: &str) {
        let elapsed_ms = self.start.elapsed().as_millis();
        let stats = self
            .allocator
            .zip(self.heap_start)
            .map(|(a, b)| a.stats() - b);
        if self.allocator.is_some() {
            eprintln!("{name}: {elapsed_ms} ms");
        }
        self.phases.insert(
            name.into(),
            PhaseMeasurement {
                elapsed_ms,
                allocation_calls: stats.map(|s| s.allocations),
                allocated_bytes: stats.map(|s| s.bytes_allocated),
                requested_live_delta_bytes: stats
                    .map(|s| s.bytes_allocated as i128 - s.bytes_deallocated as i128),
            },
        );
    }
}

/// Writes a fresh local result directory. Failed attempts retain their source inputs for diagnosis.
/// Timings are kept outside the byte-compared canonical manifest.
pub fn generate(
    config: &UrbanConfig,
    scale: Scale,
    directory: &Path,
    allocator: Option<&StatsAlloc<System>>,
) -> Result<Manifest> {
    config.validate()?;
    fs::create_dir(directory)?;
    let mut recorder = Recorder::new(allocator);
    let generated = generate_source(config, scale)?;
    recorder.finish("source");
    let mut files = BTreeMap::new();
    write(
        directory,
        "config.toml",
        toml::to_string_pretty(config)?.as_bytes(),
        &mut files,
    )?;
    write(
        directory,
        "common.lfre",
        generated.common.as_bytes(),
        &mut files,
    )?;
    write(
        directory,
        "topology.lfre",
        generated.topology.as_bytes(),
        &mut files,
    )?;
    recorder.begin();
    let output = compile_source(&generated)?;
    recorder.finish("compile");
    let metrics = output.metrics();
    let lir_counts = lir_counts(&output);
    recorder.begin();
    let staging = tempfile::tempdir()?;
    let provenance = PortableEmissionProvenance::try_new(source::BUILD_ID)
        .map_err(|e| validation("provenance", e))?;
    let candidate = emit_portable_candidate_to_staging(
        &output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
        staging.path(),
    )
    .map_err(|e| validation("file-backed emission", e))?;
    recorder.finish("emit");
    drop(output);
    recorder.begin();
    let checked = check_portable_candidate(candidate, FormatLimits::HARD)
        .map_err(|e| validation("post-emission", e))?;
    recorder.finish("post-emission");
    write(
        directory,
        "network.lfca",
        checked.canonical_artifact_view().bytes(),
        &mut files,
    )?;
    write(
        directory,
        "source-map.lfsm",
        checked.source_map_view().bytes(),
        &mut files,
    )?;
    write(
        directory,
        "genesis.lfsd",
        checked.semantic_diff_view().bytes(),
        &mut files,
    )?;
    let lfca_tables = table_counts(
        checked.canonical_artifact_view(),
        PortableObjectKind::CanonicalArtifact,
    );
    let lfsm_tables = table_counts(checked.source_map_view(), PortableObjectKind::SourceMap);
    let canonical_input = checked.canonical_network_input();
    let limits =
        SharedNetworkBuildLimits::new(2 * 1_024 * 1_024 * 1_024, 2 * 1_024 * 1_024 * 1_024);
    recorder.begin();
    let headless = build_shared_network_revision(
        canonical_input.clone(),
        SharedNetworkBuildOptions::new(SpatialBuildOption::Omit, limits),
    )
    .map_err(|e| validation("headless build", e))?;
    let headless_bytes = headless.retained_logical_bytes();
    recorder.finish("headless-build");
    recorder.begin();
    let revision = build_shared_network_revision(
        canonical_input,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, limits),
    )
    .map_err(|e| validation("spatial build", e))?;
    recorder.finish("spatial-build");
    let catalog = catalog::build(&generated, config, &revision)?;
    recorder.begin();
    let headless_world = catalog::install(headless, &catalog, scale)?;
    recorder.finish("headless-install");
    drop(headless_world);
    recorder.begin();
    let mut world = catalog::install(revision.clone(), &catalog, scale)?;
    recorder.finish("spatial-install");
    recorder.begin();
    catalog::validate(&generated, &catalog, &mut world)?;
    recorder.finish("routes-and-topology");
    let mut route_categories = BTreeMap::new();
    for route in &catalog.routes {
        *route_categories.entry(route.category.clone()).or_default() += 1;
    }
    let pose = revision
        .spatial()
        .and_then(|s| s.lane_pose())
        .ok_or_else(|| validation("spatial", "missing geometry"))?;
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for raw in 0..revision.traffic().lane_edge_count() {
        let geometry = pose
            .lane_geometry(laneflow_static_contract::LaneEdgeOrdinal::from_raw(raw))
            .expect("compiled lane geometry");
        for p in geometry.points() {
            for (i, v) in [p.x, p.y, p.z].into_iter().enumerate() {
                min[i] = min[i].min(v);
                max[i] = max[i].max(v);
            }
        }
    }
    let spatial = revision.spatial().expect("retained spatial root");
    for raw in 0..revision.identity().entity_count(EntityKind::ConflictZone) {
        let region = spatial
            .conflict_zone_region(laneflow_static_contract::ConflictZoneOrdinal::from_raw(raw))
            .expect("declared conflict region");
        let (low, high) = region.height_range();
        for p in region.ring_xz() {
            for (i, v) in [p.x, low, p.z].into_iter().enumerate() {
                min[i] = min[i].min(v);
            }
            for (i, v) in [p.x, high, p.z].into_iter().enumerate() {
                max[i] = max[i].max(v);
            }
        }
    }
    // Extract real explicit-bay poses through Spatial, then include each rectangle's corners.
    let mut session = laneflow_spatial::SpatialSession::bind(revision.clone())
        .map_err(|e| validation("Spatial bind", e))?
        .ok_or_else(|| validation("Spatial bind", "missing spatial root"))?;
    let inputs: Vec<_> = (0..revision.identity().entity_count(EntityKind::ParkingSpace))
        .map(|raw| {
            laneflow_spatial::PoseInput::parking(
                laneflow_spatial::PoseRecordId::new(raw),
                laneflow_static_contract::ParkingSpaceOrdinal::from_raw(raw),
            )
        })
        .collect();
    let mut poses = laneflow_spatial::CanonicalPoseBatch::new();
    session
        .extract_pose_batch(
            laneflow_spatial::FramePlacementToken::new(542),
            &inputs,
            &mut poses,
        )
        .map_err(|e| validation("parking geometry", e))?;
    for record in poses.records() {
        let pose = record.pose();
        let p = pose.position();
        let t = pose.tangent();
        let (_, _, length, width) = revision
            .traffic()
            .relations()
            .parking_space_geometry(laneflow_static_contract::ParkingSpaceOrdinal::from_raw(
                record.record().raw(),
            ))
            .expect("compiled bay");
        for along in [-1.0, 1.0] {
            for lateral in [-1.0, 1.0] {
                let a = along * length as f32 / 2_000.0;
                let b = lateral * width as f32 / 2_000.0;
                for (i, v) in [
                    p.x() + a * t.x() - b * t.z(),
                    p.y(),
                    p.z() + a * t.z() + b * t.x(),
                ]
                .into_iter()
                .enumerate()
                {
                    min[i] = min[i].min(v);
                    max[i] = max[i].max(v);
                }
            }
        }
    }
    if min.into_iter().any(|v| v < -16_384.0) || max.into_iter().any(|v| v > 16_384.0) {
        return Err(validation("frame bounds", (min, max)));
    }
    let installed_entities = EntityKind::ALL
        .into_iter()
        .map(|kind| {
            (
                kind.slug().to_owned(),
                revision.identity().entity_count(kind),
            )
        })
        .collect();
    let mut checks = BTreeMap::new();
    for name in [
        "compile",
        "post-emission",
        "shared-headless",
        "shared-spatial",
        "empty-world-headless",
        "empty-world-spatial",
        "single-weak-component",
        "directed-successors",
        "geometry-continuity",
        "route-registration",
        "parking-route-coverage",
        "explicit-parking-spatial-geometry",
    ] {
        checks.insert(name.into(), "pass".into());
    }
    checks.insert(
        "active-individual-runtime".into(),
        "not run; owned by #544".into(),
    );
    write(
        directory,
        "routes.toml",
        toml::to_string_pretty(&catalog)?.as_bytes(),
        &mut files,
    )?;
    let manifest = Manifest {
        manifest_version: 1,
        generator: source::BUILD_ID.into(),
        scale: scale.name().into(),
        cells: generated.layout.cells.len(),
        tiles: scale.tile_count(),
        tile_columns: generated.layout.tile_columns,
        nominal_individuals: scale.nominal_individual_count(),
        fixed_step_ms: scale.fixed_step_ms(),
        compile_limits: CompileLimits::single_network_1m_v2().profile_id().into(),
        format_limits: "FormatLimits::HARD".into(),
        shared_build_limits: BTreeMap::from([
            ("max_retained_bytes".into(), limits.max_retained_bytes()),
            ("max_scratch_bytes".into(), limits.max_scratch_bytes()),
            ("max_policy_work".into(), limits.max_policy_work()),
        ]),
        network_revision: catalog.network_revision,
        files,
        source_declarations: generated.declarations,
        source_references: generated.references,
        source_import_edges: vec![format!(
            "{} -> {}",
            source::NAMESPACE,
            source::COMMON_NAMESPACE
        )],
        lir_counts,
        lfca_tables,
        lfsm_tables,
        lir_logical_records: metrics.lir_record_count(),
        compiler_output_logical_bytes: metrics.output_logical_bytes(),
        compiler_controlled_peak_bytes: metrics.compiler_controlled_peak_bytes(),
        shared_headless_retained_bytes: headless_bytes,
        shared_spatial_retained_bytes: revision.retained_logical_bytes(),
        resolved_policy_gate_rows: (0..revision.identity().entity_count(EntityKind::ManeuverGate))
            .map(|raw| {
                revision
                    .policy()
                    .policy(laneflow_static_contract::RightOfWayPolicySetOrdinal::from_raw(0))
                    .expect("the declared policy")
                    .gate_classes(laneflow_static_contract::ManeuverGateOrdinal::from_raw(raw))
                    .len() as u64
            })
            .sum(),
        resolved_policy_stream_rows: (0..revision
            .identity()
            .entity_count(EntityKind::ParticipantStream))
            .map(|raw| {
                revision
                    .policy()
                    .policy(laneflow_static_contract::RightOfWayPolicySetOrdinal::from_raw(0))
                    .expect("the declared policy")
                    .stream_classes(
                        laneflow_static_contract::ParticipantStreamOrdinal::from_raw(raw),
                    )
                    .len() as u64
            })
            .sum(),
        hir_mir_counts: "unmeasured: private IR; accepted under the unchanged compiler profile"
            .into(),
        lane_length_mm: revision
            .traffic()
            .lane_lengths_millimetres()
            .iter()
            .map(|&n| u64::from(n))
            .sum(),
        road_alignment_length_mm: generated
            .edges
            .values()
            .filter(|e| e.key.ends_with(".in") || e.key.ends_with(".out"))
            .map(|e| {
                ((e.end[0] - e.start[0]).hypot(e.end[1] - e.start[1]) * 1_000.0).round() as u64
            })
            .sum(),
        bounds_min_meters: min,
        bounds_max_meters: max,
        installed_entities,
        route_categories,
        route_edge_occurrences: world.config().route_edge_occurrence_capacity(),
        route_conflict_occurrences: world.config().route_conflict_occurrence_capacity(),
        appearance_catalog: "host-owned; no appearance definitions emitted".into(),
        parking_capacity_by_tile: catalog.parking_capacity_by_tile,
        checks,
    };
    fs::write(
        directory.join("manifest.toml"),
        toml::to_string_pretty(&manifest)?,
    )?;
    fs::write(directory.join("measurements.toml"),toml::to_string_pretty(&Measurements {phases:recorder.phases,
        memory_note:"Optional instrumented allocator reports requested live byte deltas and total allocated bytes per phase, not heap peak or OS working set. Compiler controlled peak and shared-root logical bytes are reported separately in manifest.".into()})?)?;
    Ok(manifest)
}

fn write(
    directory: &Path,
    name: &str,
    bytes: &[u8],
    files: &mut BTreeMap<String, FileDigest>,
) -> Result<()> {
    fs::write(directory.join(name), bytes)?;
    files.insert(
        name.into(),
        FileDigest {
            bytes: bytes.len() as u64,
            sha256: source::digest(bytes),
        },
    );
    Ok(())
}

fn lir_counts(output: &CompilationOutput) -> BTreeMap<String, u64> {
    let lir = output.lir();
    let mut counts = BTreeMap::new();
    macro_rules! count { ($($method:ident),* $(,)?) => { $(counts.insert(stringify!($method).into(),lir.$method().len() as u64);)* }; }
    count!(
        lane_edges,
        road_corridors,
        road_sections,
        authoring_lanes,
        lane_groups,
        facility_bands,
        junctions,
        movements,
        maneuver_paths,
        stop_lines,
        maneuver_gates,
        waiting_zones,
        signal_groups,
        signal_controllers,
        signal_phases,
        parking_facilities,
        parking_spaces,
        participant_classes,
        vehicle_profiles,
        canonical_frames,
        access_rules,
        junction_internal_edges
    );
    counts.insert(
        "lane_successors".into(),
        lir.lane_edges().map(|e| e.successors().len() as u64).sum(),
    );
    counts.insert(
        "path_edge_occurrences".into(),
        lir.maneuver_paths().map(|p| p.edges().len() as u64).sum(),
    );
    counts
}

fn table_counts(view: ValueCheckedObjectView<'_>, kind: PortableObjectKind) -> Vec<TableCount> {
    let mut output = Vec::new();
    for (section, schema) in view
        .registry_view()
        .sections()
        .zip(portable_object_schema(kind).sections)
    {
        for (table, definition) in section.tables().zip(schema.tables) {
            let mut max_chunk_rows = 0;
            let mut max_chunk_bytes = 0;
            for i in 0..table.chunk_count() {
                max_chunk_rows =
                    max_chunk_rows.max(table.chunk_row_count(i).expect("checked chunk"));
                max_chunk_bytes =
                    max_chunk_bytes.max(table.chunk_exact_byte_length(i).expect("checked chunk"));
            }
            let mut nested_rows = BTreeMap::new();
            for field in definition.row.fields {
                if field.field_type == PortableFieldType::RecordVector {
                    let count = table
                        .rows()
                        .map(|row| {
                            let Some(value) = row.field_by_tag(field.tag) else {
                                return 0;
                            };
                            match value.value().expect("checked field") {
                                RegistryCheckedFieldValue::RecordVector(values) => {
                                    u64::from(values.len())
                                }
                                _ => 0,
                            }
                        })
                        .sum();
                    nested_rows.insert(field.name.into(), count);
                }
            }
            output.push(TableCount {
                section: schema.name.into(),
                table: definition.name.into(),
                rows: table.row_count(),
                chunks: table.chunk_count(),
                max_chunk_rows,
                max_chunk_bytes,
                nested_rows,
            });
        }
    }
    output
}
