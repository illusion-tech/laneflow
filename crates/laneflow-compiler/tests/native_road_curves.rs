//! #1516 的作者编辑、独立线形及带坡横断面对照。
use laneflow_compiler::road_editing::*;
use laneflow_compiler::{
    CompilationOutput, CompilationUnitBuilder, CompileProfile, Compiler, DiagnosticBundle,
    GeometryAccuracyProfile, GeometryDirectionProfile, PortableDiffBase,
    PortableEmissionProvenance, PortablePublicationCandidate, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
#[path = "native_road_curves/vehicle.rs"]
mod vehicle;

type Segment = RoadEditingCurveSegment;
type Program = RoadEditingCurveProgram;
fn p(x: f64, y: f64, z: f64) -> RoadEditingPoint3 {
    RoadEditingPoint3::try_new(x, y, z).unwrap()
}

fn save(
    program: Program,
    cuts: &[f64],
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    backward: bool,
    coverage: CrossSectionCoverage,
) -> OwnedRoadEditingSourceBuffer {
    save_lanes(
        program,
        cuts,
        (accuracy, direction),
        backward,
        coverage,
        0,
        [[3.5; 2]; 2],
    )
}

fn save_lanes(
    program: Program,
    cuts: &[f64],
    profiles: (GeometryAccuracyProfile, GeometryDirectionProfile),
    backward: bool,
    coverage: CrossSectionCoverage,
    reference: usize,
    widths: [[f64; 2]; 2],
) -> OwnedRoadEditingSourceBuffer {
    save_lane_declarations(
        program,
        cuts,
        profiles,
        [
            RoadEditingLaneDirection::Forward,
            if backward {
                RoadEditingLaneDirection::Backward
            } else {
                RoadEditingLaneDirection::Forward
            },
        ],
        coverage,
        reference,
        SectionFixture {
            lanes: widths.map(|w| (w, true)),
            ..SectionFixture::default()
        },
    )
}

struct SectionFixture<'a> {
    lanes: [([f64; 2], bool); 2],
    lane_cuts_m: [&'a [f64]; 2],
    extents: Vec<BoundaryExtentInput>,
}

impl Default for SectionFixture<'_> {
    fn default() -> Self {
        Self {
            lanes: [([3.5; 2], true); 2],
            lane_cuts_m: [&[]; 2],
            extents: Vec::new(),
        }
    }
}

fn save_lane_declarations(
    program: Program,
    cuts: &[f64],
    (accuracy, direction): (GeometryAccuracyProfile, GeometryDirectionProfile),
    directions: [RoadEditingLaneDirection; 2],
    coverage: CrossSectionCoverage,
    reference: usize,
    fixture: SectionFixture<'_>,
) -> OwnedRoadEditingSourceBuffer {
    let limits = CompileProfile::single_network_10k_stable_entities();
    let header = RoadEditingModuleHeader::try_new(
        "city",
        "native",
        vec![],
        RoadEditingProvenance::direct("curve edit").unwrap(),
    )
    .unwrap();
    let mut module =
        RoadEditingSourceModuleBuilder::new(header, accuracy, direction, &limits).unwrap();
    module
        .add_alignment(
            RoadAlignmentInput::try_new(
                "road",
                CanonicalFrameReference::local("frame").unwrap(),
                program,
            )
            .unwrap(),
        )
        .unwrap();
    module
        .add_declaration(RoadEditingDeclaration::CanonicalFrame(
            CanonicalFrameInput::try_new("frame").unwrap(),
        ))
        .unwrap();
    let mut start = 0.0;
    for i in 0..=cuts.len() {
        let key = format!("part-{i}");
        let corridor = RoadCorridorReference::local(&key).unwrap();
        let section = RoadSectionReference::owner_scoped(vec![key.clone()], "section").unwrap();
        let lanes = ["inner", "outer"].map(|l| {
            AuthoringLaneReference::owner_scoped(vec![key.clone(), "section".into()], l).unwrap()
        });
        let mut elements = vec![RoadEditingCorridorElement::RoadSection(section.clone())];
        // 两条待测车道都反向时，另用一个正向区段提供走廊参考线。
        let (reference_section, reference_lane) =
            if directions == [RoadEditingLaneDirection::Backward; 2] {
                let s = RoadSectionReference::owner_scoped(vec![key.clone()], "reference").unwrap();
                let l = AuthoringLaneReference::owner_scoped(
                    vec![key.clone(), "reference".into()],
                    "forward",
                )
                .unwrap();
                let edge = format!("reference-{i}");
                module
                    .add_declaration(RoadEditingDeclaration::RoadSection(
                        RoadSectionInput::try_new(
                            "reference",
                            "motorLane",
                            vec![l.clone()],
                            corridor.clone(),
                        )
                        .unwrap(),
                    ))
                    .unwrap();
                module
                    .add_declaration(RoadEditingDeclaration::AuthoringLane(
                        AuthoringLaneInput::try_new(
                            "forward",
                            LaneEdgeReference::local(&edge).unwrap(),
                            RoadEditingLaneDirection::Forward,
                            LinearWidthProfile::try_new(3.5, 3.5).unwrap(),
                            None,
                            s.clone(),
                        )
                        .unwrap()
                        .with_wheel_support(Some(true)),
                    ))
                    .unwrap();
                module
                    .add_declaration(RoadEditingDeclaration::LaneEdge(
                        LaneEdgeInput::try_new(edge, 10., vec![], None).unwrap(),
                    ))
                    .unwrap();
                elements.push(RoadEditingCorridorElement::RoadSection(s.clone()));
                (s, l)
            } else {
                (section.clone(), lanes[reference].clone())
            };
        let end = cuts.get(i).copied().map_or(
            RoadEditingStationEnd::AlignmentEnd,
            RoadEditingStationEnd::Finite,
        );
        module
            .add_declaration(RoadEditingDeclaration::RoadCorridor(
                RoadCorridorInput::try_new(
                    &key,
                    RoadAlignmentReference::try_new("road").unwrap(),
                    start,
                    end,
                    reference_section,
                    reference_lane,
                    elements,
                )
                .unwrap()
                .with_cross_section(coverage, vec![]),
            ))
            .unwrap();
        module
            .add_declaration(RoadEditingDeclaration::RoadSection(
                RoadSectionInput::try_new("section", "motorLane", lanes.to_vec(), corridor)
                    .unwrap()
                    .with_boundary_extents(fixture.extents.clone()),
            ))
            .unwrap();
        for (n, lane) in ["inner", "outer"].into_iter().enumerate() {
            let ([low, high], supports_wheel) = fixture.lanes[n];
            let cuts = fixture.lane_cuts_m[n];
            let keys: Vec<_> = std::iter::once(format!("{lane}-{i}"))
                .chain((1..=cuts.len()).map(|part| format!("{lane}-{i}-{part}")))
                .collect();
            let edges: Vec<_> = keys
                .iter()
                .map(|k| LaneEdgeReference::local(k).unwrap())
                .collect();
            for (index, key) in keys.into_iter().enumerate() {
                let next = edges.get(index + 1).cloned().into_iter().collect();
                module
                    .add_declaration(RoadEditingDeclaration::LaneEdge(
                        LaneEdgeInput::try_new(key, 10.0, next, None).unwrap(),
                    ))
                    .unwrap();
            }
            module
                .add_declaration(RoadEditingDeclaration::AuthoringLane(
                    AuthoringLaneInput::try_new(
                        lane,
                        edges[0].clone(),
                        directions[n],
                        LinearWidthProfile::try_new(low, high).unwrap(),
                        None,
                        section.clone(),
                    )
                    .unwrap()
                    .with_edge_chain(edges, cuts.to_vec())
                    .unwrap()
                    .with_wheel_support(
                        (coverage == CrossSectionCoverage::Complete).then_some(supports_wheel),
                    ),
                ))
                .unwrap();
        }
        if let Some(next) = cuts.get(i) {
            start = *next;
        }
    }
    RoadEditingSourceWriter::new(&limits)
        .write(module.finish().unwrap())
        .unwrap()
}

#[derive(Debug, PartialEq)]
struct Chord {
    key: String,
    segment: u32,
    t: [f64; 2],
    points: [[f32; 3]; 2],
    bounds: NativeChordBounds,
    backward: bool,
}
fn compile(bytes: &OwnedRoadEditingSourceBuffer) -> Result<Vec<Chord>, DiagnosticBundle> {
    let mut b = CompilationUnitBuilder::new(CompileProfile::single_network_10k_stable_entities());
    let mut out = Vec::new();
    b.add_road_editing_module_with_native_chords(
        RoadEditingModuleInput::try_new("native", bytes.as_bytes(), None).unwrap(),
        |e| {
            out.push(Chord {
                key: e.target_key.to_string(),
                segment: e.segment_ordinal,
                t: e.parameters,
                points: e.endpoints,
                bounds: e.bounds,
                backward: e.backward,
            });
        },
    )?;
    let output = Compiler::new().compile(b.build()?)?;
    for edge in output.lir().lane_edges() {
        let geometry = edge.spatial_geometry().unwrap();
        let pts: Vec<_> = geometry.points().collect();
        // LIR accumulates each canonical chord in f32 before rounding the total to millimetres.
        let length = pts
            .windows(2)
            .map(|p| {
                let a = p[0];
                let b = p[1];
                (b.x - a.x).hypot(b.y - a.y).hypot(b.z - a.z)
            })
            .sum::<f32>();
        assert_eq!(
            Some(edge.length_mm()),
            laneflow_static_contract::millimetres_from_si(f64::from(length))
        );
        let mut trace = out.iter().filter(|c| {
            edge.identity_fields()
                .any(|f| f.value_bytes() == c.key.as_bytes())
        });
        if let Some(first) = trace.next() {
            let end = trace.next_back().unwrap_or(first).points[1];
            if first.backward {
                assert_eq!([pts[0].x, pts[0].y, pts[0].z], end);
            }
        }
    }
    Ok(out)
}
fn balanced(curve: Program) -> OwnedRoadEditingSourceBuffer {
    save(
        curve,
        &[],
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        false,
        CrossSectionCoverage::None,
    )
}

fn compile_complete(curve: Program) -> Result<CompilationOutput, DiagnosticBundle> {
    let source = save(
        curve,
        &[],
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        false,
        CrossSectionCoverage::Complete,
    );
    compile_saved(&source)
}

fn compile_saved(
    source: &OwnedRoadEditingSourceBuffer,
) -> Result<CompilationOutput, DiagnosticBundle> {
    let mut unit =
        CompilationUnitBuilder::new(CompileProfile::single_network_10k_stable_entities());
    unit.add_road_editing_module(
        RoadEditingModuleInput::try_new("native", source.as_bytes(), None).unwrap(),
    )?;
    Compiler::new().compile(unit.build()?)
}

fn publish(output: &CompilationOutput, name: &str) -> PortablePublicationCandidate {
    emit_portable_candidate(
        output,
        &PortableEmissionProvenance::try_new(name).unwrap(),
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap()
}

fn load_network(
    candidate: &PortablePublicationCandidate,
    spatial: SpatialBuildOption,
) -> std::sync::Arc<SharedNetworkRevision> {
    let checked =
        check_canonical_network_input(candidate.canonical_artifact().bytes(), FormatLimits::HARD)
            .unwrap();
    build_shared_network_revision(
        checked,
        SharedNetworkBuildOptions::new(spatial, SharedNetworkBuildLimits::new(64 << 20, 16 << 20)),
    )
    .unwrap()
}

#[test]
fn neighboring_footprints_share_the_same_quantized_interface() {
    for (reference, widths, expected_um) in [
        (0, [[3.000_003; 2], [3.0; 2]], [-1_500_001; 2]),
        (
            1,
            [[3.0; 2], [3.000_003, 3.000_005]],
            [1_500_001, 1_500_002],
        ),
    ] {
        let curve =
            Program::try_new(p(0.0, 0.0, 0.0), vec![Segment::line(p(0.0, 0.0, 50.0))]).unwrap();
        let source = save_lanes(
            curve,
            &[],
            (
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
            ),
            false,
            CrossSectionCoverage::Complete,
            reference,
            widths,
        );
        let output = compile_saved(&source).unwrap();
        let candidate = publish(&output, "lane-interface");
        for spatial in [
            SpatialBuildOption::Omit,
            SpatialBuildOption::RetainAvailable,
        ] {
            let network = load_network(&candidate, spatial);
            let section = output.lir().road_sections().next().unwrap().ordinal();
            let lanes = network
                .traffic()
                .relations()
                .section_lanes(section)
                .unwrap();
            let mapping = network.station_mapping();
            let left = mapping.lane_footprint(lanes[0]).unwrap();
            let right = mapping.lane_footprint(lanes[1]).unwrap();
            for (i, expected) in expected_um.into_iter().enumerate() {
                let shared = left.ends_um[i][1];
                assert_eq!(shared, expected);
                assert_eq!(shared, right.ends_um[i][0]);
                let exact = widths[reference][i] * if reference == 0 { -0.5 } else { 0.5 };
                let error_nm = exact.mul_add(1_000_000.0, -f64::from(shared)).abs() * 1000.0;
                for footprint in [left, right] {
                    assert!(error_nm <= f64::from(footprint.error_nm));
                    assert!(footprint.error_nm <= 1000);
                }
            }
        }
    }
}

#[test]
fn oblique_grade_seam_publishes_both_one_sided_surface_derivatives() {
    let curve = Program::try_new(
        p(0.0, 0.0, 0.0),
        vec![
            Segment::line(p(30.0, 0.0, 40.0)),
            Segment::line(p(60.0, 0.5, 80.0)),
        ],
    )
    .unwrap();
    let output = compile_complete(curve).unwrap();
    let corridor = output.lir().road_corridors().next().unwrap().ordinal();
    let reference = output
        .lir()
        .lane_edges()
        .find(|edge| {
            edge.identity_fields()
                .any(|field| field.value_bytes() == b"inner-0")
        })
        .unwrap();
    // 两段的解析和略小于 100.0025 米；实际规范弦的 binary32 累计值略大于它。
    assert_eq!(reference.length_mm(), 100_003);
    let candidate = publish(&output, "grade-seam");
    for spatial in [
        SpatialBuildOption::Omit,
        SpatialBuildOption::RetainAvailable,
    ] {
        let network = load_network(&candidate, spatial);
        let mapping = network.station_mapping();
        let lane = network
            .traffic()
            .relations()
            .lane_edge_authoring_lane(reference.ordinal())
            .unwrap();
        let end = mapping.corridor_domain(corridor).unwrap().length_nm;
        let laneflow_static_network::StationQuery::Complete(found) =
            mapping.at_station(corridor, end, lane)
        else {
            panic!("terminal station unresolved");
        };
        let terminal = found.addresses().next().unwrap();
        assert_eq!(
            (terminal.edge, terminal.progress_mm),
            (reference.ordinal(), [100_003; 2])
        );
        let spans = mapping.surface_spans(corridor);
        let seam = spans
            .windows(2)
            .find(|pair| pair[0].stations[1].0 == 50_000_000_000)
            .unwrap();
        let left = seam[0].jets[1];
        let right = seam[1].jets[0];
        assert_eq!(seam[0].stations[1], seam[1].stations[0]);
        assert_eq!(left[..3], [30.0, 0.0, 40.0]);
        assert_eq!(left[..3], right[..3]);
        let [x, z, u, v] = [left[3], left[5], right[3], right[5]].map(f64::from);
        assert_eq!(x * v, z * u);
        assert!(x * u + z * v > 0.0);
        assert_eq!(left[4], 0.0);
        assert_eq!(right[4], (0.5 / 50.0_f64.hypot(0.5)) as f32);
        for ((row, jet), rise) in seam.iter().zip([left, right]).zip([0.0, 0.5]) {
            let length = 50.0_f64.hypot(rise);
            let expected = [30.0, rise, 40.0].map(|v| v / length);
            let error = expected
                .into_iter()
                .zip(&jet[3..6])
                .map(|(source, &stored)| (source - f64::from(stored)).powi(2))
                .sum::<f64>()
                .sqrt();
            assert!(error <= f64::from(row.derivative_errors[1]));
        }
    }
}

#[test]
fn oblique_grade_seam_rejects_a_source_turn_hidden_by_quantization() {
    use laneflow_compiler::{
        DiagnosticPayload, RoadEditingNumericViolation, RoadEditingRelationKind,
        RoadEditingRelationOccurrence, RoadEditingSourceViolation, RoadEditingSubject,
    };
    let curve = Program::try_new(
        p(0.0, 0.0, 0.0),
        vec![
            Segment::line(p(30.0, 0.0, 40.0)),
            Segment::line(p(60.0, 0.0, 80.0_f64.next_up())),
        ],
    )
    .unwrap();
    let err = compile_complete(curve).err().expect("horizontal turn");
    let diagnostic = &err.diagnostics()[0];
    assert!(matches!(
        diagnostic.payload(),
        DiagnosticPayload::InvalidRoadEditingSource {
            violation: RoadEditingSourceViolation::NumericFreeze(
                RoadEditingNumericViolation::StationMappingNotProven
            ),
            ..
        }
    ));
    let source = diagnostic
        .primary_location()
        .unwrap()
        .road_editing()
        .unwrap();
    assert!(matches!(
        source.subject(),
        RoadEditingSubject::OwnerLocal {
            relation: RoadEditingRelationKind::CurveSegment,
            occurrence: RoadEditingRelationOccurrence::OrderedProductOrdinal(1),
            ..
        }
    ));
}

#[test]
fn grade_seams_accept_bezier_endpoint_handles_and_inherited_native_headings() {
    let approach = Segment::line(p(30.0, 0.0, 40.0));
    let bezier = Segment::cubic_bezier(p(42.0, 0.2, 56.0), p(54.0, 0.4, 72.0), p(66.0, 0.6, 88.0));
    let spiral = Segment::clothoid(20.0, 0.0, 1.0 / 100.0, 0.01).unwrap();
    let arc = Segment::circular_arc(20.0, 100.0, 0.0).unwrap();
    for segments in [
        vec![approach.clone(), bezier, Segment::line(p(81.0, 0.6, 108.0))],
        vec![approach, spiral, arc],
    ] {
        let curve = Program::try_new(p(0.0, 0.0, 0.0), segments).unwrap();
        compile_complete(curve).unwrap();
    }
}

fn publish_bezier(
    [control1, control2, end]: [RoadEditingPoint3; 3],
    source_lengths_m: [f64; 2],
) -> (
    std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
    [laneflow_static_contract::LaneEdgeOrdinal; 2],
) {
    use laneflow_format::{RegistryCheckedFieldValue, RegistryCheckedRowView};
    let curve = Program::try_new(
        p(0.0, 0.0, 0.0),
        vec![Segment::cubic_bezier(control1, control2, end)],
    )
    .unwrap();
    let output = compile_complete(curve).unwrap();
    let candidate = publish(&output, "nonuniform-bezier");
    let checked =
        check_canonical_network_input(candidate.canonical_artifact().bytes(), FormatLimits::HARD)
            .unwrap();
    let mapping = checked
        .value_checked_view()
        .registry_view()
        .section(8)
        .unwrap();
    let mapped_edges = mapping.table(3).unwrap();
    let spans = mapping.table(4).unwrap();
    let u32_field = |row: RegistryCheckedRowView<'_>, tag| {
        let RegistryCheckedFieldValue::U32(value) = row.field_by_tag(tag).unwrap().value().unwrap()
        else {
            panic!("expected u32")
        };
        value
    };
    let edges = [
        ("inner-0", source_lengths_m[0]),
        ("outer-0", source_lengths_m[1]),
    ]
    .map(|(name, length)| {
        let edge = output
            .lir()
            .lane_edges()
            .find(|e| {
                e.identity_fields()
                    .any(|f| f.value_bytes() == name.as_bytes())
            })
            .unwrap();
        let canonical_length = edge
            .spatial_geometry()
            .unwrap()
            .segments()
            .last()
            .unwrap()
            .cumulative_end_meters;
        let owner = mapped_edges
            .rows()
            .find(|row| u32_field(*row, 1) as usize == edge.ordinal().index())
            .unwrap();
        let terminal = spans
            .row(u32_field(owner, 3) + u32_field(owner, 4) - 1)
            .unwrap();
        let source_error_nm = u32_field(terminal, 4);
        assert!(source_error_nm <= 1_000_000);
        assert!(
            (length - f64::from(canonical_length)).abs() * 1e9 <= f64::from(source_error_nm),
            "{name}: {length} m, {canonical_length} m, {source_error_nm} nm"
        );
        edge.ordinal()
    });
    let root = build_shared_network_revision(
        checked,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 << 20, 16 << 20),
        ),
    )
    .unwrap();
    let corridor = output.lir().road_corridors().next().unwrap().ordinal();
    assert!(!root.station_mapping().surface_spans(corridor).is_empty());
    assert!(root.station_mapping().boundary_extents().is_empty());
    (root, edges)
}

#[test]
fn nonuniform_straight_bezier_publishes_station_mapping() {
    use laneflow_static_network::StationQuery;
    let z = |meters| p(0.0, 0.0, meters);
    let (root, [reference, target]) = publish_bezier([z(10.0), z(30.0), z(60.0)], [60.0; 2]);
    let relations = root.traffic().relations();
    let target_lane = relations.lane_edge_authoring_lane(target).unwrap();
    let corridor = relations
        .section_corridor(relations.authoring_section(target_lane).unwrap())
        .unwrap();
    let map = root.station_mapping();
    assert_eq!(
        map.corridor_domain(corridor).unwrap().length_nm,
        60_000_000_000
    );
    for (station_nm, expected_mm) in [(0, 0), (60_000_000_000, 60_000)] {
        let StationQuery::Complete(found) = map.at_station(corridor, station_nm, target_lane)
        else {
            panic!("endpoint unresolved")
        };
        let addresses = found.addresses().collect::<Vec<_>>();
        assert_eq!(addresses.len(), 1);
        assert_eq!(
            (addresses[0].edge, addresses[0].progress_mm),
            (target, [expected_mm; 2])
        );
    }
    // 两条恒宽车道平行且同向，原线在同一截面的行程相等。
    let StationQuery::Complete(found) = map.from_progress(reference, 22_500, target_lane) else {
        panic!("interior progress unresolved")
    };
    let addresses = found.addresses().collect::<Vec<_>>();
    assert_eq!(addresses.len(), 1);
    assert_eq!(addresses[0].edge, target);
    let [lo, hi] = addresses[0].progress_mm;
    assert!((lo..=hi).contains(&22_500));
}

#[test]
fn curved_bezier_offset_travel_matches_analytic_length() {
    use laneflow_static_network::{StationObservation, StationQuery};
    // (x', z') = (12t, 48 − 0.75t²)，速率为 48 + 0.75t²，转角为 2 atan(t/8)。
    // 右偏车道的全长为参考线长加上偏移距离乘以总转角。
    let source_lengths = [48.25, 48.25 + 7.0 * 0.125_f64.atan()];
    let (root, [reference, outer]) = publish_bezier(
        [p(0.0, 0.0, 16.0), p(2.0, 0.0, 32.0), p(6.0, 0.0, 47.75)],
        source_lengths,
    );
    let map = root.station_mapping();
    let StationObservation::Complete { corridor, .. } = map.stations_for_progress(reference, 0)
    else {
        panic!("start unresolved");
    };
    let target = root
        .traffic()
        .relations()
        .lane_edge_authoring_lane(outer)
        .unwrap();
    let end = map.corridor_domain(corridor).unwrap().length_nm;
    let StationQuery::Complete(found) = map.at_station(corridor, end, target) else {
        panic!("offset endpoint unresolved");
    };
    let addresses = found.addresses().collect::<Vec<_>>();
    assert_eq!(addresses.len(), 1);
    let expected = root.traffic().lane_lengths_millimetres()[outer.index()];
    assert_eq!(
        (addresses[0].edge, addresses[0].progress_mm),
        (outer, [expected; 2])
    );
}

// 精确转角单独积分，复合 Simpson 的误差使用 ADR 0022 §6.1 的四阶导数界。
fn integrate(length: f64, k0: f64, k1: f64, heading: f64, u: f64) -> ([f64; 2], f64) {
    const N: usize = 512;
    let h = u / N as f64;
    let slope = (k1 - k0) / length;
    let angle = |v: f64| heading + k0 * v + 0.5 * slope * v * v;
    let mut sum = [0.0; 2];
    for i in 0..=N {
        let (x, z) = angle(i as f64 * h).sin_cos();
        let w = if i == 0 || i == N {
            1.0
        } else if i % 2 == 0 {
            2.0
        } else {
            4.0
        };
        sum[0] += w * x;
        sum[1] += w * z;
    }
    let k = k0.abs().max(k1.abs());
    let d = slope.abs();
    let truncation = u * h.powi(4) * (k.powi(4) + 6.0 * k * k * d + 3.0 * d * d) / 180.0;
    // 测试域 |θ|<4，有限次 binary64 求角、求和和三角对照的保守余量。
    let error = truncation + u * 1e-12;
    (sum.map(|v| v * h / 3.0), error)
}
fn chain(entry: f64, radius: f64) -> (Program, [f64; 2]) {
    let k = 1.0 / radius;
    let mut end = [0.0, 10.0];
    let mut heading = 0.0;
    for (length, k0, k1) in [(entry, 0.0, k), (24.0, k, k), (48.0, k, 0.0)] {
        let (delta, _) = integrate(length, k0, k1, heading, length);
        end[0] += delta[0];
        end[1] += delta[1];
        heading += (k0 + k1) * 0.5 * length;
    }
    (
        Program::try_new(
            p(0.0, 0.0, 0.0),
            vec![
                Segment::line(p(0.0, 0.0, 10.0)),
                Segment::clothoid(entry, 0.0, k, 0.0)
                    .unwrap()
                    .with_smooth_entry(true),
                Segment::circular_arc(24.0, radius, 0.0)
                    .unwrap()
                    .with_canvas_selection("arc")
                    .unwrap()
                    .with_smooth_entry(true),
                Segment::clothoid(48.0, k, 0.0, 0.0)
                    .unwrap()
                    .with_smooth_entry(true),
                Segment::line(p(
                    end[0] + 10.0 * heading.sin(),
                    0.0,
                    end[1] + 10.0 * heading.cos(),
                ))
                .with_smooth_entry(true),
            ],
        )
        .unwrap(),
        end,
    )
}
fn read(bytes: &OwnedRoadEditingSourceBuffer) -> Program {
    Program::read_from(
        RoadEditingModuleInput::try_new("native", bytes.as_bytes(), None).unwrap(),
        RoadEditingCurveOwner::Alignment("road"),
        &CompileProfile::single_network_10k_stable_entities(),
    )
    .unwrap()
    .unwrap()
}
#[test]
fn edit_saved_parameters_and_recompile_the_complete_smooth_chain() {
    let (original, old_end) = chain(48.0, 48.0);
    let original_bytes = balanced(original.clone());
    let restored = read(&original_bytes);
    assert_eq!(restored, original);
    let first = compile(&original_bytes).unwrap();
    assert_eq!(first, compile(&original_bytes).unwrap());
    for (length, radius) in [(60.0, 48.0), (48.0, 60.0), (48.0, -48.0)] {
        let (wanted, end) = chain(length, radius);
        let mut segments = restored.segments().to_vec();
        segments[1..].clone_from_slice(&wanted.segments()[1..]);
        let saved = balanced(Program::try_new(restored.start(), segments.clone()).unwrap());
        assert_eq!(read(&saved), wanted);
        let chords = compile(&saved).unwrap();
        let last = chords
            .iter()
            .rfind(|v| v.key == "inner-0" && v.segment == 3)
            .unwrap()
            .points[1];
        assert!((f64::from(last[0]) - end[0]).hypot(f64::from(last[2]) - end[1]) < 1e-5);
        segments[3] = segments[3]
            .clone()
            .with_fixed_end(p(old_end[0], 0.0, old_end[1]))
            .unwrap();
        let failure = compile(&balanced(
            Program::try_new(restored.start(), segments).unwrap(),
        ))
        .unwrap_err();
        let diag = &failure.diagnostics()[0];
        let location = diag.primary_location().unwrap().road_editing().unwrap();
        assert_eq!(
            location.property_path().unwrap().steps(),
            &[laneflow_compiler::RoadEditingPropertyStep::TableField {
                table: laneflow_compiler::RoadEditingTableKind::CurveSegment,
                field_id: 4
            }]
        );
        assert!(format!("{failure:?}").contains("FixedEndConflict"));
    }
    let mut changed = restored.segments().to_vec();
    changed[2] = Segment::circular_arc(24.0, 48.0, 0.001)
        .unwrap()
        .with_smooth_entry(true);
    assert!(
        format!(
            "{:?}",
            compile(&balanced(
                Program::try_new(restored.start(), changed).unwrap()
            ))
            .unwrap_err()
        )
        .contains("SmoothEntryConflict")
    );
}
#[test]
fn alternating_smooth_bends_preserve_the_inherited_direction() {
    let mut segments = Vec::new();
    let mut reference = Vec::new();
    let mut start = [0.0; 2];
    let mut heading = 0.0;
    let mut oracle_error = 0.0;
    for bend in 0..16 {
        let radius = if bend % 2 == 0 { 80.0 } else { -80.0 };
        let k = 1.0 / radius;
        for (segment, length, k0, k1) in [
            (Segment::clothoid(32.0, 0.0, k, 0.0), 32.0, 0.0, k),
            (Segment::circular_arc(16.0, radius, 0.0), 16.0, k, k),
            (Segment::clothoid(32.0, k, 0.0, 0.0), 32.0, k, 0.0),
        ] {
            segments.push(segment.unwrap().with_smooth_entry(!segments.is_empty()));
            reference.push((start, heading, oracle_error, length, k0, k1));
            let (delta, error) = integrate(length, k0, k1, heading, length);
            start = [start[0] + delta[0], start[1] + delta[1]];
            oracle_error += error;
            heading += 0.5 * (k0 + k1) * length;
        }
    }
    let program =
        Program::try_new_with_tangent(p(0.0, 0.0, 0.0), segments, Some([0.0, 1.0])).unwrap();
    let chords = compile(&balanced(program)).unwrap();
    let mut endpoints = 0;
    for c in chords.iter().filter(|c| c.key == "inner-0") {
        let (start, heading, oracle_error, length, k0, k1) = reference[c.segment as usize];
        let u = length * (c.t[0] + c.t[1]) * 0.5;
        let (delta, error) = integrate(length, k0, k1, heading, u);
        let dx = start[0] + delta[0] - c.bounds.midpoint[0];
        let dz = start[1] + delta[1] - c.bounds.midpoint[2];
        assert!(dx.hypot(dz) <= oracle_error + error + c.bounds.midpoint_error);
        let chord_x = f64::from(c.points[1][0]) - f64::from(c.points[0][0]);
        let chord_z = f64::from(c.points[1][2]) - f64::from(c.points[0][2]);
        let chord_length = chord_x.hypot(chord_z);
        for t in c.t {
            let u = length * t;
            let theta = heading + k0 * u + 0.5 * (k1 - k0) * u * u / length;
            let (x, z) = theta.sin_cos();
            assert!(
                (x - chord_x / chord_length).hypot(z - chord_z / chord_length)
                    <= c.bounds.heading_error + 1e-12
            );
        }
        endpoints += usize::from(c.t[1] == 1.0);
    }
    assert_eq!(endpoints, reference.len());
}

#[test]
fn inherited_heading_still_checks_grade_and_curvature_at_the_seam() {
    let k = 1.0 / 80.0;
    for (grade, k0, smooth) in [(0.5e-9, k, true), (2e-9, k, false), (0.0, k + 2e-12, false)] {
        let program = Program::try_new_with_tangent(
            p(0.0, 0.0, 0.0),
            vec![
                Segment::circular_arc(16.0, 80.0, 0.0).unwrap(),
                Segment::clothoid(32.0, k0, 0.0, grade)
                    .unwrap()
                    .with_smooth_entry(true),
            ],
            Some([0.0, 1.0]),
        )
        .unwrap();
        let result = compile(&balanced(program));
        if smooth {
            result.unwrap();
        } else {
            assert!(format!("{:?}", result.unwrap_err()).contains("SmoothEntryConflict"));
        }
    }
}

#[test]
fn continuous_bounds_cover_reverse_bends_mirror_and_adjacent_curvatures() {
    for (k0, k1) in [(-0.03, 0.03), (0.03, -0.03), (0.01, 0.01_f64.next_up())] {
        let length = 40.0;
        let g = 0.035;
        let curve = Program::try_new_with_tangent(
            p(0.0, 0.0, 0.0),
            vec![Segment::clothoid(length, k0, k1, g).unwrap()],
            Some([0.0, 8.0]),
        )
        .unwrap();
        let saved = balanced(curve.clone());
        assert_eq!(read(&saved), curve);
        let out = compile(&saved).unwrap();
        for c in out.iter().filter(|v| v.key == "inner-0") {
            let u = length * (c.t[0] + c.t[1]) * 0.5;
            let (xz, oracle) = integrate(length, k0, k1, 0.0, u);
            let mid = [xz[0], g * u, xz[1]];
            let diff =
                |a: [f64; 3], b: [f64; 3]| (a[0] - b[0]).hypot(a[1] - b[1]).hypot(a[2] - b[2]);
            assert!(diff(mid, c.bounds.midpoint) <= oracle + c.bounds.midpoint_error);
            let chord_mid = std::array::from_fn(|i| {
                0.5 * (f64::from(c.points[0][i]) + f64::from(c.points[1][i]))
            });
            assert!(diff(mid, chord_mid) <= oracle + c.bounds.position_error);
            let delta =
                std::array::from_fn(|i| f64::from(c.points[1][i]) - f64::from(c.points[0][i]));
            let unit = |d: [f64; 3]| d.map(|v| v / d[0].hypot(d[1]).hypot(d[2]));
            for t in c.t {
                let u = t * length;
                let angle = k0 * u + 0.5 * (k1 - k0) * u * u / length;
                assert!(
                    diff(unit([angle.sin(), g, angle.cos()]), unit(delta))
                        <= c.bounds.direction_error + 1e-12
                );
            }
            let travel = length * (c.t[1] - c.t[0]) * g.hypot(1.0);
            assert!(c.bounds.travel_distance[0] <= travel && travel <= c.bounds.travel_distance[1]);
        }
    }
}

#[test]
fn both_foreign_writers_restore_editable_geometry_that_reaches_lir() {
    for name in ["cpp_writer.lfre", "csharp_writer.lfre"] {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/lfre-crosslang")
                .join(name),
        )
        .unwrap();
        let curve = Program::read_from(
            RoadEditingModuleInput::try_new("roads/crosslang-writer", &bytes, None).unwrap(),
            RoadEditingCurveOwner::Alignment("alignment"),
            &CompileProfile::single_network_10k_stable_entities(),
        )
        .unwrap()
        .unwrap();
        let out = compile(&balanced(curve)).unwrap();
        assert!(out.iter().any(|c| c.segment == 1));
        assert!(out.iter().any(|c| c.segment == 2));
    }
}

#[test]
fn graded_station_cuts_use_each_lanes_own_surface_distance() {
    for g in [40.0_f64 / 399.0, 0.0] {
        let scale = 1.0_f64.hypot(g);
        for (cuts, lane, idx, origin) in [
            (vec![50.0], "inner", 1, 50.0),
            (vec![70.0], "outer", 0, 0.0),
            (vec![47.0, 59.0, 68.0], "inner", 2, 59.0),
            (vec![53.0, 60.0, 74.0], "outer", 2, 60.0),
        ] {
            let curve = Program::try_new(
                p(0.0, 0.0, 0.0),
                vec![
                    Segment::line(p(0.0, g * 40.0, 40.0)),
                    Segment::circular_arc(45.0, 35.0, g).unwrap(),
                ],
            )
            .unwrap();
            let stations: Vec<_> = cuts.iter().map(|u| u * scale).collect();
            for accuracy in [
                GeometryAccuracyProfile::Fine2Cm,
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryAccuracyProfile::Compact10Cm,
            ] {
                for direction in [
                    GeometryDirectionProfile::Smooth1Deg,
                    GeometryDirectionProfile::Balanced2Deg,
                    GeometryDirectionProfile::Compact5Deg,
                ] {
                    let out = compile(&save(
                        curve.clone(),
                        &stations,
                        accuracy,
                        direction,
                        false,
                        CrossSectionCoverage::None,
                    ))
                    .unwrap();
                    let key = format!("{lane}-{idx}");
                    let chords: Vec<_> = out.iter().filter(|c| c.key == key).collect();
                    let first = chords.first().unwrap();
                    let start = (origin - 40.0_f64).max(0.0);
                    assert!((first.t[0] * 45.0 - start).abs() < 1e-12);
                    let speed = if lane == "outer" {
                        1.1_f64.hypot(g)
                    } else {
                        scale
                    };
                    let expected = (40.0 - origin).max(0.0) * scale + (20.0 - start) * speed;
                    let straight = (40.0 - origin).max(0.0) * scale;
                    let mut low = straight;
                    let mut high = straight;
                    let mut poly = straight;
                    for c in chords {
                        let a = c.t[0] * 45.0;
                        let b = c.t[1] * 45.0;
                        let width = (20.0_f64.min(b) - a).max(0.0);
                        if width > 0.0 {
                            low = (low + width * c.bounds.speed[0]).next_down();
                            high = (high + width * c.bounds.speed[1]).next_up();
                            let [p, q] = c.points;
                            let delta = std::array::from_fn::<_, 3, _>(|i| {
                                f64::from(q[i]) - f64::from(p[i])
                            });
                            poly += width / (b - a) * delta[0].hypot(delta[1]).hypot(delta[2]);
                        }
                    }
                    assert!(low - 1e-11 <= expected && expected <= high + 1e-11);
                    assert!((expected - poly).abs() < 0.02);
                    if origin == 60.0 {
                        assert!(poly < 1e-12);
                    }
                }
            }
        }
    }
}

#[test]
fn continuous_clearance_and_compile_hard_limits_are_distinct() {
    let curve = |length, radius| {
        Program::try_new_with_tangent(
            p(0.0, 0.0, 0.0),
            vec![Segment::circular_arc(length, radius, 0.0).unwrap()],
            Some([0.0, 1.0]),
        )
        .unwrap()
    };
    let out = compile(&save(
        curve(1.0, 35.0),
        &[],
        GeometryAccuracyProfile::Compact10Cm,
        GeometryDirectionProfile::Compact5Deg,
        true,
        CrossSectionCoverage::None,
    ))
    .unwrap();
    let chord = out.iter().find(|c| c.key == "inner-0").unwrap();
    assert_eq!(chord.t, [0.0, 1.0]);
    assert_eq!(chord.bounds.integration_blocks, 1);
    let phi = 0.5_f64 / 35.0;
    let sag = 35.0 * (1.0 - phi.cos());
    let n = [phi.cos(), -phi.sin()];
    let analytic_mid = [sag, 35.0 * phi.sin()];
    let gap =
        |p: [f64; 3]| (p[0] - analytic_mid[0]) * n[0] + (p[2] - analytic_mid[1]) * n[1] - 0.001;
    let chord_gap = chord
        .points
        .map(|p| gap(p.map(f64::from)))
        .into_iter()
        .fold(f64::INFINITY, f64::min);
    assert!((chord_gap - (sag - 0.001)).abs() < 1e-7);
    assert_eq!(
        chord.bounds.clearance([chord_gap; 2], 0.0, 0.0),
        NativeClearance::Unproven(NativeUnprovenReason::ThresholdOverlap)
    );
    assert!(gap(chord.bounds.midpoint) + chord.bounds.midpoint_error < 0.0);
    assert_eq!(
        chord.bounds.clearance([0.02; 2], 0.0, 0.0),
        NativeClearance::Clear
    );
    assert_eq!(
        chord.bounds.clearance([-0.02; 2], 0.0, 0.0),
        NativeClearance::Contact
    );
    assert!(
        out.iter()
            .filter(|c| c.key == "outer-0")
            .all(|c| c.backward)
    );
    assert!(chord.bounds.inverse_travel_error(0.0005).unwrap()[1] >= 0.0005);
    assert!(compile(&balanced(curve(10000.0, 0.1))).is_err());
    for (length, radius, g) in [
        (0.0, 35.0, 0.0),
        (1.0, 0.0, 0.0),
        (1.0, f64::from_bits(1), 0.0),
        (1.0, 1.0, f64::NAN),
    ] {
        assert!(Segment::circular_arc(length, radius, g).is_err());
    }
    assert!(Segment::clothoid(1.0, -0.0, 0.0, 0.0).is_err());
    assert!(
        Program::try_new(
            p(0.0, 0.0, 0.0),
            vec![Segment::circular_arc(1.0, 3.0, 0.0).unwrap()]
        )
        .is_err()
    );
}
