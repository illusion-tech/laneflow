//! 道路机动车型的来源构型、SI 参数与两个编制前端共用的量化。

use crate::ScalarViolation;
use laneflow_static_contract::{
    RoadVehicleGeometry, RoadVehicleGeometryField, millimetres_from_si, millimetres_i32_from_si,
};

/// 来源声明的车型构型。首版规范几何只接纳刚性双轴。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RoadVehicleConfiguration {
    /// 单节刚性双轴机动车。
    RigidTwoAxle = 1,
    /// 牵引车与半挂车。
    SemiTrailer = 2,
    /// 牵引车与全挂车。
    FullTrailer = 3,
    /// 铰接公交车。
    ArticulatedBus = 4,
}

impl RoadVehicleConfiguration {
    /// 解析来源构型的一字节编码。未知编码没有对应的已登记构型。
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::RigidTwoAxle),
            2 => Some(Self::SemiTrailer),
            3 => Some(Self::FullTrailer),
            4 => Some(Self::ArticulatedBus),
            _ => None,
        }
    }
}

/// 道路机动车型的来源构型与该构型的编制参数。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RoadVehicleGeometryInput {
    /// 刚性双轴、前轮转向机动车的完整参数。
    RigidTwoAxle(RigidTwoAxleGeometryInput),
    /// 牵引车与半挂车，首版报告构型未受支持。
    SemiTrailer,
    /// 牵引车与全挂车，首版报告构型未受支持。
    FullTrailer,
    /// 铰接公交车，首版报告构型未受支持。
    ArticulatedBus,
}

/// 编制配置中声明的构型和可选 SI 字段。构型决定哪些字段必填。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoadVehicleGeometryConfig {
    /// 来源构型编码。
    pub configuration: u8,
    /// 前后轴距，单位米。
    pub wheelbase_m: Option<f64>,
    /// 全轮距，单位米。
    pub track_width_m: Option<f64>,
    /// 后轴参考点高度，单位米。
    pub reference_height_m: Option<f64>,
    /// 后悬，单位米。
    pub rear_overhang_m: Option<f64>,
    /// 车身全宽，单位米。
    pub width_m: Option<f64>,
    /// 车身底面相对参考点的高度，单位米。
    pub bottom_height_m: Option<f64>,
    /// 车身顶面相对参考点的高度，单位米。
    pub top_height_m: Option<f64>,
    /// 最大前轮绝对转角，单位弧度。
    pub max_steering_angle_rad: Option<f64>,
    /// 最大绝对转向角速率，单位弧度每秒。
    pub max_steering_rate_rad_s: Option<f64>,
}

/// 车型配置无法还原为带构型的来源输入。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VehicleGeometryConfigError {
    /// 配置包含未登记的构型编码。
    UnknownConfiguration(u8),
    /// 缺少刚性双轴构型必需的字段。
    MissingField(RoadVehicleGeometryField),
}

impl std::fmt::Display for VehicleGeometryConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use RoadVehicleGeometryField as Field;
        match self {
            Self::UnknownConfiguration(code) => write!(formatter, "unknown configuration {code}"),
            Self::MissingField(field) => {
                let name = match field {
                    Field::Wheelbase => "wheelbase_m",
                    Field::TrackWidth => "track_width_m",
                    Field::ReferenceHeight => "reference_height_m",
                    Field::RearOverhang => "rear_overhang_m",
                    Field::Width => "width_m",
                    Field::BottomHeight => "bottom_height_m",
                    Field::TopHeight => "top_height_m",
                    Field::MaxSteeringAngle => "max_steering_angle_rad",
                    Field::MaxSteeringRate => "max_steering_rate_rad_s",
                };
                write!(formatter, "missing {name}")
            }
        }
    }
}

impl std::error::Error for VehicleGeometryConfigError {}

impl TryFrom<RoadVehicleGeometryConfig> for RoadVehicleGeometryInput {
    type Error = VehicleGeometryConfigError;

    fn try_from(config: RoadVehicleGeometryConfig) -> Result<Self, Self::Error> {
        use RoadVehicleGeometryField as Field;
        let model = RoadVehicleConfiguration::from_code(config.configuration).ok_or(
            VehicleGeometryConfigError::UnknownConfiguration(config.configuration),
        )?;
        let required = |value: Option<f64>, field| {
            value.ok_or(VehicleGeometryConfigError::MissingField(field))
        };
        Ok(match model {
            RoadVehicleConfiguration::SemiTrailer => Self::SemiTrailer,
            RoadVehicleConfiguration::FullTrailer => Self::FullTrailer,
            RoadVehicleConfiguration::ArticulatedBus => Self::ArticulatedBus,
            RoadVehicleConfiguration::RigidTwoAxle => {
                Self::RigidTwoAxle(RigidTwoAxleGeometryInput {
                    wheelbase_m: required(config.wheelbase_m, Field::Wheelbase)?,
                    track_width_m: required(config.track_width_m, Field::TrackWidth)?,
                    reference_height_m: required(
                        config.reference_height_m,
                        Field::ReferenceHeight,
                    )?,
                    rear_overhang_m: required(config.rear_overhang_m, Field::RearOverhang)?,
                    width_m: required(config.width_m, Field::Width)?,
                    bottom_height_m: required(config.bottom_height_m, Field::BottomHeight)?,
                    top_height_m: required(config.top_height_m, Field::TopHeight)?,
                    max_steering_angle_rad: required(
                        config.max_steering_angle_rad,
                        Field::MaxSteeringAngle,
                    )?,
                    max_steering_rate_rad_s: required(
                        config.max_steering_rate_rad_s,
                        Field::MaxSteeringRate,
                    )?,
                })
            }
        })
    }
}

/// 刚性双轴车型显式编制参数。所有距离以米计，角度以弧度计。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RigidTwoAxleGeometryInput {
    /// 前后轴距。
    pub wheelbase_m: f64,
    /// 同轴左右轮中心间距。
    pub track_width_m: f64,
    /// 后轴参考点相对路面的竖直高度。
    pub reference_height_m: f64,
    /// 后轴至车尾的距离。
    pub rear_overhang_m: f64,
    /// 有效车身全宽。
    pub width_m: f64,
    /// 相对后轴参考点的车身底高。
    pub bottom_height_m: f64,
    /// 相对后轴参考点的车身顶高。
    pub top_height_m: f64,
    /// 最大前轮绝对转角。
    pub max_steering_angle_rad: f64,
    /// 最大绝对转向角速率，单位为弧度每秒。
    pub max_steering_rate_rad_s: f64,
}

/// 车型几何来源属性的准入失败。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VehicleGeometryInputError {
    index: u16,
    pub(crate) value: f64,
    pub(crate) violation: ScalarViolation,
}

impl VehicleGeometryInputError {
    /// 几何属性按 LFRE 的零基字段下标定位，构型位于 9。
    #[must_use]
    pub const fn field_index(self) -> u16 {
        self.index
    }

    /// 失败的来源字段名。
    #[must_use]
    pub fn field(self) -> &'static str {
        [
            "wheelbase",
            "trackWidth",
            "referenceHeight",
            "rearOverhang",
            "width",
            "bottomHeight",
            "topHeight",
            "maxSteeringAngle",
            "maxSteeringRate",
            "configuration",
        ][usize::from(self.index)]
    }
}

impl std::fmt::Display for VehicleGeometryInputError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "车型字段 {}={} 无法编制：{}",
            self.field(),
            self.value,
            crate::diagnostic::ScalarViolationDisplay(self.violation)
        )
    }
}

impl std::error::Error for VehicleGeometryInputError {}

impl RoadVehicleGeometryInput {
    /// 返回来源声明的构型。
    #[must_use]
    pub const fn configuration(self) -> RoadVehicleConfiguration {
        match self {
            Self::RigidTwoAxle(_) => RoadVehicleConfiguration::RigidTwoAxle,
            Self::SemiTrailer => RoadVehicleConfiguration::SemiTrailer,
            Self::FullTrailer => RoadVehicleConfiguration::FullTrailer,
            Self::ArticulatedBus => RoadVehicleConfiguration::ArticulatedBus,
        }
    }

    pub(crate) fn rigid(self) -> Result<RigidTwoAxleGeometryInput, VehicleGeometryInputError> {
        match self {
            Self::RigidTwoAxle(geometry) => Ok(geometry),
            _ => Err(VehicleGeometryInputError {
                index: 9,
                value: f64::from(self.configuration() as u8),
                violation: ScalarViolation::UnsupportedVehicleConfiguration,
            }),
        }
    }

    /// 量化并对照纵向模型的规范总车长校验全部字段。
    ///
    /// # Errors
    ///
    /// 构型、标量表示域或量化后的几何关系不满足要求时返回失败的属性。
    pub fn canonical(
        self,
        length_mm: u32,
    ) -> Result<RoadVehicleGeometry, VehicleGeometryInputError> {
        self.rigid()?.canonical(length_mm)
    }
}

impl From<RigidTwoAxleGeometryInput> for RoadVehicleGeometryInput {
    fn from(geometry: RigidTwoAxleGeometryInput) -> Self {
        Self::RigidTwoAxle(geometry)
    }
}

impl RigidTwoAxleGeometryInput {
    /// 读取九个 SI 标量，量化并对照规范车长校验刚性双轴几何。
    ///
    /// # Errors
    ///
    /// 数值越界或量化后的几何约束失效时，错误指明对应的来源属性。
    pub fn canonical(
        self,
        length_mm: u32,
    ) -> Result<RoadVehicleGeometry, VehicleGeometryInputError> {
        let values = [
            self.wheelbase_m,
            self.track_width_m,
            self.reference_height_m,
            self.rear_overhang_m,
            self.width_m,
            self.bottom_height_m,
            self.top_height_m,
            self.max_steering_angle_rad,
            self.max_steering_rate_rad_s,
        ];
        let error = |index: u16, violation| VehicleGeometryInputError {
            index,
            value: values[usize::from(index)],
            violation,
        };
        let finite = |i: u16| {
            let value = values[usize::from(i)];
            value
                .is_finite()
                .then_some(value)
                .ok_or_else(|| error(i, ScalarViolation::NotFinite))
        };
        let nonnegative = |i: u16| {
            let value = finite(i)?;
            (value >= 0.).then_some(value).ok_or_else(|| {
                error(
                    i,
                    ScalarViolation::NotLessThan {
                        inclusive_minimum_bits: 0,
                    },
                )
            })
        };
        let unsigned_mm = |i| {
            millimetres_from_si(nonnegative(i)?)
                .ok_or_else(|| error(i, ScalarViolation::QuantizeFailed))
        };
        let signed_mm = |i| {
            millimetres_i32_from_si(finite(i)?)
                .ok_or_else(|| error(i, ScalarViolation::QuantizeFailed))
        };
        let angle = nonnegative(7)?;
        if angle >= std::f64::consts::FRAC_PI_2 {
            return Err(error(7, ScalarViolation::VehicleGeometryConstraint));
        }
        let geometry = RoadVehicleGeometry {
            wheelbase_mm: unsigned_mm(0)?,
            track_width_mm: unsigned_mm(1)?,
            reference_height_mm: signed_mm(2)?,
            rear_overhang_mm: unsigned_mm(3)?,
            width_mm: unsigned_mm(4)?,
            bottom_height_mm: signed_mm(5)?,
            top_height_mm: signed_mm(6)?,
            max_steering_angle_rad: conservative_limit(angle),
            max_steering_rate_rad_s: conservative_limit(nonnegative(8)?),
        };
        geometry
            .validate(length_mm)
            .map_err(|field| error(field as u16 - 1, ScalarViolation::VehicleGeometryConstraint))?;
        Ok(geometry)
    }
}

// 源值已经证明有限且非负。向下包括最近舍入溢出为无穷的有限大数。
fn conservative_limit(source: f64) -> f32 {
    if source == 0. {
        return 0.;
    }
    let nearest = source as f32;
    if f64::from(nearest) > source {
        nearest.next_down()
    } else {
        nearest
    }
}

/// Synthetic 记录与编译器内的规范摘要均取九个规范字的有序小端字节。
pub(crate) fn canonical_words(geometry: RoadVehicleGeometry) -> [u8; 36] {
    let words = [
        geometry.wheelbase_mm.to_le_bytes(),
        geometry.track_width_mm.to_le_bytes(),
        geometry.reference_height_mm.to_le_bytes(),
        geometry.rear_overhang_mm.to_le_bytes(),
        geometry.width_mm.to_le_bytes(),
        geometry.bottom_height_mm.to_le_bytes(),
        geometry.top_height_mm.to_le_bytes(),
        geometry.max_steering_angle_rad.to_le_bytes(),
        geometry.max_steering_rate_rad_s.to_le_bytes(),
    ];
    let mut bytes = [0; 36];
    bytes.copy_from_slice(words.as_flattened());
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_articulated_configurations_preserve_their_tag_without_rigid_fields() {
        for (configuration, expected) in [
            (2, RoadVehicleGeometryInput::SemiTrailer),
            (3, RoadVehicleGeometryInput::FullTrailer),
            (4, RoadVehicleGeometryInput::ArticulatedBus),
        ] {
            let input = RoadVehicleGeometryInput::try_from(RoadVehicleGeometryConfig {
                configuration,
                wheelbase_m: None,
                track_width_m: None,
                reference_height_m: None,
                rear_overhang_m: None,
                width_m: None,
                bottom_height_m: None,
                top_height_m: None,
                max_steering_angle_rad: None,
                max_steering_rate_rad_s: None,
            })
            .unwrap();
            assert_eq!(input, expected);
            let error = input.canonical(4_500).unwrap_err();
            assert_eq!(
                error.violation,
                ScalarViolation::UnsupportedVehicleConfiguration
            );
            assert_eq!(error.field(), "configuration");
        }
    }
}
