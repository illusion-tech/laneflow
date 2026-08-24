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
use laneflow_format::{
    FormatLimits, check_canonical_network_input_v1, check_post_emission_bundle_v1,
};
use laneflow_runtime::{TickInput, TrafficWorld, VehicleSpawnInput, WorldConfig};
use laneflow_static_contract::{
    AccessRuleKind, AuthoringLaneKind, CanonicalFrameKind, EntityKindMarker, FacilityBandKind,
    JunctionKind, LaneEdgeKind, LaneGroupKind, ManeuverGateKind, ManeuverPathKind, MovementKind,
    Ordinal, OrdinalKind, ParkingAreaKind, ParkingSpaceKind, ParticipantClassKind,
    RoadCorridorKind, RoadSectionKind, SignalControllerKind, SignalGroupKind, SignalPhaseKind,
    StaticRouteKind, StopLineKind, VehicleProfileKind, VehicleProfileOrdinal, WaitingZoneKind,
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

const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024);
const KERNEL_STEPS: u32 = 1_024;
const WARMUP: usize = 1;
const SAMPLES: usize = 7;
const CORRIDOR_LFCA_LEN: usize = 458_702;

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

fn round_trip<K: EntityKindMarker + OrdinalKind>(identity: &SharedIdentityIndex) -> u32 {
    let count = identity.entity_count(K::KIND);
    for raw in 0..count {
        let ordinal = Ordinal::<K>::from_raw(raw);
        let id = identity.stable_id(ordinal).expect("forward identity");
        assert_eq!(identity.ordinal(id).map(Ordinal::raw), Some(raw));
    }
    count
}

fn identity_round_trips(identity: &SharedIdentityIndex) -> u32 {
    round_trip::<RoadCorridorKind>(identity)
        + round_trip::<RoadSectionKind>(identity)
        + round_trip::<AuthoringLaneKind>(identity)
        + round_trip::<LaneEdgeKind>(identity)
        + round_trip::<JunctionKind>(identity)
        + round_trip::<MovementKind>(identity)
        + round_trip::<ManeuverPathKind>(identity)
        + round_trip::<ManeuverGateKind>(identity)
        + round_trip::<WaitingZoneKind>(identity)
        + round_trip::<StopLineKind>(identity)
        + round_trip::<SignalGroupKind>(identity)
        + round_trip::<SignalControllerKind>(identity)
        + round_trip::<SignalPhaseKind>(identity)
        + round_trip::<ParkingAreaKind>(identity)
        + round_trip::<ParkingSpaceKind>(identity)
        + round_trip::<LaneGroupKind>(identity)
        + round_trip::<FacilityBandKind>(identity)
        + round_trip::<ParticipantClassKind>(identity)
        + round_trip::<AccessRuleKind>(identity)
        + round_trip::<VehicleProfileKind>(identity)
        + round_trip::<StaticRouteKind>(identity)
        + round_trip::<CanonicalFrameKind>(identity)
}

fn print_ledger(
    scene: &str,
    lfca: &[u8],
    lfsm: Option<&[u8]>,
    lfsd: Option<&[u8]>,
    spatial: SpatialBuildOption,
) {
    let revision = build(lfca, spatial);
    let spatial_bytes = revision.spatial().map_or(
        0,
        laneflow_static_network::SharedSpatialNetwork::retained_logical_bytes,
    );
    let facility = revision
        .spatial()
        .map_or(0, |spatial| spatial.facility_geometry_count());
    let lane_pose = revision
        .spatial()
        .and_then(laneflow_static_network::SharedSpatialNetwork::lane_pose)
        .is_some();
    println!(
        "shared-static-network-evidence ledger scene={scene} spatial={spatial:?} lfca_exact={} lfsm_exact={} lfsd_exact={} traffic_retained={} identity_retained={} hints_retained={} spatial_retained={} root_retained={} scratch_required={} facility_geometry_count={} lane_pose={lane_pose} identity_round_trips={}",
        lfca.len(),
        lfsm.map_or(0, <[u8]>::len),
        lfsd.map_or(0, <[u8]>::len),
        revision.traffic().retained_logical_bytes(),
        revision.identity().retained_logical_bytes(),
        revision.planning_hints().retained_logical_bytes(),
        spatial_bytes,
        revision.retained_logical_bytes(),
        scratch_required(lfca, spatial),
        facility,
        identity_round_trips(revision.identity()),
    );
}

fn spawn_two(world: &mut TrafficWorld) {
    let route = world
        .static_route(laneflow_static_contract::StaticRouteOrdinal::from_raw(0))
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

fn step_kernel(revision: Arc<SharedNetworkRevision>, delta_ms: u64) {
    let mut world =
        TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, delta_ms)).expect("install");
    spawn_two(&mut world);
    let input = TickInput::new(delta_ms);
    for _ in 0..KERNEL_STEPS {
        world.step(input).expect("step");
    }
    black_box(world.tick_index());
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

#[test]
fn frozen_fixtures_match_g1_and_print_ledgers() {
    assert_eq!(CORRIDOR.len(), CORRIDOR_LFCA_LEN);
    print_ledger(
        "min-headless",
        MIN_HEADLESS,
        None,
        None,
        SpatialBuildOption::RetainAvailable,
    );
    print_ledger(
        "full-spatial-omit",
        FULL_SPATIAL,
        Some(FULL_SPATIAL_LFSM),
        Some(FULL_SPATIAL_LFSD),
        SpatialBuildOption::Omit,
    );
    print_ledger(
        "full-lane-spatial",
        FULL_SPATIAL,
        Some(FULL_SPATIAL_LFSM),
        Some(FULL_SPATIAL_LFSD),
        SpatialBuildOption::RetainAvailable,
    );
    print_ledger(
        "corridor",
        CORRIDOR,
        None,
        None,
        SpatialBuildOption::RetainAvailable,
    );

    let corridor = build(CORRIDOR, SpatialBuildOption::RetainAvailable);
    assert_eq!(
        corridor
            .canonical_origin()
            .canonical_artifact_byte_length()
            .get(),
        u64::try_from(CORRIDOR_LFCA_LEN).expect("len")
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
fn frame_only_and_facility_only_spatial_variants() {
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
    print_ledger(
        "profile-frame-only",
        &frame_lfca,
        Some(&frame_lfsm),
        Some(&frame_lfsd),
        SpatialBuildOption::RetainAvailable,
    );

    let full = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let corridor = build(CORRIDOR, SpatialBuildOption::RetainAvailable);
    let full_facilities = full
        .spatial()
        .expect("full spatial")
        .facility_geometry_count();
    let corridor_facilities = corridor
        .spatial()
        .expect("corridor spatial")
        .facility_geometry_count();
    assert!(full_facilities > 0);
    println!(
        "shared-static-network-evidence facility-only-note independent_lfca=none facility_geometry_count full_spatial={full_facilities} corridor={corridor_facilities}"
    );
}

#[test]
fn publish_and_editable_coexistence_peaks_are_bounded() {
    let current = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let candidate = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let scratch = scratch_required(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let publish = u64::try_from(FULL_SPATIAL.len()).expect("lfca")
        + candidate.retained_logical_bytes()
        + scratch;
    let editable = current.retained_logical_bytes()
        + u64::try_from(FULL_SPATIAL.len()).expect("base")
        + u64::try_from(FULL_SPATIAL.len()).expect("target lfca")
        + u64::try_from(FULL_SPATIAL_LFSM.len()).expect("lfsm")
        + u64::try_from(FULL_SPATIAL_LFSD.len()).expect("lfsd")
        + candidate.retained_logical_bytes()
        + scratch;
    let post_emission =
        u64::try_from(FULL_SPATIAL.len() + FULL_SPATIAL_LFSM.len() + FULL_SPATIAL_LFSD.len())
            .expect("bundle");
    assert!(publish > 0);
    assert!(editable > publish);
    println!(
        "shared-static-network-evidence coexistence scene=full-spatial publish_peak={publish} editable_peak={editable} post_emission_peak={post_emission} scratch={scratch} current_ptr_eq={}",
        Arc::ptr_eq(&current, &candidate)
    );

    let current_corridor = build(CORRIDOR, SpatialBuildOption::RetainAvailable);
    let candidate_corridor = build(CORRIDOR, SpatialBuildOption::RetainAvailable);
    let corridor_scratch = scratch_required(CORRIDOR, SpatialBuildOption::RetainAvailable);
    let corridor_publish = u64::try_from(CORRIDOR.len()).expect("lfca")
        + candidate_corridor.retained_logical_bytes()
        + corridor_scratch;
    let corridor_editable = current_corridor.retained_logical_bytes()
        + u64::try_from(CORRIDOR.len()).expect("base")
        + u64::try_from(CORRIDOR.len()).expect("target")
        + candidate_corridor.retained_logical_bytes()
        + corridor_scratch;
    println!(
        "shared-static-network-evidence coexistence scene=corridor publish_peak={corridor_publish} editable_peak={corridor_editable} lfsm_lfsd=not-checked-in scratch={corridor_scratch}"
    );
    black_box((current, candidate, current_corridor, candidate_corridor));
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
    for count in [2_u32, 8, 32] {
        let worlds: Vec<_> = (0..count)
            .map(|_| {
                TrafficWorld::install(Arc::clone(&revision), WorldConfig::new(8, 4, 1, 16))
                    .expect("install")
            })
            .collect();
        assert!(
            worlds
                .iter()
                .all(|world| Arc::ptr_eq(&world.revision(), &revision))
        );
        println!(
            "shared-static-network-evidence worlds count={count} static_retained={} per_world_control=Arc+WorldConfig+tables",
            revision.retained_logical_bytes()
        );
    }
}

#[test]
fn production_kernel_steps_full_spatial_and_corridor() {
    step_kernel(
        build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable),
        100,
    );
    step_kernel(build(CORRIDOR, SpatialBuildOption::RetainAvailable), 16);
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

    for (scene, bytes, spatial, delta) in [
        (
            "min-headless",
            MIN_HEADLESS,
            SpatialBuildOption::RetainAvailable,
            None,
        ),
        (
            "full-lane-spatial",
            FULL_SPATIAL,
            SpatialBuildOption::RetainAvailable,
            Some(100_u64),
        ),
        (
            "corridor",
            CORRIDOR,
            SpatialBuildOption::RetainAvailable,
            Some(16_u64),
        ),
    ] {
        let build_ns = measure_ns(WARMUP, SAMPLES, || {
            black_box(build(bytes, spatial));
        });
        println!(
            "shared-static-network-evidence wallclock kind=build scene={scene} min_ns={} median_ns={} max_ns={}",
            build_ns.0, build_ns.1, build_ns.2
        );
        let revision = build(bytes, spatial);
        let identity_ns = measure_ns(WARMUP, SAMPLES, || {
            black_box(identity_round_trips(revision.identity()));
        });
        println!(
            "shared-static-network-evidence wallclock kind=identity scene={scene} min_ns={} median_ns={} max_ns={}",
            identity_ns.0, identity_ns.1, identity_ns.2
        );
        if let Some(delta) = delta {
            let kernel_ns = measure_ns(WARMUP, SAMPLES, || {
                step_kernel(Arc::clone(&revision), delta);
            });
            let below_floor = kernel_ns.1 <= empty.1.saturating_mul(8);
            println!(
                "shared-static-network-evidence wallclock kind=kernel scene={scene} min_ns={} median_ns={} max_ns={} below_timer_floor={below_floor}",
                kernel_ns.0, kernel_ns.1, kernel_ns.2
            );
        }
    }
}
