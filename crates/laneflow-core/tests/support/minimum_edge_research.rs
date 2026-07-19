//! #127 最小 edge 长度候选的转换压力研究助手。
//!
//! 生产 Core 固定样例负责单次跨界事件语义；本模块的紧凑内核只逐次执行多 edge
//! 比较、相减和计数，不分配海量事件，因此只用于放大不可回避的跨界工作量。

pub const MAX_TICK_TRAVEL_METERS: f32 = 100.0;
pub const MIN_EDGE_CANDIDATES_METERS: [f64; 4] = [0.0, 0.01, 0.1, 1.0];
pub const SELECTED_MIN_EDGE_EXCLUSIVE_METERS: f64 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionPressureEstimate {
    pub min_exclusive_meters: f64,
    pub first_valid_edge_length_meters: f32,
    pub crossings_per_vehicle: Option<u64>,
    pub crossings_10k: Option<u128>,
    pub crossings_100k: Option<u128>,
}

pub fn first_valid_edge_length(min_exclusive_meters: f64) -> f32 {
    let rounded = min_exclusive_meters as f32;
    if f64::from(rounded) > min_exclusive_meters {
        rounded
    } else {
        rounded.next_up()
    }
}

pub fn transition_pressure_estimate(min_exclusive_meters: f64) -> TransitionPressureEstimate {
    let edge_length = first_valid_edge_length(min_exclusive_meters);
    let crossings = (f64::from(MAX_TICK_TRAVEL_METERS) / f64::from(edge_length)).floor();
    let crossings_per_vehicle = (crossings <= u64::MAX as f64).then_some(crossings as u64);
    TransitionPressureEstimate {
        min_exclusive_meters,
        first_valid_edge_length_meters: edge_length,
        crossings_per_vehicle,
        crossings_10k: crossings_per_vehicle.map(|value| u128::from(value) * 10_000),
        crossings_100k: crossings_per_vehicle.map(|value| u128::from(value) * 100_000),
    }
}

pub fn compact_transition_kernel(vehicle_count: usize, min_exclusive_meters: f64) -> (u128, f64) {
    let edge_length = first_valid_edge_length(min_exclusive_meters);
    assert!(
        min_exclusive_meters > 0.0,
        "exact-positive control has no executable bounded crossing count",
    );
    let mut total_crossings = 0_u128;
    let mut remainder_checksum = 0.0_f64;
    for vehicle_index in 0..vehicle_count {
        let phase = (vehicle_index % 17) as f32 / 32.0;
        let mut remaining = MAX_TICK_TRAVEL_METERS + phase * edge_length;
        while remaining >= edge_length {
            remaining -= edge_length;
            total_crossings += 1;
        }
        remainder_checksum += f64::from(remaining);
    }
    (total_crossings, remainder_checksum)
}
