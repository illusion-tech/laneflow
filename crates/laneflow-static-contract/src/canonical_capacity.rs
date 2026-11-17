//! 规范路网容量的值类型、计数定义与关系角色登记。
//!
//! 规范路网容量（ADR 0035 第 4 节）约束一份最终规范路网的规模，编译端按最终 LIR、消费端
//! 按最终 LFCA 字节各自独立统计，两侧对同一规范路网的结果逐维相等。本模块只登记维度、
//! 规模与容量的值类型和关系角色的容量归属；两侧投影由各自的生产者与消费者实现。

/// 规范关联在 LFCA、LFSM 与 LFSD 中登记的有类型角色。
///
/// 数值即可移植规范制品设计第 4.3 节登记的 `sourceRelationRole` 代码。是否计入
/// `CanonicalRelationCount` 由 [`SourceRelationRole::canonical_relation_capacity`] 穷尽登记，
/// 与 LFSD 差异通道分别冻结。
///
/// 本枚举有意不标 `#[non_exhaustive]`：两侧投影须穷尽匹配全部角色，新增角色时编译即要求
/// 登记其容量归属与投影。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum SourceRelationRole {
    /// `LaneEdge` 声明中的一项下游连接。
    LaneEdgeSuccessor = 1,
    /// 道路走廊有序横断面中的一项成员。
    RoadCorridorElement = 2,
    /// 道路区段有序车道集合中的一项成员。
    RoadSectionLane = 3,
    /// 编制车道有序覆盖链中的一项车道图边。
    AuthoringLaneEdge = 4,
    /// 车道组中的一项编制车道成员。
    LaneGroupMember = 5,
    /// 路口拥有的一项转向动作。
    JunctionMovement = 6,
    /// 转向动作拥有的一项机动路径。
    MovementManeuverPath = 7,
    /// 机动路径完整 `entry + internal + exit` 序列中的一项边引用。
    ManeuverPathEdge = 8,
    /// 从全部机动路径派生的一项路口内部边排他所有权。
    JunctionInternalEdge = 9,
    /// 机动路径按转换顺序拥有的一项机动门。
    ManeuverPathGate = 10,
    /// 机动路径按入口转换顺序拥有的一项等待区。
    ManeuverPathWaitingZone = 11,
    /// 停止线被一项机动门引用的反向关系。
    StopLineManeuverGate = 12,
    /// 停车设施的一项虚拟入口锚点。
    ParkingFacilityVirtualEntry = 13,
    /// 停车设施的一项虚拟出口锚点。
    ParkingFacilityVirtualExit = 14,
    /// 路口拥有的一项冲突区。
    JunctionConflictZone = 15,
    /// 路口拥有的一项参与者流。
    JunctionParticipantStream = 16,
    /// 信号控制器唯一拥有的一项信号组。
    SignalControllerGroup = 17,
    /// 信号控制器固定时制程序中的一个有序相位。
    SignalControllerPhase = 18,
    /// 信号相位对控制器内一个信号组的灯色赋值。
    SignalPhaseState = 19,
    /// 机动门到固定时制信号组的控制绑定。
    ManeuverGateSignalGroup = 20,
    /// 停车位到可选停车设施的组织归属。
    ParkingSpaceFacility = 21,
    /// 停车位入口到车道图边严格内部位置的锚定。
    ParkingSpaceEntry = 22,
    /// 停车位出口到车道图边严格内部位置的锚定。
    ParkingSpaceExit = 23,
    /// 参与者类别到其可选单继承父类的关系。
    ParticipantClassExtends = 24,
    /// 准入规则到其静态目标的关系。
    AccessRuleTarget = 25,
    /// 准入规则集合中的一项参与者类别选择器。
    AccessRuleParticipantClass = 26,
    /// 车辆配置到其唯一参与者类别的静态分类关系。
    VehicleProfileParticipantClass = 27,
    /// 规范坐标框架拥有的一条车道图边中心线。
    CanonicalFrameLaneEdgeGeometry = 28,
    /// 规范坐标框架拥有的一条 non-traversable 设施带中心线。
    CanonicalFrameFacilityBandGeometry = 29,
    /// 参与者流绑定的唯一机动路径。
    ParticipantStreamManeuverPath = 30,
    /// 参与者流中的一项冲突通行段。
    ParticipantStreamConflictPassage = 31,
    /// 规范坐标框架拥有的一项冲突区区域。
    CanonicalFrameConflictZoneRegion = 32,
    /// 策略内按 key 排序的一项依据。
    PolicyEvidence = 33,
    /// 策略内按 key 排序的一项间隙配置。
    PolicyGapProfile = 34,
    /// 策略内按 key 排序的一项流规则。
    PolicyStreamRule = 35,
    /// 策略内按 key 排序的一项门规则。
    PolicyGateRule = 36,
}

impl SourceRelationRole {
    /// 按代码升序排列的全部登记角色。
    pub const ALL: [Self; 36] = [
        Self::LaneEdgeSuccessor,
        Self::RoadCorridorElement,
        Self::RoadSectionLane,
        Self::AuthoringLaneEdge,
        Self::LaneGroupMember,
        Self::JunctionMovement,
        Self::MovementManeuverPath,
        Self::ManeuverPathEdge,
        Self::JunctionInternalEdge,
        Self::ManeuverPathGate,
        Self::ManeuverPathWaitingZone,
        Self::StopLineManeuverGate,
        Self::ParkingFacilityVirtualEntry,
        Self::ParkingFacilityVirtualExit,
        Self::JunctionConflictZone,
        Self::JunctionParticipantStream,
        Self::SignalControllerGroup,
        Self::SignalControllerPhase,
        Self::SignalPhaseState,
        Self::ManeuverGateSignalGroup,
        Self::ParkingSpaceFacility,
        Self::ParkingSpaceEntry,
        Self::ParkingSpaceExit,
        Self::ParticipantClassExtends,
        Self::AccessRuleTarget,
        Self::AccessRuleParticipantClass,
        Self::VehicleProfileParticipantClass,
        Self::CanonicalFrameLaneEdgeGeometry,
        Self::CanonicalFrameFacilityBandGeometry,
        Self::ParticipantStreamManeuverPath,
        Self::ParticipantStreamConflictPassage,
        Self::CanonicalFrameConflictZoneRegion,
        Self::PolicyEvidence,
        Self::PolicyGapProfile,
        Self::PolicyStreamRule,
        Self::PolicyGateRule,
    ];

    /// 登记的 `sourceRelationRole` 代码。
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// 由 `sourceRelationRole` 代码取回登记角色；未登记的代码返回 `None`。
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::LaneEdgeSuccessor),
            2 => Some(Self::RoadCorridorElement),
            3 => Some(Self::RoadSectionLane),
            4 => Some(Self::AuthoringLaneEdge),
            5 => Some(Self::LaneGroupMember),
            6 => Some(Self::JunctionMovement),
            7 => Some(Self::MovementManeuverPath),
            8 => Some(Self::ManeuverPathEdge),
            9 => Some(Self::JunctionInternalEdge),
            10 => Some(Self::ManeuverPathGate),
            11 => Some(Self::ManeuverPathWaitingZone),
            12 => Some(Self::StopLineManeuverGate),
            13 => Some(Self::ParkingFacilityVirtualEntry),
            14 => Some(Self::ParkingFacilityVirtualExit),
            15 => Some(Self::JunctionConflictZone),
            16 => Some(Self::JunctionParticipantStream),
            17 => Some(Self::SignalControllerGroup),
            18 => Some(Self::SignalControllerPhase),
            19 => Some(Self::SignalPhaseState),
            20 => Some(Self::ManeuverGateSignalGroup),
            21 => Some(Self::ParkingSpaceFacility),
            22 => Some(Self::ParkingSpaceEntry),
            23 => Some(Self::ParkingSpaceExit),
            24 => Some(Self::ParticipantClassExtends),
            25 => Some(Self::AccessRuleTarget),
            26 => Some(Self::AccessRuleParticipantClass),
            27 => Some(Self::VehicleProfileParticipantClass),
            28 => Some(Self::CanonicalFrameLaneEdgeGeometry),
            29 => Some(Self::CanonicalFrameFacilityBandGeometry),
            30 => Some(Self::ParticipantStreamManeuverPath),
            31 => Some(Self::ParticipantStreamConflictPassage),
            32 => Some(Self::CanonicalFrameConflictZoneRegion),
            33 => Some(Self::PolicyEvidence),
            34 => Some(Self::PolicyGapProfile),
            35 => Some(Self::PolicyStreamRule),
            36 => Some(Self::PolicyGateRule),
            _ => None,
        }
    }

    /// 本角色在 `CanonicalRelationCount` 中的归属。
    ///
    /// 本版 role 1–32 计入；role 33–36 的 subject 是没有独立实体身份的路权策略局部成员，
    /// 不计入。新增角色时必须在此登记归属，不以代码区间或默认分支决定。
    #[must_use]
    pub const fn canonical_relation_capacity(self) -> CanonicalRelationCapacity {
        match self {
            Self::LaneEdgeSuccessor => CanonicalRelationCapacity::Counted,
            Self::RoadCorridorElement => CanonicalRelationCapacity::Counted,
            Self::RoadSectionLane => CanonicalRelationCapacity::Counted,
            Self::AuthoringLaneEdge => CanonicalRelationCapacity::Counted,
            Self::LaneGroupMember => CanonicalRelationCapacity::Counted,
            Self::JunctionMovement => CanonicalRelationCapacity::Counted,
            Self::MovementManeuverPath => CanonicalRelationCapacity::Counted,
            Self::ManeuverPathEdge => CanonicalRelationCapacity::Counted,
            Self::JunctionInternalEdge => CanonicalRelationCapacity::Counted,
            Self::ManeuverPathGate => CanonicalRelationCapacity::Counted,
            Self::ManeuverPathWaitingZone => CanonicalRelationCapacity::Counted,
            Self::StopLineManeuverGate => CanonicalRelationCapacity::Counted,
            Self::ParkingFacilityVirtualEntry => CanonicalRelationCapacity::Counted,
            Self::ParkingFacilityVirtualExit => CanonicalRelationCapacity::Counted,
            Self::JunctionConflictZone => CanonicalRelationCapacity::Counted,
            Self::JunctionParticipantStream => CanonicalRelationCapacity::Counted,
            Self::SignalControllerGroup => CanonicalRelationCapacity::Counted,
            Self::SignalControllerPhase => CanonicalRelationCapacity::Counted,
            Self::SignalPhaseState => CanonicalRelationCapacity::Counted,
            Self::ManeuverGateSignalGroup => CanonicalRelationCapacity::Counted,
            Self::ParkingSpaceFacility => CanonicalRelationCapacity::Counted,
            Self::ParkingSpaceEntry => CanonicalRelationCapacity::Counted,
            Self::ParkingSpaceExit => CanonicalRelationCapacity::Counted,
            Self::ParticipantClassExtends => CanonicalRelationCapacity::Counted,
            Self::AccessRuleTarget => CanonicalRelationCapacity::Counted,
            Self::AccessRuleParticipantClass => CanonicalRelationCapacity::Counted,
            Self::VehicleProfileParticipantClass => CanonicalRelationCapacity::Counted,
            Self::CanonicalFrameLaneEdgeGeometry => CanonicalRelationCapacity::Counted,
            Self::CanonicalFrameFacilityBandGeometry => CanonicalRelationCapacity::Counted,
            Self::ParticipantStreamManeuverPath => CanonicalRelationCapacity::Counted,
            Self::ParticipantStreamConflictPassage => CanonicalRelationCapacity::Counted,
            Self::CanonicalFrameConflictZoneRegion => CanonicalRelationCapacity::Counted,
            Self::PolicyEvidence => CanonicalRelationCapacity::Excluded,
            Self::PolicyGapProfile => CanonicalRelationCapacity::Excluded,
            Self::PolicyStreamRule => CanonicalRelationCapacity::Excluded,
            Self::PolicyGateRule => CanonicalRelationCapacity::Excluded,
        }
    }
}

/// 关系角色在 `CanonicalRelationCount` 中的归属。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CanonicalRelationCapacity {
    /// 每个合法出现位置计 1。
    Counted,
    /// 不计入规范关系容量，作为独立类别的规范统计进入资源计划。
    Excluded,
}

/// 规范路网容量的维度。
///
/// 维度只与规范路网规模的同名维度比较，不与来源处理限额、制品格式边界或操作资源预算
/// 比较。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum CanonicalNetworkDimension {
    /// 最终规范路网中的稳定实体。
    StableEntityCount,
    /// 最终规范路网中的机动门。
    ManeuverGateCount,
    /// 最终规范路网中的等待区。
    WaitingZoneCount,
    /// 登记为容量计数对象的规范关联出现记录。
    CanonicalRelationCount,
    /// 规范几何点记录：车道边与设施带几何的点，以及冲突区区域环的点。
    GeometryPointCount,
}

impl CanonicalNetworkDimension {
    /// 按固定检查顺序排列的全部维度。
    pub const ALL: [Self; 5] = [
        Self::StableEntityCount,
        Self::ManeuverGateCount,
        Self::WaitingZoneCount,
        Self::CanonicalRelationCount,
        Self::GeometryPointCount,
    ];

    /// 返回设计文档登记的精确配置字段名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StableEntityCount => "max_stable_entity_count",
            Self::ManeuverGateCount => "max_maneuver_gate_count",
            Self::WaitingZoneCount => "max_waiting_zone_count",
            Self::CanonicalRelationCount => "max_canonical_relation_count",
            Self::GeometryPointCount => "max_geometry_point_count",
        }
    }
}

/// 一份最终规范路网实际包含的规范路网容量各维度数量，即规范路网规模。
///
/// 规范路网规模是事实，不是限额，只与 [`CanonicalNetworkCapacity`] 比较。
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CanonicalNetworkUsage {
    stable_entities: u64,
    maneuver_gates: u64,
    waiting_zones: u64,
    canonical_relations: u64,
    geometry_points: u64,
}

impl CanonicalNetworkUsage {
    /// 由逐维数量构造规范路网规模。
    #[must_use]
    pub const fn new(
        stable_entities: u64,
        maneuver_gates: u64,
        waiting_zones: u64,
        canonical_relations: u64,
        geometry_points: u64,
    ) -> Self {
        Self {
            stable_entities,
            maneuver_gates,
            waiting_zones,
            canonical_relations,
            geometry_points,
        }
    }

    /// 某维度的数量。
    #[must_use]
    pub const fn get(&self, dimension: CanonicalNetworkDimension) -> u64 {
        match dimension {
            CanonicalNetworkDimension::StableEntityCount => self.stable_entities,
            CanonicalNetworkDimension::ManeuverGateCount => self.maneuver_gates,
            CanonicalNetworkDimension::WaitingZoneCount => self.waiting_zones,
            CanonicalNetworkDimension::CanonicalRelationCount => self.canonical_relations,
            CanonicalNetworkDimension::GeometryPointCount => self.geometry_points,
        }
    }
}

/// 规范路网容量：一份规范路网各维度允许的最大数量。
///
/// 具名配置档提供容量值；生产端在编译成功前、消费端在建立外部 LFCA 索引前按本容量准入。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CanonicalNetworkCapacity {
    limits: CanonicalNetworkUsage,
}

impl CanonicalNetworkCapacity {
    /// 由逐维上限构造规范路网容量。
    #[must_use]
    pub const fn new(
        max_stable_entities: u64,
        max_maneuver_gates: u64,
        max_waiting_zones: u64,
        max_canonical_relations: u64,
        max_geometry_points: u64,
    ) -> Self {
        Self {
            limits: CanonicalNetworkUsage::new(
                max_stable_entities,
                max_maneuver_gates,
                max_waiting_zones,
                max_canonical_relations,
                max_geometry_points,
            ),
        }
    }

    /// 某维度的上限。
    #[must_use]
    pub const fn get(&self, dimension: CanonicalNetworkDimension) -> u64 {
        self.limits.get(dimension)
    }

    /// 按 [`CanonicalNetworkDimension::ALL`] 的顺序逐维比较，返回第一个超出容量的维度。
    ///
    /// # Errors
    ///
    /// 任一维度的规模大于上限时返回 [`CanonicalNetworkCapacityExceeded`]。
    pub const fn admit(
        &self,
        usage: &CanonicalNetworkUsage,
    ) -> Result<(), CanonicalNetworkCapacityExceeded> {
        let mut index = 0;
        while index < CanonicalNetworkDimension::ALL.len() {
            let dimension = CanonicalNetworkDimension::ALL[index];
            let observed = usage.get(dimension);
            let limit = self.get(dimension);
            if observed > limit {
                return Err(CanonicalNetworkCapacityExceeded {
                    dimension,
                    observed,
                    limit,
                });
            }
            index += 1;
        }
        Ok(())
    }
}

/// 规范路网规模超出规范路网容量。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CanonicalNetworkCapacityExceeded {
    /// 第一个超出容量的维度。
    pub dimension: CanonicalNetworkDimension,
    /// 该维度的规范路网规模。
    pub observed: u64,
    /// 该维度的容量上限。
    pub limit: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_codes_round_trip_and_follow_registry_order() {
        for (index, role) in SourceRelationRole::ALL.into_iter().enumerate() {
            assert_eq!(usize::from(role.code()), index + 1);
            assert_eq!(SourceRelationRole::from_code(role.code()), Some(role));
        }
        assert_eq!(SourceRelationRole::from_code(0), None);
        assert_eq!(SourceRelationRole::from_code(37), None);
    }

    #[test]
    fn canonical_relation_capacity_counts_roles_one_through_thirty_two() {
        for role in SourceRelationRole::ALL {
            let expected = if role.code() <= 32 {
                CanonicalRelationCapacity::Counted
            } else {
                CanonicalRelationCapacity::Excluded
            };
            assert_eq!(role.canonical_relation_capacity(), expected, "{role:?}");
        }
    }

    #[test]
    fn admit_reports_first_exceeded_dimension_in_fixed_order() {
        let capacity = CanonicalNetworkCapacity::new(10, 2, 3, 40, 50);
        assert_eq!(
            capacity.admit(&CanonicalNetworkUsage::new(10, 2, 3, 40, 50)),
            Ok(())
        );
        assert_eq!(
            capacity.admit(&CanonicalNetworkUsage::new(10, 2, 4, 41, 50)),
            Err(CanonicalNetworkCapacityExceeded {
                dimension: CanonicalNetworkDimension::WaitingZoneCount,
                observed: 4,
                limit: 3,
            })
        );
        assert_eq!(
            capacity.admit(&CanonicalNetworkUsage::new(0, 0, 0, 0, 51)),
            Err(CanonicalNetworkCapacityExceeded {
                dimension: CanonicalNetworkDimension::GeometryPointCount,
                observed: 51,
                limit: 50,
            })
        );
    }
}
