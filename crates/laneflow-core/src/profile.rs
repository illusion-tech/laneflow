//! Vehicle Profile、immutable registry 与 resolver。

use indexmap::IndexMap;

use crate::{
    error::{CoreError, NumericConversionStage},
    handle::VehicleProfileHandle,
    id::validate_external_id,
    numeric_policy::{
        MAX_LOCAL_EXTENT_OR_OFFSET_INCLUSIVE_METERS,
        MAX_PROFILE_ACCELERATION_INCLUSIVE_METERS_PER_SECOND_SQUARED,
        MAX_SPEED_INCLUSIVE_METERS_PER_SECOND, MAX_TIME_HEADWAY_INCLUSIVE_SECONDS,
        MIN_VEHICLE_LENGTH_INCLUSIVE_METERS,
    },
};

/// IIDM Vehicle Profile 的命名构造输入。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IidmProfileSpec {
    /// 车辆长度，单位为 meter。
    pub length: f32,
    /// free-flow 期望速度，单位为 meter/second。
    pub desired_speed: f32,
    /// 行为最小间距，单位为 meter。
    pub min_gap: f32,
    /// 期望时间间隔，单位为 second。
    pub time_headway: f32,
    /// 最大舒适加速度，单位为 meter/second^2。
    pub max_acceleration: f32,
    /// 舒适减速度幅值，单位为 meter/second^2。
    pub comfortable_deceleration: f32,
    /// 紧急减速度幅值，单位为 meter/second^2。
    pub emergency_deceleration: f32,
}

/// 尚未进入 Core 目标数值权威的高保真 IIDM 输入。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RawIidmProfileSpec {
    pub length: f64,
    pub desired_speed: f64,
    pub min_gap: f64,
    pub time_headway: f64,
    pub max_acceleration: f64,
    pub comfortable_deceleration: f64,
    pub emergency_deceleration: f64,
}

/// 经过校验的 immutable Vehicle Profile。
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleProfile {
    external_id: String,
    iidm: IidmProfileSpec,
}

impl VehicleProfile {
    /// 创建经过完整 domain validation 的 IIDM profile。
    ///
    /// # Errors
    ///
    /// external ID 非法、任一数值不满足 finite/范围约束，或 emergency deceleration
    /// 小于 comfortable deceleration 时返回 `CoreError`。
    pub fn try_new_iidm(
        external_id: impl Into<String>,
        spec: IidmProfileSpec,
    ) -> Result<Self, CoreError> {
        let external_id = external_id.into();
        validate_external_id("vehicleProfiles[].id", &external_id)?;

        if !spec.length.is_finite()
            || !(MIN_VEHICLE_LENGTH_INCLUSIVE_METERS..=MAX_LOCAL_EXTENT_OR_OFFSET_INCLUSIVE_METERS)
                .contains(&spec.length)
        {
            return Err(CoreError::InvalidVehicleProfileValue {
                profile_id: external_id,
                field: "length",
                value: spec.length,
                requirement: "必须是 finite 且位于 0.1..=128 m",
            });
        }
        validate_positive_bounded_profile_value(
            &external_id,
            "desiredSpeed",
            spec.desired_speed,
            MAX_SPEED_INCLUSIVE_METERS_PER_SECOND,
            "必须是 finite 且位于 0 < value <= 100 m/s",
        )?;
        validate_nonnegative_bounded_profile_value(
            &external_id,
            "minGap",
            spec.min_gap,
            MAX_LOCAL_EXTENT_OR_OFFSET_INCLUSIVE_METERS,
            "必须是 finite 且位于 0..=128 m",
        )?;
        validate_positive_bounded_profile_value(
            &external_id,
            "timeHeadway",
            spec.time_headway,
            MAX_TIME_HEADWAY_INCLUSIVE_SECONDS,
            "必须是 finite 且位于 0 < value <= 60 s",
        )?;
        validate_positive_bounded_profile_value(
            &external_id,
            "maxAcceleration",
            spec.max_acceleration,
            MAX_PROFILE_ACCELERATION_INCLUSIVE_METERS_PER_SECOND_SQUARED,
            "必须是 finite 且位于 0 < value <= 50 m/s²",
        )?;
        validate_positive_bounded_profile_value(
            &external_id,
            "comfortableDeceleration",
            spec.comfortable_deceleration,
            MAX_PROFILE_ACCELERATION_INCLUSIVE_METERS_PER_SECOND_SQUARED,
            "必须是 finite 且位于 0 < value <= 50 m/s²",
        )?;
        validate_positive_bounded_profile_value(
            &external_id,
            "emergencyDeceleration",
            spec.emergency_deceleration,
            MAX_PROFILE_ACCELERATION_INCLUSIVE_METERS_PER_SECOND_SQUARED,
            "必须是 finite 且位于 0 < value <= 50 m/s²",
        )?;

        if spec.emergency_deceleration < spec.comfortable_deceleration {
            return Err(CoreError::InvalidVehicleProfileDecelerationOrder {
                profile_id: external_id,
                comfortable_deceleration: spec.comfortable_deceleration,
                emergency_deceleration: spec.emergency_deceleration,
            });
        }

        Ok(Self {
            external_id,
            iidm: spec,
        })
    }

    /// 从高保真 `f64` 输入按固定顺序完成范围校验与受检转换。
    pub fn try_new_iidm_from_f64(
        external_id: impl Into<String>,
        raw: RawIidmProfileSpec,
    ) -> Result<Self, CoreError> {
        let external_id = external_id.into();
        validate_external_id("vehicleProfiles[].id", &external_id)?;
        let spec = IidmProfileSpec {
            length: convert_raw_profile_value(
                &external_id,
                "length",
                raw.length,
                MIN_VEHICLE_LENGTH_INCLUSIVE_METERS,
                MAX_LOCAL_EXTENT_OR_OFFSET_INCLUSIVE_METERS,
                true,
                "必须位于 0.1..=128 m 且可表示为 finite f32",
            )?,
            desired_speed: convert_raw_profile_value(
                &external_id,
                "desiredSpeed",
                raw.desired_speed,
                0.0,
                MAX_SPEED_INCLUSIVE_METERS_PER_SECOND,
                false,
                "必须位于 0 < value <= 100 m/s 且可表示为 finite f32",
            )?,
            min_gap: convert_raw_profile_value(
                &external_id,
                "minGap",
                raw.min_gap,
                0.0,
                MAX_LOCAL_EXTENT_OR_OFFSET_INCLUSIVE_METERS,
                true,
                "必须位于 0..=128 m 且可表示为 finite f32",
            )?,
            time_headway: convert_raw_profile_value(
                &external_id,
                "timeHeadway",
                raw.time_headway,
                0.0,
                MAX_TIME_HEADWAY_INCLUSIVE_SECONDS,
                false,
                "必须位于 0 < value <= 60 s 且可表示为 finite f32",
            )?,
            max_acceleration: convert_raw_profile_value(
                &external_id,
                "maxAcceleration",
                raw.max_acceleration,
                0.0,
                MAX_PROFILE_ACCELERATION_INCLUSIVE_METERS_PER_SECOND_SQUARED,
                false,
                "必须位于 0 < value <= 50 m/s² 且可表示为 finite f32",
            )?,
            comfortable_deceleration: convert_raw_profile_value(
                &external_id,
                "comfortableDeceleration",
                raw.comfortable_deceleration,
                0.0,
                MAX_PROFILE_ACCELERATION_INCLUSIVE_METERS_PER_SECOND_SQUARED,
                false,
                "必须位于 0 < value <= 50 m/s² 且可表示为 finite f32",
            )?,
            emergency_deceleration: convert_raw_profile_value(
                &external_id,
                "emergencyDeceleration",
                raw.emergency_deceleration,
                0.0,
                MAX_PROFILE_ACCELERATION_INCLUSIVE_METERS_PER_SECOND_SQUARED,
                false,
                "必须位于 0 < value <= 50 m/s² 且可表示为 finite f32",
            )?,
        };
        Self::try_new_iidm(external_id, spec)
    }

    /// 返回 profile external ID。
    pub fn external_id(&self) -> &str {
        &self.external_id
    }

    /// 返回经过校验的 IIDM 参数。
    pub const fn iidm(&self) -> IidmProfileSpec {
        self.iidm
    }
}

/// world 初始化后保持 immutable 的 Vehicle Profile registry。
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleProfileRegistry {
    profiles: Vec<VehicleProfile>,
    handles: IndexMap<String, VehicleProfileHandle>,
}

impl VehicleProfileRegistry {
    /// 创建空 registry，用于不声明 Vehicle Profile 的程序化输入或空 profile package。
    pub fn empty() -> Self {
        Self {
            profiles: Vec::new(),
            handles: IndexMap::new(),
        }
    }

    /// 按输入顺序创建并校验 immutable registry。
    ///
    /// # Errors
    ///
    /// profile external ID 重复时返回 `CoreError::DuplicateVehicleProfileId`。
    pub fn try_new<I>(profiles: I) -> Result<Self, CoreError>
    where
        I: IntoIterator<Item = VehicleProfile>,
    {
        let mut resolved_profiles = Vec::new();
        let mut handles = IndexMap::new();

        for profile in profiles {
            if handles.contains_key(profile.external_id()) {
                return Err(CoreError::DuplicateVehicleProfileId {
                    profile_id: profile.external_id().to_owned(),
                });
            }

            let handle = VehicleProfileHandle::new(resolved_profiles.len());
            handles.insert(profile.external_id().to_owned(), handle);
            resolved_profiles.push(profile);
        }

        Ok(Self {
            profiles: resolved_profiles,
            handles,
        })
    }

    /// 返回 profile 数量。
    pub const fn len(&self) -> usize {
        self.profiles.len()
    }

    /// 返回 registry 是否为空。
    pub const fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    /// 返回 external ID 对应的 profile handle。
    pub fn profile_handle(&self, external_id: &str) -> Option<VehicleProfileHandle> {
        self.handles.get(external_id).copied()
    }

    /// 返回 handle 对应的 profile。
    pub fn profile(&self, handle: VehicleProfileHandle) -> Option<&VehicleProfile> {
        self.profiles.get(handle.index())
    }

    /// 返回 handle 对应的 external ID。
    pub fn profile_external_id(&self, handle: VehicleProfileHandle) -> Option<&str> {
        self.profile(handle).map(VehicleProfile::external_id)
    }

    /// 按输入顺序返回所有 profile。
    pub fn profiles(&self) -> impl ExactSizeIterator<Item = &VehicleProfile> {
        self.profiles.iter()
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        let profile_bytes = self.profiles.capacity() * std::mem::size_of::<VehicleProfile>()
            + self
                .profiles
                .iter()
                .map(|profile| profile.external_id.capacity())
                .sum::<usize>();
        let handle_bytes = self.handles.capacity()
            * std::mem::size_of::<(String, VehicleProfileHandle)>()
            + self.handles.keys().map(String::capacity).sum::<usize>();
        profile_bytes + handle_bytes
    }
}

impl Default for VehicleProfileRegistry {
    fn default() -> Self {
        Self::empty()
    }
}

fn validate_positive_bounded_profile_value(
    profile_id: &str,
    field: &'static str,
    value: f32,
    max_inclusive: f32,
    requirement: &'static str,
) -> Result<(), CoreError> {
    if !value.is_finite() || value <= 0.0 || value > max_inclusive {
        return Err(CoreError::InvalidVehicleProfileValue {
            profile_id: profile_id.to_owned(),
            field,
            value,
            requirement,
        });
    }

    Ok(())
}

fn validate_nonnegative_bounded_profile_value(
    profile_id: &str,
    field: &'static str,
    value: f32,
    max_inclusive: f32,
    requirement: &'static str,
) -> Result<(), CoreError> {
    if !value.is_finite() || value < 0.0 || value > max_inclusive {
        return Err(CoreError::InvalidVehicleProfileValue {
            profile_id: profile_id.to_owned(),
            field,
            value,
            requirement,
        });
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn convert_raw_profile_value(
    profile_id: &str,
    field: &'static str,
    value: f64,
    min: f32,
    max: f32,
    min_inclusive: bool,
    requirement: &'static str,
) -> Result<f32, CoreError> {
    let in_raw_range = value.is_finite()
        && if min_inclusive {
            value >= f64::from(min)
        } else {
            value > f64::from(min)
        }
        && value <= f64::from(max);
    if !in_raw_range {
        return Err(CoreError::InvalidVehicleProfileInput {
            profile_id: profile_id.to_owned(),
            field,
            value,
            stage: NumericConversionStage::RawInput,
            requirement,
        });
    }
    let converted = value as f32;
    if !converted.is_finite() {
        return Err(CoreError::InvalidVehicleProfileInput {
            profile_id: profile_id.to_owned(),
            field,
            value,
            stage: NumericConversionStage::TargetValue,
            requirement,
        });
    }
    Ok(if converted == 0.0 { 0.0 } else { converted })
}
