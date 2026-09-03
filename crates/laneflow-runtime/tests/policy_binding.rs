use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, InstallError, PolicyPin, PublishedLfcaReference, SnapshotRestoreLimits,
    TrafficWorld, WorldConfig, WorldPolicySelection, deterministic_state_digest, encode_lfrs,
    restore_lfrs,
};
use laneflow_static_contract::{
    EntityKind, ManeuverGateOrdinal, ParticipantStreamOrdinal, RightOfWayPolicySetId,
    RightOfWayPolicySetOrdinal, StableId128, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use std::sync::Arc;

const POLICIES: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/expected.lfca"
);
const HEADLESS: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-variants/min-headless.lfca"
);

fn root(bytes: &[u8]) -> Arc<SharedNetworkRevision> {
    build_shared_network_revision(
        check_canonical_network_input(bytes, FormatLimits::HARD).unwrap(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .unwrap()
}
fn install(
    root: &Arc<SharedNetworkRevision>,
    selection: WorldPolicySelection,
    dt: u64,
) -> Result<TrafficWorld, InstallError> {
    let origin = root.canonical_origin();
    TrafficWorld::install(
        Arc::clone(root),
        WorldConfig::new(4, 4, 1_024, 1_024, 1, dt),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://w3",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .unwrap(),
        },
        1,
        selection,
    )
}
fn pin(root: &SharedNetworkRevision, ordinal: u32) -> WorldPolicySelection {
    WorldPolicySelection::Pinned(PolicyPin {
        policy: root
            .identity()
            .stable_id(RightOfWayPolicySetOrdinal::from_raw(ordinal))
            .unwrap(),
    })
}

#[test]
fn same_root_worlds_isolate_selection_attribution_targets_and_step_derivation() {
    let root = root(POLICIES);
    assert_eq!(
        root.identity()
            .entity_count(EntityKind::RightOfWayPolicySet),
        2
    );
    for order in [[0, 1], [1, 0]] {
        let a = install(&root, pin(&root, order[0]), 100).unwrap();
        let b = install(&root, pin(&root, order[1]), 200).unwrap();
        assert!(Arc::ptr_eq(&a.revision(), &b.revision()));
        assert_ne!(a.policy_selection(), b.policy_selection());
        let profile = VehicleProfileOrdinal::from_raw(0);
        let mut priority_difference = false;
        let mut gate_difference = false;
        let mut target_difference = false;
        for s in 0..root.identity().entity_count(EntityKind::ParticipantStream) {
            let stream = ParticipantStreamOrdinal::from_raw(s);
            let pa = a.policy().unwrap();
            let pb = b.policy().unwrap();
            priority_difference |= pa.stream(stream, profile).unwrap().priority()
                != pb.stream(stream, profile).unwrap().priority();
            let aa = pa.stream_attribution(stream, profile).unwrap();
            let ab = pb.stream_attribution(stream, profile).unwrap();
            assert_ne!(aa.policy, ab.policy);
            assert_eq!(aa.policy, pa.id());
            assert_eq!(ab.policy, pb.id());
            for (i, passage) in root
                .conflict()
                .participant_stream(stream)
                .unwrap()
                .passages()
                .iter()
                .enumerate()
            {
                let (zone, at) = pa.yield_targets(stream, profile, i as u32).unwrap();
                let (_, bt) = pb.yield_targets(stream, profile, i as u32).unwrap();
                assert_eq!(zone, passage.conflict_zone());
                target_difference |= at != bt;
                for t in at {
                    assert_eq!(
                        root.conflict()
                            .participant_stream(t.stream())
                            .unwrap()
                            .passages()[t.passage_local_index() as usize]
                            .conflict_zone(),
                        zone
                    );
                    assert!(
                        pa.stream(t.stream(), profile).unwrap().priority()
                            > pa.stream(stream, profile).unwrap().priority()
                    );
                }
            }
        }
        for g in 0..root.identity().entity_count(EntityKind::ManeuverGate) {
            let gate = ManeuverGateOrdinal::from_raw(g);
            gate_difference |= a
                .policy()
                .unwrap()
                .gate(gate, profile)
                .unwrap()
                .prohibition()
                != b.policy()
                    .unwrap()
                    .gate(gate, profile)
                    .unwrap()
                    .prohibition();
        }
        assert!(priority_difference && gate_difference && target_difference);
        // 同一策略换步长：lead 覆盖的未来 interval 增长，已清空后的 lag 阈值不变。
        let slower = install(&root, a.policy_selection(), 200).unwrap();
        for (fast_gap, slow_gap) in a
            .policy_gap_profiles()
            .iter()
            .zip(slower.policy_gap_profiles())
        {
            assert_eq!(
                slow_gap.required_lead_ms() - fast_gap.required_lead_ms(),
                100
            );
            assert_eq!(slow_gap.required_lag_ms(), fast_gap.required_lag_ms());
        }
        for (world, dt) in [(&a, 100), (&b, 200)] {
            let raw = &world.policy().unwrap().gap_profiles()[0];
            let derived = world.policy_gap_profiles()[0];
            assert_eq!(
                derived.required_lead_ms(),
                dt + raw.minimum_lead_ms() + raw.clearance_ms()
            );
            assert_eq!(
                derived.required_lag_ms(),
                raw.minimum_lag_ms() + raw.clearance_ms()
            );
            assert_eq!(
                world.frontier_proof_horizon_ms(),
                Some(derived.required_lead_ms() + 1)
            );
        }
    }
}

#[test]
fn not_required_is_structural_and_unknown_pin_never_falls_back() {
    let headless = root(HEADLESS);
    let world = install(&headless, WorldPolicySelection::NotRequired, 100).unwrap();
    assert!(world.policy().is_none());
    assert!(world.policy_gap_profiles().is_empty());
    let gated = root(POLICIES);
    assert_eq!(
        install(&gated, WorldPolicySelection::NotRequired, 100).err(),
        Some(InstallError::PolicyRequired)
    );
    let unknown = RightOfWayPolicySetId::from_untyped(StableId128::ZERO);
    assert_eq!(
        install(
            &gated,
            WorldPolicySelection::Pinned(PolicyPin { policy: unknown }),
            100
        )
        .err(),
        Some(InstallError::UnknownPolicy { policy: unknown })
    );
}

#[test]
fn snapshots_preserve_each_explicit_pin_and_rebuild_derived_gaps() {
    let root = root(POLICIES);
    let mut digests = Vec::new();
    for ordinal in 0..2 {
        let world = install(&root, pin(&root, ordinal), 100).unwrap();
        let snapshot = world.capture_snapshot().unwrap();
        assert_eq!(snapshot.policy_selection(), world.policy_selection());
        let bytes = encode_lfrs(&snapshot);
        let restored = restore_lfrs(
            &bytes,
            Arc::clone(&root),
            world.committed_source().clone(),
            world.config(),
            SnapshotRestoreLimits::new(1_024 * 1_024, 1_024),
        )
        .unwrap();
        assert_eq!(
            restored.world().policy_selection(),
            world.policy_selection()
        );
        assert_eq!(
            restored.world().policy_gap_profiles(),
            world.policy_gap_profiles()
        );
        let digest = deterministic_state_digest(&snapshot).unwrap();
        assert_eq!(
            digest,
            deterministic_state_digest(&restored.world().capture_snapshot().unwrap()).unwrap()
        );
        digests.push(digest);
    }
    assert_ne!(digests[0], digests[1]);
}

#[test]
fn gap_overflow_rejects_only_the_selected_policy_without_publishing_a_world() {
    let root = root(include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/overflow.lfca"
    ));
    for ordinal in 0..2 {
        let raw = root
            .policy()
            .policy(RightOfWayPolicySetOrdinal::from_raw(ordinal))
            .unwrap();
        let result = install(&root, pin(&root, ordinal), 100);
        if raw.gap_profiles()[0].minimum_lead_ms() > 1_000 {
            assert_eq!(
                result.err(),
                Some(InstallError::PolicyGapOverflow {
                    gap_profile_index: 0
                })
            );
        } else {
            assert!(result.is_ok());
        }
    }
    assert_eq!(Arc::strong_count(&root), 1);
}

#[test]
fn snapshot_policy_wire_is_required_closed_and_never_falls_back() {
    use laneflow_runtime::SnapshotRestoreError;
    use laneflow_runtime_snapshot_wire::generated::lane_flow::runtime_snapshot::v5 as wire;
    let revision = root(POLICIES);
    let world = install(&revision, pin(&revision, 0), 100).unwrap();
    let original = encode_lfrs(&world.capture_snapshot().unwrap());
    let checked = wire::size_prefixed_root_as_runtime_snapshot(&original).unwrap();
    let policy = checked.world_policy();
    let table = policy._tab.loc();
    let offset = |field| table + usize::from(policy._tab.vtable().get(field));
    let tag = offset(wire::WorldPolicyBinding::VT_SELECTION);
    let identity = offset(wire::WorldPolicyBinding::VT_POLICY);
    let root_table = checked._tab.loc();
    let vtable = |bytes: &[u8], at: usize| -> usize {
        (at as i64 - i64::from(i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()))) as usize
    };
    let restore = |bytes: &[u8]| {
        restore_lfrs(
            bytes,
            Arc::clone(&revision),
            world.committed_source().clone(),
            world.config(),
            SnapshotRestoreLimits::new(1_048_576, 1_024),
        )
        .unwrap_err()
    };
    for invalid in [0, 1, 255] {
        // 1 携带了多余的 policy；不降级为 NotRequired。
        let mut bytes = original.clone();
        bytes[tag] = invalid;
        assert_eq!(restore(&bytes), SnapshotRestoreError::InvalidPolicyBinding);
    }
    let mut absent_id = original.clone();
    let field = vtable(&absent_id, table) + usize::from(wire::WorldPolicyBinding::VT_POLICY);
    absent_id[field..field + 2].fill(0);
    assert_eq!(
        restore(&absent_id),
        SnapshotRestoreError::InvalidPolicyBinding
    );
    let mut absent_binding = original.clone();
    let field =
        vtable(&absent_binding, root_table) + usize::from(wire::RuntimeSnapshot::VT_WORLD_POLICY);
    absent_binding[field..field + 2].fill(0);
    assert_eq!(
        restore(&absent_binding),
        SnapshotRestoreError::InvalidFlatbuffer
    );
    let mut unknown_id = original.clone();
    unknown_id[identity..identity + 16].fill(0);
    assert!(matches!(
        restore(&unknown_id),
        SnapshotRestoreError::Install(InstallError::UnknownPolicy { .. })
    ));
    let mut extra_field = original.clone();
    let at = vtable(&extra_field, table);
    extra_field[at..at + 2].copy_from_slice(&10_u16.to_le_bytes());
    assert_eq!(
        restore(&extra_field),
        SnapshotRestoreError::UnknownTableFields {
            table: "WorldPolicyBinding",
            supported: 2,
            actual: 3,
        }
    );
}
