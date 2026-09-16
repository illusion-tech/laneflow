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
    /// 同一门（按门下标累计比较）重复声明互斥的右转灯型；不同门之间不比较
    /// （`build_shared_network_revision` 策略闭合）。
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
    /// 让行目标可解析的最低优先级数值不严格大于让行方（目标 ≤ 让行方），流间
    /// 让行关系不成严格次序（`build_shared_network_revision` 策略闭合）。
    YieldPriority,
    /// 同一冲突区内的 protected 门绑定到不同信号 controller，或同一冲突区内
    /// 重复绑定同一绿灯 phase（跨冲突区的同 phase 不触发；
    /// `build_shared_network_revision` 策略闭合）。
    ProtectedConflict,
}

/// 构建失败涉及的稳定结构分类。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildStructure {
    /// 静态契约版本表。
    ContractVersions,
    /// 规范身份表。
    CanonicalIdentity,
    /// 规范实体段与实体表。
    CanonicalEntityTable,
    /// `LaneEdge` 实体表行。
    LaneEdge,
    /// `LaneEdge` 的后继引用向量及其派生对。
    LaneSuccessors,
    /// `LaneEdge` 的前驱索引构建。
    LanePredecessors,
    /// `ManeuverPath` 实体表行及其行内引用。
    ManeuverPath,
    /// 机动候选表（`ManeuverGate` 与 `StopLine` 行）。
    ManeuverCandidates,
    /// Conflict component 构建。
    Conflict,
    /// 路权策略闭合。
    Policy,
    /// 策略解析的工作预算。
    PolicyWork,
    /// 分区规划提示派生。
    PlanningHints,
    /// ExecutionContract 行。
    ExecutionContract,
    /// Spatial presence 标记行。
    SpatialPresence,
    /// `LaneEdge` 几何表。
    LaneEdgeGeometry,
    /// `FacilityBand` 几何表。
    FacilityBandGeometry,
    /// 冲突区空间区域表。
    ConflictZoneRegion,
    /// 静态关系闭合。
    RelationClosure,
    /// Access 准入平面构建。
    AccessPlane,
    /// 构建输出的保留内存预算。
    RetainedOutput,
    /// 构建过程的暂存内存预算。
    BuilderScratch,
}

/// 构建失败的粗粒度稳定分类。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildErrorClass {
    /// 输入违反结构不变量。
    InputInvariant,
    /// 序次失败：typed ordinal 与期望不符或序列非严格递增。
    Order,
    /// 引用失败：引用越界与全部策略闭合失败（`BuildError::Policy` 整族归本类，
    /// 含法规不匹配、信号绑定等）或 Access 规则歧义（同类深度、目标
    /// 具体度与优先级并列且效果相反时判歧义；效果相同的并列按最小规范 ordinal
    /// 确定性取胜、不报歧义）。
    Reference,
    /// 身份失败：实体计数不一致或 StableId 不一致、重复。
    Identity,
    /// 静态契约版本不受支持或声明不一致。
    Contract,
    /// Spatial payload 覆盖、长度或几何校验失败。
    Spatial,
    /// retained/scratch/work 构建预算超限。
    Budget,
    /// 构建期 checked 算术溢出。
    Arithmetic,
    /// 构建缓冲的容量预留失败。
    Allocation,
    /// 调用方取消构建。
    Cancelled,
}

/// 受检 LFCA 无法闭合为共享静态路网的稳定错误。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BuildError {
    /// 策略闭合失败，携带策略序号与具体 `PolicyBuildViolation`
    /// （`build_shared_network_revision` 策略阶段）。
    Policy {
        /// 违反闭合的路权策略集序号。
        policy: u32,
        /// 具体的策略违反类型。
        violation: PolicyBuildViolation,
    },
    /// 受检 LFCA 违反结构不变量：段/表缺失、字段值类型不符、成员记录缺失或实体
    /// 归属不闭合等（`build_shared_network_revision`；structure 标明所属组件）。
    InputInvariant {
        /// 违反不变量所属的稳定结构分类。
        structure: BuildStructure,
    },
    /// 实体行声明的 typed ordinal 与按行序派生的期望序号不一致
    /// （`build_shared_network_revision` 的身份表、实体表、关系闭合与车道几何行
    /// 扫描）。
    UnexpectedOrdinal {
        /// 出现序号不符的稳定结构分类。
        structure: BuildStructure,
        /// 按行序派生的期望序号。
        expected: u32,
        /// 行实际声明的序号；`LaneEdgeGeometry` 行为所有者 ordinal 而非实体表
        /// 行序。
        actual: u32,
    },
    /// 规范身份表的实体种类序列出现回退（`build_shared_network_revision`
    /// 的身份闭合）。
    EntityKindOrder {
        /// 序列中前一个实体种类。
        previous: EntityKind,
        /// 出现回退的实体种类。
        actual: EntityKind,
    },
    /// 某实体种类的身份条目数与实体表行数不一致（`build_shared_network_revision`
    /// 的身份闭合）。
    EntityCountMismatch {
        /// 计数不一致的实体种类。
        entity_kind: EntityKind,
        /// 身份表中该种类的条目数。
        identity_count: u32,
        /// 实体表中该种类的行数。
        entity_count: u32,
    },
    /// 实体表行的 StableId128 与身份表同种类同序号声明不一致
    /// （`build_shared_network_revision` 的实体表扫描）。
    StableIdMismatch {
        /// 声明不一致的实体种类。
        entity_kind: EntityKind,
        /// 该种类内的实体序号。
        ordinal: u32,
    },
    /// 两个规范身份条目声明同一 StableId128（`build_shared_network_revision`
    /// 的反向身份基数排序）。
    DuplicateStableId {
        /// 被重复声明的 StableId128。
        stable_id: StableId128,
    },
    /// 结构内的序号引用越过目标实体数上限（`build_shared_network_revision`；
    /// 如身份序号、Access/authoring 引用、几何引用）。
    ReferenceOutOfBounds {
        /// 引用越界所属的稳定结构分类。
        structure: BuildStructure,
        /// 越界的引用序号。
        ordinal: u32,
        /// 目标实体的数量上限。
        limit: u32,
    },
    /// 要求规范有序的成员或位置序列出现非严格递增（`build_shared_network_revision`
    /// 的关系闭合与 spatial 构建——设施带几何行、冲突区 region 行同样要求严格
    /// 递增）。
    NonCanonicalOrder {
        /// 序列非严格递增所属的稳定结构分类。
        structure: BuildStructure,
        /// 序列中前一项的值；冲突通行段行的负载为复合排序键
        /// `(entry_position, exit_position, stable_id)` 的对应分量。
        previous: u32,
        /// 破坏严格递增的值；同上，随结构种类取对应分量。
        actual: u32,
    },
    /// 受检 LFCA 携带的静态契约版本不受支持（ContractVersions），或与
    /// ExecutionContract 行声明的执行/约束契约版本不一致（`build_shared_network_revision`）。
    ContractMismatch {
        /// 契约版本不受支持或声明不一致所属的稳定结构分类。
        structure: BuildStructure,
    },
    /// 头部声明的 Spatial presence 标记与实际 spatial payload（方向 profile、
    /// 规范系、车道/设施几何、冲突区面）不一致；无 spatial payload 的 headless
    /// LFCA 是合法输入（`build_shared_network_revision`）。
    SpatialPresenceMismatch,
    /// 车道几何行数与 LaneEdge 实体数不一致；零几何行在 presence 矩阵允许的
    /// 无车道几何形态（headless、仅 profile、仅 frame、仅设施带等）下显式跳过
    /// 本检查（`build_shared_network_revision` 的 spatial 构建）。
    SpatialCoverageMismatch {
        /// `LaneEdge` 实体数。
        lane_edges: u32,
        /// 车道几何行数。
        geometries: u32,
    },
    /// 车道几何弧长与同车道交通网络的毫米长度不匹配（超出绝对/相对容差；
    /// `build_shared_network_revision` 的 spatial 构建）。
    SpatialLengthMismatch {
        /// 长度不匹配的车道边序号。
        lane_edge: u32,
        /// 交通网络声明的车道长度（毫米）。
        traffic_length_mm: u32,
        /// 车道几何的弧长（米）。
        spatial_length_meters: f32,
    },
    /// 相连通的前后车道边引用不同的规范系（`build_shared_network_revision`
    /// 的几何连通校验）。
    SpatialFrameMismatch {
        /// 前驱车道边序号。
        predecessor: u32,
        /// 后继车道边序号。
        successor: u32,
        /// 前驱边引用的规范系序号。
        predecessor_frame: u32,
        /// 后继边引用的规范系序号。
        successor_frame: u32,
    },
    /// 相连通的前后车道边端点间隙超过拼接位置容差（`build_shared_network_revision`
    /// 的几何连通校验）。
    SpatialJoinGapMismatch {
        /// 前驱车道边序号。
        predecessor: u32,
        /// 后继车道边序号。
        successor: u32,
        /// 前驱末点与后继首点的间隙（米）。
        gap_meters: f32,
        /// 允许的拼接位置容差（米）。
        tolerance_meters: f32,
    },
    /// retained/scratch/work 构建预算任一超限（`build_shared_network_revision`；
    /// 如最终 retained 复核、关系 intern 表、策略工作预算）。
    BudgetExceeded {
        /// 超限预算所属的稳定结构分类。
        structure: BuildStructure,
        /// 实际需要的预算量（字节或工作计数，随 structure 而定）。
        required: u64,
        /// 对应预算的上限（字节或工作计数，随 structure 而定）。
        limit: u64,
    },
    /// 构建期计数、容量或偏移派生的 checked 算术溢出
    /// （`build_shared_network_revision`；structure 标明所属组件）。
    ArithmeticOverflow {
        /// 溢出发生所属的稳定结构分类。
        structure: BuildStructure,
    },
    /// 构建缓冲的容量预留失败（`build_shared_network_revision`；structure 标明
    /// 分配所属组件）。
    AllocationFailure {
        /// 分配失败所属的稳定结构分类。
        structure: BuildStructure,
    },
    /// 同一 Access 单元与参与者类别在相同深度/目标具体度/优先级下同时命中
    /// allow 与 deny 规则；效果相同的并列按最小规范 ordinal 确定性取胜、
    /// 不报歧义（`build_shared_network_revision` 的 Access 闭合）。
    AccessAmbiguity {
        /// 发生歧义的 Access 平面标识。
        plane: &'static str,
        /// 该平面内发生歧义的单元序号。
        unit: u32,
        /// 发生歧义的参与者类别序号。
        class: u32,
        /// 相互冲突的第一条规则序号。
        first_rule: u32,
        /// 相互冲突的第二条规则序号。
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
