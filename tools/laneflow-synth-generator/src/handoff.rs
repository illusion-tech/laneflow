//! L1 同拍竞争的申请与阶段记录。只读取逻辑槽位及已提交事实。
use crate::{InitialState, VehicleRecord};

mod history;
pub use history::check_history;

/// 车辆已经持有的横移预约。
#[derive(Clone, Copy, Debug)]
pub struct Reservation {
    pub source_index: u32,
    pub acquired_tick: u64,
    pub front_handed_off: bool,
}

/// 按逻辑车辆、指定路线读出的拍初游标、毫米前杠和预约。
#[derive(Clone, Copy, Debug)]
pub struct VehicleObservation {
    pub cursor: u32,
    pub front_mm: u32,
    pub handoff: Option<Reservation>,
}

/// 一次 L1 申请的逻辑身份与候选。运行侧将逻辑身份映射为本世界的句柄。
pub struct Request {
    pub vehicle_rank: usize,
    pub route_rank: usize,
    pub source_index: u32,
    pub station_window_mm: [u64; 2],
    pub rear_stations_mm: [u64; 2],
    pub speed_mm_s: u32,
}

fn clusters(state: &InitialState, handles_len: usize) -> Result<&[[VehicleRecord; 2]], String> {
    let params = state.identity.lane_change().ok_or("无换道族参数")?;
    params.validate().map_err(|e| e.to_string())?;
    let records = state
        .vehicles
        .get(state.identity.scale as usize..)
        .ok_or("换道车辆缺失")?;
    let (pairs, remainder) = records.as_chunks::<2>();
    if pairs.len() != params.clusters as usize
        || !remainder.is_empty()
        || handles_len != state.vehicles.len()
    {
        return Err("换道簇逻辑槽位不完整".into());
    }
    Ok(pairs)
}

/// 首拍两车同投。授予后下一拍检验排他性，随后待占用清空时按最新前杠重提。
/// 读回函数以车辆逻辑下标与预期路线 rank 为输入。构造函数直接返回调用方的请求类型。
///
/// # Errors
///
/// 输入槽位、已提交车辆、路线窗口或位移不可用时返回错误。
pub fn requests<T>(
    state: &InitialState,
    tick: u64,
    handles_len: usize,
    mut read: impl FnMut(usize, u32) -> Result<VehicleObservation, String>,
    mut build: impl FnMut(Request) -> Result<T, String>,
) -> Result<Vec<T>, String> {
    if state.identity.lane_change().is_none() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    for (cluster, pair) in clusters(state, handles_len)?.iter().enumerate() {
        let first = state.identity.scale as usize + cluster * 2;
        // 授予后的额外一拍已过，首车仍占用时整簇无需读第二辆车。
        let leader = read(first, pair[0].route_rank)?;
        if leader.handoff.is_some_and(|h| h.acquired_tick < tick) {
            continue;
        }
        let follower = read(first + 1, pair[1].route_rank)?;
        if follower.handoff.is_some_and(|h| h.acquired_tick < tick) {
            continue;
        }
        for (slot, (record, vehicle)) in pair.iter().zip([leader, follower]).enumerate() {
            if vehicle.handoff.is_some() {
                continue;
            }
            let route_rank = record.route_rank as usize;
            let input = state
                .routes
                .get(route_rank)
                .and_then(|r| r.handoff.as_ref())
                .ok_or("缺少换道窗口")?;
            if vehicle.cursor != input.source_index {
                continue;
            }
            let offset = u64::from(
                vehicle
                    .front_mm
                    .checked_sub(record.progress_mm)
                    .ok_or("换道前杠退至出生位之前")?,
            );
            let [lo, hi] = input.rear_stations_mm;
            let rear = [
                lo.checked_add(offset).ok_or("后轴起点溢出")?,
                hi.checked_add(offset).ok_or("后轴终点溢出")?,
            ];
            if rear[1] <= input.station_window_mm[1] {
                result.push(build(Request {
                    vehicle_rank: first + slot,
                    route_rank,
                    source_index: input.source_index,
                    station_window_mm: input.station_window_mm,
                    rear_stations_mm: rear,
                    speed_mm_s: input.speed_mm_s,
                })?);
            }
        }
    }
    Ok(result)
}

/// 按逻辑槽位核对进入、前杠交接及车尾清空。第四列保存输入中的路线 rank。
///
/// # Errors
///
/// 输入与已提交游标、授权拍、互斥或进入顺序不符时返回错误。
pub fn observe(
    state: &InitialState,
    tick: u64,
    handles_len: usize,
    read: impl FnMut(usize, u32) -> Result<VehicleObservation, String>,
    last: &mut [u32],
    journal: &mut Vec<(u64, u16, u32, u32)>,
) -> Result<(), String> {
    let start = journal.len();
    if let Err(error) = record_phases(state, tick, handles_len, read, last, journal) {
        journal.truncate(start);
        return Err(error);
    }
    for &(_, slot, phase, _) in &journal[start..] {
        last[usize::from(slot)] = phase;
    }
    Ok(())
}

fn record_phases(
    state: &InitialState,
    tick: u64,
    handles_len: usize,
    mut read: impl FnMut(usize, u32) -> Result<VehicleObservation, String>,
    last: &[u32],
    journal: &mut Vec<(u64, u16, u32, u32)>,
) -> Result<(), String> {
    let clusters = clusters(state, handles_len)?;
    if last.len() != 2 * clusters.len() {
        return Err("换道观测槽位不完整".into());
    }
    for (cluster, pair) in clusters.iter().enumerate() {
        let first = cluster * 2;
        let mut held = false;
        let mut leader_phase = last[first];
        for (offset, record) in pair.iter().enumerate() {
            let slot = first + offset;
            let input = state
                .routes
                .get(record.route_rank as usize)
                .and_then(|r| r.handoff.as_ref())
                .ok_or("缺少换道观测窗口")?;
            let vehicle = read(state.identity.scale as usize + slot, record.route_rank)?;
            let phase = if let Some(handoff) = vehicle.handoff {
                if held {
                    return Err(format!(
                        "两车同时持有相交的中道换道预约 cluster={cluster} tick={tick}"
                    ));
                }
                held = true;
                let target = input
                    .source_index
                    .checked_add(1)
                    .ok_or("横移目标出现溢出")?;
                if handoff.source_index != input.source_index
                    || handoff.front_handed_off != (vehicle.cursor == target)
                    || !(input.source_index..=target).contains(&vehicle.cursor)
                {
                    return Err(format!(
                        "前杠交接阶段与路线出现不一致 slot={slot} tick={tick}"
                    ));
                }
                if last[slot] == 0 && handoff.acquired_tick != tick {
                    return Err("首次观察到横移时授权拍号不符".into());
                }
                if handoff.front_handed_off { 11 } else { 10 }
            } else if vehicle.cursor > input.source_index {
                12
            } else {
                0
            };
            if phase == last[slot] {
                continue;
            }
            if !matches!((last[slot], phase), (0, 10) | (10, 11) | (11, 12))
                || offset != 0 && leader_phase != 12
            {
                return Err(format!(
                    "横移阶段或两车竞争顺序错误 slot={slot} tick={tick} {}→{phase}",
                    last[slot]
                ));
            }
            if offset == 0 {
                leader_phase = phase;
            }
            journal.push((tick, slot as u16, phase, record.route_rank));
        }
    }
    Ok(())
}
