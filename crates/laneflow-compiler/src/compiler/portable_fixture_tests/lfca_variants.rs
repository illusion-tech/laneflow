use super::*;

use sha2::{Digest, Sha256};
use std::fmt::Write as _;

const MIN_HEADLESS_BUILD_ID: &str = "laneflow-fixture-298-min-headless-v1";
const VARIANT_BUILD_ID: &str = "laneflow-fixture-298-variants-v1";
const VARIANT_ALTERNATE_BUILD_ID: &str = "laneflow-fixture-298-variants-v2";
const MIN_HEADLESS_REVISION: [u8; 32] = [
    0x4b, 0x61, 0xb2, 0x8f, 0xca, 0x27, 0xbd, 0xec, 0xd0, 0x39, 0x7f, 0x82, 0x6c, 0xfa, 0xe1, 0xad,
    0xa0, 0xb2, 0xea, 0x37, 0x5b, 0x72, 0x5d, 0xdc, 0x84, 0xec, 0xd6, 0x68, 0x96, 0x0c, 0x1c, 0x89,
];

const MIN_HEADLESS_EXPECTED: &[u8] =
    include_bytes!("../../../tests/fixtures/portable-v1/lfca-v1-variants/min-headless.lfca");
const PROVENANCE_BASE_EXPECTED: &[u8] =
    include_bytes!("../../../tests/fixtures/portable-v1/lfca-v1-variants/provenance-base.lfca");
const PROVENANCE_SOURCE_EXPECTED: &[u8] =
    include_bytes!("../../../tests/fixtures/portable-v1/lfca-v1-variants/provenance-source.lfca");
const PROVENANCE_BUILD_EXPECTED: &[u8] =
    include_bytes!("../../../tests/fixtures/portable-v1/lfca-v1-variants/provenance-build.lfca");
const REORDER_EQUIVALENT_EXPECTED: &[u8] =
    include_bytes!("../../../tests/fixtures/portable-v1/lfca-v1-variants/reorder-equivalent.lfca");
const SIGNED_ZERO_EXPECTED: &[u8] =
    include_bytes!("../../../tests/fixtures/portable-v1/lfca-v1-variants/signed-zero.lfca");
const CLAIM_MISMATCH_EXPECTED: &[u8] =
    include_bytes!("../../../tests/fixtures/portable-v1/lfca-v1-variants/claim-mismatch.lfca");

const MIN_HEADLESS_LENGTH: u64 = 1_255;
const MIN_HEADLESS_KEY: &str =
    "sha256/c799e91f6a7b20d9324bccf1e6e91a12c945c0a14d914ea396211205c72d8b2b";
const PROVENANCE_BASE_LENGTH: u64 = 1_251;
const PROVENANCE_BASE_KEY: &str =
    "sha256/eb77ff67a286a9148cc977e4a824a728ebde9269e470af7b3f7f4934b1aa8b7f";
const PROVENANCE_SOURCE_LENGTH: u64 = 1_251;
const PROVENANCE_SOURCE_KEY: &str =
    "sha256/67164e7fd5e50ed1c68a89dc3ed96c8d29b5a6375578ef68160c7cf0774d289d";
const PROVENANCE_BUILD_LENGTH: u64 = 1_251;
const PROVENANCE_BUILD_KEY: &str =
    "sha256/9cc40523737b85e443532042cda705f886bd916184c811d0c180079eb2a261d8";
const REORDER_EQUIVALENT_LENGTH: u64 = 3_185;
const REORDER_EQUIVALENT_KEY: &str =
    "sha256/1cb156511ca147d942875dc4a145e7477c6ce39a2132f56ef8f0603b0eb60d73";
const SIGNED_ZERO_LENGTH: u64 = 2_239;
const SIGNED_ZERO_KEY: &str =
    "sha256/4401ac342ee5d065694c98119d48f3ca3347ce3bb0af3f04429dc77775cc0038";
const CLAIM_MISMATCH_LENGTH: u64 = 1_255;
const CLAIM_MISMATCH_KEY: &str =
    "sha256/2267ba3128b80b86623807b816516c2052212db8d218be0f6b4e0f0d8f6ee4aa";

fn emit(output: &CompilationOutput, build_id: &str) -> crate::PortablePublicationCandidate {
    let provenance = crate::PortableEmissionProvenanceV1::try_new(build_id).unwrap();
    crate::emit_portable_candidate(
        output,
        &provenance,
        laneflow_format::FormatLimits::V1_HARD,
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
    laneflow_format::preflight_object_values_v1(
        bytes,
        laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
        laneflow_format::FormatLimits::V1_HARD,
    )
    .unwrap()
    .registry_view()
}

fn field_offset(object: &[u8], value: &[u8]) -> usize {
    let object_start = object.as_ptr() as usize;
    let value_start = value.as_ptr() as usize;
    let offset = value_start
        .checked_sub(object_start)
        .expect("field value is borrowed from object bytes");
    assert!(offset + value.len() <= object.len());
    offset
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
        .row(0)
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
) {
    assert_eq!(candidate.canonical_artifact().bytes(), expected);
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
    );
    assert_eq!(
        candidate.network_revision(),
        network_revision(MIN_HEADLESS_REVISION)
    );

    let view = artifact_view(MIN_HEADLESS_EXPECTED);
    let first_section_offset =
        field_offset(MIN_HEADLESS_EXPECTED, view.section(0).unwrap().bytes());
    assert_eq!(first_section_offset, 0x00e0);
    let expected_lengths = [120, 20, 356, 84, 94, 64];
    let expected_digests = [
        "8682b46d765cdc7cf4e880dbf1dcd8d046d6ca82990d57cf3abc2a3568220869",
        "3a85cd4b4d295cdd6cfe6ea3cb119b7c59f1addcc36faf58c33809f958191c7e",
        "54975e3435099f8ac2f6b6ec53e3bf68104d236da4a840318e9d0486a46e0f6e",
        "041fb436600f0bd293d9a9a78bb1367144e03e51ecfafca712bbc4dedb67dc19",
        "1ac4a913965b92e3dec446f935384fc18f6947038140c2feb3b474cc854dc5ed",
        "79e8acf6943d876fd8ee1f45f6856c3b8285562f0c30e4d9de559317316f025f",
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
    assert_eq!(view.section(2).unwrap().table_count(), 22);
    assert_eq!(view.section(3).unwrap().table_count(), 5);
    assert_eq!(view.section(4).unwrap().table_count(), 3);
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
    );
    assert_exact_candidate(
        &source,
        PROVENANCE_SOURCE_EXPECTED,
        PROVENANCE_SOURCE_LENGTH,
        PROVENANCE_SOURCE_KEY,
    );
    assert_exact_candidate(
        &build,
        PROVENANCE_BUILD_EXPECTED,
        PROVENANCE_BUILD_LENGTH,
        PROVENANCE_BUILD_KEY,
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
    for ordinal in 0..6 {
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
    );
    assert_exact_candidate(
        &reverse,
        REORDER_EQUIVALENT_EXPECTED,
        REORDER_EQUIVALENT_LENGTH,
        REORDER_EQUIVALENT_KEY,
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
    );
    assert_exact_candidate(
        &positive,
        SIGNED_ZERO_EXPECTED,
        SIGNED_ZERO_LENGTH,
        SIGNED_ZERO_KEY,
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
    assert_eq!(
        laneflow_format::preflight_object_values_v1(
            &negative_zero_wire,
            laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
            laneflow_format::FormatLimits::V1_HARD,
        )
        .unwrap_err()
        .class(),
        laneflow_format::FormatErrorClass::NonCanonicalValue
    );
}

#[test]
fn portable_claim_mismatch_is_structurally_valid_but_not_the_frozen_revision() {
    let candidate = min_headless_portable_fixture_candidate();
    assert_eq!(
        claim_mismatch_bytes(candidate.canonical_artifact().bytes()).as_ref(),
        CLAIM_MISMATCH_EXPECTED
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
pub(super) fn min_headless_portable_fixture_candidate() -> crate::PortablePublicationCandidate {
    emit(&empty_output(), MIN_HEADLESS_BUILD_ID)
}
