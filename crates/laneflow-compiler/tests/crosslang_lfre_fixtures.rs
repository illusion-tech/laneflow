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

fn assert_fixture_vehicle(root: wire::RoadEditingSource<'_>) {
    assert_eq!(root.participant_classes().len(), 1);
    assert_eq!(
        root.participant_classes().get(0).participant_class_key(),
        "motor"
    );
    assert_eq!(root.vehicle_profiles().len(), 1);
    let profile = root.vehicle_profiles().get(0);
    assert_eq!(profile.vehicle_profile_key(), "car");
    assert_eq!(profile.participant_class(), "motor");
    assert_eq!(profile.iidm().length_meters(), 4.15);
    let g = profile.geometry().unwrap();
    assert_eq!(g.configuration(), Some(1));
    assert_eq!(
        [
            g.wheelbase_m(),
            g.track_width_m(),
            g.reference_height_m(),
            g.rear_overhang_m(),
            g.width_m(),
            g.bottom_height_m(),
            g.top_height_m(),
            g.max_steering_angle_rad(),
            g.max_steering_rate_rad_s(),
        ],
        [2.60, 1.501, 0., 0., 1.901, -0.2, 1.4, 0.6, 0.8].map(Some)
    );
}

/// 把固定模块的文档化语义逐字段断言在解码后的 wire 视图上；任一 writer 内容漂移
/// （限速、几何、键、向量基数）都会在此失败。
fn assert_fixed_module_semantics(fixture_name: &str, bytes: &[u8]) {
    let root = wire::size_prefixed_root_as_road_editing_source(bytes)
        .unwrap_or_else(|_| panic!("{fixture_name}: size-prefixed root 解码失败"));

    assert_eq!(root.format_version(), 8, "{fixture_name}: format_version");
    assert_fixture_vehicle(root);
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
                .unwrap()
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
    let empty_vectors: [(&str, usize); 15] = [
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
        ("access_rules", root.access_rules().len()),
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
    assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), 9);
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

#[test]
fn cross_language_complete_bend_preserves_station_support_and_named_payloads() {
    use laneflow_compiler::{
        Compiler, PortableDiffBase, PortableEmissionProvenance, emit_portable_candidate,
    };
    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_static_network::{
        BoundaryPayload, SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };
    let mut expected = None;
    for filename in ["cpp_writer_complete.lfre", "csharp_writer_complete.lfre"] {
        let bytes = read_fixture(filename);
        let input =
            RoadEditingModuleInput::try_new(EXPECTED_SOURCE_DOCUMENT_KEY, &bytes, None).unwrap();
        let profile = CompileProfile::single_network_10k_stable_entities();
        let owner = RoadSectionReference::owner_scoped(vec!["road".into()], "section").unwrap();
        let decl = BoundaryExtentInput::read_from(input, BoundaryOwner::Section(&owner), &profile)
            .unwrap()
            .unwrap();
        assert_eq!(decl.len(), 3);
        let b = AuthoringLaneInput::read_from(input, &owner, "B", &profile)
            .unwrap()
            .unwrap();
        assert_eq!(b.supports_wheel(), Some(false));
        let wire = wire::size_prefixed_root_as_road_editing_source(&bytes).unwrap();
        assert_fixture_vehicle(wire);
        assert!(
            wire.authoring_lanes()
                .get(1)
                .interior_cut_stations_meters()
                .is_none()
        );
        let mut builder = CompilationUnitBuilder::new(profile);
        builder.add_road_editing_module(input).unwrap();
        let output = Compiler::new().compile(builder.build().unwrap()).unwrap();
        let result = emit_portable_candidate(
            &output,
            &PortableEmissionProvenance::try_new("crosslang-complete").unwrap(),
            FormatLimits::HARD,
            PortableDiffBase::Genesis,
        )
        .unwrap();
        let checked =
            check_canonical_network_input(result.canonical_artifact().bytes(), FormatLimits::HARD)
                .unwrap();
        for spatial in [
            SpatialBuildOption::Omit,
            SpatialBuildOption::RetainAvailable,
        ] {
            let shared = build_shared_network_revision(
                checked.clone(),
                SharedNetworkBuildOptions::new(
                    spatial,
                    SharedNetworkBuildLimits::new(64 << 20, 16 << 20),
                ),
            )
            .unwrap();
            let car = shared
                .traffic()
                .relations()
                .vehicle_profile(
                    laneflow_static_contract::VehicleProfileOrdinal::try_from_usize(0).unwrap(),
                )
                .unwrap();
            let geometry = car.geometry();
            assert_eq!(car.length_mm(), 4_150);
            assert_eq!(
                [
                    geometry.wheelbase_mm,
                    geometry.track_width_mm,
                    geometry.rear_overhang_mm,
                    geometry.width_mm
                ],
                [2_600, 1_501, 0, 1_901]
            );
            assert_eq!(
                [
                    geometry.reference_height_mm,
                    geometry.bottom_height_mm,
                    geometry.top_height_mm
                ],
                [0, -200, 1_400]
            );
            assert_eq!(geometry.max_steering_angle_rad.to_bits(), 0x3f19_9999);
            assert_eq!(geometry.max_steering_rate_rad_s.to_bits(), 0x3f4c_cccc);
            let map = shared.station_mapping();
            let rail = map
                .boundary_extents()
                .iter()
                .find(|e| e.local_key() == "rail")
                .unwrap();
            assert_eq!(rail.stations_mm, [0, 7_035]);
            assert!(matches!(
                rail.payload,
                Some(BoundaryPayload::Guardrail {
                    anchor_role: 2,
                    heights_mm: [0, 850],
                    ..
                })
            ));
            let curb = map
                .boundary_extents()
                .iter()
                .find(|e| e.local_key() == "curb")
                .unwrap();
            assert!(matches!(
                curb.payload,
                Some(BoundaryPayload::Curb {
                    ends_mm: [[_, _, 0, 100], [_, _, 0, 180]],
                    ..
                })
            ));
            let s = map.surface_spans(rail.corridor);
            assert_eq!(s.first().unwrap().stations_mm[0].0, 0);
            assert!(
                s.windows(2)
                    .all(|w| w[0].stations_mm[1] == w[1].stations_mm[0])
            );
            assert_eq!(
                output
                    .lir()
                    .authoring_lanes()
                    .filter(|l| map
                        .lane_footprint(l.ordinal())
                        .is_some_and(|footprint| footprint.supports_wheel))
                    .count(),
                1
            );
        }
        if filename == "cpp_writer_complete.lfre" {
            // 重算修订后，结构读方仍须独立检出错属锚点、缺失路面覆盖、错绑载荷。
            let original = result.canonical_artifact().bytes();
            let table = laneflow_format::preflight_object_values(
                original,
                laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
                FormatLimits::HARD,
            )
            .unwrap();
            let obstacle = table
                .registry_view()
                .section(8)
                .unwrap()
                .table(6)
                .unwrap()
                .rows()
                .position(|r| r.field_by_tag(4).unwrap().value_bytes() == [2])
                .unwrap() as u32;
            for (table, tag, value) in [(6, 7, u32::MAX), (8, 3, 1), (9, 1, obstacle)] {
                let mut corrupt = original.to_vec();
                edit_first_station_row(&mut corrupt, 8, table, tag, |field| {
                    field.copy_from_slice(&value.to_le_bytes())
                });
                rebind_mapping_revision(&mut corrupt);
                for spatial in [
                    SpatialBuildOption::Omit,
                    SpatialBuildOption::RetainAvailable,
                ] {
                    let checked =
                        check_canonical_network_input(&corrupt[..], FormatLimits::HARD).unwrap();
                    assert!(
                        build_shared_network_revision(
                            checked,
                            SharedNetworkBuildOptions::new(
                                spatial,
                                SharedNetworkBuildLimits::new(64 << 20, 16 << 20)
                            )
                        )
                        .is_err()
                    );
                }
            }
        }
        if let Some(revision) = expected {
            assert_eq!(result.network_revision(), revision);
        }
        expected = Some(result.network_revision());
    }
}

#[test]
fn vehicle_geometry_errors_locate_the_vehicle_and_the_source_field() {
    use laneflow_compiler::{
        DiagnosticPayload, RoadEditingInputViolation as Reason, RoadEditingPropertyStep as Step,
        RoadEditingSourceViolation, RoadEditingSubject, RoadEditingTableKind as Table,
    };
    let bytes = read_fixture("cpp_writer.lfre");
    let root = wire::size_prefixed_root_as_road_editing_source(&bytes).unwrap();
    let profile = root.vehicle_profiles().get(0);
    let geometry = profile.geometry().unwrap();
    let slot = |table: usize, field: u16| {
        let displacement = i32::from_le_bytes(bytes[table..table + 4].try_into().unwrap());
        let entry = usize::try_from(table as i64 - i64::from(displacement)).unwrap()
            + 4
            + 2 * usize::from(field);
        let offset = u16::from_le_bytes(bytes[entry..entry + 2].try_into().unwrap());
        assert_ne!(offset, 0);
        (entry, table + usize::from(offset))
    };
    let mut unit =
        CompilationUnitBuilder::new(CompileProfile::single_network_10k_stable_entities());
    let mut reject = |bytes: &[u8], field: Option<u16>, reason: Reason| {
        let input =
            RoadEditingModuleInput::try_new(EXPECTED_SOURCE_DOCUMENT_KEY, bytes, None).unwrap();
        let Err(error) = unit.add_road_editing_module(input) else {
            panic!("invalid geometry accepted")
        };
        let diagnostic = &error.diagnostics()[0];
        assert!(
            matches!(diagnostic.payload(), DiagnosticPayload::InvalidRoadEditingSource {
            violation: RoadEditingSourceViolation::InvalidSemanticValue(actual), ..
        } if *actual == reason)
        );
        let loc = diagnostic
            .primary_location()
            .unwrap()
            .road_editing()
            .unwrap();
        let RoadEditingSubject::Declaration { address } = loc.subject() else {
            panic!("wrong geometry owner")
        };
        assert_eq!(address.local_key(loc.context()), "car");
        let mut path = vec![Step::TableField {
            table: Table::VehicleProfile,
            field_id: 4,
        }];
        if let Some(field_id) = field {
            path.push(Step::TableField {
                table: Table::RoadVehicleGeometry,
                field_id,
            })
        }
        assert_eq!(loc.property_path().unwrap().steps(), path);
    };
    let missing = Reason::MissingVehicleGeometryField;
    let mut no_group = bytes.clone();
    let entry = slot(profile._tab.loc(), 4).0;
    no_group[entry..entry + 2].fill(0);
    reject(&no_group, None, missing);
    for field in 0..=9 {
        let mut absent = bytes.clone();
        let entry = slot(geometry._tab.loc(), field).0;
        absent[entry..entry + 2].fill(0);
        reject(&absent, Some(field), missing);
    }
    let mut only_model = bytes.clone();
    for field in 0..9 {
        let entry = slot(geometry._tab.loc(), field).0;
        only_model[entry..entry + 2].fill(0);
    }
    let config = slot(geometry._tab.loc(), 9).1;
    for model in [0, 2, 3, 4, 255] {
        only_model[config] = model;
        reject(
            &only_model,
            Some(9),
            Reason::UnsupportedVehicleConfiguration { model },
        );
    }
    unit.add_road_editing_module(
        RoadEditingModuleInput::try_new(EXPECTED_SOURCE_DOCUMENT_KEY, &bytes, None).unwrap(),
    )
    .unwrap();
}

#[test]
fn invalid_cross_section_fields_are_located_before_admitting_the_module() {
    use laneflow_compiler::{
        RoadEditingPropertyStep as Step, RoadEditingStructKind as Struct,
        RoadEditingTableKind as Table, RoadEditingUnionKind as Union,
    };
    let valid = read_fixture("cpp_writer_complete.lfre");
    let root = wire::size_prefixed_root_as_road_editing_source(&valid).unwrap();
    let section = root
        .road_sections()
        .iter()
        .find(|s| s.road_corridor() == "road")
        .unwrap();
    let rail = section
        .boundary_extents()
        .iter()
        .find(|e| e.local_key() == "rail")
        .unwrap();
    let curb = section
        .boundary_extents()
        .iter()
        .find(|e| e.local_key() == "curb")
        .unwrap();
    let lane = root
        .authoring_lanes()
        .iter()
        .find(|l| l.authoring_lane_key() == "B")
        .unwrap();
    let mut unit =
        CompilationUnitBuilder::new(CompileProfile::single_network_10k_stable_entities());
    let field = |table, field_id| Step::TableField { table, field_id };
    let variant = |discriminant| Step::UnionVariant {
        union: Union::BoundaryPayload,
        discriminant,
    };
    // 直接改有界 wire 字段，覆盖非 Rust 写方可能交付的输入。
    let slot = |table: usize, id: usize| {
        let displacement = i32::from_le_bytes(valid[table..table + 4].try_into().unwrap());
        let entry = usize::try_from(table as i64 - i64::from(displacement)).unwrap() + 4 + 2 * id;
        let offset = u16::from_le_bytes(valid[entry..entry + 2].try_into().unwrap());
        assert_ne!(offset, 0);
        (entry, table + usize::from(offset))
    };
    let high = slot(curb.payload_as_curb().unwrap()._tab.loc(), 1).1;
    let right = slot(rail._tab.loc(), 1).1;
    let left_member = rail.left()._tab.loc();
    let invalid = [
        (
            slot(lane._tab.loc(), 8).0,
            vec![0, 0],
            vec![field(Table::AuthoringLane, 8)],
            false,
        ),
        (
            slot(rail._tab.loc(), 2).1,
            vec![1],
            vec![field(Table::BoundaryExtent, 6)],
            false,
        ),
        (
            slot(rail.payload_as_guardrail().unwrap()._tab.loc(), 3).0,
            vec![0, 0],
            vec![
                field(Table::BoundaryExtent, 7),
                variant(2),
                field(Table::Guardrail, 3),
            ],
            false,
        ),
        (
            high + 24,
            (-0.1_f64).to_le_bytes().to_vec(),
            vec![
                field(Table::BoundaryExtent, 7),
                variant(3),
                field(Table::Curb, 1),
                Step::StructMember {
                    structure: Struct::CurbEnd,
                    member_id: 3,
                },
            ],
            false,
        ),
        (
            right,
            u32::try_from(left_member - right)
                .unwrap()
                .to_le_bytes()
                .to_vec(),
            vec![field(Table::RoadSection, 5)],
            true,
        ),
        (
            slot(rail._tab.loc(), 5).1 + 8,
            (500.0_f64).to_le_bytes().to_vec(),
            vec![
                field(Table::BoundaryExtent, 5),
                Step::StructMember {
                    structure: Struct::BoundaryStation,
                    member_id: 1,
                },
            ],
            false,
        ),
    ];
    for (at, replacement, expected_path, ambiguous) in invalid {
        let mut corrupt = valid.clone();
        corrupt[at..at + replacement.len()].copy_from_slice(&replacement);
        let input =
            RoadEditingModuleInput::try_new(EXPECTED_SOURCE_DOCUMENT_KEY, &corrupt, None).unwrap();
        let Err(error) = unit.add_road_editing_module(input) else {
            panic!("invalid boundary accepted")
        };
        let location = error.diagnostics()[0]
            .primary_location()
            .unwrap()
            .road_editing()
            .unwrap();
        assert_eq!(location.property_path().unwrap().steps(), expected_path);
        if ambiguous {
            let range = location.byte_range().unwrap();
            assert_eq!(range.start() as usize, rail._tab.loc());
            assert_eq!(range.length(), 4);
        }
    }
    unit.add_road_editing_module(
        RoadEditingModuleInput::try_new(EXPECTED_SOURCE_DOCUMENT_KEY, &valid, None).unwrap(),
    )
    .unwrap();
    laneflow_compiler::Compiler::new()
        .compile(unit.build().unwrap())
        .unwrap();
}

// 模拟编辑器读回本样例中的走向与编制车道，再把它们写回同一个模块。
fn graded_alignment<'a>(
    bytes: &[u8],
    limits: &'a CompileProfile,
    include_exit_spiral: bool,
) -> RoadEditingSourceModuleBuilder<'a> {
    use laneflow_compiler::{GeometryAccuracyProfile, GeometryDirectionProfile};
    let input = RoadEditingModuleInput::try_new(EXPECTED_SOURCE_DOCUMENT_KEY, bytes, None).unwrap();
    let curve = RoadEditingCurveProgram::read_from(
        input,
        RoadEditingCurveOwner::Alignment("alignment"),
        limits,
    )
    .unwrap()
    .unwrap();
    let curve = if include_exit_spiral {
        curve
    } else {
        RoadEditingCurveProgram::try_new(curve.start(), curve.segments()[..2].to_vec()).unwrap()
    };
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
        limits,
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
    module
}

fn edit_for_mapping(
    bytes: &[u8],
    reverse: bool,
    publish: bool,
    moved: bool,
) -> OwnedRoadEditingSourceBuffer {
    let limits = CompileProfile::single_network_10k_stable_entities();
    let mut module = graded_alignment(bytes, &limits, true);
    let input = RoadEditingModuleInput::try_new(EXPECTED_SOURCE_DOCUMENT_KEY, bytes, None).unwrap();
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
        if key == "B" && moved {
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
        let bytes = edit_for_mapping(&original, reverse, false, true);
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
fn published_mapping_projects_the_graded_bend_from_lfca_alone() {
    use laneflow_compiler::{
        Compiler, PortableDiffBase, PortableEmissionProvenance, emit_portable_candidate,
    };
    use laneflow_format::{FormatLimits, check_canonical_network_input, preflight_object_values};
    use laneflow_static_contract::PortableObjectKind;
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption, StationQuery,
        build_shared_network_revision,
    };
    for (fixture, reverse) in [
        ("cpp_writer.lfre", false),
        ("csharp_writer_reverse.lfre", true),
    ] {
        let mut base = None;
        let source = read_fixture(fixture);
        for moved in [false, true] {
            let bytes = edit_for_mapping(&source, reverse, true, moved);
            let mut unit =
                CompilationUnitBuilder::new(CompileProfile::single_network_10k_stable_entities());
            unit.add_road_editing_module(
                RoadEditingModuleInput::try_new(
                    EXPECTED_SOURCE_DOCUMENT_KEY,
                    bytes.as_bytes(),
                    None,
                )
                .unwrap(),
            )
            .unwrap();
            let output = Compiler::new().compile(unit.build().unwrap()).unwrap();
            let edge = |name: &[u8]| {
                output
                    .lir()
                    .lane_edges()
                    .find(|e| e.identity_fields().any(|id| id.value_bytes() == name))
                    .unwrap()
                    .ordinal()
            };
            let (a2, b1, b2) = (edge(b"A-2"), edge(b"B-1"), edge(b"B-2"));
            let limits = FormatLimits::HARD;
            let diff_base = base.as_deref().map_or(PortableDiffBase::Genesis, |b| {
                PortableDiffBase::Artifact(
                    preflight_object_values(b, PortableObjectKind::CanonicalArtifact, limits)
                        .unwrap(),
                )
            });
            let candidate = emit_portable_candidate(
                &output,
                &PortableEmissionProvenance::try_new("graded-station-test").unwrap(),
                limits,
                diff_base,
            )
            .unwrap();
            let artifact = candidate.canonical_artifact().bytes();
            if !reverse && !moved {
                check_error_only_revision(&output, &candidate);
            }
            let diff = preflight_object_values(
                candidate.semantic_diff().bytes(),
                PortableObjectKind::SemanticDiff,
                limits,
            )
            .unwrap()
            .registry_view();
            assert!(diff.section(7).unwrap().table(0).unwrap().row_count() > 0);
            for spatial in [
                SpatialBuildOption::Omit,
                SpatialBuildOption::RetainAvailable,
            ] {
                let root = build_shared_network_revision(
                    check_canonical_network_input(artifact, limits).unwrap(),
                    SharedNetworkBuildOptions::new(
                        spatial,
                        SharedNetworkBuildLimits::new(64 << 20, 16 << 20),
                    ),
                )
                .unwrap();
                let relations = root.traffic().relations();
                let target = relations.lane_edge_authoring_lane(b1).unwrap();
                let corridor = relations
                    .section_corridor(relations.authoring_section(target).unwrap())
                    .unwrap();
                let StationQuery::Complete(exact) =
                    root.station_mapping().at_station(corridor, 60_301, target)
                else {
                    panic!("exact station unresolved")
                };
                let addresses = exact.addresses().collect::<Vec<_>>();
                assert_eq!(addresses.len(), 1);
                let expected = if moved {
                    if reverse { 4418 } else { 7732 }
                } else {
                    0
                };
                assert_eq!(addresses[0].edge, if moved || reverse { b1 } else { b2 });
                let [lo, hi] = addresses[0].progress_mm;
                if moved {
                    assert!((lo..=hi).contains(&expected) && hi - lo <= 4);
                } else {
                    assert_eq!([lo, hi], [0, 0]);
                    for (station_mm, expected_edge) in [(60_300, b1), (60_302, b2)] {
                        let StationQuery::Complete(found) = root
                            .station_mapping()
                            .at_station(corridor, station_mm, target)
                        else {
                            panic!("unresolved station {station_mm} mm, reverse={reverse}")
                        };
                        assert_eq!(
                            found
                                .addresses()
                                .map(|address| address.edge)
                                .collect::<Vec<_>>(),
                            [expected_edge],
                            "station {station_mm} mm, reverse={reverse}"
                        );
                    }
                }
                let StationQuery::Complete(ordinary) =
                    root.station_mapping().from_progress(a2, 1005, target)
                else {
                    panic!("ordinary station unresolved")
                };
                assert_eq!(
                    ordinary.addresses().map(|a| a.edge).collect::<Vec<_>>(),
                    if moved {
                        vec![b1]
                    } else if reverse {
                        vec![b2, b1]
                    } else {
                        vec![b1, b2]
                    }
                );
            }
            base = Some(artifact.to_vec());
        }
    }
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
    let station = a.interior_cut_stations_meters().unwrap().get(1);
    let start = original
        .subslice_range(b.interior_cut_stations_meters().unwrap().bytes())
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
    let same_mm = ((station * 1000.0).round_ties_even() + 0.25) / 1000.0;
    assert_ne!(same_mm, station);
    for (value, collision) in [
        (same_mm, true),
        (station.next_up(), true),
        (50.0, false),
        (f64::NAN, false),
    ] {
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

#[test]
fn partial_coextension_uses_each_corridors_local_domain() {
    use laneflow_compiler::{
        Compiler, PortableDiffBase, PortableEmissionProvenance, emit_portable_candidate,
    };
    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        StationObservation, StationQuery, build_shared_network_revision,
    };
    let limits = CompileProfile::single_network_10k_stable_entities();
    let mut module = graded_alignment(&read_fixture("cpp_writer.lfre"), &limits, false);
    let mut add = |decl| {
        module.add_declaration(decl).unwrap();
    };
    add(RoadEditingDeclaration::CanonicalFrame(
        CanonicalFrameInput::try_new("frame").unwrap(),
    ));
    let station = |u: f64| u * 401.0 / 399.0;
    let edge_ref = |key: &str| LaneEdgeReference::local(key).unwrap();
    for (corridor, start, end) in [("first", 0., 53.), ("middle", 53., 74.), ("last", 74., 85.)] {
        let section = RoadSectionReference::owner_scoped(vec![corridor.into()], "section").unwrap();
        let lane = |key| {
            AuthoringLaneReference::owner_scoped(vec![corridor.into(), "section".into()], key)
                .unwrap()
        };
        let mut members = vec![lane("A")];
        if corridor == "middle" {
            members.push(lane("B"));
        }
        add(RoadEditingDeclaration::RoadCorridor(
            RoadCorridorInput::try_new(
                corridor,
                RoadAlignmentReference::try_new("alignment").unwrap(),
                station(start),
                if corridor == "last" {
                    RoadEditingStationEnd::AlignmentEnd
                } else {
                    RoadEditingStationEnd::Finite(station(end))
                },
                section.clone(),
                lane("A"),
                vec![RoadEditingCorridorElement::RoadSection(section.clone())],
            )
            .unwrap()
            .with_station_mapping(true),
        ));
        add(RoadEditingDeclaration::RoadSection(
            RoadSectionInput::try_new(
                "section",
                "motorLane",
                members,
                RoadCorridorReference::local(corridor).unwrap(),
            )
            .unwrap(),
        ));
        type LaneChain = (
            &'static str,
            &'static [&'static str],
            &'static [f64],
            Option<&'static str>,
        );
        let chains: &[LaneChain] = match corridor {
            "first" => &[("A", &["first-A"], &[], Some("middle-A1"))],
            "middle" => &[
                (
                    "A",
                    &["middle-A1", "middle-A2", "middle-A3"],
                    &[59., 68.],
                    Some("last-A"),
                ),
                ("B", &["middle-B1", "middle-B2"], &[60.], None),
            ],
            _ => &[("A", &["last-A"], &[], None)],
        };
        for &(key, edges, cuts, successor) in chains {
            let mut source_lane = AuthoringLaneInput::try_new(
                key,
                edge_ref(edges[0]),
                RoadEditingLaneDirection::Forward,
                LinearWidthProfile::try_new(3.5, 3.5).unwrap(),
                None,
                section.clone(),
            )
            .unwrap();
            let references = edges.iter().map(|key| edge_ref(key)).collect();
            source_lane
                .replace_edge_chain(references, cuts.iter().copied().map(station).collect())
                .unwrap();
            for (i, &name) in edges.iter().enumerate() {
                let next = edges
                    .get(i + 1)
                    .copied()
                    .or(successor)
                    .map(edge_ref)
                    .into_iter()
                    .collect();
                add(RoadEditingDeclaration::LaneEdge(
                    LaneEdgeInput::try_new(name, 10., next, None).unwrap(),
                ));
            }
            add(RoadEditingDeclaration::AuthoringLane(source_lane));
        }
    }
    let source = RoadEditingSourceWriter::new(&limits)
        .write(module.finish().unwrap())
        .unwrap();
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_road_editing_module(
        RoadEditingModuleInput::try_new(EXPECTED_SOURCE_DOCUMENT_KEY, source.as_bytes(), None)
            .unwrap(),
    )
    .unwrap();
    let output = Compiler::new().compile(unit.build().unwrap()).unwrap();
    let artifact = emit_portable_candidate(
        &output,
        &PortableEmissionProvenance::try_new("partial-bend").unwrap(),
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    let edge = |key: &str| {
        output
            .lir()
            .lane_edges()
            .find(|e| {
                e.identity_fields()
                    .any(|id| id.value_bytes() == key.as_bytes())
            })
            .unwrap()
            .ordinal()
    };
    for spatial in [
        SpatialBuildOption::Omit,
        SpatialBuildOption::RetainAvailable,
    ] {
        let root = build_shared_network_revision(
            check_canonical_network_input(
                artifact.canonical_artifact().bytes(),
                FormatLimits::HARD,
            )
            .unwrap(),
            SharedNetworkBuildOptions::new(
                spatial,
                SharedNetworkBuildLimits::new(64 << 20, 16 << 20),
            ),
        )
        .unwrap();
        let relations = root.traffic().relations();
        let b = relations
            .lane_edge_authoring_lane(edge("middle-B1"))
            .unwrap();
        let mapped = root.station_mapping();
        for (key, origin, end) in [
            ("first-A", 0., 53.),
            ("middle-B1", 53., 74.),
            ("last-A", 74., 85.),
        ] {
            let StationObservation::Complete {
                corridor,
                stations_mm,
            } = mapped.stations_for_progress(edge(key), 0)
            else {
                panic!("missing source domain")
            };
            let domain = mapped.corridor_domain(corridor).unwrap();
            assert_eq!(stations_mm[0], 0);
            assert_eq!(
                domain.origin_mm(),
                (station(origin) * 1000.).round_ties_even() as u64
            );
            assert_eq!(
                domain.length_mm(),
                ((station(end) - station(origin)) * 1000.).round_ties_even() as u64
            );
            if origin != 53. {
                assert_eq!(mapped.at_station(corridor, 0, b), StationQuery::Unavailable);
                let adjacent_mm = if origin == 0. {
                    domain.length_mm() - 1
                } else {
                    1
                };
                assert_eq!(
                    mapped.at_station(corridor, adjacent_mm, b),
                    StationQuery::Unavailable
                );
                assert_eq!(
                    mapped.from_progress(edge(key), 0, b),
                    StationQuery::Unavailable
                );
                continue;
            }
            let last = edge("middle-B2");
            for (s, id, mm) in [
                (0, edge(key), 0),
                (
                    domain.length_mm(),
                    last,
                    root.traffic().lane_lengths_millimetres()[last.index()],
                ),
            ] {
                let StationQuery::Complete(found) = mapped.at_station(corridor, s, b) else {
                    panic!("boundary missing")
                };
                let out = found.addresses().collect::<Vec<_>>();
                assert_eq!(out.len(), 1);
                assert_eq!((out[0].edge, out[0].progress_mm), (id, [mm, mm]));
            }
            assert_eq!(
                mapped.at_station(corridor, domain.length_mm() + 1, b),
                StationQuery::Outside
            );
        }
    }
}

// 在已通过生产预检的首个物理块上改一个字段，同时重算该块的摘要。
// 调用方可分别验证旧修订声明、重新绑定后的制品和语义差异。
fn edit_first_station_row(
    bytes: &mut [u8],
    section: u32,
    table: u32,
    tag: u16,
    edit: impl FnOnce(&mut [u8]),
) {
    use laneflow_format::{FormatLimits, preflight_object_values};
    use laneflow_static_contract::{
        CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH, PortableObjectKind,
        TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH,
    };
    use sha2::{Digest, Sha256};
    let view = preflight_object_values(
        bytes,
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap()
    .registry_view();
    let sec = view.section(section).unwrap();
    let t = sec.table(table).unwrap();
    let field = t.rows().next().unwrap().field_by_tag(tag).unwrap();
    let field_range = bytes.subslice_range(field.value_bytes()).unwrap();
    let start = bytes.subslice_range(sec.bytes()).unwrap().start;
    let chunk = start + t.chunk_byte_offset(0).unwrap() as usize;
    let length = t.chunk_exact_byte_length(0).unwrap() as usize;
    let ordinal: u32 = sec
        .tables()
        .take(table as usize)
        .map(|t| t.chunk_count())
        .sum();
    let hash = start
        + CHUNKED_SECTION_PREAMBLE_BYTE_LENGTH as usize
        + ordinal as usize * TABLE_CHUNK_DIRECTORY_ENTRY_BYTE_LENGTH as usize
        + 40;
    edit(&mut bytes[field_range]);
    let digest = Sha256::digest(&bytes[chunk..chunk + length]);
    bytes[hash..hash + 32].copy_from_slice(&digest);
}

fn rebind_mapping_revision(bytes: &mut [u8]) {
    use laneflow_format::{FormatLimits, preflight_object_values};
    use laneflow_static_contract::{NETWORK_REVISION_DOMAIN_PREFIX, PortableObjectKind};
    use sha2::{Digest, Sha256};
    let kind = PortableObjectKind::CanonicalArtifact;
    let view = preflight_object_values(bytes, kind, FormatLimits::HARD)
        .unwrap()
        .registry_view();
    let mut h = Sha256::new();
    h.update(NETWORK_REVISION_DOMAIN_PREFIX);
    for i in [0, 1, 2, 3, 4, 5, 8] {
        let sec = view.section(i).unwrap();
        h.update(sec.kind().to_le_bytes());
        h.update(kind.section_format_version().to_le_bytes());
        h.update((sec.bytes().len() as u64).to_le_bytes());
        h.update(sec.bytes());
    }
    edit_first_station_row(bytes, 7, 0, 1, |claim| claim.copy_from_slice(&h.finalize()));
}

fn check_error_only_revision(
    output: &laneflow_compiler::CompilationOutput,
    candidate: &laneflow_compiler::PortablePublicationCandidate,
) {
    use laneflow_compiler::{
        PortableDiffBase, PortableEmissionProvenance, emit_portable_candidate,
    };
    use laneflow_format::{
        CanonicalNetworkInputError, FormatLimits, check_canonical_network_input,
        check_post_emission_bundle, preflight_object_values,
    };
    use laneflow_static_contract::PortableObjectKind;
    use laneflow_static_network::{
        BuildError, SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };
    let original = candidate.canonical_artifact().bytes();
    let limits = FormatLimits::HARD;
    let mut edited = original.to_vec();
    edit_first_station_row(&mut edited, 8, 4, 3, |f| {
        let old = u32::from_le_bytes((&*f).try_into().unwrap());
        f.copy_from_slice(&(old + 1).to_le_bytes());
    });
    assert!(matches!(
        check_canonical_network_input(&edited[..], limits),
        Err(CanonicalNetworkInputError::NetworkRevisionMismatch)
    ));
    rebind_mapping_revision(&mut edited);
    let build = |b: &[u8]| {
        build_shared_network_revision(
            check_canonical_network_input(b, limits).unwrap(),
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::Omit,
                SharedNetworkBuildLimits::new(64 << 20, 16 << 20),
            ),
        )
    };
    assert_ne!(
        build(original)
            .unwrap()
            .canonical_origin()
            .network_revision(),
        build(&edited)
            .unwrap()
            .canonical_origin()
            .network_revision()
    );
    assert!(
        check_post_emission_bundle(
            &edited[..],
            candidate.source_map().bytes(),
            candidate.semantic_diff().bytes(),
            candidate.expected_semantic_diff_base(),
            limits
        )
        .is_err()
    );
    let base =
        preflight_object_values(&edited, PortableObjectKind::CanonicalArtifact, limits).unwrap();
    let updated = emit_portable_candidate(
        output,
        &PortableEmissionProvenance::try_new("graded-station-test").unwrap(),
        limits,
        PortableDiffBase::Artifact(base),
    )
    .unwrap();
    let view = preflight_object_values(
        updated.semantic_diff().bytes(),
        PortableObjectKind::SemanticDiff,
        limits,
    )
    .unwrap()
    .registry_view();
    let table = view.section(7).unwrap().table(0).unwrap();
    assert_eq!(table.row_count(), 1);
    let row = table.rows().next().unwrap();
    assert_eq!(row.field_by_tag(1).unwrap().value_bytes(), [2]);
    assert_eq!(row.field_by_tag(2).unwrap().value_bytes(), [5]);
    assert_ne!(
        row.field_by_tag(4).unwrap().value_bytes(),
        row.field_by_tag(5).unwrap().value_bytes()
    );
    let mut gap = original.to_vec();
    edit_first_station_row(&mut gap, 8, 3, 3, |f| {
        f.copy_from_slice(&1u32.to_le_bytes())
    });
    rebind_mapping_revision(&mut gap);
    assert!(matches!(
        build(&gap),
        Err(BuildError::StationMappingRow {
            table_kind: 4,
            row: 0
        })
    ));
    let mut nonmonotone = original.to_vec();
    edit_first_station_row(&mut nonmonotone, 8, 4, 1, |f| {
        f.copy_from_slice(&0u32.to_le_bytes())
    });
    rebind_mapping_revision(&mut nonmonotone);
    assert!(matches!(
        build(&nonmonotone),
        Err(BuildError::StationMappingRow {
            table_kind: 5,
            row: 0
        })
    ));
}
