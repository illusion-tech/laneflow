//! #441 共享静态路网资源与性能证据（uninstrumented 墙钟与功能断言）。
//!
//! 分配次数走 `shared_network_allocation_evidence`；本文件不加全局分配器。

use std::hint::black_box;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Instant;

use laneflow_compiler::{
    CanonicalFrameInput, CompilationUnitBuilder, CompileLimits, Compiler, PortableDiffBase,
    PortableEmissionProvenanceV1, SourceModuleHeader, SourceModuleHeaderInput,
    SyntheticModuleBuilder, emit_portable_candidate,
};
use laneflow_corridor_generator::{CorridorConfig, generate};
use laneflow_format::{
    FormatLimits, check_canonical_network_input_v1, check_post_emission_bundle_v1,
};
use laneflow_runtime::{PoseSource, TickInput, TrafficWorld, VehicleSpawnInput, WorldConfig};
use laneflow_scenario::signalized_corridor::{
    BoundCorridorCatalog, BoundSpawnSlot, CorridorCatalog, PASSENGER_CAR_PROFILE_KEY, bind,
};
use laneflow_static_contract::{
    AccessRuleKind, AuthoringLaneKind, CanonicalFrameKind, EntityKind, EntityKindMarker,
    FacilityBandKind, JunctionKind, LaneEdgeKind, LaneGroupKind, ManeuverGateKind,
    ManeuverPathKind, MovementKind, Ordinal, OrdinalKind, ParkingAreaKind, ParkingSpaceKind,
    ParticipantClassKind, RoadCorridorKind, RoadSectionKind, Sha256Digest, SignalControllerKind,
    SignalGroupKind, SignalPhaseKind, StaticRouteKind, StaticRouteOrdinal, StopLineKind,
    VehicleProfileKind, VehicleProfileOrdinal, WaitingZoneKind,
};
use laneflow_static_network::{
    BuildError, BuildStructure, SharedIdentityIndex, SharedNetworkBuildLimits,
    SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

const MIN_HEADLESS: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-variants/min-headless.lfca"
);
const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-full-spatial/expected.lfca"
);
const FULL_SPATIAL_LFSM: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-full-spatial/expected.lfsm"
);
const FULL_SPATIAL_LFSD: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-full-spatial/expected.lfsd"
);
const CORRIDOR: &[u8] = include_bytes!("../../../examples/data/v0.2-signalized-corridor.lfca");
const CORRIDOR_CATALOG: &str =
    include_str!("../../../examples/data/v0.2-signalized-corridor.catalog.toml");
const CORRIDOR_CONFIG: &str =
    include_str!("../../../examples/config/v0.10-signalized-corridor.toml");

const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024);
const FULL_SPATIAL_KERNEL_STEPS: u32 = 1_024;
const FULL_SPATIAL_DELTA_MS: u64 = 1;
const CORRIDOR_KERNEL_STEPS: u32 = 1_024;
const CORRIDOR_DELTA_MS: u64 = 16;
const WARMUP: usize = 1;
const SAMPLES: usize = 7;
const CORRIDOR_LFCA_LEN: usize = 458_702;
const CORRIDOR_SHA256: [u8; 32] = [
    0xd0, 0x4d, 0xeb, 0x8c, 0xa2, 0x3d, 0x33, 0x1a, 0x8a, 0x22, 0xa9, 0x10, 0x97, 0xf7, 0x44, 0x13,
    0xe9, 0x72, 0xd4, 0x18, 0xdb, 0xef, 0x7e, 0xc6, 0x0d, 0x76, 0xeb, 0xb8, 0x73, 0x63, 0x01, 0x81,
];
const CORRIDOR_NETWORK_REVISION: [u8; 32] = [
    0x1c, 0x38, 0x91, 0xc7, 0x71, 0xd5, 0x03, 0x2d, 0xba, 0x6d, 0x5b, 0x83, 0x7c, 0x8c, 0xc8, 0xbe,
    0x1c, 0x95, 0x6b, 0xf1, 0x27, 0xcb, 0x32, 0xf0, 0xf5, 0x54, 0xb0, 0xaa, 0xee, 0x72, 0x8f, 0xe9,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SceneLedger {
    lfca_exact: usize,
    artifact_digest: Sha256Digest,
    network_revision: Sha256Digest,
    traffic: u64,
    identity: u64,
    hints: u64,
    spatial: u64,
    root: u64,
    scratch: u64,
    facility_geometry_count: u32,
    lane_pose: bool,
    identity_round_trips: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CoexistenceLedger {
    current_retained: u64,
    base_lfca: usize,
    target_lfca: usize,
    target_lfsm: usize,
    target_lfsd: usize,
    candidate_retained: u64,
    scratch: u64,
    publish: u64,
    editable: u64,
    post_emission: u64,
}

fn build(bytes: &[u8], spatial: SpatialBuildOption) -> Arc<SharedNetworkRevision> {
    let input =
        check_canonical_network_input_v1(bytes, FormatLimits::V1_HARD).expect("checked lfca");
    build_shared_network_revision(input, SharedNetworkBuildOptions::new(spatial, BUILD_LIMITS))
        .expect("shared network revision")
}

fn emit_and_build(
    configure: impl FnOnce(&mut SyntheticModuleBuilder),
    spatial: SpatialBuildOption,
) -> (Vec<u8>, Vec<u8>, Vec<u8>, Arc<SharedNetworkRevision>) {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "city/441-evidence",
            source_document_key: "441-evidence.document",
            generator_build_id: "git:441-evidence",
            parameters_and_inputs_digest: [0x41; 32],
            frontend_options_digest: [0x42; 32],
            random_seed: Some(441),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .expect("source header");
    let mut module = SyntheticModuleBuilder::new(header, &limits).expect("synthetic module");
    configure(&mut module);
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().expect("finished module"))
        .expect("add module");
    let output = Compiler::new()
        .compile(unit.build().expect("unit"))
        .expect("compile");
    let candidate = emit_portable_candidate(
        &output,
        &PortableEmissionProvenanceV1::try_new("laneflow-441-evidence-v1").expect("provenance"),
        FormatLimits::V1_HARD,
        PortableDiffBase::Genesis,
    )
    .expect("emit");
    let lfca = candidate.canonical_artifact().bytes().to_vec();
    let lfsm = candidate.source_map().bytes().to_vec();
    let lfsd = candidate.semantic_diff().bytes().to_vec();
    let checked = check_post_emission_bundle_v1(
        &lfca,
        &lfsm,
        &lfsd,
        candidate.expected_semantic_diff_base(),
        FormatLimits::V1_HARD,
    )
    .expect("post-emission");
    let revision = build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(spatial, BUILD_LIMITS),
    )
    .expect("build");
    (lfca, lfsm, lfsd, revision)
}

fn scratch_required(bytes: &[u8], spatial: SpatialBuildOption) -> u64 {
    let input = check_canonical_network_input_v1(bytes, FormatLimits::V1_HARD).expect("checked");
    match build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(spatial, SharedNetworkBuildLimits::new(u64::MAX, 1)),
    ) {
        Err(BuildError::BudgetExceeded {
            structure: BuildStructure::BuilderScratch,
            required,
            ..
        }) => required,
        Err(other) => panic!("scratch budget should fail, got {other:?}"),
        Ok(_) => panic!("scratch budget should fail, got a root"),
    }
}

fn lookup_kind<K: EntityKindMarker + OrdinalKind>(identity: &SharedIdentityIndex) -> u32 {
    let count = identity.entity_count(K::KIND);
    for raw in 0..count {
        let ordinal = Ordinal::<K>::from_raw(raw);
        let id = identity.stable_id(ordinal).expect("forward identity");
        let back = identity.ordinal(id).expect("reverse identity");
        black_box(back.raw());
    }
    count
}

fn identity_lookups(identity: &SharedIdentityIndex) -> u32 {
    lookup_kind::<RoadCorridorKind>(identity)
        + lookup_kind::<RoadSectionKind>(identity)
        + lookup_kind::<AuthoringLaneKind>(identity)
        + lookup_kind::<LaneEdgeKind>(identity)
        + lookup_kind::<JunctionKind>(identity)
        + lookup_kind::<MovementKind>(identity)
        + lookup_kind::<ManeuverPathKind>(identity)
        + lookup_kind::<ManeuverGateKind>(identity)
        + lookup_kind::<WaitingZoneKind>(identity)
        + lookup_kind::<StopLineKind>(identity)
        + lookup_kind::<SignalGroupKind>(identity)
        + lookup_kind::<SignalControllerKind>(identity)
        + lookup_kind::<SignalPhaseKind>(identity)
        + lookup_kind::<ParkingAreaKind>(identity)
        + lookup_kind::<ParkingSpaceKind>(identity)
        + lookup_kind::<LaneGroupKind>(identity)
        + lookup_kind::<FacilityBandKind>(identity)
        + lookup_kind::<ParticipantClassKind>(identity)
        + lookup_kind::<AccessRuleKind>(identity)
        + lookup_kind::<VehicleProfileKind>(identity)
        + lookup_kind::<StaticRouteKind>(identity)
        + lookup_kind::<CanonicalFrameKind>(identity)
}

fn assert_kind_round_trip<K: EntityKindMarker + OrdinalKind>(identity: &SharedIdentityIndex) {
    let count = identity.entity_count(K::KIND);
    for raw in 0..count {
        let ordinal = Ordinal::<K>::from_raw(raw);
        let id = identity.stable_id(ordinal).expect("forward identity");
        assert_eq!(identity.ordinal(id).map(Ordinal::raw), Some(raw));
    }
}

fn identity_round_trips(identity: &SharedIdentityIndex) -> u32 {
    assert_kind_round_trip::<RoadCorridorKind>(identity);
    assert_kind_round_trip::<RoadSectionKind>(identity);
    assert_kind_round_trip::<AuthoringLaneKind>(identity);
    assert_kind_round_trip::<LaneEdgeKind>(identity);
    assert_kind_round_trip::<JunctionKind>(identity);
    assert_kind_round_trip::<MovementKind>(identity);
    assert_kind_round_trip::<ManeuverPathKind>(identity);
    assert_kind_round_trip::<ManeuverGateKind>(identity);
    assert_kind_round_trip::<WaitingZoneKind>(identity);
    assert_kind_round_trip::<StopLineKind>(identity);
    assert_kind_round_trip::<SignalGroupKind>(identity);
    assert_kind_round_trip::<SignalControllerKind>(identity);
    assert_kind_round_trip::<SignalPhaseKind>(identity);
    assert_kind_round_trip::<ParkingAreaKind>(identity);
    assert_kind_round_trip::<ParkingSpaceKind>(identity);
    assert_kind_round_trip::<LaneGroupKind>(identity);
    assert_kind_round_trip::<FacilityBandKind>(identity);
    assert_kind_round_trip::<ParticipantClassKind>(identity);
    assert_kind_round_trip::<AccessRuleKind>(identity);
    assert_kind_round_trip::<VehicleProfileKind>(identity);
    assert_kind_round_trip::<StaticRouteKind>(identity);
    assert_kind_round_trip::<CanonicalFrameKind>(identity);
    identity_lookups(identity)
}

fn scene_ledger(lfca: &[u8], spatial: SpatialBuildOption) -> SceneLedger {
    let revision = build(lfca, spatial);
    let origin = revision.canonical_origin();
    assert_eq!(
        origin.canonical_artifact_byte_length().get(),
        u64::try_from(lfca.len()).expect("lfca length")
    );
    SceneLedger {
        lfca_exact: lfca.len(),
        artifact_digest: origin.canonical_artifact_digest(),
        network_revision: origin.network_revision().into_digest(),
        traffic: revision.traffic().retained_logical_bytes(),
        identity: revision.identity().retained_logical_bytes(),
        hints: revision.planning_hints().retained_logical_bytes(),
        spatial: revision.spatial().map_or(
            0,
            laneflow_static_network::SharedSpatialNetwork::retained_logical_bytes,
        ),
        root: revision.retained_logical_bytes(),
        scratch: scratch_required(lfca, spatial),
        facility_geometry_count: revision
            .spatial()
            .map_or(0, |spatial| spatial.facility_geometry_count()),
        lane_pose: revision
            .spatial()
            .and_then(laneflow_static_network::SharedSpatialNetwork::lane_pose)
            .is_some(),
        identity_round_trips: identity_round_trips(revision.identity()),
    }
}

fn optional_exact(bytes: Option<&[u8]>) -> String {
    bytes.map_or_else(
        || "not-checked-in".to_owned(),
        |bytes| bytes.len().to_string(),
    )
}

fn assert_stable_ledger(
    scene: &str,
    lfca: &[u8],
    lfsm: Option<&[u8]>,
    lfsd: Option<&[u8]>,
    spatial: SpatialBuildOption,
) -> SceneLedger {
    let first = scene_ledger(lfca, spatial);
    let second = scene_ledger(lfca, spatial);
    assert_eq!(first, second, "{scene} ledger must be deterministic");
    println!(
        "shared-static-network-evidence ledger scene={scene} spatial={spatial:?} lfca_exact={} artifact_digest={:x} network_revision={:x} lfsm_exact={} lfsd_exact={} traffic_retained={} identity_retained={} hints_retained={} spatial_retained={} root_retained={} scratch_required={} facility_geometry_count={} lane_pose={} identity_round_trips={}",
        first.lfca_exact,
        first.artifact_digest,
        first.network_revision,
        optional_exact(lfsm),
        optional_exact(lfsd),
        first.traffic,
        first.identity,
        first.hints,
        first.spatial,
        first.root,
        first.scratch,
        first.facility_geometry_count,
        first.lane_pose,
        first.identity_round_trips,
    );
    first
}

fn spawn_full_spatial_pair(world: &mut TrafficWorld) {
    let route = world
        .static_route(StaticRouteOrdinal::from_raw(0))
        .expect("static route 0");
    let profile = world
        .traffic()
        .relations()
        .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
        .expect("profile 0");
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            1.0 + profile.length() + profile.min_gap() + 2.0,
            0.0,
        ))
        .expect("leader");
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            1.0,
            0.0,
        ))
        .expect("follower");
}

fn spawn_on_slot(world: &mut TrafficWorld, profile: VehicleProfileOrdinal, slot: &BoundSpawnSlot) {
    let edges = world
        .traffic()
        .relations()
        .static_route_edges(slot.entry_route)
        .expect("route edges");
    let index = edges
        .iter()
        .position(|edge| *edge == slot.edge)
        .expect("slot edge is on its entry route");
    let route = world.static_route(slot.entry_route).expect("static route");
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            profile,
            route,
            u32::try_from(index).expect("edge index"),
            slot.progress,
            0.0,
        ))
        .expect("catalog slot must spawn");
}

fn follow_pair<'a>(
    catalog: &CorridorCatalog,
    bound: &'a BoundCorridorCatalog,
) -> (&'a BoundSpawnSlot, &'a BoundSpawnSlot) {
    let lane = catalog
        .portals
        .first()
        .and_then(|portal| portal.lanes.first())
        .expect("portal lane");
    let follower = bound
        .spawn_slots
        .iter()
        .find(|slot| slot.slot_id == lane.entry_spawn_slot_id)
        .expect("entry spawn slot");
    let leader = bound
        .spawn_slots
        .iter()
        .find(|slot| {
            slot.portal_id == follower.portal_id
                && slot.lane_index == follower.lane_index
                && slot.edge == follower.edge
                && slot.progress > follower.progress
        })
        .expect("leader spawn slot");
    (follower, leader)
}

fn spawn_corridor_pair(world: &mut TrafficWorld, revision: &SharedNetworkRevision) {
    let catalog: CorridorCatalog = toml::from_str(CORRIDOR_CATALOG).expect("catalog TOML");
    let bound = bind(&catalog, revision).expect("prepare bind");
    assert_eq!(bound.network_revision, revision.network_revision());
    let profile = *bound
        .profiles
        .get(PASSENGER_CAR_PROFILE_KEY)
        .expect("passenger-car profile");
    let (follower, leader) = follow_pair(&catalog, &bound);
    spawn_on_slot(world, profile, leader);
    spawn_on_slot(world, profile, follower);
}

fn assert_two_lane_poses(world: &TrafficWorld) {
    let poses = world.committed_pose_sources();
    assert_eq!(poses.as_slice().len(), 2);
    assert!(
        poses
            .as_slice()
            .iter()
            .all(|(_, source)| matches!(source, PoseSource::Lane { .. }))
    );
}

fn install_kernel_world(
    revision: Arc<SharedNetworkRevision>,
    delta_ms: u64,
    corridor: bool,
) -> TrafficWorld {
    let mut world =
        TrafficWorld::install(Arc::clone(&revision), WorldConfig::new(8, 8, 1, delta_ms))
            .expect("install");
    if corridor {
        spawn_corridor_pair(&mut world, revision.as_ref());
    } else {
        spawn_full_spatial_pair(&mut world);
    }
    assert_two_lane_poses(&world);
    world
}

fn step_kernel(world: &mut TrafficWorld, delta_ms: u64, steps: u32) {
    let input = TickInput::new(delta_ms);
    for _ in 0..steps {
        world.step(input).expect("step");
    }
}

fn step_while_on_lane(world: &mut TrafficWorld, delta_ms: u64, steps: u32) {
    let input = TickInput::new(delta_ms);
    for _ in 0..steps {
        world.step(input).expect("step");
        assert_two_lane_poses(world);
    }
}

fn summarize_ns(samples: &mut [u128]) -> (u128, u128, u128) {
    samples.sort_unstable();
    let min = samples[0];
    let max = samples[samples.len() - 1];
    let median = samples[samples.len() / 2];
    (min, median, max)
}

fn measure_ns(warmup: usize, samples: usize, mut op: impl FnMut()) -> (u128, u128, u128) {
    for _ in 0..warmup {
        op();
    }
    let mut values = vec![0_u128; samples];
    for slot in &mut values {
        let started = Instant::now();
        op();
        *slot = started.elapsed().as_nanos();
    }
    summarize_ns(&mut values)
}

fn coexistence_ledger(
    current: &SharedNetworkRevision,
    base: &[u8],
    target_lfca: &[u8],
    target_lfsm: &[u8],
    target_lfsd: &[u8],
    candidate: &SharedNetworkRevision,
    scratch: u64,
) -> CoexistenceLedger {
    let current_retained = current.retained_logical_bytes();
    let candidate_retained = candidate.retained_logical_bytes();
    let publish = u64::try_from(target_lfca.len()).expect("lfca") + candidate_retained + scratch;
    let editable = current_retained
        + u64::try_from(base.len()).expect("base")
        + u64::try_from(target_lfca.len()).expect("target lfca")
        + u64::try_from(target_lfsm.len()).expect("lfsm")
        + u64::try_from(target_lfsd.len()).expect("lfsd")
        + candidate_retained
        + scratch;
    let post_emission =
        u64::try_from(target_lfca.len() + target_lfsm.len() + target_lfsd.len()).expect("bundle");
    CoexistenceLedger {
        current_retained,
        base_lfca: base.len(),
        target_lfca: target_lfca.len(),
        target_lfsm: target_lfsm.len(),
        target_lfsd: target_lfsd.len(),
        candidate_retained,
        scratch,
        publish,
        editable,
        post_emission,
    }
}

fn hold_coexistence(
    scene: &str,
    current_lfca: &[u8],
    target_lfca: &[u8],
    target_lfsm: &[u8],
    target_lfsd: &[u8],
    spatial: SpatialBuildOption,
) {
    let base = current_lfca.to_vec();
    let target_lfca = target_lfca.to_vec();
    let target_lfsm = target_lfsm.to_vec();
    let target_lfsd = target_lfsd.to_vec();
    assert!(!target_lfsm.is_empty());
    assert!(!target_lfsd.is_empty());
    let scratch = scratch_required(&target_lfca, spatial);
    assert_eq!(
        scratch,
        scratch_required(&target_lfca, spatial),
        "{scene} scratch_required must be deterministic"
    );
    let first_current = build(&base, spatial);
    let first_candidate = build(&target_lfca, spatial);
    let first = coexistence_ledger(
        first_current.as_ref(),
        &base,
        &target_lfca,
        &target_lfsm,
        &target_lfsd,
        first_candidate.as_ref(),
        scratch,
    );
    let second_current = build(&base, spatial);
    let second_candidate = build(&target_lfca, spatial);
    let second = coexistence_ledger(
        second_current.as_ref(),
        &base,
        &target_lfca,
        &target_lfsm,
        &target_lfsd,
        second_candidate.as_ref(),
        scratch,
    );
    assert_eq!(
        first, second,
        "{scene} coexistence terms must be deterministic"
    );
    assert!(first.current_retained > 0);
    assert!(first.candidate_retained > 0);
    assert!(first.editable > first.publish);
    black_box((
        &first_current,
        &base,
        &target_lfca,
        &target_lfsm,
        &target_lfsd,
        &first_candidate,
    ));
    println!(
        "shared-static-network-evidence coexistence scene={scene} current_retained={} base_lfca={} target_lfca={} target_lfsm={} target_lfsd={} candidate_retained={} scratch={} publish_terms={} editable_terms={} post_emission={} same_root={}",
        first.current_retained,
        first.base_lfca,
        first.target_lfca,
        first.target_lfsm,
        first.target_lfsd,
        first.candidate_retained,
        first.scratch,
        first.publish,
        first.editable,
        first.post_emission,
        Arc::ptr_eq(&first_current, &first_candidate)
    );
}

#[test]
fn frozen_fixtures_match_g1_and_print_ledgers() {
    assert_eq!(CORRIDOR.len(), CORRIDOR_LFCA_LEN);
    assert_stable_ledger(
        "min-headless",
        MIN_HEADLESS,
        None,
        None,
        SpatialBuildOption::RetainAvailable,
    );
    assert_stable_ledger(
        "full-spatial-omit",
        FULL_SPATIAL,
        Some(FULL_SPATIAL_LFSM),
        Some(FULL_SPATIAL_LFSD),
        SpatialBuildOption::Omit,
    );
    assert_stable_ledger(
        "full-lane-spatial",
        FULL_SPATIAL,
        Some(FULL_SPATIAL_LFSM),
        Some(FULL_SPATIAL_LFSD),
        SpatialBuildOption::RetainAvailable,
    );
    assert_stable_ledger(
        "corridor",
        CORRIDOR,
        None,
        None,
        SpatialBuildOption::RetainAvailable,
    );

    let corridor = build(CORRIDOR, SpatialBuildOption::RetainAvailable);
    let origin = corridor.canonical_origin();
    assert_eq!(
        origin.canonical_artifact_byte_length().get(),
        u64::try_from(CORRIDOR_LFCA_LEN).expect("len")
    );
    assert_eq!(
        origin.canonical_artifact_digest().as_bytes(),
        &CORRIDOR_SHA256
    );
    assert_eq!(
        origin.network_revision().as_digest().as_bytes(),
        &CORRIDOR_NETWORK_REVISION
    );
    assert_eq!(
        origin.canonical_artifact_digest(),
        Sha256Digest::from_bytes(CORRIDOR_SHA256)
    );
    assert_eq!(corridor.traffic().lane_edge_count(), 66);
    assert!(
        corridor
            .spatial()
            .and_then(|spatial| spatial.lane_pose())
            .is_some()
    );
}

#[test]
fn frame_only_spatial_variant_records_facility_geometry_as_uncovered() {
    let (frame_lfca, frame_lfsm, frame_lfsd, frame) = emit_and_build(
        |module| {
            module
                .add_canonical_frame(CanonicalFrameInput {
                    canonical_frame_key: "frame-main",
                    lane_edge_geometries: &[],
                })
                .expect("frame");
        },
        SpatialBuildOption::RetainAvailable,
    );
    let spatial = frame.spatial().expect("frame-only spatial");
    assert!(spatial.lane_pose().is_none());
    assert_eq!(spatial.facility_geometry_count(), 0);
    assert_stable_ledger(
        "profile-frame-only",
        &frame_lfca,
        Some(&frame_lfsm),
        Some(&frame_lfsd),
        SpatialBuildOption::RetainAvailable,
    );

    let full = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let corridor = build(CORRIDOR, SpatialBuildOption::RetainAvailable);
    println!(
        "shared-static-network-evidence uncovered facility-only-lfca facility_geometry_count full_spatial={} corridor={}",
        full.spatial()
            .expect("full spatial")
            .facility_geometry_count(),
        corridor
            .spatial()
            .expect("corridor spatial")
            .facility_geometry_count(),
    );
}

#[test]
fn publish_and_editable_coexistence_terms_are_held() {
    hold_coexistence(
        "full-spatial",
        FULL_SPATIAL,
        FULL_SPATIAL,
        FULL_SPATIAL_LFSM,
        FULL_SPATIAL_LFSD,
        SpatialBuildOption::RetainAvailable,
    );

    let config = CorridorConfig::parse(CORRIDOR_CONFIG).expect("corridor config");
    let generated = generate(&config).expect("generate corridor");
    hold_coexistence(
        "corridor",
        CORRIDOR,
        generated.lfca_bytes(),
        generated.lfsm_bytes(),
        generated.lfsd_bytes(),
        SpatialBuildOption::RetainAvailable,
    );
}

#[test]
fn failure_and_cancel_do_not_return_a_root() {
    let cancelled = AtomicBool::new(true);
    let input =
        check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD).expect("checked");
    let err = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS)
            .with_cancellation(&cancelled),
    );
    assert!(matches!(err, Err(BuildError::Cancelled)));

    let input =
        check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD).expect("checked");
    let err = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(1, u64::MAX),
        ),
    );
    assert!(matches!(
        err,
        Err(BuildError::BudgetExceeded {
            structure: BuildStructure::RetainedOutput,
            ..
        })
    ));
}

#[test]
fn worlds_2_8_32_share_one_static_root() {
    let revision = build(CORRIDOR, SpatialBuildOption::RetainAvailable);
    let static_retained = revision.retained_logical_bytes();
    let signal_groups = revision
        .traffic()
        .entity_counts()
        .count(EntityKind::SignalGroup);
    let parking_spaces = revision
        .traffic()
        .entity_counts()
        .count(EntityKind::ParkingSpace);
    for count in [2_u32, 8, 32] {
        let worlds: Vec<_> = (0..count)
            .map(|_| {
                TrafficWorld::install(
                    Arc::clone(&revision),
                    WorldConfig::new(8, 8, 1, CORRIDOR_DELTA_MS),
                )
                .expect("install")
            })
            .collect();
        assert!(
            worlds
                .iter()
                .all(|world| Arc::ptr_eq(&world.revision(), &revision))
        );
        println!(
            "shared-static-network-evidence worlds count={count} ptr_eq=true static_retained={static_retained} signal_groups={signal_groups} parking_spaces={parking_spaces} vehicle_capacity=8 per_world_live_bytes=allocation-binary"
        );
    }
}

#[test]
fn production_kernel_keeps_two_lane_poses() {
    let full = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let mut full_world = install_kernel_world(full, FULL_SPATIAL_DELTA_MS, false);
    step_while_on_lane(
        &mut full_world,
        FULL_SPATIAL_DELTA_MS,
        FULL_SPATIAL_KERNEL_STEPS,
    );

    let corridor = build(CORRIDOR, SpatialBuildOption::RetainAvailable);
    let mut corridor_world = install_kernel_world(corridor, CORRIDOR_DELTA_MS, true);
    step_while_on_lane(
        &mut corridor_world,
        CORRIDOR_DELTA_MS,
        CORRIDOR_KERNEL_STEPS,
    );
}

#[test]
#[ignore = "manual release wall-clock evidence; CI 不当墙钟基线"]
fn wall_clock_build_identity_and_kernel() {
    let empty = measure_ns(WARMUP, SAMPLES, || {
        black_box(Instant::now());
    });
    println!(
        "shared-static-network-evidence calibrate instant_noop_ns min={} median={} max={}",
        empty.0, empty.1, empty.2
    );

    for (scene, bytes, spatial) in [
        (
            "min-headless",
            MIN_HEADLESS,
            SpatialBuildOption::RetainAvailable,
        ),
        (
            "full-lane-spatial",
            FULL_SPATIAL,
            SpatialBuildOption::RetainAvailable,
        ),
        ("corridor", CORRIDOR, SpatialBuildOption::RetainAvailable),
    ] {
        let build_ns = measure_ns(WARMUP, SAMPLES, || {
            black_box(build(bytes, spatial));
        });
        println!(
            "shared-static-network-evidence wallclock kind=build scene={scene} min_ns={} median_ns={} max_ns={}",
            build_ns.0, build_ns.1, build_ns.2
        );
        let revision = build(bytes, spatial);
        let expected = identity_round_trips(revision.identity());
        assert_eq!(identity_lookups(revision.identity()), expected);
        let identity_ns = measure_ns(WARMUP, SAMPLES, || {
            black_box(identity_lookups(revision.identity()));
        });
        println!(
            "shared-static-network-evidence wallclock kind=identity scene={scene} lookups={expected} min_ns={} median_ns={} max_ns={}",
            identity_ns.0, identity_ns.1, identity_ns.2
        );
    }

    let full = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let mut full_worlds: Vec<_> = (0..WARMUP + SAMPLES)
        .map(|_| install_kernel_world(Arc::clone(&full), FULL_SPATIAL_DELTA_MS, false))
        .collect();
    let mut full_iter = full_worlds.iter_mut();
    for _ in 0..WARMUP {
        let world = full_iter.next().expect("warmup world");
        step_kernel(world, FULL_SPATIAL_DELTA_MS, FULL_SPATIAL_KERNEL_STEPS);
        assert_two_lane_poses(world);
    }
    let mut full_samples = vec![0_u128; SAMPLES];
    for slot in &mut full_samples {
        let world = full_iter.next().expect("sample world");
        let started = Instant::now();
        step_kernel(world, FULL_SPATIAL_DELTA_MS, FULL_SPATIAL_KERNEL_STEPS);
        *slot = started.elapsed().as_nanos();
        assert_two_lane_poses(world);
    }
    let full_kernel = summarize_ns(&mut full_samples);
    println!(
        "shared-static-network-evidence wallclock kind=kernel scene=full-lane-spatial steps={FULL_SPATIAL_KERNEL_STEPS} min_ns={} median_ns={} max_ns={} below_timer_floor={}",
        full_kernel.0,
        full_kernel.1,
        full_kernel.2,
        full_kernel.1 <= empty.1.saturating_mul(8)
    );

    let corridor = build(CORRIDOR, SpatialBuildOption::RetainAvailable);
    let mut corridor_worlds: Vec<_> = (0..WARMUP + SAMPLES)
        .map(|_| install_kernel_world(Arc::clone(&corridor), CORRIDOR_DELTA_MS, true))
        .collect();
    let mut corridor_iter = corridor_worlds.iter_mut();
    for _ in 0..WARMUP {
        let world = corridor_iter.next().expect("warmup world");
        step_kernel(world, CORRIDOR_DELTA_MS, CORRIDOR_KERNEL_STEPS);
        assert_two_lane_poses(world);
    }
    let mut corridor_samples = vec![0_u128; SAMPLES];
    for slot in &mut corridor_samples {
        let world = corridor_iter.next().expect("sample world");
        let started = Instant::now();
        step_kernel(world, CORRIDOR_DELTA_MS, CORRIDOR_KERNEL_STEPS);
        *slot = started.elapsed().as_nanos();
        assert_two_lane_poses(world);
    }
    let corridor_kernel = summarize_ns(&mut corridor_samples);
    println!(
        "shared-static-network-evidence wallclock kind=kernel scene=corridor steps={CORRIDOR_KERNEL_STEPS} min_ns={} median_ns={} max_ns={} below_timer_floor={}",
        corridor_kernel.0,
        corridor_kernel.1,
        corridor_kernel.2,
        corridor_kernel.1 <= empty.1.saturating_mul(8)
    );
}
