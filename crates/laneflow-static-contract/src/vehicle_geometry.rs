//! 道路机动车型的规范单刚体几何，单位与 LFCA 车型行一致。

/// 前后轴共用轮距、前轮转向的双轴单刚体车型。车长由同一车型的纵向字段提供。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoadVehicleGeometry {
    /// 前后轴中心距，单位毫米。
    pub wheelbase_mm: u32,
    /// 左右轮中心的全轮距，单位毫米。
    pub track_width_mm: u32,
    /// 后轴参考点相对路面的竖直高度，单位毫米。
    pub reference_height_mm: i32,
    /// 后轴到后保险杠的距离，单位毫米。
    pub rear_overhang_mm: u32,
    /// 整车包络全宽，单位毫米。
    pub width_mm: u32,
    /// 车身底面相对参考点的车体上向坐标，单位毫米。
    pub bottom_height_mm: i32,
    /// 车身顶面相对参考点的车体上向坐标，单位毫米。
    pub top_height_mm: i32,
    /// 最大前轮绝对转角，单位弧度。
    pub max_steering_angle_rad: f32,
    /// 最大绝对转向角速率，单位弧度每秒。
    pub max_steering_rate_rad_s: f32,
}

/// 单刚体几何内嵌记录的字段标签。
#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoadVehicleGeometryField {
    /// 轴距。
    Wheelbase = 1,
    /// 全轮距。
    TrackWidth = 2,
    /// 参考点高度。
    ReferenceHeight = 3,
    /// 后悬。
    RearOverhang = 4,
    /// 车宽。
    Width = 5,
    /// 车身底高。
    BottomHeight = 6,
    /// 车身顶高。
    TopHeight = 7,
    /// 最大前轮绝对转角。
    MaxSteeringAngle = 8,
    /// 最大绝对转向角速率。
    MaxSteeringRate = 9,
}

impl RoadVehicleGeometry {
    /// 核验量化后单刚体几何及其与已发布车长的关系。
    ///
    /// # Errors
    ///
    /// 返回首个违反尺寸、包络或转向限值的字段。
    pub fn validate(self, length_mm: u32) -> Result<(), RoadVehicleGeometryField> {
        use RoadVehicleGeometryField as Field;
        let front_from_rear = length_mm
            .checked_sub(self.rear_overhang_mm)
            .ok_or(Field::RearOverhang)?;
        if self.wheelbase_mm == 0 || self.wheelbase_mm > front_from_rear {
            return Err(Field::Wheelbase);
        }
        if self.track_width_mm == 0 {
            return Err(Field::TrackWidth);
        }
        if self.width_mm < self.track_width_mm {
            return Err(Field::Width);
        }
        if self.top_height_mm <= self.bottom_height_mm {
            return Err(Field::TopHeight);
        }
        if !self.max_steering_angle_rad.is_finite()
            || self.max_steering_angle_rad.is_sign_negative()
            || f64::from(self.max_steering_angle_rad) >= core::f64::consts::FRAC_PI_2
        {
            return Err(Field::MaxSteeringAngle);
        }
        if !self.max_steering_rate_rad_s.is_finite()
            || self.max_steering_rate_rad_s.is_sign_negative()
        {
            return Err(Field::MaxSteeringRate);
        }
        Ok(())
    }
}
