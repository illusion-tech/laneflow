use laneflow_static_contract::{EntityKind, StableId128};

/// 共享根独立闭合策略失败的原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyBuildViolation {
    /// 策略局部表（evidence/gap/stream/gate）的行序未按 `(policy, key)` 严格递增
    /// （`build_shared_network_revision` 策略闭合）。
    CanonicalMembers,
    /// 策略引用越界或缺失：策略/owner/成员序号越界、局部 key 或机动门查不到、
    /// 参与者类别选择器为空、StableId 缺失或非升序（`build_shared_network_revision`
    /// 策略闭合）。
    Reference,
    /// 流/门规则的 evidence key 序非严格递增，或规则无任何 evidence 引用且所属
    /// 策略未声明来源文本（`build_shared_network_revision` 策略闭合）。
    Evidence,
    /// 流规则的让行目标列表与间隙参数档绑定不一致：声明目标必须给出 gap key，
    /// 缺省目标必须省略 gap（`build_shared_network_revision` 策略闭合）。
    GapBinding,
    /// 策略表的 `(jurisdiction, version)` 与 AccessRule 段唯一声明的法规不一致
    /// （`build_shared_network_revision` 策略闭合）。
    RegulationMismatch,
    /// 门的解释与信号组绑定矛盾：受控/非受控与有无信号组不匹配，或无信号组的
    /// 门声明红灯禁止（`build_shared_network_revision` 策略闭合）。
    SignalBinding,
    /// 右转灯型解释（圆形右转/方向右转保护/许可）落在非右转机动上
    /// （`build_shared_network_revision` 策略闭合）。
    RightTurnRequired,
    /// 同一机动上的多个门声明互斥的右转灯型（`build_shared_network_revision`
    /// 策略闭合）。
    LampTypeConflict,
    /// 流规则的让行目标包含所属流自身（`build_shared_network_revision` 策略闭合）。
    SelfYield,
    /// 让行目标流与所属流不共享任何冲突区通行段（`build_shared_network_revision`
    /// 策略闭合）。
    DisjointYield,
    /// 某 `(owner, class)` 允许单元没有任何类选择器覆盖它的适用规则
    /// （`build_shared_network_revision` 策略闭合）。
    MissingRule,
    /// 适用规则中出现相同的（类选择器深度， 优先级）最高排名，无法唯一裁决
    /// （`build_shared_network_revision` 策略闭合）。
    AmbiguousRule,
    /// 让行目标的最低优先级不严格低于让行方，流间让行关系不成严格次序
    /// （`build_shared_network_revision` 策略闭合）。
    YieldPriority,
    /// 同一冲突区内的 protected 门绑定到不同信号 controller，或同一策略的不同
    /// protected 冲突区争用同一绿灯 phase（`build_shared_network_revision` 策略闭合）。
    ProtectedConflict,
}

/// 构建失败涉及的稳定结构分类。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildStructure {
    ContractVersions,
    CanonicalIdentity,
    CanonicalEntityTable,
    LaneEdge,
    LaneSuccessors,
    LanePredecessors,
    ManeuverPath,
    ManeuverCandidates,
    Conflict,
    Policy,
    PolicyWork,
    PlanningHints,
    ExecutionContract,
    SpatialPresence,
    LaneEdgeGeometry,
    FacilityBandGeometry,
    ConflictZoneRegion,
    RelationClosure,
    AccessPlane,
    RetainedOutput,
    BuilderScratch,
}

/// 构建失败的粗粒度稳定分类。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildErrorClass {
    InputInvariant,
    Order,
    Reference,
    Identity,
    Contract,
    Spatial,
    Budget,
    Arithmetic,
    Allocation,
    Cancelled,
}

/// 受检 LFCA 无法闭合为共享静态路网的稳定错误。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BuildError {
    /// 策略闭合失败，携带策略序号与具体 `PolicyBuildViolation`
    /// （`build_shared_network_revision` 策略阶段）。
    Policy {
        policy: u32,
        violation: PolicyBuildViolation,
    },
    /// 受检 LFCA 违反结构不变量：段/表缺失、字段值类型不符、成员记录缺失或实体
    /// 归属不闭合等（`build_shared_network_revision`；structure 标明所属组件）。
    InputInvariant { structure: BuildStructure },
    /// 实体行声明的 typed ordinal 与按行序派生的期望序号不一致
    /// （`build_shared_network_revision` 的身份表、实体表与关系闭合扫描）。
    UnexpectedOrdinal {
        structure: BuildStructure,
        expected: u32,
        actual: u32,
    },
    /// 规范身份表的实体种类序列出现回退（`build_shared_network_revision`
    /// 的身份闭合）。
    EntityKindOrder {
        previous: EntityKind,
        actual: EntityKind,
    },
    /// 某实体种类的身份条目数与实体表行数不一致（`build_shared_network_revision`
    /// 的身份闭合）。
    EntityCountMismatch {
        entity_kind: EntityKind,
        identity_count: u32,
        entity_count: u32,
    },
    /// 实体表行的 StableId128 与身份表同种类同序号声明不一致
    /// （`build_shared_network_revision` 的实体表扫描）。
    StableIdMismatch {
        entity_kind: EntityKind,
        ordinal: u32,
    },
    /// 两个规范身份条目声明同一 StableId128（`build_shared_network_revision`
    /// 的反向身份基数排序）。
    DuplicateStableId { stable_id: StableId128 },
    /// 结构内的序号引用越过目标实体数上限（`build_shared_network_revision`；
    /// 如身份序号、Access/authoring 引用、几何引用）。
    ReferenceOutOfBounds {
        structure: BuildStructure,
        ordinal: u32,
        limit: u32,
    },
    /// 要求规范有序的成员或位置序列出现非严格递增（`build_shared_network_revision`
    /// 的关系闭合）。
    NonCanonicalOrder {
        structure: BuildStructure,
        previous: u32,
        actual: u32,
    },
    /// 受检 LFCA 携带的静态契约版本不受支持（ContractVersions），或与
    /// ExecutionContract 行声明的执行/约束契约版本不一致（`build_shared_network_revision`）。
    ContractMismatch { structure: BuildStructure },
    /// 头部声明的 Spatial presence 标记与实际 spatial payload（方向 profile、
    /// 规范系、车道/设施几何、冲突区面）不一致；无 spatial payload 的 headless
    /// LFCA 是合法输入（`build_shared_network_revision`）。
    SpatialPresenceMismatch,
    /// 车道几何行数与 LaneEdge 实体数不一致（`build_shared_network_revision`
    /// 的 spatial 构建）。
    SpatialCoverageMismatch { lane_edges: u32, geometries: u32 },
    /// 车道几何弧长与同车道交通网络的毫米长度不匹配（超出绝对/相对容差；
    /// `build_shared_network_revision` 的 spatial 构建）。
    SpatialLengthMismatch {
        lane_edge: u32,
        traffic_length_mm: u32,
        spatial_length_meters: f32,
    },
    /// 相连通的前后车道边引用不同的规范系（`build_shared_network_revision`
    /// 的几何连通校验）。
    SpatialFrameMismatch {
        predecessor: u32,
        successor: u32,
        predecessor_frame: u32,
        successor_frame: u32,
    },
    /// 相连通的前后车道边端点间隙超过拼接位置容差（`build_shared_network_revision`
    /// 的几何连通校验）。
    SpatialJoinGapMismatch {
        predecessor: u32,
        successor: u32,
        gap_meters: f32,
        tolerance_meters: f32,
    },
    /// retained/scratch/work 构建预算任一超限（`build_shared_network_revision`；
    /// 如最终 retained 复核、关系 intern 表、策略工作预算）。
    BudgetExceeded {
        structure: BuildStructure,
        required: u64,
        limit: u64,
    },
    /// 构建期计数、容量或偏移派生的 checked 算术溢出
    /// （`build_shared_network_revision`；structure 标明所属组件）。
    ArithmeticOverflow { structure: BuildStructure },
    /// 构建缓冲的容量预留失败（`build_shared_network_revision`；structure 标明
    /// 分配所属组件）。
    AllocationFailure { structure: BuildStructure },
    /// 同一 Access 单元与参与者类别在相同深度/目标具体度/优先级下同时命中
    /// allow 与 deny 规则（`build_shared_network_revision` 的 Access 闭合）。
    AccessAmbiguity {
        plane: &'static str,
        unit: u32,
        class: u32,
        first_rule: u32,
        second_rule: u32,
    },
    /// 调用方提供的取消标志已置位（`build_shared_network_revision` 的各阶段
    /// 取消检查点）。
    Cancelled,
}

impl BuildError {
    /// 返回该构建错误的类别，供宿主按类归并诊断。
    #[must_use]
    pub const fn class(self) -> BuildErrorClass {
        match self {
            Self::Policy { .. } => BuildErrorClass::Reference,
            Self::InputInvariant { .. } => BuildErrorClass::InputInvariant,
            Self::UnexpectedOrdinal { .. }
            | Self::EntityKindOrder { .. }
            | Self::NonCanonicalOrder { .. } => BuildErrorClass::Order,
            Self::ReferenceOutOfBounds { .. } => BuildErrorClass::Reference,
            Self::EntityCountMismatch { .. }
            | Self::StableIdMismatch { .. }
            | Self::DuplicateStableId { .. } => BuildErrorClass::Identity,
            Self::ContractMismatch { .. } => BuildErrorClass::Contract,
            Self::SpatialPresenceMismatch
            | Self::SpatialCoverageMismatch { .. }
            | Self::SpatialLengthMismatch { .. }
            | Self::SpatialFrameMismatch { .. }
            | Self::SpatialJoinGapMismatch { .. } => BuildErrorClass::Spatial,
            Self::BudgetExceeded { .. } => BuildErrorClass::Budget,
            Self::ArithmeticOverflow { .. } => BuildErrorClass::Arithmetic,
            Self::AllocationFailure { .. } => BuildErrorClass::Allocation,
            Self::AccessAmbiguity { .. } => BuildErrorClass::Reference,
            Self::Cancelled => BuildErrorClass::Cancelled,
        }
    }
}
