use std::sync::Arc;

use laneflow_compiler::*;
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_runtime::*;
use laneflow_static_contract::{LaneEdgeOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::*;

pub const LANES: u32 = 256;
pub const PARKING: u32 = 128;

pub fn revision() -> Arc<SharedNetworkRevision> {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "research/pose-681",
            source_document_key: "pose.document",
            generator_build_id: "research-pose-681",
            parameters_and_inputs_digest: [0x68; 32],
            frontend_options_digest: [0x81; 32],
            random_seed: Some(681),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .unwrap();
    let mut module = SyntheticModuleBuilder::new(header, &limits).unwrap();
    module
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "car",
            extends: None,
        })
        .unwrap()
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "car",
            participant_class: ParticipantClassReference::local("car"),
            iidm: IidmVehicleProfileInput {
                length_meters: 4.5,
                desired_speed_meters_per_second: 13.75,
                min_gap_meters: 2.0,
                time_headway_seconds: 1.4,
                max_acceleration_meters_per_second_squared: 1.8,
                comfortable_deceleration_meters_per_second_squared: 2.0,
                emergency_deceleration_meters_per_second_squared: 4.5,
            },
        })
        .unwrap();
    let keys: Vec<_> = (0..LANES).map(|i| format!("edge-{i}")).collect();
    let points: Vec<_> = (0..LANES)
        .map(|i| {
            [
                CanonicalPoint3F32Input {
                    x: 0.0,
                    y: 0.0,
                    z: i as f32 * 10.0,
                },
                CanonicalPoint3F32Input {
                    x: 8_000.0,
                    y: 0.0,
                    z: i as f32 * 10.0,
                },
            ]
        })
        .collect();
    for key in &keys {
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: key,
                length_meters: 8_000.0,
                speed_limit_meters_per_second: 15.0,
                successors: &[],
            })
            .unwrap();
    }
    for i in 0..PARKING {
        module
            .add_parking_space(ParkingSpaceInput {
                parking_space_key: &format!("space-{i}"),
                parking_facility: None,
                entry: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local(&keys[0]),
                    progress_meters: f64::from(i + 1) * 10.0,
                },
                exit: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local(&keys[0]),
                    progress_meters: f64::from(i + 1) * 10.0,
                },
                geometry: ParkingSpaceGeometryInput {
                    lateral_offset_meters: -3.0,
                    heading_offset_radians: 0.2,
                    length_meters: 5.5,
                    width_meters: 2.6,
                },
            })
            .unwrap();
    }
    let geometries: Vec<_> = keys
        .iter()
        .zip(&points)
        .map(|(key, points)| LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local(key),
            centerline_points: points,
        })
        .collect();
    module
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame",
            lane_edge_geometries: &geometries,
        })
        .unwrap();
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().unwrap()).unwrap();
    let compiled = Compiler::new().compile(unit.build().unwrap()).unwrap();
    let artifact = emit_portable_candidate(
        &compiled,
        &PortableEmissionProvenance::try_new("research-pose-681").unwrap(),
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    let checked = check_post_emission_bundle(
        artifact.canonical_artifact().bytes(),
        artifact.source_map().bytes(),
        artifact.semantic_diff().bytes(),
        artifact.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .unwrap();
    build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .unwrap()
}

pub fn world(root: &Arc<SharedNetworkRevision>, count: u32) -> TrafficWorld {
    let origin = root.canonical_origin();
    let source = CommittedNetworkSource::Published {
        reference: PublishedLfcaReference::new(
            "fixture://pose-681",
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .unwrap(),
    };
    let mut world = TrafficWorld::install(
        Arc::clone(root),
        WorldConfig::new(count, LANES, u64::from(LANES), 1_024, 100),
        ExecutionConfig::new(std::num::NonZeroU32::new(1).unwrap()),
        source,
        681,
        WorldPolicySelection::NotRequired,
    )
    .unwrap();
    let routes: Vec<_> = (0..LANES)
        .map(|i| {
            world
                .register_route(RouteRegisterInput::new(vec![LaneEdgeOrdinal::from_raw(i)]))
                .unwrap()
        })
        .collect();
    for i in 0..count {
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                routes[(i % LANES) as usize],
                0,
                (i / LANES + 1) * 10_000,
                0,
            ))
            .unwrap();
    }
    world
}
