//! 启动时确定性生成参考路口网格，并装配已绑定路线和车辆的 Session。
//! 不写文件，不引入随机量；任一步失败都不发布部分场景。
use std::{error::Error, num::NonZeroU32};

use bevy_math::Vec3;
use laneflow_bevy::LaneFlowSession;
use laneflow_junction_generator::{JunctionConfig, generate_grid};
use laneflow_runtime::{RouteRegisterInput, VehicleHandle, VehicleSpawnInput, WorldConfig};
use laneflow_scenario::complex_junction::{VEHICLE_PROFILE_KEY, bind};
use laneflow_static_contract::LaneEdgeOrdinal;
use laneflow_static_network::SharedNetworkRevision;

#[path = "scene_loader.rs"]
mod scene_loader;

const CONFIG: &str = include_str!("../../../../examples/config/v0.1-complex-junction.toml");
const TICK_MS: u64 = 16;
const MAX_CATCH_UP: NonZeroU32 = NonZeroU32::new(8).expect("non-zero");
const VEHICLES_PER_CELL: usize = 312;
const ROUTE_LEGS: usize = 64;
/// 每格世界容量与 #285 网格相同：路线 11 条，边与路线存储按格线性放大。
const ROUTES_PER_CELL: usize = 11;
const ROUTE_EDGES_PER_CELL: u64 = 16_384;
const ROUTE_OCCURRENCES_PER_CELL: u64 = 65_536;

#[derive(Clone, Copy, Debug)]
pub(super) struct SceneConfig {
    pub cells: usize,
    pub vehicles_per_cell: usize,
    pub route_legs: usize,
}

impl SceneConfig {
    pub(super) const fn new(cells: usize) -> Self {
        Self {
            cells,
            vehicles_per_cell: VEHICLES_PER_CELL,
            route_legs: ROUTE_LEGS,
        }
    }
}

pub(super) struct Scene {
    pub session: LaneFlowSession,
    pub vehicles: Vec<VehicleHandle>,
    pub lanes: Vec<Vec<Vec3>>,
    pub lane_lengths_mm: Vec<u32>,
    pub vehicle_length: f32,
    pub lane_width: f32,
    pub min: Vec3,
    pub max: Vec3,
}

pub(super) fn build(config: SceneConfig) -> Result<Scene, Box<dyn Error>> {
    let junction = JunctionConfig::parse(CONFIG)?;
    let grid = generate_grid(&junction, config.cells)?;
    let revision = scene_loader::shared_revision(&grid.lfca)?;
    let first = bind(grid.catalog.cells.first().ok_or("网格没有路口")?, &revision)?;
    let profile = *first
        .profiles
        .get(VEHICLE_PROFILE_KEY)
        .ok_or("缺少车型 profile")?;
    let vehicle_length = revision
        .traffic()
        .relations()
        .vehicle_profile(profile)
        .ok_or("profile 不在共享路网修订内")?
        .length_mm() as f32
        / 1_000.0;
    let cells = config.cells as u64;
    let mut world = scene_loader::install_world(
        &revision,
        "scenario://culling-extraction",
        WorldConfig::new(
            u32::try_from(config.cells * config.vehicles_per_cell)?,
            u32::try_from(config.cells * ROUTES_PER_CELL)?,
            cells * ROUTE_EDGES_PER_CELL,
            cells * ROUTE_OCCURRENCES_PER_CELL,
            TICK_MS,
        ),
        first.policy_selection,
    )?;
    let mut vehicles = Vec::with_capacity(config.cells * config.vehicles_per_cell);
    for catalog in &grid.catalog.cells {
        let bound = bind(catalog, &revision)?;
        // catalog 路段依次接续成长路线，保留全部真实边、门和策略。
        let mut routes = Vec::with_capacity(bound.route_exits.len());
        for (initial_index, initial) in bound.route_exits.iter().enumerate() {
            let mut edges = initial.edges.to_vec();
            for leg in 1..config.route_legs {
                let end = *edges.last().ok_or("catalog 路线为空")?;
                let choices: Vec<_> = bound
                    .route_exits
                    .iter()
                    .filter(|route| route.edges.first() == Some(&end))
                    .collect();
                if choices.is_empty() {
                    return Err("环路边没有后续 catalog 路线".into());
                }
                let next = choices[(initial_index + leg) % choices.len()];
                edges.extend_from_slice(&next.edges[1..]);
            }
            routes.push(world.register_route(RouteRegisterInput::new(edges))?);
        }
        let mut lanes = vec![Vec::new(); bound.portal_lanes.len()];
        for slot in &bound.spawn_slots {
            lanes[slot.portal_lane_index].push(slot);
        }
        for lane in &mut lanes {
            lane.sort_by_key(|slot| std::cmp::Reverse(slot.progress_mm));
        }
        // 按入口车道轮转，从靠前的槽位开始；同参数启动的摆放计划相同。
        for index in 0..config.vehicles_per_cell {
            let lane = index % lanes.len();
            let depth = index / lanes.len();
            let slot = lanes[lane].get(depth).ok_or("入口车道生成槽位不足")?;
            let choices = &bound.portal_lanes[lane].choices;
            let route = routes[choices[depth % choices.len()].route_index];
            vehicles.push(world.spawn_vehicle(
                VehicleSpawnInput::new(profile, route, 0, slot.progress_mm, 0).with_open_entrance(),
            )?);
        }
    }
    let (lanes, min, max) = lane_geometry(&revision)?;
    let lane_lengths_mm = revision.traffic().lane_lengths_millimetres().to_vec();
    let session = scene_loader::paired_session(world, MAX_CATCH_UP)?;
    Ok(Scene {
        session,
        vehicles,
        lanes,
        lane_lengths_mm,
        vehicle_length,
        lane_width: junction.geometry.lane_width_meters as f32,
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
    if lanes.is_empty() {
        return Err("场景缺少车道".into());
    }
    Ok((lanes, min, max))
}
