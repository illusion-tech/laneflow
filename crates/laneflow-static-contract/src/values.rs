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

/// 停车锚点距 `LaneEdge` 两端必须严格大于的距离，单位为米。
///
/// 该排他边界避免入口/出口落在拓扑连接点上，从而让锚点始终唯一属于一条边。
pub const PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS: f64 = 1.0e-9;

/// 停车位横向偏移绝对值必须严格大于的距离，单位为米。
///
/// 零偏移及数值噪声范围会把停车位中心放回入口边中心线，不能表达受支持的路外泊位。
pub const MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS: f64 = 1.0e-9;

/// 停车位长度和宽度必须严格大于的距离，单位为米。
pub const MIN_PARKING_EXTENT_EXCLUSIVE_METERS: f64 = 1.0e-9;

/// 停车位朝向偏移的包含下界，单位为弧度。
pub const PARKING_HEADING_OFFSET_MINIMUM_RADIANS: f64 = -core::f64::consts::PI;

/// 停车位朝向偏移的排他上界，单位为弧度。
pub const PARKING_HEADING_OFFSET_MAXIMUM_RADIANS: f64 = core::f64::consts::PI;
