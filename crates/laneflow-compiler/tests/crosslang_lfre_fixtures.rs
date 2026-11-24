//! 跨语言 writer golden fixture 验收（#376）：C++/C# writer 产出的 size-prefixed LFRE
//! bytes 必须被生产 reader（`add_road_editing_module`）无诊断接受，且解码后的语义字段
//! 必须等于文档化的固定模块（两个 writer 描述同一个模块，而非各自漂移的合法内容）。
//! fixture 的再生成流程、钉版来源与固定模块定义见 `tools/lfre-crosslang-writer/README.md`。

use std::path::Path;

use laneflow_compiler::road_editing::*;
use laneflow_compiler::{CompilationUnitBuilder, CompileProfile};
use laneflow_road_editing_wire::generated::lane_flow::road_editing::v1 as wire;

const EXPECTED_SOURCE_DOCUMENT_KEY: &str = "roads/crosslang-writer";

/// 与 Rust `RoadEditingProvenance::direct` 冻结值一致
/// （`crates/laneflow-compiler/src/road_editing/model.rs`），两个 writer 共用。
const EXPECTED_INPUTS_DIGEST: [u8; 32] = [
    0x6b, 0x27, 0xd0, 0xf7, 0x66, 0x93, 0xbc, 0xd3, 0x86, 0xac, 0x13, 0xdf, 0x72, 0x4e, 0x30, 0xf5,
    0xfb, 0x5a, 0xd3, 0xb9, 0xa1, 0x52, 0xa5, 0xe1, 0xf8, 0x8d, 0xe1, 0xa6, 0x24, 0xce, 0xa8, 0xaa,
];
const EXPECTED_FRONTEND_OPTIONS_DIGEST: [u8; 32] = [
    0xb1, 0x62, 0x1e, 0x4a, 0x2d, 0xb8, 0xd7, 0x17, 0xb6, 0x50, 0x6b, 0x0a, 0xfb, 0x6f, 0xef, 0x5b,
    0xd4, 0xd5, 0x15, 0x6e, 0xcf, 0xe8, 0x87, 0xc5, 0xab, 0xf3, 0x6d, 0x08, 0x86, 0x9c, 0x78, 0x92,
];

fn read_fixture(fixture_name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/lfre-crosslang")
        .join(fixture_name);
    std::fs::read(&path)
        .unwrap_or_else(|error| panic!("无法读取 fixture `{}`: {error}", path.display()))
}

/// 把固定模块的文档化语义逐字段断言在解码后的 wire 视图上；任一 writer 内容漂移
/// （限速、几何、键、向量基数）都会在此失败。
fn assert_fixed_module_semantics(fixture_name: &str, bytes: &[u8]) {
    let root = wire::size_prefixed_root_as_road_editing_source(bytes)
        .unwrap_or_else(|_| panic!("{fixture_name}: size-prefixed root 解码失败"));

    assert_eq!(root.format_version(), 6, "{fixture_name}: format_version");
    assert_eq!(
        root.geometry_accuracy_profile(),
        wire::GeometryAccuracyProfile::Fine2Cm,
        "{fixture_name}: 精度 profile"
    );
    assert_eq!(
        root.geometry_direction_profile(),
        wire::GeometryDirectionProfile::Smooth1Deg,
        "{fixture_name}: 方向 profile"
    );

    let header = root.module_header();
    assert_eq!(
        header.authoring_namespace_id(),
        "city",
        "{fixture_name}: namespace"
    );
    assert_eq!(
        header.source_document_key(),
        EXPECTED_SOURCE_DOCUMENT_KEY,
        "{fixture_name}: 文档键"
    );
    assert_eq!(header.imports().len(), 0, "{fixture_name}: imports 为空");
    let provenance = header.provenance();
    assert_eq!(
        provenance.kind(),
        wire::ProvenanceKind::Direct,
        "{fixture_name}: provenance kind"
    );
    assert_eq!(
        provenance.generator_build_id(),
        "laneflow-road-editing-direct-v1",
        "{fixture_name}: generator build id"
    );
    assert_eq!(
        provenance
            .parameters_and_inputs_digest()
            .bytes()
            .iter()
            .collect::<Vec<u8>>(),
        EXPECTED_INPUTS_DIGEST,
        "{fixture_name}: inputs digest"
    );
    assert_eq!(
        provenance
            .frontend_options_digest()
            .bytes()
            .iter()
            .collect::<Vec<u8>>(),
        EXPECTED_FRONTEND_OPTIONS_DIGEST,
        "{fixture_name}: frontend options digest"
    );
    assert!(
        provenance.random_seed().is_none(),
        "{fixture_name}: Direct 不带 random_seed"
    );
    assert_eq!(
        provenance.description(),
        "cross-language writer fixture",
        "{fixture_name}: provenance description"
    );

    // 唯一 CanonicalFrame `frame`。
    assert_eq!(root.canonical_frames().len(), 1, "{fixture_name}: frame 数");
    assert_eq!(
        root.canonical_frames().get(0).canonical_frame_key(),
        "frame",
        "{fixture_name}: frame 键"
    );

    assert_eq!(root.road_alignments().len(), 1);
    assert_eq!(root.road_corridors().len(), 1);
    let reverse = fixture_name.contains("_reverse");
    assert_eq!(root.road_sections().len(), if reverse { 2 } else { 1 });
    assert_eq!(root.authoring_lanes().len(), 2);
    assert_eq!(root.lane_edges().len(), 8);
    for (i, (key, cuts)) in [("A", [47.0, 59.0, 68.0]), ("B", [53.0, 60.0, 74.0])]
        .into_iter()
        .enumerate()
    {
        let lane = root.authoring_lanes().get(i);
        assert_eq!(lane.authoring_lane_key(), key);
        let backward = reverse && key == "B";
        let cuts = if backward {
            [cuts[2], cuts[1], cuts[0]]
        } else {
            cuts
        };
        assert_eq!(
            lane.direction(),
            if backward {
                wire::LaneDirection::Backward
            } else {
                wire::LaneDirection::Forward
            }
        );
        assert_eq!(
            lane.road_section(),
            if backward {
                "road>opposite"
            } else {
                "road>section"
            }
        );
        assert_eq!(
            lane.interior_cut_stations_meters()
                .iter()
                .collect::<Vec<_>>(),
            cuts.map(|v| v * 401.0 / 399.0)
        );
        assert_eq!(lane.edge_chain().len(), 4);
        for (j, reference) in lane.edge_chain().iter().enumerate() {
            let edge = root.lane_edges().get(i * 4 + j);
            let edge_number = if backward { 3 - j } else { j };
            assert_eq!(reference, format!("{key}-{edge_number}"));
            assert_eq!(edge.lane_edge_key(), reference);
            assert_eq!(edge.speed_limit_meters_per_second(), 10.0);
            assert!(edge.explicit_geometry().is_none());
            assert_eq!(edge.successors().len(), usize::from(j < 3));
            if j < 3 {
                assert_eq!(edge.successors().get(0), lane.edge_chain().get(j + 1));
            }
        }
    }
    let alignment = root.road_alignments().get(0);
    assert_eq!(alignment.road_alignment_key(), "alignment");
    let geometry = alignment.reference_line();
    let start = geometry.start();
    assert_eq!((start.x(), start.y(), start.z()), (0.0, 0.0, 0.0));
    assert_eq!(geometry.segments().len(), 3, "{fixture_name}: 曲线段数");
    let segment = geometry.segments().get(0);
    assert_eq!(
        segment.geometry_type(),
        wire::CurveSegmentGeometry::LineSegment,
        "{fixture_name}: 曲线段类型"
    );
    let line = segment
        .geometry_as_line_segment()
        .expect("LineSegment union payload");
    let end = line.end();
    assert_eq!(
        (end.x(), end.y(), end.z()),
        (0.0, 40.0 * (40.0 / 399.0), 40.0)
    );

    let transition = geometry.segments().get(2);
    assert!(transition.smooth_entry());
    let spiral = transition.geometry_as_clothoid_segment().unwrap();
    assert_eq!(
        (
            spiral.length_meters(),
            spiral.start_curvature_per_meter(),
            spiral.end_curvature_per_meter(),
            spiral.grade()
        ),
        (Some(16.0), Some(1.0 / 35.0), Some(0.0), Some(40.0 / 399.0))
    );
    let bend = geometry.segments().get(1);
    assert!(!bend.smooth_entry());
    let arc = bend.geometry_as_circular_arc_segment().unwrap();
    assert_eq!(
        (arc.length_meters(), arc.signed_radius_meters(), arc.grade()),
        (Some(45.0), Some(35.0), Some(40.0 / 399.0))
    );

    // 其余声明向量全部为空。
    let empty_vectors: [(&str, usize); 17] = [
        ("junctions", root.junctions().len()),
        ("movements", root.movements().len()),
        ("maneuver_paths", root.maneuver_paths().len()),
        ("maneuver_gates", root.maneuver_gates().len()),
        ("waiting_zones", root.waiting_zones().len()),
        ("stop_lines", root.stop_lines().len()),
        ("signal_groups", root.signal_groups().len()),
        ("signal_controllers", root.signal_controllers().len()),
        ("signal_phases", root.signal_phases().len()),
        ("parking_facilities", root.parking_facilities().len()),
        ("parking_spaces", root.parking_spaces().len()),
        ("lane_groups", root.lane_groups().len()),
        ("facility_bands", root.facility_bands().len()),
        ("participant_classes", root.participant_classes().len()),
        ("access_rules", root.access_rules().len()),
        ("vehicle_profiles", root.vehicle_profiles().len()),
        ("conflict_zones", root.conflict_zones().len()),
    ];
    for (name, len) in empty_vectors {
        assert_eq!(len, 0, "{fixture_name}: {name} 应为空");
    }
    assert_eq!(root.participant_streams().len(), 0);
    assert_eq!(root.conflict_zone_regions().len(), 0);
    assert_eq!(root.right_of_way_policy_sets().len(), 0);
}

fn assert_reader_accepts(fixture_name: &str) {
    let bytes = read_fixture(fixture_name);

    // framing 快检：size prefix + LFRE identifier 必须在 verifier 之前自洽。
    assert!(bytes.len() >= 12, "{fixture_name}: 长度不足 framing 下限");
    let declared = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    assert_eq!(
        declared,
        bytes.len() - 4,
        "{fixture_name}: size prefix 与实际长度不一致"
    );
    assert_eq!(&bytes[8..12], b"LFRE", "{fixture_name}: identifier 不匹配");

    assert_fixed_module_semantics(fixture_name, &bytes);

    let limits = CompileProfile::single_network_10k_stable_entities();
    let mut builder = CompilationUnitBuilder::new(limits);
    let input = RoadEditingModuleInput::try_new(EXPECTED_SOURCE_DOCUMENT_KEY, &bytes, None)
        .expect("fixture 的 expected key/bytes 必须满足输入契约");
    builder
        .add_road_editing_module(input)
        .unwrap_or_else(|bundle| {
            panic!("{fixture_name}: 生产 reader 必须接受跨语言 writer bytes，诊断: {bundle:?}")
        });
    let output = laneflow_compiler::Compiler::new()
        .compile(builder.build().unwrap())
        .unwrap();
    assert_eq!(output.lir().lane_edges().count(), 8);
    assert_source_edge_ordinals(&output);
}

fn assert_source_edge_ordinals(output: &laneflow_compiler::CompilationOutput) {
    use laneflow_compiler::{
        PortableDiffBase, PortableEmissionProvenance, emit_portable_candidate,
    };
    use laneflow_format::{FormatLimits, RegistryCheckedRowView, preflight_object_values};
    use laneflow_static_contract::PortableObjectKind;
    fn read_u32(row: RegistryCheckedRowView<'_>, tag: u16) -> u32 {
        u32::from_le_bytes(
            row.field_by_tag(tag)
                .unwrap()
                .value_bytes()
                .try_into()
                .unwrap(),
        )
    }
    let candidate = emit_portable_candidate(
        output,
        &PortableEmissionProvenance::try_new("independent-lane-cuts").unwrap(),
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    let bytes = candidate.source_map().bytes();
    assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), 6);
    let map = preflight_object_values(bytes, PortableObjectKind::SourceMap, FormatLimits::HARD)
        .unwrap()
        .registry_view();
    let owners = map.section(3).unwrap().table(0).unwrap();
    let locations = map.section(1).unwrap().table(2).unwrap();
    let mut members = Vec::new();
    for row in (0..owners.row_count()).map(|i| owners.row(i).unwrap()) {
        if row.field_by_tag(3).unwrap().value_bytes() != [4] {
            continue;
        }
        let location = locations.row(read_u32(row, 5)).unwrap();
        assert_eq!(location.field_by_tag(17).unwrap().value_bytes(), [20]);
        assert_eq!(location.field_by_tag(18).unwrap().value_bytes(), [0]);
        let ordinal = read_u32(row, 4);
        assert_eq!(read_u32(location, 19), ordinal);
        let lane = location.field_by_tag(15).unwrap().value_bytes();
        members.push((lane, ordinal));
    }
    members.sort();
    let expected: Vec<_> = [b"A".as_slice(), b"B"]
        .into_iter()
        .flat_map(|lane| (0..4).map(move |i| (lane, i)))
        .collect();
    assert_eq!(members, expected);
}

#[test]
fn cpp_writer_fixture_is_accepted_by_production_reader() {
    assert_reader_accepts("cpp_writer.lfre");
    assert_reader_accepts("cpp_writer_reverse.lfre");
}

#[test]
fn csharp_writer_fixture_is_accepted_by_production_reader() {
    assert_reader_accepts("csharp_writer.lfre");
    assert_reader_accepts("csharp_writer_reverse.lfre");
}

// 模拟编辑器读回本样例中的走向与编制车道，再把它们写回同一个模块。
fn move_outer_cut(bytes: &[u8], reverse: bool, publish: bool) -> OwnedRoadEditingSourceBuffer {
    use laneflow_compiler::{GeometryAccuracyProfile, GeometryDirectionProfile};
    let limits = CompileProfile::single_network_10k_stable_entities();
    let input = RoadEditingModuleInput::try_new(EXPECTED_SOURCE_DOCUMENT_KEY, bytes, None).unwrap();
    let curve = RoadEditingCurveProgram::read_from(
        input,
        RoadEditingCurveOwner::Alignment("alignment"),
        &limits,
    )
    .unwrap()
    .unwrap();
    let header = RoadEditingModuleHeader::try_new(
        "city",
        EXPECTED_SOURCE_DOCUMENT_KEY,
        vec![],
        RoadEditingProvenance::direct("cross-language writer fixture").unwrap(),
    )
    .unwrap();
    let mut module = RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Fine2Cm,
        GeometryDirectionProfile::Smooth1Deg,
        &limits,
    )
    .unwrap();
    module
        .add_alignment(
            RoadAlignmentInput::try_new(
                "alignment",
                CanonicalFrameReference::local("frame").unwrap(),
                curve,
            )
            .unwrap(),
        )
        .unwrap();
    let section = RoadSectionReference::owner_scoped(vec!["road".into()], "section").unwrap();
    let opposite = RoadSectionReference::owner_scoped(vec!["road".into()], "opposite").unwrap();
    let lane_ref = |section: &str, lane| {
        AuthoringLaneReference::owner_scoped(vec!["road".into(), section.into()], lane).unwrap()
    };
    let mut elements = vec![RoadEditingCorridorElement::RoadSection(section.clone())];
    let mut forward = vec![lane_ref("section", "A")];
    if reverse {
        elements.push(RoadEditingCorridorElement::RoadSection(opposite.clone()));
    } else {
        forward.push(lane_ref("section", "B"));
    }
    let mut add = |declaration| {
        module.add_declaration(declaration).unwrap();
    };
    add(RoadEditingDeclaration::CanonicalFrame(
        CanonicalFrameInput::try_new("frame").unwrap(),
    ));
    add(RoadEditingDeclaration::RoadCorridor(
        RoadCorridorInput::try_new(
            "road",
            RoadAlignmentReference::try_new("alignment").unwrap(),
            0.0,
            RoadEditingStationEnd::AlignmentEnd,
            section.clone(),
            lane_ref("section", "A"),
            elements,
        )
        .unwrap()
        .with_station_mapping(publish),
    ));
    add(RoadEditingDeclaration::RoadSection(
        RoadSectionInput::try_new(
            "section",
            "motorLane",
            forward,
            RoadCorridorReference::local("road").unwrap(),
        )
        .unwrap(),
    ));
    if reverse {
        add(RoadEditingDeclaration::RoadSection(
            RoadSectionInput::try_new(
                "opposite",
                "motorLane",
                vec![lane_ref("opposite", "B")],
                RoadCorridorReference::local("road").unwrap(),
            )
            .unwrap(),
        ));
    }
    for key in ["A", "B"] {
        let owner = if key == "B" && reverse {
            &opposite
        } else {
            &section
        };
        let mut lane = AuthoringLaneInput::read_from(input, owner, key, &limits)
            .unwrap()
            .unwrap();
        if key == "B" {
            let before = lane.clone();
            assert!(lane.move_cut(1, 90.0).is_err());
            assert!(lane.replace_edge_chain(vec![], vec![]).is_err());
            assert_eq!(before, lane);
            lane.move_cut(1, 64.0 * 401.0 / 399.0).unwrap();
        }
        for (index, edge) in lane.edge_chain().iter().enumerate() {
            let next = lane
                .edge_chain()
                .get(index + 1)
                .cloned()
                .into_iter()
                .collect();
            add(RoadEditingDeclaration::LaneEdge(
                LaneEdgeInput::try_new(edge.local_key(), 10.0, next, None).unwrap(),
            ));
        }
        add(RoadEditingDeclaration::AuthoringLane(lane));
    }
    RoadEditingSourceWriter::new(&limits)
        .write(module.finish().unwrap())
        .unwrap()
}

#[test]
fn moved_cut_uses_the_outer_edges_own_canonical_progress() {
    for fixture in [
        "cpp_writer.lfre",
        "cpp_writer_reverse.lfre",
        "csharp_writer.lfre",
        "csharp_writer_reverse.lfre",
    ] {
        let reverse = fixture.contains("_reverse");
        let original = read_fixture(fixture);
        let mut original_unit =
            CompilationUnitBuilder::new(CompileProfile::single_network_10k_stable_entities());
        original_unit
            .add_road_editing_module(
                RoadEditingModuleInput::try_new(EXPECTED_SOURCE_DOCUMENT_KEY, &original, None)
                    .unwrap(),
            )
            .unwrap();
        let baseline = laneflow_compiler::Compiler::new()
            .compile(original_unit.build().unwrap())
            .unwrap();
        let bytes = move_outer_cut(&original, reverse, false);
        let limits = CompileProfile::single_network_10k_stable_entities();
        let mut unit = CompilationUnitBuilder::new(limits);
        let mut matches = Vec::new();
        let parameter = 20.0 / 45.0;
        unit.add_road_editing_module_with_native_chords(
            RoadEditingModuleInput::try_new(EXPECTED_SOURCE_DOCUMENT_KEY, bytes.as_bytes(), None)
                .unwrap(),
            |c| {
                if c.segment_ordinal == 1
                    && (c.parameters[0]..=c.parameters[1]).contains(&parameter)
                {
                    let mut ends = c.endpoints;
                    let mut f = (parameter - c.parameters[0]) / (c.parameters[1] - c.parameters[0]);
                    if c.backward {
                        ends.reverse();
                        f = 1.0 - f;
                    }
                    matches.push((c.target_key.to_owned(), ends, f));
                }
            },
        )
        .unwrap();
        let network = laneflow_compiler::Compiler::new()
            .compile(unit.build().unwrap())
            .unwrap();
        for index in 0..4 {
            let key = format!("A-{index}");
            let before = baseline
                .lir()
                .lane_edges()
                .find(|edge| {
                    edge.identity_fields()
                        .any(|f| f.value_bytes() == key.as_bytes())
                })
                .unwrap();
            let after = network
                .lir()
                .lane_edges()
                .find(|edge| edge.stable_id() == before.stable_id())
                .unwrap();
            assert_eq!(before.length_mm(), after.length_mm(), "{fixture} {key}");
            let left = before.spatial_geometry().unwrap();
            let right = after.spatial_geometry().unwrap();
            assert!(
                left.points()
                    .zip(right.points())
                    .all(|(a, b)| [a.x, a.y, a.z] == [b.x, b.y, b.z])
            );
            assert_eq!(left.points().count(), right.points().count());
            assert!(
                left.segments()
                    .map(|s| s.cumulative_end_meters.to_bits())
                    .eq(right.segments().map(|s| s.cumulative_end_meters.to_bits()))
            );
        }
        let mut addresses = Vec::new();
        for (key, ends, fraction) in matches {
            let edge = network
                .lir()
                .lane_edges()
                .find(|e| {
                    e.identity_fields()
                        .any(|id| id.value_bytes() == key.as_bytes())
                })
                .unwrap();
            let geo = edge.spatial_geometry().unwrap();
            let points: Vec<_> = geo.points().map(|p| [p.x, p.y, p.z]).collect();
            let mut prior = 0.0;
            for (pair, segment) in points.windows(2).zip(geo.segments()) {
                let end = f64::from(segment.cumulative_end_meters);
                if pair == ends {
                    addresses.push((
                        key.clone(),
                        laneflow_static_contract::millimetres_from_si(
                            prior + (end - prior) * fraction,
                        )
                        .unwrap(),
                    ));
                }
                prior = end;
            }
        }
        addresses.sort();
        addresses.dedup();
        assert_eq!(
            addresses,
            [
                ("A-2".to_owned(), 1005),
                ("B-1".to_owned(), if reverse { 4418 } else { 7732 })
            ],
            "{fixture} reverse={reverse}"
        );
    }
}

#[test]
fn mapping_request_requires_certified_output() {
    let bytes = move_outer_cut(&read_fixture("cpp_writer.lfre"), false, true);
    let mut unit =
        CompilationUnitBuilder::new(CompileProfile::single_network_10k_stable_entities());
    let diagnostics = unit
        .add_road_editing_module(
            RoadEditingModuleInput::try_new(EXPECTED_SOURCE_DOCUMENT_KEY, bytes.as_bytes(), None)
                .unwrap(),
        )
        .err()
        .expect("mapping must be certified before accepting this corridor");
    assert!(matches!(
        diagnostics.diagnostics()[0].payload(),
        laneflow_compiler::DiagnosticPayload::InvalidRoadEditingSource {
            violation: laneflow_compiler::RoadEditingSourceViolation::NumericFreeze(
                laneflow_compiler::RoadEditingNumericViolation::StationMappingNotProven
            ),
            ..
        }
    ));
}

#[test]
fn required_cross_sections_share_an_anchor_only_at_the_same_station() {
    use laneflow_compiler::{
        DiagnosticPayload, RoadEditingNumericViolation, RoadEditingRelationKind,
        RoadEditingRelationOccurrence, RoadEditingSourceViolation, RoadEditingSubject,
        SourceLocation,
    };
    let original = read_fixture("cpp_writer.lfre");
    let root = wire::size_prefixed_root_as_road_editing_source(&original).unwrap();
    let a = root
        .authoring_lanes()
        .iter()
        .find(|lane| lane.authoring_lane_key() == "A")
        .unwrap();
    let b = root
        .authoring_lanes()
        .iter()
        .find(|lane| lane.authoring_lane_key() == "B")
        .unwrap();
    let station = a.interior_cut_stations_meters().get(1);
    let start = original
        .subslice_range(b.interior_cut_stations_meters().bytes())
        .unwrap()
        .start
        + 8;
    let mut builder =
        CompilationUnitBuilder::new(CompileProfile::single_network_10k_stable_entities());
    let mut bytes = original.clone();
    let check_location = |source: &SourceLocation| {
        let SourceLocation::RoadEditing(location) = source else {
            panic!("expected Road Editing source")
        };
        assert!(matches!(
            location.subject(),
            RoadEditingSubject::OwnerLocal {
                relation: RoadEditingRelationKind::InteriorCut,
                occurrence: RoadEditingRelationOccurrence::OrderedProductOrdinal(1),
                ..
            }
        ));
    };
    for (value, collision) in [(station.next_up(), true), (50.0, false), (f64::NAN, false)] {
        bytes[start..start + 8].copy_from_slice(&value.to_le_bytes());
        let err = builder
            .add_road_editing_module(
                RoadEditingModuleInput::try_new(EXPECTED_SOURCE_DOCUMENT_KEY, &bytes, None)
                    .unwrap(),
            )
            .err()
            .unwrap();
        let diagnostic = &err.diagnostics()[0];
        check_location(diagnostic.primary_location().unwrap());
        if collision {
            assert!(matches!(
                diagnostic.payload(),
                DiagnosticPayload::InvalidRoadEditingSource {
                    violation: RoadEditingSourceViolation::NumericFreeze(
                        RoadEditingNumericViolation::IndistinguishableStations
                    ),
                    ..
                }
            ));
            assert_eq!(diagnostic.related_locations().len(), 1);
            check_location(&diagnostic.related_locations()[0]);
            assert_ne!(
                diagnostic.primary_location().unwrap(),
                &diagnostic.related_locations()[0]
            );
        }
    }
    bytes[start..start + 8].copy_from_slice(&station.to_le_bytes());
    builder
        .add_road_editing_module(
            RoadEditingModuleInput::try_new(EXPECTED_SOURCE_DOCUMENT_KEY, &bytes, None).unwrap(),
        )
        .unwrap();
    assert_eq!(
        laneflow_compiler::Compiler::new()
            .compile(builder.build().unwrap())
            .unwrap()
            .lir()
            .lane_edges()
            .count(),
        8
    );
}
