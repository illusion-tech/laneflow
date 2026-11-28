//! libtest 使用本地世界以触发内核故障。申请和阶段裁决与正式宿主共用。
use laneflow_static_network::{BoundaryOwner, vehicle::GeometryWorkLimits};
use laneflow_synth_generator::{InitialState, RouteRecord, handoff, runtime};

use crate::{
    ClearanceSpeedCurve, ClearanceSpeedPoint, HandoffDrivingConfig, RouteHandle,
    RouteHandoffOccurrence, RouteHandoffRequest, RouteRegisterInput, TrafficWorld, VehicleHandle,
    WorldConfig,
};

pub(super) fn configure_world(
    state: &InitialState,
    config: WorldConfig,
) -> Result<WorldConfig, String> {
    let Some((points, max_span_mm, travel_tolerance_mm)) = runtime::driving_parts(state)? else {
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

pub(super) fn register_route(world: &mut TrafficWorld, record: &RouteRecord) -> RouteHandle {
    let revision = world.revision();
    let (edges, window) = runtime::route_parts(&revision, record).expect("正式输入的路线绑定");
    let mut input = RouteRegisterInput::new(edges);
    if let Some(window) = window {
        input = input.with_handoffs(vec![RouteHandoffOccurrence {
            revision: revision.network_revision(),
            source_index: window.source_index,
            owner: BoundaryOwner::Section(window.section),
            left_index: window.left_index,
            stations_mm: window.stations_mm,
        }]);
    }
    world.register_route(input).expect("登记测试世界路线")
}

pub(super) fn vehicle_reader<'a>(
    world: &'a TrafficWorld,
    handles: &'a [VehicleHandle],
    routes: &'a [RouteHandle],
) -> impl Fn(usize, u32) -> Result<handoff::VehicleObservation, String> + 'a {
    move |vehicle_rank, route_rank| {
        let handle = *handles.get(vehicle_rank).ok_or("缺少换道句柄")?;
        let vehicle = world.vehicle(handle).ok_or("换道车辆失效")?;
        if routes.get(route_rank as usize) != Some(&vehicle.route()) {
            return Err("车辆不在指定换道路线".into());
        }
        let mut active = world.vehicle_route_handoffs(handle);
        let handoff = active.next().map(|p| handoff::Reservation {
            acquired_tick: p.acquired_tick,
            source_index: p.source_index,
            front_handed_off: p.front_handed_off,
        });
        if active.next().is_some() {
            return Err("同一车辆持有多条横移".into());
        }
        Ok(handoff::VehicleObservation {
            cursor: vehicle.route_edge_index(),
            front_mm: vehicle.rounded_front_progress_mm(),
            handoff,
        })
    }
}

pub(super) fn requests(
    world: &TrafficWorld,
    state: &InitialState,
    handles: &[VehicleHandle],
    routes: &[RouteHandle],
) -> Result<Vec<RouteHandoffRequest>, String> {
    let revision = world.revision().network_revision();
    handoff::requests(
        state,
        world.tick_index(),
        handles.len(),
        vehicle_reader(world, handles, routes),
        |request| {
            Ok(RouteHandoffRequest {
                vehicle: handles[request.vehicle_rank],
                route: routes[request.route_rank],
                revision,
                source_index: request.source_index,
                station_window_mm: request.station_window_mm,
                rear_stations_mm: request.rear_stations_mm,
                speed_m_s: f64::from(request.speed_mm_s) / 1_000.,
                limits: GeometryWorkLimits::STANDARD,
            })
        },
    )
}
