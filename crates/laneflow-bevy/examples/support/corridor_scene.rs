//! GUI 与 headless 共用的两阶段人口启动和静态表现数据；不拥有交通规则。
use std::{error::Error, num::NonZeroU32, sync::Arc};

use bevy_ecs::resource::Resource;
use bevy_math::Vec3;
use laneflow_bevy::LaneFlowSession;
use laneflow_runtime::WorldConfig;
use laneflow_scenario::signalized_corridor::{
    BoundCorridorCatalog, CorridorCatalog, CorridorPopulationConfig, CorridorPopulationController,
    CorridorPopulationPrepare, PASSENGER_CAR_PROFILE_KEY, bind,
};
use laneflow_static_contract::{
    LaneEdgeOrdinal, SignalGroupOrdinal, StopLineOrdinal, VehicleProfileOrdinal,
};
use laneflow_static_network::SharedNetworkRevision;

#[path = "scene_loader.rs"]
mod scene_loader;

pub(super) const TICK_MS: u64 = 16;
pub(super) const MAX_CATCH_UP: NonZeroU32 = NonZeroU32::new(8).expect("non-zero");
pub(super) const LFCA: &[u8] =
    include_bytes!("../../../../examples/data/v0.2-signalized-corridor.lfca");
pub(super) const CATALOG: &str =
    include_str!("../../../../examples/data/v0.2-signalized-corridor.catalog.toml");
const AUTHORING: &str = include_str!("../../../../examples/config/v0.10-signalized-corridor.toml");

pub(super) struct Scene {
    pub session: LaneFlowSession,
    pub controller: CorridorPopulationController,
    pub geometry: Geometry,
    pub config: CorridorPopulationConfig,
    pub vehicle_length: f32,
}

#[derive(Clone, Debug)]
pub(super) struct LampBinding {
    pub stop: StopLineOrdinal,
    pub group: SignalGroupOrdinal,
    pub position: Vec3,
    pub tangent: Vec3,
}

#[derive(Resource, Clone, Debug)]
pub(super) struct Geometry {
    pub lanes: Vec<Vec<Vec3>>,
    pub lamps: Vec<LampBinding>,
    pub min: Vec3,
    pub max: Vec3,
    pub lane_width: f32,
}

pub(super) fn build(config: CorridorPopulationConfig) -> Result<Scene, Box<dyn Error>> {
    let catalog = toml::from_str(CATALOG)?;
    build_catalog(config, &catalog)
}

pub(super) fn build_catalog(
    config: CorridorPopulationConfig,
    catalog: &CorridorCatalog,
) -> Result<Scene, Box<dyn Error>> {
    let revision = scene_loader::shared_revision(LFCA)?;
    let bound = bind(catalog, &revision)?;
    let profile = *bound
        .profiles
        .get(PASSENGER_CAR_PROFILE_KEY)
        .ok_or("缺少乘用车 profile")?;
    let route_capacity = u32::try_from(bound.routes.len())?;
    build_bound(
        config,
        revision,
        bound,
        profile,
        WorldConfig::new(
            u32::try_from(config.target_vehicle_count())?,
            route_capacity,
            1_024,
            1_024,
            TICK_MS,
        ),
    )
}

fn build_bound(
    config: CorridorPopulationConfig,
    revision: Arc<SharedNetworkRevision>,
    bound: BoundCorridorCatalog,
    profile: VehicleProfileOrdinal,
    world_config: WorldConfig,
) -> Result<Scene, Box<dyn Error>> {
    let policy = bound.policy_selection;
    let vehicle_length = revision
        .traffic()
        .relations()
        .vehicle_profile(profile)
        .ok_or("profile 不在共享路网修订内")?
        .length_mm() as f32
        / 1_000.0;
    let mut prepared = CorridorPopulationPrepare::prepare(config, bound, &revision, profile)?;
    let mut runtime = scene_loader::install_world(
        &revision,
        "scenario://corridor-traffic",
        world_config,
        policy,
    )?;
    // 任一失败直接丢弃未发布实例；包括 admission 回滚不完整，不拿残余世界重试。
    let (vehicles, routes) = prepared.spawn_initial_vehicles(&mut runtime)?;
    let controller = prepared.bind(&mut runtime, &vehicles, &routes)?;
    let geometry = geometry(&revision)?;
    let session = scene_loader::paired_session(runtime, MAX_CATCH_UP)?;
    Ok(Scene {
        session,
        controller,
        geometry,
        config,
        vehicle_length,
    })
}

pub(super) fn common_group(
    groups: impl IntoIterator<Item = Option<SignalGroupOrdinal>>,
) -> Result<SignalGroupOrdinal, &'static str> {
    let mut groups = groups.into_iter();
    let first = groups.next().flatten().ok_or("停止线缺少信号组")?;
    if groups.any(|group| group != Some(first)) {
        return Err("共享停止线的信号组缺失或有歧义");
    }
    Ok(first)
}

fn geometry(revision: &SharedNetworkRevision) -> Result<Geometry, Box<dyn Error>> {
    let authoring: toml::Value = toml::from_str(AUTHORING)?;
    let lane_width = authoring["geometry"]["lane_width_meters"]
        .as_float()
        .ok_or("缺少展示车道宽度")? as f32;
    if !lane_width.is_finite() || lane_width <= 0.0 {
        return Err("无效展示车道宽度".into());
    }
    let pose = revision
        .spatial()
        .and_then(|spatial| spatial.lane_pose())
        .ok_or("缺少 Spatial centerline")?;
    let relations = revision.traffic().relations();
    let mut lanes = Vec::new();
    let mut lamps = Vec::new();
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for index in 0..pose.lane_edge_count() {
        let edge = LaneEdgeOrdinal::from_raw(index);
        let points: Vec<_> = pose
            .lane_geometry(edge)
            .ok_or("缺少车道几何")?
            .points()
            .iter()
            .map(|point| Vec3::new(point.x, point.y, point.z))
            .collect();
        if points.len() < 2
            || points.iter().any(|point| !point.is_finite())
            || points
                .windows(2)
                .any(|pair| pair[0].distance_squared(pair[1]) <= f32::EPSILON)
        {
            return Err("无效车道 centerline".into());
        }
        for &point in &points {
            min = min.min(point);
            max = max.max(point);
        }
        if let Some(stop) = relations.stop_line_for_edge(edge) {
            let view = relations.stop_line(stop).ok_or("缺失停止线")?;
            let group = common_group(view.gates().iter().map(|gate| {
                relations
                    .maneuver_gate(*gate)
                    .and_then(|gate| gate.signal_group())
            }))?;
            if relations.signal_group(group).is_none() {
                return Err("缺失信号组".into());
            }
            if lamps.iter().any(|lamp: &LampBinding| lamp.stop == stop) {
                return Err("重复停止线".into());
            }
            let position = *points.last().ok_or("缺少停止线位置")?;
            lamps.push(LampBinding {
                stop,
                group,
                position,
                tangent: (position - points[points.len() - 2]).normalize(),
            });
        }
        lanes.push(points);
    }
    if lanes.is_empty() || lamps.is_empty() {
        return Err("场景缺少道路或灯具".into());
    }
    Ok(Geometry {
        lanes,
        lamps,
        min,
        max,
        lane_width,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c02_profile_and_partial_admission_fail_without_publishing_a_session() {
        let revision = scene_loader::shared_revision(LFCA).unwrap();
        let catalog = toml::from_str(CATALOG).unwrap();
        let bound = bind(&catalog, &revision).unwrap();
        let profile = bound.profiles[PASSENGER_CAR_PROFILE_KEY];
        let config = CorridorPopulationConfig::default();
        let bad_profile = build_bound(
            config,
            Arc::clone(&revision),
            bound.clone(),
            VehicleProfileOrdinal::from_raw(u32::MAX),
            WorldConfig::new(100, 28, 1_024, 1_024, TICK_MS),
        )
        .err()
        .unwrap();
        assert!(bad_profile.to_string().contains("profile"));
        let rejected = build_bound(
            config,
            revision,
            bound,
            profile,
            WorldConfig::new(1, 28, 1_024, 1_024, TICK_MS),
        )
        .err()
        .unwrap();
        assert!(matches!(rejected.downcast_ref::<laneflow_scenario::signalized_corridor::CorridorPopulationError>(),
            Some(laneflow_scenario::signalized_corridor::CorridorPopulationError::InitialSpawnRejected { .. })),
            "original admission error: {rejected:?}");
    }
}
