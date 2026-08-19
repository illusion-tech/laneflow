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

    #[must_use]
    pub const fn canonical_format_version(self) -> u16 {
        self.canonical_format_version
    }

    #[must_use]
    pub const fn identity_encoding_version(self) -> u16 {
        self.identity_encoding_version
    }

    #[must_use]
    pub const fn identity_registry_revision(self) -> u16 {
        self.identity_registry_revision
    }

    #[must_use]
    pub const fn network_revision_derivation_version(self) -> u16 {
        self.network_revision_derivation_version
    }

    #[must_use]
    pub const fn constraint_contract_version(self) -> u16 {
        self.constraint_contract_version
    }

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

    #[must_use]
    pub const fn canonical_artifact_digest(self) -> Sha256Digest {
        self.canonical_artifact_digest
    }

    #[must_use]
    pub const fn canonical_artifact_byte_length(self) -> ExactByteLength {
        self.canonical_artifact_byte_length
    }

    #[must_use]
    pub const fn network_revision(self) -> NetworkRevisionId {
        self.network_revision
    }

    #[must_use]
    pub const fn static_contract_versions(self) -> StaticContractVersions {
        self.contracts
    }

    #[must_use]
    pub const fn partition_planning_hints_derivation_version(self) -> u16 {
        self.partition_planning_hints_derivation_version
    }
}
