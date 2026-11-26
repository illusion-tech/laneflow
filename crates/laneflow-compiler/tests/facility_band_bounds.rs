//! 同中心线的变宽分隔带从 LFRE 到共享路网边界与语义差异的验收。
use laneflow_compiler::road_editing::*;
use laneflow_compiler::{
    CompilationOutput, CompilationUnitBuilder, CompileProfile, Compiler, DiagnosticBundle,
    GeometryAccuracyProfile, GeometryDirectionProfile, PortableDiffBase,
    PortableEmissionProvenance, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_canonical_network_input, preflight_object_values};
use laneflow_static_contract::PortableObjectKind;
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

const DOCUMENT: &str = "median-bend";
const LEG: f64 = 10.5 * 401.0 / 399.0;

#[derive(Clone, Copy)]
enum Edit {
    NarrowPeak,
    CollapseTip(&'static str),
    FractionalLaneWidths,
}

fn source(previous: Option<(&[u8], Edit)>) -> Vec<u8> {
    bend(previous, None)
}

#[derive(Clone, Copy)]
enum RailEdit {
    Baseline,
    Raise,
    MoveEnd,
    Rename,
    ChangeSide,
    Remove,
    MappingOnly,
}

fn bend(previous: Option<(&[u8], Edit)>, boundary: Option<(Option<&[u8]>, RailEdit)>) -> Vec<u8> {
    let profile = CompileProfile::single_network_10k_stable_entities();
    let previous = previous.map(|(bytes, edit)| {
        (
            RoadEditingModuleInput::try_new(DOCUMENT, bytes, None).unwrap(),
            edit,
        )
    });
    let header = RoadEditingModuleHeader::try_new(
        "city",
        DOCUMENT,
        vec![],
        RoadEditingProvenance::direct("median design").unwrap(),
    )
    .unwrap();
    let mut builder = RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Fine2Cm,
        GeometryDirectionProfile::Smooth1Deg,
        &profile,
    )
    .unwrap();
    let curve = RoadEditingCurveProgram::try_new_with_tangent(
        RoadEditingPoint3::try_new(0.0, 0.0, 0.0).unwrap(),
        vec![RoadEditingCurveSegment::circular_arc(21.0, 35.0, 40.0 / 399.0).unwrap()],
        Some([0.0, 1.0]),
    )
    .unwrap();
    builder
        .add_alignment(
            RoadAlignmentInput::try_new(
                "alignment",
                CanonicalFrameReference::local("frame").unwrap(),
                curve,
            )
            .unwrap(),
        )
        .unwrap();
    let mut add = |declaration| {
        builder.add_declaration(declaration).unwrap();
    };
    add(RoadEditingDeclaration::CanonicalFrame(
        CanonicalFrameInput::try_new("frame").unwrap(),
    ));
    for (key, start, end, [w0, w1]) in [
        ("widen", 0.0, RoadEditingStationEnd::Finite(LEG), [0.0, 0.5]),
        (
            "narrow",
            LEG,
            RoadEditingStationEnd::AlignmentEnd,
            [0.5, 0.0],
        ),
    ] {
        let corridor = RoadCorridorReference::local(key).unwrap();
        let section = |side| RoadSectionReference::owner_scoped(vec![key.into()], side).unwrap();
        let lane = |side: &str| {
            AuthoringLaneReference::owner_scoped(vec![key.into(), side.into()], side).unwrap()
        };
        let band = FacilityBandReference::owner_scoped(vec![key.into()], "median").unwrap();
        let coverage = boundary.map_or(CrossSectionCoverage::StationMapping, |(_, edit)| {
            if matches!(edit, RailEdit::MappingOnly) {
                CrossSectionCoverage::StationMapping
            } else {
                CrossSectionCoverage::Complete
            }
        });
        let mut extents = Vec::new();
        if let Some((old, edit)) = boundary
            && key == "widen"
        {
            let rail = BoundaryExtentInput {
                left: BoundaryMemberInput::RoadSection(section("A")),
                right: BoundaryMemberInput::FacilityBand(band.clone()),
                kind: BoundaryExtentKind::Obstacle,
                local_key: "rail".into(),
                low: BoundaryStation::CorridorEndpoint,
                high: BoundaryStation::Fixed(7. * 401. / 399.),
                payload: Some(BoundaryPayloadInput::Guardrail {
                    anchor_role: 2,
                    lateral_offset_meters: 0.,
                    horizontal_length_meters: 2.,
                    radius_meters: 0.15,
                    bottom_height_meters: 0.,
                    top_height_meters: 0.85,
                    parameter_error_meters: 0.,
                }),
                canvas_selection: Some("rail-selection".into()),
            };
            let mut rule = rail.clone();
            rule.kind = BoundaryExtentKind::Rule;
            rule.local_key = "mark".into();
            rule.payload = Some(BoundaryPayloadInput::CrossingRule {
                directions: 1,
                effect: 2,
            });
            let mut curb = rail.clone();
            curb.local_key = "curb".into();
            curb.left = BoundaryMemberInput::FacilityBand(band.clone());
            curb.right = BoundaryMemberInput::RoadSection(section("B"));
            let low = CurbEndInput {
                left_offset_meters: 0.15,
                right_offset_meters: -0.05,
                bottom_height_meters: 0.,
                top_height_meters: 0.1,
            };
            curb.payload = Some(BoundaryPayloadInput::Curb {
                low,
                high: CurbEndInput {
                    top_height_meters: 0.2,
                    ..low
                },
                parameter_error_meters: 0.000_001,
            });
            extents = vec![rule, rail, curb];
            if let Some(old) = old {
                let input = RoadEditingModuleInput::try_new(DOCUMENT, old, None).unwrap();
                let saved = BoundaryExtentInput::read_from(
                    input,
                    BoundaryOwner::Corridor(&corridor),
                    &profile,
                )
                .unwrap()
                .unwrap();
                assert_eq!(&*saved, extents);
                extents = saved.into_vec();
            }
            let rail = &mut extents[1];
            match edit {
                RailEdit::Baseline => {}
                RailEdit::Raise => {
                    let Some(BoundaryPayloadInput::Guardrail {
                        top_height_meters, ..
                    }) = &mut rail.payload
                    else {
                        panic!()
                    };
                    *top_height_meters = 1.05;
                }
                RailEdit::MoveEnd => {
                    rail.high = BoundaryStation::Fixed(6.5 * 401. / 399.);
                }
                RailEdit::Rename => rail.local_key = "newRail".into(),
                RailEdit::ChangeSide => {
                    rail.left = BoundaryMemberInput::FacilityBand(band.clone());
                    rail.right = BoundaryMemberInput::RoadSection(section("B"));
                }
                RailEdit::Remove => {
                    extents.remove(1);
                }
                RailEdit::MappingOnly => {
                    for e in &mut extents {
                        e.payload = None
                    }
                }
            }
        }
        add(RoadEditingDeclaration::RoadCorridor(
            RoadCorridorInput::try_new(
                key,
                RoadAlignmentReference::try_new("alignment").unwrap(),
                start,
                end,
                section("A"),
                lane("A"),
                vec![
                    RoadEditingCorridorElement::RoadSection(section("A")),
                    RoadEditingCorridorElement::FacilityBand(band),
                    RoadEditingCorridorElement::RoadSection(section("B")),
                ],
            )
            .unwrap()
            .with_cross_section(coverage, extents),
        ));
        let initial_band = FacilityBandInput::try_new(
            "median",
            "median",
            LinearWidthProfile::try_new(w0, w1).unwrap(),
            corridor.clone(),
        )
        .unwrap()
        .with_canvas_selection(format!("{key}-median"))
        .unwrap();
        let band = if let Some((input, edit)) = previous {
            let mut band = FacilityBandInput::read_from(input, &corridor, "median", &profile)
                .unwrap()
                .unwrap();
            assert_eq!(band, initial_band);
            let old = band.width_profile();
            let (w0, w1) = (old.start_width_meters(), old.end_width_meters());
            let [a, b] = match edit {
                Edit::NarrowPeak => [w0 * 0.5, w1 * 0.5],
                Edit::CollapseTip(owner) if owner == key => {
                    [w0.max(0.000_000_4), w1.max(0.000_000_4)]
                }
                Edit::CollapseTip(_) | Edit::FractionalLaneWidths => [w0, w1],
            };
            band.set_width_profile(LinearWidthProfile::try_new(a, b).unwrap());
            band
        } else {
            initial_band
        };
        add(RoadEditingDeclaration::FacilityBand(band));
        for side in ["A", "B"] {
            let edge = format!("{key}-{side}");
            add(RoadEditingDeclaration::RoadSection(
                RoadSectionInput::try_new(side, "motorLane", vec![lane(side)], corridor.clone())
                    .unwrap(),
            ));
            let authoring_lane = if let Some((input, edit)) = previous {
                let mut source_lane =
                    AuthoringLaneInput::read_from(input, &section(side), side, &profile)
                        .unwrap()
                        .unwrap();
                let old = source_lane.width_profile();
                assert_eq!(
                    [old.start_width_meters(), old.end_width_meters()],
                    [3.5 - w0, 3.5 - w1]
                );
                if matches!(edit, Edit::NarrowPeak) {
                    // 两侧等量补回车道宽度，设施带及两车道的中心坐标在两个修订上相同。
                    source_lane.set_width_profile(
                        LinearWidthProfile::try_new(3.5 - w0 * 0.5, 3.5 - w1 * 0.5).unwrap(),
                    );
                } else if matches!(edit, Edit::FractionalLaneWidths) {
                    let [low, high] = if side == "A" {
                        [3.000_003; 2]
                    } else {
                        // 分隔带增宽时右侧车道等量移回中心，两个走廊的后继方向保持连续。
                        [3.0 - 2.0 * w0, 3.0 - 2.0 * w1]
                    };
                    source_lane.set_width_profile(LinearWidthProfile::try_new(low, high).unwrap());
                }
                source_lane
            } else {
                AuthoringLaneInput::try_new(
                    side,
                    LaneEdgeReference::local(&edge).unwrap(),
                    RoadEditingLaneDirection::Forward,
                    LinearWidthProfile::try_new(3.5 - w0, 3.5 - w1).unwrap(),
                    None,
                    section(side),
                )
                .unwrap()
            };
            add(RoadEditingDeclaration::AuthoringLane(
                authoring_lane.with_wheel_support(
                    (coverage == CrossSectionCoverage::Complete).then_some(side == "A"),
                ),
            ));
            let next = (key == "widen")
                .then(|| LaneEdgeReference::local(format!("narrow-{side}")).unwrap())
                .into_iter()
                .collect();
            add(RoadEditingDeclaration::LaneEdge(
                LaneEdgeInput::try_new(edge, 10.0, next, None).unwrap(),
            ));
        }
    }
    RoadEditingSourceWriter::new(&profile)
        .write(builder.finish().unwrap())
        .unwrap()
        .as_bytes()
        .to_vec()
}

fn compile(bytes: &[u8]) -> Result<CompilationOutput, DiagnosticBundle> {
    let mut unit =
        CompilationUnitBuilder::new(CompileProfile::single_network_10k_stable_entities());
    unit.add_road_editing_module(RoadEditingModuleInput::try_new(DOCUMENT, bytes, None).unwrap())?;
    Compiler::new().compile(unit.build()?)
}

// 从两端公开的值与导数重建 Hermite 多项式，对照解析带坡圆弧及其各阶导数。
fn check_graded_arc_surface(row: &laneflow_static_network::RoadSurfaceSpan, origin: f64) {
    let [lo, hi] = row.stations.map(|s| s.0 as f64 * 1e-9);
    let h = hi - lo;
    let rate = 399.0 / (401.0 * 35.0);
    for t in [0.0_f64, 0.25, 0.5, 0.75, 1.0] {
        let s = origin + lo + h * t;
        for order in 0..=4 {
            let angle = s * rate + order as f64 * std::f64::consts::FRAC_PI_2;
            let scale = 35.0 * rate.powi(order as i32);
            let source = [
                if order == 0 { 35.0 } else { 0.0 } - scale * angle.cos(),
                match order {
                    0 => s * 40.0 / 401.0,
                    1 => 40.0 / 401.0,
                    _ => 0.0,
                },
                scale * angle.sin(),
            ];
            let mut squared = 0.0;
            for (axis, actual) in source.into_iter().enumerate() {
                let [a, b] = row.jets.map(|jet| {
                    [
                        f64::from(jet[axis]),
                        f64::from(jet[3 + axis]) * h,
                        f64::from(jet[6 + axis]) * h * h,
                    ]
                });
                let delta = b[0] - a[0];
                let coeff = [
                    a[0],
                    a[1],
                    a[2] / 2.0,
                    10.0 * delta - 6.0 * a[1] - 4.0 * b[1] - 1.5 * a[2] + b[2] / 2.0,
                    -15.0 * delta + 8.0 * a[1] + 7.0 * b[1] + 1.5 * a[2] - b[2],
                    6.0 * delta - 3.0 * (a[1] + b[1]) + (b[2] - a[2]) / 2.0,
                ];
                let derived: f64 = (order..6)
                    .map(|i| {
                        let factor = (0..order).fold(1, |p, k| p * (i - k));
                        coeff[i] * factor as f64 * t.powi((i - order) as i32)
                    })
                    .sum();
                squared += (actual - derived / h.powi(order as i32)).powi(2);
            }
            let oracle_roundoff = 5e-12 * (1.0 + h.powi(-(order as i32)));
            assert!(
                squared.sqrt() <= f64::from(row.derivative_errors[order]) + oracle_roundoff,
                "s={s} order={order} observed={} bound={}",
                squared.sqrt(),
                row.derivative_errors[order]
            );
        }
    }
}

#[test]
fn named_guardrail_edits_keep_their_owner_and_have_a_stable_diff() {
    let bytes = bend(None, Some((None, RailEdit::Baseline)));
    let output = compile(&bytes).unwrap();
    let provenance = PortableEmissionProvenance::try_new(DOCUMENT).unwrap();
    let base = emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    let checked = preflight_object_values(
        base.canonical_artifact().bytes(),
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap();
    for spatial in [
        SpatialBuildOption::Omit,
        SpatialBuildOption::RetainAvailable,
    ] {
        let shared = build_shared_network_revision(
            check_canonical_network_input(base.canonical_artifact().bytes(), FormatLimits::HARD)
                .unwrap(),
            SharedNetworkBuildOptions::new(
                spatial,
                SharedNetworkBuildLimits::new(64 << 20, 16 << 20),
            ),
        )
        .unwrap();
        let map = shared.station_mapping();
        for corridor in output.lir().road_corridors() {
            let c = corridor.ordinal();
            let origin = if map.corridor_domain(c).unwrap().origin_nm == 0 {
                0.0
            } else {
                LEG
            };
            let rows = map.surface_spans(c);
            assert!(!rows.is_empty());
            let names = map
                .boundary_extents()
                .iter()
                .filter(|e| e.corridor == c)
                .count();
            assert_eq!(names, if origin == 0.0 { 3 } else { 0 });
            if origin == 0. {
                use laneflow_static_network::vehicle::*;
                let relations = shared.traffic().relations();
                let section = relations.corridor_reference_section(c).unwrap();
                let input = TrackQuery {
                    revision: shared.network_revision(),
                    corridor: c,
                    frame: rows[0].frame,
                    station_window_nm: [0, map.corridor_domain(c).unwrap().length_nm],
                    traversable_lanes: relations.section_lanes(section).unwrap(),
                    axles: AxleDimensions {
                        wheelbase_m: 2.60,
                        half_track_m: 0.80,
                        reference_height_m: 0.55,
                    },
                    trace: RearAxleTrace::Corridor(CorridorTrace {
                        stations_nm: [2_000_000_000, 4_000_000_000],
                        from_m: [0.3; 2],
                        to_m: [0.3; 2],
                    }),
                };
                let report = shared.vehicle_geometry(
                    &input,
                    SteeringLimits {
                        max_angle_rad: 0.6,
                        max_rate_rad_s: 1.,
                        speed_m_s: 5.,
                    },
                    BodyEnvelope {
                        front_from_rear_m: 3.40,
                        rear_overhang_m: 0.75,
                        half_width_m: 0.95,
                        heights_m: [0., 1.5],
                    },
                    GeometryWorkLimits::STANDARD,
                );
                assert!(
                    matches!(report.tracks.outcome, TrackOutcome::Covered(_))
                        && matches!(report.sweep.outcome,SweepOutcome::Clear(Some(clear)) if clear.lower_m>0.),
                    "{report:?}"
                );
            }
            for row in rows {
                check_graded_arc_surface(row, origin);
            }
        }
    }
    let section = checked.registry_view().section(8).unwrap();
    for (table, count) in [(6, 2), (7, 3), (8, 4), (10, 1), (11, 2)] {
        assert_eq!(section.table(table - 1).unwrap().row_count(), count);
    }
    assert!(section.table(8).unwrap().row_count() >= 3);
    let source = preflight_object_values(
        base.source_map().bytes(),
        PortableObjectKind::SourceMap,
        FormatLimits::HARD,
    )
    .unwrap();
    let mut counts = [0; 6];
    for row in source
        .registry_view()
        .section(3)
        .unwrap()
        .table(0)
        .unwrap()
        .rows()
    {
        let role = row.field_by_tag(3).unwrap().value_bytes()[0];
        if role >= 37 {
            counts[usize::from(role - 37)] += 1;
        }
    }
    for (i, count) in counts.into_iter().enumerate() {
        assert_eq!(count, section.table(i as u32 + 5).unwrap().row_count());
    }
    for edit in [
        RailEdit::Raise,
        RailEdit::MoveEnd,
        RailEdit::Rename,
        RailEdit::ChangeSide,
        RailEdit::Remove,
        RailEdit::MappingOnly,
    ] {
        let edited = bend(None, Some((Some(&bytes), edit)));
        let output = compile(&edited).unwrap();
        let result = emit_portable_candidate(
            &output,
            &provenance,
            FormatLimits::HARD,
            PortableDiffBase::Artifact(checked),
        )
        .unwrap();
        assert_ne!(result.network_revision(), base.network_revision());
        let delta = preflight_object_values(
            result.semantic_diff().bytes(),
            PortableObjectKind::SemanticDiff,
            FormatLimits::HARD,
        )
        .unwrap();
        let changes: Vec<_> = delta
            .registry_view()
            .section(7)
            .unwrap()
            .table(0)
            .unwrap()
            .rows()
            .map(|r| {
                (
                    r.field_by_tag(2).unwrap().value_bytes()[0],
                    r.field_by_tag(1).unwrap().value_bytes()[0],
                )
            })
            .collect();
        match edit {
            RailEdit::Raise => assert_eq!(changes, [(11, 2)]),
            RailEdit::Rename | RailEdit::ChangeSide => {
                assert_eq!(changes.len(), 4);
                for v in [(7, 0), (7, 1), (11, 0), (11, 1)] {
                    assert!(changes.contains(&v));
                }
            }
            RailEdit::MoveEnd => {
                assert!(changes.contains(&(7, 2)));
                assert!(changes.iter().any(|c| c.0 == 9));
            }
            RailEdit::Remove => {
                assert!(changes.contains(&(7, 1)));
                assert!(changes.contains(&(11, 1)));
            }
            RailEdit::MappingOnly => {
                let v = preflight_object_values(
                    result.canonical_artifact().bytes(),
                    PortableObjectKind::CanonicalArtifact,
                    FormatLimits::HARD,
                )
                .unwrap();
                let s = v.registry_view().section(8).unwrap();
                assert_eq!(s.table(6).unwrap().row_count(), 3);
                for t in 7..11 {
                    assert_eq!(s.table(t).unwrap().row_count(), 0);
                }
            }
            RailEdit::Baseline => unreachable!(),
        }
    }
}

fn check_bounds(bytes: &[u8], output: &CompilationOutput, peak_nm: i64) {
    let mut previous = None;
    for spatial in [
        SpatialBuildOption::Omit,
        SpatialBuildOption::RetainAvailable,
    ] {
        let checked = check_canonical_network_input(bytes, FormatLimits::HARD).unwrap();
        let build = |retained| {
            build_shared_network_revision(
                checked.clone(),
                SharedNetworkBuildOptions::new(
                    spatial,
                    SharedNetworkBuildLimits::new(retained, 16 << 20),
                ),
            )
        };
        let root = build(64 << 20).unwrap();
        let retained = root.retained_logical_bytes();
        assert!(build(retained).is_ok());
        assert!(matches!(
            build(retained - 1),
            Err(laneflow_static_network::BuildError::BudgetExceeded {
                structure: laneflow_static_network::BuildStructure::RetainedOutput,
                ..
            })
        ));
        let map = root.station_mapping();
        let band_table = checked
            .value_checked_view()
            .registry_view()
            .section(8)
            .unwrap()
            .table(5)
            .unwrap();
        let number = |row: laneflow_format::RegistryCheckedRowView<'_>, tag| {
            u32::from_le_bytes(
                row.field_by_tag(tag)
                    .unwrap()
                    .value_bytes()
                    .try_into()
                    .unwrap(),
            )
        };
        let mut actual = Vec::new();
        assert_eq!(output.lir().facility_bands().count(), 2);
        for band in output.lir().facility_bands() {
            let row = band_table
                .rows()
                .find(|row| number(*row, 1) as usize == band.ordinal().index())
                .unwrap();
            let error = i64::from(number(row, 8));
            assert!(error <= 1000);
            let corridor = band.road_corridor();
            let domain = map.corridor_domain(corridor).unwrap();
            let length = domain.length_nm;
            let rising = domain.origin_nm == 0;
            assert_eq!(length, (LEG * 1e9).round() as u64);
            let widths = |station_nm: u64| {
                let from_tip = if rising {
                    station_nm
                } else {
                    length - station_nm
                };
                let half_width = peak_nm as f64 * (from_tip as f64 / length as f64) * 0.5;
                [-1_750_000_000.0 + half_width, -1_750_000_000.0 - half_width]
            };
            for stations in [[0, 0], [length / 2; 2], [length; 2], [length / 2, length]] {
                let bounds = map
                    .facility_band_bounds(corridor, band.ordinal(), stations)
                    .unwrap();
                let [a, b] = stations.map(widths);
                for (observed, x, y) in
                    [(bounds.left_nm, a[0], b[0]), (bounds.right_nm, a[1], b[1])]
                {
                    let min = x.min(y);
                    let max = x.max(y);
                    assert_eq!(
                        observed,
                        [min.floor() as i64 - error, max.ceil() as i64 + error]
                    );
                }
                actual.push(bounds);
            }
        }
        if let Some(headless) = &previous {
            assert_eq!(&actual, headless);
        }
        previous = Some(actual);
    }
}

#[test]
fn median_width_changes_revision_and_two_band_rows_with_identical_centerlines() {
    let wide = source(None);
    assert!(
        FacilityBandInput::read_from(
            RoadEditingModuleInput::try_new(DOCUMENT, &wide, None).unwrap(),
            &RoadCorridorReference::local("other-corridor").unwrap(),
            "median",
            &CompileProfile::single_network_10k_stable_entities(),
        )
        .unwrap()
        .is_none()
    );
    let narrow = source(Some((&wide, Edit::NarrowPeak)));
    let provenance = PortableEmissionProvenance::try_new(DOCUMENT).unwrap();
    let wide_output = compile(&wide).unwrap();
    let narrow_output = compile(&narrow).unwrap();
    let wide = emit_portable_candidate(
        &wide_output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    let wide_bytes = wide.canonical_artifact().bytes();
    let wide_view = preflight_object_values(
        wide_bytes,
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap();
    let narrow = emit_portable_candidate(
        &narrow_output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Artifact(wide_view),
    )
    .unwrap();
    let narrow_bytes = narrow.canonical_artifact().bytes();
    let narrow_view = preflight_object_values(
        narrow_bytes,
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap();
    for section in 0..6 {
        // 此范围含全部身份、拓扑和空间几何，按实际 LFCA 核对两车道及设施带的中心线。
        assert_eq!(
            wide_view.registry_view().section(section).unwrap().bytes(),
            narrow_view
                .registry_view()
                .section(section)
                .unwrap()
                .bytes()
        );
    }
    assert_ne!(
        check_canonical_network_input(wide_bytes, FormatLimits::HARD)
            .unwrap()
            .network_revision(),
        check_canonical_network_input(narrow_bytes, FormatLimits::HARD)
            .unwrap()
            .network_revision()
    );
    check_bounds(wide_bytes, &wide_output, 500_000_000);
    check_bounds(narrow_bytes, &narrow_output, 250_000_000);
    let diff = preflight_object_values(
        narrow.semantic_diff().bytes(),
        PortableObjectKind::SemanticDiff,
        FormatLimits::HARD,
    )
    .unwrap()
    .registry_view();
    let changes = diff.section(7).unwrap().table(0).unwrap();
    assert_eq!(changes.row_count(), 2);
    let mut ends_changed = Vec::new();
    for row in changes.rows() {
        let field = |tag| row.field_by_tag(tag).unwrap().value_bytes();
        assert_eq!(field(1), [2]);
        assert_eq!(field(2), [6]);
        let offsets = |value: &[u8]| -> [i32; 4] {
            core::array::from_fn(|i| {
                i32::from_le_bytes(value[16 + i * 4..20 + i * 4].try_into().unwrap())
            })
        };
        let (old, new) = (offsets(field(4)), offsets(field(5)));
        let mut changed = Vec::new();
        for i in [0, 2] {
            if old[i] == new[i] {
                assert_eq!(old[i..i + 2], [-1_750_000; 2]);
                assert_eq!(new[i..i + 2], [-1_750_000; 2]);
            } else {
                assert_eq!(old[i..i + 2], [-1_500_000, -2_000_000]);
                assert_eq!(new[i..i + 2], [-1_625_000, -1_875_000]);
                changed.push(i);
            }
        }
        assert_eq!(changed.len(), 1);
        ends_changed.push(changed[0]);
    }
    ends_changed.sort_unstable();
    assert_eq!(ends_changed, [0, 2]);
}

#[test]
fn tapered_median_reuses_both_lane_interfaces_including_its_zero_width_tips() {
    use laneflow_static_network::CorridorElement::{FacilityBand, RoadSection};
    let original = source(None);
    let bytes = bend(
        Some((&original, Edit::FractionalLaneWidths)),
        Some((None, RailEdit::Baseline)),
    );
    let output = compile(&bytes).unwrap();
    let candidate = emit_portable_candidate(
        &output,
        &PortableEmissionProvenance::try_new(DOCUMENT).unwrap(),
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    let checked =
        check_canonical_network_input(candidate.canonical_artifact().bytes(), FormatLimits::HARD)
            .unwrap();
    let network = build_shared_network_revision(
        checked,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 << 20, 16 << 20),
        ),
    )
    .unwrap();
    let map = network.station_mapping();
    let relations = network.traffic().relations();
    for corridor in output.lir().road_corridors().map(|c| c.ordinal()) {
        let [RoadSection(a), FacilityBand(band), RoadSection(b)] =
            relations.corridor_elements(corridor).unwrap()
        else {
            panic!("expected median")
        };
        let footprint = |s| {
            map.lane_footprint(relations.section_lanes(s).unwrap()[0])
                .unwrap()
        };
        let domain = map.corridor_domain(corridor).unwrap();
        for (i, station) in [0, domain.length_nm].into_iter().enumerate() {
            let bounds = map
                .facility_band_bounds(corridor, *band, [station; 2])
                .unwrap();
            let mid_um = |[lo, hi]: [i64; 2]| lo.midpoint(hi) / 1000;
            assert_eq!(
                mid_um(bounds.left_nm),
                i64::from(footprint(*a).ends_um[i][1])
            );
            assert_eq!(
                mid_um(bounds.right_nm),
                i64::from(footprint(*b).ends_um[i][0])
            );
            assert_eq!(mid_um(bounds.left_nm), -1_500_001);
            assert_eq!(
                bounds.left_nm == bounds.right_nm,
                (domain.origin_nm == 0) == (i == 0)
            );
        }
    }
}

#[test]
fn positive_band_width_cannot_encode_as_a_zero_width_tip() {
    use laneflow_compiler::{
        DiagnosticPayload, RoadEditingNumericViolation, RoadEditingPropertyStep,
        RoadEditingSourceViolation, RoadEditingStructKind, RoadEditingSubject,
        RoadEditingTableKind,
    };
    let original = source(None);
    for key in ["widen", "narrow"] {
        let bytes = source(Some((&original, Edit::CollapseTip(key))));
        let err = compile(&bytes).err().expect("positive width collapsed");
        let diag = &err.diagnostics()[0];
        assert!(matches!(
            diag.payload(),
            DiagnosticPayload::InvalidRoadEditingSource {
                violation: RoadEditingSourceViolation::NumericFreeze(
                    RoadEditingNumericViolation::StationMappingNotProven
                ),
                ..
            }
        ));
        let location = diag.primary_location().unwrap().road_editing().unwrap();
        let RoadEditingSubject::Declaration { address } = location.subject() else {
            panic!("facility declaration missing")
        };
        assert_eq!(address.local_key(location.context()), "median");
        assert_eq!(
            address
                .owner_local_keys(location.context())
                .collect::<Vec<_>>(),
            [key]
        );
        assert_eq!(
            location.canvas_selection(),
            Some(format!("{key}-median").as_str())
        );
        assert_eq!(
            location.property_path().unwrap().steps(),
            &[
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::FacilityBand,
                    field_id: 2
                },
                RoadEditingPropertyStep::StructMember {
                    structure: RoadEditingStructKind::LinearWidthProfile,
                    member_id: u8::from(key == "narrow")
                },
            ]
        );
    }
}
