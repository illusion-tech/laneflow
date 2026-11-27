//! 标准位姿类型与共享几何常量。

pub use laneflow_static_contract::{
    SPATIAL_JOIN_POSITION_TOLERANCE_METERS, SPATIAL_LENGTH_ABS_TOLERANCE_METERS,
    SPATIAL_LENGTH_REL_TOLERANCE, SPATIAL_MIN_PROJECTED_UP_LENGTH,
    SPATIAL_MIN_SEGMENT_LENGTH_METERS,
};

use crate::{CanonicalPoint3F32, CanonicalUnitVector3F32};

/// canonical frame 中的采样位姿。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalPoseF32 {
    position: CanonicalPoint3F32,
    tangent: CanonicalUnitVector3F32,
    up: CanonicalUnitVector3F32,
}

impl CanonicalPoseF32 {
    pub(crate) const fn from_parts(
        position: CanonicalPoint3F32,
        tangent: CanonicalUnitVector3F32,
        up: CanonicalUnitVector3F32,
    ) -> Self {
        Self {
            position,
            tangent,
            up,
        }
    }

    /// 返回采样位置，单位为米。
    pub const fn position(self) -> CanonicalPoint3F32 {
        self.position
    }

    /// 返回行驶方向的单位向量。横移采用车体纵轴。
    pub const fn tangent(self) -> CanonicalUnitVector3F32 {
        self.tangent
    }

    /// 返回单位上向。横移采用有滚转的车体上向，普通车道采用规范上向的正交投影。
    pub const fn up(self) -> CanonicalUnitVector3F32 {
        self.up
    }
}
