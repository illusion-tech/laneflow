//! 与 Canonical LIR 同次成功编译冻结的来源伴随数据。
//!
//! 本模块保存来源模块描述符、唯一来源文档登记，以及 LIR 稳定实体和 owner-local 关系
//! 到来源位置的关联。它只描述“这项已冻结语义来自哪里”，无权补充 LIR 中不存在的
//! 默认值、身份字段、所有者或连接。后继 #298 的源映射发射器必须同时借用本类型和同一
//! [`crate::CompilationOutput`] 中的 [`crate::ValidatedCanonicalLir`]。

mod freeze;

pub(crate) use freeze::freeze_source_map;

use laneflow_static_contract::{
    AccessRuleId, AccessRuleOrdinal, AuthoringLaneId, AuthoringLaneOrdinal, CanonicalFrameId,
    CanonicalFrameOrdinal, EntityKind, FacilityBandId, FacilityBandOrdinal, JunctionId,
    JunctionOrdinal, LaneEdgeId, LaneEdgeOrdinal, LaneGroupId, LaneGroupOrdinal, ManeuverGateId,
    ManeuverGateOrdinal, ManeuverPathId, ManeuverPathOrdinal, MovementId, MovementOrdinal,
    ParkingAreaId, ParkingAreaOrdinal, ParkingSpaceId, ParkingSpaceOrdinal, ParticipantClassId,
    ParticipantClassOrdinal, RoadCorridorId, RoadCorridorOrdinal, RoadSectionId,
    RoadSectionOrdinal, SignalControllerId, SignalControllerOrdinal, SignalGroupId,
    SignalGroupOrdinal, SignalPhaseId, SignalPhaseOrdinal, StaticRouteId, StaticRouteOrdinal,
    StopLineId, StopLineOrdinal, VehicleProfileId, VehicleProfileOrdinal, WaitingZoneId,
    WaitingZoneOrdinal,
};

use crate::diagnostic::DiagnosticCollector;
use crate::lir::LirFreezeOutput;
use crate::mir::{MirModuleKey, MirSignalControl, MirUnit};
use crate::module::SourceDocumentOrdinal;
use crate::{
    CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceDocumentDescriptor,
    SourceModuleDescriptor, SourcePosition, SourceSpan,
};

const LANE_EDGE_SOURCE_LOGICAL_BYTES: u64 = 4 + 16 + 4 + 16 + 4;
const LANE_EDGE_SUCCESSOR_SOURCE_LOGICAL_BYTES: u64 = 16 + 4 + 2 + 4 + 4 + 16 + 4;
const STABLE_ENTITY_SOURCE_LOGICAL_BYTES: u64 = LANE_EDGE_SOURCE_LOGICAL_BYTES;
const CROSS_SECTION_RELATION_SOURCE_LOGICAL_BYTES: u64 = 2 + 4 + 16 + 2 + 4 + 4 + 16 + 4;
const JUNCTION_RELATION_SOURCE_LOGICAL_BYTES: u64 = CROSS_SECTION_RELATION_SOURCE_LOGICAL_BYTES;
const SOURCE_LOCATION_LOGICAL_BYTES: u64 = 4 + 8 + 8;
const ROUTE_RELATION_SOURCE_LOGICAL_BYTES: u64 = CROSS_SECTION_RELATION_SOURCE_LOGICAL_BYTES + 1;
const SIGNAL_RELATION_SOURCE_LOGICAL_BYTES: u64 = CROSS_SECTION_RELATION_SOURCE_LOGICAL_BYTES;
const PARKING_RELATION_SOURCE_LOGICAL_BYTES: u64 = CROSS_SECTION_RELATION_SOURCE_LOGICAL_BYTES;
const ACCESS_RELATION_SOURCE_LOGICAL_BYTES: u64 = CROSS_SECTION_RELATION_SOURCE_LOGICAL_BYTES;
const SPATIAL_RELATION_SOURCE_LOGICAL_BYTES: u64 = CROSS_SECTION_RELATION_SOURCE_LOGICAL_BYTES;

/// owner-local 来源记录中登记的有类型语义角色。
///
/// 数值只在当前编译结果内区分来源记录类别，不是后继源映射线格式代码。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u16)]
#[non_exhaustive]
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
    /// 静态路线有序边序列中的一次边出现。
    StaticRouteEdge = 13,
    /// 静态路线中一次完整机动路径匹配。
    StaticRouteManeuverOccurrence = 14,
    /// 静态路线中一次机动门匹配。
    StaticRouteGateOccurrence = 15,
    /// 静态路线中一次等待区匹配。
    StaticRouteWaitingZoneOccurrence = 16,
    /// 信号控制器唯一拥有的一项信号组。
    SignalControllerGroup = 17,
    /// 信号控制器固定时制程序中的一个有序相位。
    SignalControllerPhase = 18,
    /// 信号相位对控制器内一个信号组的灯色赋值。
    SignalPhaseState = 19,
    /// 机动门到固定时制信号组的控制绑定。
    ManeuverGateSignalGroup = 20,
    /// 停车位到可选停车区域的组织归属。
    ParkingSpaceArea = 21,
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
}

#[derive(Clone, Copy)]
struct SourceLocationRecord {
    source_document_ordinal: SourceDocumentOrdinal,
    start: SourcePosition,
    end: SourcePosition,
}

struct LaneEdgeSourceRecord {
    ordinal: LaneEdgeOrdinal,
    stable_id: LaneEdgeId,
    primary: SourceLocationRecord,
}

struct StableEntitySourceRecord<O, I> {
    ordinal: O,
    stable_id: I,
    primary: SourceLocationRecord,
}

struct CrossSectionRelationSourceRecord {
    owner: CrossSectionRelationOwnerRecord,
    role: SourceRelationRole,
    local_index: u32,
    primary: SourceLocationRecord,
}

#[derive(Clone, Copy)]
enum CrossSectionRelationOwnerRecord {
    RoadCorridor(RoadCorridorOrdinal, RoadCorridorId),
    RoadSection(RoadSectionOrdinal, RoadSectionId),
    AuthoringLane(AuthoringLaneOrdinal, AuthoringLaneId),
    LaneGroup(LaneGroupOrdinal, LaneGroupId),
}

struct JunctionRelationSourceRecord {
    owner: JunctionRelationOwnerRecord,
    role: SourceRelationRole,
    local_index: u32,
    primary: SourceLocationRecord,
}

#[derive(Clone, Copy)]
enum JunctionRelationOwnerRecord {
    Junction(JunctionOrdinal, JunctionId),
    Movement(MovementOrdinal, MovementId),
    ManeuverPath(ManeuverPathOrdinal, ManeuverPathId),
    StopLine(StopLineOrdinal, StopLineId),
}

struct LaneEdgeSuccessorSourceRecord {
    owner_ordinal: LaneEdgeOrdinal,
    owner_stable_id: LaneEdgeId,
    role: SourceRelationRole,
    local_index: u32,
    primary: SourceLocationRecord,
}

struct RouteRelationSourceRecord {
    owner_ordinal: StaticRouteOrdinal,
    owner_stable_id: StaticRouteId,
    role: SourceRelationRole,
    local_index: u32,
    primary: SourceLocationRecord,
    contributing: Option<SourceLocationRecord>,
}

struct SignalRelationSourceRecord {
    owner: SignalRelationOwnerRecord,
    role: SourceRelationRole,
    local_index: u32,
    primary: SourceLocationRecord,
}

struct ParkingRelationSourceRecord {
    owner_ordinal: ParkingSpaceOrdinal,
    owner_stable_id: ParkingSpaceId,
    role: SourceRelationRole,
    local_index: u32,
    primary: SourceLocationRecord,
}

struct SpatialRelationSourceRecord {
    owner_ordinal: CanonicalFrameOrdinal,
    owner_stable_id: CanonicalFrameId,
    role: SourceRelationRole,
    local_index: u32,
    primary: SourceLocationRecord,
}

struct AccessRelationSourceRecord {
    owner: AccessRelationOwnerRecord,
    role: SourceRelationRole,
    local_index: u32,
    primary: SourceLocationRecord,
}

#[derive(Clone, Copy)]
enum AccessRelationOwnerRecord {
    ParticipantClass(ParticipantClassOrdinal, ParticipantClassId),
    VehicleProfile(VehicleProfileOrdinal, VehicleProfileId),
    AccessRule(AccessRuleOrdinal, AccessRuleId),
}

#[derive(Clone, Copy)]
enum SignalRelationOwnerRecord {
    SignalController(SignalControllerOrdinal, SignalControllerId),
    SignalPhase(SignalPhaseOrdinal, SignalPhaseId),
    ManeuverGate(ManeuverGateOrdinal, ManeuverGateId),
}

/// 与一个 Canonical LIR 原子配对的已验证源映射输入。
///
/// 本类型不能由调用方构造。来源模块按编译单元的依赖优先规范顺序保存，每个模块的
/// 一份或多份来源文档再按文档键字节序保存；`sourceDocumentKey` 已在共同准入时证明
/// 全局唯一，并与所属逻辑模块不可分绑定。
pub struct ValidatedSourceMapInput {
    source_modules: Box<[SourceModuleDescriptor]>,
    source_documents: Box<[SourceDocumentDescriptor]>,
    lane_edge_sources: Box<[LaneEdgeSourceRecord]>,
    lane_edge_successor_sources: Box<[LaneEdgeSuccessorSourceRecord]>,
    road_corridor_sources: Box<[StableEntitySourceRecord<RoadCorridorOrdinal, RoadCorridorId>]>,
    road_section_sources: Box<[StableEntitySourceRecord<RoadSectionOrdinal, RoadSectionId>]>,
    authoring_lane_sources: Box<[StableEntitySourceRecord<AuthoringLaneOrdinal, AuthoringLaneId>]>,
    lane_group_sources: Box<[StableEntitySourceRecord<LaneGroupOrdinal, LaneGroupId>]>,
    facility_band_sources: Box<[StableEntitySourceRecord<FacilityBandOrdinal, FacilityBandId>]>,
    cross_section_relation_sources: Box<[CrossSectionRelationSourceRecord]>,
    junction_sources: Box<[StableEntitySourceRecord<JunctionOrdinal, JunctionId>]>,
    movement_sources: Box<[StableEntitySourceRecord<MovementOrdinal, MovementId>]>,
    maneuver_path_sources: Box<[StableEntitySourceRecord<ManeuverPathOrdinal, ManeuverPathId>]>,
    stop_line_sources: Box<[StableEntitySourceRecord<StopLineOrdinal, StopLineId>]>,
    maneuver_gate_sources: Box<[StableEntitySourceRecord<ManeuverGateOrdinal, ManeuverGateId>]>,
    waiting_zone_sources: Box<[StableEntitySourceRecord<WaitingZoneOrdinal, WaitingZoneId>]>,
    signal_group_sources: Box<[StableEntitySourceRecord<SignalGroupOrdinal, SignalGroupId>]>,
    signal_controller_sources:
        Box<[StableEntitySourceRecord<SignalControllerOrdinal, SignalControllerId>]>,
    signal_phase_sources: Box<[StableEntitySourceRecord<SignalPhaseOrdinal, SignalPhaseId>]>,
    signal_relation_sources: Box<[SignalRelationSourceRecord]>,
    parking_area_sources: Box<[StableEntitySourceRecord<ParkingAreaOrdinal, ParkingAreaId>]>,
    parking_space_sources: Box<[StableEntitySourceRecord<ParkingSpaceOrdinal, ParkingSpaceId>]>,
    parking_relation_sources: Box<[ParkingRelationSourceRecord]>,
    participant_class_sources:
        Box<[StableEntitySourceRecord<ParticipantClassOrdinal, ParticipantClassId>]>,
    vehicle_profile_sources:
        Box<[StableEntitySourceRecord<VehicleProfileOrdinal, VehicleProfileId>]>,
    canonical_frame_sources:
        Box<[StableEntitySourceRecord<CanonicalFrameOrdinal, CanonicalFrameId>]>,
    spatial_relation_sources: Box<[SpatialRelationSourceRecord]>,
    access_rule_sources: Box<[StableEntitySourceRecord<AccessRuleOrdinal, AccessRuleId>]>,
    access_relation_sources: Box<[AccessRelationSourceRecord]>,
    junction_relation_sources: Box<[JunctionRelationSourceRecord]>,
    static_route_sources: Box<[StableEntitySourceRecord<StaticRouteOrdinal, StaticRouteId>]>,
    route_relation_sources: Box<[RouteRelationSourceRecord]>,
    peak_controlled_live_bytes: u64,
}

impl ValidatedSourceMapInput {
    /// 返回源映射冻结阶段的编译器控制峰值。
    pub(crate) const fn peak_controlled_live_bytes(&self) -> u64 {
        self.peak_controlled_live_bytes
    }

    /// 按依赖优先规范顺序遍历来源模块描述符。
    pub fn source_modules(&self) -> impl ExactSizeIterator<Item = &SourceModuleDescriptor> {
        self.source_modules.iter()
    }

    /// 按独立冻结序遍历全局唯一来源文档登记。
    pub fn source_documents(&self) -> impl ExactSizeIterator<Item = SourceDocumentView<'_>> {
        self.source_documents
            .iter()
            .map(|descriptor| SourceDocumentView { descriptor })
    }

    /// 按 `LaneEdgeOrdinal` 递增顺序遍历稳定实体来源记录。
    pub fn lane_edge_sources(&self) -> impl ExactSizeIterator<Item = LaneEdgeSourceView<'_>> {
        self.lane_edge_sources
            .iter()
            .map(|record| LaneEdgeSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 owner ordinal、角色和 local index 遍历下游连接来源记录。
    pub fn lane_edge_successor_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = LaneEdgeSuccessorSourceView<'_>> {
        self.lane_edge_successor_sources
            .iter()
            .map(|record| LaneEdgeSuccessorSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `RoadCorridorOrdinal` 递增顺序遍历道路走廊来源记录。
    pub fn road_corridor_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = RoadCorridorSourceView<'_>> {
        self.road_corridor_sources
            .iter()
            .map(|record| RoadCorridorSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `RoadSectionOrdinal` 递增顺序遍历道路区段来源记录。
    pub fn road_section_sources(&self) -> impl ExactSizeIterator<Item = RoadSectionSourceView<'_>> {
        self.road_section_sources
            .iter()
            .map(|record| RoadSectionSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `AuthoringLaneOrdinal` 递增顺序遍历编制车道来源记录。
    pub fn authoring_lane_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = AuthoringLaneSourceView<'_>> {
        self.authoring_lane_sources
            .iter()
            .map(|record| AuthoringLaneSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `LaneGroupOrdinal` 递增顺序遍历车道组来源记录。
    pub fn lane_group_sources(&self) -> impl ExactSizeIterator<Item = LaneGroupSourceView<'_>> {
        self.lane_group_sources
            .iter()
            .map(|record| LaneGroupSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `FacilityBandOrdinal` 递增顺序遍历设施带来源记录。
    pub fn facility_band_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = FacilityBandSourceView<'_>> {
        self.facility_band_sources
            .iter()
            .map(|record| FacilityBandSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 owner kind、owner ordinal、角色和 local index 遍历横断面关系来源。
    pub fn cross_section_relation_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = CrossSectionRelationSourceView<'_>> {
        self.cross_section_relation_sources
            .iter()
            .map(|record| CrossSectionRelationSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `JunctionOrdinal` 递增顺序遍历路口来源记录。
    pub fn junction_sources(&self) -> impl ExactSizeIterator<Item = JunctionSourceView<'_>> {
        self.junction_sources
            .iter()
            .map(|record| JunctionSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `MovementOrdinal` 递增顺序遍历转向动作来源记录。
    pub fn movement_sources(&self) -> impl ExactSizeIterator<Item = MovementSourceView<'_>> {
        self.movement_sources
            .iter()
            .map(|record| MovementSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `ManeuverPathOrdinal` 递增顺序遍历机动路径来源记录。
    pub fn maneuver_path_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = ManeuverPathSourceView<'_>> {
        self.maneuver_path_sources
            .iter()
            .map(|record| ManeuverPathSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `StopLineOrdinal` 递增顺序遍历停止线来源记录。
    pub fn stop_line_sources(&self) -> impl ExactSizeIterator<Item = StopLineSourceView<'_>> {
        self.stop_line_sources
            .iter()
            .map(|record| StopLineSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `ManeuverGateOrdinal` 递增顺序遍历机动门来源记录。
    pub fn maneuver_gate_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = ManeuverGateSourceView<'_>> {
        self.maneuver_gate_sources
            .iter()
            .map(|record| ManeuverGateSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `WaitingZoneOrdinal` 递增顺序遍历等待区来源记录。
    pub fn waiting_zone_sources(&self) -> impl ExactSizeIterator<Item = WaitingZoneSourceView<'_>> {
        self.waiting_zone_sources
            .iter()
            .map(|record| WaitingZoneSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `SignalGroupOrdinal` 递增顺序遍历信号组来源记录。
    pub fn signal_group_sources(&self) -> impl ExactSizeIterator<Item = SignalGroupSourceView<'_>> {
        self.signal_group_sources
            .iter()
            .map(|record| SignalGroupSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `SignalControllerOrdinal` 递增顺序遍历信号控制器来源记录。
    pub fn signal_controller_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = SignalControllerSourceView<'_>> {
        self.signal_controller_sources
            .iter()
            .map(|record| SignalControllerSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `SignalPhaseOrdinal` 递增顺序遍历信号相位来源记录。
    pub fn signal_phase_sources(&self) -> impl ExactSizeIterator<Item = SignalPhaseSourceView<'_>> {
        self.signal_phase_sources
            .iter()
            .map(|record| SignalPhaseSourceView {
                source_map: self,
                record,
            })
    }

    /// 遍历控制器程序、相位状态和机动门信号绑定的规范来源记录。
    pub fn signal_relation_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = SignalRelationSourceView<'_>> {
        self.signal_relation_sources
            .iter()
            .map(|record| SignalRelationSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `ParkingAreaOrdinal` 递增顺序遍历停车区域来源记录。
    pub fn parking_area_sources(&self) -> impl ExactSizeIterator<Item = ParkingAreaSourceView<'_>> {
        self.parking_area_sources
            .iter()
            .map(|record| ParkingAreaSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `ParkingSpaceOrdinal` 递增顺序遍历停车位来源记录。
    pub fn parking_space_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = ParkingSpaceSourceView<'_>> {
        self.parking_space_sources
            .iter()
            .map(|record| ParkingSpaceSourceView {
                source_map: self,
                record,
            })
    }

    /// 遍历停车位可选区域归属和入口/出口锚点的规范来源记录。
    pub fn parking_relation_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = ParkingRelationSourceView<'_>> {
        self.parking_relation_sources
            .iter()
            .map(|record| ParkingRelationSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `ParticipantClassOrdinal` 递增顺序遍历参与者类别来源记录。
    pub fn participant_class_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = ParticipantClassSourceView<'_>> {
        self.participant_class_sources
            .iter()
            .map(|record| ParticipantClassSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `VehicleProfileOrdinal` 递增顺序遍历车辆配置来源记录。
    pub fn vehicle_profile_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = VehicleProfileSourceView<'_>> {
        self.vehicle_profile_sources
            .iter()
            .map(|record| VehicleProfileSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `CanonicalFrameOrdinal` 递增顺序遍历规范坐标框架来源记录。
    pub fn canonical_frame_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalFrameSourceView<'_>> {
        self.canonical_frame_sources
            .iter()
            .map(|record| CanonicalFrameSourceView {
                source_map: self,
                record,
            })
    }

    /// 按规范坐标框架序号和局部下标遍历中心线归属来源记录。
    pub fn spatial_relation_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = SpatialRelationSourceView<'_>> {
        self.spatial_relation_sources
            .iter()
            .map(|record| SpatialRelationSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `AccessRuleOrdinal` 递增顺序遍历准入规则来源记录。
    pub fn access_rule_sources(&self) -> impl ExactSizeIterator<Item = AccessRuleSourceView<'_>> {
        self.access_rule_sources
            .iter()
            .map(|record| AccessRuleSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 owner、角色和局部下标遍历准入关系来源记录。
    pub fn access_relation_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = AccessRelationSourceView<'_>> {
        self.access_relation_sources
            .iter()
            .map(|record| AccessRelationSourceView {
                source_map: self,
                record,
            })
    }

    /// 遍历路口所有者树、完整路径序列、派生内部边与静态控制边界的规范来源记录。
    pub fn junction_relation_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = JunctionRelationSourceView<'_>> {
        self.junction_relation_sources
            .iter()
            .map(|record| JunctionRelationSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `StaticRouteOrdinal` 递增顺序遍历静态路线来源记录。
    pub fn static_route_sources(&self) -> impl ExactSizeIterator<Item = StaticRouteSourceView<'_>> {
        self.static_route_sources
            .iter()
            .map(|record| StaticRouteSourceView {
                source_map: self,
                record,
            })
    }

    /// 按路线、角色和路线内下标遍历静态路线及其预编译出现项来源。
    pub fn route_relation_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = RouteRelationSourceView<'_>> {
        self.route_relation_sources
            .iter()
            .map(|record| RouteRelationSourceView {
                source_map: self,
                record,
            })
    }

    fn location(&self, record: SourceLocationRecord) -> SourceLocationView<'_> {
        let descriptor = &self.source_documents[record.source_document_ordinal.index()];
        SourceLocationView {
            source_document_key: descriptor.source_document_key(),
            start: record.start,
            end: record.end,
        }
    }
}

/// 来源文档登记的一项只读视图。
#[derive(Clone, Copy)]
pub struct SourceDocumentView<'a> {
    descriptor: &'a SourceDocumentDescriptor,
}

impl<'a> SourceDocumentView<'a> {
    /// 返回与机器路径无关、在本编译单元内唯一的文档键。
    #[must_use]
    pub fn source_document_key(&self) -> &'a str {
        self.descriptor.source_document_key()
    }

    /// 返回拥有该文档的来源模块 authoring namespace。
    #[must_use]
    pub fn authoring_namespace_id(&self) -> &'a str {
        self.descriptor.authoring_namespace_id()
    }

    /// 返回该文档规范来源记录的 SHA-256 摘要。
    #[must_use]
    pub const fn source_document_digest(&self) -> &'a [u8; 32] {
        self.descriptor.source_document_digest()
    }

    /// 返回该文档规范来源记录的精确字节数。
    #[must_use]
    pub const fn source_record_byte_len(&self) -> u32 {
        self.descriptor.source_record_byte_len()
    }

    /// 返回该文档的冷显示/审计来源。
    #[must_use]
    pub const fn origin(&self) -> &'a crate::SourceDocumentOrigin {
        self.descriptor.origin()
    }
}

/// 已解析到来源文档登记的一项只读来源位置。
#[derive(Clone, Copy)]
pub struct SourceLocationView<'a> {
    source_document_key: &'a str,
    start: SourcePosition,
    end: SourcePosition,
}

impl<'a> SourceLocationView<'a> {
    /// 返回稳定来源文档键，而不是宿主文件系统路径。
    #[must_use]
    pub const fn source_document_key(&self) -> &'a str {
        self.source_document_key
    }

    /// 返回一基起始行列。
    #[must_use]
    pub const fn start(&self) -> SourcePosition {
        self.start
    }

    /// 返回一基结束行列。
    #[must_use]
    pub const fn end(&self) -> SourcePosition {
        self.end
    }
}

/// 一条 `LaneEdge` 稳定实体来源记录的只读视图。
#[derive(Clone, Copy)]
pub struct LaneEdgeSourceView<'a> {
    source_map: &'a ValidatedSourceMapInput,
    record: &'a LaneEdgeSourceRecord,
}

impl LaneEdgeSourceView<'_> {
    /// 返回本次 LIR 中定位实体的有类型序号。
    #[must_use]
    pub const fn ordinal(&self) -> LaneEdgeOrdinal {
        self.record.ordinal
    }

    /// 返回跨编译定位实体的稳定标识。
    #[must_use]
    pub const fn stable_id(&self) -> LaneEdgeId {
        self.record.stable_id
    }

    /// 返回拥有该声明的主要来源位置。
    #[must_use]
    pub fn primary_source(&self) -> SourceLocationView<'_> {
        self.source_map.location(self.record.primary)
    }

    /// 返回额外贡献来源位置；当前显式 `LaneEdge` 声明没有额外贡献项。
    pub fn contributing_sources(&self) -> impl ExactSizeIterator<Item = SourceLocationView<'_>> {
        core::iter::empty()
    }
}

macro_rules! stable_source_view {
    ($view:ident, $ordinal:ty, $id:ty) => {
        /// 一个已冻结稳定实体的主要来源位置借用视图。
        #[derive(Clone, Copy)]
        pub struct $view<'a> {
            source_map: &'a ValidatedSourceMapInput,
            record: &'a StableEntitySourceRecord<$ordinal, $id>,
        }

        impl $view<'_> {
            /// 返回本次 LIR 中定位实体的有类型序号。
            #[must_use]
            pub const fn ordinal(&self) -> $ordinal {
                self.record.ordinal
            }

            /// 返回跨编译定位实体的有类型稳定标识。
            #[must_use]
            pub const fn stable_id(&self) -> $id {
                self.record.stable_id
            }

            /// 返回拥有该声明的主要来源位置。
            #[must_use]
            pub fn primary_source(&self) -> SourceLocationView<'_> {
                self.source_map.location(self.record.primary)
            }

            /// 当前显式声明没有额外贡献来源。
            pub fn contributing_sources(
                &self,
            ) -> impl ExactSizeIterator<Item = SourceLocationView<'_>> {
                core::iter::empty()
            }
        }
    };
}

stable_source_view!(RoadCorridorSourceView, RoadCorridorOrdinal, RoadCorridorId);
stable_source_view!(RoadSectionSourceView, RoadSectionOrdinal, RoadSectionId);
stable_source_view!(
    AuthoringLaneSourceView,
    AuthoringLaneOrdinal,
    AuthoringLaneId
);
stable_source_view!(LaneGroupSourceView, LaneGroupOrdinal, LaneGroupId);
stable_source_view!(FacilityBandSourceView, FacilityBandOrdinal, FacilityBandId);
stable_source_view!(JunctionSourceView, JunctionOrdinal, JunctionId);
stable_source_view!(MovementSourceView, MovementOrdinal, MovementId);
stable_source_view!(ManeuverPathSourceView, ManeuverPathOrdinal, ManeuverPathId);
stable_source_view!(StopLineSourceView, StopLineOrdinal, StopLineId);
stable_source_view!(ManeuverGateSourceView, ManeuverGateOrdinal, ManeuverGateId);
stable_source_view!(WaitingZoneSourceView, WaitingZoneOrdinal, WaitingZoneId);
stable_source_view!(SignalGroupSourceView, SignalGroupOrdinal, SignalGroupId);
stable_source_view!(
    SignalControllerSourceView,
    SignalControllerOrdinal,
    SignalControllerId
);
stable_source_view!(SignalPhaseSourceView, SignalPhaseOrdinal, SignalPhaseId);
stable_source_view!(ParkingAreaSourceView, ParkingAreaOrdinal, ParkingAreaId);
stable_source_view!(ParkingSpaceSourceView, ParkingSpaceOrdinal, ParkingSpaceId);
stable_source_view!(
    ParticipantClassSourceView,
    ParticipantClassOrdinal,
    ParticipantClassId
);
stable_source_view!(
    VehicleProfileSourceView,
    VehicleProfileOrdinal,
    VehicleProfileId
);
stable_source_view!(
    CanonicalFrameSourceView,
    CanonicalFrameOrdinal,
    CanonicalFrameId
);
stable_source_view!(AccessRuleSourceView, AccessRuleOrdinal, AccessRuleId);
stable_source_view!(StaticRouteSourceView, StaticRouteOrdinal, StaticRouteId);

/// 一条横断面 owner-local 关系来源记录的只读视图。
#[derive(Clone, Copy)]
pub struct CrossSectionRelationSourceView<'a> {
    source_map: &'a ValidatedSourceMapInput,
    record: &'a CrossSectionRelationSourceRecord,
}

/// 横断面关系 owner 的有类型 LIR 序号与稳定标识。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CrossSectionRelationOwner {
    /// 道路走廊及其有类型身份。
    RoadCorridor(RoadCorridorOrdinal, RoadCorridorId),
    /// 道路区段及其有类型身份。
    RoadSection(RoadSectionOrdinal, RoadSectionId),
    /// 编制车道及其有类型身份。
    AuthoringLane(AuthoringLaneOrdinal, AuthoringLaneId),
    /// 车道组及其有类型身份。
    LaneGroup(LaneGroupOrdinal, LaneGroupId),
}

impl CrossSectionRelationOwner {
    /// 返回 owner 对应的 Identity v1 实体种类。
    #[must_use]
    pub const fn entity_kind(self) -> EntityKind {
        match self {
            Self::RoadCorridor(_, _) => EntityKind::RoadCorridor,
            Self::RoadSection(_, _) => EntityKind::RoadSection,
            Self::AuthoringLane(_, _) => EntityKind::AuthoringLane,
            Self::LaneGroup(_, _) => EntityKind::LaneGroup,
        }
    }
}

impl CrossSectionRelationSourceView<'_> {
    /// 返回关系 owner 的有类型 LIR 序号与稳定标识。
    #[must_use]
    pub const fn owner(&self) -> CrossSectionRelationOwner {
        match self.record.owner {
            CrossSectionRelationOwnerRecord::RoadCorridor(ordinal, stable_id) => {
                CrossSectionRelationOwner::RoadCorridor(ordinal, stable_id)
            }
            CrossSectionRelationOwnerRecord::RoadSection(ordinal, stable_id) => {
                CrossSectionRelationOwner::RoadSection(ordinal, stable_id)
            }
            CrossSectionRelationOwnerRecord::AuthoringLane(ordinal, stable_id) => {
                CrossSectionRelationOwner::AuthoringLane(ordinal, stable_id)
            }
            CrossSectionRelationOwnerRecord::LaneGroup(ordinal, stable_id) => {
                CrossSectionRelationOwner::LaneGroup(ordinal, stable_id)
            }
        }
    }

    /// 返回 owner-local 关系的有类型角色。
    #[must_use]
    pub const fn role(&self) -> SourceRelationRole {
        self.record.role
    }

    /// 返回同一 owner 与角色内的零基序号。
    #[must_use]
    pub const fn local_index(&self) -> u32 {
        self.record.local_index
    }

    /// 返回关系声明的主要来源位置。
    #[must_use]
    pub fn primary_source(&self) -> SourceLocationView<'_> {
        self.source_map.location(self.record.primary)
    }

    /// 当前显式关系没有额外贡献来源。
    pub fn contributing_sources(&self) -> impl ExactSizeIterator<Item = SourceLocationView<'_>> {
        core::iter::empty()
    }
}

/// 一条路口拓扑或静态控制边界 owner-local 关系来源记录的只读视图。
#[derive(Clone, Copy)]
pub struct JunctionRelationSourceView<'a> {
    source_map: &'a ValidatedSourceMapInput,
    record: &'a JunctionRelationSourceRecord,
}

/// 路口拓扑或静态控制边界关系 owner 的有类型 LIR 序号与稳定标识。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum JunctionRelationOwner {
    /// 路口及其有类型身份。
    Junction(JunctionOrdinal, JunctionId),
    /// 转向动作及其有类型身份。
    Movement(MovementOrdinal, MovementId),
    /// 机动路径及其有类型身份。
    ManeuverPath(ManeuverPathOrdinal, ManeuverPathId),
    /// 停止线及其有类型身份。
    StopLine(StopLineOrdinal, StopLineId),
}

impl JunctionRelationOwner {
    /// 返回 owner 对应的 Identity v1 实体种类。
    #[must_use]
    pub const fn entity_kind(self) -> EntityKind {
        match self {
            Self::Junction(_, _) => EntityKind::Junction,
            Self::Movement(_, _) => EntityKind::Movement,
            Self::ManeuverPath(_, _) => EntityKind::ManeuverPath,
            Self::StopLine(_, _) => EntityKind::StopLine,
        }
    }
}

impl JunctionRelationSourceView<'_> {
    /// 返回关系 owner 的有类型 LIR 序号与稳定标识。
    #[must_use]
    pub const fn owner(&self) -> JunctionRelationOwner {
        match self.record.owner {
            JunctionRelationOwnerRecord::Junction(ordinal, stable_id) => {
                JunctionRelationOwner::Junction(ordinal, stable_id)
            }
            JunctionRelationOwnerRecord::Movement(ordinal, stable_id) => {
                JunctionRelationOwner::Movement(ordinal, stable_id)
            }
            JunctionRelationOwnerRecord::ManeuverPath(ordinal, stable_id) => {
                JunctionRelationOwner::ManeuverPath(ordinal, stable_id)
            }
            JunctionRelationOwnerRecord::StopLine(ordinal, stable_id) => {
                JunctionRelationOwner::StopLine(ordinal, stable_id)
            }
        }
    }

    /// 返回 owner-local 关系的有类型角色。
    #[must_use]
    pub const fn role(&self) -> SourceRelationRole {
        self.record.role
    }

    /// 返回同一 owner 与角色内的零基序号。
    ///
    /// `ManeuverPathEdge` 按完整路径序列计数；`ManeuverPathGate` 按转换位置计数；
    /// `ManeuverPathWaitingZone` 按入口、释放转换位置计数；`StopLineManeuverGate` 按所引用
    /// 机动门的 Stable ID 排序；`JunctionInternalEdge` 先按边的 Canonical LIR 序号排序，
    /// 再在同一路口内稠密计数。所有规则都不受来源声明排列影响。
    #[must_use]
    pub const fn local_index(&self) -> u32 {
        self.record.local_index
    }

    /// 返回显式关系或派生内部边声明的规范主要来源位置。
    #[must_use]
    pub fn primary_source(&self) -> SourceLocationView<'_> {
        self.source_map.location(self.record.primary)
    }

    /// 当前关系没有额外贡献来源；共享内部边只登记规范选定的主要路径来源。
    pub fn contributing_sources(&self) -> impl ExactSizeIterator<Item = SourceLocationView<'_>> {
        core::iter::empty()
    }
}

/// 信号所有权、程序或控制绑定关系 owner 的有类型身份。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SignalRelationOwner {
    /// 固定时制控制器及其有类型身份。
    SignalController(SignalControllerOrdinal, SignalControllerId),
    /// 控制器 owner-local 相位及其有类型身份。
    SignalPhase(SignalPhaseOrdinal, SignalPhaseId),
    /// 绑定信号组的机动门及其有类型身份。
    ManeuverGate(ManeuverGateOrdinal, ManeuverGateId),
}

impl SignalRelationOwner {
    /// 返回 owner 对应的 Identity v1 实体种类。
    #[must_use]
    pub const fn entity_kind(self) -> EntityKind {
        match self {
            Self::SignalController(_, _) => EntityKind::SignalController,
            Self::SignalPhase(_, _) => EntityKind::SignalPhase,
            Self::ManeuverGate(_, _) => EntityKind::ManeuverGate,
        }
    }
}

/// 一条固定时制信号 owner-local 关系来源记录的只读视图。
#[derive(Clone, Copy)]
pub struct SignalRelationSourceView<'a> {
    source_map: &'a ValidatedSourceMapInput,
    record: &'a SignalRelationSourceRecord,
}

impl SignalRelationSourceView<'_> {
    /// 返回关系 owner 的有类型 LIR 序号与稳定标识。
    #[must_use]
    pub const fn owner(&self) -> SignalRelationOwner {
        match self.record.owner {
            SignalRelationOwnerRecord::SignalController(ordinal, stable_id) => {
                SignalRelationOwner::SignalController(ordinal, stable_id)
            }
            SignalRelationOwnerRecord::SignalPhase(ordinal, stable_id) => {
                SignalRelationOwner::SignalPhase(ordinal, stable_id)
            }
            SignalRelationOwnerRecord::ManeuverGate(ordinal, stable_id) => {
                SignalRelationOwner::ManeuverGate(ordinal, stable_id)
            }
        }
    }

    /// 返回控制器成员、程序状态或门绑定的有类型角色。
    #[must_use]
    pub const fn role(&self) -> SourceRelationRole {
        self.record.role
    }

    /// 返回同一 owner 与角色内的零基序号。
    #[must_use]
    pub const fn local_index(&self) -> u32 {
        self.record.local_index
    }

    /// 返回建立该静态信号关系的主要来源位置。
    #[must_use]
    pub fn primary_source(&self) -> SourceLocationView<'_> {
        self.source_map.location(self.record.primary)
    }

    /// 当前显式信号关系没有额外贡献来源。
    pub fn contributing_sources(&self) -> impl ExactSizeIterator<Item = SourceLocationView<'_>> {
        core::iter::empty()
    }
}

/// 一条停车位 owner-local 关系来源记录的只读视图。
#[derive(Clone, Copy)]
pub struct ParkingRelationSourceView<'a> {
    source_map: &'a ValidatedSourceMapInput,
    record: &'a ParkingRelationSourceRecord,
}

impl ParkingRelationSourceView<'_> {
    /// 返回拥有区域归属或锚点关系的停车位序号。
    #[must_use]
    pub const fn owner_ordinal(&self) -> ParkingSpaceOrdinal {
        self.record.owner_ordinal
    }

    /// 返回拥有该关系的停车位稳定标识。
    #[must_use]
    pub const fn owner_stable_id(&self) -> ParkingSpaceId {
        self.record.owner_stable_id
    }

    /// 返回区域归属、入口锚点或出口锚点角色。
    #[must_use]
    pub const fn role(&self) -> SourceRelationRole {
        self.record.role
    }

    /// 返回同一停车位与角色内的零基下标；当前三个角色都只有下标 `0`。
    #[must_use]
    pub const fn local_index(&self) -> u32 {
        self.record.local_index
    }

    /// 返回建立该停车静态关系的主要来源位置。
    #[must_use]
    pub fn primary_source(&self) -> SourceLocationView<'_> {
        self.source_map.location(self.record.primary)
    }

    /// 当前显式停车关系没有额外贡献来源。
    pub fn contributing_sources(&self) -> impl ExactSizeIterator<Item = SourceLocationView<'_>> {
        core::iter::empty()
    }
}

/// 一条 canonical frame 到中心线的 owner-local 来源记录。
#[derive(Clone, Copy)]
pub struct SpatialRelationSourceView<'a> {
    source_map: &'a ValidatedSourceMapInput,
    record: &'a SpatialRelationSourceRecord,
}

impl SpatialRelationSourceView<'_> {
    #[must_use]
    pub const fn owner_ordinal(&self) -> CanonicalFrameOrdinal {
        self.record.owner_ordinal
    }

    #[must_use]
    pub const fn owner_stable_id(&self) -> CanonicalFrameId {
        self.record.owner_stable_id
    }

    #[must_use]
    pub const fn role(&self) -> SourceRelationRole {
        self.record.role
    }

    #[must_use]
    pub const fn local_index(&self) -> u32 {
        self.record.local_index
    }

    #[must_use]
    pub fn primary_source(&self) -> SourceLocationView<'_> {
        self.source_map.location(self.record.primary)
    }

    pub fn contributing_sources(&self) -> impl ExactSizeIterator<Item = SourceLocationView<'_>> {
        core::iter::empty()
    }
}

/// 参与者类别继承或准入规则关系 owner 的有类型身份。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AccessRelationOwner {
    /// 参与者类别及其有类型身份。
    ParticipantClass(ParticipantClassOrdinal, ParticipantClassId),
    /// 当前道路机动车车辆配置及其有类型身份。
    VehicleProfile(VehicleProfileOrdinal, VehicleProfileId),
    /// 准入规则及其有类型身份。
    AccessRule(AccessRuleOrdinal, AccessRuleId),
}

impl AccessRelationOwner {
    /// 返回 owner 对应的 Identity v1 实体种类。
    #[must_use]
    pub const fn entity_kind(self) -> EntityKind {
        match self {
            Self::ParticipantClass(_, _) => EntityKind::ParticipantClass,
            Self::VehicleProfile(_, _) => EntityKind::VehicleProfile,
            Self::AccessRule(_, _) => EntityKind::AccessRule,
        }
    }
}

/// 一条类别继承、规则目标或规则类别选择器的来源视图。
#[derive(Clone, Copy)]
pub struct AccessRelationSourceView<'a> {
    source_map: &'a ValidatedSourceMapInput,
    record: &'a AccessRelationSourceRecord,
}

impl AccessRelationSourceView<'_> {
    /// 返回关系 owner 的有类型 LIR 序号与稳定标识。
    #[must_use]
    pub const fn owner(&self) -> AccessRelationOwner {
        match self.record.owner {
            AccessRelationOwnerRecord::ParticipantClass(ordinal, stable_id) => {
                AccessRelationOwner::ParticipantClass(ordinal, stable_id)
            }
            AccessRelationOwnerRecord::VehicleProfile(ordinal, stable_id) => {
                AccessRelationOwner::VehicleProfile(ordinal, stable_id)
            }
            AccessRelationOwnerRecord::AccessRule(ordinal, stable_id) => {
                AccessRelationOwner::AccessRule(ordinal, stable_id)
            }
        }
    }

    /// 返回继承、目标或类别选择器角色。
    #[must_use]
    pub const fn role(&self) -> SourceRelationRole {
        self.record.role
    }

    /// 返回同一 owner 与角色内的规范零基下标。
    #[must_use]
    pub const fn local_index(&self) -> u32 {
        self.record.local_index
    }

    /// 返回关系引用的精确来源位置。
    #[must_use]
    pub fn primary_source(&self) -> SourceLocationView<'_> {
        self.source_map.location(self.record.primary)
    }

    /// 当前显式准入关系没有额外贡献来源。
    pub fn contributing_sources(&self) -> impl ExactSizeIterator<Item = SourceLocationView<'_>> {
        core::iter::empty()
    }
}

/// 一条 owner-local 下游连接来源记录的只读视图。
#[derive(Clone, Copy)]
pub struct LaneEdgeSuccessorSourceView<'a> {
    source_map: &'a ValidatedSourceMapInput,
    record: &'a LaneEdgeSuccessorSourceRecord,
}

impl LaneEdgeSuccessorSourceView<'_> {
    /// 返回拥有该关系的稳定实体序号。
    #[must_use]
    pub const fn owner_ordinal(&self) -> LaneEdgeOrdinal {
        self.record.owner_ordinal
    }

    /// 返回拥有该关系的稳定实体标识。
    #[must_use]
    pub const fn owner_stable_id(&self) -> LaneEdgeId {
        self.record.owner_stable_id
    }

    /// 返回 owner-local 关系的有类型角色。
    #[must_use]
    pub const fn role(&self) -> SourceRelationRole {
        self.record.role
    }

    /// 返回本次编译中、同一 owner 与角色内的零基序号。
    #[must_use]
    pub const fn local_index(&self) -> u32 {
        self.record.local_index
    }

    /// 返回显式关系声明的主要来源位置。
    #[must_use]
    pub fn primary_source(&self) -> SourceLocationView<'_> {
        self.source_map.location(self.record.primary)
    }

    /// 返回生成关系的贡献来源；当前显式 successor 关系没有推导链。
    pub fn contributing_sources(&self) -> impl ExactSizeIterator<Item = SourceLocationView<'_>> {
        core::iter::empty()
    }
}

/// 一条静态路线 owner-local 关系来源记录的只读视图。
#[derive(Clone, Copy)]
pub struct RouteRelationSourceView<'a> {
    source_map: &'a ValidatedSourceMapInput,
    record: &'a RouteRelationSourceRecord,
}

impl RouteRelationSourceView<'_> {
    /// 返回拥有该出现项的静态路线序号。
    #[must_use]
    pub const fn owner_ordinal(&self) -> StaticRouteOrdinal {
        self.record.owner_ordinal
    }

    /// 返回拥有该出现项的静态路线稳定标识。
    #[must_use]
    pub const fn owner_stable_id(&self) -> StaticRouteId {
        self.record.owner_stable_id
    }

    /// 返回边、机动路径、机动门或等待区出现项角色。
    #[must_use]
    pub const fn role(&self) -> SourceRelationRole {
        self.record.role
    }

    /// 返回同一路线和角色中的零基局部下标。
    #[must_use]
    pub const fn local_index(&self) -> u32 {
        self.record.local_index
    }

    /// 返回声明边引用或派生出现项所锚定的路线边引用位置。
    pub fn primary_source(&self) -> SourceLocationView<'_> {
        self.source_map.location(self.record.primary)
    }

    /// 返回生成出现项所依赖的静态控制声明位置。
    ///
    /// 显式 `StaticRouteEdge` 没有贡献来源；机动、门和等待区出现项分别返回对应
    /// `ManeuverPath`、`ManeuverGate` 或 `WaitingZone` 声明位置。
    pub fn contributing_sources(&self) -> impl ExactSizeIterator<Item = SourceLocationView<'_>> {
        self.record
            .contributing
            .into_iter()
            .map(|record| self.source_map.location(record))
    }
}
