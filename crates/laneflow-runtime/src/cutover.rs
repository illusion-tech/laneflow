//! 切换描述符（#302 切片 A 第二步）：宿主对象外可信封闭输入的进程内类型
//! 与一致性验证。状态机与原子晋升由后续步骤交付。

use laneflow_static_contract::{
    ExactByteLength, NETWORK_REVISION_DERIVATION_VERSION, NetworkRevisionId,
    SEMANTIC_DIFF_FORMAT_VERSION, Sha256Digest,
};
use std::sync::Arc;

use laneflow_static_network::{CanonicalNetworkOrigin, SharedNetworkRevision};
use thiserror::Error;

use crate::source::CommittedNetworkSource;
use crate::tables::CompiledRoute;
use crate::tables::compile_route;
use crate::{StepError, TrafficWorld};

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
}

/// 世界绑定（#302 切换合同 §2）：目标世界身份与基线命令/事件双游标。
///
/// 游标是描述符签发时点对齐的基线，不是实时值；比对时点为事务启动时
/// （后续步骤交付）。世界身份编码与二进制形态属 G2 留白。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorldBinding {
    world_id: u64,
    baseline_command_cursor: u64,
    baseline_event_cursor: u64,
}

impl WorldBinding {
    /// 构造世界绑定。
    #[must_use]
    pub const fn new(
        world_id: u64,
        baseline_command_cursor: u64,
        baseline_event_cursor: u64,
    ) -> Self {
        Self {
            world_id,
            baseline_command_cursor,
            baseline_event_cursor,
        }
    }

    /// 目标世界身份（宿主任意定义；编码 G2 留白）。
    #[must_use]
    pub const fn world_id(&self) -> u64 {
        self.world_id
    }

    /// 基线输入命令游标。
    #[must_use]
    pub const fn baseline_command_cursor(&self) -> u64 {
        self.baseline_command_cursor
    }

    /// 基线已提交事件游标。当前 Runtime 没有事件发布通道，v1 为预留
    /// 恒零值（#511 G2 记录登记的口径）。
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
    /// v1 预留事件游标非零。
    #[error("v1 预留事件游标必须为零，实际 {actual}")]
    ReservedEventCursorNonZero {
        /// 描述符携带的事件游标值。
        actual: u64,
    },
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
        if self.world.baseline_event_cursor != 0 {
            return Err(CutoverDescriptorError::ReservedEventCursorNonZero {
                actual: self.world.baseline_event_cursor,
            });
        }
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
    #[error("描述符世界绑定与本世界身份不一致")]
    WorldBindingMismatch,
    /// 候选构造失败：某条已注册路线对 target 根重编译失败（同修订下
    /// 属防御深度，语义上不可达）。
    #[error("路线对 target 根重编译失败")]
    RouteRevalidationFailed,
    /// 晋升后占用索引重建失败（同修订不变量下语义上不可达）。
    #[error("占用索引重建失败")]
    OccupancyRebuild(#[from] StepError),
}

impl TrafficWorld {
    /// 同修订换根事务（#302 切换合同 §3/§4 的 `same_revision_restore`）。
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
    /// base 绑定的认证承担。后台候选、journal 内容与摘要复核属切片 C。
    /// 旧修订回收由 `Arc` 引用计数自然承担（最后借用退出即回收）。
    pub fn cutover_same_revision(
        &mut self,
        target_revision: Arc<SharedNetworkRevision>,
        target_source: CommittedNetworkSource,
        descriptor: &NetworkRevisionCutoverDescriptor,
        limits: &CutoverPreflightLimits,
    ) -> Result<(), CutoverError> {
        // 认证先于策略拒绝：伪造 origins 的描述符必须先收到 origin
        // 认证错误，而不是被策略门遮蔽（#516 同一原则）。
        let base_origin = *self.revision.canonical_origin();
        let target_origin = *target_revision.canonical_origin();
        descriptor.validate(base_origin, target_origin, limits)?;
        // worldBinding：世界身份在事务启动时一次性比对（命令/事件基线
        // 游标随切片 B 快照面存在，当前比对世界身份）。
        if descriptor.world_binding().world_id() != self.world_id {
            return Err(CutoverError::WorldBindingMismatch);
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
        // Prepare（staging，失败不触及旧世界）：逐路线对 target 根重编译。
        let target_traffic = target_revision.traffic();
        let mut staged: Vec<(usize, CompiledRoute)> = Vec::with_capacity(self.routes.len());
        for (index, slot) in self.routes.iter().enumerate() {
            if let Some(compiled) = slot.compiled.as_ref() {
                staged.push((
                    index,
                    compile_route(target_traffic, compiled.edges.as_ref())
                        .map_err(|_| CutoverError::RouteRevalidationFailed)?,
                ));
            }
        }
        // Prepare（续）：针对 target 根与 staged 路线在暂存区完成可失败的
        // 占用索引重建；commit 段只剩不可失败换绑（#302 切换合同 §4）。
        let staged_occupancy = self.build_occupancy_index_for(&target_revision, &staged)?;
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
            WorldBinding::new(7, 120, 0),
        );
        assert_eq!(descriptor.validate(origin, origin, &limits()), Ok(()));
        assert_eq!(
            descriptor.format_version(),
            CUTOVER_DESCRIPTOR_FORMAT_VERSION
        );
        assert_eq!(descriptor.world_binding().baseline_command_cursor(), 120);
        assert_eq!(descriptor.world_binding().baseline_event_cursor(), 0);
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
            WorldBinding::new(7, 0, 0),
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
            WorldBinding::new(1, 0, 0),
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
            WorldBinding::new(1, 0, 0),
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
            WorldBinding::new(1, 0, 0),
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
            WorldBinding::new(1, 0, 0),
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
            WorldBinding::new(1, 0, 0),
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
            WorldBinding::new(1, 0, 0),
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
            WorldBinding::new(1, 0, 0),
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
            WorldBinding::new(1, 0, 0),
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
            WorldBinding::new(1, 0, 0),
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
            WorldBinding::new(3, 42, 0),
        );
        assert_eq!(descriptor.validate(base, target, &limits()), Ok(()));
    }

    #[test]
    fn reserved_event_cursor_must_be_zero_in_v1() {
        let base = origin(FULL_SPATIAL, true);
        let nonzero = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(base),
            LfcaOriginBinding::from_canonical_origin(base),
            None,
            MigrationPolicyKind::SameRevisionRestore,
            WorldBinding::new(1, 0, 7),
        );
        assert_eq!(
            nonzero.validate(base, base, &limits()),
            Err(CutoverDescriptorError::ReservedEventCursorNonZero { actual: 7 })
        );
    }

    #[cfg(test)]
    mod transaction_tests {
        use crate::PublishedLfcaReference;
        use laneflow_format::{FormatLimits, check_canonical_network_input};
        use laneflow_static_network::{
            SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
            build_shared_network_revision,
        };

        use super::*;
        use crate::{RouteRegisterInput, TickInput, VehicleSpawnInput, WorldConfig};

        const PROVENANCE_BASE: &[u8] = include_bytes!(
            "../../laneflow-compiler/tests/fixtures/portable/lfca-variants/provenance-base.lfca"
        );
        const PROVENANCE_BUILD: &[u8] = include_bytes!(
            "../../laneflow-compiler/tests/fixtures/portable/lfca-variants/provenance-build.lfca"
        );

        /// 同一 LFCA 字节按不同 Spatial 构建选项构建两个根对象：origin
        /// 四联相同（同修订换根判据成立），根对象身份不同（真实换绑路径）。
        /// 可行驶的同修订不同字节对不在现有夹具内；provenance 对只覆盖
        /// 描述符层验证（见 cutover::tests）。
        fn revision(retain: bool) -> Arc<SharedNetworkRevision> {
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

        fn source_for(origin: CanonicalNetworkOrigin, key: &str) -> CommittedNetworkSource {
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

        fn world_with_vehicle(
            retain: bool,
        ) -> (TrafficWorld, crate::RouteHandle, crate::VehicleHandle) {
            let revision = revision(retain);
            let origin = *revision.canonical_origin();
            let mut world = TrafficWorld::install(
                Arc::clone(&revision),
                WorldConfig::new(8, 4, 1, 100),
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
                WorldBinding::new(1, 5, 0),
            );
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
            // 换绑后世界继续确定性步进。
            world.step(TickInput::new(100)).expect("step after");
        }

        #[test]
        fn same_revision_cutover_is_logically_identity() {
            // 逻辑恒等 oracle：切换世界与未切换世界的后续步进逐点一致。
            let (mut cut, _, _) = world_with_vehicle(true);
            let (mut plain, _, vehicle_plain) = world_with_vehicle(true);
            for _ in 0..2 {
                cut.step(TickInput::new(100)).expect("step cut");
                plain.step(TickInput::new(100)).expect("step plain");
            }
            let base_origin = *cut.revision().canonical_origin();
            let target = revision(false);
            let target_origin = *target.canonical_origin();
            let descriptor = NetworkRevisionCutoverDescriptor::new(
                LfcaOriginBinding::from_canonical_origin(base_origin),
                LfcaOriginBinding::from_canonical_origin(target_origin),
                None,
                MigrationPolicyKind::SameRevisionRestore,
                WorldBinding::new(1, 2, 0),
            );
            cut.cutover_same_revision(
                target,
                source_for(target_origin, "fixture://republished"),
                &descriptor,
                &limits(),
            )
            .expect("cutover");
            for _ in 0..5 {
                cut.step(TickInput::new(100)).expect("step cut");
                plain.step(TickInput::new(100)).expect("step plain");
            }
            let cut_state = cut.vehicle_state(vehicle_plain).copied().expect("vehicle");
            let plain_state = plain
                .vehicle_state(vehicle_plain)
                .copied()
                .expect("vehicle");
            assert_eq!(
                (
                    cut_state.progress_mm,
                    cut_state.speed_mm_s,
                    cut_state.status
                ),
                (
                    plain_state.progress_mm,
                    plain_state.speed_mm_s,
                    plain_state.status
                )
            );
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
                WorldBinding::new(1, 0, 0),
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
                WorldBinding::new(9, 0, 0),
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
                WorldBinding::new(1, 0, 0),
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
    }
}
