use crate::{RigidTwoAxleGeometryInput, RoadVehicleGeometryInput};

/// 编译器单元测试共用的刚性双轴乘用车。
pub(crate) const CAR_GEOMETRY: RoadVehicleGeometryInput =
    RoadVehicleGeometryInput::RigidTwoAxle(RigidTwoAxleGeometryInput {
        wheelbase_m: 2.6,
        track_width_m: 1.6,
        reference_height_m: 0.35,
        rear_overhang_m: 0.75,
        width_m: 1.9,
        bottom_height_m: -0.15,
        top_height_m: 1.15,
        max_steering_angle_rad: 0.6,
        max_steering_rate_rad_s: 0.7,
    });
