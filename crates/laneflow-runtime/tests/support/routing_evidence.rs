//! 共享的 Routing 证据 fixture。墙钟/预算/集成三个测试二进制各自只消费
//! 本模块的一部分，逐二进制看必有未用项，整体允许 dead_code（与
//! `policy.rs` 的定向 allow 同理）。
#![allow(dead_code)]

#[path = "../../../../tools/laneflow-synth-harness/src/handoff.rs"]
mod synth_handoff;
#[path = "policy.rs"]
mod test_policy;

use std::{fmt::Write, sync::Arc};

use laneflow_compiler::{
    CompilationUnitBuilder, CompileProfile, Compiler, LaneEdgeInput, LaneEdgeReference,
    PortableDiffBase, PortableEmissionProvenance, SourceModuleHeader, SourceModuleHeaderInput,
    SyntheticModuleBuilder, derive_canonical_stable_id_v1, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_canonical_network_input, check_post_emission_bundle};
use laneflow_runtime::{
    CandidateRouteInput, CommittedNetworkSource, CommittedTrafficObservationBatch, CostModelId,
    CostModelKey, DYNAMIC_COST_BINDING_VERSION, DynamicCostSnapshotBinding, ObservationExportMode,
    ObservationExportSession, ObservationSelection, ParkedVehicleSpawnInput, ParkingTarget,
    PublishedLfcaReference, RouteHandle, RouteHandoffRequest, RoutingAdmissionSession,
    SnapshotRestoreLimits, StableRouteHandoffWindow, StepOutcome, TickInput, TrafficWorld,
    VehicleHandle, VehicleSpawnInput, WorldConfig, WorldConfigInput, WorldId, WorldInstallInput,
    bind_observation_set,
};
use laneflow_static_contract::{
    EntityKind, LaneEdgeId, NetworkRevisionId, ParkingSpaceId, RightOfWayPolicySetId,
    RoadSectionId, Sha256Digest, StableId128, VehicleProfileId,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use laneflow_synth_generator::{InitialState, RouteRecord};
use sha2::{Digest, Sha256};

pub const WORKLOAD_ID: &str = "LF-ROUTING-G2-LINEAR-v1";
pub const WORKLOAD_SEED: u64 = 303;
pub const EDGE_COUNT: usize = 4_096;
pub const TYPICAL_ROUTE_EDGE_COUNT: usize = 128;
pub const LONG_ROUTE_EDGE_COUNT: usize = EDGE_COUNT;
pub const COST_ENTRY_BYTES: usize = 24;
pub const COST_PAYLOAD_BYTES: usize = EDGE_COUNT * COST_ENTRY_BYTES;
pub const WARMUP_TICKS: u32 = 64;
pub const DELTA_MS: u64 = 4;
pub const WORLD_ID: u64 = 303;

const NAMESPACE: &str = "city/routing-g2-linear-v1";
const SOURCE_DOCUMENT: &str = "routing-g2-linear-v1.document";
const DYNAMIC_COST_SNAPSHOT_DIGEST_DOMAIN: &[u8] = b"laneflow:dynamic-cost-snapshot:v1\0";
const WORKLOAD_MANIFEST: &str = concat!(
    "workloadId=LF-ROUTING-G2-LINEAR-v1\n",
    "seed=303\n",
    "compilerLimits=p100-initial-v1\n",
    "sharedNetworkSpatial=omit\n",
    "sharedNetworkRetainedLimitBytes=134217728\n",
    "sharedNetworkScratchLimitBytes=67108864\n",
    "topology=4096-edge-directed-linear-chain\n",
    "edgeLengthMm=100000\n",
    "speedLimitMmPerSecond=13900\n",
    "vehicleCount=0\n",
    "worldConfig=vehicle:0,route:1,occurrence:4096,worker:1,deltaMs:4\n",
    "warmupTicks=64\n",
    "observationSelection=AllLaneEdges\n",
    "costEntry=StableId128+u64-le\n",
    "costEntryCount=4096\n",
    "costPayloadBytes=98304\n",
    "candidateEdgeCounts=1,128,4096\n",
    "lifecycle=register-then-remove\n",
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceiverLimits {
    pub max_entry_count: u64,
    pub max_exact_byte_length: u64,
}

impl ReceiverLimits {
    pub const EXACT_WORKLOAD: Self = Self {
        max_entry_count: EDGE_COUNT as u64,
        max_exact_byte_length: COST_PAYLOAD_BYTES as u64,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiverError {
    UnknownBindingVersion,
    ByteLimitExceeded,
    EntryLimitExceeded,
    ExactByteLengthMismatch,
    MalformedPayload,
    EntryCountMismatch,
    SnapshotDigestMismatch,
}

pub struct RoutingEvidenceFixture {
    pub world: TrafficWorld,
    pub observation_session: ObservationExportSession,
    pub initial_full: CommittedTrafficObservationBatch,
    pub cost_payload: Vec<u8>,
    pub cost_binding: DynamicCostSnapshotBinding,
    pub admission: RoutingAdmissionSession,
    pub stable_route: Vec<StableId128>,
    pub artifact_digest: Sha256Digest,
    pub network_revision: NetworkRevisionId,
    pub lfca_exact_bytes: u64,
    pub workload_manifest_digest: Sha256Digest,
    pub state_digest: Sha256Digest,
}

impl RoutingEvidenceFixture {
    pub fn candidate_input(&self, edge_count: usize) -> CandidateRouteInput {
        assert!(edge_count > 0 && edge_count <= self.stable_route.len());
        CandidateRouteInput::new(self.cost_binding, self.stable_route[..edge_count].to_vec())
    }
}

pub fn build_fixture() -> RoutingEvidenceFixture {
    let (revision, stable_route) = build_linear_revision();
    let origin = *revision.canonical_origin();
    let source = CommittedNetworkSource::Published {
        reference: PublishedLfcaReference::new(
            "fixture://routing-g2-linear-v1",
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .expect("non-empty source key"),
    };
    let mut world = TrafficWorld::install(WorldInstallInput {
        revision: std::sync::Arc::clone(&revision),
        config: WorldConfig::try_new(WorldConfigInput {
            vehicle_capacity: 0,
            route_capacity: 1,
            route_edge_occurrence_capacity: EDGE_COUNT as u64,
            route_conflict_occurrence_capacity: EDGE_COUNT as u64,
            fixed_delta_time_ms: DELTA_MS,
        })
        .expect("有效的交通配置"),
        execution: laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        source,
        world_id: WorldId::new(WORLD_ID),
        policy_selection: test_policy::selection(&revision),
    })
    .expect("install routing evidence world");
    for _ in 0..WARMUP_TICKS {
        world.step(TickInput::new(DELTA_MS)).expect("warmup tick");
    }

    let mut observation_session = world
        .open_observation_export(ObservationSelection::AllLaneEdges)
        .expect("open all-edge observation");
    let initial_full = world
        .export_observation(&mut observation_session, ObservationExportMode::Full)
        .expect("initial full observation");
    assert_eq!(initial_full.rows().len(), EDGE_COUNT);
    assert!(initial_full.rows().iter().all(|row| {
        row.front_vehicle_count() == 0
            && row.occupied_length_mm() == 0
            && row.front_speed_sum_mm_per_second() == 0
    }));

    let cost_payload = cost_payload(&initial_full);
    let cost_model = CostModelKey::new(
        CostModelId::from_bytes(*domain_digest(b"lf-routing-g2-cost-model-v1\0").as_bytes()),
        1,
    );
    let observations = bind_observation_set(&[&initial_full]).expect("observation set");
    let placeholder = Sha256Digest::from_bytes([0x5a; 32]);
    let unsigned = DynamicCostSnapshotBinding::new(
        observations,
        cost_model,
        u64::MAX,
        EDGE_COUNT as u64,
        COST_PAYLOAD_BYTES as u64,
        placeholder,
    )
    .expect("cost binding header");
    let snapshot_digest = dynamic_cost_snapshot_digest(unsigned, &cost_payload);
    let cost_binding = DynamicCostSnapshotBinding::new(
        observations,
        cost_model,
        u64::MAX,
        EDGE_COUNT as u64,
        COST_PAYLOAD_BYTES as u64,
        snapshot_digest,
    )
    .expect("cost binding");
    receive_cost_snapshot(cost_binding, &cost_payload, ReceiverLimits::EXACT_WORKLOAD)
        .expect("fixture receiver");
    let admission = world.open_routing_admission(cost_model);

    RoutingEvidenceFixture {
        artifact_digest: origin.canonical_artifact_digest(),
        network_revision: origin.network_revision(),
        lfca_exact_bytes: origin.canonical_artifact_byte_length().get(),
        workload_manifest_digest: domain_digest(WORKLOAD_MANIFEST.as_bytes()),
        state_digest: committed_state_digest(&world),
        world,
        observation_session,
        initial_full,
        cost_payload,
        cost_binding,
        admission,
        stable_route,
    }
}

pub fn fixed_input_sequence_digest(sequence: &str) -> Sha256Digest {
    domain_digest(sequence.as_bytes())
}

pub fn receive_cost_snapshot(
    binding: DynamicCostSnapshotBinding,
    payload: &[u8],
    limits: ReceiverLimits,
) -> Result<DynamicCostSnapshotBinding, ReceiverError> {
    if binding.binding_version() != DYNAMIC_COST_BINDING_VERSION {
        return Err(ReceiverError::UnknownBindingVersion);
    }
    let actual_bytes =
        u64::try_from(payload.len()).map_err(|_| ReceiverError::ByteLimitExceeded)?;
    if binding.exact_byte_length() > limits.max_exact_byte_length
        || actual_bytes > limits.max_exact_byte_length
    {
        return Err(ReceiverError::ByteLimitExceeded);
    }
    if binding.entry_count() > limits.max_entry_count {
        return Err(ReceiverError::EntryLimitExceeded);
    }
    if binding.exact_byte_length() != actual_bytes {
        return Err(ReceiverError::ExactByteLengthMismatch);
    }
    if !payload.len().is_multiple_of(COST_ENTRY_BYTES) {
        return Err(ReceiverError::MalformedPayload);
    }
    let actual_entries = u64::try_from(payload.len() / COST_ENTRY_BYTES)
        .map_err(|_| ReceiverError::EntryLimitExceeded)?;
    if binding.entry_count() != actual_entries {
        return Err(ReceiverError::EntryCountMismatch);
    }
    if binding.snapshot_sha256() != dynamic_cost_snapshot_digest(binding, payload) {
        return Err(ReceiverError::SnapshotDigestMismatch);
    }
    Ok(binding)
}

pub fn dynamic_cost_snapshot_digest(
    binding: DynamicCostSnapshotBinding,
    payload: &[u8],
) -> Sha256Digest {
    let observations = binding.observation_set();
    let stream = observations.stream_binding();
    let model = binding.cost_model();
    let mut hasher = Sha256::new();
    hasher.update(DYNAMIC_COST_SNAPSHOT_DIGEST_DOMAIN);
    hasher.update(binding.binding_version().to_le_bytes());
    hasher.update(stream.world_id().get().to_le_bytes());
    hasher.update(stream.world_generation().get().to_le_bytes());
    hasher.update(observations.network_revision().as_digest().as_bytes());
    hasher.update(
        observations
            .network_revision_derivation_version()
            .to_le_bytes(),
    );
    hasher.update(observations.observation_tick().to_le_bytes());
    hasher.update(
        observations
            .observation_state_sequence()
            .get()
            .to_le_bytes(),
    );
    hasher.update(observations.digest().as_bytes());
    hasher.update(model.model_id().as_bytes());
    hasher.update(model.model_version().to_le_bytes());
    hasher.update(binding.valid_through_tick().to_le_bytes());
    hasher.update(binding.entry_count().to_le_bytes());
    hasher.update(binding.exact_byte_length().to_le_bytes());
    hasher.update(payload);
    Sha256Digest::from_bytes(hasher.finalize().into())
}

fn build_linear_revision() -> (Arc<SharedNetworkRevision>, Vec<StableId128>) {
    let limits = CompileProfile::single_network_10k_stable_entities();
    let workload_manifest_digest = domain_digest(WORKLOAD_MANIFEST.as_bytes());
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: NAMESPACE,
            source_document_key: SOURCE_DOCUMENT,
            generator_build_id: WORKLOAD_ID,
            parameters_and_inputs_digest: workload_manifest_digest.into_bytes(),
            frontend_options_digest: [0; 32],
            random_seed: Some(WORKLOAD_SEED),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .expect("source header");
    let mut module = SyntheticModuleBuilder::new(header, &limits).expect("synthetic module");
    let keys: Vec<_> = (0..EDGE_COUNT)
        .map(|index| format!("edge-{index:04}"))
        .collect();
    for (index, key) in keys.iter().enumerate() {
        if let Some(next) = keys.get(index + 1) {
            let successors = [LaneEdgeReference::local(next)];
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: 100.0,
                    speed_limit_meters_per_second: 13.9,
                    successors: &successors,
                })
                .expect("linear edge");
        } else {
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: 100.0,
                    speed_limit_meters_per_second: 13.9,
                    successors: &[],
                })
                .expect("terminal edge");
        }
    }
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    unit.add_synthetic_module(module.finish().expect("finished module"))
        .expect("add module");
    let output = Compiler::new()
        .compile(unit.build().expect("compilation unit"))
        .expect("compile linear workload");
    let candidate = emit_portable_candidate(
        &output,
        &PortableEmissionProvenance::try_new(WORKLOAD_ID).expect("portable provenance"),
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .expect("portable candidate");
    let checked = check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .expect("post-emission checked bundle");
    let revision = build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(128 * 1_024 * 1_024, 64 * 1_024 * 1_024),
        ),
    )
    .expect("shared linear revision");
    assert_eq!(revision.traffic().lane_edge_count() as usize, EDGE_COUNT);

    let stable_route: Vec<_> = keys
        .iter()
        .map(|key| {
            let raw = derive_canonical_stable_id_v1(EntityKind::LaneEdge, NAMESPACE, key)
                .expect("LaneEdge stable id");
            let ordinal = revision
                .identity()
                .ordinal(LaneEdgeId::from_untyped(raw))
                .expect("LaneEdge identity in revision");
            assert_eq!(
                revision
                    .identity()
                    .stable_id(ordinal)
                    .expect("round trip")
                    .into_untyped(),
                raw
            );
            raw
        })
        .collect();
    (revision, stable_route)
}

fn cost_payload(batch: &CommittedTrafficObservationBatch) -> Vec<u8> {
    let mut payload = Vec::new();
    payload
        .try_reserve_exact(COST_PAYLOAD_BYTES)
        .expect("cost payload reserve");
    for row in batch.rows() {
        payload.extend_from_slice(row.lane_edge_stable_id().as_untyped().as_bytes());
        payload.extend_from_slice(&1_u64.to_le_bytes());
    }
    assert_eq!(payload.len(), COST_PAYLOAD_BYTES);
    payload
}

fn committed_state_digest(world: &TrafficWorld) -> Sha256Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"laneflow:routing-g2-evidence-state:v1\0");
    hasher.update(world.world_id().get().to_le_bytes());
    hasher.update(world.world_generation().get().to_le_bytes());
    hasher.update(world.tick_index().to_le_bytes());
    hasher.update(world.observation_state_sequence().get().to_le_bytes());
    hasher.update(0_u64.to_le_bytes());
    Sha256Digest::from_bytes(hasher.finalize().into())
}

fn domain_digest(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
}

/// 墙钟/预算证据的测量助手面向的最小 fixture 面（G2 与 LF-SYNTH 共用）。
pub trait CandidateAdmissionFixture {
    fn world_mut(&mut self) -> &mut TrafficWorld;
    fn session_mut(&mut self) -> &mut ObservationExportSession;
    fn cost_binding(&self) -> DynamicCostSnapshotBinding;
    fn cost_payload(&self) -> &[u8];
    /// 观测会话持有在 fixture 内；经方法导出避免外部双重借用。
    fn export_observation(
        &mut self,
        mode: ObservationExportMode,
    ) -> Result<CommittedTrafficObservationBatch, laneflow_runtime::ObservationError>;
    /// 准入会话持有在 fixture 内；经方法注册避免外部双重借用。
    fn register_candidate(
        &mut self,
        input: CandidateRouteInput,
    ) -> Result<laneflow_runtime::RouteHandle, laneflow_runtime::CandidateRouteError>;
}

impl CandidateAdmissionFixture for RoutingEvidenceFixture {
    fn world_mut(&mut self) -> &mut TrafficWorld {
        &mut self.world
    }
    fn session_mut(&mut self) -> &mut ObservationExportSession {
        &mut self.observation_session
    }
    fn cost_binding(&self) -> DynamicCostSnapshotBinding {
        self.cost_binding
    }
    fn cost_payload(&self) -> &[u8] {
        &self.cost_payload
    }
    fn export_observation(
        &mut self,
        mode: ObservationExportMode,
    ) -> Result<CommittedTrafficObservationBatch, laneflow_runtime::ObservationError> {
        self.world
            .export_observation(&mut self.observation_session, mode)
    }
    fn register_candidate(
        &mut self,
        input: CandidateRouteInput,
    ) -> Result<laneflow_runtime::RouteHandle, laneflow_runtime::CandidateRouteError> {
        self.world.register_candidate_route(&self.admission, input)
    }
}

impl CandidateAdmissionFixture for LfSynthFixture {
    fn world_mut(&mut self) -> &mut TrafficWorld {
        &mut self.world
    }
    fn session_mut(&mut self) -> &mut ObservationExportSession {
        &mut self.observation_session
    }
    fn cost_binding(&self) -> DynamicCostSnapshotBinding {
        self.cost_binding
    }
    fn cost_payload(&self) -> &[u8] {
        &self.cost_payload
    }
    fn export_observation(
        &mut self,
        mode: ObservationExportMode,
    ) -> Result<CommittedTrafficObservationBatch, laneflow_runtime::ObservationError> {
        self.world
            .export_observation(&mut self.observation_session, mode)
    }
    fn register_candidate(
        &mut self,
        input: CandidateRouteInput,
    ) -> Result<laneflow_runtime::RouteHandle, laneflow_runtime::CandidateRouteError> {
        self.world.register_candidate_route(&self.admission, input)
    }
}

// ===== LF-SYNTH-v2 正式负载（#1267 候选准入集成验收） =====
//
// 与上面的 G2 具名 case 不同，本 fixture 消费 `laneflow-synth-generator`
// 的正式四文件输入目录（network.lfca/state.json/input-plan.json/manifest.json），
// 经 `verify_directory` 重编译逐字节比对绑定输入身份。安装与 spawn 语义
// 对齐 `tools/laneflow-synth-harness` 的 `install.rs`，裁剪到候选准入证据
// 所需（不解析信号组、不驱动换道日志）；WorldConfig 在 harness 同式之上
// 加候选/重放余量，公式冻结进设计文档 §7.2。

/// 具名工作负载族；每个身份目录是一个 case（`<族>-<身份目录名>`）。
pub const LFSYNTH_WORKLOAD_FAMILY: &str = "LF-ROUTING-CANDIDATE-LFSYNTH-v1";
pub const LFSYNTH_INPUT_ROOT_ENV: &str = "LANEFLOW_LF_SYNTH_INPUT_ROOT";
pub const LFSYNTH_IDENTITY_ENV: &str = "LANEFLOW_ROUTING_EVIDENCE_IDENTITY";
/// 安装后固定热身拍数（与 §7.1 的 64 对齐，进冻结表）。
pub const LFSYNTH_WARMUP_TICKS: u32 = 64;
/// 路线槽余量：容量探针的并发填充 + 检查点候选与重放后缀的并发
/// （admitted + 候选 + spawn 后仍在新路线上的句柄）。
pub const LFSYNTH_ROUTE_CAPACITY_HEADROOM: u32 = 12;
/// 车辆槽余量：检查点后重放侧的宿主 spawn。
pub const LFSYNTH_VEHICLE_CAPACITY_HEADROOM: u32 = 2;
/// 正式身份恢复上限：与 F3 restore-matrix 在 100k 规模的先例一致。
pub fn lf_synth_restore_limits() -> SnapshotRestoreLimits {
    SnapshotRestoreLimits::new(512 * 1_024 * 1_024, 4 * 1_024)
}

/// 读取环境变量选身份；错误信息即使用指引（未设置时测试以 panic 给出）。
pub fn lf_synth_identity_from_env() -> Result<(std::path::PathBuf, String), String> {
    let root = std::env::var_os(LFSYNTH_INPUT_ROOT_ENV)
        .map(std::path::PathBuf::from)
        .ok_or_else(|| {
            format!("未设置 {LFSYNTH_INPUT_ROOT_ENV}：须指向正式输入根目录（其下为各身份子目录）")
        })?;
    let name = std::env::var(LFSYNTH_IDENTITY_ENV).map_err(|_| {
        format!("未设置 {LFSYNTH_IDENTITY_ENV}：须为具体身份目录名，如 10000-W2-16-a")
    })?;
    let directory = root.join(&name);
    if !directory.is_dir() {
        return Err(format!("身份目录不存在：{}", directory.display()));
    }
    Ok((directory, name))
}

pub struct LfSynthFixture {
    pub world: TrafficWorld,
    pub observation_session: ObservationExportSession,
    pub initial_full: CommittedTrafficObservationBatch,
    pub cost_payload: Vec<u8>,
    pub cost_binding: DynamicCostSnapshotBinding,
    pub admission: RoutingAdmissionSession,
    /// 1 边候选样本（最长 state 路线的末端边）。
    pub one_route: Vec<StableId128>,
    /// 典型候选样本：中位边数的 state 路线（构造即合法）。
    pub typical_route: Vec<StableId128>,
    pub typical_handoffs: Vec<StableRouteHandoffWindow>,
    /// 长候选样本：最长 state 路线。输入族拓扑是无环短链（W=2 边、
    /// L1 簇路线 4 边），重复边与更长序列在合法路线中不可构造——长轴
    /// 扩展性证据由 §7.1 的 4096 边 G2-LINEAR case 承担，本边界如实
    /// 写入冻结表与结论。
    pub long_route: Vec<StableId128>,
    pub long_handoffs: Vec<StableRouteHandoffWindow>,
    /// 不连通样本：两条不同的终端边。
    pub disconnected_route: Vec<StableId128>,
    /// 安装用的车型 ordinal（重放侧 spawn 复用）。
    pub profile_ordinal: laneflow_static_contract::VehicleProfileOrdinal,
    /// 安装用的钉定策略（孪生世界复用）。
    pub policy: laneflow_runtime::PolicyPin,
    pub identity_name: String,
    pub workload_id: String,
    pub edge_count: usize,
    pub state_route_count: usize,
    pub state_route_edge_total: u64,
    /// 出现记录余量 = max(2×长候选边数 + 7, 路线槽余量)：2L+7 覆盖检查点
    /// 候选 + 重放后缀 admitted/候选注册（2L+1）与探针预算；下限取路线
    /// 槽余量保证槽探针的 1 边填充不先撞出现记录。公式冻结进 §7.2。
    pub occurrence_headroom: u64,
    pub vehicle_count: usize,
    pub active_count: usize,
    pub parked_count: usize,
    pub fixed_delta_ms: u64,
    pub manifest_digest: Sha256Digest,
    pub network_revision: NetworkRevisionId,
    pub lfca_exact_bytes: u64,
    pub workload_manifest_digest: Sha256Digest,
    pub state_digest: Sha256Digest,
    handoff_driver: Option<LfSynthHandoffDriver>,
}

struct LfSynthHandoffDriver {
    state: InitialState,
    handles: Vec<VehicleHandle>,
    routes: Vec<RouteHandle>,
}

impl LfSynthFixture {
    pub fn step_requests(&self) -> Vec<RouteHandoffRequest> {
        self.handoff_driver
            .as_ref()
            .map_or_else(Vec::new, |driver| {
                synth_handoff::requests(&self.world, &driver.state, &driver.handles, &driver.routes)
                    .expect("按正式输入与拍初提交态生成横移申请")
            })
    }

    pub fn step(&mut self) -> StepOutcome {
        let requests = self.step_requests();
        self.world
            .step_with_route_handoffs(TickInput::new(self.fixed_delta_ms), &requests)
            .expect("推进正式输入")
    }

    pub fn fixed_input_sequence_digest(&self, sequence: &str) -> Sha256Digest {
        if self.handoff_driver.is_some() {
            return fixed_input_sequence_digest(&format!(
                "{sequence}each-step=route-handoffs-from-committed-state;candidate=stable-handoff-windows;"
            ));
        }
        fixed_input_sequence_digest(sequence)
    }

    pub fn candidate_one_input(&self) -> CandidateRouteInput {
        CandidateRouteInput::new(self.cost_binding, self.one_route.clone())
    }

    pub fn candidate_typical_input(&self) -> CandidateRouteInput {
        CandidateRouteInput::new(self.cost_binding, self.typical_route.clone())
            .with_handoffs(self.typical_handoffs.clone())
    }

    pub fn candidate_long_input(&self) -> CandidateRouteInput {
        CandidateRouteInput::new(self.cost_binding, self.long_route.clone())
            .with_handoffs(self.long_handoffs.clone())
    }

    pub fn disconnected_candidate_input(&self) -> CandidateRouteInput {
        CandidateRouteInput::new(self.cost_binding, self.disconnected_route.clone())
    }

    pub fn receiver_limits(&self) -> ReceiverLimits {
        ReceiverLimits {
            max_entry_count: self.cost_binding.entry_count(),
            max_exact_byte_length: self.cost_binding.exact_byte_length(),
        }
    }
}

/// 安装正式身份世界：校验输入 → 共享修订 → 候选余量配置 → 注册 state 路线
/// → spawn 全部车辆 → 固定热身 → 全量观测 → 成本绑定 → 准入会话。
pub fn build_lf_synth_fixture() -> LfSynthFixture {
    let (directory, identity_name) = match lf_synth_identity_from_env() {
        Ok(selection) => selection,
        Err(guidance) => panic!("{guidance}"),
    };
    let bundle = laneflow_synth_generator::verify_directory(&directory)
        .expect("正式身份目录经 verify_directory 重编译比对");
    let manifest_bytes = bundle.files()["manifest.json"].clone();
    let state: InitialState =
        serde_json::from_slice(&bundle.files()["state.json"]).expect("state.json 反序列化");
    // 三个维度来自经重编译校验的输入；目录可另有副本后缀（例如 -a）。
    let semantic_name = format!(
        "{}-{}-{}",
        state.identity.scale,
        state.identity.scenario_label(),
        state.identity.fixed_delta_ms,
    );
    assert!(
        identity_name == semantic_name
            || identity_name
                .strip_prefix(&semantic_name)
                .and_then(|suffix| suffix.strip_prefix('-'))
                .is_some_and(|copy| !copy.is_empty()),
        "身份目录 {identity_name} 与正式输入的规模、场景、步长 {semantic_name} 不符"
    );

    let input = check_canonical_network_input(
        bundle.files()["network.lfca"].as_slice(),
        FormatLimits::HARD,
    )
    .expect("LFCA 规范校验");
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(2 * 1_024 * 1_024 * 1_024, 1_024 * 1_024 * 1_024),
        ),
    )
    .expect("共享路网修订");
    let identity = revision.identity();
    let origin = *revision.canonical_origin();

    let profile_id = state
        .vehicle_profile_id
        .parse::<VehicleProfileId>()
        .expect("profile id");
    let profile = identity.ordinal(profile_id).expect("profile ordinal");
    let policy = laneflow_runtime::PolicyPin {
        policy: state
            .policy_id
            .parse::<RightOfWayPolicySetId>()
            .expect("policy id"),
    };

    let state_route_count = state.routes.len();
    let state_route_edge_total: u64 = state.routes.iter().map(|r| r.edge_ids.len() as u64).sum();
    let handoff_routes = state.routes.iter().filter(|r| r.handoff.is_some()).count();
    let clusters = state
        .identity
        .lane_change()
        .map_or(0, |p| p.clusters as usize);
    assert_eq!(handoff_routes, clusters * 2, "每簇两条外侧来源路线");
    let conflict_capacity = if handoff_routes == 0 {
        0
    } else {
        (handoff_routes as u64 * 2).max(16)
    };
    let fixed_delta_ms = u64::from(state.identity.fixed_delta_ms);
    let long_len = state
        .routes
        .iter()
        .map(|route| route.edge_ids.len() as u64)
        .max()
        .expect("state 路线非空");
    let occurrence_headroom = (2 * long_len + 7).max(LFSYNTH_ROUTE_CAPACITY_HEADROOM as u64);
    let config = WorldConfig::try_new(WorldConfigInput {
        vehicle_capacity: state.vehicles.len() as u32 + LFSYNTH_VEHICLE_CAPACITY_HEADROOM,
        route_capacity: state_route_count as u32 + LFSYNTH_ROUTE_CAPACITY_HEADROOM,
        route_edge_occurrence_capacity: state_route_edge_total + occurrence_headroom,
        route_conflict_occurrence_capacity: conflict_capacity,
        fixed_delta_time_ms: fixed_delta_ms,
    })
    .expect("有效的交通配置");

    let source = CommittedNetworkSource::Published {
        reference: PublishedLfcaReference::new(
            "fixture://lf-synth-candidate-admission",
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .expect("non-empty source key"),
    };
    let mut world = TrafficWorld::install(WorldInstallInput {
        revision: Arc::clone(&revision),
        config: synth_handoff::configure_world(&state, config).expect("正式横移驾驶配置"),
        execution: laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        source,
        world_id: WorldId::new(1_267),
        policy_selection: laneflow_runtime::WorldPolicySelection::Pinned(policy),
    })
    .expect("安装正式身份世界");

    let mut state_routes = Vec::with_capacity(state_route_count);
    for route in &state.routes {
        state_routes
            .push(synth_handoff::register_route(&mut world, route).expect("注册 state 路线"));
    }

    let mut active_count = 0_usize;
    let mut parked_count = 0_usize;
    let mut handles = Vec::with_capacity(state.vehicles.len());
    for vehicle in &state.vehicles {
        let route = state_routes[vehicle.route_rank as usize];
        let handle = if vehicle.status == "active" {
            let spawn = VehicleSpawnInput::new(
                profile,
                route,
                vehicle.edge_occurrence,
                vehicle.progress_mm,
                vehicle.speed_mm_per_s,
            );
            active_count += 1;
            world.spawn_vehicle(spawn).expect("spawn active 车辆")
        } else {
            let space_id = vehicle
                .parking_space_id
                .as_deref()
                .expect("parked 车辆有车位标识")
                .parse::<ParkingSpaceId>()
                .expect("车位稳定标识可解析");
            let space = identity.ordinal(space_id).expect("车位在修订内");
            parked_count += 1;
            world
                .spawn_parked_vehicle(
                    ParkedVehicleSpawnInput::new(
                        profile,
                        route,
                        vehicle.edge_occurrence,
                        vehicle.progress_mm,
                    ),
                    ParkingTarget::ExplicitSpace(space),
                )
                .expect("spawn parked 车辆")
                .vehicle
        };
        handles.push(handle);
    }
    assert_eq!(active_count + parked_count, state.vehicles.len());
    assert_eq!(world.live_routes().count(), state_route_count);

    for tick in 1..=LFSYNTH_WARMUP_TICKS {
        let requests = synth_handoff::requests(&world, &state, &handles, &state_routes)
            .expect("按正式输入与拍初提交态生成热身申请");
        if clusters > 0 && tick <= 2 {
            assert_eq!(
                requests.len(),
                if tick == 1 { 2 * clusters } else { clusters }
            );
        }
        world
            .step_with_route_handoffs(TickInput::new(fixed_delta_ms), &requests)
            .expect("热身拍");
    }
    if clusters > 0 {
        let (pairs, rest) = handles[state.identity.scale as usize..].as_chunks::<2>();
        assert_eq!(pairs.len(), clusters);
        assert!(rest.is_empty());
        for &[first, second] in pairs {
            let mut active = world.vehicle_route_handoffs(first);
            assert_eq!(
                active.next().map(|p| p.acquired_tick),
                Some(1),
                "第一辆车仍持有同拍竞争取得的预约"
            );
            assert!(active.next().is_none());
            assert!(world.vehicle_route_handoffs(second).next().is_none());
        }
    }

    let mut observation_session = world
        .open_observation_export(ObservationSelection::AllLaneEdges)
        .expect("打开全边观测会话");
    let initial_full = world
        .export_observation(&mut observation_session, ObservationExportMode::Full)
        .expect("初始全量观测");
    let edge_count = revision.traffic().lane_edge_count() as usize;
    assert_eq!(initial_full.rows().len(), edge_count);
    assert_eq!(initial_full.entry_count(), edge_count as u64);

    let cost_payload = lf_synth_cost_payload(&initial_full);
    let cost_model = lf_synth_cost_model();
    let observations = bind_observation_set(&[&initial_full]).expect("观测集合绑定");
    let placeholder = Sha256Digest::from_bytes([0x5a; 32]);
    let unsigned = DynamicCostSnapshotBinding::new(
        observations,
        cost_model,
        u64::MAX,
        edge_count as u64,
        cost_payload.len() as u64,
        placeholder,
    )
    .expect("成本绑定头");
    let snapshot_digest = dynamic_cost_snapshot_digest(unsigned, &cost_payload);
    let cost_binding = DynamicCostSnapshotBinding::new(
        observations,
        cost_model,
        u64::MAX,
        edge_count as u64,
        cost_payload.len() as u64,
        snapshot_digest,
    )
    .expect("成本绑定");
    let receiver_limits = ReceiverLimits {
        max_entry_count: edge_count as u64,
        max_exact_byte_length: cost_payload.len() as u64,
    };
    receive_cost_snapshot(cost_binding, &cost_payload, receiver_limits)
        .expect("fixture receiver 接受正式负载");
    let admission = world.open_routing_admission(cost_model);

    // 候选样本取自 state 路线本身（构造即合法）：典型 = 中位边数路线、
    // 长 = 最长路线；1 边 = 长路线末端；不连通 = 两条不同的终端边。
    let typical_len = {
        let mut lengths: Vec<usize> = state.routes.iter().map(|r| r.edge_ids.len()).collect();
        lengths.sort_unstable();
        lengths[lengths.len() / 2]
    };
    let typical_index = state
        .routes
        .iter()
        .position(|route| route.edge_ids.len() == typical_len)
        .expect("中位边数路线存在");
    let typical_route = state_route_stable_ids(&state.routes[typical_index]);
    let typical_handoffs = stable_windows(
        &world,
        &state.routes[typical_index],
        state_routes[typical_index],
    );
    let (long_index, long_len) = state
        .routes
        .iter()
        .enumerate()
        .map(|(index, route)| (index, route.edge_ids.len()))
        .max_by_key(|&(_, len)| len)
        .expect("state 路线非空");
    let long_route = state_route_stable_ids(&state.routes[long_index]);
    let long_handoffs = stable_windows(&world, &state.routes[long_index], state_routes[long_index]);
    assert_eq!(long_handoffs.len(), usize::from(clusters > 0));
    let one_route = vec![*long_route.last().expect("长路线非空")];
    let disconnected_route = {
        let first = *long_route.last().expect("长路线非空");
        let other = state
            .routes
            .iter()
            .find(|route| route.edge_ids.last() != state.routes[long_index].edge_ids.last())
            .expect("存在末端边不同的路线");
        // 两条机动已完成的末端边：机动检查放行，连通性检查必然拒绝。
        let second = *state_route_stable_ids(other).last().expect("对照路线非空");
        vec![first, second]
    };
    // 墙钟验收从回执头绑定工作负载 ID。动态窗口与固定过渡的旧结果有各自的身份。
    let workload_id = if clusters > 0 {
        format!("{LFSYNTH_WORKLOAD_FAMILY}-{identity_name}-route-handoff")
    } else {
        format!("{LFSYNTH_WORKLOAD_FAMILY}-{identity_name}")
    };
    let mut workload_manifest = format!(
        "{}\n",
        [
            format!("workloadId={workload_id}"),
            format!("inputIdentity={identity_name}"),
            format!("lfcaExactBytes={}", origin.canonical_artifact_byte_length().get()),
            format!("edges={edge_count}"),
            format!(
                "vehicles={} active={active_count} parked={parked_count}",
                state.vehicles.len()
            ),
            format!("stateRoutes={state_route_count} stateRouteEdges={state_route_edge_total}"),
            format!(
                "worldConfig=vehicle+2,route+12,occurrence+{occurrence_headroom},conflict:{conflict_capacity},worker:1,deltaMs:{fixed_delta_ms}"
            ),
            format!("warmupTicks={LFSYNTH_WARMUP_TICKS}"),
            "observationSelection=AllLaneEdges".to_owned(),
            "costEntry=StableId128+u64-le".to_owned(),
            format!("costEntryCount={edge_count}"),
            format!("costPayloadBytes={}", cost_payload.len()),
            format!(
                "candidateEdgeCounts=1,{typical_len},{long_len};topology=acyclic-short-chains-no-repeat"
            ),
            "lifecycle=register-then-remove".to_owned(),
        ]
        .join("\n")
    );
    let state_digest = if clusters > 0 {
        let state_input_digest = domain_digest(&bundle.files()["state.json"]);
        writeln!(
            workload_manifest,
            "handoffWarmup=committed-state-requests;handoffRoutes={handoff_routes};stateSha256={state_input_digest:x};warmStateDigest=sha256-lfrs"
        )
        .expect("写入正式窗口及驾驶参数的输入指纹");
        let snapshot = world.capture_snapshot().expect("L1 热身后的完整状态");
        domain_digest(&laneflow_runtime::encode_lfrs(&snapshot))
    } else {
        committed_state_digest(&world)
    };

    LfSynthFixture {
        state_digest,
        workload_manifest_digest: domain_digest(workload_manifest.as_bytes()),
        manifest_digest: domain_digest(&manifest_bytes),
        network_revision: origin.network_revision(),
        lfca_exact_bytes: origin.canonical_artifact_byte_length().get(),
        world,
        observation_session,
        initial_full,
        cost_payload,
        cost_binding,
        admission,
        one_route,
        typical_route,
        typical_handoffs,
        long_route,
        long_handoffs,
        disconnected_route,
        profile_ordinal: profile,
        policy,
        identity_name,
        workload_id,
        edge_count,
        state_route_count,
        state_route_edge_total,
        occurrence_headroom,
        vehicle_count: state.vehicles.len(),
        active_count,
        parked_count,
        fixed_delta_ms,
        handoff_driver: (clusters > 0).then_some(LfSynthHandoffDriver {
            state,
            handles,
            routes: state_routes,
        }),
    }
}

fn state_route_stable_ids(route: &RouteRecord) -> Vec<StableId128> {
    route
        .edge_ids
        .iter()
        .map(|id| {
            id.parse::<LaneEdgeId>()
                .expect("state 路线边稳定标识可解析")
                .into_untyped()
        })
        .collect()
}

/// 从已登记的物理绑定取稳定窗口，再以输入区段及其左右成员独立核对。
fn stable_windows(
    world: &TrafficWorld,
    route: &RouteRecord,
    handle: RouteHandle,
) -> Vec<StableRouteHandoffWindow> {
    let revision = world.revision();
    let bound = world.route_handoff_windows(handle).expect("路线句柄有效");
    let Some(input) = &route.handoff else {
        assert!(bound.is_empty(), "普通路线不得有横移窗口");
        return Vec::new();
    };
    let &[(source_index, binding)] = bound else {
        panic!("正式 L1 路线须保留输入的一条窗口");
    };
    let stable = StableRouteHandoffWindow::from_binding(&revision, source_index, binding)
        .expect("已绑定窗口有稳定身份");
    let section = input
        .section_id
        .parse::<RoadSectionId>()
        .expect("输入区段稳定身份");
    let ids = revision.identity();
    let lanes = revision
        .traffic()
        .relations()
        .section_lanes(ids.ordinal(section).expect("输入区段存在"))
        .expect("输入区段有车道");
    let left = input.left_index as usize;
    assert_eq!(
        stable,
        StableRouteHandoffWindow {
            source_index: input.source_index,
            owner_kind: 2,
            owner: section.into_untyped(),
            left_member: ids
                .stable_id(lanes[left])
                .expect("左侧车道身份")
                .into_untyped(),
            right_member: ids
                .stable_id(lanes[left + 1])
                .expect("右侧车道身份")
                .into_untyped(),
            stations_mm: input.stations_mm,
        }
    );
    vec![stable]
}

/// 正式负载的成本模型标识（域摘要派生，与 G2 case 同法不同域）。
pub fn lf_synth_cost_model() -> CostModelKey {
    CostModelKey::new(
        CostModelId::from_bytes(*domain_digest(b"lf-routing-candidate-lfsynth-v1\0").as_bytes()),
        1,
    )
}

/// 由观测行构造 24 字节条目的成本 payload（StableId128 + u64-le）。
pub fn lf_synth_cost_payload(batch: &CommittedTrafficObservationBatch) -> Vec<u8> {
    let mut payload = Vec::new();
    payload
        .try_reserve_exact(batch.rows().len() * COST_ENTRY_BYTES)
        .expect("cost payload reserve");
    for row in batch.rows() {
        payload.extend_from_slice(row.lane_edge_stable_id().as_untyped().as_bytes());
        payload.extend_from_slice(&1_u64.to_le_bytes());
    }
    payload
}
