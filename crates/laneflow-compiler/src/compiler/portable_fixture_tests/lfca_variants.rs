use super::*;

use sha2::{Digest, Sha256};
use std::fmt::Write as _;

const MIN_HEADLESS_BUILD_ID: &str = "laneflow-fixture-298-min-headless-v1";
const VARIANT_BUILD_ID: &str = "laneflow-fixture-298-variants-v1";
const VARIANT_ALTERNATE_BUILD_ID: &str = "laneflow-fixture-298-variants-v2";
const MIN_HEADLESS_REVISION: [u8; 32] = [
    0x52, 0xba, 0xc8, 0x85, 0x4d, 0x1c, 0x12, 0xff, 0x6a, 0x0c, 0x04, 0x65, 0xf1, 0x7d, 0x68, 0x19,
    0xbb, 0x39, 0x7d, 0x6a, 0x3a, 0x36, 0xfc, 0x53, 0xa2, 0x5e, 0x3f, 0xb7, 0x05, 0x17, 0xe2, 0x87,
];

const MIN_HEADLESS_EXPECTED: &[u8] =
    include_bytes!("../../../tests/fixtures/portable/lfca-variants/min-headless.lfca");
const PROVENANCE_BASE_EXPECTED: &[u8] =
    include_bytes!("../../../tests/fixtures/portable/lfca-variants/provenance-base.lfca");
const PROVENANCE_SOURCE_EXPECTED: &[u8] =
    include_bytes!("../../../tests/fixtures/portable/lfca-variants/provenance-source.lfca");
const PROVENANCE_BUILD_EXPECTED: &[u8] =
    include_bytes!("../../../tests/fixtures/portable/lfca-variants/provenance-build.lfca");
const REORDER_EQUIVALENT_EXPECTED: &[u8] =
    include_bytes!("../../../tests/fixtures/portable/lfca-variants/reorder-equivalent.lfca");
const SIGNED_ZERO_EXPECTED: &[u8] =
    include_bytes!("../../../tests/fixtures/portable/lfca-variants/signed-zero.lfca");
const CLAIM_MISMATCH_EXPECTED: &[u8] =
    include_bytes!("../../../tests/fixtures/portable/lfca-variants/claim-mismatch.lfca");

const MIN_HEADLESS_LENGTH: u64 = 1_271;
const MIN_HEADLESS_KEY: &str =
    "sha256/2d9e6c139f0d278283f54b542e3f434dc7ce868c5ef67bebee3fab5a3f4cef35";
const PROVENANCE_BASE_LENGTH: u64 = 1_267;
const PROVENANCE_BASE_KEY: &str =
    "sha256/15e07b43dd032dcbdfee90ff5403d9ec540a913a08817a7e29e91e6ff6549b72";
const PROVENANCE_SOURCE_LENGTH: u64 = 1_267;
const PROVENANCE_SOURCE_KEY: &str =
    "sha256/11e03cebf9b86b4f9b0403f553d263392b66539965e3970a0d30226b2e2f867e";
const PROVENANCE_BUILD_LENGTH: u64 = 1_267;
const PROVENANCE_BUILD_KEY: &str =
    "sha256/f9f59a4e735ee227b131423aca39da43c841f55cd828c380d888254b7f6ea9b3";
const REORDER_EQUIVALENT_LENGTH: u64 = 3_329;
const REORDER_EQUIVALENT_KEY: &str =
    "sha256/645a07a984c664a404b9b55eaf7af56a466a114c043bf2d524a1c296f8bc4ba3";
const SIGNED_ZERO_LENGTH: u64 = 2_599;
const SIGNED_ZERO_KEY: &str =
    "sha256/67f57205e050cd501130f46964a5ce3e567af81aa6ade5a91cb05f127bb01965";
const CLAIM_MISMATCH_LENGTH: u64 = 1_271;
const CLAIM_MISMATCH_KEY: &str =
    "sha256/67f73f13d3e494439311132ce83e8ff0bd8aa40f8ac2e8269fa7bcdc70adaca6";

fn emit(output: &CompilationOutput, build_id: &str) -> crate::PortablePublicationCandidate {
    let provenance = crate::PortableEmissionProvenance::try_new(build_id).unwrap();
    crate::emit_portable_candidate(
        output,
        &provenance,
        laneflow_format::FormatLimits::HARD,
        crate::PortableDiffBase::Genesis,
    )
    .unwrap()
}

fn empty_output() -> CompilationOutput {
    Compiler::new()
        .compile(unit(std::iter::empty::<SyntheticModule>()))
        .unwrap()
}

fn provenance_module(document: &str) -> SyntheticModule {
    portable_fixture_builder("city/portable-provenance", document)
        .finish()
        .unwrap()
}

fn provenance_output(document: &str) -> CompilationOutput {
    Compiler::new()
        .compile(unit([provenance_module(document)]))
        .unwrap()
}

fn reorder_module(namespace: &str, document: &str) -> SyntheticModule {
    let root_successors = [
        LaneEdgeReference::local("branch-a"),
        LaneEdgeReference::local("branch-b"),
    ];
    let order = ["root", "branch-a", "branch-b"];
    let mut builder = portable_fixture_builder(namespace, document);
    for key in &order {
        let successors: &[LaneEdgeReference<'_>] = if *key == "root" {
            &root_successors
        } else {
            &[]
        };
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: key,
                length_meters: if *key == "root" { 1.0 } else { 2.0 },
                speed_limit_meters_per_second: 1.0,
                successors,
            })
            .unwrap();
    }
    builder.finish().unwrap()
}

fn reorder_output(reverse: bool) -> CompilationOutput {
    let primary = reorder_module("city/portable-reorder-a", "portable-reorder-a.document");
    let secondary = reorder_module("city/portable-reorder-b", "portable-reorder-b.document");
    let modules = if reverse {
        [secondary, primary]
    } else {
        [primary, secondary]
    };
    let mut compilation_unit = unit(modules);
    if reverse {
        for module in &mut compilation_unit.modules {
            module.declarations.reverse();
            for declaration in &mut module.declarations {
                if let TypedAstDeclaration::LaneEdge(edge) = declaration {
                    edge.successors.reverse();
                }
            }
        }
    }
    Compiler::new().compile(compilation_unit).unwrap()
}

fn signed_zero_module(zero: f32) -> SyntheticModule {
    let points = [
        CanonicalPoint3F32Input {
            x: zero,
            y: zero,
            z: zero,
        },
        CanonicalPoint3F32Input {
            x: 1.0,
            y: zero,
            z: zero,
        },
    ];
    let geometries = [LaneEdgeGeometryInput {
        lane_edge: LaneEdgeReference::local("signed-zero-edge"),
        centerline_points: &points,
    }];
    let mut builder =
        portable_fixture_builder("city/portable-signed-zero", "portable-signed-zero.document");
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "signed-zero-edge",
            length_meters: 1.0,
            speed_limit_meters_per_second: 1.0,
            successors: &[],
        })
        .unwrap()
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "signed-zero-frame",
            lane_edge_geometries: &geometries,
        })
        .unwrap();
    builder.finish().unwrap()
}

fn signed_zero_output(zero: f32) -> CompilationOutput {
    Compiler::new()
        .compile(unit([signed_zero_module(zero)]))
        .unwrap()
}

fn artifact_view(bytes: &[u8]) -> laneflow_format::RegistryCheckedObjectView<'_> {
    laneflow_format::preflight_object_values(
        bytes,
        laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
        laneflow_format::FormatLimits::HARD,
    )
    .unwrap()
    .registry_view()
}

fn field_offset(object: &[u8], value: &[u8]) -> usize {
    object
        .subslice_range(value)
        .expect("field value is borrowed from object bytes")
        .start
}

fn claim_bytes(bytes: &[u8]) -> &[u8] {
    artifact_view(bytes)
        .section(7)
        .unwrap()
        .table(0)
        .unwrap()
        .row(0)
        .unwrap()
        .field_by_tag(1)
        .unwrap()
        .value_bytes()
}

fn claim_mismatch_bytes(bytes: &[u8]) -> Box<[u8]> {
    let offset = field_offset(bytes, claim_bytes(bytes));
    let mut mismatch = bytes.to_vec();
    mismatch[offset] ^= 0x01;
    refresh_portable_chunk_digest_containing(
        &mut mismatch,
        laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
        offset,
    );
    mismatch.into_boxed_slice()
}

fn first_lane_edge_point_x(bytes: &[u8]) -> &[u8] {
    let geometry = artifact_view(bytes)
        .section(4)
        .unwrap()
        .table(1)
        .unwrap()
        .row(0)
        .unwrap();
    let points = match geometry.field_by_tag(4).unwrap().value().unwrap() {
        laneflow_format::RegistryCheckedFieldValue::RecordVector(points) => points,
        _ => panic!("lane-edge geometry points use RecordVector"),
    };
    points
        .rows()
        .next()
        .unwrap()
        .field_by_tag(1)
        .unwrap()
        .value_bytes()
}

fn assert_exact_candidate(
    candidate: &crate::PortablePublicationCandidate,
    expected: &[u8],
    expected_length: u64,
    expected_key: &str,
    fixture: &str,
) {
    laneflow_artifact_inspect::assert_portable_object_matches!(
        candidate.canonical_artifact().bytes(),
        expected,
        fixture
    );
    assert_eq!(
        candidate.canonical_artifact().byte_length(),
        exact_byte_length(expected_length)
    );
    assert_eq!(candidate.canonical_artifact().object_key(), expected_key);
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn digest_hex(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(64);
    for byte in digest(bytes) {
        write!(&mut value, "{byte:02x}").unwrap();
    }
    value
}

#[test]
fn portable_min_headless_matches_g1_anchor_and_frozen_exact_bytes() {
    let candidate = min_headless_portable_fixture_candidate();
    assert_exact_candidate(
        &candidate,
        MIN_HEADLESS_EXPECTED,
        MIN_HEADLESS_LENGTH,
        MIN_HEADLESS_KEY,
        "lfca-variants/min-headless.lfca（DUMP_PORTABLE 重生成）",
    );
    assert_eq!(
        candidate.network_revision(),
        network_revision(MIN_HEADLESS_REVISION)
    );

    let view = artifact_view(MIN_HEADLESS_EXPECTED);
    let first_section_offset =
        field_offset(MIN_HEADLESS_EXPECTED, view.section(0).unwrap().bytes());
    assert_eq!(first_section_offset, 0x00f8);
    let expected_lengths = [204, 16, 16, 16, 146, 148];
    let expected_digests = [
        "51e25294cc514ced0fa3ac21c7f6e428bb51c5ad30bdab63f663f4854f181016",
        "dfa9925a2db09a0a641cc3908a1b4c47b32dcf958247f428e5306b871a524276",
        "dfa9925a2db09a0a641cc3908a1b4c47b32dcf958247f428e5306b871a524276",
        "dfa9925a2db09a0a641cc3908a1b4c47b32dcf958247f428e5306b871a524276",
        "9177e786da481ef65111262d38e857431dd9db7cf6bdae03a778698d9e9f5d95",
        "e1807294856d5e781d1ca97d8a385e44a7347f42ece47d496fadb8b5f90475a6",
    ];
    for (ordinal, (length, expected_digest)) in expected_lengths
        .into_iter()
        .zip(expected_digests)
        .enumerate()
    {
        let section = view.section(u32::try_from(ordinal).unwrap()).unwrap();
        assert_eq!(section.bytes().len(), length);
        assert_eq!(digest_hex(section.bytes()), expected_digest);
    }
    assert_eq!(view.section(2).unwrap().table_count(), 24);
    assert_eq!(view.section(3).unwrap().table_count(), 5);
    assert_eq!(view.section(4).unwrap().table_count(), 4);
}

#[test]
fn portable_provenance_only_variants_preserve_revision_and_fix_distinct_bytes() {
    let base = emit(
        &provenance_output("portable-provenance-a.document"),
        VARIANT_BUILD_ID,
    );
    let source = emit(
        &provenance_output("portable-provenance-b.document"),
        VARIANT_BUILD_ID,
    );
    let build = emit(
        &provenance_output("portable-provenance-a.document"),
        VARIANT_ALTERNATE_BUILD_ID,
    );
    assert_exact_candidate(
        &base,
        PROVENANCE_BASE_EXPECTED,
        PROVENANCE_BASE_LENGTH,
        PROVENANCE_BASE_KEY,
        "lfca-variants/provenance-base.lfca（DUMP_PORTABLE 重生成）",
    );
    assert_exact_candidate(
        &source,
        PROVENANCE_SOURCE_EXPECTED,
        PROVENANCE_SOURCE_LENGTH,
        PROVENANCE_SOURCE_KEY,
        "lfca-variants/provenance-source.lfca（DUMP_PORTABLE 重生成）",
    );
    assert_exact_candidate(
        &build,
        PROVENANCE_BUILD_EXPECTED,
        PROVENANCE_BUILD_LENGTH,
        PROVENANCE_BUILD_KEY,
        "lfca-variants/provenance-build.lfca（DUMP_PORTABLE 重生成）",
    );
    let expected_revision = network_revision(MIN_HEADLESS_REVISION);
    assert_eq!(base.network_revision(), expected_revision);
    assert_eq!(source.network_revision(), expected_revision);
    assert_eq!(build.network_revision(), expected_revision);
    assert_ne!(
        base.canonical_artifact().bytes(),
        source.canonical_artifact().bytes()
    );
    assert_ne!(
        base.canonical_artifact().bytes(),
        build.canonical_artifact().bytes()
    );
    assert_ne!(base.source_map().bytes(), source.source_map().bytes());
    assert_ne!(base.source_map().bytes(), build.source_map().bytes());

    let base_view = artifact_view(PROVENANCE_BASE_EXPECTED);
    let source_view = artifact_view(PROVENANCE_SOURCE_EXPECTED);
    let build_view = artifact_view(PROVENANCE_BUILD_EXPECTED);
    for ordinal in [0, 1, 2, 3, 4, 5, 8] {
        assert_eq!(
            base_view.section(ordinal).unwrap().bytes(),
            source_view.section(ordinal).unwrap().bytes()
        );
        assert_eq!(
            base_view.section(ordinal).unwrap().bytes(),
            build_view.section(ordinal).unwrap().bytes()
        );
    }
    let base_provenance = base_view
        .section(6)
        .unwrap()
        .table(0)
        .unwrap()
        .row(0)
        .unwrap();
    let source_provenance = source_view
        .section(6)
        .unwrap()
        .table(0)
        .unwrap()
        .row(0)
        .unwrap();
    let build_provenance = build_view
        .section(6)
        .unwrap()
        .table(0)
        .unwrap()
        .row(0)
        .unwrap();
    assert_eq!(
        base_provenance.field_by_tag(1).unwrap().value_bytes(),
        source_provenance.field_by_tag(1).unwrap().value_bytes()
    );
    assert_ne!(
        base_provenance.field_by_tag(3).unwrap().value_bytes(),
        source_provenance.field_by_tag(3).unwrap().value_bytes()
    );
    assert_ne!(
        base_provenance.field_by_tag(1).unwrap().value_bytes(),
        build_provenance.field_by_tag(1).unwrap().value_bytes()
    );
    assert_eq!(
        base_provenance.field_by_tag(3).unwrap().value_bytes(),
        build_provenance.field_by_tag(3).unwrap().value_bytes()
    );
}

#[test]
fn portable_reorder_equivalent_inputs_match_one_frozen_artifact() {
    let forward = emit(&reorder_output(false), VARIANT_BUILD_ID);
    let reverse = emit(&reorder_output(true), VARIANT_BUILD_ID);
    assert_exact_candidate(
        &forward,
        REORDER_EQUIVALENT_EXPECTED,
        REORDER_EQUIVALENT_LENGTH,
        REORDER_EQUIVALENT_KEY,
        "lfca-variants/reorder-equivalent.lfca（DUMP_PORTABLE 重生成）",
    );
    assert_exact_candidate(
        &reverse,
        REORDER_EQUIVALENT_EXPECTED,
        REORDER_EQUIVALENT_LENGTH,
        REORDER_EQUIVALENT_KEY,
        "lfca-variants/reorder-equivalent.lfca（DUMP_PORTABLE 重生成）",
    );
    assert_eq!(forward.network_revision(), reverse.network_revision());
    assert_eq!(forward.source_map(), reverse.source_map());
    assert_eq!(forward.semantic_diff(), reverse.semantic_diff());
}

#[test]
fn portable_signed_zero_input_matches_positive_zero_and_wire_negative_zero_fails() {
    let negative = emit(&signed_zero_output(-0.0), VARIANT_BUILD_ID);
    let positive = emit(&signed_zero_output(0.0), VARIANT_BUILD_ID);
    assert_exact_candidate(
        &negative,
        SIGNED_ZERO_EXPECTED,
        SIGNED_ZERO_LENGTH,
        SIGNED_ZERO_KEY,
        "lfca-variants/signed-zero.lfca（DUMP_PORTABLE 重生成）",
    );
    assert_exact_candidate(
        &positive,
        SIGNED_ZERO_EXPECTED,
        SIGNED_ZERO_LENGTH,
        SIGNED_ZERO_KEY,
        "lfca-variants/signed-zero.lfca（DUMP_PORTABLE 重生成）",
    );
    assert_eq!(
        first_lane_edge_point_x(SIGNED_ZERO_EXPECTED),
        0.0_f32.to_le_bytes()
    );

    let offset = field_offset(
        SIGNED_ZERO_EXPECTED,
        first_lane_edge_point_x(SIGNED_ZERO_EXPECTED),
    );
    let mut negative_zero_wire = SIGNED_ZERO_EXPECTED.to_vec();
    negative_zero_wire[offset..offset + 4].copy_from_slice(&(-0.0_f32).to_le_bytes());
    refresh_portable_chunk_digest_containing(
        &mut negative_zero_wire,
        laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
        offset,
    );
    assert_eq!(
        laneflow_format::preflight_object_values(
            &negative_zero_wire,
            laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
            laneflow_format::FormatLimits::HARD,
        )
        .unwrap_err()
        .class(),
        laneflow_format::FormatErrorClass::NonCanonicalValue
    );
}

#[test]
fn portable_claim_mismatch_is_structurally_valid_but_not_the_frozen_revision() {
    let candidate = min_headless_portable_fixture_candidate();
    laneflow_artifact_inspect::assert_portable_object_matches!(
        claim_mismatch_bytes(candidate.canonical_artifact().bytes()).as_ref(),
        CLAIM_MISMATCH_EXPECTED,
        "lfca-variants/claim-mismatch.lfca（DUMP_PORTABLE 重生成）"
    );
    assert_eq!(CLAIM_MISMATCH_EXPECTED.len() as u64, CLAIM_MISMATCH_LENGTH);
    assert_eq!(
        format!("sha256/{}", digest_hex(CLAIM_MISMATCH_EXPECTED)),
        CLAIM_MISMATCH_KEY
    );
    let mismatch = artifact_view(CLAIM_MISMATCH_EXPECTED);
    assert_ne!(claim_bytes(CLAIM_MISMATCH_EXPECTED), MIN_HEADLESS_REVISION);
    let canonical = artifact_view(MIN_HEADLESS_EXPECTED);
    for ordinal in 0..7 {
        assert_eq!(
            canonical.section(ordinal).unwrap().bytes(),
            mismatch.section(ordinal).unwrap().bytes()
        );
    }
}

// Keep shared factories below the fixture-producing call sites: SyntheticModuleBuilder captures
// caller locations, so inserting lines above those sites would intentionally change exact bytes.
/// 发射最小 headless fixture 的可移植发布候选。
pub(super) fn min_headless_portable_fixture_candidate() -> crate::PortablePublicationCandidate {
    emit(&empty_output(), MIN_HEADLESS_BUILD_ID)
}

#[test]
fn dump_portable_variants_when_requested() {
    if std::env::var_os("DUMP_PORTABLE").is_none() {
        return;
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/portable/lfca-variants");
    std::fs::create_dir_all(&dir).unwrap();
    let write_lfca = |name: &str, bytes: &[u8]| {
        std::fs::write(dir.join(name), bytes).unwrap();
        format!(
            "{name} len={} key=sha256/{}\n",
            bytes.len(),
            digest_hex(bytes)
        )
    };
    let min = min_headless_portable_fixture_candidate();
    let mut report = write_lfca("min-headless.lfca", min.canonical_artifact().bytes());
    report.push_str("min-headless revision=");
    for byte in min.network_revision().into_digest().into_bytes() {
        write!(&mut report, "{byte:02x}").unwrap();
    }
    report.push('\n');
    let view = artifact_view(min.canonical_artifact().bytes());
    for ordinal in [0, 1, 2, 3, 4, 5, 8] {
        let section = view.section(ordinal).unwrap();
        report.push_str(&format!(
            "min-headless section{ordinal} len={} digest={}\n",
            section.bytes().len(),
            digest_hex(section.bytes())
        ));
    }
    report.push_str(&write_lfca(
        "provenance-base.lfca",
        emit(
            &provenance_output("portable-provenance-a.document"),
            VARIANT_BUILD_ID,
        )
        .canonical_artifact()
        .bytes(),
    ));
    report.push_str(&write_lfca(
        "provenance-source.lfca",
        emit(
            &provenance_output("portable-provenance-b.document"),
            VARIANT_BUILD_ID,
        )
        .canonical_artifact()
        .bytes(),
    ));
    report.push_str(&write_lfca(
        "provenance-build.lfca",
        emit(
            &provenance_output("portable-provenance-a.document"),
            VARIANT_ALTERNATE_BUILD_ID,
        )
        .canonical_artifact()
        .bytes(),
    ));
    report.push_str(&write_lfca(
        "reorder-equivalent.lfca",
        emit(&reorder_output(false), VARIANT_BUILD_ID)
            .canonical_artifact()
            .bytes(),
    ));
    report.push_str(&write_lfca(
        "signed-zero.lfca",
        emit(&signed_zero_output(0.0), VARIANT_BUILD_ID)
            .canonical_artifact()
            .bytes(),
    ));
    let mismatch = claim_mismatch_bytes(min.canonical_artifact().bytes());
    report.push_str(&write_lfca("claim-mismatch.lfca", &mismatch));
    std::fs::write(dir.join("bindings.txt"), report).unwrap();
}
