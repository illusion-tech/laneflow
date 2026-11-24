//! 同中心线的变宽分隔带从 LFRE 到规范制品与语义差异的验收。
use laneflow_compiler::road_editing::*;
use laneflow_compiler::{
    CompilationOutput, CompilationUnitBuilder, CompileProfile, Compiler, DiagnosticBundle,
    GeometryAccuracyProfile, GeometryDirectionProfile, PortableDiffBase,
    PortableEmissionProvenance, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_canonical_network_input, preflight_object_values};
use laneflow_static_contract::PortableObjectKind;

const DOCUMENT: &str = "median-bend";
const LEG: f64 = 10.5 * 401.0 / 399.0;

#[derive(Clone, Copy)]
enum Edit {
    NarrowPeak,
    CollapseTip(&'static str),
}

fn source(previous: Option<(&[u8], Edit)>) -> Vec<u8> {
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
            .with_station_mapping(true),
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
                Edit::CollapseTip(_) => [w0, w1],
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
            add(RoadEditingDeclaration::AuthoringLane(authoring_lane));
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
