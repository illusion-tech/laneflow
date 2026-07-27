//! Vehicle Profile、immutable registry 与 resolver。

use indexmap::IndexMap;

use crate::{
    error::CoreError,
    handle::{ParticipantClassHandle, VehicleProfileHandle},
    id::validate_external_id,
    numeric_policy::MIN_VEHICLE_LENGTH_EXCLUSIVE_METERS,
    participant_class::ParticipantClassRegistry,
};

/// IIDM Vehicle Profile 的命名构造输入。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IidmProfileSpec {
    /// 车辆长度，单位为 meter。
    pub length: f64,
    /// free-flow 期望速度，单位为 meter/second。
    pub desired_speed: f64,
    /// 行为最小间距，单位为 meter。
    pub min_gap: f64,
    /// 期望时间间隔，单位为 second。
    pub time_headway: f64,
    /// 最大舒适加速度，单位为 meter/second^2。
    pub max_acceleration: f64,
    /// 舒适减速度幅值，单位为 meter/second^2。
    pub comfortable_deceleration: f64,
    /// 紧急减速度幅值，单位为 meter/second^2。
    pub emergency_deceleration: f64,
}

/// 经过校验的 immutable Vehicle Profile。
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleProfile {
    external_id: String,
    participant_class: ParticipantClassHandle,
    iidm: IidmProfileSpec,
}

impl VehicleProfile {
    /// 创建经过完整 domain validation 的 IIDM profile。
    ///
    /// `participant_class` 必须来自注册时使用的 `ParticipantClassRegistry`（范围由
    /// `VehicleProfileRegistry::try_new` 校验）。
    ///
    /// # Errors
    ///
    /// external ID 非法、任一数值不满足 finite/范围约束，或 emergency deceleration
    /// 小于 comfortable deceleration 时返回 `CoreError`。
    pub fn try_new_iidm(
        external_id: impl Into<String>,
        participant_class: ParticipantClassHandle,
        spec: IidmProfileSpec,
    ) -> Result<Self, CoreError> {
        let external_id = external_id.into();
        validate_external_id("vehicleProfiles[].id", &external_id)?;

        if !spec.length.is_finite() || spec.length <= MIN_VEHICLE_LENGTH_EXCLUSIVE_METERS {
            return Err(CoreError::InvalidVehicleProfileValue {
                profile_id: external_id,
                field: "length",
                value: spec.length,
                requirement: "必须是 finite 且大于 GEOMETRY_GAP_EPSILON",
            });
        }
        validate_positive_profile_value(&external_id, "desiredSpeed", spec.desired_speed)?;
        validate_nonnegative_profile_value(&external_id, "minGap", spec.min_gap)?;
        validate_positive_profile_value(&external_id, "timeHeadway", spec.time_headway)?;
        validate_positive_profile_value(&external_id, "maxAcceleration", spec.max_acceleration)?;
        validate_positive_profile_value(
            &external_id,
            "comfortableDeceleration",
            spec.comfortable_deceleration,
        )?;
        validate_positive_profile_value(
            &external_id,
            "emergencyDeceleration",
            spec.emergency_deceleration,
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
            participant_class,
            iidm: spec,
        })
    }

    /// 返回 profile external ID。
    pub fn external_id(&self) -> &str {
        &self.external_id
    }

    /// 返回 profile 归属的 ParticipantClass handle。
    pub const fn participant_class(&self) -> ParticipantClassHandle {
        self.participant_class
    }

    /// 返回经过校验的 IIDM 参数。
    pub const fn iidm(&self) -> IidmProfileSpec {
        self.iidm
    }
}

/// world 初始化后保持 immutable 的 Vehicle Profile registry。
///
/// `class_external_ids` 是 registry 侧 retained identity（junction retained
/// definitions 先例）：按 profile 对齐保留构造时解析出的 ParticipantClass
/// external ID，供 `rebind_classes` 对另一个 class registry 重新解析 handle，
/// 不进入 steady tick 求值路径。
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleProfileRegistry {
    profiles: Vec<VehicleProfile>,
    handles: IndexMap<String, VehicleProfileHandle>,
    class_external_ids: Vec<String>,
}

impl VehicleProfileRegistry {
    /// 创建空 registry，用于不声明 Vehicle Profile 的程序化输入或空 profile package。
    pub fn empty() -> Self {
        Self {
            profiles: Vec::new(),
            handles: IndexMap::new(),
            class_external_ids: Vec::new(),
        }
    }

    /// 按输入顺序创建并校验 immutable registry。
    ///
    /// 每个 profile 的 ParticipantClass handle 必须落在 `classes` 范围内
    /// （index < class_count），防止跨 registry 混用 handle；通过范围校验后
    /// 按 handle 保留对应的 class external ID 作为 rebind 的 identity。
    ///
    /// # Errors
    ///
    /// profile external ID 重复时返回 `CoreError::DuplicateVehicleProfileId`；
    /// ParticipantClass handle 越界时返回
    /// `CoreError::VehicleProfileParticipantClassOutOfRange`。
    pub fn try_new<I>(classes: &ParticipantClassRegistry, profiles: I) -> Result<Self, CoreError>
    where
        I: IntoIterator<Item = VehicleProfile>,
    {
        let mut resolved_profiles = Vec::new();
        let mut handles = IndexMap::new();
        let mut class_external_ids = Vec::new();

        for profile in profiles {
            if handles.contains_key(profile.external_id()) {
                return Err(CoreError::DuplicateVehicleProfileId {
                    profile_id: profile.external_id().to_owned(),
                });
            }
            if profile.participant_class().index() >= classes.class_count() {
                return Err(CoreError::VehicleProfileParticipantClassOutOfRange {
                    profile_id: profile.external_id().to_owned(),
                    class_index: profile.participant_class().index(),
                    class_count: classes.class_count(),
                });
            }
            class_external_ids.push(
                classes
                    .class_external_id(profile.participant_class())
                    .expect("range-checked class handle must have external ID")
                    .to_owned(),
            );

            let handle = VehicleProfileHandle::new(resolved_profiles.len());
            handles.insert(profile.external_id().to_owned(), handle);
            resolved_profiles.push(profile);
        }

        Ok(Self {
            profiles: resolved_profiles,
            handles,
            class_external_ids,
        })
    }

    /// 按 retained class external ID 对目标 ParticipantClassRegistry 重新解析
    /// 每个 profile 的 class handle（junction `rebind_to_lane_graph` 先例）。
    ///
    /// handle 是 dense index，只在构造它们的 registry 内有意义；profile registry
    /// 与 final assembly 的 class registry 声明顺序不同时，直接沿用旧 handle 会
    /// 把 profile 静默错挂到另一个 class。本方法按外部 ID 重新绑定语义。
    ///
    /// # Errors
    ///
    /// 任一 retained class external ID 在目标 registry 中不存在时返回
    /// `CoreError::UnknownVehicleProfileParticipantClass`。
    pub fn rebind_classes(&self, classes: &ParticipantClassRegistry) -> Result<Self, CoreError> {
        let profiles = self
            .profiles
            .iter()
            .zip(&self.class_external_ids)
            .map(|(profile, class_id)| {
                let class = classes.class_handle(class_id).ok_or_else(|| {
                    CoreError::UnknownVehicleProfileParticipantClass {
                        profile_id: profile.external_id().to_owned(),
                        class_id: class_id.clone(),
                    }
                })?;
                Ok(VehicleProfile {
                    external_id: profile.external_id.clone(),
                    participant_class: class,
                    iidm: profile.iidm,
                })
            })
            .collect::<Result<Vec<_>, CoreError>>()?;
        Self::try_new(classes, profiles)
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
        let Self {
            profiles,
            handles,
            class_external_ids,
        } = self;
        let profile_bytes = profiles.capacity() * std::mem::size_of::<VehicleProfile>()
            + profiles
                .iter()
                .map(|profile| profile.external_id.capacity())
                .sum::<usize>();
        let handle_bytes = handles.capacity()
            * std::mem::size_of::<(String, VehicleProfileHandle)>()
            + handles.keys().map(String::capacity).sum::<usize>();
        let retained_class_bytes = class_external_ids.capacity() * std::mem::size_of::<String>()
            + class_external_ids
                .iter()
                .map(String::capacity)
                .sum::<usize>();
        profile_bytes + handle_bytes + retained_class_bytes
    }
}

impl Default for VehicleProfileRegistry {
    fn default() -> Self {
        Self::empty()
    }
}

fn validate_positive_profile_value(
    profile_id: &str,
    field: &'static str,
    value: f64,
) -> Result<(), CoreError> {
    if !value.is_finite() || value <= 0.0 {
        return Err(CoreError::InvalidVehicleProfileValue {
            profile_id: profile_id.to_owned(),
            field,
            value,
            requirement: "必须是 finite 且大于 0",
        });
    }

    Ok(())
}

fn validate_nonnegative_profile_value(
    profile_id: &str,
    field: &'static str,
    value: f64,
) -> Result<(), CoreError> {
    if !value.is_finite() || value < 0.0 {
        return Err(CoreError::InvalidVehicleProfileValue {
            profile_id: profile_id.to_owned(),
            field,
            value,
            requirement: "必须是 finite 且大于或等于 0",
        });
    }

    Ok(())
}
