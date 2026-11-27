//! LaneFlow-owned 标准空间原语。

pub use laneflow_static_contract::{
    CANONICAL_POINT_COMPONENT_MAX_METERS, CANONICAL_POINT_COMPONENT_MIN_METERS,
};

use crate::{SpatialAxis, SpatialError};

/// 调用方分配的不透明 pose 记录身份。Spatial 不解释为车辆。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PoseRecordId(u32);

/// Adapter 为 canonical frame 的某次宿主放置颁发的不透明等值 token。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FramePlacementToken(u64);

impl FramePlacementToken {
    /// 从调用方拥有的稳定值创建 token。
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// 返回底层稳定数值。
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl PoseRecordId {
    /// 从调用方拥有的稳定值创建记录身份。
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// 返回底层稳定数值。
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

const POINT_VALUE_KIND: &str = "CanonicalPoint3F32";
const VECTOR_VALUE_KIND: &str = "CanonicalVector3F32";

/// LaneFlow 有界 canonical frame 中的三维 `f32` 点，单位为米。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalPoint3F32 {
    x: f32,
    y: f32,
    z: f32,
}

impl CanonicalPoint3F32 {
    /// 创建有限且每轴位于 `[-16_384, 16_384] m` 的点，并规范化带符号零。
    ///
    /// # Errors
    ///
    /// 任一分量非有限或超出 `[-16_384, 16_384] m` 时返回相应 [`SpatialError`]；
    /// 带符号零被规范化后构造。
    pub fn try_new(x: f32, y: f32, z: f32) -> Result<Self, SpatialError> {
        let [x, y, z] = checked_point_components(x, y, z)?;
        Ok(Self { x, y, z })
    }

    /// 返回 X 坐标，单位为米。
    pub const fn x(self) -> f32 {
        self.x
    }

    /// 返回 Y 坐标，单位为米。
    pub const fn y(self) -> f32 {
        self.y
    }

    /// 返回 Z 坐标，单位为米。
    pub const fn z(self) -> f32 {
        self.z
    }

    /// 受检地把标准向量加到本点。
    ///
    /// # Errors
    ///
    /// 结果点任一分量非有限或超出 `[-16_384, 16_384] m` 时返回相应 [`SpatialError`]。
    pub fn checked_add_vector(self, vector: CanonicalVector3F32) -> Result<Self, SpatialError> {
        Self::try_new(self.x + vector.x, self.y + vector.y, self.z + vector.z)
    }

    /// 受检地从本点减去标准向量。
    ///
    /// # Errors
    ///
    /// 结果点任一分量非有限或超出 `[-16_384, 16_384] m` 时返回相应 [`SpatialError`]。
    pub fn checked_sub_vector(self, vector: CanonicalVector3F32) -> Result<Self, SpatialError> {
        Self::try_new(self.x - vector.x, self.y - vector.y, self.z - vector.z)
    }

    /// 受检地计算从本点指向目标点的标准向量。
    ///
    /// # Errors
    ///
    /// 两个端点均由 [`CanonicalPoint3F32::try_new`] 保证有限且在规范范围内，差
    /// 向量必然有限，本方法当前不返回 `Err`；保留 `Result` 形态以与受检家族一致。
    pub fn checked_vector_to(self, target: Self) -> Result<CanonicalVector3F32, SpatialError> {
        CanonicalVector3F32::try_new(target.x - self.x, target.y - self.y, target.z - self.z)
    }
}

/// LaneFlow canonical frame 中的三维有限 `f32` 向量。
///
/// 向量不套用点的每轴范围；两个合法端点的差可以达到 `32_768 m`。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalVector3F32 {
    x: f32,
    y: f32,
    z: f32,
}

impl CanonicalVector3F32 {
    /// 创建有限的标准向量，并把所有 `-0.0` 规范化为 `+0.0`。
    ///
    /// # Errors
    ///
    /// 任一分量非有限时返回 [`SpatialError`]；`-0.0` 规范化为 `+0.0` 后构造。
    pub fn try_new(x: f32, y: f32, z: f32) -> Result<Self, SpatialError> {
        let [x, y, z] = checked_components(VECTOR_VALUE_KIND, x, y, z)?;
        Ok(Self { x, y, z })
    }

    /// 返回 X 分量。
    pub const fn x(self) -> f32 {
        self.x
    }

    /// 返回 Y 分量。
    pub const fn y(self) -> f32 {
        self.y
    }

    /// 返回 Z 分量。
    pub const fn z(self) -> f32 {
        self.z
    }

    /// 受检地计算向量和。
    ///
    /// # Errors
    ///
    /// 结果任一分量非有限时返回 [`SpatialError`]。
    pub fn checked_add(self, other: Self) -> Result<Self, SpatialError> {
        Self::try_new(self.x + other.x, self.y + other.y, self.z + other.z)
    }

    /// 受检地计算向量差。
    ///
    /// # Errors
    ///
    /// 结果任一分量非有限时返回 [`SpatialError`]。
    pub fn checked_sub(self, other: Self) -> Result<Self, SpatialError> {
        Self::try_new(self.x - other.x, self.y - other.y, self.z - other.z)
    }

    /// 受检地按标量缩放向量。
    ///
    /// # Errors
    ///
    /// 结果任一分量非有限时返回 [`SpatialError`]。
    pub fn checked_scale(self, scale: f32) -> Result<Self, SpatialError> {
        Self::try_new(self.x * scale, self.y * scale, self.z * scale)
    }

    /// 受检地把非零向量归一化为单位方向。
    ///
    /// # Errors
    ///
    /// 向量为零向量时返回 [`SpatialError::ZeroLengthDirection`]。
    pub fn try_normalize(self) -> Result<CanonicalUnitVector3F32, SpatialError> {
        CanonicalUnitVector3F32::try_from_vector(self)
    }
}

/// LaneFlow canonical frame 中的三维 `f32` 单位方向。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalUnitVector3F32(CanonicalVector3F32);

impl CanonicalUnitVector3F32 {
    /// 从有限非零向量创建单位方向。
    ///
    /// 归一化先按最大绝对分量缩放，避免有限大分量在平方求和时溢出。
    ///
    /// # Errors
    ///
    /// 输入向量为零向量时返回 [`SpatialError::ZeroLengthDirection`]。
    pub fn try_from_vector(vector: CanonicalVector3F32) -> Result<Self, SpatialError> {
        let scale = vector.x.abs().max(vector.y.abs()).max(vector.z.abs());
        if scale == 0.0 {
            return Err(SpatialError::ZeroLengthDirection);
        }

        let scaled_x = vector.x / scale;
        let scaled_y = vector.y / scale;
        let scaled_z = vector.z / scale;
        let scaled_length = scaled_x.hypot(scaled_y).hypot(scaled_z);
        let normalized = CanonicalVector3F32::try_new(
            scaled_x / scaled_length,
            scaled_y / scaled_length,
            scaled_z / scaled_length,
        )?;

        Ok(Self(normalized))
    }

    /// 返回 X 分量。
    pub const fn x(self) -> f32 {
        self.0.x
    }

    /// 返回 Y 分量。
    pub const fn y(self) -> f32 {
        self.0.y
    }

    /// 返回 Z 分量。
    pub const fn z(self) -> f32 {
        self.0.z
    }

    /// 返回同一方向的有限标准向量值。
    pub const fn as_vector(self) -> CanonicalVector3F32 {
        self.0
    }
}

fn checked_components(
    value_kind: &'static str,
    x: f32,
    y: f32,
    z: f32,
) -> Result<[f32; 3], SpatialError> {
    for (axis, value) in [
        (SpatialAxis::X, x),
        (SpatialAxis::Y, y),
        (SpatialAxis::Z, z),
    ] {
        if !value.is_finite() {
            return Err(SpatialError::NonFiniteComponent {
                value_kind,
                axis,
                value,
            });
        }
    }

    Ok([
        normalize_signed_zero(x),
        normalize_signed_zero(y),
        normalize_signed_zero(z),
    ])
}

fn checked_point_components(x: f32, y: f32, z: f32) -> Result<[f32; 3], SpatialError> {
    let components = checked_components(POINT_VALUE_KIND, x, y, z)?;
    for (axis, value) in [
        (SpatialAxis::X, components[0]),
        (SpatialAxis::Y, components[1]),
        (SpatialAxis::Z, components[2]),
    ] {
        if !(CANONICAL_POINT_COMPONENT_MIN_METERS..=CANONICAL_POINT_COMPONENT_MAX_METERS)
            .contains(&value)
        {
            return Err(SpatialError::PointComponentOutOfRange {
                axis,
                value,
                min: CANONICAL_POINT_COMPONENT_MIN_METERS,
                max: CANONICAL_POINT_COMPONENT_MAX_METERS,
            });
        }
    }

    Ok(components)
}

fn normalize_signed_zero(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}
