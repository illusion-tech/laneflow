use std::{
    hint::black_box,
    num::NonZeroU32,
    sync::{Arc, OnceLock},
};

use laneflow_compiler::{
    derive_canonical_stable_id_v1, emit_portable_candidate,
    road_editing::{
        CanonicalFrameInput, RoadEditingDeclaration, RoadEditingModuleHeader,
        RoadEditingModuleInput, RoadEditingProvenance, RoadEditingSourceModuleBuilder,
        RoadEditingSourceWriter,
    },
    CompilationUnitBuilder, CompileLimits, Compiler, GeometryAccuracyProfile,
    GeometryDirectionProfile, PortableDiffBase, PortableEmissionProvenance,
};
use laneflow_format::{check_canonical_network_input, check_post_emission_bundle};
use laneflow_runtime::{
    deterministic_state_digest, encode_lfrs, restore_lfrs, CommittedNetworkSource,
    EditableDiffBase, ExecutionConfig, PolicyPin, PublishedLfcaReference, RoadEditingState,
    RoadEditingStateModule, RouteRegisterInput, SnapshotRestoreLimits, TickInput, TrafficWorld,
    VehicleSpawnInput, WorldConfig, WorldPolicySelection,
};
use laneflow_static_contract::{
    EntityKind, LaneEdgeOrdinal, RightOfWayPolicySetId, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    build_shared_network_revision, SharedNetworkBuildLimits, SharedNetworkBuildOptions,
    SharedNetworkRevision, SpatialBuildOption,
};

use crate::{mutate, portable, MAX_INPUT_BYTES};

const DOCUMENT_KEY: &str = "roads/security-fuzz";

struct Fixture {
    revision: Arc<SharedNetworkRevision>,
    source: CommittedNetworkSource,
    bytes: Vec<u8>,
}

pub(crate) fn config() -> WorldConfig {
    WorldConfig::new(8, 4, 1_024, 1_024, 100)
}

pub(crate) fn execution() -> ExecutionConfig {
    ExecutionConfig::new(NonZeroU32::MIN)
}

pub(crate) fn revision(bytes: &[u8]) -> Arc<SharedNetworkRevision> {
    build_shared_network_revision(
        check_canonical_network_input(bytes, portable::limits()).expect("checked fixture"),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("shared fixture")
}

pub(crate) fn published(revision: &SharedNetworkRevision) -> CommittedNetworkSource {
    let origin = revision.canonical_origin();
    CommittedNetworkSource::Published {
        reference: PublishedLfcaReference::new(
            "fixture://security-fuzz",
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .expect("published fixture"),
    }
}

fn policy(revision: &SharedNetworkRevision) -> WorldPolicySelection {
    let policy = RightOfWayPolicySetId::from_untyped(
        derive_canonical_stable_id_v1(
            EntityKind::RightOfWayPolicySet,
            "runtime-fixture-policy",
            "fixture-policy",
            &CompileLimits::p100_initial_v1(),
        )
        .expect("fixture policy ID"),
    );
    if revision.identity().ordinal(policy).is_some() {
        return WorldPolicySelection::Pinned(PolicyPin { policy });
    }
    assert!(
        [
            EntityKind::ManeuverGate,
            EntityKind::ConflictZone,
            EntityKind::ParticipantStream
        ]
        .iter()
        .all(|kind| revision.traffic().entity_counts().count(*kind) == 0),
        "gate-bearing fixture needs an explicit policy"
    );
    WorldPolicySelection::NotRequired
}

pub(crate) fn world(
    revision: &Arc<SharedNetworkRevision>,
    source: CommittedNetworkSource,
) -> TrafficWorld {
    TrafficWorld::install(
        Arc::clone(revision),
        config(),
        execution(),
        source,
        536,
        policy(revision),
    )
    .expect("install fixture")
}

pub(crate) fn populate(world: &mut TrafficWorld, full_spatial: bool) {
    let edges = if full_spatial {
        [10_000, 8_000, 12_000]
            .map(|length| {
                let index = world
                    .traffic()
                    .lane_lengths_millimetres()
                    .iter()
                    .position(|actual| *actual == length)
                    .expect("full-spatial fixture edge");
                LaneEdgeOrdinal::try_from_usize(index).expect("fixture ordinal")
            })
            .to_vec()
    } else {
        let first = (0..world.traffic().lane_edge_count())
            .map(LaneEdgeOrdinal::from_raw)
            .find(|edge| {
                world
                    .traffic()
                    .successors(*edge)
                    .is_some_and(|items| !items.is_empty())
            })
            .expect("fixture has connected edges");
        vec![first, world.traffic().successors(first).unwrap()[0]]
    };
    let first = edges[0];
    let route = world
        .register_route(RouteRegisterInput::new(edges))
        .expect("route");
    let progress = 4_500.min(world.traffic().lane_lengths_millimetres()[first.index()]);
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, progress, 0)
                .with_open_entrance(),
        )
        .expect("vehicle");
    world.step(TickInput::new(100)).expect("seed tick");
}

pub(crate) fn lfre_module(document_key: &str) -> Vec<u8> {
    let limits = CompileLimits::p100_initial_v1();
    let header = RoadEditingModuleHeader::try_new(
        "security-fuzz",
        document_key,
        Vec::new(),
        RoadEditingProvenance::direct("security fuzz seed").expect("provenance"),
    )
    .expect("header");
    let mut builder = RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        &limits,
    )
    .expect("module builder");
    builder
        .add_declaration(RoadEditingDeclaration::CanonicalFrame(
            CanonicalFrameInput::try_new("frame").expect("frame"),
        ))
        .expect("declaration");
    RoadEditingSourceWriter::new(&limits)
        .write(builder.finish().expect("module"))
        .expect("LFRE seed")
        .as_bytes()
        .to_vec()
}

fn editable_fixture() -> Fixture {
    let bytes = lfre_module(DOCUMENT_KEY);
    let mut unit = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    unit.add_road_editing_module(
        RoadEditingModuleInput::try_new(DOCUMENT_KEY, &bytes, None).expect("module input"),
    )
    .expect("LFRE admission");
    let output = Compiler::new()
        .compile(unit.build().expect("unit"))
        .expect("compile editable seed");
    let candidate = emit_portable_candidate(
        &output,
        &PortableEmissionProvenance::try_new("security-fuzz").expect("build provenance"),
        portable::limits(),
        PortableDiffBase::Genesis,
    )
    .expect("emit editable seed");
    check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        portable::limits(),
    )
    .expect("editable bundle");
    let revision = revision(candidate.canonical_artifact().bytes());
    let source = CommittedNetworkSource::Editable {
        state: RoadEditingState::try_new(vec![RoadEditingStateModule::new(
            DOCUMENT_KEY.to_owned(),
            bytes,
        )])
        .expect("road editing state"),
        diff_base: EditableDiffBase::try_new(
            candidate.canonical_artifact().bytes().to_vec(),
            *revision.canonical_origin(),
        )
        .expect("editable exact diff base"),
    };
    let world = world(&revision, source.clone());
    finish_fixture(revision, source, world)
}

fn finish_fixture(
    revision: Arc<SharedNetworkRevision>,
    source: CommittedNetworkSource,
    world: TrafficWorld,
) -> Fixture {
    let snapshot = world.capture_snapshot().expect("capture seed");
    let expected = deterministic_state_digest(&snapshot).expect("seed digest");
    let bytes = encode_lfrs(&snapshot);
    drop(world);
    let restored = restore_lfrs(
        &bytes,
        Arc::clone(&revision),
        source.clone(),
        config(),
        execution(),
        restore_limits(),
    )
    .expect("unmutated snapshot must restore")
    .into_world();
    let actual = restored.capture_snapshot().expect("restored seed capture");
    assert_eq!(
        deterministic_state_digest(&actual).expect("restored digest"),
        expected,
        "seed roundtrip"
    );
    Fixture {
        revision,
        source,
        bytes,
    }
}

fn fixtures() -> &'static [Fixture; 2] {
    static FIXTURES: OnceLock<[Fixture; 2]> = OnceLock::new();
    FIXTURES.get_or_init(|| {
        let revision = revision(portable::FULL_SPATIAL);
        let source = published(&revision);
        let mut world = world(&revision, source.clone());
        populate(&mut world, true);
        [finish_fixture(revision, source, world), editable_fixture()]
    })
}

fn restore_limits() -> SnapshotRestoreLimits {
    SnapshotRestoreLimits::new(MAX_INPUT_BYTES as u64, 4_096)
}

/// 任意字节与合法 Published/Editable 快照变异，成功后再次编码、恢复并比较逻辑摘要。
///
/// # Panics
///
/// 合法种子失效、恢复成功后无法捕获或 roundtrip 改变逻辑状态时触发 oracle。
pub fn check_snapshot(data: &[u8]) {
    if data.len() > MAX_INPUT_BYTES {
        return;
    }
    let mode = data.first().copied().unwrap_or(0);
    let index = usize::from((mode >> 1) & 1);
    let fixture = &fixtures()[index];
    let payload = data.get(1..).unwrap_or_default();
    let mutated;
    let bytes = if mode & 1 == 0 {
        payload
    } else {
        mutated = mutate(&fixture.bytes, payload);
        &mutated
    };
    let source = if mode & 4 == 0 {
        fixture.source.clone()
    } else {
        fixtures()[1 - index].source.clone()
    };
    let result = restore_lfrs(
        bytes,
        Arc::clone(&fixture.revision),
        source.clone(),
        config(),
        execution(),
        restore_limits(),
    );
    if let Ok(restored) = result {
        let world = restored.into_world();
        let snapshot = world
            .capture_snapshot()
            .expect("accepted snapshot is capturable");
        let expected = deterministic_state_digest(&snapshot).expect("accepted state digest");
        let encoded = encode_lfrs(&snapshot);
        drop(world);
        let mut roundtrip = restore_lfrs(
            &encoded,
            Arc::clone(&fixture.revision),
            source,
            config(),
            execution(),
            restore_limits(),
        )
        .expect("accepted snapshot roundtrip")
        .into_world();
        assert_eq!(
            deterministic_state_digest(&roundtrip.capture_snapshot().expect("roundtrip capture"))
                .expect("roundtrip digest"),
            expected
        );
        // 合法的时钟边界可能返回步进错误；错误仍须有界、安全地返回。
        black_box(roundtrip.step(TickInput::new(100))).ok();
    }
}

pub(crate) fn seeds() -> Vec<(&'static str, &'static str, Vec<u8>)> {
    let mut published = vec![0];
    published.extend_from_slice(&fixtures()[0].bytes);
    let mut editable = vec![2];
    editable.extend_from_slice(&fixtures()[1].bytes);
    vec![
        ("lfrs_restore", "published", published),
        ("lfrs_restore", "editable", editable),
        ("lfrs_restore", "published-mutation", vec![1]),
        ("lfrs_restore", "editable-mutation", vec![3]),
        ("lfrs_restore", "published-wrong-source", vec![5]),
        ("lfrs_restore", "editable-wrong-source", vec![7]),
        // 首轮正式验收的最小化 crash 单元（run 40933268900，
        // crash-9303b8943eeeb463f8aaf9bd7000b712b776935c）：变异后的 vtable
        // 声明长度过短，曾使恢复路径字段计数下溢；修复后须稳定回归。
        (
            "lfrs_restore",
            "truncated-vtable",
            vec![
                0xdb, 0x0d, 0x34, 0xb5, 0x00, 0x34, 0x38, 0x38, 0x38, 0x38, 0x61, 0x2e, 0x41,
                0x0d, 0x24,
            ],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_vtable_does_not_panic() {
        check_snapshot(&[
            0xdb, 0x0d, 0x34, 0xb5, 0x00, 0x34, 0x38, 0x38, 0x38, 0x38, 0x61, 0x2e, 0x41,
            0x0d, 0x24,
        ]);
    }

    #[test]
    fn source_kinds_and_wire_limit_reject() {
        for index in 0..2 {
            let fixture = &fixtures()[index];
            assert!(restore_lfrs(
                &fixture.bytes,
                Arc::clone(&fixture.revision),
                fixtures()[1 - index].source.clone(),
                config(),
                execution(),
                restore_limits()
            )
            .is_err());
            assert!(restore_lfrs(
                &fixture.bytes,
                Arc::clone(&fixture.revision),
                fixture.source.clone(),
                config(),
                execution(),
                SnapshotRestoreLimits::new(fixture.bytes.len() as u64 - 1, 4_096)
            )
            .is_err());
        }
    }
}
