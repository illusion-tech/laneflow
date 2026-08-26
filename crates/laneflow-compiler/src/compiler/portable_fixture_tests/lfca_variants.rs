use super::*;

use sha2::{Digest, Sha256};
use std::fmt::Write as _;

const MIN_HEADLESS_BUILD_ID: &str = "laneflow-fixture-298-min-headless-v1";
const VARIANT_BUILD_ID: &str = "laneflow-fixture-298-variants-v1";
const VARIANT_ALTERNATE_BUILD_ID: &str = "laneflow-fixture-298-variants-v2";
const MIN_HEADLESS_REVISION: [u8; 32] = [
    0x89, 0x60, 0xe5, 0xea, 0xfc, 0xdc, 0x9a, 0xc9, 0xcc, 0xc9, 0x14, 0x72, 0xe6, 0x47, 0xd1, 0xd9,
    0x9b, 0xef, 0x72, 0xf8, 0xd3, 0x96, 0x6d, 0x27, 0x10, 0xb6, 0xa6, 0xdc, 0xdb, 0x8d, 0x5f, 0x4e,
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

const MIN_HEADLESS_LENGTH: u64 = 1_175;
const MIN_HEADLESS_KEY: &str =
    "sha256/6ba6c488d93c3398bbe65261f01db52f3441813436966262c33edf6e82b15f3f";
const PROVENANCE_BASE_LENGTH: u64 = 1_171;
const PROVENANCE_BASE_KEY: &str =
    "sha256/cfddeccc1c1d809f5c60f7867e000c6b871eb449f3a5c259ba403f4d728558ba";
const PROVENANCE_SOURCE_LENGTH: u64 = 1_171;
const PROVENANCE_SOURCE_KEY: &str =
    "sha256/f419e9510d0deabdec0f863c38cf5397d423bd4d8321dbac0269f60549670119";
const PROVENANCE_BUILD_LENGTH: u64 = 1_171;
const PROVENANCE_BUILD_KEY: &str =
    "sha256/46e2b95ec8f24b800357b338351e62a13b39b61db5f0d4905bd7f2ed203c83d1";
const REORDER_EQUIVALENT_LENGTH: u64 = 3_057;
const REORDER_EQUIVALENT_KEY: &str =
    "sha256/42fc47cdc7bd77e6cc829fb42386c3d23a58ca640955a1a595f19631d155a5fc";
const SIGNED_ZERO_LENGTH: u64 = 2_151;
const SIGNED_ZERO_KEY: &str =
    "sha256/571fe4edd6a1ec3e702e23a35afe156d06b4ddb0f0aec5aaa4ffc92248eba029";
const CLAIM_MISMATCH_LENGTH: u64 = 1_175;
const CLAIM_MISMATCH_KEY: &str =
    "sha256/e98d32fc552fdb8c8f5ad9ba75bd4c7502c9801dae39ee4c03f6719de7f057b4";

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
    let expected_lengths = [120, 20, 340, 20, 94, 64];
    let expected_digests = [
        "7c85ff5c1e7b073c4446ddccedc62374b95f4fa854068c6eaaa9a628b34dab47",
        "3a85cd4b4d295cdd6cfe6ea3cb119b7c59f1addcc36faf58c33809f958191c7e",
        "49c04a89e680826aa85f1eb937e6221265799d57109864cf133c0284e96317bb",
        "3a85cd4b4d295cdd6cfe6ea3cb119b7c59f1addcc36faf58c33809f958191c7e",
        "1ac4a913965b92e3dec446f935384fc18f6947038140c2feb3b474cc854dc5ed",
        "dcd2ff297a948d5736d068e856fb9edcafbe9d721e79ef00428bedfc0ff23757",
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
    assert_eq!(view.section(2).unwrap().table_count(), 21);
    assert_eq!(view.section(3).unwrap().table_count(), 1);
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
    report.push_str(&format!(
        "min-headless revision={}\n",
        digest_hex(min.network_revision().into_digest().as_bytes())
    ));
    let view = artifact_view(min.canonical_artifact().bytes());
    for ordinal in 0..6 {
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
