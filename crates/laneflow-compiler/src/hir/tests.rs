use super::*;
use crate::declaration::{
    CompiledFacilityBandGeometry, CompiledGeometrySourceRange, EdgeLength, OwnedEntityReference,
};
use crate::lir::freeze_lir;
use crate::mir::lower_to_mir;
use crate::{
    AccessRegulationInput, AccessRuleInput, AccessRuleTargetInput, AuthoringLaneInput,
    CanonicalFrameInput, CompilationUnitBuilder, CompileLimits, CorridorElementReference,
    DiagnosticCode, DiagnosticPayload, FacilityBandInput, FacilityBandReference,
    GeometryAccuracyProfile, GeometryDirectionProfile, IidmVehicleProfileInput, JunctionInput,
    JunctionReference, LaneEdgeGeometryInput, LaneEdgeInput, LaneEdgeReference, LaneGroupInput,
    LaneGroupReference, ManeuverGateInput, ManeuverGateReference, ManeuverPathInput,
    ManeuverPathReference, MovementInput, MovementReference, ParkingAreaInput,
    ParkingAreaReference, ParkingLaneAnchorInput, ParkingSpaceGeometryInput, ParkingSpaceInput,
    ParticipantClassInput, ParticipantClassReference, RoadCorridorInput, RoadSectionInput,
    RoadSectionReference, SignalControlInput, SignalControllerInput, SignalGroupInput,
    SignalGroupReference, SignalGroupStateInput, SignalPhaseInput, SourceModuleHeader,
    SourceModuleHeaderInput, SourceSpan, StopLineInput, StopLineReference, SyntheticModule,
    SyntheticModuleBuilder, VehicleProfileInput, WaitingZoneInput,
};
use laneflow_static_contract::{
    CanonicalFrameKind, LaneEdgeKind, PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM, millimetres_from_si,
};

fn header(namespace: &str) -> SourceModuleHeader {
    SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: namespace,
            source_document_key: namespace,
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: Some(42),
            provenance: "repository:laneflow",
        },
        &CompileLimits::p100_initial_v1(),
    )
    .unwrap()
}

#[test]
fn spatial_join_distance_uses_the_canonical_f32_predicate() {
    let end = HirCanonicalPoint3F32 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    let start = HirCanonicalPoint3F32 {
        x: 0.000_528_320_15,
        y: 0.004_972_009_5,
        z: 0.0,
    };

    assert_eq!(canonical_point_distance(end, start), 0.005_f32);
    assert!(canonical_point_distance(end, start) <= SPATIAL_JOIN_POSITION_TOLERANCE_METERS);
}

fn module(
    namespace: &str,
    imports: &[&str],
    edges: &[(&str, &[LaneEdgeReference<'_>])],
) -> SyntheticModule {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(header(namespace), &limits).unwrap();
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

fn install_compiled_lane_geometries(
    unit: &mut CompilationUnit,
    module_namespace: &str,
    profiles: GeometryCompilationProfiles,
    mut geometry: impl FnMut(&str) -> (Option<(&str, &str)>, Vec<CanonicalPoint3F32Input>),
) {
    let module = unit
        .modules
        .iter_mut()
        .find(|module| module.descriptor().authoring_namespace_id() == module_namespace)
        .expect("test module must exist");
    module.geometry_profiles = Some(profiles);
    for declaration in &mut module.declarations {
        let TypedAstDeclaration::LaneEdge(edge) = declaration else {
            continue;
        };
        let (frame, points) = geometry(&edge.header.stable_key);
        let length = points
            .windows(2)
            .map(|pair| {
                let x = f64::from(pair[1].x) - f64::from(pair[0].x);
                let y = f64::from(pair[1].y) - f64::from(pair[0].y);
                let z = f64::from(pair[1].z) - f64::from(pair[0].z);
                x.hypot(y).hypot(z)
            })
            .sum::<f64>();
        edge.geometry_authority =
            LaneEdgeGeometryAuthority::Compiled(crate::declaration::CompiledLaneEdgeGeometry {
                length: crate::declaration::EdgeLength::try_new(length).unwrap(),
                canonical_frame: frame.map(|(namespace, key)| {
                    OwnedEntityReference::<CanonicalFrameKind>::new(
                        Arc::from(namespace),
                        Arc::from(key),
                        edge.header.span.clone(),
                    )
                }),
                source_ranges: Box::new([CompiledGeometrySourceRange {
                    point_start: 0,
                    point_end_exclusive: u32::try_from(points.len()).unwrap(),
                    source_segment_ordinal: 0,
                    source: edge.header.span.clone(),
                }]),
                centerline_points: points.into_boxed_slice(),
            });
    }
}

fn install_compiled_facility_geometry(
    unit: &mut CompilationUnit,
    module_namespace: &str,
    facility_band_key: &str,
    frame_namespace: &str,
    frame_key: &str,
    points: Vec<CanonicalPoint3F32Input>,
) {
    let length = points
        .windows(2)
        .map(|pair| {
            let x = f64::from(pair[1].x) - f64::from(pair[0].x);
            let y = f64::from(pair[1].y) - f64::from(pair[0].y);
            let z = f64::from(pair[1].z) - f64::from(pair[0].z);
            x.hypot(y).hypot(z)
        })
        .sum::<f64>();
    let module = unit
        .modules
        .iter_mut()
        .find(|module| module.descriptor().authoring_namespace_id() == module_namespace)
        .expect("test module must exist");
    let band = module
        .declarations
        .iter_mut()
        .find_map(|declaration| match declaration {
            TypedAstDeclaration::FacilityBand(band)
                if band.header.stable_key.as_ref() == facility_band_key =>
            {
                Some(band)
            }
            _ => None,
        })
        .expect("test FacilityBand must exist");
    band.compiled_geometry = Some(CompiledFacilityBandGeometry {
        length: EdgeLength::try_new(length).unwrap(),
        canonical_frame: OwnedEntityReference::<CanonicalFrameKind>::new(
            Arc::from(frame_namespace),
            Arc::from(frame_key),
            band.header.span.clone(),
        ),
        source_ranges: Box::new([CompiledGeometrySourceRange {
            point_start: 0,
            point_end_exclusive: u32::try_from(points.len()).unwrap(),
            source_segment_ordinal: 0,
            source: band.header.span.clone(),
        }]),
        centerline_points: points.into_boxed_slice(),
    });
}

fn point(x: f32, y: f32, z: f32) -> CanonicalPoint3F32Input {
    CanonicalPoint3F32Input { x, y, z }
}

fn compiled_junction_unit(conflicting_frames: bool) -> CompilationUnit {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(header("city/junction"), &limits).unwrap();
    let entry_successors = [LaneEdgeReference::local("internal")];
    let internal_successors = [
        LaneEdgeReference::local("exit-a"),
        LaneEdgeReference::local("exit-b"),
    ];
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry-a",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &entry_successors,
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry-b",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &entry_successors,
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "internal",
            length_meters: 8.0,
            speed_limit_meters_per_second: 8.0,
            successors: &internal_successors,
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit-a",
            length_meters: 12.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit-b",
            length_meters: 12.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame-a",
            lane_edge_geometries: &[],
        })
        .unwrap()
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame-b",
            lane_edge_geometries: &[],
        })
        .unwrap()
        .add_junction(JunctionInput {
            junction_key: "junction-main",
        })
        .unwrap()
        .add_movement(MovementInput {
            movement_key: "movement-through",
            junction: JunctionReference::local("junction-main"),
            directed_entry_approach_key: "approach-westbound",
            directed_exit_approach_key: "approach-eastbound",
        })
        .unwrap();
    let internal = [LaneEdgeReference::local("internal")];
    builder
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path-a",
            movement: MovementReference::local("movement-through"),
            entry_edge: LaneEdgeReference::local("entry-a"),
            internal_edges: &internal,
            exit_edge: LaneEdgeReference::local("exit-a"),
        })
        .unwrap()
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path-b",
            movement: MovementReference::local("movement-through"),
            entry_edge: LaneEdgeReference::local("entry-b"),
            internal_edges: &internal,
            exit_edge: LaneEdgeReference::local("exit-b"),
        })
        .unwrap()
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-a",
            lane_edge: LaneEdgeReference::local("entry-a"),
        })
        .unwrap()
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-a",
            maneuver_path: ManeuverPathReference::local("path-a"),
            transition_index: 0,
            stop_line: StopLineReference::local("stop-a"),
            signal_control: SignalControlInput::None,
        })
        .unwrap();
    let mut unit = unit([builder.finish().unwrap()]);
    install_compiled_lane_geometries(
        &mut unit,
        "city/junction",
        GeometryCompilationProfiles {
            accuracy: GeometryAccuracyProfile::Balanced5Cm,
            direction: GeometryDirectionProfile::Balanced2Deg,
        },
        |key| {
            let frame = match key {
                "internal" => None,
                "entry-b" | "exit-b" if conflicting_frames => Some(("city/junction", "frame-b")),
                _ => Some(("city/junction", "frame-a")),
            };
            let points = match key {
                "entry-a" | "entry-b" => vec![point(-10.0, 0.0, 0.0), point(0.0, 0.0, 0.0)],
                "internal" => vec![point(0.0, 0.0, 0.0), point(8.0, 0.0, 0.0)],
                "exit-a" | "exit-b" => {
                    vec![point(8.0, 0.0, 0.0), point(20.0, 0.0, 0.0)]
                }
                _ => unreachable!("unexpected fixture edge"),
            };
            (frame, points)
        },
    );
    unit
}

#[test]
fn compiled_geometry_resolves_imported_frame_and_freezes_through_shared_kernel() {
    let limits = CompileLimits::p100_initial_v1();
    let mut base = SyntheticModuleBuilder::new(header("city/base"), &limits).unwrap();
    base.add_canonical_frame(CanonicalFrameInput {
        canonical_frame_key: "world",
        lane_edge_geometries: &[],
    })
    .unwrap();
    let mut roads = SyntheticModuleBuilder::new(header("city/roads"), &limits).unwrap();
    roads
        .add_import("city/base")
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap();
    let mut unit = unit([roads.finish().unwrap(), base.finish().unwrap()]);
    install_compiled_lane_geometries(
        &mut unit,
        "city/roads",
        GeometryCompilationProfiles {
            accuracy: GeometryAccuracyProfile::Balanced5Cm,
            direction: GeometryDirectionProfile::Balanced2Deg,
        },
        |_| {
            (
                Some(("city/base", "world")),
                vec![point(0.0, 0.0, 0.0), point(10.0, 0.0, 0.0)],
            )
        },
    );

    let hir = build_hir(&unit).unwrap();
    assert_eq!(hir.canonical_frames.len(), 1);
    assert_eq!(hir.lane_edge_geometries.len(), 1);
    assert_eq!(hir.canonical_points.len(), 2);
    assert_eq!(hir.spatial_segments.len(), 1);
    let geometry = &hir.lane_edge_geometries[0];
    assert_eq!(geometry.canonical_frame.raw(), 0);
    assert_eq!(geometry.arc_length_meters, 10.0);
    assert_eq!(hir.canonical_frames[0].lane_edge_geometries.len(), 1);

    let output = crate::Compiler::new().compile(unit).unwrap();
    let relation = output
        .source_map_input()
        .spatial_relation_sources()
        .find(|source| source.role() == crate::SourceRelationRole::CanonicalFrameLaneEdgeGeometry)
        .expect("compiled lane geometry retains a source relation");
    assert_eq!(
        relation.primary_source().source_document_key(),
        "city/roads"
    );
}

#[test]
fn facility_band_geometry_freezes_points_without_spatial_segments() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(header("city/roads"), &limits).unwrap();
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "world",
            lane_edge_geometries: &[],
        })
        .unwrap()
        .add_facility_band(FacilityBandInput {
            facility_band_key: "median",
            kind_id: "median",
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "carriageway",
            kind_id: "motorLane",
            lanes: &[AuthoringLaneInput {
                authoring_lane_key: "lane",
                edge_chain: &[LaneEdgeReference::local("edge")],
                lane_group: None,
            }],
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "road",
            reference_section: RoadSectionReference::local("carriageway"),
            elements: &[
                CorridorElementReference::road_section(RoadSectionReference::local("carriageway")),
                CorridorElementReference::facility_band(FacilityBandReference::local("median")),
            ],
        })
        .unwrap();
    let mut unit = unit([builder.finish().unwrap()]);
    let profiles = GeometryCompilationProfiles {
        accuracy: GeometryAccuracyProfile::Balanced5Cm,
        direction: GeometryDirectionProfile::Balanced2Deg,
    };
    install_compiled_lane_geometries(&mut unit, "city/roads", profiles, |_| {
        (
            Some(("city/roads", "world")),
            vec![point(0.0, 0.0, 0.0), point(10.0, 0.0, 0.0)],
        )
    });
    install_compiled_facility_geometry(
        &mut unit,
        "city/roads",
        "median",
        "city/roads",
        "world",
        vec![point(0.0, 0.0, 2.0), point(10.0, 0.0, 2.0)],
    );

    let hir = build_hir(&unit).unwrap();
    assert_eq!(hir.geometry_profiles, Some(profiles));
    assert_eq!(hir.lane_edge_geometries.len(), 1);
    assert_eq!(hir.facility_band_geometries.len(), 1);
    assert_eq!(hir.canonical_points.len(), 4);
    assert_eq!(hir.spatial_segments.len(), 1);
    let geometry = &hir.facility_band_geometries[0];
    assert_eq!(geometry.canonical_frame.raw(), 0);
    assert_eq!(geometry.points.len(), 2);
    let source_range = &hir.geometry_source_ranges[geometry.source_ranges.as_usize_range()][0];
    assert_eq!(source_range.points.as_usize_range(), 2..4);
    assert_eq!(source_range.source_segment_ordinal, 0);
    assert_eq!(hir.canonical_frames[0].facility_band_geometries.len(), 1);
}

#[test]
fn compiled_geometry_source_ranges_rebase_and_retain_imported_module_sources() {
    let limits = CompileLimits::p100_initial_v1();
    let mut base = SyntheticModuleBuilder::new(header("city/base"), &limits).unwrap();
    base.add_canonical_frame(CanonicalFrameInput {
        canonical_frame_key: "world",
        lane_edge_geometries: &[],
    })
    .unwrap();
    let mut roads = SyntheticModuleBuilder::new(header("city/roads"), &limits).unwrap();
    roads
        .add_import("city/base")
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap();
    let mut unit = unit([roads.finish().unwrap(), base.finish().unwrap()]);
    install_compiled_lane_geometries(
        &mut unit,
        "city/roads",
        GeometryCompilationProfiles {
            accuracy: GeometryAccuracyProfile::Balanced5Cm,
            direction: GeometryDirectionProfile::Balanced2Deg,
        },
        |_| {
            (
                Some(("city/base", "world")),
                vec![
                    point(0.0, 0.0, 0.0),
                    point(5.0, 0.0, 0.0),
                    point(10.0, 0.0, 0.0),
                ],
            )
        },
    );
    let roads = unit
        .modules
        .iter_mut()
        .find(|module| module.descriptor().authoring_namespace_id() == "city/roads")
        .unwrap();
    let TypedAstDeclaration::LaneEdge(edge) = &mut roads.declarations[0] else {
        panic!("test expected LaneEdge")
    };
    let LaneEdgeGeometryAuthority::Compiled(compiled) = &mut edge.geometry_authority else {
        panic!("test installed compiled geometry")
    };
    compiled.source_ranges = Box::new([
        CompiledGeometrySourceRange {
            point_start: 0,
            point_end_exclusive: 1,
            source_segment_ordinal: 1,
            source: SourceSpan::point(Arc::from("city/roads"), 7, 1).into(),
        },
        CompiledGeometrySourceRange {
            point_start: 1,
            point_end_exclusive: 3,
            source_segment_ordinal: 0,
            source: SourceSpan::point(Arc::from("city/roads"), 8, 1).into(),
        },
    ]);

    let hir = build_hir(&unit).unwrap();
    let geometry = &hir.lane_edge_geometries[0];
    let ranges = &hir.geometry_source_ranges[geometry.source_ranges.as_usize_range()];
    assert_eq!(ranges[0].points.as_usize_range(), 0..1);
    assert_eq!(ranges[0].source_segment_ordinal, 1);
    assert_eq!(ranges[1].points.as_usize_range(), 1..3);
    assert_eq!(ranges[1].source_segment_ordinal, 0);

    let output = crate::Compiler::new().compile(unit).unwrap();
    let relation = output
        .source_map_input()
        .spatial_relation_sources()
        .next()
        .unwrap();
    let source_ranges = relation.geometry_source_ranges().collect::<Vec<_>>();
    assert_eq!(source_ranges[0].point_range(), 0..1);
    assert_eq!(source_ranges[0].source_segment_ordinal(), 1);
    assert_eq!(
        source_ranges[0].source().source_document_key(),
        "city/roads"
    );
    assert_eq!(source_ranges[1].point_range(), 1..3);
    assert_eq!(source_ranges[1].source_segment_ordinal(), 0);
    assert_eq!(
        source_ranges[1].source().source_document_key(),
        "city/roads"
    );
}

#[test]
fn geometry_sources_follow_lir_edge_order_across_dependency_order() {
    let limits = CompileLimits::p100_initial_v1();
    let mut base = SyntheticModuleBuilder::new(header("city/base"), &limits).unwrap();
    base.add_canonical_frame(CanonicalFrameInput {
        canonical_frame_key: "world",
        lane_edge_geometries: &[],
    })
    .unwrap();
    let z = module("city/z", &["city/base"], &[("edge", &[])]);
    // The otherwise-unused z import forces HIR dependency order z -> a, while the final
    // Identity v1 order remains city/a -> city/z.
    let a = module("city/a", &["city/base", "city/z"], &[("edge", &[])]);
    let mut unit = unit([a, z, base.finish().unwrap()]);
    let profiles = GeometryCompilationProfiles {
        accuracy: GeometryAccuracyProfile::Balanced5Cm,
        direction: GeometryDirectionProfile::Balanced2Deg,
    };
    for namespace in ["city/z", "city/a"] {
        install_compiled_lane_geometries(&mut unit, namespace, profiles, |_| {
            (
                Some(("city/base", "world")),
                vec![point(0.0, 0.0, 0.0), point(12.5, 0.0, 0.0)],
            )
        });
    }

    let hir = build_hir(&unit).unwrap();
    let hir_geometry_namespaces = hir
        .lane_edge_geometries
        .iter()
        .map(|geometry| {
            let edge = &hir.lane_edges[geometry.lane_edge.index()];
            hir.modules[edge.module.index()]
                .authoring_namespace_id
                .as_ref()
        })
        .collect::<Vec<_>>();
    assert_eq!(hir_geometry_namespaces, ["city/z", "city/a"]);

    let output = crate::Compiler::new().compile(unit).unwrap();
    let relations = output
        .source_map_input()
        .spatial_relation_sources()
        .collect::<Vec<_>>();
    assert_eq!(relations.len(), 2);
    assert_eq!(relations[0].local_index(), 0);
    assert_eq!(relations[1].local_index(), 1);
    assert_eq!(
        relations[0].primary_source().source_document_key(),
        "city/a"
    );
    assert_eq!(
        relations[1].primary_source().source_document_key(),
        "city/z"
    );
    assert_eq!(
        relations[0]
            .geometry_source_ranges()
            .next()
            .unwrap()
            .source()
            .source_document_key(),
        "city/a"
    );
    assert_eq!(
        relations[1]
            .geometry_source_ranges()
            .next()
            .unwrap()
            .source()
            .source_document_key(),
        "city/z"
    );
}

#[test]
fn hir_limits_the_actual_compiled_canonical_point_count() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(header("city/roads"), &limits).unwrap();
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "world",
            lane_edge_geometries: &[],
        })
        .unwrap();
    let mut unit = unit([builder.finish().unwrap()]);
    install_compiled_lane_geometries(
        &mut unit,
        "city/roads",
        GeometryCompilationProfiles {
            accuracy: GeometryAccuracyProfile::Balanced5Cm,
            direction: GeometryDirectionProfile::Balanced2Deg,
        },
        |_| {
            (
                Some(("city/roads", "world")),
                vec![
                    point(0.0, 0.0, 0.0),
                    point(5.0, 0.0, 0.0),
                    point(10.0, 0.0, 0.0),
                ],
            )
        },
    );
    unit.limits = CompileLimits::p100_initial_v1()
        .with_test_admission_limit(CompileLimitDimension::GeometryPointCount, 2);

    let diagnostics = match build_hir(&unit) {
        Ok(_) => panic!("actual canonical output points must be limited before allocation"),
        Err(diagnostics) => diagnostics,
    };
    assert!(diagnostics.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.payload(),
        DiagnosticPayload::CompileLimitExceeded {
            dimension: CompileLimitDimension::GeometryPointCount,
            limit: 2,
            observed: 3,
        }
    )));
}

#[test]
fn shared_internal_edge_derives_one_frame_from_every_path() {
    let hir = build_hir(&compiled_junction_unit(false)).unwrap();
    assert_eq!(hir.lane_edge_geometries.len(), 5);
    assert_eq!(hir.stop_lines.len(), 1);
    assert_eq!(hir.maneuver_gates.len(), 1);
    assert_eq!(hir.canonical_frames[0].lane_edge_geometries.len(), 5);
    assert!(
        hir.lane_edge_geometries
            .iter()
            .all(|geometry| geometry.canonical_frame.raw() == 0)
    );
    let internal = hir
        .lane_edges
        .iter()
        .position(|edge| edge.stable_key.as_ref() == "internal")
        .unwrap();
    let internal_geometry = hir
        .lane_edge_geometries
        .iter()
        .find(|geometry| geometry.lane_edge.index() == internal)
        .unwrap();
    assert_eq!(internal_geometry.canonical_frame.raw(), 0);
}

#[test]
fn shared_internal_edge_rejects_conflicting_path_frames() {
    let diagnostics = match build_hir(&compiled_junction_unit(true)) {
        Ok(_) => panic!("conflicting derived frames must reject HIR"),
        Err(diagnostics) => diagnostics,
    };
    assert!(diagnostics.diagnostics().iter().any(|diagnostic| {
        matches!(
            diagnostic.payload(),
            DiagnosticPayload::InvalidSpatialGeometry {
                violation: SpatialGeometryViolation::InternalEdgeFrameConflict,
                ..
            }
        )
    }));
}

#[test]
fn compiled_geometry_profiles_must_match_across_the_compilation_unit() {
    let limits = CompileLimits::p100_initial_v1();
    let module_with_frame = |namespace: &str| {
        let mut builder = SyntheticModuleBuilder::new(header(namespace), &limits).unwrap();
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_canonical_frame(CanonicalFrameInput {
                canonical_frame_key: "frame",
                lane_edge_geometries: &[],
            })
            .unwrap();
        builder.finish().unwrap()
    };
    let mut unit = unit([module_with_frame("city/b"), module_with_frame("city/a")]);
    install_compiled_lane_geometries(
        &mut unit,
        "city/a",
        GeometryCompilationProfiles {
            accuracy: GeometryAccuracyProfile::Fine2Cm,
            direction: GeometryDirectionProfile::Smooth1Deg,
        },
        |_| {
            (
                Some(("city/a", "frame")),
                vec![point(0.0, 0.0, 0.0), point(10.0, 0.0, 0.0)],
            )
        },
    );
    install_compiled_lane_geometries(
        &mut unit,
        "city/b",
        GeometryCompilationProfiles {
            accuracy: GeometryAccuracyProfile::Compact10Cm,
            direction: GeometryDirectionProfile::Compact5Deg,
        },
        |_| {
            (
                Some(("city/b", "frame")),
                vec![point(0.0, 0.0, 0.0), point(10.0, 0.0, 0.0)],
            )
        },
    );

    let diagnostics = match build_hir(&unit) {
        Ok(_) => panic!("mixed geometry profiles must reject HIR"),
        Err(diagnostics) => diagnostics,
    };
    assert!(diagnostics.diagnostics().iter().any(|diagnostic| {
        matches!(
            diagnostic.payload(),
            DiagnosticPayload::InvalidSpatialGeometry {
                violation: SpatialGeometryViolation::GeometryProfileMismatch {
                    expected_accuracy_code: 1,
                    expected_direction_code: 1,
                    actual_accuracy_code: 3,
                    actual_direction_code: 3,
                },
                ..
            }
        )
    }));
}

#[test]
fn geometry_profiles_are_retained_in_hir_and_mir_and_change_the_lir_digest() {
    let compile = |profiles: GeometryCompilationProfiles| {
        let limits = CompileLimits::p100_initial_v1();
        let mut builder = SyntheticModuleBuilder::new(header("city/roads"), &limits).unwrap();
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_canonical_frame(CanonicalFrameInput {
                canonical_frame_key: "world",
                lane_edge_geometries: &[],
            })
            .unwrap();
        let mut unit = unit([builder.finish().unwrap()]);
        install_compiled_lane_geometries(&mut unit, "city/roads", profiles, |_| {
            (
                Some(("city/roads", "world")),
                vec![point(0.0, 0.0, 0.0), point(10.0, 0.0, 0.0)],
            )
        });
        let hir = build_hir(&unit).unwrap();
        assert_eq!(hir.geometry_profiles, Some(profiles));
        let points = hir
            .canonical_points
            .iter()
            .map(|point| [point.x.to_bits(), point.y.to_bits(), point.z.to_bits()])
            .collect::<Vec<_>>();
        let mir = lower_to_mir(&unit, &hir).unwrap();
        assert_eq!(mir.geometry_profiles, Some(profiles));
        let lir = freeze_lir(&unit, &mir).unwrap().lir;
        assert_eq!(lir.geometry_profiles, Some(profiles));
        let digest = lir.semantic_digest;
        (points, digest)
    };

    let (balanced_points, balanced_digest) = compile(GeometryCompilationProfiles {
        accuracy: GeometryAccuracyProfile::Balanced5Cm,
        direction: GeometryDirectionProfile::Balanced2Deg,
    });
    let (fine_points, fine_digest) = compile(GeometryCompilationProfiles {
        accuracy: GeometryAccuracyProfile::Fine2Cm,
        direction: GeometryDirectionProfile::Balanced2Deg,
    });
    let (smooth_points, smooth_digest) = compile(GeometryCompilationProfiles {
        accuracy: GeometryAccuracyProfile::Balanced5Cm,
        direction: GeometryDirectionProfile::Smooth1Deg,
    });
    assert_eq!(balanced_points, fine_points);
    assert_eq!(balanced_points, smooth_points);
    assert_ne!(balanced_digest, fine_digest);
    assert_ne!(balanced_digest, smooth_digest);
}

fn two_edge_compiled_unit(direction: GeometryDirectionProfile) -> CompilationUnit {
    let limits = CompileLimits::p100_initial_v1();
    let successors = [LaneEdgeReference::local("edge-b")];
    let mut builder = SyntheticModuleBuilder::new(header("city/roads"), &limits).unwrap();
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &successors,
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-b",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame",
            lane_edge_geometries: &[],
        })
        .unwrap();
    let mut unit = unit([builder.finish().unwrap()]);
    install_compiled_lane_geometries(
        &mut unit,
        "city/roads",
        GeometryCompilationProfiles {
            accuracy: GeometryAccuracyProfile::Balanced5Cm,
            direction,
        },
        |key| match key {
            "edge-a" => (
                Some(("city/roads", "frame")),
                vec![point(0.0, 0.0, 0.0), point(10.0, 0.0, 0.0)],
            ),
            "edge-b" => (
                Some(("city/roads", "frame")),
                vec![point(10.004, 0.0, 0.0), point(20.0, 0.0, 0.5)],
            ),
            _ => unreachable!("unexpected fixture edge"),
        },
    );
    unit
}

#[test]
fn cross_edge_join_checks_direction_without_welding_or_snapping() {
    let diagnostics = match build_hir(&two_edge_compiled_unit(
        GeometryDirectionProfile::Smooth1Deg,
    )) {
        Ok(_) => panic!("smooth profile must reject the near-three-degree join"),
        Err(diagnostics) => diagnostics,
    };
    assert!(diagnostics.diagnostics().iter().any(|diagnostic| {
        matches!(
            diagnostic.payload(),
            DiagnosticPayload::InvalidSpatialGeometry {
                violation: SpatialGeometryViolation::DirectionDiscontinuity { .. },
                ..
            }
        )
    }));
    let hir = build_hir(&two_edge_compiled_unit(
        GeometryDirectionProfile::Compact5Deg,
    ))
    .expect("the same endpoints pass the looser direction profile");
    let first = &hir.lane_edge_geometries[0];
    let second = &hir.lane_edge_geometries[1];
    let first_end = hir.canonical_points[first.points.as_usize_range().end - 1];
    let second_start = hir.canonical_points[second.points.as_usize_range().start];
    assert_eq!([first_end.x, first_end.y, first_end.z], [10.0, 0.0, 0.0]);
    assert_eq!(
        [second_start.x, second_start.y, second_start.z],
        [10.004, 0.0, 0.0]
    );
}

#[test]
fn unrelated_road_editing_profiles_do_not_restrict_synthetic_connections() {
    let limits = CompileLimits::p100_initial_v1();
    let successors = [LaneEdgeReference::local("edge-b")];
    let edge_a_points = [point(0.0, 0.0, 0.0), point(10.0, 0.0, 0.0)];
    let edge_b_points = [point(10.004, 0.0, 0.0), point(20.0, 0.0, 0.5)];
    let synthetic_geometries = [
        LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local("edge-a"),
            centerline_points: &edge_a_points,
        },
        LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local("edge-b"),
            centerline_points: &edge_b_points,
        },
    ];
    let mut synthetic = SyntheticModuleBuilder::new(header("city/synthetic"), &limits).unwrap();
    synthetic
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &successors,
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-b",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame",
            lane_edge_geometries: &synthetic_geometries,
        })
        .unwrap();
    let mut road_editing =
        SyntheticModuleBuilder::new(header("city/road-editing"), &limits).unwrap();
    road_editing
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame",
            lane_edge_geometries: &[],
        })
        .unwrap();

    let mut unit = unit([synthetic.finish().unwrap(), road_editing.finish().unwrap()]);
    let profiles = GeometryCompilationProfiles {
        accuracy: GeometryAccuracyProfile::Balanced5Cm,
        direction: GeometryDirectionProfile::Smooth1Deg,
    };
    install_compiled_lane_geometries(&mut unit, "city/road-editing", profiles, |_| {
        (
            Some(("city/road-editing", "frame")),
            vec![point(0.0, 0.0, 0.0), point(10.0, 0.0, 0.0)],
        )
    });

    build_hir(&unit)
        .expect("an unrelated RoadEditing profile must not reject the Synthetic-to-Synthetic join");
}

#[test]
fn typed_symbol_table_distinguishes_equal_local_keys_under_different_owners() {
    let local_key: Arc<str> = Arc::from("lane-1");
    let left_address = TypedAstEntityAddress::owner_scoped(
        Arc::from([Arc::from("section-a")]),
        Arc::clone(&local_key),
    );
    let right_address = TypedAstEntityAddress::owner_scoped(
        Arc::from([Arc::from("section-b")]),
        Arc::clone(&local_key),
    );
    assert_eq!(left_address.owner_local_keys()[0].as_ref(), "section-a");
    assert_eq!(right_address.owner_local_keys()[0].as_ref(), "section-b");

    let module = HirModuleKey::from_raw(0);
    let mut symbols = SymbolTable::new([2]);
    symbols.insert(module, left_address.clone(), 11_u32);
    symbols.insert(module, right_address.clone(), 29_u32);
    assert_eq!(symbols.get(module, &left_address), Some(11));
    assert_eq!(symbols.get(module, &right_address), Some(29));

    let span = SourceSpan::point(Arc::from("city/main.lfre"), 1, 1);
    let reference = OwnedEntityReference::<LaneEdgeKind>::with_target_address(
        Arc::from("city/main"),
        right_address.clone(),
        span.clone(),
    );
    assert_eq!(reference.declaration_key().as_ref(), "lane-1");
    assert_eq!(reference.target_address, right_address);

    let diagnostic = Diagnostic::unknown_owner_qualified_reference_target(
        EntityKind::LaneEdge,
        "source-edge",
        "city/main",
        reference.target_address.owner_local_keys(),
        reference.declaration_key(),
        span.clone(),
        span.clone(),
    );
    assert!(matches!(
        diagnostic.payload(),
        crate::DiagnosticPayload::UnknownReferenceTarget {
            target_owner_local_keys,
            target_key,
            ..
        } if target_owner_local_keys.iter().map(AsRef::as_ref).collect::<Vec<_>>() == ["section-b"]
            && target_key.as_ref() == "lane-1"
    ));
    assert!(
        diagnostic
            .to_string()
            .contains("city/main::section-b>lane-1")
    );

    let header = crate::declaration::DeclarationHeader::with_source_address(
        EntityKind::LaneEdge,
        left_address.clone(),
        local_key,
        span.into(),
    );
    assert_eq!(header.source_address, left_address);
    assert_eq!(header.stable_key.as_ref(), "lane-1");
}

#[test]
fn hir_resolves_local_and_imported_lane_edge_references_to_typed_keys() {
    let base = module("city/base", &[], &[("edge-b", &[])]);
    let app_successors = [
        LaneEdgeReference::imported("city/base", "edge-b"),
        LaneEdgeReference::local("edge-c"),
    ];
    let app = module(
        "city/app",
        &["city/base"],
        &[("edge-c", &[]), ("edge-a", &app_successors)],
    );
    let unit = unit([app, base]);
    let hir = build_hir(&unit).unwrap();

    assert_eq!(hir.modules.len(), 2);
    assert_eq!(hir.modules[0].authoring_namespace_id.as_ref(), "city/base");
    assert_eq!(hir.modules[1].authoring_namespace_id.as_ref(), "city/app");
    assert_eq!(hir.imports.len(), 1);
    assert_eq!(hir.imports[0].target.raw(), 0);
    assert_eq!(hir.imports[0].source_span.source_document_key(), "city/app");
    assert_eq!(hir.modules[1].imports.start(), 0);
    assert_eq!(hir.modules[1].imports.len(), 1);
    assert_eq!(hir.lane_edges.len(), 3);
    assert_eq!(
        hir.lane_edges
            .iter()
            .map(|edge| edge.stable_key.as_ref())
            .collect::<Vec<_>>(),
        ["edge-b", "edge-a", "edge-c"]
    );
    let edge_a = &hir.lane_edges[1];
    let targets = hir.lane_edge_references[edge_a.successors.as_usize_range()]
        .iter()
        .map(|reference| reference.target.raw())
        .collect::<Vec<_>>();
    assert_eq!(targets, [2, 0]);
    assert!(hir.modules[0].imports.is_empty());
    assert_eq!(hir.hir_record_count, 16);
}

#[test]
fn hir_reports_every_unknown_target_in_canonical_module_order() {
    let z_successors = [LaneEdgeReference::local("missing-z")];
    let a_successors = [LaneEdgeReference::local("missing-a")];
    let unit = unit([
        module("city/z", &[], &[("edge-z", &z_successors)]),
        module("city/a", &[], &[("edge-a", &a_successors)]),
    ]);
    let diagnostics = match build_hir(&unit) {
        Ok(_) => panic!("unknown targets must reject HIR construction"),
        Err(diagnostics) => diagnostics,
    };

    assert_eq!(diagnostics.diagnostics().len(), 2);
    assert_eq!(
        diagnostics
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.stable_key().unwrap())
            .collect::<Vec<_>>(),
        ["edge-a", "edge-z"]
    );
    assert!(diagnostics.diagnostics().iter().all(|diagnostic| {
        diagnostic.code() == DiagnosticCode::UnknownReferenceTarget
            && diagnostic.primary_span().is_some()
            && diagnostic.related_locations().len() == 1
    }));
}

#[test]
fn hir_symbol_and_reference_order_ignore_declaration_insertion_order() {
    let successors = [
        LaneEdgeReference::local("edge-c"),
        LaneEdgeReference::local("edge-b"),
    ];
    let left = unit([module(
        "city/a",
        &[],
        &[("edge-a", &successors), ("edge-b", &[]), ("edge-c", &[])],
    )]);
    let right = unit([module(
        "city/a",
        &[],
        &[("edge-c", &[]), ("edge-a", &successors), ("edge-b", &[])],
    )]);
    let left = build_hir(&left).unwrap();
    let right = build_hir(&right).unwrap();

    let projection = |hir: &HirUnit| {
        hir.lane_edges
            .iter()
            .map(|edge| {
                (
                    edge.stable_key.to_string(),
                    hir.lane_edge_references[edge.successors.as_usize_range()]
                        .iter()
                        .map(|reference| reference.target.raw())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(projection(&left), projection(&right));
    assert_eq!(
        left.lane_edges
            .iter()
            .map(|edge| edge.stable_id)
            .collect::<Vec<_>>(),
        right
            .lane_edges
            .iter()
            .map(|edge| edge.stable_id)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        projection(&left),
        [
            ("edge-a".into(), vec![1, 2]),
            ("edge-b".into(), vec![]),
            ("edge-c".into(), vec![]),
        ]
    );
}

/// 覆盖全部静态语义领域的成功输入：横断面、路口、控制、信号、停车、空间与准入。
///
/// 几何档与编译几何的安装方式沿用 `compiled_junction_unit`；共享内部边由两条路径
/// 推导规范代表，停车位与准入规则引用各自独立的边。
fn full_domain_unit() -> CompilationUnit {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(header("city/full"), &limits).unwrap();
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry-a",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[LaneEdgeReference::local("internal")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry-b",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[LaneEdgeReference::local("internal")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "internal",
            length_meters: 8.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[
                LaneEdgeReference::local("exit-a"),
                LaneEdgeReference::local("exit-b"),
            ],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit-a",
            length_meters: 12.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit-b",
            length_meters: 12.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 10.0,
            speed_limit_meters_per_second: 12.0,
            successors: &[LaneEdgeReference::local("edge-b")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-b",
            length_meters: 12.0,
            speed_limit_meters_per_second: 12.0,
            successors: &[],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "parking-entry",
            length_meters: 20.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "parking-exit",
            length_meters: 20.0,
            speed_limit_meters_per_second: 8.0,
            successors: &[],
        })
        .unwrap()
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame-a",
            lane_edge_geometries: &[],
        })
        .unwrap()
        .add_facility_band(FacilityBandInput {
            facility_band_key: "sidewalk-left",
            kind_id: "sidewalk",
        })
        .unwrap()
        .add_lane_group(LaneGroupInput {
            lane_group_key: "through",
            road_section: RoadSectionReference::local("carriageway"),
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "carriageway",
            kind_id: "motorLane",
            lanes: &[AuthoringLaneInput {
                authoring_lane_key: "lane-main",
                edge_chain: &[
                    LaneEdgeReference::local("edge-a"),
                    LaneEdgeReference::local("edge-b"),
                ],
                lane_group: Some(LaneGroupReference::local("through")),
            }],
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "main-road",
            reference_section: RoadSectionReference::local("carriageway"),
            elements: &[
                CorridorElementReference::facility_band(FacilityBandReference::local(
                    "sidewalk-left",
                )),
                CorridorElementReference::road_section(RoadSectionReference::local("carriageway")),
            ],
        })
        .unwrap()
        .add_junction(JunctionInput {
            junction_key: "junction-main",
        })
        .unwrap()
        .add_movement(MovementInput {
            movement_key: "movement-through",
            junction: JunctionReference::local("junction-main"),
            directed_entry_approach_key: "approach-westbound",
            directed_exit_approach_key: "approach-eastbound",
        })
        .unwrap()
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path-a",
            movement: MovementReference::local("movement-through"),
            entry_edge: LaneEdgeReference::local("entry-a"),
            internal_edges: &[LaneEdgeReference::local("internal")],
            exit_edge: LaneEdgeReference::local("exit-a"),
        })
        .unwrap()
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path-b",
            movement: MovementReference::local("movement-through"),
            entry_edge: LaneEdgeReference::local("entry-b"),
            internal_edges: &[LaneEdgeReference::local("internal")],
            exit_edge: LaneEdgeReference::local("exit-b"),
        })
        .unwrap()
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-entry",
            lane_edge: LaneEdgeReference::local("entry-a"),
        })
        .unwrap()
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-middle",
            lane_edge: LaneEdgeReference::local("internal"),
        })
        .unwrap()
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-a",
            maneuver_path: ManeuverPathReference::local("path-a"),
            transition_index: 0,
            stop_line: StopLineReference::local("stop-entry"),
            signal_control: SignalControlInput::Group(SignalGroupReference::local("group-entry")),
        })
        .unwrap()
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-b",
            maneuver_path: ManeuverPathReference::local("path-a"),
            transition_index: 1,
            stop_line: StopLineReference::local("stop-middle"),
            signal_control: SignalControlInput::Group(SignalGroupReference::local("group-release")),
        })
        .unwrap()
        .add_waiting_zone(WaitingZoneInput {
            waiting_zone_key: "waiting-main",
            maneuver_path: ManeuverPathReference::local("path-a"),
            entry_gate: ManeuverGateReference::local("gate-a"),
            release_gate: ManeuverGateReference::local("gate-b"),
            max_occupancy: 3,
        })
        .unwrap()
        .add_signal_group(SignalGroupInput {
            signal_group_key: "group-entry",
        })
        .unwrap()
        .add_signal_group(SignalGroupInput {
            signal_group_key: "group-release",
        })
        .unwrap()
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: "controller-main",
            offset_ms: 1_000,
            signal_groups: &[
                SignalGroupReference::local("group-entry"),
                SignalGroupReference::local("group-release"),
            ],
            phases: &[
                SignalPhaseInput {
                    signal_phase_key: "phase-go",
                    duration_ms: 30_000,
                    states: &[
                        SignalGroupStateInput {
                            signal_group: SignalGroupReference::local("group-entry"),
                            aspect: SignalAspect::Green,
                        },
                        SignalGroupStateInput {
                            signal_group: SignalGroupReference::local("group-release"),
                            aspect: SignalAspect::Red,
                        },
                    ],
                },
                SignalPhaseInput {
                    signal_phase_key: "phase-clear",
                    duration_ms: 5_000,
                    states: &[
                        SignalGroupStateInput {
                            signal_group: SignalGroupReference::local("group-entry"),
                            aspect: SignalAspect::Yellow,
                        },
                        SignalGroupStateInput {
                            signal_group: SignalGroupReference::local("group-release"),
                            aspect: SignalAspect::Green,
                        },
                    ],
                },
            ],
        })
        .unwrap()
        .add_parking_area(ParkingAreaInput {
            parking_area_key: "area-main",
        })
        .unwrap();
    let parking_geometry = ParkingSpaceGeometryInput {
        lateral_offset_meters: -3.0,
        heading_offset_radians: 0.25,
        length_meters: 5.5,
        width_meters: 2.6,
    };
    builder
        .add_parking_space(ParkingSpaceInput {
            parking_space_key: "space-owned",
            parking_area: Some(ParkingAreaReference::local("area-main")),
            entry: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("parking-entry"),
                progress_meters: 4.0,
            },
            exit: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("parking-exit"),
                progress_meters: 6.0,
            },
            geometry: parking_geometry,
        })
        .unwrap()
        .add_parking_space(ParkingSpaceInput {
            parking_space_key: "space-independent",
            parking_area: None,
            entry: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("parking-entry"),
                progress_meters: 4.0,
            },
            exit: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("parking-exit"),
                progress_meters: 6.0,
            },
            geometry: parking_geometry,
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .unwrap()
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "car",
            extends: Some(ParticipantClassReference::local("road-user")),
        })
        .unwrap()
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "standard-car",
            participant_class: ParticipantClassReference::local("car"),
            iidm: IidmVehicleProfileInput {
                length_meters: 4.5,
                desired_speed_meters_per_second: 13.75,
                min_gap_meters: 2.0,
                time_headway_seconds: 1.4,
                max_acceleration_meters_per_second_squared: 1.8,
                comfortable_deceleration_meters_per_second_squared: 2.0,
                emergency_deceleration_meters_per_second_squared: 4.5,
            },
        })
        .unwrap()
        .add_access_rule(AccessRuleInput {
            access_rule_key: "allow-road-users",
            target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-a")),
            effect: AccessEffect::Allow,
            participant_classes: &[ParticipantClassReference::local("road-user")],
            regulation: Some(AccessRegulationInput {
                jurisdiction: "CN-test",
                version: "2026-01",
                source: Some("fixture"),
            }),
            priority: 0,
        })
        .unwrap()
        .add_access_rule(AccessRuleInput {
            access_rule_key: "deny-cars",
            target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-a")),
            effect: AccessEffect::Deny,
            participant_classes: &[ParticipantClassReference::local("car")],
            regulation: None,
            priority: 0,
        })
        .unwrap();
    let mut unit = unit([builder.finish().unwrap()]);
    install_compiled_lane_geometries(
        &mut unit,
        "city/full",
        GeometryCompilationProfiles {
            accuracy: GeometryAccuracyProfile::Balanced5Cm,
            direction: GeometryDirectionProfile::Balanced2Deg,
        },
        |key| {
            let frame = match key {
                "internal" => None,
                _ => Some(("city/full", "frame-a")),
            };
            let points = match key {
                "entry-a" | "entry-b" => vec![point(-10.0, 0.0, 0.0), point(0.0, 0.0, 0.0)],
                "internal" => vec![point(0.0, 0.0, 0.0), point(8.0, 0.0, 0.0)],
                "exit-a" | "exit-b" => vec![point(8.0, 0.0, 0.0), point(20.0, 0.0, 0.0)],
                "edge-a" => vec![point(0.0, 10.0, 0.0), point(10.0, 10.0, 0.0)],
                "edge-b" => vec![point(10.0, 10.0, 0.0), point(22.0, 10.0, 0.0)],
                "parking-entry" => vec![point(0.0, 20.0, 0.0), point(20.0, 20.0, 0.0)],
                "parking-exit" => vec![point(0.0, 30.0, 0.0), point(20.0, 30.0, 0.0)],
                _ => unreachable!("unexpected fixture edge"),
            };
            (frame, points)
        },
    );
    unit
}

#[test]
fn hir_full_tables_are_deterministic_across_rebuilds() {
    let app_successors = [
        LaneEdgeReference::imported("city/base", "edge-b"),
        LaneEdgeReference::local("edge-c"),
    ];
    let simple = unit([
        module("city/base", &[], &[("edge-b", &[])]),
        module(
            "city/app",
            &["city/base"],
            &[("edge-c", &[]), ("edge-a", &app_successors)],
        ),
    ]);
    let full = full_domain_unit();

    // 全表对比覆盖 StableId、规范表顺序与来源位置；浮点字段按值比较。
    for candidate in [simple, full] {
        let first = build_hir(&candidate).unwrap();
        let second = build_hir(&candidate).unwrap();
        assert_eq!(first, second);
    }
}

#[test]
fn hir_lane_edge_identity_uses_namespace_and_key_instead_of_dense_position() {
    let city_a = unit([module("city/a", &[], &[("edge-a", &[]), ("edge-b", &[])])]);
    let city_b = unit([module("city/b", &[], &[("edge-a", &[])])]);
    let city_a = build_hir(&city_a).unwrap();
    let city_b = build_hir(&city_b).unwrap();

    assert_ne!(
        city_a.lane_edges[0].stable_id,
        city_a.lane_edges[1].stable_id
    );
    assert_ne!(
        city_a.lane_edges[0].stable_id,
        city_b.lane_edges[0].stable_id
    );
    assert_eq!(
        city_a.lane_edges[0].stable_id.to_string(),
        format!(
            "lfid1_lane-edge_{:x}",
            city_a.lane_edges[0].stable_id.as_untyped()
        )
    );
}

#[test]
fn hir_lane_edge_identity_ignores_non_identity_scalars_and_connections() {
    let baseline = unit([module("city/a", &[], &[("edge-a", &[]), ("edge-b", &[])])]);

    let limits = CompileLimits::p100_initial_v1();
    let mut changed = SyntheticModuleBuilder::new(header("city/a"), &limits).unwrap();
    changed
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-a",
            length_meters: 99.0,
            speed_limit_meters_per_second: 2.0,
            successors: &[LaneEdgeReference::local("edge-b")],
        })
        .unwrap();
    changed
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-b",
            length_meters: 1.0,
            speed_limit_meters_per_second: 1.0,
            successors: &[],
        })
        .unwrap();
    let changed = unit([changed.finish().unwrap()]);

    let baseline = build_hir(&baseline).unwrap();
    let changed = build_hir(&changed).unwrap();
    assert_eq!(baseline.lane_edges[0].stable_key.as_ref(), "edge-a");
    assert_eq!(changed.lane_edges[0].stable_key.as_ref(), "edge-a");
    assert_eq!(
        baseline.lane_edges[0].stable_id,
        changed.lane_edges[0].stable_id
    );
}

#[test]
fn explicit_junction_internal_set_must_equal_the_path_internal_union() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder =
        SyntheticModuleBuilder::new(header("city/junction-closure"), &limits).unwrap();
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[LaneEdgeReference::local("exit")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "unused-internal",
            length_meters: 5.0,
            speed_limit_meters_per_second: 5.0,
            successors: &[],
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "section",
            kind_id: "motorLane",
            lanes: &[AuthoringLaneInput {
                authoring_lane_key: "lane",
                edge_chain: &[
                    LaneEdgeReference::local("entry"),
                    LaneEdgeReference::local("exit"),
                ],
                lane_group: None,
            }],
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "corridor",
            reference_section: RoadSectionReference::local("section"),
            elements: &[CorridorElementReference::road_section(
                RoadSectionReference::local("section"),
            )],
        })
        .unwrap()
        .add_junction(JunctionInput {
            junction_key: "junction",
        })
        .unwrap()
        .add_movement(MovementInput {
            movement_key: "movement",
            junction: JunctionReference::local("junction"),
            directed_entry_approach_key: "entry",
            directed_exit_approach_key: "exit",
        })
        .unwrap()
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path",
            movement: MovementReference::local("movement"),
            entry_edge: LaneEdgeReference::local("entry"),
            internal_edges: &[],
            exit_edge: LaneEdgeReference::local("exit"),
        })
        .unwrap();
    let mut unit = unit([builder.finish().unwrap()]);
    let module = &mut unit.modules[0];
    let junction = module
        .declarations
        .iter_mut()
        .find_map(|declaration| match declaration {
            TypedAstDeclaration::Junction(junction) => Some(junction),
            _ => None,
        })
        .unwrap();
    let namespace = Arc::<str>::from("city/junction-closure");
    let location = |column| SourceSpan::point(Arc::from("city/junction-closure"), 1, column);
    junction.approach_edges = Box::new([
        OwnedEntityReference::new(Arc::clone(&namespace), Arc::from("entry"), location(1)),
        OwnedEntityReference::new(Arc::clone(&namespace), Arc::from("exit"), location(2)),
    ]);
    junction.internal_edges = Box::new([OwnedEntityReference::new(
        namespace,
        Arc::from("unused-internal"),
        location(3),
    )]);

    let diagnostics = match build_hir(&unit) {
        Ok(_) => panic!("unused explicit internal edge must fail"),
        Err(diagnostics) => diagnostics,
    };
    assert!(diagnostics.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.payload(),
        DiagnosticPayload::JunctionEdgeSetMismatch {
            edge_key,
            violation: JunctionEdgeSetViolation::DeclaredInternalUnused,
            ..
        } if edge_key.as_ref() == "unused-internal"
    )));
}

#[test]
fn explicit_junction_internal_edge_cannot_be_section_derived() {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(header("city/junction-role"), &limits).unwrap();
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[LaneEdgeReference::local("internal")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "internal",
            length_meters: 5.0,
            speed_limit_meters_per_second: 5.0,
            successors: &[LaneEdgeReference::local("exit")],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "section",
            kind_id: "motorLane",
            lanes: &[AuthoringLaneInput {
                authoring_lane_key: "lane",
                edge_chain: &[
                    LaneEdgeReference::local("entry"),
                    LaneEdgeReference::local("internal"),
                    LaneEdgeReference::local("exit"),
                ],
                lane_group: None,
            }],
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "corridor",
            reference_section: RoadSectionReference::local("section"),
            elements: &[CorridorElementReference::road_section(
                RoadSectionReference::local("section"),
            )],
        })
        .unwrap()
        .add_junction(JunctionInput {
            junction_key: "junction",
        })
        .unwrap()
        .add_movement(MovementInput {
            movement_key: "movement",
            junction: JunctionReference::local("junction"),
            directed_entry_approach_key: "entry",
            directed_exit_approach_key: "exit",
        })
        .unwrap()
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path",
            movement: MovementReference::local("movement"),
            entry_edge: LaneEdgeReference::local("entry"),
            internal_edges: &[LaneEdgeReference::local("internal")],
            exit_edge: LaneEdgeReference::local("exit"),
        })
        .unwrap();
    let mut unit = unit([builder.finish().unwrap()]);
    let junction = unit.modules[0]
        .declarations
        .iter_mut()
        .find_map(|declaration| match declaration {
            TypedAstDeclaration::Junction(junction) => Some(junction),
            _ => None,
        })
        .unwrap();
    let namespace = Arc::<str>::from("city/junction-role");
    let location = |column| SourceSpan::point(Arc::from("city/junction-role"), 1, column);
    junction.approach_edges = Box::new([
        OwnedEntityReference::new(Arc::clone(&namespace), Arc::from("entry"), location(1)),
        OwnedEntityReference::new(Arc::clone(&namespace), Arc::from("exit"), location(2)),
    ]);
    junction.internal_edges = Box::new([OwnedEntityReference::new(
        namespace,
        Arc::from("internal"),
        location(3),
    )]);

    let diagnostics = match build_hir(&unit) {
        Ok(_) => panic!("section-derived junction internal edge must fail"),
        Err(diagnostics) => diagnostics,
    };
    assert!(diagnostics.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.payload(),
        DiagnosticPayload::JunctionEdgeSetMismatch {
            edge_key,
            violation: JunctionEdgeSetViolation::InternalIsSectionDerived,
            ..
        } if edge_key.as_ref() == "internal"
    )));
}

fn explicit_junction_internal_unit(
    internal_has_successor: bool,
    entry_targets_internal: bool,
) -> CompilationUnit {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder =
        SyntheticModuleBuilder::new(header("city/junction-successors"), &limits).unwrap();
    let internal_successors = [LaneEdgeReference::local("exit")];
    let entry_to_exit = [LaneEdgeReference::local("exit")];
    let entry_to_internal = [LaneEdgeReference::local("internal")];
    let entry_chain = [LaneEdgeReference::local("entry")];
    let exit_chain = [LaneEdgeReference::local("exit")];
    let approach_lanes = [
        AuthoringLaneInput {
            authoring_lane_key: "lane-entry",
            edge_chain: &entry_chain,
            lane_group: None,
        },
        AuthoringLaneInput {
            authoring_lane_key: "lane-exit",
            edge_chain: &exit_chain,
            lane_group: None,
        },
    ];
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "entry",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: if entry_targets_internal {
                &entry_to_internal
            } else {
                &entry_to_exit
            },
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "internal",
            length_meters: 5.0,
            speed_limit_meters_per_second: 5.0,
            successors: if internal_has_successor {
                &internal_successors
            } else {
                &[]
            },
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "exit",
            length_meters: 10.0,
            speed_limit_meters_per_second: 10.0,
            successors: &[],
        })
        .unwrap()
        .add_road_section(RoadSectionInput {
            road_section_key: "section",
            kind_id: "motorLane",
            lanes: &approach_lanes,
        })
        .unwrap()
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "corridor",
            reference_section: RoadSectionReference::local("section"),
            elements: &[CorridorElementReference::road_section(
                RoadSectionReference::local("section"),
            )],
        })
        .unwrap()
        .add_junction(JunctionInput {
            junction_key: "junction",
        })
        .unwrap()
        .add_movement(MovementInput {
            movement_key: "movement",
            junction: JunctionReference::local("junction"),
            directed_entry_approach_key: "entry",
            directed_exit_approach_key: "exit",
        })
        .unwrap()
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path",
            movement: MovementReference::local("movement"),
            entry_edge: LaneEdgeReference::local("entry"),
            internal_edges: &[LaneEdgeReference::local("internal")],
            exit_edge: LaneEdgeReference::local("exit"),
        })
        .unwrap();
    let mut unit = unit([builder.finish().unwrap()]);
    let junction = unit.modules[0]
        .declarations
        .iter_mut()
        .find_map(|declaration| match declaration {
            TypedAstDeclaration::Junction(junction) => Some(junction),
            _ => None,
        })
        .unwrap();
    let namespace = Arc::<str>::from("city/junction-successors");
    let location = |column| SourceSpan::point(Arc::clone(&namespace), 1, column);
    junction.approach_edges = Box::new([
        OwnedEntityReference::new(Arc::clone(&namespace), Arc::from("entry"), location(1)),
        OwnedEntityReference::new(Arc::clone(&namespace), Arc::from("exit"), location(2)),
    ]);
    junction.internal_edges = Box::new([OwnedEntityReference::new(
        Arc::clone(&namespace),
        Arc::from("internal"),
        location(3),
    )]);

    unit
}

#[test]
fn explicit_junction_internal_edge_without_successors_uses_path_authority() {
    let hir = build_hir(&explicit_junction_internal_unit(false, false)).unwrap();
    assert_eq!(hir.junction_internal_edges.len(), 1);
    let internal = &hir.junction_internal_edges[0];
    assert_eq!(
        hir.lane_edges[internal.edge.index()].stable_key.as_ref(),
        "internal"
    );
}

#[test]
fn explicit_junction_internal_edge_rejects_successors() {
    let unit = explicit_junction_internal_unit(true, false);

    let diagnostics = match build_hir(&unit) {
        Ok(_) => panic!("junction-internal successors must fail"),
        Err(diagnostics) => diagnostics,
    };
    assert!(diagnostics.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.payload(),
        DiagnosticPayload::JunctionEdgeSetMismatch {
            edge_key,
            violation: JunctionEdgeSetViolation::InternalHasSuccessors,
            ..
        } if edge_key.as_ref() == "internal"
    )));
}

#[test]
fn explicit_junction_internal_edge_rejects_inbound_successor_authority() {
    let unit = explicit_junction_internal_unit(false, true);

    let diagnostics = match build_hir(&unit) {
        Ok(_) => panic!("successor references into junction-internal edges must fail"),
        Err(diagnostics) => diagnostics,
    };
    assert!(
        diagnostics.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.payload(),
            DiagnosticPayload::JunctionEdgeSetMismatch {
                edge_key,
                violation: JunctionEdgeSetViolation::InternalReferencedBySuccessor,
                ..
            } if edge_key.as_ref() == "internal"
        )),
        "unexpected diagnostics: {:?}",
        diagnostics
            .diagnostics()
            .iter()
            .map(crate::Diagnostic::payload)
            .collect::<Vec<_>>()
    );
}

#[test]
fn hir_checks_record_scratch_and_live_byte_limits_before_stage_allocation() {
    let mut unit = unit([module("city/a", &[], &[("edge-a", &[])])]);
    unit.limits =
        CompileLimits::p100_initial_v1().with_test_pipeline_limits(3, u32::MAX, u32::MAX, u32::MAX);
    let record_failure = match build_hir(&unit) {
        Ok(_) => panic!("HIR record limit must fail closed"),
        Err(diagnostics) => diagnostics,
    };
    assert!(matches!(
        record_failure.diagnostics()[0].payload(),
        DiagnosticPayload::CompileLimitExceeded {
            dimension: CompileLimitDimension::HirRecordCount,
            limit: 3,
            observed: 4,
        }
    ));

    unit.limits =
        CompileLimits::p100_initial_v1().with_test_pipeline_limits(u32::MAX, u32::MAX, 0, u32::MAX);
    let scratch_failure = match build_hir(&unit) {
        Ok(_) => panic!("HIR scratch limit must fail closed"),
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

    let source_live_bytes = u32::try_from(unit.controlled_live_bytes).unwrap();
    unit.limits = CompileLimits::p100_initial_v1().with_test_pipeline_limits(
        u32::MAX,
        u32::MAX,
        u32::MAX,
        source_live_bytes,
    );
    let live_failure = match build_hir(&unit) {
        Ok(_) => panic!("HIR live byte limit must fail closed"),
        Err(diagnostics) => diagnostics,
    };
    assert!(live_failure.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.payload(),
        DiagnosticPayload::CompileLimitExceeded {
            dimension: CompileLimitDimension::CompilerControlledLiveBytes,
            limit,
            observed,
        } if *limit == u64::from(source_live_bytes) && observed > limit
    )));
}

fn parking_unit_with_declared_length_and_polyline(
    declared_meters: f64,
    polyline_end_x: f32,
    progress_meters: f64,
) -> CompilationUnit {
    let limits = CompileLimits::p100_initial_v1();
    let mut builder = SyntheticModuleBuilder::new(header("city/parking-mm"), &limits).unwrap();
    builder
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "world",
            lane_edge_geometries: &[],
        })
        .unwrap()
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "bay",
            length_meters: declared_meters,
            speed_limit_meters_per_second: 8.0,
            successors: &[],
        })
        .unwrap()
        .add_parking_area(ParkingAreaInput {
            parking_area_key: "lot",
        })
        .unwrap()
        .add_parking_space(ParkingSpaceInput {
            parking_space_key: "space",
            parking_area: Some(ParkingAreaReference::local("lot")),
            entry: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("bay"),
                progress_meters,
            },
            exit: ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("bay"),
                progress_meters,
            },
            geometry: ParkingSpaceGeometryInput {
                lateral_offset_meters: -3.0,
                heading_offset_radians: 0.25,
                length_meters: 5.5,
                width_meters: 2.6,
            },
        })
        .unwrap();
    let mut compiled = unit([builder.finish().unwrap()]);
    install_compiled_lane_geometries(
        &mut compiled,
        "city/parking-mm",
        GeometryCompilationProfiles {
            accuracy: GeometryAccuracyProfile::Balanced5Cm,
            direction: GeometryDirectionProfile::Balanced2Deg,
        },
        |_| {
            (
                Some(("city/parking-mm", "world")),
                vec![point(0.0, 0.0, 0.0), point(polyline_end_x, 0.0, 0.0)],
            )
        },
    );
    compiled
}

#[test]
fn parking_anchor_rejects_progress_past_arc_backed_millimetre_length() {
    let unit = parking_unit_with_declared_length_and_polyline(10.0, 9.99, 9.999);
    let arc_mm = millimetres_from_si(f64::from(9.99_f32)).unwrap();
    let progress_mm = millimetres_from_si(9.999).unwrap();
    let declared_mm = millimetres_from_si(10.0).unwrap();
    assert!(
        (PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM
            ..=declared_mm.saturating_sub(PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM))
            .contains(&progress_mm),
        "fixture must be legal against the declared traffic length"
    );
    assert!(
        progress_mm > arc_mm.saturating_sub(PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM),
        "fixture must sit past the arc-backed parking closure"
    );

    let diagnostics = match build_hir(&unit) {
        Ok(_) => panic!("parking past the emitted millimetre length must fail closed"),
        Err(diagnostics) => diagnostics,
    };
    assert!(
        diagnostics
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::InvalidParkingAnchorProgress)
    );
}

#[test]
fn parking_anchor_accepts_progress_legal_against_longer_arc() {
    let polyline_end = 10.01_f32;
    let progress = 10.0;
    let arc_mm = millimetres_from_si(f64::from(polyline_end)).unwrap();
    let progress_mm = millimetres_from_si(progress).unwrap();
    let declared_mm = millimetres_from_si(10.0).unwrap();
    assert!(
        progress_mm > declared_mm.saturating_sub(PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM),
        "fixture must be illegal against the declared traffic length"
    );
    assert!(
        (PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM
            ..=arc_mm.saturating_sub(PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM))
            .contains(&progress_mm),
        "fixture must be legal against the arc-backed parking closure"
    );

    let unit = parking_unit_with_declared_length_and_polyline(10.0, polyline_end, progress);
    build_hir(&unit).expect("arc-backed millimetre length must admit this parking progress");
    crate::Compiler::new()
        .compile(unit)
        .expect("emission must use the same millimetre length as HIR parking closure");
}
