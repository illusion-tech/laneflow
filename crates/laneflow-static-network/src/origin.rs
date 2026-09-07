use laneflow_static_contract::{ExactByteLength, NetworkRevisionId, Sha256Digest};

/// v1 分区规划提示的非语义派生版本。
pub const PARTITION_PLANNING_HINTS_DERIVATION_VERSION: u16 = 1;

/// 共享修订恢复与消费所需的静态契约版本。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StaticContractVersions {
    canonical_format_version: u16,
    identity_encoding_version: u16,
    identity_registry_revision: u16,
    network_revision_derivation_version: u16,
    constraint_contract_version: u16,
    static_execution_contract_version: u16,
}

impl StaticContractVersions {
    /// 组装静态契约版本集合；仅供 builder 在核对受检规范路网输入后调用。
    pub(crate) const fn new(
        canonical_format_version: u16,
        identity_encoding_version: u16,
        identity_registry_revision: u16,
        network_revision_derivation_version: u16,
        constraint_contract_version: u16,
        static_execution_contract_version: u16,
    ) -> Self {
        Self {
            canonical_format_version,
            identity_encoding_version,
            identity_registry_revision,
            network_revision_derivation_version,
            constraint_contract_version,
            static_execution_contract_version,
        }
    }

    /// 返回规范制品（LFCA）格式版本。
    #[must_use]
    pub const fn canonical_format_version(self) -> u16 {
        self.canonical_format_version
    }

    /// 返回标识编码版本。
    #[must_use]
    pub const fn identity_encoding_version(self) -> u16 {
        self.identity_encoding_version
    }

    /// 返回标识注册表修订。
    #[must_use]
    pub const fn identity_registry_revision(self) -> u16 {
        self.identity_registry_revision
    }

    /// 返回路网修订派生版本。
    #[must_use]
    pub const fn network_revision_derivation_version(self) -> u16 {
        self.network_revision_derivation_version
    }

    /// 返回约束契约版本。
    #[must_use]
    pub const fn constraint_contract_version(self) -> u16 {
        self.constraint_contract_version
    }

    /// 返回静态执行契约版本。
    #[must_use]
    pub const fn static_execution_contract_version(self) -> u16 {
        self.static_execution_contract_version
    }
}

/// 共享根对其 LFCA 来源与静态契约的只读进程内绑定。
///
/// 该值不是可序列化 descriptor，也不建立发布真实性。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CanonicalNetworkOrigin {
    canonical_artifact_digest: Sha256Digest,
    canonical_artifact_byte_length: ExactByteLength,
    network_revision: NetworkRevisionId,
    contracts: StaticContractVersions,
    partition_planning_hints_derivation_version: u16,
}

impl CanonicalNetworkOrigin {
    /// 组装来源绑定；分区规划提示派生版本取当前常量。
    pub(crate) const fn new(
        canonical_artifact_digest: Sha256Digest,
        canonical_artifact_byte_length: ExactByteLength,
        network_revision: NetworkRevisionId,
        contracts: StaticContractVersions,
    ) -> Self {
        Self {
            canonical_artifact_digest,
            canonical_artifact_byte_length,
            network_revision,
            contracts,
            partition_planning_hints_derivation_version:
                PARTITION_PLANNING_HINTS_DERIVATION_VERSION,
        }
    }

    /// 返回 LFCA 制品的 SHA-256 摘要。
    #[must_use]
    pub const fn canonical_artifact_digest(self) -> Sha256Digest {
        self.canonical_artifact_digest
    }

    /// 返回 LFCA 制品的精确字节长度。
    #[must_use]
    pub const fn canonical_artifact_byte_length(self) -> ExactByteLength {
        self.canonical_artifact_byte_length
    }

    /// 返回路网修订标识。
    #[must_use]
    pub const fn network_revision(self) -> NetworkRevisionId {
        self.network_revision
    }

    /// 返回静态契约版本集合。
    #[must_use]
    pub const fn static_contract_versions(self) -> StaticContractVersions {
        self.contracts
    }

    /// 返回分区规划提示派生版本。
    #[must_use]
    pub const fn partition_planning_hints_derivation_version(self) -> u16 {
        self.partition_planning_hints_derivation_version
    }
}
