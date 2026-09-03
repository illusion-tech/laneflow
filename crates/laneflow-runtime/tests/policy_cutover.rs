use std::sync::Arc;

use laneflow_compiler::*;
use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::*;
use laneflow_static_contract::{
    EntityKind, ExactByteLength, RightOfWayPolicySetId, SEMANTIC_DIFF_FORMAT_VERSION, Sha256Digest,
};
use laneflow_static_network::*;
use sha2::{Digest, Sha256};

fn candidate(
    key: &str,
    jurisdiction: &str,
    version: &str,
    lead: u64,
    extra: bool,
    base: PortableDiffBase<'_>,
) -> PortablePublicationCandidate {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "policy-cutover",
            source_document_key: "policy-cutover.document",
            generator_build_id: "fixture-1",
            parameters_and_inputs_digest: [1; 32],
            frontend_options_digest: [2; 32],
            random_seed: None,
            provenance: "repository:policy-cutover-1",
        },
        &limits,
    )
    .unwrap();
    let mut builder = SyntheticModuleBuilder::new(header, &limits).unwrap();
    builder
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge",
            length_meters: 100.,
            speed_limit_meters_per_second: 10.,
            successors: &[],
        })
        .unwrap();
    let span = builder.policy_source_span();
    let source = PolicyInputSource {
        primary: &span,
        contributing: &[],
    };
    let gap = [PolicyGapProfileInput {
        profile_key: "gap",
        parameter_version: "gap-1",
        minimum_lead_gap_ms: lead,
        minimum_lag_gap_ms: 5,
        clearance_buffer_ms: 2,
        source,
    }];
    let policy = RightOfWayPolicySetInput {
        policy_set_key: key,
        regulation: RegulationIdentity {
            jurisdiction,
            version,
            source: Some("repository:policy-cutover-1"),
        },
        evidence: &[],
        gap_profiles: &gap,
        stream_rules: &[],
        gate_rules: &[],
        source,
    };
    builder.add_right_of_way_policy_set(policy).unwrap();
    if extra {
        // 明确挑选排序在所选身份之前的独立策略，迫使目标根 ordinal 改变。
        let extra_key = (0..100)
            .map(|n| format!("other-{n}"))
            .find(|key| id(key) < id("selected"))
            .unwrap();
        builder
            .add_right_of_way_policy_set(RightOfWayPolicySetInput {
                policy_set_key: &extra_key,
                gap_profiles: &[],
                ..policy
            })
            .unwrap();
    }
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(builder.finish().unwrap())
        .unwrap();
    let output = Compiler::new().compile(unit.build().unwrap()).unwrap();
    emit_portable_candidate(
        &output,
        &PortableEmissionProvenance::try_new("policy-cutover-1").unwrap(),
        FormatLimits::HARD,
        base,
    )
    .unwrap()
}
fn id(key: &str) -> RightOfWayPolicySetId {
    RightOfWayPolicySetId::from_untyped(
        derive_canonical_stable_id_v1(
            EntityKind::RightOfWayPolicySet,
            "policy-cutover",
            key,
            &CompileLimits::p100_initial_v1(),
        )
        .unwrap(),
    )
}
fn root(candidate: &PortablePublicationCandidate) -> Arc<SharedNetworkRevision> {
    build_shared_network_revision(
        check_canonical_network_input(candidate.canonical_artifact().bytes(), FormatLimits::HARD)
            .unwrap(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(16_777_216, 16_777_216),
        ),
    )
    .unwrap()
}
fn source(root: &SharedNetworkRevision) -> CommittedNetworkSource {
    let origin = root.canonical_origin();
    CommittedNetworkSource::Published {
        reference: PublishedLfcaReference::new(
            "fixture://policy-cutover",
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .unwrap(),
    }
}
fn world(root: Arc<SharedNetworkRevision>) -> TrafficWorld {
    TrafficWorld::install(
        Arc::clone(&root),
        WorldConfig::new(4, 4, 64, 64, 1, 100),
        source(&root),
        17,
        WorldPolicySelection::Pinned(PolicyPin {
            policy: id("selected"),
        }),
    )
    .unwrap()
}
fn descriptor(
    world: &TrafficWorld,
    target: &SharedNetworkRevision,
    bytes: &[u8],
) -> NetworkRevisionCutoverDescriptor {
    NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*world.revision().canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(*target.canonical_origin()),
        Some(SemanticDiffOriginBinding::new(
            SEMANTIC_DIFF_FORMAT_VERSION,
            Sha256Digest::from_bytes(Sha256::digest(bytes).into()),
            ExactByteLength::new(bytes.len() as u64),
        )),
        MigrationPolicyKind::CrossRevisionDirect,
        world.world_binding(),
    )
}
#[test]
fn cross_revision_preserves_pin_rebinds_ordinal_and_atomically_rebuilds_gap() {
    let base = candidate(
        "selected",
        "engineering",
        "fixture-1",
        10,
        false,
        PortableDiffBase::Genesis,
    );
    let target = candidate(
        "selected",
        "engineering",
        "fixture-1",
        30,
        true,
        PortableDiffBase::Artifact(
            laneflow_format::preflight_object_values(
                base.canonical_artifact().bytes(),
                laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
                FormatLimits::HARD,
            )
            .unwrap(),
        ),
    );
    let base_root = root(&base);
    let target_root = root(&target);
    assert_ne!(
        base_root.identity().ordinal(id("selected")),
        target_root.identity().ordinal(id("selected"))
    );
    let mut world = world(base_root);
    let old_gap = world.policy_gap_profiles()[0];
    let diff = target.semantic_diff().bytes();
    let descriptor = descriptor(&world, &target_root, diff);
    let transaction = world
        .prepare_cross_revision_cutover(
            Arc::clone(&target_root),
            source(&target_root),
            &descriptor,
            diff,
            &CutoverPreflightLimits::new(1_048_576),
            &CutoverTransactionLimits::default(),
        )
        .unwrap();
    assert_eq!(world.policy_gap_profiles()[0], old_gap);
    world.step(TickInput::new(100)).unwrap(); // 在途 tick 追赶也保持所选身份。
    let _commit = transaction.commit(&mut world).unwrap();
    assert_eq!(
        world.policy_selection(),
        WorldPolicySelection::Pinned(PolicyPin {
            policy: id("selected")
        })
    );
    assert_eq!(world.policy().unwrap().id(), id("selected"));
    assert_eq!(world.policy_gap_profiles()[0].required_lead_ms(), 132);
    assert_eq!(world.frontier_proof_horizon_ms(), Some(133));
    assert_eq!(world.tick_index(), 1);
    let saved = world.capture_snapshot().unwrap();
    let restored = restore_lfrs(
        &encode_lfrs(&saved),
        target_root,
        world.committed_source().clone(),
        world.config(),
        SnapshotRestoreLimits::new(1_048_576, 1_024),
    )
    .unwrap();
    assert_eq!(
        deterministic_state_digest(&saved).unwrap(),
        deterministic_state_digest(&restored.world().capture_snapshot().unwrap()).unwrap()
    );
}
#[test]
fn incompatible_target_rejects_without_arming_or_modifying_world() {
    let base = candidate(
        "selected",
        "engineering",
        "fixture-1",
        10,
        false,
        PortableDiffBase::Genesis,
    );
    let mut world = world(root(&base));
    let saved = world.capture_snapshot().unwrap();
    for (key, jurisdiction, version) in [
        ("replacement", "engineering", "fixture-1"),
        ("selected", "other", "fixture-1"),
        ("selected", "engineering", "fixture-2"),
    ] {
        let target = candidate(
            key,
            jurisdiction,
            version,
            20,
            false,
            PortableDiffBase::Artifact(
                laneflow_format::preflight_object_values(
                    base.canonical_artifact().bytes(),
                    laneflow_static_contract::PortableObjectKind::CanonicalArtifact,
                    FormatLimits::HARD,
                )
                .unwrap(),
            ),
        );
        let root = root(&target);
        let diff = target.semantic_diff().bytes();
        let descriptor = descriptor(&world, &root, diff);
        for _ in 0..2 {
            let error = world
                .prepare_cross_revision_cutover(
                    Arc::clone(&root),
                    source(&root),
                    &descriptor,
                    diff,
                    &CutoverPreflightLimits::new(1_048_576),
                    &CutoverTransactionLimits::default(),
                )
                .err()
                .unwrap();
            if key == "selected" {
                assert_eq!(error, CutoverError::PolicyRegulationMismatch);
            } else {
                assert!(matches!(
                    error,
                    CutoverError::PolicyInstall(InstallError::UnknownPolicy { .. })
                ));
            }
            assert_eq!(world.capture_snapshot().unwrap(), saved);
        }
    }
}
