//! 启动时在进程内编译两个修订并生成双向差分，装配已绑定路线的 Session。
//! 不写文件，不引入随机量；任一步失败都不发布部分场景。
use std::{error::Error, fmt::Debug, num::NonZeroU32, sync::Arc};

use bevy_math::Vec3;
use laneflow_bevy::LaneFlowSession;
use laneflow_compiler::{
    CanonicalFrameInput, CanonicalPoint3F32Input, CompilationOutput, CompilationUnitBuilder,
    CompileLimits, Compiler, IidmVehicleProfileInput, LaneEdgeGeometryInput, LaneEdgeInput,
    LaneEdgeReference, ParticipantClassInput, ParticipantClassReference, PortableDiffBase,
    PortableEmissionProvenance, SourceModuleHeader, SourceModuleHeaderInput,
    SyntheticModuleBuilder, VehicleProfileInput, derive_canonical_stable_id_v1,
    emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle, preflight_object_values};
use laneflow_runtime::{
    CommittedNetworkSource, PublishedLfcaReference, RouteRegisterInput, SemanticDiffOriginBinding,
    VehicleHandle, VehicleSpawnInput, WorldConfig, WorldPolicySelection,
};
use laneflow_static_contract::{
    EntityKind, ExactByteLength, LaneEdgeId, LaneEdgeOrdinal, PortableObjectKind,
    SEMANTIC_DIFF_FORMAT_VERSION, Sha256Digest, StableId128, VehicleProfileId,
    VehicleProfileOrdinal,
};
use laneflow_static_network::SharedNetworkRevision;
use sha2::Digest as _;

#[path = "scene_loader.rs"]
mod scene_loader;

const NAMESPACE: &str = "bevy/revision-cutover-example";
const TICK_MS: u64 = 16;
const MAX_CATCH_UP: NonZeroU32 = NonZeroU32::new(8).expect("non-zero");
const LANE_WIDTH: f32 = 3.5;
const VEHICLE_LENGTH: f32 = 4.5;
const LAPS: u32 = 64;
const RING: [&str; 4] = ["south", "east", "north", "west"];
/// 支路车的起点离开分叉处，避免与刚驶上 `north` 的环路车在画面上重叠。
pub(super) const SPUR_START_MM: u32 = 10_000;
/// 4 辆环路车的 (路线出现项, 进度)，互相隔开足够跟驰距离。
const RING_CARS: [(u32, u32); 4] = [(0, 20_000), (1, 10_000), (2, 50_000), (3, 5_000)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RevisionName {
    R1,
    R2,
}

impl RevisionName {
    pub(super) const fn other(self) -> Self {
        match self {
            Self::R1 => Self::R2,
            Self::R2 => Self::R1,
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::R1 => "R1",
            Self::R2 => "R2",
        }
    }
}

struct EdgeSpec {
    key: &'static str,
    length_meters: f64,
    successors: &'static [&'static str],
    points: &'static [(f32, f32)],
}

const fn edge_spec(
    key: &'static str,
    length_meters: f64,
    successors: &'static [&'static str],
    points: &'static [(f32, f32)],
) -> EdgeSpec {
    EdgeSpec {
        key,
        length_meters,
        successors,
        points,
    }
}

const R1_EDGES: &[EdgeSpec] = &[
    edge_spec("south", 120.0, &["east"], &[(0.0, 0.0), (120.0, 0.0)]),
    edge_spec("east", 60.0, &["north"], &[(120.0, 0.0), (120.0, -60.0)]),
    edge_spec("north", 120.0, &["west"], &[(120.0, -60.0), (0.0, -60.0)]),
    edge_spec("west", 60.0, &["south"], &[(0.0, -60.0), (0.0, 0.0)]),
];

/// `north` 的声明长度必须与弯折后的中心线相符，否则目标修订无法编译。
const R2_EDGES: &[EdgeSpec] = &[
    edge_spec("south", 120.0, &["east"], &[(0.0, 0.0), (120.0, 0.0)]),
    edge_spec(
        "east",
        60.0,
        &["north", "spur"],
        &[(120.0, 0.0), (120.0, -60.0)],
    ),
    edge_spec(
        "north",
        123.693,
        &["west"],
        &[(120.0, -60.0), (60.0, -75.0), (0.0, -60.0)],
    ),
    edge_spec("west", 60.0, &["south"], &[(0.0, -60.0), (0.0, 0.0)]),
    edge_spec("spur", 100.0, &[], &[(120.0, -60.0), (180.0, -140.0)]),
];

/// 切换到本修订所需的全部制品。
pub(super) struct Target {
    pub name: RevisionName,
    pub root: Arc<SharedNetworkRevision>,
    pub diff: Arc<[u8]>,
    pub binding: SemanticDiffOriginBinding,
}

impl Target {
    pub(super) fn source(&self) -> Result<CommittedNetworkSource, String> {
        source_of(&self.root, self.name)
    }
}

pub(super) struct Scene {
    pub session: LaneFlowSession,
    pub r1: Target,
    pub r2: Target,
    pub ring: Vec<VehicleHandle>,
    pub vehicle_length: f32,
    pub lane_width: f32,
    /// 两个修订路面的并集，初始视角据此覆盖切换前后的路面。
    pub min: Vec3,
    pub max: Vec3,
}

fn debug(error: impl Debug) -> String {
    format!("{error:?}")
}

fn compile(edges: &[EdgeSpec]) -> Result<CompilationOutput, String> {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: NAMESPACE,
            source_document_key: "revision-cutover.document",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x29; 32],
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
    for edge in edges {
        let successors: Vec<_> = edge
            .successors
            .iter()
            .map(|key| LaneEdgeReference::local(key))
            .collect();
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: edge.key,
                length_meters: edge.length_meters,
                speed_limit_meters_per_second: 10.0,
                successors: &successors,
            })
            .map_err(debug)?;
    }
    let points: Vec<Vec<_>> = edges
        .iter()
        .map(|edge| {
            edge.points
                .iter()
                .map(|&(x, z)| CanonicalPoint3F32Input { x, y: 0.0, z })
                .collect()
        })
        .collect();
    let geometries: Vec<_> = edges
        .iter()
        .zip(&points)
        .map(|(edge, points)| LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local(edge.key),
            centerline_points: points,
        })
        .collect();
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

struct Emitted {
    bytes: Vec<u8>,
    root: Arc<SharedNetworkRevision>,
    diff: Arc<[u8]>,
    binding: SemanticDiffOriginBinding,
}

fn emit(edges: &[EdgeSpec], base: Option<&[u8]>) -> Result<Emitted, Box<dyn Error>> {
    let output = compile(edges)?;
    let provenance = PortableEmissionProvenance::try_new("revision-cutover-v1").map_err(debug)?;
    let base = match base {
        None => PortableDiffBase::Genesis,
        Some(base) => PortableDiffBase::Artifact(
            preflight_object_values(
                base,
                PortableObjectKind::CanonicalArtifact,
                FormatLimits::HARD,
            )
            .map_err(debug)?,
        ),
    };
    let candidate =
        emit_portable_candidate(&output, &provenance, FormatLimits::HARD, base).map_err(debug)?;
    check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .map_err(debug)?;
    let diff = candidate.semantic_diff().bytes();
    let binding = SemanticDiffOriginBinding::new(
        SEMANTIC_DIFF_FORMAT_VERSION,
        Sha256Digest::from_bytes(sha2::Sha256::digest(diff).into()),
        ExactByteLength::new(diff.len() as u64),
    );
    Ok(Emitted {
        bytes: candidate.canonical_artifact().bytes().to_vec(),
        root: scene_loader::shared_revision(candidate.canonical_artifact().bytes())?,
        diff: Arc::from(diff),
        binding,
    })
}

fn scenario_key(name: RevisionName) -> &'static str {
    match name {
        RevisionName::R1 => "scenario://revision-cutover/r1",
        RevisionName::R2 => "scenario://revision-cutover/r2",
    }
}

fn source_of(
    root: &SharedNetworkRevision,
    name: RevisionName,
) -> Result<CommittedNetworkSource, String> {
    let origin = root.canonical_origin();
    let reference = PublishedLfcaReference::new(
        scenario_key(name),
        origin.canonical_artifact_digest(),
        origin.canonical_artifact_byte_length(),
        origin.network_revision(),
    )
    .map_err(debug)?;
    Ok(CommittedNetworkSource::Published { reference })
}

fn stable(kind: EntityKind, key: &str) -> Result<StableId128, String> {
    derive_canonical_stable_id_v1(kind, NAMESPACE, key, &CompileLimits::p100_initial_v1())
        .map_err(debug)
}

/// 序号可能随修订变化，边必须按稳定身份逐修订解析。
pub(super) fn edge(revision: &SharedNetworkRevision, key: &str) -> Result<LaneEdgeOrdinal, String> {
    let id = LaneEdgeId::from_untyped(stable(EntityKind::LaneEdge, key)?);
    revision
        .identity()
        .ordinal(id)
        .ok_or_else(|| format!("车道 {key} 不在修订内"))
}

pub(super) fn profile(revision: &SharedNetworkRevision) -> Result<VehicleProfileOrdinal, String> {
    let id = VehicleProfileId::from_untyped(stable(EntityKind::VehicleProfile, "car")?);
    revision
        .identity()
        .ordinal(id)
        .ok_or_else(|| "车型不在修订内".to_owned())
}

/// 取修订 Spatial component 的中心线，路面表现只从这里来。
pub(super) fn centerlines(revision: &SharedNetworkRevision) -> Result<Vec<Vec<Vec3>>, String> {
    let pose = revision
        .spatial()
        .and_then(|spatial| spatial.lane_pose())
        .ok_or("缺少 Spatial centerline")?;
    (0..pose.lane_edge_count())
        .map(|index| {
            let geometry = pose
                .lane_geometry(LaneEdgeOrdinal::from_raw(index))
                .ok_or("缺少车道几何")?;
            Ok(geometry
                .points()
                .iter()
                .map(|point| Vec3::new(point.x, point.y, point.z))
                .collect())
        })
        .collect()
}

pub(super) fn build() -> Result<Scene, Box<dyn Error>> {
    let r1 = emit(R1_EDGES, None)?;
    let r2 = emit(R2_EDGES, Some(&r1.bytes))?;
    let r1_back = emit(R1_EDGES, Some(&r2.bytes))?;
    // 回切差分的基线是 R2，产物内容必须仍是 R1，才能共用同一个根对象。
    let (origin, back) = (r1.root.canonical_origin(), r1_back.root.canonical_origin());
    if origin.network_revision() != back.network_revision()
        || origin.canonical_artifact_digest() != back.canonical_artifact_digest()
    {
        return Err("回切制品与 R1 内容不一致".into());
    }
    let mut world = scene_loader::install_world(
        &r1.root,
        scenario_key(RevisionName::R1),
        WorldConfig::new(8, 2, 4_096, 4_096, TICK_MS),
        WorldPolicySelection::NotRequired,
    )?;
    let ring = RING
        .map(|key| edge(&r1.root, key))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let route_edges: Vec<_> = (0..LAPS as usize * ring.len())
        .map(|occurrence| ring[occurrence % ring.len()])
        .collect();
    let route = world.register_route(RouteRegisterInput::new(route_edges))?;
    let car = profile(&r1.root)?;
    let mut cars = Vec::with_capacity(RING_CARS.len());
    for (occurrence, progress_mm) in RING_CARS {
        cars.push(world.spawn_vehicle(
            VehicleSpawnInput::new(car, route, occurrence, progress_mm, 0).with_open_entrance(),
        )?);
    }
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for root in [&r1.root, &r2.root] {
        for point in centerlines(root)?.into_iter().flatten() {
            min = min.min(point);
            max = max.max(point);
        }
    }
    let session = scene_loader::paired_session(world, MAX_CATCH_UP)?;
    Ok(Scene {
        session,
        r1: Target {
            name: RevisionName::R1,
            root: r1.root,
            diff: r1_back.diff,
            binding: r1_back.binding,
        },
        r2: Target {
            name: RevisionName::R2,
            root: r2.root,
            diff: r2.diff,
            binding: r2.binding,
        },
        ring: cars,
        vehicle_length: VEHICLE_LENGTH,
        lane_width: LANE_WIDTH,
        min,
        max,
    })
}
