//! #1516 的作者编辑、独立线形及带坡横断面对照。
use laneflow_compiler::road_editing::*;
use laneflow_compiler::{
    CompilationUnitBuilder, CompileProfile, Compiler, DiagnosticBundle, GeometryAccuracyProfile,
    GeometryDirectionProfile,
};
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
                    section.clone(),
                    lanes[0].clone(),
                    vec![RoadEditingCorridorElement::RoadSection(section.clone())],
                )
                .unwrap(),
            ))
            .unwrap();
        module
            .add_declaration(RoadEditingDeclaration::RoadSection(
                RoadSectionInput::try_new("section", "motorLane", lanes.to_vec(), corridor)
                    .unwrap(),
            ))
            .unwrap();
        for lane in ["inner", "outer"] {
            let edge = format!("{lane}-{i}");
            let direction = if backward && lane == "outer" {
                RoadEditingLaneDirection::Backward
            } else {
                RoadEditingLaneDirection::Forward
            };
            module
                .add_declaration(RoadEditingDeclaration::AuthoringLane(
                    AuthoringLaneInput::try_new(
                        lane,
                        LaneEdgeReference::local(&edge).unwrap(),
                        direction,
                        LinearWidthProfile::try_new(3.5, 3.5).unwrap(),
                        None,
                        section.clone(),
                    )
                    .unwrap(),
                ))
                .unwrap();
            module
                .add_declaration(RoadEditingDeclaration::LaneEdge(
                    LaneEdgeInput::try_new(edge, 10.0, vec![], None).unwrap(),
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
        let length = pts
            .windows(2)
            .map(|p| {
                let a = p[0];
                let b = p[1];
                let dx = f64::from(b.x) - f64::from(a.x);
                let dy = f64::from(b.y) - f64::from(a.y);
                let dz = f64::from(b.z) - f64::from(a.z);
                (dx * dx + dy * dy + dz * dz).sqrt()
            })
            .sum::<f64>();
        assert_eq!(
            Some(edge.length_mm()),
            laneflow_static_contract::millimetres_from_si(length)
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
    )
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
                    let out = compile(&save(curve.clone(), &stations, accuracy, direction, false))
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
