//! 共享路网修订认证（#959 切片 A；`traffic-runtime-state-integrity.md` §4.2）。
//!
//! 宿主在安装、恢复、Spatial 绑定与切换之前及在用修订审计时调用的只读入口：
//! ① 来源准备——reader 受检、同修订判据、按同一 builder 完整构建候选修订
//! （不读取被认证修订）；② 修订比较——`origin` 逐字段与期望值比对、候选修订与
//! 被认证修订逐 component / 表按值比对；③ 确认与处置——来源复核与按首次失配
//! 位置归类的复算确认（设计 §2.2 复算归类表）。认证只读不可变修订，不进入
//! tick 路径，不给修订引入可变状态或常驻完整性状态；结论只覆盖每个字节
//! 被读取的时刻，不声称结论交付时刻整份修订无损（设计 §4.2 观测时刻）。

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Instant;

use laneflow_format::{
    BoundedReReadableObjectSource, CanonicalNetworkInputError, FormatError, FormatLimits,
    check_canonical_network_input,
};
use laneflow_static_contract::{ExactByteLength, NetworkRevisionId, Sha256Digest};
use sha2::{Digest, Sha256};

use crate::attestation_compare::{compare_origin, compare_revisions_by_value};
use crate::builder::{SharedNetworkBuildOptions, build_shared_network_revision};
use crate::{BuildError, SharedNetworkRevision};

/// 修订认证三值结论（完整性设计 §7）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttestationVerdict {
    /// 本次认证覆盖的内容在各自被读取时一致；不外推到读取时刻之外。
    Pass,
    /// 期望值与重算值不等，复算确认仍不等。
    Mismatch,
    /// 认证过程不可信（设计 §2.4 `Untrusted` 生产者穷尽清单）。
    Untrusted,
}

/// 失配判因（完整性设计 §7 判因轴；不依赖「修订 ↔ 来源」绑定）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttestationCause {
    /// 来源准备阶段失败：坏的是审计输入，不是修订。
    Source,
    /// 修订比较阶段逐表失配（revision 已相等且来源复核确认字节未变）。
    SharedStaticRoot,
    /// 无法判因：修订头部失配、Spatial 存在性失配或认证过程不可信。
    Indeterminate,
}

/// 认证阶段（设计 §4.2 三段边界中可失败的两段）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttestationStage {
    /// 来源准备：reader、同修订判据与候选修订完整构建。
    SourcePreparation,
    /// 修订比较：`origin` 与期望值、候选修订与被认证修订逐表比对。
    RootComparison,
}

/// 修订来源期望值（完整性设计 §2.2；任何值不得取自被认证修订自身的 `origin`）。
///
/// 发布来源由宿主随 [`crate::CanonicalNetworkOrigin`] 之外的
/// `PublishedLfcaReference` 持久：`NetworkRevisionId` 参与同修订判据；
/// LFCA digest / exact length 在任何调用时机都没有独立期望值，只作来源
/// 审计诊断（设计 §2.2「发布来源的字节身份」）。编辑来源期望值随 #529 落地。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootAttestationExpectation {
    network_revision: NetworkRevisionId,
    reference_digest: Sha256Digest,
    reference_byte_length: ExactByteLength,
}

impl RootAttestationExpectation {
    /// 由宿主持有的来源引用组装期望值。
    #[must_use]
    pub const fn new(
        network_revision: NetworkRevisionId,
        reference_digest: Sha256Digest,
        reference_byte_length: ExactByteLength,
    ) -> Self {
        Self {
            network_revision,
            reference_digest,
            reference_byte_length,
        }
    }

    /// 同修订判据的比对对象。
    #[must_use]
    pub const fn network_revision(&self) -> NetworkRevisionId {
        self.network_revision
    }

    /// 引用登记的 LFCA 摘要（仅来源审计诊断）。
    #[must_use]
    pub const fn reference_digest(&self) -> Sha256Digest {
        self.reference_digest
    }

    /// 引用登记的 LFCA 精确字节长度（仅来源审计诊断）。
    #[must_use]
    pub const fn reference_byte_length(&self) -> ExactByteLength {
        self.reference_byte_length
    }
}

/// 失配所在的 `origin` 字段（字节身份字段不在比对面内）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OriginField {
    /// 路网修订标识。
    NetworkRevision,
    /// LFCA 格式版本。
    CanonicalFormatVersion,
    /// 标识编码版本。
    IdentityEncodingVersion,
    /// 标识注册表修订。
    IdentityRegistryRevision,
    /// 路网修订派生版本。
    NetworkRevisionDerivationVersion,
    /// 约束契约版本。
    ConstraintContractVersion,
    /// 静态执行契约版本。
    StaticExecutionContractVersion,
    /// 分区规划提示派生版本。
    PartitionPlanningHintsDerivationVersion,
}

/// 逐表比对报告的 component。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootComponent {
    /// Traffic 静态列。
    Traffic,
    /// 路口机动路径与 transition candidate。
    Maneuver,
    /// Runtime 关系闭包。
    Relations,
    /// 稳定身份索引。
    Identity,
    /// 分区规划提示（派生表）。
    PlanningHints,
    /// 冲突静态语义与派生覆盖索引。
    Conflict,
    /// 路权解析表。
    Policy,
    /// Spatial component。
    Spatial,
}

/// 来源准备阶段失败的完整证据（设计 §2.2：证据含所在步骤与完整错误变体与载荷）。
#[derive(Clone, Debug, PartialEq)]
pub enum SourceFailureDetail {
    /// reader 受检拒绝（内容类；完整错误变体与载荷即证据身份）。
    Reader(CanonicalNetworkInputError),
    /// 同修订判据失配：宿主字节重算的修订与引用不等。
    RevisionCriterion {
        /// 引用登记的修订。
        reference: NetworkRevisionId,
        /// 按宿主字节重算的修订。
        recomputed: NetworkRevisionId,
    },
    /// builder 内容类闭合失败（完整错误变体与载荷即证据身份）。
    Builder(BuildError),
    /// 来源复核发现来源字节在验证之后改变（设计 §4.2 确认与处置）。
    SourceBytesChanged {
        /// 来源准备阶段读取的摘要。
        prepared_digest: Sha256Digest,
        /// 来源准备阶段读取的精确字节长度。
        prepared_byte_length: u64,
        /// 复核重算的摘要。
        recheck_digest: Sha256Digest,
        /// 复核重算的精确字节长度。
        recheck_byte_length: u64,
    },
    /// 来源准备阶段 panic（按来源失败归类，设计 §2.4）。
    Panic(String),
}

/// 失配现场（L2）：所在阶段与首个不等的逻辑位置、期望 / 实际值
/// （无值对的错误路径为完整错误变体与载荷）。
#[derive(Clone, Debug, PartialEq)]
pub enum MismatchSite {
    /// 来源准备阶段失败（判因 `来源`）。
    SourceFailure {
        /// 完整失败证据。
        detail: SourceFailureDetail,
    },
    /// `origin` 字段与期望值失配（判因 `无法判因`）。
    OriginField {
        /// 失配字段。
        field: OriginField,
        /// 期望值文本。
        expected: String,
        /// 实际值文本。
        actual: String,
    },
    /// 表内首个不等元素与字段。
    TableValue {
        /// 所属 component。
        component: RootComponent,
        /// 表名。
        table: &'static str,
        /// 按 `EntityKind` 展开的表（identity 正向索引）所属实体种类；
        /// 其他表为 `None`。进入证据身份：失配移到另一个 kind 即不同证据。
        entity_kind: Option<laneflow_static_contract::EntityKind>,
        /// 元素序号。
        index: u32,
        /// 字段名。
        field: &'static str,
        /// 期望值文本（候选修订）。
        expected: String,
        /// 实际值文本（被认证修订）。
        actual: String,
    },
    /// 表长度失配（元素序号不适用）。
    TableLength {
        /// 所属 component。
        component: RootComponent,
        /// 表名。
        table: &'static str,
        /// 按 `EntityKind` 展开的表所属实体种类；其他表为 `None`。
        entity_kind: Option<laneflow_static_contract::EntityKind>,
        /// 期望长度（候选修订）。
        expected_len: u32,
        /// 实际长度（被认证修订）。
        actual_len: u32,
    },
    /// Spatial component 存在性失配（判因 `无法判因`：构建选项与修订失真不可区分）。
    SpatialPresence {
        /// 期望存在性（候选修订）。
        expected: bool,
        /// 实际存在性（被认证修订）。
        actual: bool,
    },
    /// 修订比较阶段 panic（直接判 `Untrusted`，设计 §2.4）。
    Panicked {
        /// panic 载荷文本。
        payload: String,
    },
}

/// 一条失配证据（含所在阶段）。
#[derive(Clone, Debug, PartialEq)]
pub struct MismatchEvidence {
    /// 失配 / 失败所在阶段。
    pub stage: AttestationStage,
    /// 失配现场。
    pub site: MismatchSite,
}

/// 来源审计诊断（不进入结论；同修订重发布等字节身份差异按现行合同合法）。
#[derive(Clone, Debug, PartialEq)]
pub enum SourceAuditNote {
    /// 宿主字节的 LFCA digest / exact length 与引用登记不同（同修订重发布；
    /// 设计 §2.2「发布来源的字节身份」、漏检面 §5 第 15 条）。
    ByteIdentityDiffersFromReference {
        /// 引用登记的摘要。
        reference_digest: Sha256Digest,
        /// 引用登记的精确字节长度。
        reference_byte_length: u64,
        /// 来源准备阶段实测的摘要。
        source_digest: Sha256Digest,
        /// 来源准备阶段实测的精确字节长度。
        source_byte_length: u64,
    },
    /// 首次来源侧失配后复算完整通过（首次失配记为来源审计诊断）。
    TransientSourceFailure {
        /// 首次失配证据。
        evidence: Box<MismatchEvidence>,
    },
    /// 首次来源侧失配后复算未完成（取消、预留 / 分配失败、限额超限）：
    /// 首次证据保留供重试对照，但**不得**记为瞬时恢复——复算没有通过。
    SourceRecomputeIncomplete {
        /// 首次失配证据。
        evidence: Box<MismatchEvidence>,
    },
    /// 首次来源侧失配后复算证据不同（来源证据不稳定）。
    SourceEvidenceUnstable {
        /// 首次失配证据。
        first: Box<MismatchEvidence>,
        /// 复算失配证据。
        recompute: Box<MismatchEvidence>,
    },
}

/// 操作失败的稳定分类（设计 §2.4；不进入三值结论、不判因、不触发停用）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationFailureKind {
    /// reader 超出宿主 `FormatLimits`（含 `Format` 内限额类拒绝）。
    ReaderLimitExceeded,
    /// LFCA 来源无法建立字节视图（与宿主未能提供来源同类）。
    ObjectSourceFailure,
    /// builder 调用方 retained / scratch / policy-work 预算超限。
    BuilderBudgetExceeded,
    /// builder 构建缓冲容量预留失败（#532 按计数可失败预留原语）。
    BuilderAllocationFailure,
    /// 调用方取消（只有宿主提供取消标志才可能发生）。
    Cancelled,
    /// 首次来源侧失配后复算证据不同（来源证据不稳定）。
    SourceEvidenceUnstable,
    /// 首次来源侧失配后复算未完成（取消、预留 / 分配失败、限额超限）。
    RecomputeIncomplete,
}

/// 一条操作失败（不进入三值结论）。
#[derive(Clone, Debug, PartialEq)]
pub struct OperationFailure {
    /// 稳定分类。
    pub kind: OperationFailureKind,
    /// 完整错误变体与载荷的文本（按原因重试的诊断输入）。
    pub detail: String,
    /// 操作失败前已收集的来源审计诊断（含首次 / 复算证据与字节身份差异；
    /// 设计 §2.2 复算归类表的证据不因操作失败丢失）。
    pub source_audit: Vec<SourceAuditNote>,
    /// 遥测边界吸收的 sink 失败 / panic 次数（记为遥测丢失，设计 §7；
    /// 不改变结论，也不经故障 sink 自身报告）。
    pub telemetry_dropped: u32,
}

/// 判因对应的处置提示（L3，完整性设计 §7；宿主按调用时机取用）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Disposition {
    /// 认证通过：无处置义务（`Pass` 不外推到读取时刻之外）。
    None,
    /// 修订不得投入或继续使用：判因 `共享路网修订` / `无法判因` 的 `Mismatch`
    /// 与全部 `Untrusted`；在用时按宿主侧停止边界停用并重建。
    DisableRoot,
    /// 判因 `来源`：坏的是审计输入，修订未被证伪；不停用、不重建修订。
    IsolateSource,
    /// 操作失败：不判因；按原因重试认证，修订照常使用。
    Retry,
}

impl Disposition {
    /// 投入使用前的处置提示。
    #[must_use]
    pub const fn pre_use_guidance(self) -> &'static str {
        match self {
            Self::None => "认证通过；结论只覆盖本次认证各字节被读取的时刻。",
            Self::DisableRoot => {
                "该修订不得投入使用；在用修订按停止边界停用并重建：不再发起新调用，在途同步调用正常返回，结论后返回的结果按已发布结果处置。"
            }
            Self::IsolateSource => {
                "该修订不投入使用；隔离或重新加载来源后重新认证（失配属来源侧，修订未被证伪）。"
            }
            Self::Retry => "该修订不投入使用；按操作失败原因重试认证。",
        }
    }

    /// 在用修订审计时的处置提示。
    #[must_use]
    pub const fn in_use_guidance(self) -> &'static str {
        match self {
            Self::None => "认证通过；结论只覆盖本次认证各字节被读取的时刻。",
            Self::DisableRoot => {
                "停用该修订的全部消费者（所有世界与 Spatial 会话）并重建：不再发起新调用，在途同步调用正常返回，结论后返回的结果按已发布结果处置。"
            }
            Self::IsolateSource => {
                "修订未被证伪：不停用、不重建；隔离或重新加载来源后重试审计，该次审计对修订无结论。"
            }
            Self::Retry => "该次审计无结论；按操作失败原因重试，修订照常使用，不停用。",
        }
    }
}

/// L0 Trace 步骤（默认关：仅当宿主提供遥测 sink 时产生）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceStep {
    /// 来源准备开始。
    SourcePreparationStarted,
    /// reader 受检通过。
    ReaderAccepted,
    /// 同修订判据通过。
    SameRevisionCriterionChecked,
    /// 候选修订构建完成。
    CandidateRootBuilt,
    /// 修订比较开始。
    RootComparisonStarted,
    /// 来源复核开始（只在失配路径）。
    SourceRecheckStarted,
    /// 复算开始（只在失配路径）。
    RecomputeStarted,
}

/// 认证结论报告（L1 Verdict + L2 Mismatch + L3 Disposition + 来源审计诊断）。
#[derive(Clone, Debug)]
pub struct AttestationReport {
    /// 三值结论。
    pub verdict: AttestationVerdict,
    /// 判因（`Mismatch` / `Untrusted` 携带；`Pass` 为 `None`）。
    pub cause: Option<AttestationCause>,
    /// L1：认证时刻（结论确定时刻；跨调用间隔由宿主计算，Runtime 不保留状态）。
    pub completed_at: Instant,
    /// L2：失配现场（仅 `Mismatch` / `Untrusted` 携带）。
    pub mismatch: Option<MismatchEvidence>,
    /// L3：处置提示。
    pub disposition: Disposition,
    /// 来源审计诊断（有界，仅在对应事件发生时记录）。
    pub source_audit: Vec<SourceAuditNote>,
    /// 遥测边界吸收的 sink 失败 / panic 次数（记为遥测丢失，设计 §7）。
    pub telemetry_dropped: u32,
}

/// 认证结果：三值结论或操作失败（操作失败不进三值，设计 §2.4）。
#[derive(Clone, Debug)]
#[must_use = "丢弃认证结果会静默丢弃结论、判因与处置提示（编译器警告，不构成处置强制）"]
pub enum AttestationOutcome {
    /// 认证完成，携带三值结论与 L1–L3 报告。
    Concluded(AttestationReport),
    /// 操作失败：不判因、不触发停用、不宣称 `Pass`。
    OperationFailed(OperationFailure),
}

/// L0–L3 遥测事件（递进包含；在结论确定后、阶段 `catch_unwind` 边界之外发出）。
pub enum AttestationEvent<'a> {
    /// L0：步骤进度（默认关）。
    Trace(TraceStep),
    /// L1：结论与认证时刻。
    Verdict {
        /// 三值结论。
        verdict: AttestationVerdict,
        /// 判因（`Pass` 为 `None`）。
        cause: Option<AttestationCause>,
        /// 认证时刻。
        completed_at: Instant,
    },
    /// L2：失配现场。
    Mismatch(&'a MismatchEvidence),
    /// L2 补充：保留的来源审计诊断（同修订重发布的字节身份差异、瞬时恢复、
    /// 复算未完成与来源证据不稳定）。不冒充最终 `Mismatch`——`Pass` 结论
    /// 也会携带这些诊断事件（完整性设计 §7）。
    SourceAudit(&'a SourceAuditNote),
    /// L3：判因与处置提示（同时给出两种调用时机，宿主按所处时机取用）。
    Disposition {
        /// 判因对应的处置类别。
        disposition: Disposition,
        /// 投入使用前提示。
        pre_use: &'static str,
        /// 在用修订审计提示。
        in_use: &'static str,
    },
}

/// 认证遥测 sink。sink 的失败或 panic 由遥测边界吸收并记为遥测丢失，
/// 不改变、不替换已确定的认证结论（完整性设计 §7）。
pub trait AttestationTelemetry {
    /// 接收一次认证遥测事件；不得依赖被调用。
    /// 异步 sink 的成功仅表示已接收或入队，后续投递失败由 sink 自行观测。
    ///
    /// # Errors
    ///
    /// 无法接收事件（如队列已满或同步写入失败）时返回 [`TelemetryDeliveryError`]。
    fn emit(&mut self, event: AttestationEvent<'_>) -> Result<(), TelemetryDeliveryError>;
}

/// sink 未能接收本次遥测事件；后端诊断由宿主保留，不参与认证结论。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TelemetryDeliveryError;

impl std::fmt::Display for TelemetryDeliveryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("遥测事件接收失败")
    }
}

impl std::error::Error for TelemetryDeliveryError {}

/// 首次认证尝试（或复算）中一次阶段运行的结局。
#[derive(Debug)]
pub(crate) enum AttemptOutcome {
    /// 来源准备与修订比较全部通过。
    Concluded,
    /// 失配候选（来源准备失败或修订比较失配），待复算确认。
    Failed(FirstFailure),
    /// 操作失败（不进三值结论）。
    OperationFailed(OperationFailure),
}

/// 失配候选按首次失配所在阶段归类（设计 §2.2 复算归类表的行维度）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FirstFailure {
    /// 来源准备阶段失败（含该阶段的 panic）。
    Source(MismatchEvidence),
    /// 修订比较阶段失配（含该阶段的 panic）。
    RootCompare(MismatchEvidence),
}

/// 认证共享路网修订（完整性设计 §4.2；术语表：shared network revision authentication）。
///
/// 三段边界：来源准备（reader、同修订判据、候选修订完整构建，不读取被认证修订）→
/// 修订比较（`origin` 与期望值逐字段、候选修订与被认证修订逐表按值）→ 确认与处置
/// （来源复核、按首次失配位置归类的复算确认）。候选修订构建复用现行
/// [`build_shared_network_revision`]，候选修订缓冲与 builder scratch 沿用其
/// 按计数可失败预留与调用方限额（设计 §2.3）；预留失败属操作失败。
///
/// 本入口只读不可变修订，可与在用世界的 `step` 并发执行，不加锁、不阻塞；
/// 不改变 `TrafficWorld::install`、`restore_lfrs`、`SpatialSession::bind`
/// 与切换入口的签名与语义。限额不进入期望值：reader / builder 的限额拒绝
/// 是操作失败，不是失配。同修订判据之外，宿主字节与引用的 LFCA
/// digest / exact length 差异只记来源审计诊断（同修订重发布合法）。
///
/// # Panics
///
/// 认证过程按阶段以 `catch_unwind` 包住，panic 不向宿主传播：来源准备阶段
/// panic 按来源失败归类（复算确认），修订比较阶段 panic 直接判 `Untrusted`
/// （设计 §2.4）；`panic=abort` 构建下按进程终止语义处理。reader / builder
/// 中不可失败分配的 OOM 同属进程终止，不在认证结果集合内。
pub fn attest_shared_network_revision<S>(
    root: &SharedNetworkRevision,
    lfca: S,
    expectation: RootAttestationExpectation,
    format_limits: FormatLimits,
    build_options: SharedNetworkBuildOptions<'_>,
    telemetry: Option<&mut dyn AttestationTelemetry>,
) -> AttestationOutcome
where
    S: BoundedReReadableObjectSource,
{
    let mut source_audit = Vec::new();
    let mut trace = Vec::new();
    let mut prepared_identity: Option<(Sha256Digest, ExactByteLength)> = None;
    let first = run_attempt(
        root,
        &lfca,
        &expectation,
        format_limits,
        build_options,
        Some(&mut prepared_identity),
        &mut source_audit,
        &mut trace,
    );
    let mut outcome = match first {
        AttemptOutcome::Concluded => concluded(
            AttestationVerdict::Pass,
            None,
            None,
            Disposition::None,
            source_audit,
        ),
        AttemptOutcome::OperationFailed(mut failure) => {
            // 操作失败保留已收集的来源审计诊断（字节身份差异等）。
            failure.source_audit = source_audit;
            AttestationOutcome::OperationFailed(failure)
        }
        AttemptOutcome::Failed(FirstFailure::RootCompare(evidence)) => {
            if matches!(evidence.site, MismatchSite::Panicked { .. }) {
                // 修订比较阶段 panic 直接判 Untrusted（设计 §2.4），不进复算。
                concluded(
                    AttestationVerdict::Untrusted,
                    Some(AttestationCause::Indeterminate),
                    Some(evidence),
                    Disposition::DisableRoot,
                    source_audit,
                )
            } else {
                confirm_root_mismatch(
                    root,
                    &lfca,
                    &expectation,
                    format_limits,
                    build_options,
                    evidence,
                    prepared_identity,
                    source_audit,
                    &mut trace,
                )
            }
        }
        AttemptOutcome::Failed(FirstFailure::Source(evidence)) => {
            // 来源准备阶段失败直接按复算归类表确认（设计 §2.2）。
            trace.push(TraceStep::RecomputeStarted);
            let recompute = run_attempt(
                root,
                &lfca,
                &expectation,
                format_limits,
                build_options,
                None,
                &mut source_audit,
                &mut trace,
            );
            classify_confirmed_failure(FirstFailure::Source(evidence), recompute, source_audit)
        }
    };
    emit_telemetry(telemetry, &mut outcome, &trace);
    outcome
}

/// 修订比较失配路径：来源复核 + 复算 + 归类（设计 §4.2 第 ③ 段）。
#[allow(clippy::too_many_arguments)]
fn confirm_root_mismatch<S>(
    root: &SharedNetworkRevision,
    lfca: &S,
    expectation: &RootAttestationExpectation,
    format_limits: FormatLimits,
    build_options: SharedNetworkBuildOptions<'_>,
    evidence: MismatchEvidence,
    prepared_identity: Option<(Sha256Digest, ExactByteLength)>,
    mut source_audit: Vec<SourceAuditNote>,
    trace: &mut Vec<TraceStep>,
) -> AttestationOutcome
where
    S: BoundedReReadableObjectSource,
{
    trace.push(TraceStep::SourceRecheckStarted);
    let first = match recheck_source_identity(lfca) {
        Err(error) => {
            return AttestationOutcome::OperationFailed(OperationFailure {
                kind: OperationFailureKind::ObjectSourceFailure,
                detail: format!("{error:?}"),
                source_audit,
                telemetry_dropped: 0,
            });
        }
        Ok(recheck) => match prepared_identity {
            Some((prepared_digest, prepared_length))
                if recheck != (prepared_digest, prepared_length) =>
            {
                // 来源字节在验证之后改变：候选修订不由已验证字节构建，
                // 按来源准备阶段失败处理（设计 §4.2；复算归类表 §2.2）。
                // 证据保留完整的 digest / length 差异（复核比较的是二者
                // 的元组，证据不得只携带 digest）。
                FirstFailure::Source(MismatchEvidence {
                    stage: AttestationStage::SourcePreparation,
                    site: MismatchSite::SourceFailure {
                        detail: SourceFailureDetail::SourceBytesChanged {
                            prepared_digest,
                            prepared_byte_length: prepared_length.get(),
                            recheck_digest: recheck.0,
                            recheck_byte_length: recheck.1.get(),
                        },
                    },
                })
            }
            _ => FirstFailure::RootCompare(evidence),
        },
    };
    trace.push(TraceStep::RecomputeStarted);
    let recompute = run_attempt(
        root,
        lfca,
        expectation,
        format_limits,
        build_options,
        None,
        &mut source_audit,
        trace,
    );
    classify_confirmed_failure(first, recompute, source_audit)
}

/// 单次完整认证尝试：来源准备（含 panic 边界）→ 修订比较（含 panic 边界）。
#[allow(clippy::too_many_arguments)]
fn run_attempt<S>(
    root: &SharedNetworkRevision,
    lfca: &S,
    expectation: &RootAttestationExpectation,
    format_limits: FormatLimits,
    build_options: SharedNetworkBuildOptions<'_>,
    prepared_identity: Option<&mut Option<(Sha256Digest, ExactByteLength)>>,
    source_audit: &mut Vec<SourceAuditNote>,
    trace: &mut Vec<TraceStep>,
) -> AttemptOutcome
where
    S: BoundedReReadableObjectSource,
{
    trace.push(TraceStep::SourcePreparationStarted);
    let candidate = match run_source_preparation(
        lfca,
        expectation,
        format_limits,
        build_options,
        prepared_identity,
        source_audit,
        trace,
    ) {
        Ok(candidate) => candidate,
        Err(stage) => return stage,
    };
    trace.push(TraceStep::RootComparisonStarted);
    let comparison = catch_unwind(AssertUnwindSafe(|| {
        #[cfg(test)]
        if test_injection::root_comparison_panic_armed() {
            panic!("injected root-comparison panic");
        }
        compare_origin(root.canonical_origin(), expectation)
            .and_then(|()| compare_revisions_by_value(&candidate, root))
    }));
    match comparison {
        Ok(Ok(())) => AttemptOutcome::Concluded,
        Ok(Err(evidence)) => AttemptOutcome::Failed(FirstFailure::RootCompare(evidence)),
        Err(payload) => AttemptOutcome::Failed(FirstFailure::RootCompare(MismatchEvidence {
            stage: AttestationStage::RootComparison,
            site: MismatchSite::Panicked {
                payload: panic_payload_text(payload),
            },
        })),
    }
}

/// 来源准备阶段：reader 受检 → 同修订判据 → 候选修订完整构建。
/// 本阶段不读取被认证修订（设计 §4.2）；panic 按来源失败归类。
#[allow(clippy::too_many_arguments)]
fn run_source_preparation<S>(
    lfca: &S,
    expectation: &RootAttestationExpectation,
    format_limits: FormatLimits,
    build_options: SharedNetworkBuildOptions<'_>,
    prepared_identity: Option<&mut Option<(Sha256Digest, ExactByteLength)>>,
    source_audit: &mut Vec<SourceAuditNote>,
    trace: &mut Vec<TraceStep>,
) -> Result<Arc<SharedNetworkRevision>, AttemptOutcome>
where
    S: BoundedReReadableObjectSource,
{
    let prepared = catch_unwind(AssertUnwindSafe(|| {
        #[cfg(test)]
        if test_injection::source_preparation_panic_armed() {
            panic!("injected source-preparation panic");
        }
        let checked = match check_canonical_network_input(lfca, format_limits) {
            Ok(checked) => checked,
            Err(error) => return Err(map_reader_error(error)),
        };
        trace.push(TraceStep::ReaderAccepted);
        // 字节身份在受检身份可用后立即记录：同修订判据失配的提前返回
        // 不得吞掉字节差异诊断（诊断不进入结论，设计 §2.2）。
        let digest = checked.canonical_artifact_digest();
        let length = checked.canonical_artifact_byte_length();
        if digest != expectation.reference_digest() || length != expectation.reference_byte_length()
        {
            source_audit.push(SourceAuditNote::ByteIdentityDiffersFromReference {
                reference_digest: expectation.reference_digest(),
                reference_byte_length: expectation.reference_byte_length().get(),
                source_digest: digest,
                source_byte_length: length.get(),
            });
        }
        if let Some(slot) = prepared_identity {
            *slot = Some((digest, length));
        }
        if checked.network_revision() != expectation.network_revision() {
            return Err(AttemptOutcome::Failed(FirstFailure::Source(
                MismatchEvidence {
                    stage: AttestationStage::SourcePreparation,
                    site: MismatchSite::SourceFailure {
                        detail: SourceFailureDetail::RevisionCriterion {
                            reference: expectation.network_revision(),
                            recomputed: checked.network_revision(),
                        },
                    },
                },
            )));
        }
        trace.push(TraceStep::SameRevisionCriterionChecked);
        match build_shared_network_revision(checked, build_options) {
            Ok(candidate) => Ok(candidate),
            Err(error) => Err(map_builder_error(error)),
        }
    }));
    match prepared {
        Ok(Ok(candidate)) => {
            trace.push(TraceStep::CandidateRootBuilt);
            Ok(candidate)
        }
        Ok(Err(stage)) => Err(stage),
        Err(payload) => Err(AttemptOutcome::Failed(FirstFailure::Source(
            MismatchEvidence {
                stage: AttestationStage::SourcePreparation,
                site: MismatchSite::SourceFailure {
                    detail: SourceFailureDetail::Panic(panic_payload_text(payload)),
                },
            },
        ))),
    }
}

/// reader 错误映射：限额与来源视图失败属操作失败；内容类拒绝属来源失败
/// （设计 §2.4；reader 只证明 framing、直接值域与 revision 绑定）。
fn map_reader_error(error: CanonicalNetworkInputError) -> AttemptOutcome {
    // 此处的操作失败尚无本阶段来源审计诊断；外层出口把已收集诊断附加进结果。
    let operational = |kind| {
        AttemptOutcome::OperationFailed(OperationFailure {
            kind,
            detail: format!("{error:?}"),
            source_audit: Vec::new(),
            telemetry_dropped: 0,
        })
    };
    match &error {
        CanonicalNetworkInputError::ObjectSource(_) => {
            operational(OperationFailureKind::ObjectSourceFailure)
        }
        CanonicalNetworkInputError::LimitExceeded { .. } => {
            operational(OperationFailureKind::ReaderLimitExceeded)
        }
        CanonicalNetworkInputError::Format(FormatError::LimitExceeded { .. }) => {
            operational(OperationFailureKind::ReaderLimitExceeded)
        }
        _ => AttemptOutcome::Failed(FirstFailure::Source(MismatchEvidence {
            stage: AttestationStage::SourcePreparation,
            site: MismatchSite::SourceFailure {
                detail: SourceFailureDetail::Reader(error),
            },
        })),
    }
}

/// builder 错误映射：预算 / 分配 / 取消属操作失败；输入内容类错误（含
/// 计数 / 范围溢出，设计 §2.4 把「计数 / 范围溢出」列属来源内容失败）属来源失败。
fn map_builder_error(error: BuildError) -> AttemptOutcome {
    let operational = |kind| {
        AttemptOutcome::OperationFailed(OperationFailure {
            kind,
            detail: format!("{error:?}"),
            source_audit: Vec::new(),
            telemetry_dropped: 0,
        })
    };
    match &error {
        BuildError::BudgetExceeded { .. } => {
            operational(OperationFailureKind::BuilderBudgetExceeded)
        }
        BuildError::AllocationFailure { .. } => {
            operational(OperationFailureKind::BuilderAllocationFailure)
        }
        BuildError::Cancelled => operational(OperationFailureKind::Cancelled),
        _ => AttemptOutcome::Failed(FirstFailure::Source(MismatchEvidence {
            stage: AttestationStage::SourcePreparation,
            site: MismatchSite::SourceFailure {
                detail: SourceFailureDetail::Builder(error),
            },
        })),
    }
}

/// 来源复核：对宿主字节重算 digest / exact length（真实触达被认证字节，
/// 不缓存折叠）。只证明前后两次读取一致，不证明构建全过程字节未变
/// （设计 §4.2 确认与处置；A → B → A 式瞬态变化列入漏检面 §5 第 17 条）。
fn recheck_source_identity<S>(
    lfca: &S,
) -> Result<(Sha256Digest, ExactByteLength), laneflow_format::ObjectSourceError>
where
    S: BoundedReReadableObjectSource,
{
    const CHUNK_BYTES: usize = 65_536;
    let length = lfca.exact_byte_length();
    let total = length.get();
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; CHUNK_BYTES];
    let mut offset = 0_u64;
    while offset < total {
        let chunk = (total - offset).min(u64::try_from(CHUNK_BYTES).expect("chunk fits u64"));
        let window = usize::try_from(chunk).expect("chunk fits usize");
        lfca.read_exact_at(offset, &mut buffer[..window])?;
        hasher.update(&buffer[..window]);
        offset += chunk;
    }
    Ok((Sha256Digest::from_bytes(hasher.finalize().into()), length))
}

/// 复算归类（完整性设计 §2.2 复算归类表；完成与未完成同表）。
pub(crate) fn classify_confirmed_failure(
    first: FirstFailure,
    recompute: AttemptOutcome,
    mut source_audit: Vec<SourceAuditNote>,
) -> AttestationOutcome {
    match (first, recompute) {
        (FirstFailure::Source(first_evidence), recompute) => {
            let recompute_failure = match recompute {
                AttemptOutcome::Concluded => {
                    source_audit.push(SourceAuditNote::TransientSourceFailure {
                        evidence: Box::new(first_evidence),
                    });
                    return concluded(
                        AttestationVerdict::Pass,
                        None,
                        None,
                        Disposition::None,
                        source_audit,
                    );
                }
                AttemptOutcome::OperationFailed(failure) => {
                    // 复算未完成不是瞬时恢复：首次证据保留，标签独立记录，
                    // 不得混入 TransientSourceFailure（其语义是复算完整通过）。
                    source_audit.push(SourceAuditNote::SourceRecomputeIncomplete {
                        evidence: Box::new(first_evidence),
                    });
                    return AttestationOutcome::OperationFailed(OperationFailure {
                        kind: OperationFailureKind::RecomputeIncomplete,
                        detail: failure.detail,
                        source_audit,
                        telemetry_dropped: 0,
                    });
                }
                AttemptOutcome::Failed(failure) => failure,
            };
            match recompute_failure {
                FirstFailure::Source(recompute_evidence) => {
                    if first_evidence == recompute_evidence {
                        concluded(
                            AttestationVerdict::Mismatch,
                            Some(AttestationCause::Source),
                            Some(first_evidence),
                            Disposition::IsolateSource,
                            source_audit,
                        )
                    } else {
                        // 来源证据不稳定：与宿主未能提供来源同类（设计 §2.4）。
                        let detail = format!("{first_evidence:?} vs {recompute_evidence:?}");
                        source_audit.push(SourceAuditNote::SourceEvidenceUnstable {
                            first: Box::new(first_evidence),
                            recompute: Box::new(recompute_evidence),
                        });
                        AttestationOutcome::OperationFailed(OperationFailure {
                            kind: OperationFailureKind::SourceEvidenceUnstable,
                            detail,
                            source_audit,
                            telemetry_dropped: 0,
                        })
                    }
                }
                // 复算改在修订比较阶段失配：出现未确认的修订侧证据（设计 §2.2）。
                FirstFailure::RootCompare(recompute_evidence) => concluded(
                    AttestationVerdict::Untrusted,
                    Some(AttestationCause::Indeterminate),
                    Some(recompute_evidence),
                    Disposition::DisableRoot,
                    source_audit,
                ),
            }
        }
        (FirstFailure::RootCompare(first_evidence), recompute) => {
            // 修订侧证据不能被复算阶段的操作失败撤销（设计 §2.2）：复算未完成、
            // 证据不同（含复算通过、复算改判来源失败）一律 Untrusted，保留首次证据。
            let same_evidence = matches!(
                recompute,
                AttemptOutcome::Failed(FirstFailure::RootCompare(ref recompute_evidence))
                    if *recompute_evidence == first_evidence
            );
            if same_evidence {
                let cause = cause_for_root_evidence(&first_evidence);
                let disposition = match cause {
                    AttestationCause::SharedStaticRoot | AttestationCause::Indeterminate => {
                        Disposition::DisableRoot
                    }
                    AttestationCause::Source => Disposition::IsolateSource,
                };
                concluded(
                    AttestationVerdict::Mismatch,
                    Some(cause),
                    Some(first_evidence),
                    disposition,
                    source_audit,
                )
            } else {
                concluded(
                    AttestationVerdict::Untrusted,
                    Some(AttestationCause::Indeterminate),
                    Some(first_evidence),
                    Disposition::DisableRoot,
                    source_audit,
                )
            }
        }
    }
}

/// 修订比较阶段失配的判因（完整性设计 §7 判因轴）。
fn cause_for_root_evidence(evidence: &MismatchEvidence) -> AttestationCause {
    match &evidence.site {
        MismatchSite::OriginField { .. } | MismatchSite::SpatialPresence { .. } => {
            AttestationCause::Indeterminate
        }
        MismatchSite::TableValue { .. } | MismatchSite::TableLength { .. } => {
            AttestationCause::SharedStaticRoot
        }
        MismatchSite::SourceFailure { .. } => AttestationCause::Source,
        MismatchSite::Panicked { .. } => AttestationCause::Indeterminate,
    }
}

fn concluded(
    verdict: AttestationVerdict,
    cause: Option<AttestationCause>,
    mismatch: Option<MismatchEvidence>,
    disposition: Disposition,
    source_audit: Vec<SourceAuditNote>,
) -> AttestationOutcome {
    AttestationOutcome::Concluded(AttestationReport {
        verdict,
        cause,
        completed_at: Instant::now(),
        mismatch,
        disposition,
        source_audit,
        telemetry_dropped: 0,
    })
}

/// 遥测在结论确定之后、认证阶段的 `catch_unwind` 边界之外单独发出；
/// sink 的失败或 panic 由本边界吸收，不改变结论（设计 §7）。
pub(crate) fn emit_telemetry(
    sink: Option<&mut (dyn AttestationTelemetry + '_)>,
    outcome: &mut AttestationOutcome,
    trace: &[TraceStep],
) -> u32 {
    let Some(sink) = sink else {
        return 0;
    };
    let mut dropped = 0_u32;
    let mut emit = |sink: &mut dyn AttestationTelemetry, event: AttestationEvent<'_>| {
        if !matches!(
            catch_unwind(AssertUnwindSafe(|| sink.emit(event))),
            Ok(Ok(()))
        ) {
            // 遥测丢失计数不经故障 sink 报告，随认证结果返回（设计 §7）。
            dropped += 1;
        }
    };
    for step in trace {
        emit(sink, AttestationEvent::Trace(*step));
    }
    match outcome {
        AttestationOutcome::Concluded(report) => {
            emit(
                sink,
                AttestationEvent::Verdict {
                    verdict: report.verdict,
                    cause: report.cause,
                    completed_at: report.completed_at,
                },
            );
            if let Some(evidence) = &report.mismatch {
                emit(sink, AttestationEvent::Mismatch(evidence));
            }
            for note in &report.source_audit {
                emit(sink, AttestationEvent::SourceAudit(note));
            }
            emit(
                sink,
                AttestationEvent::Disposition {
                    disposition: report.disposition,
                    pre_use: report.disposition.pre_use_guidance(),
                    in_use: report.disposition.in_use_guidance(),
                },
            );
        }
        // 操作失败不进入三值结论（不伪造 Verdict），但 L3 重试处置照发
        // （设计 §7：调用方可能依赖遥测通道取得处置提示）；保留的来源
        // 审计诊断（复算未完成 / 证据不稳定的首次证据）同路径发出。
        AttestationOutcome::OperationFailed(failure) => {
            for note in &failure.source_audit {
                emit(sink, AttestationEvent::SourceAudit(note));
            }
            emit(
                sink,
                AttestationEvent::Disposition {
                    disposition: Disposition::Retry,
                    pre_use: Disposition::Retry.pre_use_guidance(),
                    in_use: Disposition::Retry.in_use_guidance(),
                },
            );
        }
    }
    match outcome {
        AttestationOutcome::Concluded(report) => report.telemetry_dropped = dropped,
        AttestationOutcome::OperationFailed(failure) => failure.telemetry_dropped = dropped,
    }
    dropped
}

/// panic 载荷的稳定文本（证据身份的一部分；不可 downcast 的载荷记为不透明）。
fn panic_payload_text(payload: Box<dyn Any + Send>) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "opaque panic payload".to_string()
    }
}

/// 测试注入：认证阶段的确定性 panic 钩子（#959；`BoundedReReadableObjectSource`
/// 的封印不可变借用使「来源在验证后被改写」无法在 safe 代码中真实构造，
/// panic 归类路径经此注入覆盖）。
#[cfg(test)]
pub(crate) mod test_injection {
    use std::cell::Cell;

    thread_local! {
        static SOURCE_PREPARATION_PANIC: Cell<bool> = const { Cell::new(false) };
        static SOURCE_PREPARATION_PANIC_ONCE: Cell<bool> = const { Cell::new(false) };
        static ROOT_COMPARISON_PANIC: Cell<bool> = const { Cell::new(false) };
    }

    /// 武装来源准备阶段持续 panic（复算同样触发，用于同证据复算用例）。
    pub(crate) fn arm_source_preparation_panic() {
        SOURCE_PREPARATION_PANIC.with(|flag| flag.set(true));
    }

    /// 武装来源准备阶段一次性 panic：首次触发后自动复位（用于复算恢复用例）。
    pub(crate) fn arm_source_preparation_panic_once() {
        SOURCE_PREPARATION_PANIC_ONCE.with(|flag| flag.set(true));
    }

    pub(crate) fn source_preparation_panic_armed() -> bool {
        let persistent = SOURCE_PREPARATION_PANIC.with(|flag| flag.get());
        let once = SOURCE_PREPARATION_PANIC_ONCE.with(|flag| flag.replace(false));
        persistent || once
    }

    /// 武装修订比较阶段 panic。
    pub(crate) fn arm_root_comparison_panic() {
        ROOT_COMPARISON_PANIC.with(|flag| flag.set(true));
    }

    pub(crate) fn root_comparison_panic_armed() -> bool {
        ROOT_COMPARISON_PANIC.with(|flag| flag.get())
    }

    /// 复位全部 panic 注入。
    pub(crate) fn reset() {
        SOURCE_PREPARATION_PANIC.with(|flag| flag.set(false));
        SOURCE_PREPARATION_PANIC_ONCE.with(|flag| flag.set(false));
        ROOT_COMPARISON_PANIC.with(|flag| flag.set(false));
    }
}
