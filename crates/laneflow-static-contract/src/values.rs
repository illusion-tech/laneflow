//! 编译器、制品验证器与运行时共享的静态值和数值边界。
//!
//! 本模块只定义不依赖存储布局的封闭值域；它不解释运行时通行权，也不持有控制器
//! 状态。制品编码仍须由对应格式版本显式规定，不能把 Rust 判别值直接当作线格式。

/// 固定时制信号相位对一个 `SignalGroup` 给出的灯色指示。
///
/// 该值只表达信号指示，不是交通参与单元的最终通行权。`Yellow` 在 current
/// protected-only compliance profile 中按限制性黄灯解释，但该行为策略不属于本值类型。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum SignalAspect {
    /// 红灯指示。
    Red,
    /// 黄灯指示。
    Yellow,
    /// 绿灯指示。
    Green,
}

/// 静态准入规则对匹配交通参与单元给出的平面内效果。
///
/// `Allow` 只在同一准入平面内参与更具体规则的豁免裁决，不能解除另一平面的
/// `Deny`，也不能覆盖运行时安全约束。该值不携带优先级、参与者类别或目标信息。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum AccessEffect {
    /// 显式放行。
    Allow,
    /// 显式拒绝。
    Deny,
}

/// 历史米制哨兵：停车锚点距边端必须严格大于的距离。
///
/// G2 起生产判定使用 [`PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM`]；编制 LIR 仍可读本常量。
pub const PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS: f64 = 1.0e-9;

/// 停车锚点距量化后边端的留白，单位为毫米；与最短边长不是同一个常量。
pub const PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM: u32 = 1;

/// 历史米制哨兵：停车位横向偏移绝对值必须严格大于的距离。
pub const MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS: f64 = 1.0e-9;

/// 路外停车横向偏移绝对值下限（含），单位为毫米。
pub const MIN_PARKING_LATERAL_OFFSET_ABS_MM: u32 = 1;

/// 停车横向偏移绝对值上限（含），单位为毫米。
pub const MAX_PARKING_LATERAL_OFFSET_ABS_MM: u32 = 128_000;

/// 历史米制哨兵：停车位长度和宽度必须严格大于的距离。
pub const MIN_PARKING_EXTENT_EXCLUSIVE_METERS: f64 = 1.0e-9;

/// 停车长宽与车长下限（含），单位为毫米。
pub const MIN_VEHICLE_LENGTH_MM: u32 = 100;

/// 停车长宽与车长上限（含），单位为毫米。
pub const MAX_VEHICLE_LENGTH_MM: u32 = 128_000;

/// 历史米制哨兵：车辆长度必须严格大于的距离。
pub const MIN_VEHICLE_LENGTH_EXCLUSIVE_METERS: f64 = 1.0e-9;

/// 交通边最短长度（含），单位为毫米。
pub const MIN_LANE_EDGE_LENGTH_MM: u32 = 100;

/// 交通边最长长度（含），单位为毫米。
pub const MAX_LANE_EDGE_LENGTH_MM: u32 = 10_000_000;

/// 边限速与期望车速下限（含），单位为毫米每秒；已提交静止速度可为 0。
pub const MIN_SPEED_MM_S: u32 = 1;

/// 边限速与期望车速上限（含），单位为毫米每秒。
pub const MAX_SPEED_MM_S: u32 = 100_000;

/// `min_gap` 上限（含），单位为毫米；0 合法。
pub const MAX_MIN_GAP_MM: u32 = 128_000;

/// 时距排他下限对应的受检 `f32` 上界（含），单位为秒。
pub const MAX_TIME_HEADWAY_SECONDS: f32 = 60.0;

/// 加速度/减速度下限（含），单位为米每二次方秒。
pub const MIN_ACCEL_METERS_PER_SECOND_SQUARED: f32 = 0.5;

/// 加速度/减速度上限（含），单位为米每二次方秒。
pub const MAX_ACCEL_METERS_PER_SECOND_SQUARED: f32 = 50.0;

/// 编制/发射量化后折成 `-π` 的 binary32 位型。
pub const HEADING_PLUS_PI_F32_BITS: u32 = 0x4049_0fdb;

/// 合法朝向闭包的 `-π` binary32 位型。
pub const HEADING_MINUS_PI_F32_BITS: u32 = 0xc049_0fdb;

/// canonical frame 中点分量允许的最小值，单位为米。
pub const CANONICAL_POINT_COMPONENT_MIN_METERS: f32 = -16_384.0;

/// canonical frame 中点分量允许的最大值，单位为米。
pub const CANONICAL_POINT_COMPONENT_MAX_METERS: f32 = 16_384.0;

/// 规范中心线线段允许的最小长度，单位为米；有效线段必须严格大于该值。
pub const SPATIAL_MIN_SEGMENT_LENGTH_METERS: f32 = 0.1;

/// 交通权威长度与几何弧长绑定的绝对容差下限，单位为米。
pub const SPATIAL_LENGTH_ABS_TOLERANCE_METERS: f64 = 0.01;

/// 交通权威长度与几何弧长绑定的相对容差系数。
pub const SPATIAL_LENGTH_REL_TOLERANCE: f64 = 1.0e-6;

/// current-f64 交通边长度的量化余量，单位为米。
pub const SPATIAL_CORE_LENGTH_QUANTIZATION_ALLOWANCE_METERS: f64 = 0.0;

/// 已连接车道图边端点允许的最大距离，单位为米。
pub const SPATIAL_JOIN_POSITION_TOLERANCE_METERS: f32 = 0.005;

/// canonical `+Y` 投影长度允许的最小值；等于该值时有效。
pub const SPATIAL_MIN_PROJECTED_UP_LENGTH: f32 = 0.008_726_535;

/// 停车位朝向偏移的包含下界，单位为弧度。
pub const PARKING_HEADING_OFFSET_MINIMUM_RADIANS: f64 = -core::f64::consts::PI;

/// 停车位朝向偏移的排他上界，单位为弧度。
pub const PARKING_HEADING_OFFSET_MAXIMUM_RADIANS: f64 = core::f64::consts::PI;
