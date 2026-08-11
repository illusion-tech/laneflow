//! 道路几何编译使用的封闭配置档。
//!
//! 配置档属于来源格式无关的 compiler API。道路编辑 writer、受检 reader 和后续
//! geometry lowering 共享同一组类型，避免每种来源格式各自定义一套语义相同的档位。

/// Authoring 曲线到规范运行时折线的总位置误差配置档。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum GeometryAccuracyProfile {
    Fine2Cm = 1,
    Balanced5Cm = 2,
    Compact10Cm = 3,
}

impl GeometryAccuracyProfile {
    /// 返回进入描述符、诊断与校准工件的稳定 ASCII 名称。
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Fine2Cm => "fine-2cm-v1",
            Self::Balanced5Cm => "balanced-5cm-v1",
            Self::Compact10Cm => "compact-10cm-v1",
        }
    }

    /// 返回 authoring/offset evaluator 到最终规范折线的总位置误差目标。
    #[must_use]
    pub const fn max_position_error_meters(self) -> f64 {
        match self {
            Self::Fine2Cm => 0.02,
            Self::Balanced5Cm => 0.05,
            Self::Compact10Cm => 0.10,
        }
    }
}

/// 最终规范 `f32` 折线的方向跳变配置档。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum GeometryDirectionProfile {
    Smooth1Deg = 1,
    Balanced2Deg = 2,
    Compact5Deg = 3,
}

impl GeometryDirectionProfile {
    /// 返回进入描述符、诊断与校准工件的稳定 ASCII 名称。
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Smooth1Deg => "smooth-1deg-v1",
            Self::Balanced2Deg => "balanced-2deg-v1",
            Self::Compact5Deg => "compact-5deg-v1",
        }
    }

    /// 返回最终规范 `f32` 相邻弦和相连 edge 首尾弦允许的最大方向跳变。
    #[must_use]
    pub const fn max_runtime_direction_jump_degrees(self) -> f64 {
        match self {
            Self::Smooth1Deg => 1.0,
            Self::Balanced2Deg => 2.0,
            Self::Compact5Deg => 5.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_profiles_keep_the_frozen_public_values() {
        assert_eq!(
            GeometryAccuracyProfile::Fine2Cm.stable_name(),
            "fine-2cm-v1"
        );
        assert_eq!(
            GeometryAccuracyProfile::Fine2Cm.max_position_error_meters(),
            0.02
        );
        assert_eq!(
            GeometryAccuracyProfile::Balanced5Cm.max_position_error_meters(),
            0.05
        );
        assert_eq!(
            GeometryAccuracyProfile::Compact10Cm.max_position_error_meters(),
            0.10
        );

        assert_eq!(
            GeometryDirectionProfile::Smooth1Deg.stable_name(),
            "smooth-1deg-v1"
        );
        assert_eq!(
            GeometryDirectionProfile::Smooth1Deg.max_runtime_direction_jump_degrees(),
            1.0
        );
        assert_eq!(
            GeometryDirectionProfile::Balanced2Deg.max_runtime_direction_jump_degrees(),
            2.0
        );
        assert_eq!(
            GeometryDirectionProfile::Compact5Deg.max_runtime_direction_jump_degrees(),
            5.0
        );
    }
}
