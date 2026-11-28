//! 将 LF-SYNTH 逻辑场景绑定到一个实际的 TrafficWorld。
use std::num::NonZeroU32;

use laneflow_runtime::{
    ClearanceSpeedCurve, ClearanceSpeedPoint, HandoffDrivingConfig, RouteHandle,
    RouteHandoffOccurrence, RouteHandoffRequest, RouteRegisterInput, TrafficWorld, VehicleHandle,
    WorldConfig,
};
use laneflow_static_contract::{LaneEdgeId, LaneEdgeOrdinal, RoadSectionId, RoadSectionOrdinal};
use laneflow_static_network::{BoundaryOwner, SharedNetworkRevision, vehicle::GeometryWorkLimits};

use crate::{
    InitialState, RouteRecord,
    handoff::{self, Reservation, VehicleObservation},
};

/// 输入给定的限速曲线及两个非零毫米参数（分段跨度、行程容差）。
pub type DrivingParts<'a> = (&'a [[u32; 2]], NonZeroU32, NonZeroU32);

/// 校验并借用场景的驾驶设置。
///
/// # Errors
/// 分段跨度或行程容差为零时返回错误。
pub fn driving_parts(state: &InitialState) -> Result<Option<DrivingParts<'_>>, String> {
    let Some(input) = &state.handoff_driving else {
        return Ok(None);
    };
    Ok(Some((
        &input.curve,
        NonZeroU32::new(input.max_span_mm).ok_or("横移侧向分段跨度为零")?,
        NonZeroU32::new(input.travel_tolerance_mm).ok_or("横移行程容差为零")?,
    )))
}

/// 在世界配置中安装场景声明的净空限速曲线。
///
/// # Errors
/// 驾驶参数或曲线控制点无效时返回错误。
pub fn configure_world(state: &InitialState, config: WorldConfig) -> Result<WorldConfig, String> {
    let Some((points, max_span_mm, travel_tolerance_mm)) = driving_parts(state)? else {
        return Ok(config);
    };
    let curve = ClearanceSpeedCurve::try_new(
        points
            .iter()
            .map(|&[clearance_mm, speed_mm_s]| ClearanceSpeedPoint {
                clearance_mm,
                speed_mm_s,
            })
            .collect(),
    )
    .map_err(|e| e.to_string())?;
    Ok(config.with_handoff_driving(HandoffDrivingConfig {
        curve,
        max_span_mm,
        travel_tolerance_mm,
    }))
}

/// 同一修订中解析出的路段共边与路线出现。
pub struct ResolvedWindow {
    pub section: RoadSectionOrdinal,
    pub left_index: u32,
    pub source_index: u32,
    pub stations_mm: [u64; 2],
}

/// 从源记录解析路线的稳定边与可选共边窗口。
///
/// # Errors
/// 稳定身份格式错误或当前修订没有对应边、路段时返回错误。
pub fn route_parts(
    revision: &SharedNetworkRevision,
    record: &RouteRecord,
) -> Result<(Vec<LaneEdgeOrdinal>, Option<ResolvedWindow>), String> {
    let identity = revision.identity();
    let edges = record
        .edge_ids
        .iter()
        .map(|id| {
            let stable = id
                .parse::<LaneEdgeId>()
                .map_err(|e| format!("路线边身份 {id}: {e:?}"))?;
            identity
                .ordinal(stable)
                .ok_or_else(|| format!("无路线边 {id}"))
        })
        .collect::<Result<_, _>>()?;
    let window = if let Some(input) = &record.handoff {
        let section = input
            .section_id
            .parse::<RoadSectionId>()
            .map_err(|e| format!("横移路段身份: {e:?}"))?;
        Some(ResolvedWindow {
            section: identity.ordinal(section).ok_or("无横移路段")?,
            left_index: input.left_index,
            source_index: input.source_index,
            stations_mm: input.stations_mm,
        })
    } else {
        None
    };
    Ok((edges, window))
}

/// 按状态目录注册带显式窗口的路线。
///
/// # Errors
/// 路线解析失败或世界拒绝注册时返回错误。
pub fn register_route(
    world: &mut TrafficWorld,
    record: &RouteRecord,
) -> Result<RouteHandle, String> {
    let revision = world.revision();
    let (edges, window) = route_parts(&revision, record)?;
    let mut route = RouteRegisterInput::new(edges);
    if let Some(w) = window {
        route = route.with_handoffs(vec![RouteHandoffOccurrence {
            revision: revision.network_revision(),
            owner: BoundaryOwner::Section(w.section),
            left_index: w.left_index,
            source_index: w.source_index,
            stations_mm: w.stations_mm,
        }]);
    }
    world.register_route(route).map_err(|e| e.to_string())
}

fn vehicle_observation(
    world: &TrafficWorld,
    handles: &[VehicleHandle],
    routes: &[RouteHandle],
    rank: usize,
    route_rank: u32,
) -> Result<VehicleObservation, String> {
    let handle = *handles.get(rank).ok_or("缺少换道车辆句柄")?;
    let vehicle = world.vehicle(handle).ok_or("换道车辆失效")?;
    if routes.get(route_rank as usize) != Some(&vehicle.route()) {
        return Err("车辆不在指定换道路线".into());
    }
    let mut held = world.vehicle_route_handoffs(handle);
    let handoff = held.next().map(|h| Reservation {
        source_index: h.source_index,
        acquired_tick: h.acquired_tick,
        front_handed_off: h.front_handed_off,
    });
    if held.next().is_some() {
        return Err("同一车辆持有多条横移".into());
    }
    Ok(VehicleObservation {
        cursor: vehicle.route_edge_index(),
        front_mm: vehicle.rounded_front_progress_mm(),
        handoff,
    })
}

/// 用拍初已提交状态生成本世界内的申请。
///
/// # Errors
/// 输入目录、已提交车辆或本拍位移与场景要求不符时返回错误。
pub fn requests(
    world: &TrafficWorld,
    state: &InitialState,
    handles: &[VehicleHandle],
    routes: &[RouteHandle],
) -> Result<Vec<RouteHandoffRequest>, String> {
    if state.identity.lane_change().is_none() {
        return Ok(Vec::new());
    }
    let revision = world.revision().network_revision();
    handoff::requests(
        state,
        world.tick_index(),
        handles.len(),
        |rank, route| vehicle_observation(world, handles, routes, rank, route),
        |r| {
            Ok(RouteHandoffRequest {
                vehicle: handles[r.vehicle_rank],
                route: routes[r.route_rank],
                revision,
                source_index: r.source_index,
                station_window_mm: r.station_window_mm,
                rear_stations_mm: r.rear_stations_mm,
                speed_m_s: f64::from(r.speed_mm_s) / 1_000.,
                limits: GeometryWorkLimits::STANDARD,
            })
        },
    )
}

/// 从本世界读回按逻辑槽位排序的横移历史。
///
/// # Errors
/// 已提交车辆、横移预约或阶段顺序与场景要求不符时返回错误。
pub fn observe(
    world: &TrafficWorld,
    state: &InitialState,
    handles: &[VehicleHandle],
    routes: &[RouteHandle],
    last: &mut [u32],
    journal: &mut Vec<(u64, u16, u32, u32)>,
) -> Result<(), String> {
    handoff::observe(
        state,
        world.tick_index(),
        handles.len(),
        |rank, route| vehicle_observation(world, handles, routes, rank, route),
        last,
        journal,
    )
}
