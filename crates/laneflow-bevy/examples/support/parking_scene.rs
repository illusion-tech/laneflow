//! 启动时在进程内编译环路与停车设施，并装配已绑定路线的 Session。
//! 不写文件，不引入随机量；任一步失败都不发布部分场景。
use std::{error::Error, fmt::Debug, num::NonZeroU32, sync::Arc};

use bevy_math::Vec3;
use laneflow_bevy::LaneFlowSession;
use laneflow_compiler::{
    CanonicalFrameInput, CanonicalPoint3F32Input, CompilationOutput, CompilationUnitBuilder,
    CompileLimits, Compiler, IidmVehicleProfileInput, LaneEdgeGeometryInput, LaneEdgeInput,
    LaneEdgeReference, ParkingFacilityInput, ParkingFacilityReference, ParkingLaneAnchorInput,
    ParkingSpaceGeometryInput, ParkingSpaceInput, ParticipantClassInput, ParticipantClassReference,
    PortableDiffBase, PortableEmissionProvenance, SourceModuleHeader, SourceModuleHeaderInput,
    SyntheticModuleBuilder, VehicleProfileInput, derive_canonical_stable_id_v1,
    emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_runtime::{
    RouteHandle, RouteRegisterInput, VehicleHandle, VehicleSpawnInput, WorldConfig,
    WorldPolicySelection,
};
use laneflow_spatial::{
    CanonicalPoseBatch, CanonicalPoseF32, FramePlacementToken, PoseInput, PoseRecordId,
    SpatialSession,
};
use laneflow_static_contract::{
    EntityKind, LaneEdgeId, LaneEdgeOrdinal, ParkingFacilityId, ParkingFacilityOrdinal,
    ParkingSpaceId, ParkingSpaceOrdinal, VehicleProfileId,
};
use laneflow_static_network::SharedNetworkRevision;

#[path = "scene_loader.rs"]
mod scene_loader;

const NAMESPACE: &str = "bevy/parking-lifecycle";
const TICK_MS: u64 = 16;
const MAX_CATCH_UP: NonZeroU32 = NonZeroU32::new(8).expect("non-zero");
const LANE_WIDTH: f32 = 3.5;
const VEHICLE_LENGTH: f32 = 4.5;
const SPACE_LENGTH: f32 = 5.5;
const SPACE_WIDTH: f32 = 2.6;
const VIRTUAL_ENTRY_M: f64 = 30.0;
const VIRTUAL_EXIT_M: f64 = 28.0;
/// 环路四条边在路线上的相位。
pub(super) const OCCURRENCES_PER_LAP: u32 = 4;
const SOUTH: u32 = 0;
const EAST: u32 = 1;
const NORTH: u32 = 2;
const WEST: u32 = 3;

#[derive(Clone, Copy, Debug)]
pub(super) struct SceneConfig {
    pub laps: u32,
    pub dwell_ticks: u32,
}

impl SceneConfig {
    pub(super) const fn new() -> Self {
        Self {
            laps: 64,
            dwell_ticks: 600,
        }
    }
}

/// 脚本角色；见设计 §18.3。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    Explicit,
    Virtual,
    Blocker,
    Removal,
    Through,
}

/// 车辆的停车目标与首个入口所在的路线出现记录。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Plan {
    None,
    Space {
        space: ParkingSpaceOrdinal,
        entry_occurrence: u32,
    },
    Pool {
        facility: ParkingFacilityOrdinal,
        entry_occurrence: u32,
    },
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Cast {
    pub role: Role,
    pub vehicle: VehicleHandle,
    pub plan: Plan,
}

pub(super) struct Pad {
    pub space: ParkingSpaceOrdinal,
    pub pose: CanonicalPoseF32,
    pub length: f32,
    pub width: f32,
}

pub(super) struct Scene {
    pub session: LaneFlowSession,
    pub route: RouteHandle,
    pub cast: Vec<Cast>,
    pub facility: ParkingFacilityOrdinal,
    pub route_occurrences: u32,
    pub dwell_ticks: u32,
    pub lanes: Vec<Vec<Vec3>>,
    pub pads: Vec<Pad>,
    pub garage_entry: CanonicalPoseF32,
    pub garage_exit: CanonicalPoseF32,
    pub vehicle_length: f32,
    pub lane_width: f32,
    pub min: Vec3,
    pub max: Vec3,
}

fn debug(error: impl Debug) -> String {
    format!("{error:?}")
}

fn anchor(edge: &str, progress: f64) -> ParkingLaneAnchorInput<'_> {
    ParkingLaneAnchorInput {
        lane_edge: LaneEdgeReference::local(edge),
        progress_meters: progress,
    }
}

fn compile() -> Result<CompilationOutput, String> {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: NAMESPACE,
            source_document_key: "parking-lifecycle.document",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x93; 32],
            frontend_options_digest: [0x31; 32],
            random_seed: None,
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .map_err(debug)?;
    let mut module = SyntheticModuleBuilder::new(header, &limits).map_err(debug)?;
    module
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .map_err(debug)?
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "car",
            participant_class: ParticipantClassReference::local("road-user"),
            iidm: IidmVehicleProfileInput {
                length_meters: f64::from(VEHICLE_LENGTH),
                desired_speed_meters_per_second: 8.0,
                min_gap_meters: 2.0,
                time_headway_seconds: 1.4,
                max_acceleration_meters_per_second_squared: 1.8,
                comfortable_deceleration_meters_per_second_squared: 2.0,
                emergency_deceleration_meters_per_second_squared: 4.5,
            },
        })
        .map_err(debug)?;
    for (key, length, next) in [
        ("south", 120.0, "east"),
        ("east", 60.0, "north"),
        ("north", 120.0, "west"),
        ("west", 60.0, "south"),
    ] {
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: key,
                length_meters: length,
                speed_limit_meters_per_second: 10.0,
                successors: &[LaneEdgeReference::local(next)],
            })
            .map_err(debug)?;
    }
    module
        .add_parking_facility(ParkingFacilityInput {
            parking_facility_key: "garage",
            virtual_capacity: 2,
            virtual_entries: &[anchor("east", VIRTUAL_ENTRY_M)],
            virtual_exits: &[anchor("north", VIRTUAL_EXIT_M)],
        })
        .map_err(debug)?;
    // space-b 的入口紧邻虚拟出口：停在这里的车辆会挡住虚拟离场的插入位置。
    for (key, edge, entry, exit) in [
        ("space-a", "south", 40.0, 55.0),
        ("space-b", "north", 30.0, 45.0),
    ] {
        module
            .add_parking_space(ParkingSpaceInput {
                parking_space_key: key,
                parking_facility: Some(ParkingFacilityReference::local("garage")),
                entry: anchor(edge, entry),
                exit: anchor(edge, exit),
                geometry: ParkingSpaceGeometryInput {
                    lateral_offset_meters: 3.5,
                    heading_offset_radians: 0.0,
                    length_meters: f64::from(SPACE_LENGTH),
                    width_meters: f64::from(SPACE_WIDTH),
                },
            })
            .map_err(debug)?;
    }
    let point = |x: f32, z: f32| CanonicalPoint3F32Input { x, y: 0.0, z };
    let geometries = [
        ("south", [point(0.0, 0.0), point(120.0, 0.0)]),
        ("east", [point(120.0, 0.0), point(120.0, -60.0)]),
        ("north", [point(120.0, -60.0), point(0.0, -60.0)]),
        ("west", [point(0.0, -60.0), point(0.0, 0.0)]),
    ];
    let geometries = geometries
        .iter()
        .map(|(edge, points)| LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local(edge),
            centerline_points: points,
        })
        .collect::<Vec<_>>();
    module
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame-main",
            lane_edge_geometries: &geometries,
        })
        .map_err(debug)?;
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().map_err(debug)?)
        .map_err(debug)?;
    Compiler::new()
        .compile(unit.build().map_err(debug)?)
        .map_err(debug)
}

fn revision() -> Result<Arc<SharedNetworkRevision>, Box<dyn Error>> {
    let output = compile()?;
    let provenance = PortableEmissionProvenance::try_new("parking-lifecycle-v1").map_err(debug)?;
    let candidate = emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .map_err(debug)?;
    check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .map_err(debug)?;
    scene_loader::shared_revision(candidate.canonical_artifact().bytes())
}

fn stable(kind: EntityKind, key: &str) -> Result<laneflow_static_contract::StableId128, String> {
    derive_canonical_stable_id_v1(kind, NAMESPACE, key, &CompileLimits::p100_initial_v1())
        .map_err(debug)
}

fn edge(revision: &SharedNetworkRevision, key: &str) -> Result<LaneEdgeOrdinal, String> {
    let id = LaneEdgeId::from_untyped(stable(EntityKind::LaneEdge, key)?);
    revision
        .identity()
        .ordinal(id)
        .ok_or("车道不在修订内".into())
}

fn space(revision: &SharedNetworkRevision, key: &str) -> Result<ParkingSpaceOrdinal, String> {
    let id = ParkingSpaceId::from_untyped(stable(EntityKind::ParkingSpace, key)?);
    revision
        .identity()
        .ordinal(id)
        .ok_or("泊位不在修订内".into())
}

pub(super) fn build(config: SceneConfig) -> Result<Scene, Box<dyn Error>> {
    let revision = revision()?;
    let edges = [
        edge(&revision, "south")?,
        edge(&revision, "east")?,
        edge(&revision, "north")?,
        edge(&revision, "west")?,
    ];
    let space_a = space(&revision, "space-a")?;
    let space_b = space(&revision, "space-b")?;
    let facility = revision
        .identity()
        .ordinal(ParkingFacilityId::from_untyped(stable(
            EntityKind::ParkingFacility,
            "garage",
        )?))
        .ok_or("设施不在修订内")?;
    let profile = revision
        .identity()
        .ordinal(VehicleProfileId::from_untyped(stable(
            EntityKind::VehicleProfile,
            "car",
        )?))
        .ok_or("车型不在修订内")?;
    let mut world = scene_loader::install_world(
        &revision,
        "scenario://parking-lifecycle",
        WorldConfig::new(8, 1, 4_096, 4_096, TICK_MS),
        WorldPolicySelection::NotRequired,
    )?;
    let route_occurrences = config.laps * OCCURRENCES_PER_LAP;
    let route_edges: Vec<_> = (0..route_occurrences)
        .map(|occurrence| edges[(occurrence % OCCURRENCES_PER_LAP) as usize])
        .collect();
    let route = world.register_route(RouteRegisterInput::new(route_edges))?;
    let script = [
        (
            Role::Explicit,
            SOUTH,
            15_000,
            Plan::Space {
                space: space_a,
                entry_occurrence: SOUTH,
            },
        ),
        (Role::Through, SOUTH, 5_000, Plan::None),
        (Role::Through, WEST, 20_000, Plan::None),
        (
            Role::Virtual,
            EAST,
            2_000,
            Plan::Pool {
                facility,
                entry_occurrence: EAST,
            },
        ),
        (
            Role::Blocker,
            NORTH,
            5_000,
            Plan::Space {
                space: space_b,
                entry_occurrence: NORTH,
            },
        ),
        (
            Role::Removal,
            SOUTH,
            70_000,
            Plan::Pool {
                facility,
                entry_occurrence: EAST,
            },
        ),
    ];
    let mut cast = Vec::with_capacity(script.len());
    for (role, occurrence, progress_mm, plan) in script {
        let vehicle = world.spawn_vehicle(
            VehicleSpawnInput::new(profile, route, occurrence, progress_mm, 0).with_open_entrance(),
        )?;
        cast.push(Cast {
            role,
            vehicle,
            plan,
        });
    }
    let (lanes, min, max) = lane_geometry(&revision)?;
    let mut spatial = SpatialSession::bind(Arc::clone(&revision))
        .map_err(debug)?
        .ok_or("修订没有 Spatial component")?;
    let inputs = [
        PoseInput::parking(PoseRecordId::new(0), space_a),
        PoseInput::parking(PoseRecordId::new(1), space_b),
        PoseInput::lane(
            PoseRecordId::new(2),
            edges[EAST as usize],
            (VIRTUAL_ENTRY_M * 1_000.0) as u32,
        ),
        PoseInput::lane(
            PoseRecordId::new(3),
            edges[NORTH as usize],
            (VIRTUAL_EXIT_M * 1_000.0) as u32,
        ),
    ];
    let mut batch = CanonicalPoseBatch::new();
    spatial
        .extract_pose_batch(FramePlacementToken::new(1), &inputs, &mut batch)
        .map_err(debug)?;
    let [a, b, garage_entry, garage_exit] = batch.records() else {
        return Err("泊位采样数量不符".into());
    };
    let pads = [(space_a, a), (space_b, b)]
        .map(|(space, record)| Pad {
            space,
            pose: record.pose(),
            length: SPACE_LENGTH,
            width: SPACE_WIDTH,
        })
        .into();
    let (garage_entry, garage_exit) = (garage_entry.pose(), garage_exit.pose());
    let session = scene_loader::paired_session(world, MAX_CATCH_UP)?;
    Ok(Scene {
        session,
        route,
        cast,
        facility,
        route_occurrences,
        dwell_ticks: config.dwell_ticks,
        lanes,
        pads,
        garage_entry,
        garage_exit,
        vehicle_length: VEHICLE_LENGTH,
        lane_width: LANE_WIDTH,
        min,
        max,
    })
}

type LaneGeometry = (Vec<Vec<Vec3>>, Vec3, Vec3);

fn lane_geometry(revision: &SharedNetworkRevision) -> Result<LaneGeometry, Box<dyn Error>> {
    let pose = revision
        .spatial()
        .and_then(|spatial| spatial.lane_pose())
        .ok_or("缺少 Spatial centerline")?;
    let mut lanes = Vec::with_capacity(pose.lane_edge_count() as usize);
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for index in 0..pose.lane_edge_count() {
        let points: Vec<_> = pose
            .lane_geometry(LaneEdgeOrdinal::from_raw(index))
            .ok_or("缺少车道几何")?
            .points()
            .iter()
            .map(|point| Vec3::new(point.x, point.y, point.z))
            .collect();
        for &point in &points {
            min = min.min(point);
            max = max.max(point);
        }
        lanes.push(points);
    }
    Ok((lanes, min, max))
}
