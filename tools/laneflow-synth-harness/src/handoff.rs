//! L1 真实路线交接。正常、冷恢复和失败重试以相同输入及拍初提交态生成申请。
use std::num::NonZeroU32;

use laneflow_runtime::{
    ClearanceSpeedCurve, ClearanceSpeedPoint, HandoffDrivingConfig, RouteHandle,
    RouteHandoffOccurrence, RouteHandoffRequest, RouteRegisterInput, TrafficWorld, VehicleHandle,
    WorldConfig,
};
use laneflow_static_contract::{LaneEdgeId, RoadSectionId};
use laneflow_static_network::{BoundaryOwner, vehicle::GeometryWorkLimits};
use laneflow_synth_generator::{InitialState, RouteRecord};

pub(crate) fn configure_world(
    state: &InitialState,
    config: WorldConfig,
) -> Result<WorldConfig, String> {
    let Some(input) = &state.handoff_driving else {
        return Ok(config);
    };
    let curve = ClearanceSpeedCurve::try_new(
        input
            .curve
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
        max_span_mm: NonZeroU32::new(input.max_span_mm).ok_or("横移侧向分段跨度为零")?,
        travel_tolerance_mm: NonZeroU32::new(input.travel_tolerance_mm)
            .ok_or("横移行程容差为零")?,
    }))
}

pub(crate) fn register_route(
    world: &mut TrafficWorld,
    record: &RouteRecord,
) -> Result<RouteHandle, String> {
    let revision = world.revision();
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
        .collect::<Result<Vec<_>, _>>()?;
    let mut route = RouteRegisterInput::new(edges);
    if let Some(input) = &record.handoff {
        let section = input
            .section_id
            .parse::<RoadSectionId>()
            .map_err(|e| format!("横移路段身份: {e:?}"))?;
        route = route.with_handoffs(vec![RouteHandoffOccurrence {
            revision: revision.network_revision(),
            owner: BoundaryOwner::Section(identity.ordinal(section).ok_or("无横移路段")?),
            left_index: input.left_index,
            source_index: input.source_index,
            stations_mm: input.stations_mm,
        }]);
    }
    world.register_route(route).map_err(|e| e.to_string())
}

/// 换道簇两条源边及公共目标沿同一平直站位轴。起步位置从出生时基准平移到本拍前杠。
pub(crate) fn requests(
    world: &TrafficWorld,
    state: &InitialState,
    handles: &[VehicleHandle],
    routes: &[RouteHandle],
) -> Result<Vec<RouteHandoffRequest>, String> {
    let Some(params) = state.identity.lane_change() else {
        return Ok(Vec::new());
    };
    let start = state.identity.scale as usize;
    let vehicles = state.vehicles.get(start..).ok_or("换道车辆缺失")?;
    let changers = handles.get(start..).ok_or("换道句柄缺失")?;
    let slots = params.changers_per_cluster as usize;
    if slots != 2
        || vehicles.len() != slots * params.clusters as usize
        || changers.len() != vehicles.len()
    {
        return Err("换道簇逻辑槽位不完整".into());
    }
    let revision = world.revision().network_revision();
    let mut requests = Vec::new();
    for (records, handles) in vehicles
        .chunks_exact(slots)
        .zip(changers.chunks_exact(slots))
    {
        // 刚授予的下一拍，等待车再申请一次，检验已提交预约的排他性。
        // 其余存续拍不反复重建相交轨迹；清空后按拍初前杠再次申请。
        let active = handles.iter().find_map(|&vehicle| {
            world
                .vehicle_route_handoffs(vehicle)
                .next()
                .map(|p| (vehicle, p.acquired_tick))
        });
        if active.is_some_and(|(_, acquired)| acquired < world.tick_index()) {
            continue;
        }
        for (record, &vehicle) in records.iter().zip(handles) {
            if active.is_some_and(|(owner, _)| vehicle == owner) {
                continue;
            }
            let route_index = record.route_rank as usize;
            let input = state
                .routes
                .get(route_index)
                .and_then(|r| r.handoff.as_ref())
                .ok_or("缺少换道窗口")?;
            let route = *routes.get(route_index).ok_or("缺少换道路线")?;
            let committed = world.vehicle(vehicle).ok_or("换道车辆失效")?;
            if committed.route() != route {
                return Err("车辆不在指定换道路线".into());
            }
            if committed.route_edge_index() != input.source_index {
                continue;
            }
            let front = u64::from(committed.rounded_front_progress_mm());
            let offset = front
                .checked_sub(u64::from(record.progress_mm))
                .ok_or("换道前杠退至出生位之前")?;
            let [lo, hi] = input.rear_stations_mm;
            let rear = [
                lo.checked_add(offset).ok_or("后轴起点溢出")?,
                hi.checked_add(offset).ok_or("后轴终点溢出")?,
            ];
            if rear[1] > input.station_window_mm[1] {
                continue;
            }
            requests.push(RouteHandoffRequest {
                vehicle,
                route,
                revision,
                source_index: input.source_index,
                station_window_mm: input.station_window_mm,
                rear_stations_mm: rear,
                speed_m_s: f64::from(input.speed_mm_s) / 1_000.,
                limits: GeometryWorkLimits::STANDARD,
            });
        }
    }
    Ok(requests)
}

/// 按逻辑槽位记录进入、前杠交接、车尾清空。第四列为输入中的路线身份。
pub(crate) fn observe(
    world: &TrafficWorld,
    state: &InitialState,
    handles: &[VehicleHandle],
    last: &mut [u32],
    journal: &mut Vec<(u64, u16, u32, u32)>,
) -> Result<(), String> {
    let params = state.identity.lane_change().ok_or("无换道族参数")?;
    let start = state.identity.scale as usize;
    let slots = params.changers_per_cluster as usize;
    if slots != 2
        || last.len() != slots * params.clusters as usize
        || handles.len() != start + last.len()
        || state.vehicles.len() != handles.len()
    {
        return Err("换道观测槽位不完整".into());
    }
    let tick = world.tick_index();
    for cluster in 0..params.clusters as usize {
        let first = cluster * slots;
        let mut held = 0;
        for slot in first..first + slots {
            let handle = handles[start + slot];
            let record = &state.vehicles[start + slot];
            let input = state
                .routes
                .get(record.route_rank as usize)
                .and_then(|r| r.handoff.as_ref())
                .ok_or("缺少换道观测窗口")?;
            let vehicle = world.vehicle(handle).ok_or("换道观测句柄失效")?;
            let cursor = vehicle.route_edge_index();
            let mut active = world.vehicle_route_handoffs(handle);
            let progress = active.next();
            if active.next().is_some() {
                return Err("同一车辆持有多条横移".into());
            }
            let phase = if let Some(progress) = progress {
                held += 1;
                if progress.source_index != input.source_index
                    || progress.front_handed_off != (cursor == input.source_index + 1)
                    || cursor < input.source_index
                    || cursor > input.source_index + 1
                {
                    return Err(format!(
                        "前杠交接阶段与路线出现不一致 slot={slot} tick={tick}"
                    ));
                }
                if last[slot] == 0 && progress.acquired_tick != tick {
                    return Err("首次观察到横移时授权拍号不符".into());
                }
                if progress.front_handed_off { 11 } else { 10 }
            } else if cursor > input.source_index {
                12
            } else {
                0
            };
            if phase == last[slot] {
                continue;
            }
            if !matches!((last[slot], phase), (0, 10) | (10, 11) | (11, 12))
                || slot != first && last[first] != 12
            {
                return Err(format!(
                    "横移阶段或两车竞争顺序错误 slot={slot} tick={tick} {}→{phase}",
                    last[slot]
                ));
            }
            last[slot] = phase;
            journal.push((tick, slot as u16, phase, record.route_rank));
        }
        if held > 1 {
            return Err(format!(
                "两车同时持有相交的中道换道预约 cluster={cluster} tick={tick}"
            ));
        }
    }
    Ok(())
}
