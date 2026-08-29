//! 切换描述符（#302 切片 A）：宿主对象外可信封闭输入的进程内类型与
//! 一致性验证。同修订同步换根与跨修订切换事务（#513 切片 C）同处本文件
//! 与 `cutover_transaction`。

use laneflow_static_contract::{
    ExactByteLength, NETWORK_REVISION_DERIVATION_VERSION, NetworkRevisionId,
    SEMANTIC_DIFF_FORMAT_VERSION, Sha256Digest,
};
use std::sync::Arc;

use laneflow_format::{RegistryCheckedFieldValue, preflight_object_values};
use laneflow_static_contract::PortableObjectKind;
use sha2::Digest as _;

use laneflow_static_network::{CanonicalNetworkOrigin, SharedNetworkRevision};
use thiserror::Error;

use crate::source::CommittedNetworkSource;
use crate::tables::CompiledRoute;
use crate::tables::compile_route;
use crate::{ObservationStateSequence, RouteError, StepError, TrafficWorld, WorldGeneration};

/// 描述符封闭契约版本（#302 切换合同 §2）。
pub const CUTOVER_DESCRIPTOR_FORMAT_VERSION: u16 = 1;

/// 迁移策略封闭种类选择器（#302 切换合同 §3；术语表：封闭种类选择器）。
///
/// 不是协议版本；策略语义演进随描述符契约版本整体拒绝或放行。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MigrationPolicyKind {
    /// 同修订换根：判据 = 两侧修订标识与派生版本、契约版本精确相等；
    /// origin 字节差异仅承担来源审计。
    SameRevisionRestore,
    /// 跨修订直移：经 LFSD 把每个动态实体的稳定引用直移到 target。
    CrossRevisionDirect,
}

/// LFCA origin 四联（#302 切换合同 §2）：digest / byte length /
/// `NetworkRevisionId` / `networkRevisionDerivationVersion`，与 LFSD
/// base/target binding 同构。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LfcaOriginBinding {
    canonical_artifact_digest: Sha256Digest,
    canonical_artifact_byte_length: ExactByteLength,
    network_revision: NetworkRevisionId,
    network_revision_derivation_version: u16,
}

impl LfcaOriginBinding {
    /// 构造四联；值由宿主发布链绑定，本类型不重算、不验证真实性。
    #[must_use]
    pub const fn new(
        canonical_artifact_digest: Sha256Digest,
        canonical_artifact_byte_length: ExactByteLength,
        network_revision: NetworkRevisionId,
        network_revision_derivation_version: u16,
    ) -> Self {
        Self {
            canonical_artifact_digest,
            canonical_artifact_byte_length,
            network_revision,
            network_revision_derivation_version,
        }
    }

    /// 从已加载/已认证制品的进程内 origin 建立绑定。
    #[must_use]
    pub fn from_canonical_origin(origin: CanonicalNetworkOrigin) -> Self {
        Self::new(
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
            NETWORK_REVISION_DERIVATION_VERSION,
        )
    }

    /// 与已认证制品 origin 的四联逐项比对。
    #[must_use]
    pub fn matches_origin(&self, origin: CanonicalNetworkOrigin) -> bool {
        self.network_revision_derivation_version == NETWORK_REVISION_DERIVATION_VERSION
            && self.canonical_artifact_digest == origin.canonical_artifact_digest()
            && self.canonical_artifact_byte_length == origin.canonical_artifact_byte_length()
            && self.network_revision == origin.network_revision()
    }

    /// 绑定的路网修订标识。
    #[must_use]
    pub const fn network_revision(&self) -> NetworkRevisionId {
        self.network_revision
    }

    /// 绑定的修订派生版本轴。
    #[must_use]
    pub const fn network_revision_derivation_version(&self) -> u16 {
        self.network_revision_derivation_version
    }
}

/// LFSD origin 三联（#302 切换合同 §2）：format version / digest / byte length。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SemanticDiffOriginBinding {
    semantic_diff_format_version: u16,
    semantic_diff_digest: Sha256Digest,
    semantic_diff_byte_length: ExactByteLength,
}

impl SemanticDiffOriginBinding {
    /// 构造三联；值由宿主发布链绑定。
    #[must_use]
    pub const fn new(
        semantic_diff_format_version: u16,
        semantic_diff_digest: Sha256Digest,
        semantic_diff_byte_length: ExactByteLength,
    ) -> Self {
        Self {
            semantic_diff_format_version,
            semantic_diff_digest,
            semantic_diff_byte_length,
        }
    }

    /// 绑定的 LFSD exact-byte 长度（预检对象）。
    #[must_use]
    pub const fn semantic_diff_byte_length(&self) -> ExactByteLength {
        self.semantic_diff_byte_length
    }

    /// 绑定的 LFSD exact-byte 摘要（#513 切片 C：字节级认证比对对象）。
    #[must_use]
    pub const fn semantic_diff_digest(&self) -> Sha256Digest {
        self.semantic_diff_digest
    }
}

/// 世界绑定（#302 切换合同 §2）：目标世界身份、活动聚合世代与基线命令/事件双游标。
///
/// 游标是描述符签发时点对齐的基线，不是实时值；世界身份与世代在事务
/// 启动时逐项比对。精确值域为宿主 `u64` 身份加 Runtime 签发的 `u64` 世代。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorldBinding {
    world_id: u64,
    world_generation: WorldGeneration,
    baseline_command_cursor: u64,
    baseline_event_cursor: u64,
}

impl WorldBinding {
    /// 构造世界绑定。
    #[must_use]
    pub const fn new(
        world_id: u64,
        world_generation: WorldGeneration,
        baseline_command_cursor: u64,
        baseline_event_cursor: u64,
    ) -> Self {
        Self {
            world_id,
            world_generation,
            baseline_command_cursor,
            baseline_event_cursor,
        }
    }

    /// 目标世界身份（宿主任意定义；编码 G2 留白）。
    #[must_use]
    pub const fn world_id(&self) -> u64 {
        self.world_id
    }

    /// 描述符签发时的活动世界世代。
    #[must_use]
    pub const fn world_generation(&self) -> WorldGeneration {
        self.world_generation
    }

    /// 基线输入命令游标；事务启动时与目标世界的已应用命令计数逐项
    /// 比对（快照边界锚点与切换基线指同一命令序列位置）。
    #[must_use]
    pub const fn baseline_command_cursor(&self) -> u64 {
        self.baseline_command_cursor
    }

    /// 基线已提交切换事件游标；事务启动时与本世界已提交事件计数逐项比对
    /// （失配即 `BaselineEventCursorMismatch`）。#513 切片 C 起随事件批次
    /// 通道成为真实轴。
    #[must_use]
    pub const fn baseline_event_cursor(&self) -> u64 {
        self.baseline_event_cursor
    }
}

/// 路网修订切换描述符（术语表：`NetworkRevisionCutoverDescriptor`）。
///
/// 宿主/上层在对象外可信提供的封闭契约输入。Runtime 的一致性验证只回答
/// 「这份输入与两侧已认证制品是否精确一致」，不回答「这次迁移是否该发生」。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkRevisionCutoverDescriptor {
    base: LfcaOriginBinding,
    target: LfcaOriginBinding,
    semantic_diff: Option<SemanticDiffOriginBinding>,
    policy_kind: MigrationPolicyKind,
    world: WorldBinding,
}

/// 描述符一致性验证的宿主预检上限（#302 切换合同 §2：验证先于解析）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CutoverPreflightLimits {
    /// LFSD exact-byte 长度上限：任何解析、分配或哈希之前做 O(1) 比较。
    pub max_semantic_diff_bytes: u64,
}

impl CutoverPreflightLimits {
    /// 构造预检上限。
    #[must_use]
    pub const fn new(max_semantic_diff_bytes: u64) -> Self {
        Self {
            max_semantic_diff_bytes,
        }
    }
}

/// 描述符一致性验证失败（#302 切换合同 §8「描述符不一致/不可信」）。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum CutoverDescriptorError {
    /// base 侧 origin 四联与已认证制品不一致。
    #[error("base 侧 LFCA origin 四联与已认证制品不一致")]
    BaseOriginMismatch,
    /// target 侧 origin 四联与已认证制品不一致。
    #[error("target 侧 LFCA origin 四联与已认证制品不一致")]
    TargetOriginMismatch,
    /// origin 绑定的修订派生版本不是现行支持的轴值。
    #[error("origin 绑定的修订派生版本 {binding} 不受支持（现行 {supported}）")]
    RevisionDerivationVersionUnsupported {
        /// 绑定携带的派生版本。
        binding: u16,
        /// 现行支持的派生版本。
        supported: u16,
    },
    /// 同修订换根策略下两侧修订标识不相等。
    #[error("同修订换根策略要求两侧修订标识相等")]
    SameRevisionRequiresEqualRevisions,
    /// 跨修订直移必须携带 LFSD origin。
    #[error("跨修订直移策略缺少 LFSD origin")]
    CrossRevisionRequiresSemanticDiff,
    /// 同修订换根不携带 LFSD origin（同修订 rebase 不发射 LFSD）。
    #[error("同修订换根策略不得携带 LFSD origin")]
    SameRevisionForbidsSemanticDiff,
    /// LFSD format version 不受支持。
    #[error("LFSD format version {binding} 不受支持（现行 {supported}）")]
    SemanticDiffFormatVersionUnsupported {
        /// 绑定携带的 LFSD 格式版本。
        binding: u16,
        /// 现行支持的 LFSD 格式版本。
        supported: u16,
    },
    /// LFSD 长度超过宿主预检上限（先于任何解析、分配或哈希）。
    #[error("LFSD 长度 {length} 超过预检上限 {limit}")]
    SemanticDiffByteLengthOverLimit {
        /// 绑定的 LFCA 长度。
        length: u64,
        /// 宿主声明的上限。
        limit: u64,
    },
    /// 跨修订直移策略下两侧修订标识相等（两 kind 互斥）。
    #[error("跨修订直移策略要求两侧修订标识不等")]
    CrossRevisionRequiresUnequalRevisions,
    /// 同修订换根策略下两侧静态契约版本不相等（#302 判据）。
    #[error("同修订换根策略要求两侧静态契约版本相等")]
    SameRevisionRequiresEqualContractVersions,
    /// LFSD 实际字节长度与描述符绑定声明不一致（先于解析与哈希后的
    /// 第二道长度闭合；O(1) 预检仍由 `validate` 承担）。
    #[error("LFSD 实际字节长度 {actual} 与绑定声明 {declared} 不一致")]
    SemanticDiffByteLengthMismatch {
        /// 绑定声明的 exact-byte 长度。
        declared: u64,
        /// 实际提供的字节数。
        actual: u64,
    },
    /// LFSD 字节 SHA-256 摘要与描述符绑定声明不一致。
    #[error("LFSD 字节摘要与绑定声明不一致")]
    SemanticDiffDigestMismatch,
    /// LFSD 字节未通过注册表结构/值域校验。
    #[error("LFSD 结构未通过注册表校验")]
    SemanticDiffStructureInvalid,
    /// LFSD 绑定行种类不受支持。
    #[error("LFSD 绑定行种类 {actual} 不受支持")]
    SemanticDiffBindingKindUnsupported {
        /// 绑定行携带的种类值。
        actual: u8,
    },
    /// LFSD base 侧绑定与已认证 base 制品逐项比对失败。
    #[error("LFSD base 侧绑定与已认证制品不一致")]
    SemanticDiffBaseBindingMismatch,
    /// LFSD target 侧绑定与已认证 target 制品逐项比对失败。
    #[error("LFSD target 侧绑定与已认证制品不一致")]
    SemanticDiffTargetBindingMismatch,
}

impl NetworkRevisionCutoverDescriptor {
    /// 构造描述符。字段封闭；一致性由 [`Self::validate`] 承担。
    #[must_use]
    pub const fn new(
        base: LfcaOriginBinding,
        target: LfcaOriginBinding,
        semantic_diff: Option<SemanticDiffOriginBinding>,
        policy_kind: MigrationPolicyKind,
        world: WorldBinding,
    ) -> Self {
        Self {
            base,
            target,
            semantic_diff,
            policy_kind,
            world,
        }
    }

    /// 描述符封闭契约版本。
    #[must_use]
    pub const fn format_version(&self) -> u16 {
        CUTOVER_DESCRIPTOR_FORMAT_VERSION
    }

    /// 迁移策略种类。
    #[must_use]
    pub const fn policy_kind(&self) -> MigrationPolicyKind {
        self.policy_kind
    }

    /// 世界绑定。
    #[must_use]
    pub const fn world_binding(&self) -> WorldBinding {
        self.world
    }

    /// 一致性验证（#302 切换合同 §2/§3）。
    ///
    /// 只验证「输入与两侧已认证制品、策略语义是否精确一致」；LFSD 长度
    /// 预检先于一切。事务启动时的基线游标比对由后续状态机步骤承接。
    pub fn validate(
        &self,
        base_origin: CanonicalNetworkOrigin,
        target_origin: CanonicalNetworkOrigin,
        limits: &CutoverPreflightLimits,
    ) -> Result<(), CutoverDescriptorError> {
        let supported = NETWORK_REVISION_DERIVATION_VERSION;
        if self.base.network_revision_derivation_version() != supported {
            return Err(
                CutoverDescriptorError::RevisionDerivationVersionUnsupported {
                    binding: self.base.network_revision_derivation_version(),
                    supported,
                },
            );
        }
        if self.target.network_revision_derivation_version() != supported {
            return Err(
                CutoverDescriptorError::RevisionDerivationVersionUnsupported {
                    binding: self.target.network_revision_derivation_version(),
                    supported,
                },
            );
        }
        // 认证先行：origin 四联与已认证制品逐项匹配，策略判据只作用于
        // 已认证制品（错误根因不被判据误报遮蔽）。
        if !self.base.matches_origin(base_origin) {
            return Err(CutoverDescriptorError::BaseOriginMismatch);
        }
        if !self.target.matches_origin(target_origin) {
            return Err(CutoverDescriptorError::TargetOriginMismatch);
        }
        match self.policy_kind {
            MigrationPolicyKind::SameRevisionRestore => {
                if self.semantic_diff.is_some() {
                    return Err(CutoverDescriptorError::SameRevisionForbidsSemanticDiff);
                }
                if self.base.network_revision() != self.target.network_revision() {
                    return Err(CutoverDescriptorError::SameRevisionRequiresEqualRevisions);
                }
                // 同修订判据含静态契约版本精确相等（#302 切换合同 §3）；
                // 判据作用于两侧已认证制品。
                if base_origin.static_contract_versions()
                    != target_origin.static_contract_versions()
                {
                    return Err(CutoverDescriptorError::SameRevisionRequiresEqualContractVersions);
                }
            }
            MigrationPolicyKind::CrossRevisionDirect => {
                let Some(semantic_diff) = self.semantic_diff else {
                    return Err(CutoverDescriptorError::CrossRevisionRequiresSemanticDiff);
                };
                if self.base.network_revision() == self.target.network_revision() {
                    return Err(CutoverDescriptorError::CrossRevisionRequiresUnequalRevisions);
                }
                if semantic_diff.semantic_diff_format_version != SEMANTIC_DIFF_FORMAT_VERSION {
                    return Err(
                        CutoverDescriptorError::SemanticDiffFormatVersionUnsupported {
                            binding: semantic_diff.semantic_diff_format_version,
                            supported: SEMANTIC_DIFF_FORMAT_VERSION,
                        },
                    );
                }
                let length = semantic_diff.semantic_diff_byte_length().get();
                if length > limits.max_semantic_diff_bytes {
                    return Err(CutoverDescriptorError::SemanticDiffByteLengthOverLimit {
                        length,
                        limit: limits.max_semantic_diff_bytes,
                    });
                }
            }
        }
        Ok(())
    }

    /// LFSD 字节认证消费（#513 切片 C；切换合同 §2「与 LFSD base/target
    /// binding 同构，逐项交叉验证」）。
    ///
    /// 先做 O(1) 长度比对与字节 SHA-256 认证，再经注册表校验读取器解析，
    /// 最后把绑定行与两侧已认证制品逐项交叉验证。任何不一致都按「描述符
    /// 不一致/不可信」失败关闭；O(1) 上限预检已由 [`Self::validate`]
    /// 在任何解析、分配或哈希之前承担。
    pub(crate) fn verify_semantic_diff(
        &self,
        lfsd_bytes: &[u8],
        base_origin: CanonicalNetworkOrigin,
        target_origin: CanonicalNetworkOrigin,
    ) -> Result<(), CutoverDescriptorError> {
        let Some(binding) = self.semantic_diff.as_ref() else {
            return Err(CutoverDescriptorError::CrossRevisionRequiresSemanticDiff);
        };
        let declared_length = binding.semantic_diff_byte_length().get();
        let actual_length = u64::try_from(lfsd_bytes.len()).map_err(|_| {
            CutoverDescriptorError::SemanticDiffByteLengthMismatch {
                declared: declared_length,
                actual: u64::MAX,
            }
        })?;
        if actual_length != declared_length {
            return Err(CutoverDescriptorError::SemanticDiffByteLengthMismatch {
                declared: declared_length,
                actual: actual_length,
            });
        }
        let digest: [u8; 32] = sha2::Sha256::digest(lfsd_bytes).into();
        if Sha256Digest::from_bytes(digest) != binding.semantic_diff_digest() {
            return Err(CutoverDescriptorError::SemanticDiffDigestMismatch);
        }
        let view = preflight_object_values(
            lfsd_bytes,
            PortableObjectKind::SemanticDiff,
            laneflow_format::FormatLimits::HARD,
        )
        .map_err(|_| CutoverDescriptorError::SemanticDiffStructureInvalid)?
        .registry_view();
        let row = view
            .section(0)
            .and_then(|section| section.table(0))
            .and_then(|table| table.row(0))
            .ok_or(CutoverDescriptorError::SemanticDiffStructureInvalid)?;
        let field = |tag: u16| {
            row.field_by_tag(tag)
                .expect("schema-required LFSD binding field")
                .value()
                .expect("registry-checked LFSD binding value")
        };
        let u8_of = |tag: u16| match field(tag) {
            RegistryCheckedFieldValue::U8(value) => value,
            _ => panic!("LFSD binding field type drift at tag {tag}"),
        };
        let u16_of = |tag: u16| match field(tag) {
            RegistryCheckedFieldValue::U16(value) => value,
            _ => panic!("LFSD binding field type drift at tag {tag}"),
        };
        let u64_of = |tag: u16| match field(tag) {
            RegistryCheckedFieldValue::U64(value) => value,
            _ => panic!("LFSD binding field type drift at tag {tag}"),
        };
        let sha_of = |tag: u16| match field(tag) {
            RegistryCheckedFieldValue::Sha256(value) => value,
            _ => panic!("LFSD binding field type drift at tag {tag}"),
        };
        // 绑定行种类：v1 只承认制品绑定（值 1，沿 lfsd-noop/change-set 冻结值）。
        const SEMANTIC_DIFF_ARTIFACT_BINDING_KIND: u8 = 1;
        let binding_kind = u8_of(1);
        if binding_kind != SEMANTIC_DIFF_ARTIFACT_BINDING_KIND {
            return Err(CutoverDescriptorError::SemanticDiffBindingKindUnsupported {
                actual: binding_kind,
            });
        }
        let base_matches = u16_of(2) == NETWORK_REVISION_DERIVATION_VERSION
            && NetworkRevisionId::from_digest(sha_of(3)) == base_origin.network_revision()
            && sha_of(4) == base_origin.canonical_artifact_digest()
            && u64_of(5) == base_origin.canonical_artifact_byte_length().get();
        if !base_matches {
            return Err(CutoverDescriptorError::SemanticDiffBaseBindingMismatch);
        }
        let target_matches = u16_of(6) == NETWORK_REVISION_DERIVATION_VERSION
            && NetworkRevisionId::from_digest(sha_of(7)) == target_origin.network_revision()
            && sha_of(8) == target_origin.canonical_artifact_digest()
            && u64_of(9) == target_origin.canonical_artifact_byte_length().get();
        if !target_matches {
            return Err(CutoverDescriptorError::SemanticDiffTargetBindingMismatch);
        }
        Ok(())
    }
}

/// 切换事务失败（#302 切换合同 §8）。任一失败路径都保持旧世界原样
/// 继续：旧修订、旧动态状态、旧来源、旧占用与信号语义不变。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum CutoverError {
    /// 描述符一致性验证失败（对应合同 §8「描述符不一致/不可信」）。
    #[error("切换描述符一致性验证失败")]
    Descriptor(#[from] CutoverDescriptorError),
    /// 本入口只承载 `same_revision_restore`；跨修订直移属切片 C。
    #[error("本切换入口不接受该迁移策略")]
    PolicyMismatch,
    /// 目标来源的修订标识与 target 侧已认证制品不一致。
    #[error("目标来源修订与 target 制品修订不一致")]
    TargetSourceRevisionMismatch,
    /// 描述符世界绑定与本世界身份不一致。
    #[error("描述符世界身份或活动世代与本世界不一致")]
    WorldBindingMismatch,
    /// 描述符命令基线游标与本世界已应用命令计数不一致（切换合同 §2
    /// 启动比对；快照检查点与切换基线必须指同一命令序列位置）。
    #[error("命令基线游标不一致：描述符 {descriptor}，世界 {world}")]
    BaselineCommandCursorMismatch {
        /// 描述符携带的基线命令游标。
        descriptor: u64,
        /// 事务启动时世界的已应用命令计数。
        world: u64,
    },
    /// 活动世界世代无法继续递增；事务在任何暂存或提交前失败关闭。
    #[error("活动世界世代已耗尽")]
    WorldGenerationExhausted,
    /// 目标修订的信号程序与本世界固定步长不兼容：相位短于步长或非步长
    /// 整数倍（install 路径 `PhaseShorterThanTick` / `PhaseNotMultipleOfTick`
    /// 的跨修订对等校验；切换合同 §3 重绑即重验证）。
    #[error("目标修订信号程序与本世界固定步长不兼容")]
    TargetSignalProgramInvalid,
    /// 事件游标无法继续推进；在任何换绑或晋升前失败关闭。
    #[error("事件游标已耗尽")]
    EventCursorExhausted,
    /// 候选构造失败：某条已注册路线对 target 根重编译失败（同修订下
    /// 属防御深度，语义上不可达）。
    #[error("路线对 target 根重编译失败")]
    RouteRevalidationFailed,
    /// 晋升后占用索引重建失败（同修订不变量下语义上不可达）。
    #[error("占用索引重建失败")]
    OccupancyRebuild(#[from] StepError),
    /// 暂存结构或路线编译缓冲容量预留失败。
    ///
    /// 切换候选表和每条 compiled-route 热表都在提交前可失败预留；
    /// 任一失败保持旧根、旧来源、旧路线和旧占用不变。
    #[error("切换暂存容量预留失败")]
    StagingAllocFailed,
    /// 引用不存在：base 侧车道边在 target 修订无稳定对应（#513 切片 C；
    /// 切换合同 §3 不可映射）。
    #[error("车道边在 target 修订不存在稳定对应（base 序数 {base_edge}）")]
    UnmappableLaneEdge {
        /// base 侧车道边序数原始值。
        base_edge: u32,
    },
    /// 引用不存在：base 侧停车位在 target 修订无稳定对应。
    #[error("停车位在 target 修订不存在稳定对应（base 序数 {base_space}）")]
    UnmappableParkingSpace {
        /// base 侧停车位序数原始值。
        base_space: u32,
    },
    /// 引用不存在：base 侧车辆 profile 在 target 修订无稳定对应。
    #[error("车辆 profile 在 target 修订不存在稳定对应（base 序数 {base_profile}）")]
    UnmappableVehicleProfile {
        /// base 侧车辆 profile 序数原始值。
        base_profile: u32,
    },
    /// 引用不存在：base 侧参与者类别在 target 修订无稳定对应。
    #[error("参与者类别在 target 修订不存在稳定对应（base 序数 {base_class}）")]
    UnmappableParticipantClass {
        /// base 侧参与者类别序数原始值。
        base_class: u32,
    },
    /// 重绑后非法：车辆原样重绑违反 target 修订不变量（进度越界、超速、
    /// 后缀访问被拒或与其它迁移车辆重叠）。
    #[error("车辆原样重绑违反 target 不变量（车辆序数 {vehicle}）")]
    VehicleRevalidationFailed {
        /// 违反不变量的车辆槽位序数。
        vehicle: u32,
    },
    /// 迁移后路线总 occurrence 超出世界容量配置（防御性闭合：迁移本身
    /// 不增减 occurrence）。
    #[error("迁移后路线总 occurrence {total} 超出容量 {capacity}")]
    EdgeOccurrenceCapacityExceeded {
        /// 迁移后路线边 occurrence 总数。
        total: u64,
        /// 世界配置的 occurrence 容量。
        capacity: u64,
    },
    /// 已存在在途切换事务（切换合同 §4 在途唯一；#513 切片 C）。
    #[error("存在在途切换事务")]
    InFlightTransaction,
    /// 迁移增量日志溢出（粘性失败；事务放弃，旧世界不受影响）。
    #[error("迁移增量日志溢出")]
    JournalOverflow,
    /// 追赶滞后（tick 距离）超过上限。
    #[error("追赶滞后 {lag} tick 超过上限 {limit}")]
    CatchUpLagExceeded {
        /// 事务启动时观察到的滞后。
        lag: u64,
        /// 配置的滞后上限。
        limit: u64,
    },
    /// 静默期确定性状态摘要复核失败（候选侧损坏；切换合同 §5）。
    #[error("静默期确定性摘要复核失败")]
    DigestMismatch,
    /// 切换事务已结算（提交或放弃后不可继续泵入或提交）。
    #[error("切换事务已结算")]
    TransactionSettled,
    /// 事务与世界的日志配对被破坏：世界未持有本事务武装的日志。
    #[error("世界未武装迁移增量日志")]
    JournalMissing,
    /// 描述符事件基线游标与本世界已提交事件计数不一致（#513 切片 C-4：
    /// 事件批次通道引入后在签发入口收紧的轴）。
    #[error("事件基线游标不一致：描述符 {descriptor}，世界 {world}")]
    BaselineEventCursorMismatch {
        /// 描述符携带的基线事件游标。
        descriptor: u64,
        /// 事务启动时世界的已提交事件计数。
        world: u64,
    },
    /// 迁移增量重放与候选槽位布局不一致（内部不变量破坏）。
    #[error("迁移增量重放与候选布局不一致")]
    ReplayInconsistent,
    /// 重绑后非法（派生轴）：target profile 派生的参与者类别或车长与
    /// 车辆存量不一致——直移保留存量会使迁移态与 save/restore 的派生态
    /// 分歧（class 侧恢复被拒、length 侧静默漂移），按不可映射失败关闭。
    #[error("target profile 派生属性与车辆存量不一致（车辆序数 {vehicle}）")]
    ProfileDerivationMismatch {
        /// 违反派生不变量的车辆槽位序数。
        vehicle: u32,
    },
    /// 事务与其传入的世界不匹配：事务绑定构造它的世界身份与世代，
    /// 传入其它世界按此失败关闭（防止误解除他世界的在途日志）。
    #[error("切换事务与传入世界不匹配（事务属世界 {expected_world}）")]
    TransactionWorldMismatch {
        /// 事务所属的世界身份。
        expected_world: u64,
    },
}

/// 切换事件（#302 切换合同 §6）：迁移函数生成、准备期不可见、只与新
/// 修订/状态绑定原子提交恰一次的封闭枚举。v1 不含实体消失类生命周期
/// 事件；不得扩展为通用事件通道。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CutoverEvent {
    /// 修订切换已提交：世界世代与新修订标识（观测失效与句柄保持语义的
    /// 规范通知面）。
    RevisionCutoverCommitted {
        /// 晋升后的世界世代。
        world_generation: WorldGeneration,
        /// 新活动聚合的路网修订标识。
        network_revision: NetworkRevisionId,
    },
}

/// 切换事件批次：成功提交时恰一次交付的规范排序集合。v1 每次成功切换
/// 恰含一条 [`CutoverEvent::RevisionCutoverCommitted`]；放弃零发布。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CutoverEventBatch {
    events: Vec<CutoverEvent>,
}

impl CutoverEventBatch {
    pub(crate) fn revision_cutover_committed(
        world_generation: WorldGeneration,
        network_revision: NetworkRevisionId,
    ) -> Self {
        Self {
            events: vec![CutoverEvent::RevisionCutoverCommitted {
                world_generation,
                network_revision,
            }],
        }
    }

    /// 批次内事件（规范排序）。
    #[must_use]
    pub fn as_slice(&self) -> &[CutoverEvent] {
        self.events.as_slice()
    }

    /// 批次是否为空（允许空集是合同形态，v1 每次成功切换恒非空）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub(crate) fn len(&self) -> u64 {
        u64::try_from(self.events.len()).expect("event count fits u64")
    }
}

impl TrafficWorld {
    /// 签发当前活动聚合的切换世界绑定。
    ///
    /// 输入命令游标取当前世界已应用命令计数；事件游标取已提交切换事件
    /// 计数（#513 切片 C-4 起为真实轴）。调用方无需复制世界身份、世代
    /// 或游标拼装逻辑。
    #[must_use]
    pub const fn world_binding(&self) -> WorldBinding {
        WorldBinding::new(
            self.world_id,
            self.world_generation,
            self.command_cursor,
            self.event_cursor,
        )
    }

    /// 同修订换根事务（#302 切换合同 §3/§4 的 `same_revision_restore`）。
    ///
    /// 成功返回切换事件批次（恰一次交付）；世界事件游标同界递增。
    ///
    /// 在固定步进安全边界（生命周期命令只能在两次 `step` 之间调用）以
    /// 原子方式把活动根换为同修订重发布/重编译制品：动态状态原样保留，
    /// 每条已注册路线的 compiled 表对 target 根原地重编译（当期
    /// `RouteHandle` / `VehicleHandle` 保持有效，ADR 0029 §6），信号灯色
    /// 与占用索引按 target 根重建。任一验证或构造失败都失败关闭：旧
    /// 世界原样继续、零可观察变化。
    ///
    /// 事务边界说明（v1 同步形态）：基准捕获与「日志武装」在同一次调用
    /// 内完成——同修订换根不改变动态状态，迁移增量日志退化为空路径；
    /// 在途唯一性由同步入口保证（不存在并发候选）；世代复核由入口处对
    /// base 绑定的认证承担。旧修订回收由 `Arc` 引用计数自然承担（最后
    /// 借用退出即回收）。
    pub fn cutover_same_revision(
        &mut self,
        target_revision: Arc<SharedNetworkRevision>,
        target_source: CommittedNetworkSource,
        descriptor: &NetworkRevisionCutoverDescriptor,
        limits: &CutoverPreflightLimits,
    ) -> Result<CutoverEventBatch, CutoverError> {
        // 在途唯一：武装中的日志 ⟺ 存在在途切换事务（切换合同 §4）。
        if self.migration_journal.is_some() {
            return Err(CutoverError::InFlightTransaction);
        }
        // 认证先于策略拒绝：伪造 origins 的描述符必须先收到 origin
        // 认证错误，而不是被策略门遮蔽（#516 同一原则）。
        let base_origin = *self.revision.canonical_origin();
        let target_origin = *target_revision.canonical_origin();
        descriptor.validate(base_origin, target_origin, limits)?;
        // worldBinding：世界身份、活动世代与命令/事件双基线游标都在
        // 事务启动时逐项比对。
        if descriptor.world_binding().world_id() != self.world_id
            || descriptor.world_binding().world_generation() != self.world_generation
        {
            return Err(CutoverError::WorldBindingMismatch);
        }
        if descriptor.world_binding().baseline_command_cursor() != self.command_cursor {
            return Err(CutoverError::BaselineCommandCursorMismatch {
                descriptor: descriptor.world_binding().baseline_command_cursor(),
                world: self.command_cursor,
            });
        }
        if descriptor.world_binding().baseline_event_cursor() != self.event_cursor {
            return Err(CutoverError::BaselineEventCursorMismatch {
                descriptor: descriptor.world_binding().baseline_event_cursor(),
                world: self.event_cursor,
            });
        }
        if descriptor.policy_kind() != MigrationPolicyKind::SameRevisionRestore {
            return Err(CutoverError::PolicyMismatch);
        }
        if target_source.network_revision() != target_origin.network_revision() {
            return Err(CutoverError::TargetSourceRevisionMismatch);
        }
        // 同修订不变量：实体集合与规范排序一致，槽位形态保持。
        let space_count = |revision: &Arc<SharedNetworkRevision>| {
            usize::try_from(
                revision
                    .traffic()
                    .entity_counts()
                    .count(laneflow_static_contract::EntityKind::ParkingSpace),
            )
            .unwrap_or(0)
        };
        if self.revision.traffic().lane_edge_count() != target_revision.traffic().lane_edge_count()
            || space_count(&self.revision) != space_count(&target_revision)
        {
            return Err(CutoverError::RouteRevalidationFailed);
        }
        // 世代耗尽必须在任何候选暂存/分配之前失败关闭；成功值只在
        // Quiescent Commit 与根、来源和动态派生状态同界写入。
        let next_world_generation = self
            .world_generation
            .checked_next()
            .ok_or(CutoverError::WorldGenerationExhausted)?;
        // Prepare（staging，失败不触及旧世界）：逐路线对 target 根重编译。
        let target_traffic = target_revision.traffic();
        let mut staged: Vec<(usize, CompiledRoute)> = Vec::new();
        staged
            .try_reserve(self.routes.len())
            .map_err(|_| CutoverError::StagingAllocFailed)?;
        for (index, slot) in self.routes.iter().enumerate() {
            if let Some(compiled) = slot.compiled.as_ref() {
                staged.push((
                    index,
                    compile_route(target_traffic, compiled.edges.as_slice()).map_err(|error| {
                        if error == RouteError::AllocationFailed {
                            CutoverError::StagingAllocFailed
                        } else {
                            CutoverError::RouteRevalidationFailed
                        }
                    })?,
                ));
            }
        }
        // Prepare（续）：针对 target 根与 staged 路线在暂存区完成可失败的
        // 占用索引重建；commit 段只剩不可失败换绑（#302 切换合同 §4）。
        let staged_occupancy = self.build_occupancy_index_for(&target_revision, &staged)?;
        // 事件批次与游标推进量在换绑前构建并预检：耗尽先于任何突变失败关闭。
        let events = CutoverEventBatch::revision_cutover_committed(
            next_world_generation,
            target_origin.network_revision(),
        );
        let event_advance = events.len();
        self.event_cursor
            .checked_add(event_advance)
            .ok_or(CutoverError::EventCursorExhausted)?;
        // Quiescent Commit：全部可失败步骤已过，剩余为不可失败的原地换绑。
        self.revision = target_revision;
        for (index, compiled) in staged {
            if let Some(slot) = self.routes.get_mut(index) {
                slot.compiled = Some(compiled);
            }
        }
        self.source = target_source;
        self.refresh_signals();
        self.occupancy = staged_occupancy;
        self.world_generation = next_world_generation;
        self.observation_state_sequence = ObservationStateSequence::INITIAL;
        self.event_cursor += event_advance;
        Ok(events)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    use super::*;

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
    );
    const MIN_HEADLESS: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-variants/min-headless.lfca"
    );
    const PROVENANCE_BASE: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-variants/provenance-base.lfca"
    );
    const PROVENANCE_BUILD: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-variants/provenance-build.lfca"
    );

    fn origin(bytes: &'static [u8], retain: bool) -> CanonicalNetworkOrigin {
        let input = check_canonical_network_input(bytes, FormatLimits::HARD)
            .expect("checked canonical network input");
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                if retain {
                    SpatialBuildOption::RetainAvailable
                } else {
                    SpatialBuildOption::Omit
                },
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("shared network revision");
        *revision.canonical_origin()
    }

    fn limits() -> CutoverPreflightLimits {
        CutoverPreflightLimits::new(1_048_576)
    }

    fn lfsd(format_version: u16, length: u64) -> SemanticDiffOriginBinding {
        SemanticDiffOriginBinding::new(
            format_version,
            Sha256Digest::from_bytes([5; 32]),
            ExactByteLength::new(length),
        )
    }

    #[test]
    fn same_revision_descriptor_validates() {
        let origin = origin(FULL_SPATIAL, true);
        let descriptor = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(origin),
            LfcaOriginBinding::from_canonical_origin(origin),
            None,
            MigrationPolicyKind::SameRevisionRestore,
            WorldBinding::new(7, WorldGeneration::INITIAL, 0, 0),
        );
        assert_eq!(descriptor.validate(origin, origin, &limits()), Ok(()));
        assert_eq!(
            descriptor.format_version(),
            CUTOVER_DESCRIPTOR_FORMAT_VERSION
        );
        assert_eq!(descriptor.world_binding().baseline_command_cursor(), 0);
        assert_eq!(descriptor.world_binding().baseline_event_cursor(), 0);
        assert_eq!(
            descriptor.world_binding().world_generation(),
            WorldGeneration::INITIAL
        );
    }

    #[test]
    fn cross_revision_descriptor_validates_within_lfsd_limit() {
        let base = origin(FULL_SPATIAL, true);
        let target = origin(MIN_HEADLESS, false);
        assert_ne!(base.network_revision(), target.network_revision());
        let descriptor = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(base),
            LfcaOriginBinding::from_canonical_origin(target),
            Some(lfsd(SEMANTIC_DIFF_FORMAT_VERSION, 4_096)),
            MigrationPolicyKind::CrossRevisionDirect,
            WorldBinding::new(7, WorldGeneration::INITIAL, 0, 0),
        );
        assert_eq!(descriptor.validate(base, target, &limits()), Ok(()));
    }

    #[test]
    fn origin_mismatches_fail_closed() {
        let origin = origin(FULL_SPATIAL, true);
        let mutated = LfcaOriginBinding::new(
            Sha256Digest::from_bytes([1; 32]),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
            NETWORK_REVISION_DERIVATION_VERSION,
        );
        let base = NetworkRevisionCutoverDescriptor::new(
            mutated,
            LfcaOriginBinding::from_canonical_origin(origin),
            None,
            MigrationPolicyKind::SameRevisionRestore,
            WorldBinding::new(1, WorldGeneration::INITIAL, 0, 0),
        );
        assert_eq!(
            base.validate(origin, origin, &limits()),
            Err(CutoverDescriptorError::BaseOriginMismatch)
        );
        let target = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(origin),
            mutated,
            None,
            MigrationPolicyKind::SameRevisionRestore,
            WorldBinding::new(1, WorldGeneration::INITIAL, 0, 0),
        );
        assert_eq!(
            target.validate(origin, origin, &limits()),
            Err(CutoverDescriptorError::TargetOriginMismatch)
        );
    }

    #[test]
    fn policy_semantics_fail_closed() {
        let base = origin(FULL_SPATIAL, true);
        let other = origin(MIN_HEADLESS, false);
        let with_lfsd = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(base),
            LfcaOriginBinding::from_canonical_origin(base),
            Some(lfsd(SEMANTIC_DIFF_FORMAT_VERSION, 1)),
            MigrationPolicyKind::SameRevisionRestore,
            WorldBinding::new(1, WorldGeneration::INITIAL, 0, 0),
        );
        assert_eq!(
            with_lfsd.validate(base, base, &limits()),
            Err(CutoverDescriptorError::SameRevisionForbidsSemanticDiff)
        );
        let unequal = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(base),
            LfcaOriginBinding::from_canonical_origin(other),
            None,
            MigrationPolicyKind::SameRevisionRestore,
            WorldBinding::new(1, WorldGeneration::INITIAL, 0, 0),
        );
        assert_eq!(
            unequal.validate(base, other, &limits()),
            Err(CutoverDescriptorError::SameRevisionRequiresEqualRevisions)
        );
        let without_lfsd = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(base),
            LfcaOriginBinding::from_canonical_origin(other),
            None,
            MigrationPolicyKind::CrossRevisionDirect,
            WorldBinding::new(1, WorldGeneration::INITIAL, 0, 0),
        );
        assert_eq!(
            without_lfsd.validate(base, other, &limits()),
            Err(CutoverDescriptorError::CrossRevisionRequiresSemanticDiff)
        );
    }

    #[test]
    fn lfsd_preflight_rejects_before_any_parsing() {
        let base = origin(FULL_SPATIAL, true);
        let target = origin(MIN_HEADLESS, false);
        let oversized = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(base),
            LfcaOriginBinding::from_canonical_origin(target),
            Some(lfsd(SEMANTIC_DIFF_FORMAT_VERSION, 2_048)),
            MigrationPolicyKind::CrossRevisionDirect,
            WorldBinding::new(1, WorldGeneration::INITIAL, 0, 0),
        );
        let tight = CutoverPreflightLimits::new(1_024);
        assert_eq!(
            oversized.validate(base, target, &tight),
            Err(CutoverDescriptorError::SemanticDiffByteLengthOverLimit {
                length: 2_048,
                limit: 1_024,
            })
        );
        let wrong_format = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(base),
            LfcaOriginBinding::from_canonical_origin(target),
            Some(lfsd(99, 1)),
            MigrationPolicyKind::CrossRevisionDirect,
            WorldBinding::new(1, WorldGeneration::INITIAL, 0, 0),
        );
        assert_eq!(
            wrong_format.validate(base, target, &limits()),
            Err(
                CutoverDescriptorError::SemanticDiffFormatVersionUnsupported {
                    binding: 99,
                    supported: SEMANTIC_DIFF_FORMAT_VERSION,
                }
            )
        );
    }

    #[test]
    fn derivation_version_axis_rejects_unsupported() {
        let origin_value = origin(FULL_SPATIAL, true);
        let stale = LfcaOriginBinding::new(
            origin_value.canonical_artifact_digest(),
            origin_value.canonical_artifact_byte_length(),
            origin_value.network_revision(),
            0,
        );
        let descriptor = NetworkRevisionCutoverDescriptor::new(
            stale,
            LfcaOriginBinding::from_canonical_origin(origin_value),
            None,
            MigrationPolicyKind::SameRevisionRestore,
            WorldBinding::new(1, WorldGeneration::INITIAL, 0, 0),
        );
        assert_eq!(
            descriptor.validate(origin_value, origin_value, &limits()),
            Err(
                CutoverDescriptorError::RevisionDerivationVersionUnsupported {
                    binding: 0,
                    supported: NETWORK_REVISION_DERIVATION_VERSION,
                }
            )
        );
    }

    #[test]
    fn cross_revision_rejects_equal_revisions() {
        let base = origin(FULL_SPATIAL, true);
        let equal = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(base),
            LfcaOriginBinding::from_canonical_origin(base),
            Some(lfsd(SEMANTIC_DIFF_FORMAT_VERSION, 1)),
            MigrationPolicyKind::CrossRevisionDirect,
            WorldBinding::new(1, WorldGeneration::INITIAL, 0, 0),
        );
        assert_eq!(
            equal.validate(base, base, &limits()),
            Err(CutoverDescriptorError::CrossRevisionRequiresUnequalRevisions)
        );
    }

    #[test]
    fn same_revision_restore_accepts_republished_bytes() {
        // provenance-base / provenance-build 是同文档不同构建来源：
        // 同 NetworkRevisionId、不同 exact bytes（ADR 0025 §8 重发布语义）。
        let base = origin(PROVENANCE_BASE, false);
        let target = origin(PROVENANCE_BUILD, false);
        assert_eq!(base.network_revision(), target.network_revision());
        assert_ne!(
            base.canonical_artifact_digest(),
            target.canonical_artifact_digest()
        );
        let descriptor = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(base),
            LfcaOriginBinding::from_canonical_origin(target),
            None,
            MigrationPolicyKind::SameRevisionRestore,
            WorldBinding::new(3, WorldGeneration::INITIAL, 0, 0),
        );
        assert_eq!(descriptor.validate(base, target, &limits()), Ok(()));
    }

    #[test]
    fn command_cursor_baseline_must_match_world_at_cutover() {
        let base = origin(FULL_SPATIAL, true);
        // validate 不再拒绝非零命令游标：它与描述符其余字段一样可携带
        // 任意基线值，真正的一致性比对发生在事务启动时（见下）。
        let nonzero = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(base),
            LfcaOriginBinding::from_canonical_origin(base),
            None,
            MigrationPolicyKind::SameRevisionRestore,
            WorldBinding::new(1, WorldGeneration::INITIAL, 5, 0),
        );
        assert_eq!(nonzero.validate(base, base, &limits()), Ok(()));
        assert_eq!(nonzero.world_binding().baseline_command_cursor(), 5);
    }

    #[cfg(test)]
    pub(crate) mod transaction_tests {
        use crate::PublishedLfcaReference;
        use laneflow_format::{FormatLimits, check_canonical_network_input};
        use laneflow_static_network::{
            SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
            build_shared_network_revision,
        };

        use super::*;
        use crate::tables::with_route_allocation_failure_after;
        use crate::{
            ParkingError, ReplaceError, RouteRegisterInput, SpawnError, TickInput,
            VehicleSpawnInput, VehicleStatus, WorldConfig,
        };

        /// 同一 LFCA 字节按不同 Spatial 构建选项构建两个根对象：origin
        /// 四联相同（同修订换根判据成立），根对象身份不同（真实换绑路径）。
        /// 可行驶的同修订不同字节对不在现有夹具内；provenance 对只覆盖
        /// 描述符层验证（见 cutover::tests）。
        pub(crate) fn revision(retain: bool) -> Arc<SharedNetworkRevision> {
            let input = check_canonical_network_input(
                include_bytes!(
                    "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
                ),
                FormatLimits::HARD,
            )
            .expect("checked canonical network input");
            build_shared_network_revision(
                input,
                SharedNetworkBuildOptions::new(
                    if retain {
                        SpatialBuildOption::RetainAvailable
                    } else {
                        SpatialBuildOption::Omit
                    },
                    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
                ),
            )
            .expect("shared network revision")
        }

        pub(crate) fn source_for(
            origin: CanonicalNetworkOrigin,
            key: &str,
        ) -> CommittedNetworkSource {
            CommittedNetworkSource::Published {
                reference: PublishedLfcaReference::new(
                    key,
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .expect("non-empty key"),
            }
        }

        pub(crate) fn world_with_vehicle(
            retain: bool,
        ) -> (TrafficWorld, crate::RouteHandle, crate::VehicleHandle) {
            let revision = revision(retain);
            let origin = *revision.canonical_origin();
            let mut world = TrafficWorld::install(
                Arc::clone(&revision),
                WorldConfig::new(8, 4, 1_024, 1, 100),
                source_for(origin, "fixture://base"),
                1,
            )
            .expect("install");
            let first = laneflow_static_contract::LaneEdgeOrdinal::from_raw(0);
            let successors: &[laneflow_static_contract::LaneEdgeOrdinal] =
                world.traffic().successors(first).unwrap_or(&[]);
            let edges = if let Some(second) = successors.first() {
                vec![first, *second]
            } else {
                vec![first]
            };
            let route = world
                .register_route(RouteRegisterInput::new(edges))
                .expect("route");
            let vehicle = world
                .spawn_vehicle(VehicleSpawnInput::new(
                    laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    1_000,
                    0,
                ))
                .expect("vehicle");
            (world, route, vehicle)
        }

        fn limits() -> CutoverPreflightLimits {
            CutoverPreflightLimits::new(1_048_576)
        }

        #[test]
        fn same_revision_cutover_preserves_handles_and_state() {
            let (mut world, route, vehicle) = world_with_vehicle(true);
            for _ in 0..3 {
                world.step(TickInput::new(100)).expect("step before");
            }
            let before = world.vehicle_state(vehicle).copied().expect("vehicle");
            let edges_before: Vec<_> = world.route_edges(route).expect("route").to_vec();
            let base_origin = *world.revision().canonical_origin();

            let target = revision(false);
            let target_origin = *target.canonical_origin();
            let target_source = source_for(target_origin, "fixture://republished");
            let descriptor = NetworkRevisionCutoverDescriptor::new(
                LfcaOriginBinding::from_canonical_origin(base_origin),
                LfcaOriginBinding::from_canonical_origin(target_origin),
                None,
                MigrationPolicyKind::SameRevisionRestore,
                world.world_binding(),
            );
            let before_generation = world.world_generation();
            world
                .cutover_same_revision(Arc::clone(&target), target_source, &descriptor, &limits())
                .expect("same-revision cutover");

            // 根与来源已换绑；句柄继续寻址同一逻辑实体，动态状态逐字段不变。
            assert_eq!(
                world
                    .revision()
                    .canonical_origin()
                    .canonical_artifact_digest(),
                target_origin.canonical_artifact_digest()
            );
            assert_eq!(
                world.committed_source(),
                &source_for(target_origin, "fixture://republished")
            );
            let after = world.vehicle_state(vehicle).copied().expect("vehicle");
            assert_eq!(before.handle, after.handle);
            assert_eq!(before.route, after.route);
            assert_eq!(before.route_edge_index, after.route_edge_index);
            assert_eq!(before.progress_mm, after.progress_mm);
            assert_eq!(before.carry_um, after.carry_um);
            assert_eq!(before.speed_mm_s, after.speed_mm_s);
            assert_eq!(before.status, after.status);
            let edges_after: Vec<_> = world.route_edges(route).expect("route").to_vec();
            assert_eq!(edges_before, edges_after);
            assert_eq!(world.world_generation().get(), before_generation.get() + 1);
            // 换绑后世界继续确定性步进。
            world.step(TickInput::new(100)).expect("step after");
        }

        #[test]
        fn same_revision_cutover_fails_closed_on_event_cursor_exhaustion() {
            let (mut world, _route, _vehicle) = world_with_vehicle(true);
            world.event_cursor = u64::MAX;
            let base_origin = *world.revision().canonical_origin();
            let target = revision(false);
            let target_origin = *target.canonical_origin();
            let descriptor = NetworkRevisionCutoverDescriptor::new(
                LfcaOriginBinding::from_canonical_origin(base_origin),
                LfcaOriginBinding::from_canonical_origin(target_origin),
                None,
                MigrationPolicyKind::SameRevisionRestore,
                world.world_binding(),
            );
            let before_generation = world.world_generation();
            assert_eq!(
                world
                    .cutover_same_revision(
                        target,
                        source_for(target_origin, "fixture://cursor-exhausted"),
                        &descriptor,
                        &limits(),
                    )
                    .unwrap_err(),
                CutoverError::EventCursorExhausted
            );
            assert_eq!(world.event_cursor, u64::MAX, "耗尽不改动游标");
            assert_eq!(world.world_generation(), before_generation);
            world.step(TickInput::new(100)).expect("world unaffected");
        }

        #[test]
        fn route_allocation_failure_aborts_without_partial_cutover() {
            let (mut world, route, vehicle) = world_with_vehicle(true);
            let before_origin = *world.revision().canonical_origin();
            let before_source = world.committed_source().clone();
            let before_state = world.vehicle_state(vehicle).copied().expect("vehicle");
            let before_edges = world.route_edges(route).expect("route").to_vec();

            let target = revision(false);
            let target_origin = *target.canonical_origin();
            let descriptor = NetworkRevisionCutoverDescriptor::new(
                LfcaOriginBinding::from_canonical_origin(before_origin),
                LfcaOriginBinding::from_canonical_origin(target_origin),
                None,
                MigrationPolicyKind::SameRevisionRestore,
                world.world_binding(),
            );
            let before_generation = world.world_generation();
            let result = with_route_allocation_failure_after(0, || {
                world.cutover_same_revision(
                    target,
                    source_for(target_origin, "fixture://republished"),
                    &descriptor,
                    &limits(),
                )
            });

            assert_eq!(result.unwrap_err(), CutoverError::StagingAllocFailed);
            assert_eq!(*world.revision().canonical_origin(), before_origin);
            assert_eq!(world.committed_source(), &before_source);
            assert_eq!(world.vehicle_state(vehicle), Some(&before_state));
            assert_eq!(world.route_edges(route), Some(before_edges.as_slice()));
            assert_eq!(world.world_generation(), before_generation);
            world
                .step(TickInput::new(100))
                .expect("old world still steps");
        }

        #[test]
        fn successful_cutover_stales_previous_world_binding() {
            let (mut world, _, _) = world_with_vehicle(true);
            let origin = *world.revision().canonical_origin();
            let stale_binding = world.world_binding();
            let first_target = revision(true);
            let descriptor = NetworkRevisionCutoverDescriptor::new(
                LfcaOriginBinding::from_canonical_origin(origin),
                LfcaOriginBinding::from_canonical_origin(origin),
                None,
                MigrationPolicyKind::SameRevisionRestore,
                stale_binding,
            );
            world
                .cutover_same_revision(
                    first_target,
                    source_for(origin, "fixture://first-cutover"),
                    &descriptor,
                    &limits(),
                )
                .expect("first cutover");
            assert_eq!(world.world_generation().get(), 1);

            let committed_root = world.revision();
            let committed_source = world.committed_source().clone();
            let generation_after_first = world.world_generation();
            let second_target = revision(true);
            assert_eq!(*second_target.canonical_origin(), origin);
            assert_eq!(
                world
                    .cutover_same_revision(
                        second_target,
                        source_for(origin, "fixture://stale-retry"),
                        &descriptor,
                        &limits(),
                    )
                    .unwrap_err(),
                CutoverError::WorldBindingMismatch
            );
            assert!(Arc::ptr_eq(&world.revision(), &committed_root));
            assert_eq!(world.committed_source(), &committed_source);
            assert_eq!(world.world_generation(), generation_after_first);
        }

        #[test]
        fn exhausted_world_generation_aborts_before_staging() {
            let (mut world, route, vehicle) = world_with_vehicle(true);
            world.world_generation = WorldGeneration::from_raw_for_test(u64::MAX);
            let before_root = world.revision();
            let before_source = world.committed_source().clone();
            let before_state = world.vehicle_state(vehicle).copied().expect("vehicle");
            let before_edges = world.route_edges(route).expect("route").to_vec();

            let target = revision(false);
            let target_origin = *target.canonical_origin();
            let descriptor = NetworkRevisionCutoverDescriptor::new(
                LfcaOriginBinding::from_canonical_origin(*before_root.canonical_origin()),
                LfcaOriginBinding::from_canonical_origin(target_origin),
                None,
                MigrationPolicyKind::SameRevisionRestore,
                world.world_binding(),
            );
            assert_eq!(
                world
                    .cutover_same_revision(
                        target,
                        source_for(target_origin, "fixture://generation-exhausted"),
                        &descriptor,
                        &limits(),
                    )
                    .unwrap_err(),
                CutoverError::WorldGenerationExhausted
            );
            assert!(Arc::ptr_eq(&world.revision(), &before_root));
            assert_eq!(world.committed_source(), &before_source);
            assert_eq!(world.world_generation().get(), u64::MAX);
            assert_eq!(world.vehicle_state(vehicle), Some(&before_state));
            assert_eq!(world.route_edges(route), Some(before_edges.as_slice()));
        }

        #[test]
        fn same_revision_cutover_is_logically_identity() {
            // 逻辑恒等 oracle：切换世界与未切换世界的后续步进逐点一致。
            // 逐点 = 每一步之后比对完整规范化已提交状态（tick/时间、全部
            // 车辆整值状态、位姿来源、信号灯色组、停车占用），并在切换
            // 边界处（cutover 后、继续步进前）先比对一次；各世界只使用
            // 自己的句柄。
            let (mut cut, _, _) = world_with_vehicle(true);
            let (mut plain, _, _) = world_with_vehicle(true);
            let assert_committed_state_equal = |cut: &TrafficWorld, plain: &TrafficWorld| {
                crate::cutover_migration::assert_committed_logical_state_equal(cut, plain);
            };
            for _ in 0..2 {
                cut.step(TickInput::new(100)).expect("step cut");
                plain.step(TickInput::new(100)).expect("step plain");
                assert_committed_state_equal(&cut, &plain);
            }
            let base_origin = *cut.revision().canonical_origin();
            let target = revision(false);
            let target_origin = *target.canonical_origin();
            let descriptor = NetworkRevisionCutoverDescriptor::new(
                LfcaOriginBinding::from_canonical_origin(base_origin),
                LfcaOriginBinding::from_canonical_origin(target_origin),
                None,
                MigrationPolicyKind::SameRevisionRestore,
                cut.world_binding(),
            );
            cut.cutover_same_revision(
                target,
                source_for(target_origin, "fixture://republished"),
                &descriptor,
                &limits(),
            )
            .expect("cutover");
            // 切换边界：提交后、继续步进前，已提交状态必须已与未切换
            // 世界逐点一致。
            assert_committed_state_equal(&cut, &plain);
            for _ in 0..8 {
                cut.step(TickInput::new(100)).expect("step cut");
                plain.step(TickInput::new(100)).expect("step plain");
                assert_committed_state_equal(&cut, &plain);
            }
        }

        #[test]
        fn policy_mismatch_rejects_before_any_change() {
            let (mut world, _, _) = world_with_vehicle(true);
            let before_origin = *world.revision().canonical_origin();
            let target = revision(false);
            let target_origin = *target.canonical_origin();
            let descriptor = NetworkRevisionCutoverDescriptor::new(
                LfcaOriginBinding::from_canonical_origin(before_origin),
                LfcaOriginBinding::from_canonical_origin(target_origin),
                Some(SemanticDiffOriginBinding::new(
                    2,
                    laneflow_static_contract::Sha256Digest::from_bytes([5; 32]),
                    laneflow_static_contract::ExactByteLength::new(1),
                )),
                MigrationPolicyKind::CrossRevisionDirect,
                world.world_binding(),
            );
            assert_eq!(
                world
                    .cutover_same_revision(
                        target,
                        source_for(target_origin, "fixture://x"),
                        &descriptor,
                        &limits()
                    )
                    .unwrap_err(),
                // 认证先于策略：同修订 + CrossRevisionDirect 先被描述符
                // 验证的互斥判据拒绝；策略门只拦「验证通过但不属本入口」的策略。
                CutoverError::Descriptor(
                    CutoverDescriptorError::CrossRevisionRequiresUnequalRevisions
                )
            );
            assert_eq!(
                *world.revision().canonical_origin(),
                before_origin,
                "old world keeps its root"
            );
            world
                .step(TickInput::new(100))
                .expect("old world still steps");
        }

        #[test]
        fn world_binding_mismatch_rejects() {
            let (mut world, _, _) = world_with_vehicle(true);
            let before_origin = *world.revision().canonical_origin();
            assert_eq!(world.world_id(), 1);
            let target = revision(false);
            let target_origin = *target.canonical_origin();
            // 描述符签发给其它世界（world_id=9）。
            let descriptor = NetworkRevisionCutoverDescriptor::new(
                LfcaOriginBinding::from_canonical_origin(before_origin),
                LfcaOriginBinding::from_canonical_origin(target_origin),
                None,
                MigrationPolicyKind::SameRevisionRestore,
                WorldBinding::new(9, WorldGeneration::INITIAL, 0, 0),
            );
            assert_eq!(
                world
                    .cutover_same_revision(
                        target,
                        source_for(target_origin, "fixture://x"),
                        &descriptor,
                        &limits(),
                    )
                    .unwrap_err(),
                CutoverError::WorldBindingMismatch
            );
            assert_eq!(*world.revision().canonical_origin(), before_origin);
            world
                .step(TickInput::new(100))
                .expect("old world still steps");
        }

        #[test]
        fn target_source_mismatch_fails_closed() {
            let (mut world, _, _) = world_with_vehicle(true);
            let before_origin = *world.revision().canonical_origin();
            let before_source = world.committed_source().clone();
            let target = revision(false);
            let target_origin = *target.canonical_origin();
            let descriptor = NetworkRevisionCutoverDescriptor::new(
                LfcaOriginBinding::from_canonical_origin(before_origin),
                LfcaOriginBinding::from_canonical_origin(target_origin),
                None,
                MigrationPolicyKind::SameRevisionRestore,
                world.world_binding(),
            );
            // 伪造指向其它修订的来源：来源绑定验证失败关闭。
            let wrong_revision = laneflow_static_contract::NetworkRevisionId::from_digest(
                laneflow_static_contract::Sha256Digest::from_bytes([1; 32]),
            );
            let wrong_source = CommittedNetworkSource::Published {
                reference: PublishedLfcaReference::new(
                    "fixture://wrong-target",
                    before_origin.canonical_artifact_digest(),
                    before_origin.canonical_artifact_byte_length(),
                    wrong_revision,
                )
                .expect("non-empty key"),
            };
            assert_eq!(
                world
                    .cutover_same_revision(target, wrong_source, &descriptor, &limits())
                    .unwrap_err(),
                CutoverError::TargetSourceRevisionMismatch
            );
            assert_eq!(*world.revision().canonical_origin(), before_origin);
            assert_eq!(world.committed_source(), &before_source);
            world
                .step(TickInput::new(100))
                .expect("old world still steps");
        }

        #[test]
        fn command_cursor_counts_applied_commands_only() {
            let (mut world, _, vehicle) = world_with_vehicle(true);
            // 夹具 = 1 次路线注册 + 1 次车辆生成。
            assert_eq!(world.command_cursor(), 2);
            // step 不是输入命令。
            world.step(TickInput::new(100)).expect("step");
            assert_eq!(world.command_cursor(), 2);
            // 成功的停车占用计数。
            let space = laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0);
            world.occupy_parking(vehicle, space).expect("parking");
            assert_eq!(world.command_cursor(), 3);
            // 幂等成功同样计数。
            world.occupy_parking(vehicle, space).expect("idempotent");
            assert_eq!(world.command_cursor(), 4);
        }

        #[test]
        fn command_cursor_exhaustion_fails_closed_for_lifecycle_commands() {
            let (mut world, route, vehicle) = world_with_vehicle(true);
            let route_edges = world.route_edges(route).expect("route").to_vec();
            let spawn = VehicleSpawnInput::new(
                laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            );
            let removable_route = world
                .register_route(RouteRegisterInput::new(route_edges.clone()))
                .expect("route without vehicles");

            world.command_cursor = u64::MAX;
            assert_eq!(
                world
                    .register_route(RouteRegisterInput::new(route_edges.clone()))
                    .unwrap_err(),
                RouteError::CommandCursorExhausted
            );
            assert_eq!(world.live_route_count, 2);
            assert_eq!(
                world.remove_route(removable_route).unwrap_err(),
                RouteError::CommandCursorExhausted
            );
            assert_eq!(
                world.route_edges(removable_route),
                Some(route_edges.as_slice())
            );

            let vehicle_index = usize::try_from(vehicle.index()).expect("vehicle index");
            world.vehicles[vehicle_index]
                .state
                .as_mut()
                .expect("vehicle")
                .status = VehicleStatus::Completed;
            let before_completed = *world.vehicle_state(vehicle).expect("completed vehicle");
            assert_eq!(
                world.spawn_vehicle(spawn).unwrap_err(),
                SpawnError::CommandCursorExhausted
            );
            assert_eq!(world.vehicle_state(vehicle), Some(&before_completed));
            assert_eq!(
                world.replace_completed_vehicle(vehicle, spawn).unwrap_err(),
                ReplaceError::CommandCursorExhausted
            );
            assert_eq!(world.vehicle_state(vehicle), Some(&before_completed));

            world.vehicles[vehicle_index]
                .state
                .as_mut()
                .expect("vehicle")
                .status = VehicleStatus::Active;
            let before_active = *world.vehicle_state(vehicle).expect("active vehicle");
            let space = laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0);
            assert_eq!(
                world.occupy_parking(vehicle, space).unwrap_err(),
                ParkingError::CommandCursorExhausted
            );
            assert_eq!(world.vehicle_state(vehicle), Some(&before_active));
            assert_eq!(world.parking_occupants[space.index()], None);
            assert_eq!(world.command_cursor(), u64::MAX);

            let (mut parked_world, _, parked_vehicle) = world_with_vehicle(true);
            parked_world
                .occupy_parking(parked_vehicle, space)
                .expect("parking");
            parked_world.command_cursor = u64::MAX;
            let before_parked = *parked_world
                .vehicle_state(parked_vehicle)
                .expect("parked vehicle");
            assert_eq!(
                parked_world
                    .occupy_parking(parked_vehicle, space)
                    .unwrap_err(),
                ParkingError::CommandCursorExhausted
            );
            assert_eq!(
                parked_world.vehicle_state(parked_vehicle),
                Some(&before_parked)
            );
            assert_eq!(
                parked_world.parking_occupants[space.index()],
                Some(parked_vehicle)
            );
            assert_eq!(parked_world.command_cursor(), u64::MAX);
        }

        #[test]
        fn command_cursor_baseline_mismatch_fails_closed() {
            let (mut world, _, _) = world_with_vehicle(true);
            let before_origin = *world.revision().canonical_origin();
            let target = revision(false);
            let target_origin = *target.canonical_origin();
            let descriptor = NetworkRevisionCutoverDescriptor::new(
                LfcaOriginBinding::from_canonical_origin(before_origin),
                LfcaOriginBinding::from_canonical_origin(target_origin),
                None,
                MigrationPolicyKind::SameRevisionRestore,
                WorldBinding::new(1, world.world_generation(), world.command_cursor() + 1, 0),
            );
            assert_eq!(
                world
                    .cutover_same_revision(
                        target,
                        source_for(target_origin, "fixture://stale-baseline"),
                        &descriptor,
                        &limits(),
                    )
                    .unwrap_err(),
                CutoverError::BaselineCommandCursorMismatch {
                    descriptor: world.command_cursor() + 1,
                    world: world.command_cursor(),
                }
            );
            assert_eq!(*world.revision().canonical_origin(), before_origin);
        }
        #[test]
        fn event_cursor_baseline_is_compared_at_transaction_start() {
            // #513 切片 C-4：事件批次通道引入后，事件基线游标在事务启动时
            // 与世界已提交事件计数逐项比对（不再是非零即拒的预留轴）。
            let (mut world, _, _) = world_with_vehicle(true);
            let base = origin(FULL_SPATIAL, true);
            let descriptor = NetworkRevisionCutoverDescriptor::new(
                LfcaOriginBinding::from_canonical_origin(base),
                LfcaOriginBinding::from_canonical_origin(base),
                None,
                MigrationPolicyKind::SameRevisionRestore,
                WorldBinding::new(1, WorldGeneration::INITIAL, 2, 7),
            );
            assert_eq!(descriptor.validate(base, base, &limits()), Ok(()));
            assert_eq!(world.event_cursor(), 0);
            assert_eq!(
                world
                    .cutover_same_revision(
                        revision(false),
                        source_for(base, "fixture://event-cursor"),
                        &descriptor,
                        &limits(),
                    )
                    .unwrap_err(),
                CutoverError::BaselineEventCursorMismatch {
                    descriptor: 7,
                    world: 0,
                }
            );
        }

        #[test]
        fn successful_same_revision_cutover_delivers_event_batch_once() {
            let (mut world, _, _) = world_with_vehicle(true);
            let base = *world.revision().canonical_origin();
            let descriptor = NetworkRevisionCutoverDescriptor::new(
                LfcaOriginBinding::from_canonical_origin(base),
                LfcaOriginBinding::from_canonical_origin(base),
                None,
                MigrationPolicyKind::SameRevisionRestore,
                world.world_binding(),
            );
            let events = world
                .cutover_same_revision(
                    revision(false),
                    source_for(base, "fixture://event-batch"),
                    &descriptor,
                    &limits(),
                )
                .expect("same-revision cutover");
            assert_eq!(events.as_slice().len(), 1);
            assert!(matches!(
                events.as_slice()[0],
                CutoverEvent::RevisionCutoverCommitted { world_generation, network_revision }
                    if world_generation.get() == 1 && network_revision == base.network_revision()
            ));
            assert_eq!(world.event_cursor(), 1);
            assert_eq!(world.world_binding().baseline_event_cursor(), 1);
        }
    }
}
