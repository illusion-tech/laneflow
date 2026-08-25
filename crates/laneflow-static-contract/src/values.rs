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

/// 停车锚点距量化后边端的留白，单位为毫米；与最短边长不是同一个常量。
pub const PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM: u32 = 1;

/// 路外停车横向偏移绝对值下限（含），单位为毫米。
pub const MIN_PARKING_LATERAL_OFFSET_ABS_MM: u32 = 1;

/// 停车横向偏移绝对值上限（含），单位为毫米。
pub const MAX_PARKING_LATERAL_OFFSET_ABS_MM: u32 = 128_000;

/// 停车长宽与车长下限（含），单位为毫米。
pub const MIN_VEHICLE_LENGTH_MM: u32 = 100;

/// 停车长宽与车长上限（含），单位为毫米。
pub const MAX_VEHICLE_LENGTH_MM: u32 = 128_000;

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

/// 交通边长与弧长对账不再另加量化余量；长度为独立整数毫米权威。
pub const SPATIAL_CORE_LENGTH_QUANTIZATION_ALLOWANCE_METERS: f64 = 0.0;

/// 已连接车道图边端点允许的最大距离，单位为米。
pub const SPATIAL_JOIN_POSITION_TOLERANCE_METERS: f32 = 0.005;

/// canonical `+Y` 投影长度允许的最小值；等于该值时有效。
pub const SPATIAL_MIN_PROJECTED_UP_LENGTH: f32 = 0.008_726_535;

/// 停车位朝向偏移的包含下界，单位为弧度。
pub const PARKING_HEADING_OFFSET_MINIMUM_RADIANS: f64 = -core::f64::consts::PI;

/// 停车位朝向偏移的排他上界，单位为弧度。
pub const PARKING_HEADING_OFFSET_MAXIMUM_RADIANS: f64 = core::f64::consts::PI;

/// 把编制 SI 米量化为毫米：×1000 后 `round_ties_even`。非有限、越出 `u32` 或为负时返回 `None`。
#[must_use]
pub fn millimetres_from_si(meters: f64) -> Option<u32> {
    u32::try_from(si_times_one_thousand(meters)?).ok()
}

/// 把编制 SI 米量化为有符号毫米，供停车横向偏移使用。
#[must_use]
pub fn millimetres_i32_from_si(meters: f64) -> Option<i32> {
    i32::try_from(si_times_one_thousand(meters)?).ok()
}

/// 编制朝向量化到 `f32`：`+π` 位型折成 `-π`，`-0.0` 折成 `+0.0`。非有限返回 `None`。
#[must_use]
pub fn heading_f32_from_si(radians: f64) -> Option<f32> {
    if !radians.is_finite() {
        return None;
    }
    let quantized = radians as f32;
    if !quantized.is_finite() {
        return None;
    }
    if quantized.to_bits() == HEADING_PLUS_PI_F32_BITS {
        return Some(f32::from_bits(HEADING_MINUS_PI_F32_BITS));
    }
    if quantized.to_bits() == (-0.0_f32).to_bits() {
        return Some(0.0);
    }
    Some(quantized)
}

/// 制品/热列朝向闭包：`-π <= x < π`。存着的 `+π` 位型非法。
#[must_use]
pub fn heading_f32_in_legal_closure(value: f32) -> bool {
    if !value.is_finite() || value.to_bits() == HEADING_PLUS_PI_F32_BITS {
        return false;
    }
    let min = f32::from_bits(HEADING_MINUS_PI_F32_BITS);
    let max = f32::from_bits(HEADING_PLUS_PI_F32_BITS);
    value >= min && value < max
}

fn si_times_one_thousand(value: f64) -> Option<i64> {
    if !value.is_finite() {
        return None;
    }
    let scaled = value * 1_000.0;
    if !scaled.is_finite() || scaled < i64::MIN as f64 || scaled > i64::MAX as f64 {
        return None;
    }
    Some(round_ties_even_to_i64(scaled))
}

fn round_ties_even_to_i64(value: f64) -> i64 {
    let truncated = value as i64;
    let fraction = value - truncated as f64;
    let abs_fraction = if fraction < 0.0 { -fraction } else { fraction };
    if abs_fraction > 0.5 || (abs_fraction == 0.5 && truncated % 2 != 0) {
        truncated + if value >= 0.0 { 1 } else { -1 }
    } else {
        truncated
    }
}

#[cfg(test)]
mod millimetre_quantize_tests {
    use super::*;

    #[test]
    fn rounds_length_ties_to_even() {
        assert_eq!(millimetres_from_si(0.099_6), Some(100));
        assert_eq!(millimetres_from_si(0.099_4), Some(99));
    }

    #[test]
    fn folds_plus_pi_heading_into_legal_closure() {
        let folded = heading_f32_from_si(f64::from(f32::from_bits(HEADING_PLUS_PI_F32_BITS)))
            .expect("finite heading");
        assert_eq!(folded.to_bits(), HEADING_MINUS_PI_F32_BITS);
        assert!(heading_f32_in_legal_closure(folded));
        assert!(!heading_f32_in_legal_closure(f32::from_bits(
            HEADING_PLUS_PI_F32_BITS
        )));
    }

    #[test]
    fn folds_negative_zero_heading_to_positive_zero() {
        let folded = heading_f32_from_si(-0.0).expect("finite heading");
        assert_eq!(folded.to_bits(), 0.0_f32.to_bits());
        assert!(heading_f32_in_legal_closure(folded));
    }
}
