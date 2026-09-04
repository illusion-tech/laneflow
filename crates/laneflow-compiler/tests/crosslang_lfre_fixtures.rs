//! 跨语言 writer golden fixture 验收（#376）：C++/C# writer 产出的 size-prefixed LFRE
//! bytes 必须被生产 reader（`add_road_editing_module`）无诊断接受，且解码后的语义字段
//! 必须等于文档化的固定模块（两个 writer 描述同一个模块，而非各自漂移的合法内容）。
//! fixture 的再生成流程、钉版来源与固定模块定义见 `tools/lfre-crosslang-writer/README.md`。

use std::path::Path;

use laneflow_compiler::road_editing::RoadEditingModuleInput;
use laneflow_compiler::{CompilationUnitBuilder, CompileLimits};
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

    assert_eq!(root.format_version(), 3, "{fixture_name}: format_version");
    assert_eq!(
        root.geometry_accuracy_profile(),
        wire::GeometryAccuracyProfile::Balanced5Cm,
        "{fixture_name}: 精度 profile"
    );
    assert_eq!(
        root.geometry_direction_profile(),
        wire::GeometryDirectionProfile::Balanced2Deg,
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

    // 唯一 LaneEdge `edge-a`：限速 10 m/s，显式直线几何 (0,0,0)→(10,0,0)。
    assert_eq!(root.lane_edges().len(), 1, "{fixture_name}: edge 数");
    let edge = root.lane_edges().get(0);
    assert_eq!(edge.lane_edge_key(), "edge-a", "{fixture_name}: edge 键");
    assert_eq!(
        edge.speed_limit_meters_per_second(),
        10.0,
        "{fixture_name}: 限速"
    );
    assert_eq!(
        edge.successors().len(),
        0,
        "{fixture_name}: successors 为空"
    );
    let geometry = edge
        .explicit_geometry()
        .expect("fixture 的 edge-a 带显式几何");
    let start = geometry.start();
    assert_eq!((start.x(), start.y(), start.z()), (0.0, 0.0, 0.0));
    assert_eq!(geometry.segments().len(), 1, "{fixture_name}: 曲线段数");
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
    assert_eq!((end.x(), end.y(), end.z()), (10.0, 0.0, 0.0));

    // 其余声明向量全部为空。
    let empty_vectors: [(&str, usize); 21] = [
        ("road_alignments", root.road_alignments().len()),
        ("road_corridors", root.road_corridors().len()),
        ("road_sections", root.road_sections().len()),
        ("authoring_lanes", root.authoring_lanes().len()),
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

    let limits = CompileLimits::p100_initial_v1();
    let mut builder = CompilationUnitBuilder::new(limits);
    let input = RoadEditingModuleInput::try_new(EXPECTED_SOURCE_DOCUMENT_KEY, &bytes, None)
        .expect("fixture 的 expected key/bytes 必须满足输入契约");
    builder
        .add_road_editing_module(input)
        .unwrap_or_else(|bundle| {
            panic!("{fixture_name}: 生产 reader 必须接受跨语言 writer bytes，诊断: {bundle:?}")
        });
}

#[test]
fn cpp_writer_fixture_is_accepted_by_production_reader() {
    assert_reader_accepts("cpp_writer.lfre");
}

#[test]
fn csharp_writer_fixture_is_accepted_by_production_reader() {
    assert_reader_accepts("csharp_writer.lfre");
}
