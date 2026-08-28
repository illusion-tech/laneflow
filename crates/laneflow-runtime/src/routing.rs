//! 宿主 Routing 与 Traffic Runtime 的纯契约边界（#303 G2）。
//!
//! Runtime 不接收、保存或解释动态成本 payload。它只验证候选复制的绑定，
//! 将 LaneEdge 稳定标识解析为当前根 ordinal，并进入唯一的路线注册/编译路径。

use laneflow_static_contract::{
    LaneEdgeId, LaneEdgeOrdinal, NetworkRevisionId, Sha256Digest, StableId128,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    CommittedTrafficObservationBatch, ObservationStateSequence, ObservationStreamBinding,
    RouteError, RouteHandle, TrafficWorld, WorldGeneration,
};

/// 动态成本快照绑定的封闭版本。
pub const DYNAMIC_COST_BINDING_VERSION: u16 = 1;

/// 观测输入集合摘要的域分隔前缀；末尾 NUL 是摘要输入的一部分。
const OBSERVATION_SET_DIGEST_DOMAIN: &[u8] = b"laneflow:runtime-observation-set:v1\0";

/// 宿主拥有的不透明成本模型身份与封闭版本。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CostModelKey {
    model_id: Sha256Digest,
    model_version: u32,
}

impl CostModelKey {
    #[must_use]
    pub const fn new(model_id: Sha256Digest, model_version: u32) -> Self {
        Self {
            model_id,
            model_version,
        }
    }

    #[must_use]
    pub const fn model_id(self) -> Sha256Digest {
        self.model_id
    }

    #[must_use]
    pub const fn model_version(self) -> u32 {
        self.model_version
    }
}

/// 一组形成同一动态成本快照的已提交观测输入绑定。
///
/// 字段私有且只能由 [`bind_observation_set`] 从实际成功批次构造，因此不同
/// stream、修订、tick 或状态序号的分区不能被静默拼接。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationSetBinding {
    stream: ObservationStreamBinding,
    network_revision: NetworkRevisionId,
    network_revision_derivation_version: u16,
    observation_tick: u64,
    observation_state_sequence: ObservationStateSequence,
    input_count: u64,
    digest: Sha256Digest,
}

impl ObservationSetBinding {
    #[must_use]
    pub const fn stream_binding(self) -> ObservationStreamBinding {
        self.stream
    }

    #[must_use]
    pub const fn network_revision(self) -> NetworkRevisionId {
        self.network_revision
    }

    #[must_use]
    pub const fn network_revision_derivation_version(self) -> u16 {
        self.network_revision_derivation_version
    }

    #[must_use]
    pub const fn observation_tick(self) -> u64 {
        self.observation_tick
    }

    #[must_use]
    pub const fn observation_state_sequence(self) -> ObservationStateSequence {
        self.observation_state_sequence
    }

    #[must_use]
    pub const fn input_count(self) -> u64 {
        self.input_count
    }

    #[must_use]
    pub const fn digest(self) -> Sha256Digest {
        self.digest
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ObservationSetRecord {
    stream: ObservationStreamBinding,
    delivery_sequence: u64,
    selection_digest: Sha256Digest,
}

impl ObservationSetRecord {
    fn canonical_key(self) -> (u64, u64, u64, Sha256Digest) {
        (
            self.stream.world_id(),
            self.stream.world_generation().get(),
            self.delivery_sequence,
            self.selection_digest,
        )
    }
}

/// 观测输入集合构造失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum ObservationSetError {
    #[error("观测输入集合不能为空")]
    Empty,
    #[error("观测输入集合含不同世界或活动世代")]
    StreamMismatch,
    #[error("观测输入集合含不同路网修订或派生版本")]
    NetworkRevisionMismatch,
    #[error("观测输入集合含不同已提交 tick")]
    TickMismatch,
    #[error("观测输入集合含不同观测状态序号")]
    StateSequenceMismatch,
    #[error("观测输入集合含重复 stream/delivery/selection 绑定")]
    DuplicateInput,
    #[error("观测输入集合计数或缓冲分配失败")]
    ResourceFailure,
}

/// 把一组实际成功观测批次规范化并摘要。
///
/// 摘要输入为域前缀、`inputCount:u64-le`，随后按
/// `(worldId, worldGeneration, deliverySequence, selectionDigest)` 升序逐项写入
/// `worldId:u64-le || worldGeneration:u64-le || deliverySequence:u64-le ||
/// selectionDigest:32-bytes`。
pub fn bind_observation_set(
    batches: &[&CommittedTrafficObservationBatch],
) -> Result<ObservationSetBinding, ObservationSetError> {
    let first = *batches.first().ok_or(ObservationSetError::Empty)?;
    let stream = first.stream_binding();
    let network_revision = first.network_revision();
    let network_revision_derivation_version = first.network_revision_derivation_version();
    let observation_tick = first.tick();
    let observation_state_sequence = first.observation_state_sequence();

    let mut records = Vec::new();
    records
        .try_reserve_exact(batches.len())
        .map_err(|_| ObservationSetError::ResourceFailure)?;
    for batch in batches {
        if batch.stream_binding() != stream {
            return Err(ObservationSetError::StreamMismatch);
        }
        if batch.network_revision() != network_revision
            || batch.network_revision_derivation_version() != network_revision_derivation_version
        {
            return Err(ObservationSetError::NetworkRevisionMismatch);
        }
        if batch.tick() != observation_tick {
            return Err(ObservationSetError::TickMismatch);
        }
        if batch.observation_state_sequence() != observation_state_sequence {
            return Err(ObservationSetError::StateSequenceMismatch);
        }
        records.push(ObservationSetRecord {
            stream: batch.stream_binding(),
            delivery_sequence: batch.sequence(),
            selection_digest: batch.selection_digest(),
        });
    }
    records.sort_unstable_by_key(|record| record.canonical_key());
    if records
        .windows(2)
        .any(|pair| pair[0].canonical_key() == pair[1].canonical_key())
    {
        return Err(ObservationSetError::DuplicateInput);
    }

    let input_count =
        u64::try_from(records.len()).map_err(|_| ObservationSetError::ResourceFailure)?;
    let mut hasher = Sha256::new();
    hasher.update(OBSERVATION_SET_DIGEST_DOMAIN);
    hasher.update(input_count.to_le_bytes());
    for record in records {
        hasher.update(record.stream.world_id().to_le_bytes());
        hasher.update(record.stream.world_generation().get().to_le_bytes());
        hasher.update(record.delivery_sequence.to_le_bytes());
        hasher.update(record.selection_digest.as_bytes());
    }

    Ok(ObservationSetBinding {
        stream,
        network_revision,
        network_revision_derivation_version,
        observation_tick,
        observation_state_sequence,
        input_count,
        digest: Sha256Digest::from_bytes(hasher.finalize().into()),
    })
}

/// 候选复制的动态成本快照来源绑定；不包含或拥有成本 payload。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DynamicCostSnapshotBinding {
    binding_version: u16,
    observation_set: ObservationSetBinding,
    cost_model: CostModelKey,
    valid_through_tick: u64,
    entry_count: u64,
    exact_byte_length: u64,
    snapshot_sha256: Sha256Digest,
}

impl DynamicCostSnapshotBinding {
    /// 从宿主 receiver 已验证的计数、exact bytes 与摘要构造绑定。
    /// Runtime 不重复接收或解释 payload。
    pub fn new(
        observation_set: ObservationSetBinding,
        cost_model: CostModelKey,
        valid_through_tick: u64,
        entry_count: u64,
        exact_byte_length: u64,
        snapshot_sha256: Sha256Digest,
    ) -> Result<Self, DynamicCostBindingError> {
        if valid_through_tick < observation_set.observation_tick() {
            return Err(DynamicCostBindingError::InvalidValidityWindow);
        }
        Ok(Self {
            binding_version: DYNAMIC_COST_BINDING_VERSION,
            observation_set,
            cost_model,
            valid_through_tick,
            entry_count,
            exact_byte_length,
            snapshot_sha256,
        })
    }

    #[must_use]
    pub const fn binding_version(self) -> u16 {
        self.binding_version
    }

    #[must_use]
    pub const fn observation_set(self) -> ObservationSetBinding {
        self.observation_set
    }

    #[must_use]
    pub const fn cost_model(self) -> CostModelKey {
        self.cost_model
    }

    #[must_use]
    pub const fn valid_through_tick(self) -> u64 {
        self.valid_through_tick
    }

    #[must_use]
    pub const fn entry_count(self) -> u64 {
        self.entry_count
    }

    /// 宿主成本 payload 的精确字节数。
    #[must_use]
    pub const fn exact_byte_length(self) -> u64 {
        self.exact_byte_length
    }

    #[must_use]
    pub const fn snapshot_sha256(self) -> Sha256Digest {
        self.snapshot_sha256
    }
}

/// 动态成本绑定构造失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum DynamicCostBindingError {
    #[error("动态成本有效窗末端早于观测 tick")]
    InvalidValidityWindow,
}

/// Runtime 签发、调用方持有的候选路线准入能力。
#[derive(Debug)]
pub struct RoutingAdmissionSession {
    world_id: u64,
    world_generation: WorldGeneration,
    network_revision: NetworkRevisionId,
    network_revision_derivation_version: u16,
    cost_model: CostModelKey,
}

impl RoutingAdmissionSession {
    #[must_use]
    pub const fn world_id(&self) -> u64 {
        self.world_id
    }

    #[must_use]
    pub const fn world_generation(&self) -> WorldGeneration {
        self.world_generation
    }

    #[must_use]
    pub const fn network_revision(&self) -> NetworkRevisionId {
        self.network_revision
    }

    #[must_use]
    pub const fn network_revision_derivation_version(&self) -> u16 {
        self.network_revision_derivation_version
    }

    #[must_use]
    pub const fn cost_model(&self) -> CostModelKey {
        self.cost_model
    }
}

/// Routing 提交的候选路线；稳定标识在准入时解析到当前共享根。
#[derive(Debug, Eq, PartialEq)]
pub struct CandidateRouteInput {
    cost_snapshot: DynamicCostSnapshotBinding,
    lane_edges: Box<[StableId128]>,
}

impl CandidateRouteInput {
    #[must_use]
    pub fn new(
        cost_snapshot: DynamicCostSnapshotBinding,
        lane_edges: impl Into<Vec<StableId128>>,
    ) -> Self {
        Self {
            cost_snapshot,
            lane_edges: lane_edges.into().into_boxed_slice(),
        }
    }

    #[must_use]
    pub const fn cost_snapshot(&self) -> DynamicCostSnapshotBinding {
        self.cost_snapshot
    }

    #[must_use]
    pub const fn lane_edges(&self) -> &[StableId128] {
        &self.lane_edges
    }
}

/// 候选路线准入失败；任一失败都不占路线槽或 occurrence。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum CandidateRouteError {
    #[error(transparent)]
    Route(#[from] RouteError),
    #[error("动态成本绑定版本未知: {actual}")]
    DynamicCostBindingVersionMismatch { actual: u16 },
    #[error("Routing admission session 的世界身份或世代已失效")]
    AdmissionSessionMismatch,
    #[error("动态成本绑定的世界身份或世代已失效")]
    CostWorldBindingMismatch,
    #[error("Routing admission session 的路网修订已失效")]
    AdmissionRevisionMismatch,
    #[error("动态成本绑定的路网修订已失效")]
    CostRevisionMismatch,
    #[error("动态成本模型与 Routing admission session 不一致")]
    CostModelMismatch,
    #[error("动态成本有效窗末端早于观测 tick")]
    InvalidValidityWindow,
    #[error("动态成本观测 tick 来自未来")]
    FutureObservationTick,
    #[error("动态成本已过期")]
    StaleDynamicCost,
    #[error("动态成本观测状态序号来自未来或损坏来源")]
    FutureObservationStateSequence,
    #[error("候选含当前修订未知或错误 kind 的 LaneEdge StableId128: {stable_id:?}")]
    UnknownLaneEdge { stable_id: StableId128 },
}

/// 回放/恢复使用的规范化已准入路线注册输入。
///
/// 宿主耐久路线 ID 由宿主命令序列拥有；Runtime 只消费修订绑定与稳定边序列，
/// 并返回本次世界中的新 [`RouteHandle`] 供宿主回映。
#[derive(Debug, Eq, PartialEq)]
pub struct AdmittedRouteRegisterInput {
    network_revision: NetworkRevisionId,
    network_revision_derivation_version: u16,
    lane_edges: Box<[StableId128]>,
}

impl AdmittedRouteRegisterInput {
    #[must_use]
    pub fn new(
        network_revision: NetworkRevisionId,
        network_revision_derivation_version: u16,
        lane_edges: impl Into<Vec<StableId128>>,
    ) -> Self {
        Self {
            network_revision,
            network_revision_derivation_version,
            lane_edges: lane_edges.into().into_boxed_slice(),
        }
    }

    #[must_use]
    pub const fn network_revision(&self) -> NetworkRevisionId {
        self.network_revision
    }

    #[must_use]
    pub const fn network_revision_derivation_version(&self) -> u16 {
        self.network_revision_derivation_version
    }

    #[must_use]
    pub const fn lane_edges(&self) -> &[StableId128] {
        &self.lane_edges
    }
}

/// 规范化已准入路线注册失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum AdmittedRouteRegisterError {
    #[error(transparent)]
    Route(#[from] RouteError),
    #[error("已准入路线的路网修订绑定与当前根不一致")]
    NetworkRevisionMismatch,
    #[error("已准入路线含当前修订未知或错误 kind 的 LaneEdge StableId128: {stable_id:?}")]
    UnknownLaneEdge { stable_id: StableId128 },
}

enum StableLaneEdgeResolveError {
    AllocationFailed,
    Unknown(StableId128),
}

impl TrafficWorld {
    /// 打开当前世界/世代/修订和精确成本模型绑定的 Routing admission session。
    /// 该操作无分配、无隐式 Runtime 状态，因此当前 G2 API 是不可失败的。
    #[must_use]
    pub fn open_routing_admission(&self, cost_model: CostModelKey) -> RoutingAdmissionSession {
        RoutingAdmissionSession {
            world_id: self.world_id,
            world_generation: self.world_generation,
            network_revision: self.revision.network_revision(),
            network_revision_derivation_version: self
                .revision
                .canonical_origin()
                .static_contract_versions()
                .network_revision_derivation_version(),
            cost_model,
        }
    }

    /// 验证动态成本来源与候选稳定引用，并注册为普通本世界路线。
    pub fn register_candidate_route(
        &mut self,
        admission: &RoutingAdmissionSession,
        input: CandidateRouteInput,
    ) -> Result<RouteHandle, CandidateRouteError> {
        self.preflight_route_registration(input.lane_edges.len())?;
        let cost = input.cost_snapshot;
        if cost.binding_version != DYNAMIC_COST_BINDING_VERSION {
            return Err(CandidateRouteError::DynamicCostBindingVersionMismatch {
                actual: cost.binding_version,
            });
        }
        let current_derivation = self
            .revision
            .canonical_origin()
            .static_contract_versions()
            .network_revision_derivation_version();
        if admission.world_id != self.world_id
            || admission.world_generation != self.world_generation
        {
            return Err(CandidateRouteError::AdmissionSessionMismatch);
        }
        if cost.observation_set.stream.world_id() != self.world_id
            || cost.observation_set.stream.world_generation() != self.world_generation
        {
            return Err(CandidateRouteError::CostWorldBindingMismatch);
        }
        if admission.network_revision != self.revision.network_revision()
            || admission.network_revision_derivation_version != current_derivation
        {
            return Err(CandidateRouteError::AdmissionRevisionMismatch);
        }
        if cost.observation_set.network_revision != self.revision.network_revision()
            || cost.observation_set.network_revision_derivation_version != current_derivation
        {
            return Err(CandidateRouteError::CostRevisionMismatch);
        }
        if cost.cost_model != admission.cost_model {
            return Err(CandidateRouteError::CostModelMismatch);
        }
        if cost.valid_through_tick < cost.observation_set.observation_tick {
            return Err(CandidateRouteError::InvalidValidityWindow);
        }
        if self.tick_index < cost.observation_set.observation_tick {
            return Err(CandidateRouteError::FutureObservationTick);
        }
        if self.tick_index > cost.valid_through_tick {
            return Err(CandidateRouteError::StaleDynamicCost);
        }
        if self.observation_state_sequence.get()
            < cost.observation_set.observation_state_sequence.get()
        {
            return Err(CandidateRouteError::FutureObservationStateSequence);
        }

        let resolved = self
            .resolve_stable_lane_edges(&input.lane_edges)
            .map_err(|error| match error {
                StableLaneEdgeResolveError::AllocationFailed => {
                    CandidateRouteError::Route(RouteError::AllocationFailed)
                }
                StableLaneEdgeResolveError::Unknown(stable_id) => {
                    CandidateRouteError::UnknownLaneEdge { stable_id }
                }
            })?;
        self.register_route_edges(&resolved).map_err(Into::into)
    }

    /// 重放/恢复规范化的已准入路线命令；不调用 Routing、不接收旧成本绑定。
    pub fn register_admitted_route(
        &mut self,
        input: AdmittedRouteRegisterInput,
    ) -> Result<RouteHandle, AdmittedRouteRegisterError> {
        let current_derivation = self
            .revision
            .canonical_origin()
            .static_contract_versions()
            .network_revision_derivation_version();
        if input.network_revision != self.revision.network_revision()
            || input.network_revision_derivation_version != current_derivation
        {
            return Err(AdmittedRouteRegisterError::NetworkRevisionMismatch);
        }
        self.preflight_route_registration(input.lane_edges.len())?;
        let resolved = self
            .resolve_stable_lane_edges(&input.lane_edges)
            .map_err(|error| match error {
                StableLaneEdgeResolveError::AllocationFailed => {
                    AdmittedRouteRegisterError::Route(RouteError::AllocationFailed)
                }
                StableLaneEdgeResolveError::Unknown(stable_id) => {
                    AdmittedRouteRegisterError::UnknownLaneEdge { stable_id }
                }
            })?;
        self.register_route_edges(&resolved).map_err(Into::into)
    }

    fn resolve_stable_lane_edges(
        &self,
        stable_ids: &[StableId128],
    ) -> Result<Vec<LaneEdgeOrdinal>, StableLaneEdgeResolveError> {
        let mut resolved = Vec::new();
        resolved
            .try_reserve_exact(stable_ids.len())
            .map_err(|_| StableLaneEdgeResolveError::AllocationFailed)?;
        for stable_id in stable_ids {
            let typed = LaneEdgeId::from_untyped(*stable_id);
            let ordinal = self
                .revision
                .identity()
                .ordinal(typed)
                .ok_or(StableLaneEdgeResolveError::Unknown(*stable_id))?;
            resolved.push(ordinal);
        }
        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_static_contract::{
        LaneEdgeOrdinal, RoadSectionOrdinal, Sha256Digest, StableId128, VehicleProfileOrdinal,
    };
    use laneflow_static_network::{
        CanonicalNetworkOrigin, SharedNetworkBuildLimits, SharedNetworkBuildOptions,
        SharedNetworkRevision, SpatialBuildOption, build_shared_network_revision,
    };

    use super::*;
    use crate::tables::with_route_allocation_failure_after;
    use crate::{
        CommittedNetworkSource, ObservationExportMode, ObservationSelection,
        PublishedLfcaReference, RouteRegisterInput, TickInput, VehicleSpawnInput, WorldConfig,
    };

    const DYNAMIC_COST_SNAPSHOT_DIGEST_DOMAIN: &[u8] = b"laneflow:dynamic-cost-snapshot:v1\0";
    const FIXTURE_COST_ENTRY_BYTES: usize = 8;

    #[derive(Clone, Copy)]
    struct CostReceiverLimits {
        max_entry_count: u64,
        max_exact_byte_length: u64,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum CostReceiverError {
        UnknownBindingVersion,
        ByteLimitExceeded,
        EntryLimitExceeded,
        ExactByteLengthMismatch,
        MalformedPayload,
        EntryCountMismatch,
        SnapshotDigestMismatch,
    }

    fn exact_payload_byte_length(payload: &[u8]) -> Result<u64, CostReceiverError> {
        u64::try_from(payload.len()).map_err(|_| CostReceiverError::ByteLimitExceeded)
    }

    /// #303 只要求 contract/performance fixture；这不是产品成本格式或 Routing API。
    fn receive_fixture_cost_snapshot(
        binding: DynamicCostSnapshotBinding,
        payload: &[u8],
        limits: CostReceiverLimits,
    ) -> Result<DynamicCostSnapshotBinding, CostReceiverError> {
        if binding.binding_version != DYNAMIC_COST_BINDING_VERSION {
            return Err(CostReceiverError::UnknownBindingVersion);
        }
        let actual_bytes = exact_payload_byte_length(payload)?;
        if binding.exact_byte_length > limits.max_exact_byte_length
            || actual_bytes > limits.max_exact_byte_length
        {
            return Err(CostReceiverError::ByteLimitExceeded);
        }
        if binding.entry_count > limits.max_entry_count {
            return Err(CostReceiverError::EntryLimitExceeded);
        }
        if binding.exact_byte_length != actual_bytes {
            return Err(CostReceiverError::ExactByteLengthMismatch);
        }
        if !payload.len().is_multiple_of(FIXTURE_COST_ENTRY_BYTES) {
            return Err(CostReceiverError::MalformedPayload);
        }
        let actual_entries = u64::try_from(payload.len() / FIXTURE_COST_ENTRY_BYTES)
            .map_err(|_| CostReceiverError::EntryLimitExceeded)?;
        if binding.entry_count != actual_entries {
            return Err(CostReceiverError::EntryCountMismatch);
        }
        if binding.snapshot_sha256 != dynamic_cost_snapshot_digest(binding, payload) {
            return Err(CostReceiverError::SnapshotDigestMismatch);
        }
        Ok(binding)
    }

    fn dynamic_cost_snapshot_digest(
        binding: DynamicCostSnapshotBinding,
        payload: &[u8],
    ) -> Sha256Digest {
        let observations = binding.observation_set;
        let stream = observations.stream;
        let mut hasher = Sha256::new();
        hasher.update(DYNAMIC_COST_SNAPSHOT_DIGEST_DOMAIN);
        hasher.update(binding.binding_version.to_le_bytes());
        hasher.update(stream.world_id().to_le_bytes());
        hasher.update(stream.world_generation().get().to_le_bytes());
        hasher.update(observations.network_revision.as_digest().as_bytes());
        hasher.update(
            observations
                .network_revision_derivation_version
                .to_le_bytes(),
        );
        hasher.update(observations.observation_tick.to_le_bytes());
        hasher.update(observations.observation_state_sequence.get().to_le_bytes());
        hasher.update(observations.digest.as_bytes());
        hasher.update(binding.cost_model.model_id.as_bytes());
        hasher.update(binding.cost_model.model_version.to_le_bytes());
        hasher.update(binding.valid_through_tick.to_le_bytes());
        hasher.update(binding.entry_count.to_le_bytes());
        hasher.update(binding.exact_byte_length.to_le_bytes());
        hasher.update(payload);
        Sha256Digest::from_bytes(hasher.finalize().into())
    }

    fn revision() -> Arc<SharedNetworkRevision> {
        let input = check_canonical_network_input(
            include_bytes!(
                "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
            ),
            FormatLimits::HARD,
        )
        .expect("checked LFCA");
        build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("shared revision")
    }

    fn source_for(origin: CanonicalNetworkOrigin, key: &str) -> CommittedNetworkSource {
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                key,
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("non-empty source key"),
        }
    }

    fn world_with_limits(
        world_id: u64,
        route_capacity: u32,
        occurrence_capacity: u64,
    ) -> TrafficWorld {
        let revision = revision();
        let origin = *revision.canonical_origin();
        TrafficWorld::install(
            revision,
            WorldConfig::new(16, route_capacity, occurrence_capacity, 1, 100),
            source_for(origin, "fixture://routing"),
            world_id,
        )
        .expect("install")
    }

    fn edge_for_length(world: &TrafficWorld, length: u32) -> LaneEdgeOrdinal {
        let index = world
            .traffic()
            .lane_lengths_millimetres()
            .iter()
            .position(|actual| *actual == length)
            .expect("fixture LaneEdge length");
        LaneEdgeOrdinal::try_from_usize(index).expect("fixture LaneEdge ordinal")
    }

    fn fixture_edges(world: &TrafficWorld) -> Vec<LaneEdgeOrdinal> {
        vec![
            edge_for_length(world, 10_000),
            edge_for_length(world, 8_000),
            edge_for_length(world, 12_000),
        ]
    }

    fn stable_edges(world: &TrafficWorld, edges: &[LaneEdgeOrdinal]) -> Vec<StableId128> {
        edges
            .iter()
            .map(|edge| {
                world
                    .revision
                    .identity()
                    .stable_id(*edge)
                    .expect("LaneEdge stable id")
                    .into_untyped()
            })
            .collect()
    }

    fn model(seed: u8, version: u32) -> CostModelKey {
        CostModelKey::new(Sha256Digest::from_bytes([seed; 32]), version)
    }

    fn full_observation(world: &TrafficWorld) -> CommittedTrafficObservationBatch {
        let mut session = world
            .open_observation_export(ObservationSelection::AllLaneEdges)
            .expect("open observation");
        world
            .export_observation(&mut session, ObservationExportMode::Full)
            .expect("full observation")
    }

    fn fixture_cost_binding(
        batches: &[&CommittedTrafficObservationBatch],
        cost_model: CostModelKey,
        valid_through_tick: u64,
        payload: &[u8],
    ) -> DynamicCostSnapshotBinding {
        let observation_set = bind_observation_set(batches).expect("bind observation set");
        let mut binding = DynamicCostSnapshotBinding::new(
            observation_set,
            cost_model,
            valid_through_tick,
            u64::try_from(payload.len() / FIXTURE_COST_ENTRY_BYTES).expect("fixture entry count"),
            exact_payload_byte_length(payload).expect("fixture bytes"),
            Sha256Digest::ZERO,
        )
        .expect("valid cost binding");
        binding.snapshot_sha256 = dynamic_cost_snapshot_digest(binding, payload);
        receive_fixture_cost_snapshot(
            binding,
            payload,
            CostReceiverLimits {
                max_entry_count: 1_024,
                max_exact_byte_length: 64 * 1_024,
            },
        )
        .expect("receive fixture cost")
    }

    #[test]
    fn observation_set_digest_is_order_independent_and_rejects_mixed_state() {
        let mut world = world_with_limits(41, 8, 1_024);
        let mut lane_ids: Vec<_> = (0..world.traffic().lane_edge_count())
            .map(|raw| {
                world
                    .revision
                    .identity()
                    .stable_id(LaneEdgeOrdinal::from_raw(raw))
                    .expect("LaneEdge stable id")
            })
            .collect();
        lane_ids.sort_unstable();
        let split = lane_ids.len() / 2;
        let mut first_session = world
            .open_observation_export(ObservationSelection::ExplicitLaneEdges(
                lane_ids[..split].to_vec().into_boxed_slice(),
            ))
            .expect("first partition");
        let mut second_session = world
            .open_observation_export(ObservationSelection::ExplicitLaneEdges(
                lane_ids[split..].to_vec().into_boxed_slice(),
            ))
            .expect("second partition");
        let first = world
            .export_observation(&mut first_session, ObservationExportMode::Full)
            .expect("first batch");
        let second = world
            .export_observation(&mut second_session, ObservationExportMode::Full)
            .expect("second batch");
        let forward = bind_observation_set(&[&first, &second]).expect("forward set");
        let reverse = bind_observation_set(&[&second, &first]).expect("reverse set");
        assert_eq!(forward, reverse);
        assert_eq!(forward.input_count(), 2);
        assert_eq!(
            bind_observation_set(&[&first, &first]).unwrap_err(),
            ObservationSetError::DuplicateInput
        );

        world.step(TickInput::new(100)).expect("step");
        let newer = world
            .export_observation(&mut first_session, ObservationExportMode::Full)
            .expect("newer batch");
        assert_eq!(
            bind_observation_set(&[&newer, &second]).unwrap_err(),
            ObservationSetError::TickMismatch
        );

        let mut same_tick_world = world_with_limits(42, 8, 1_024);
        let same_tick_batch = full_observation(&same_tick_world);
        let route = same_tick_world
            .register_route(RouteRegisterInput::new(fixture_edges(&same_tick_world)))
            .expect("same-tick route");
        same_tick_world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                0,
                0,
            ))
            .expect("same-tick spawn");
        let same_tick_newer_state = full_observation(&same_tick_world);
        assert_eq!(
            bind_observation_set(&[&same_tick_newer_state, &same_tick_batch]).unwrap_err(),
            ObservationSetError::StateSequenceMismatch
        );
    }

    #[test]
    fn receiver_fixture_checks_limits_before_shape_and_validates_exact_fields() {
        let world = world_with_limits(41, 8, 1_024);
        let batch = full_observation(&world);
        let payload = [7_u8; 16];
        let binding = fixture_cost_binding(&[&batch], model(1, 3), 0, &payload);
        let limits = CostReceiverLimits {
            max_entry_count: 2,
            max_exact_byte_length: 16,
        };
        assert_eq!(
            receive_fixture_cost_snapshot(binding, &payload, limits),
            Ok(binding)
        );

        let mut unknown_version = binding;
        unknown_version.binding_version = u16::MAX;
        assert_eq!(
            receive_fixture_cost_snapshot(unknown_version, &payload, limits).unwrap_err(),
            CostReceiverError::UnknownBindingVersion
        );
        let oversized_malformed = [0_u8; 17];
        assert_eq!(
            receive_fixture_cost_snapshot(binding, &oversized_malformed, limits).unwrap_err(),
            CostReceiverError::ByteLimitExceeded
        );
        let mut wrong_length = binding;
        wrong_length.exact_byte_length = 8;
        assert_eq!(
            receive_fixture_cost_snapshot(wrong_length, &payload, limits).unwrap_err(),
            CostReceiverError::ExactByteLengthMismatch
        );
        let malformed = [0_u8; 15];
        let mut malformed_binding = binding;
        malformed_binding.exact_byte_length = 15;
        assert_eq!(
            receive_fixture_cost_snapshot(malformed_binding, &malformed, limits).unwrap_err(),
            CostReceiverError::MalformedPayload
        );
        let mut wrong_count = binding;
        wrong_count.entry_count = 1;
        assert_eq!(
            receive_fixture_cost_snapshot(wrong_count, &payload, limits).unwrap_err(),
            CostReceiverError::EntryCountMismatch
        );
        let mut too_many_entries = binding;
        too_many_entries.entry_count = 3;
        assert_eq!(
            receive_fixture_cost_snapshot(too_many_entries, &payload, limits).unwrap_err(),
            CostReceiverError::EntryLimitExceeded
        );
        let mut wrong_digest = binding;
        wrong_digest.snapshot_sha256 = Sha256Digest::ZERO;
        assert_eq!(
            receive_fixture_cost_snapshot(wrong_digest, &payload, limits).unwrap_err(),
            CostReceiverError::SnapshotDigestMismatch
        );
    }

    #[test]
    fn candidate_registration_uses_stable_ids_and_the_unique_route_compiler() {
        let mut world = world_with_limits(41, 8, 1_024);
        let edges = fixture_edges(&world);
        let stable = stable_edges(&world, &edges);
        let batch = full_observation(&world);
        let cost_model = model(2, 1);
        let cost = fixture_cost_binding(&[&batch], cost_model, 0, &[9_u8; 24]);
        let admission = world.open_routing_admission(cost_model);

        let route = world
            .register_candidate_route(&admission, CandidateRouteInput::new(cost, stable))
            .expect("candidate route");
        assert_eq!(world.route_edges(route), Some(edges.as_slice()));
        assert_eq!(world.live_route_edge_occurrence_count, 3);

        world.step(TickInput::new(100)).expect("expire cost");
        assert_eq!(world.route_edges(route), Some(edges.as_slice()));
    }

    #[test]
    fn candidate_binding_time_identity_model_and_topology_fail_atomically() {
        let mut world = world_with_limits(41, 8, 1_024);
        let edges = fixture_edges(&world);
        let stable = stable_edges(&world, &edges);
        let batch = full_observation(&world);
        let cost_model = model(3, 7);
        let cost = fixture_cost_binding(&[&batch], cost_model, 0, &[4_u8; 8]);
        let admission = world.open_routing_admission(cost_model);

        assert_eq!(
            world
                .register_candidate_route(&admission, CandidateRouteInput::new(cost, Vec::new()))
                .unwrap_err(),
            CandidateRouteError::Route(RouteError::EmptySequence)
        );
        let mut unknown_cost_version = cost;
        unknown_cost_version.binding_version = u16::MAX;
        assert_eq!(
            world
                .register_candidate_route(
                    &admission,
                    CandidateRouteInput::new(unknown_cost_version, stable.clone()),
                )
                .unwrap_err(),
            CandidateRouteError::DynamicCostBindingVersionMismatch { actual: u16::MAX }
        );
        let wrong_model = world.open_routing_admission(model(4, 7));
        assert_eq!(
            world
                .register_candidate_route(
                    &wrong_model,
                    CandidateRouteInput::new(cost, stable.clone())
                )
                .unwrap_err(),
            CandidateRouteError::CostModelMismatch
        );

        let mut wrong_admission_revision = world.open_routing_admission(cost_model);
        wrong_admission_revision.network_revision =
            NetworkRevisionId::from_digest(Sha256Digest::from_bytes([0xa1; 32]));
        assert_eq!(
            world
                .register_candidate_route(
                    &wrong_admission_revision,
                    CandidateRouteInput::new(cost, stable.clone()),
                )
                .unwrap_err(),
            CandidateRouteError::AdmissionRevisionMismatch
        );
        let mut wrong_cost_revision = cost;
        wrong_cost_revision.observation_set.network_revision =
            NetworkRevisionId::from_digest(Sha256Digest::from_bytes([0xa2; 32]));
        assert_eq!(
            world
                .register_candidate_route(
                    &admission,
                    CandidateRouteInput::new(wrong_cost_revision, stable.clone()),
                )
                .unwrap_err(),
            CandidateRouteError::CostRevisionMismatch
        );
        let mut invalid_window = cost;
        invalid_window.observation_set.observation_tick = 1;
        assert_eq!(
            world
                .register_candidate_route(
                    &admission,
                    CandidateRouteInput::new(invalid_window, stable.clone()),
                )
                .unwrap_err(),
            CandidateRouteError::InvalidValidityWindow
        );

        let mut future_tick = cost;
        future_tick.observation_set.observation_tick = 1;
        future_tick.valid_through_tick = 1;
        assert_eq!(
            world
                .register_candidate_route(
                    &admission,
                    CandidateRouteInput::new(future_tick, stable.clone())
                )
                .unwrap_err(),
            CandidateRouteError::FutureObservationTick
        );
        let mut future_state = cost;
        future_state.observation_set.observation_state_sequence =
            ObservationStateSequence::from_raw_for_test(1);
        assert_eq!(
            world
                .register_candidate_route(
                    &admission,
                    CandidateRouteInput::new(future_state, stable.clone())
                )
                .unwrap_err(),
            CandidateRouteError::FutureObservationStateSequence
        );

        let unknown = StableId128::from_bytes([0xff; 16]);
        assert_eq!(
            world
                .register_candidate_route(&admission, CandidateRouteInput::new(cost, vec![unknown]))
                .unwrap_err(),
            CandidateRouteError::UnknownLaneEdge { stable_id: unknown }
        );
        let wrong_kind = world
            .revision
            .identity()
            .stable_id(RoadSectionOrdinal::from_raw(0))
            .expect("RoadSection stable id")
            .into_untyped();
        assert_eq!(
            world
                .register_candidate_route(
                    &admission,
                    CandidateRouteInput::new(cost, vec![wrong_kind])
                )
                .unwrap_err(),
            CandidateRouteError::UnknownLaneEdge {
                stable_id: wrong_kind
            }
        );
        let disconnected = stable_edges(&world, &[edges[0], edges[2]]);
        assert_eq!(
            world
                .register_candidate_route(&admission, CandidateRouteInput::new(cost, disconnected))
                .unwrap_err(),
            CandidateRouteError::Route(RouteError::Disconnected)
        );
        assert_eq!(world.live_route_count, 0);
        assert_eq!(world.live_route_edge_occurrence_count, 0);

        world.step(TickInput::new(100)).expect("step");
        assert_eq!(
            world
                .register_candidate_route(
                    &admission,
                    CandidateRouteInput::new(cost, stable.clone())
                )
                .unwrap_err(),
            CandidateRouteError::StaleDynamicCost
        );
        world.world_generation = world
            .world_generation
            .checked_next()
            .expect("next generation");
        assert_eq!(
            world
                .register_candidate_route(
                    &admission,
                    CandidateRouteInput::new(cost, stable.clone()),
                )
                .unwrap_err(),
            CandidateRouteError::AdmissionSessionMismatch
        );
        let current_admission = world.open_routing_admission(cost_model);
        assert_eq!(
            world
                .register_candidate_route(
                    &current_admission,
                    CandidateRouteInput::new(cost, stable),
                )
                .unwrap_err(),
            CandidateRouteError::CostWorldBindingMismatch
        );
        assert_eq!(world.live_route_count, 0);
        assert_eq!(world.live_route_edge_occurrence_count, 0);
    }

    #[test]
    fn direct_candidate_and_admitted_replay_share_occurrence_capacity_and_atomicity() {
        let mut world = world_with_limits(41, 8, 3);
        let edges = fixture_edges(&world);
        let stable = stable_edges(&world, &edges);
        let origin = *world.revision.canonical_origin();
        let derivation = origin
            .static_contract_versions()
            .network_revision_derivation_version();
        let direct = world
            .register_route(RouteRegisterInput::new(edges.clone()))
            .expect("direct at max");
        assert_eq!(world.live_route_edge_occurrence_count, 3);
        assert_eq!(
            world
                .register_route(RouteRegisterInput::new(vec![edges[0]]))
                .unwrap_err(),
            RouteError::EdgeOccurrenceCapacityExceeded
        );
        world.remove_route(direct).expect("remove direct");

        let batch = full_observation(&world);
        let cost_model = model(5, 1);
        let cost = fixture_cost_binding(&[&batch], cost_model, 0, &[1_u8; 8]);
        let admission = world.open_routing_admission(cost_model);
        let mut max_plus_one = stable.clone();
        max_plus_one.push(stable[0]);
        assert_eq!(
            world
                .register_candidate_route(&admission, CandidateRouteInput::new(cost, max_plus_one))
                .unwrap_err(),
            CandidateRouteError::Route(RouteError::EdgeOccurrenceCapacityExceeded)
        );
        let candidate = world
            .register_candidate_route(&admission, CandidateRouteInput::new(cost, stable.clone()))
            .expect("candidate at max");
        assert_eq!(world.live_route_edge_occurrence_count, 3);
        world.remove_route(candidate).expect("remove candidate");

        let replay = world
            .register_admitted_route(AdmittedRouteRegisterInput::new(
                origin.network_revision(),
                derivation,
                stable.clone(),
            ))
            .expect("replay at max");
        assert_eq!(world.route_edges(replay), Some(edges.as_slice()));
        assert_eq!(world.live_route_edge_occurrence_count, 3);
        world.remove_route(replay).expect("remove replay");

        with_route_allocation_failure_after(0, || {
            assert_eq!(
                world
                    .register_candidate_route(
                        &admission,
                        CandidateRouteInput::new(cost, stable.clone()),
                    )
                    .unwrap_err(),
                CandidateRouteError::Route(RouteError::AllocationFailed)
            );
        });
        assert_eq!(world.live_route_count, 0);
        assert_eq!(world.live_route_edge_occurrence_count, 0);
    }

    #[test]
    fn admitted_replay_checks_revision_and_stable_identity_without_cost_provenance() {
        let mut world = world_with_limits(41, 8, 1_024);
        let edges = fixture_edges(&world);
        let stable = stable_edges(&world, &edges);
        let origin = *world.revision.canonical_origin();
        let derivation = origin
            .static_contract_versions()
            .network_revision_derivation_version();
        let route = world
            .register_admitted_route(AdmittedRouteRegisterInput::new(
                origin.network_revision(),
                derivation,
                stable,
            ))
            .expect("admitted replay");
        assert_eq!(world.route_edges(route), Some(edges.as_slice()));
        world.remove_route(route).expect("remove replay route");

        let unknown = StableId128::from_bytes([0xfe; 16]);
        assert_eq!(
            world
                .register_admitted_route(AdmittedRouteRegisterInput::new(
                    origin.network_revision(),
                    derivation,
                    vec![unknown],
                ))
                .unwrap_err(),
            AdmittedRouteRegisterError::UnknownLaneEdge { stable_id: unknown }
        );
        assert_eq!(
            world
                .register_admitted_route(AdmittedRouteRegisterInput::new(
                    NetworkRevisionId::from_digest(Sha256Digest::from_bytes([0xdd; 32])),
                    derivation,
                    vec![unknown],
                ))
                .unwrap_err(),
            AdmittedRouteRegisterError::NetworkRevisionMismatch
        );
        assert_eq!(world.live_route_count, 0);
        assert_eq!(world.live_route_edge_occurrence_count, 0);
    }
}
