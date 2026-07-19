//! 目标 f32 生产数值判定的领域所有权。
//!
//! 产品范围与运行时绝对阈值由 #127 离线标定，并由 #144 原子启用。
//! 每个值只服务自己的领域判定；数值相同也不得互相别名。

/// `EdgeLength` 接受值的 exclusive 下限，单位为米。
pub(crate) const MIN_EDGE_LENGTH_EXCLUSIVE_METERS: f32 = 1.0;

/// 单个 edge 长度的 inclusive 上限，单位为米。
pub(crate) const MAX_EDGE_LENGTH_INCLUSIVE_METERS: f32 = 10_000.0;

/// Vehicle Profile 车辆长度的 inclusive 下限，单位为米。
pub(crate) const MIN_VEHICLE_LENGTH_INCLUSIVE_METERS: f32 = 0.1;

/// 车辆尺寸、Parking extent/min-gap 和局部偏移的 inclusive 上限，单位为米。
pub(crate) const MAX_LOCAL_EXTENT_OR_OFFSET_INCLUSIVE_METERS: f32 = 128.0;

/// 速度输入的 inclusive 上限，单位为米/秒。
pub(crate) const MAX_SPEED_INCLUSIVE_METERS_PER_SECOND: f32 = 100.0;

/// Vehicle Profile 加速度/减速度输入的 inclusive 上限，单位为米/秒²。
pub(crate) const MAX_PROFILE_ACCELERATION_INCLUSIVE_METERS_PER_SECOND_SQUARED: f32 = 50.0;

/// Vehicle Profile time headway 输入的 inclusive 上限，单位为秒。
pub(crate) const MAX_TIME_HEADWAY_INCLUSIVE_SECONDS: f32 = 60.0;

/// Parking anchor 与 edge 两端之间的最小留白，单位为米。
pub(crate) const PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS: f64 = 0.000_05;

/// Parking lateral offset 非零绝对值的 exclusive 下限，单位为米。
pub(crate) const MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS: f32 = 0.0;

/// Parking length/width 的 inclusive 下限，单位为米。
pub(crate) const MIN_PARKING_EXTENT_INCLUSIVE_METERS: f32 = 0.1;

/// edge boundary、跨 edge 余量与吸附判定的绝对阈值，单位为米。
pub(crate) const EDGE_BOUNDARY_TOLERANCE_METERS: f64 = 0.000_000_01;

/// RouteEnd、SignalStop、ParkingStop 等纵向约束判定的绝对阈值，单位为米。
pub(crate) const LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS: f64 = 0.000_05;

/// 物理 bumper gap、接触与重叠判定的绝对阈值，单位为米。
pub(crate) const PHYSICAL_GAP_TOLERANCE_METERS: f64 = 0.000_01;

/// 运行时计算速度的 near-zero 判定阈值，单位为米/秒。
pub(crate) const COMPUTED_SPEED_NEAR_ZERO_TOLERANCE_METERS_PER_SECOND: f64 = 0.000_05;

/// 判断 edge traversal 余量是否应按零处理。
pub(crate) fn is_edge_boundary_remainder_zero(remainder_meters: f64) -> bool {
    remainder_meters < EDGE_BOUNDARY_TOLERANCE_METERS
}

/// 判断纵向行程是否已到达约束距离。
pub(crate) fn longitudinal_constraint_reached(
    travel_meters: f64,
    constraint_distance_meters: f64,
) -> bool {
    travel_meters + LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS >= constraint_distance_meters
}

/// 判断两个 edge-local 纵向位置是否命中同一约束位置。
pub(crate) fn longitudinal_positions_match(left_meters: f64, right_meters: f64) -> bool {
    (left_meters - right_meters).abs() <= LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS
}

/// 判断物理 bumper gap 是否为接触或重叠。
pub(crate) fn physical_gap_is_zero_or_overlap(bumper_gap_meters: f64) -> bool {
    bumper_gap_meters <= PHYSICAL_GAP_TOLERANCE_METERS
}

/// 判断物理 bumper gap 是否超过允许的重叠阈值。
pub(crate) fn physical_gap_is_overlap(bumper_gap_meters: f64) -> bool {
    bumper_gap_meters < -PHYSICAL_GAP_TOLERANCE_METERS
}

/// 把物理接触阈值内的正负 gap 规范化为正零。
pub(crate) fn normalize_physical_gap(bumper_gap_meters: f64) -> f64 {
    if bumper_gap_meters.abs() <= PHYSICAL_GAP_TOLERANCE_METERS {
        0.0
    } else {
        bumper_gap_meters
    }
}

/// 判断计算得到的速度是否属于已有的 near-zero 语义。
pub(crate) fn computed_speed_is_near_zero(speed_meters_per_second: f64) -> bool {
    speed_meters_per_second <= COMPUTED_SPEED_NEAR_ZERO_TOLERANCE_METERS_PER_SECOND
}

/// 判断计算得到的速度是否严格高于已有的 near-zero 语义。
pub(crate) fn computed_speed_is_above_near_zero(speed_meters_per_second: f64) -> bool {
    speed_meters_per_second > COMPUTED_SPEED_NEAR_ZERO_TOLERANCE_METERS_PER_SECOND
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_f32_domain_values_remain_frozen() {
        assert_eq!(MIN_EDGE_LENGTH_EXCLUSIVE_METERS, 1.0);
        assert_eq!(MAX_EDGE_LENGTH_INCLUSIVE_METERS, 10_000.0);
        assert_eq!(MIN_VEHICLE_LENGTH_INCLUSIVE_METERS, 0.1);
        assert_eq!(MAX_LOCAL_EXTENT_OR_OFFSET_INCLUSIVE_METERS, 128.0);
        assert_eq!(MAX_SPEED_INCLUSIVE_METERS_PER_SECOND, 100.0);
        assert_eq!(
            MAX_PROFILE_ACCELERATION_INCLUSIVE_METERS_PER_SECOND_SQUARED,
            50.0
        );
        assert_eq!(MAX_TIME_HEADWAY_INCLUSIVE_SECONDS, 60.0);
        assert_eq!(PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS, 0.000_05);
        assert_eq!(MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS, 0.0);
        assert_eq!(MIN_PARKING_EXTENT_INCLUSIVE_METERS, 0.1);
        assert_eq!(EDGE_BOUNDARY_TOLERANCE_METERS, 0.000_000_01);
        assert_eq!(LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS, 0.000_05);
        assert_eq!(PHYSICAL_GAP_TOLERANCE_METERS, 0.000_01);
        assert_eq!(
            COMPUTED_SPEED_NEAR_ZERO_TOLERANCE_METERS_PER_SECOND,
            0.000_05
        );
    }

    #[test]
    fn edge_boundary_remainder_uses_strict_threshold() {
        assert!(is_edge_boundary_remainder_zero(
            EDGE_BOUNDARY_TOLERANCE_METERS.next_down()
        ));
        assert!(!is_edge_boundary_remainder_zero(
            EDGE_BOUNDARY_TOLERANCE_METERS
        ));
        assert!(!is_edge_boundary_remainder_zero(
            EDGE_BOUNDARY_TOLERANCE_METERS.next_up()
        ));
    }

    #[test]
    fn longitudinal_constraint_includes_exact_threshold() {
        assert!(longitudinal_constraint_reached(
            0.0,
            LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS.next_down()
        ));
        assert!(longitudinal_constraint_reached(
            0.0,
            LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS
        ));
        assert!(!longitudinal_constraint_reached(
            0.0,
            LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS.next_up()
        ));
        assert!(longitudinal_positions_match(
            0.0,
            LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS
        ));
        assert!(!longitudinal_positions_match(
            0.0,
            LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS.next_up()
        ));
    }

    #[test]
    fn physical_gap_keeps_contact_and_overlap_boundaries_distinct() {
        assert!(physical_gap_is_zero_or_overlap(
            PHYSICAL_GAP_TOLERANCE_METERS
        ));
        assert!(!physical_gap_is_zero_or_overlap(
            PHYSICAL_GAP_TOLERANCE_METERS.next_up()
        ));
        assert!(!physical_gap_is_overlap(
            (-PHYSICAL_GAP_TOLERANCE_METERS).next_up()
        ));
        assert!(!physical_gap_is_overlap(-PHYSICAL_GAP_TOLERANCE_METERS));
        assert!(physical_gap_is_overlap(
            (-PHYSICAL_GAP_TOLERANCE_METERS).next_down()
        ));
        assert_eq!(normalize_physical_gap(PHYSICAL_GAP_TOLERANCE_METERS), 0.0);
        assert_eq!(normalize_physical_gap(-PHYSICAL_GAP_TOLERANCE_METERS), 0.0);
        assert_ne!(
            normalize_physical_gap(PHYSICAL_GAP_TOLERANCE_METERS.next_up()),
            0.0
        );
    }

    #[test]
    fn computed_speed_near_zero_includes_exact_threshold() {
        assert!(computed_speed_is_near_zero(0.0));
        assert!(computed_speed_is_near_zero(-0.0));
        assert!(computed_speed_is_near_zero(
            COMPUTED_SPEED_NEAR_ZERO_TOLERANCE_METERS_PER_SECOND.next_down()
        ));
        assert!(computed_speed_is_near_zero(
            COMPUTED_SPEED_NEAR_ZERO_TOLERANCE_METERS_PER_SECOND
        ));
        assert!(!computed_speed_is_near_zero(
            COMPUTED_SPEED_NEAR_ZERO_TOLERANCE_METERS_PER_SECOND.next_up()
        ));
        assert!(computed_speed_is_above_near_zero(
            COMPUTED_SPEED_NEAR_ZERO_TOLERANCE_METERS_PER_SECOND.next_up()
        ));
        assert!(!computed_speed_is_above_near_zero(
            COMPUTED_SPEED_NEAR_ZERO_TOLERANCE_METERS_PER_SECOND
        ));
    }
}
