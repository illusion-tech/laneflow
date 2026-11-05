//! 共享的 Routing 证据 fixture。墙钟/预算/集成三个测试二进制各自只消费
//! 本模块的一部分，逐二进制看必有未用项，整体允许 dead_code（与
//! `policy.rs` 的定向 allow 同理）。
#![allow(dead_code)]

#[path = "policy.rs"]
mod test_policy;

use std::sync::Arc;

use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, LaneEdgeInput, LaneEdgeReference,
    PortableDiffBase, PortableEmissionProvenance, SourceModuleHeader, SourceModuleHeaderInput,
    SyntheticModuleBuilder, derive_canonical_stable_id_v1, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_canonical_network_input, check_post_emission_bundle};
use laneflow_runtime::{
    CandidateRouteInput, CommittedNetworkSource, CommittedTrafficObservationBatch, CostModelId,
    CostModelKey, DYNAMIC_COST_BINDING_VERSION, DynamicCostSnapshotBinding, ObservationExportMode,
    ObservationExportSession, ObservationSelection, ParkedVehicleSpawnInput, ParkingTarget,
    PublishedLfcaReference, RouteRegisterInput, RoutingAdmissionSession, SnapshotRestoreLimits,
    TickInput, TrafficWorld, VehicleSpawnInput, WorldConfig, bind_observation_set,
};
use laneflow_static_contract::{
    EntityKind, LaneEdgeId, NetworkRevisionId, ParkingSpaceId, RightOfWayPolicySetId, Sha256Digest,
    StableId128, VehicleProfileId,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
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
    let mut world = TrafficWorld::install(
        std::sync::Arc::clone(&revision),
        WorldConfig::new(0, 1, EDGE_COUNT as u64, EDGE_COUNT as u64, DELTA_MS),
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        source,
        WORLD_ID,
        test_policy::selection(&revision),
    )
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
    hasher.update(stream.world_id().to_le_bytes());
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
    let limits = CompileLimits::p100_initial_v1();
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
            let raw = derive_canonical_stable_id_v1(EntityKind::LaneEdge, NAMESPACE, key, &limits)
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
    hasher.update(world.world_id().to_le_bytes());
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
    /// 1 边候选样本（最长 state 路线的首边）。
    pub one_route: Vec<StableId128>,
    /// 典型候选样本：中位边数的 state 路线（构造即合法）。
    pub typical_route: Vec<StableId128>,
    /// 长候选样本：最长 state 路线。输入族拓扑是无环短链（W=2 边、
    /// L1 簇路线 5 边），重复边与更长序列在合法路线中不可构造——长轴
    /// 扩展性证据由 §7.1 的 4096 边 G2-LINEAR case 承担，本边界如实
    /// 写入冻结表与结论。
    pub long_route: Vec<StableId128>,
    /// 不连通样本：两条不相邻的 state 路线首边。
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
}

impl LfSynthFixture {
    pub fn candidate_one_input(&self) -> CandidateRouteInput {
        CandidateRouteInput::new(self.cost_binding, self.one_route.clone())
    }

    pub fn candidate_typical_input(&self) -> CandidateRouteInput {
        CandidateRouteInput::new(self.cost_binding, self.typical_route.clone())
    }

    pub fn candidate_long_input(&self) -> CandidateRouteInput {
        CandidateRouteInput::new(self.cost_binding, self.long_route.clone())
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
    let state: laneflow_synth_generator::InitialState =
        serde_json::from_slice(&bundle.files()["state.json"]).expect("state.json 反序列化");

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
    let lane_change_routes: Vec<usize> = state
        .routes
        .iter()
        .enumerate()
        .filter(|(_, route)| route.edge_ids.len() > 2)
        .map(|(index, _)| index)
        .collect();
    let conflict_capacity = if lane_change_routes.is_empty() {
        0
    } else {
        (lane_change_routes.len() as u64 * 2).max(16)
    };
    let fixed_delta_ms = u64::from(state.identity.fixed_delta_ms);
    let long_len = state
        .routes
        .iter()
        .map(|route| route.edge_ids.len() as u64)
        .max()
        .expect("state 路线非空");
    let occurrence_headroom = (2 * long_len + 7).max(LFSYNTH_ROUTE_CAPACITY_HEADROOM as u64);
    let config = WorldConfig::new(
        state.vehicles.len() as u32 + LFSYNTH_VEHICLE_CAPACITY_HEADROOM,
        state_route_count as u32 + LFSYNTH_ROUTE_CAPACITY_HEADROOM,
        state_route_edge_total + occurrence_headroom,
        conflict_capacity,
        fixed_delta_ms,
    );

    let source = CommittedNetworkSource::Published {
        reference: PublishedLfcaReference::new(
            "fixture://lf-synth-candidate-admission",
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .expect("non-empty source key"),
    };
    let mut world = TrafficWorld::install(
        Arc::clone(&revision),
        config,
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        source,
        1_267,
        laneflow_runtime::WorldPolicySelection::Pinned(policy),
    )
    .expect("安装正式身份世界");

    let mut state_routes = Vec::with_capacity(state_route_count);
    for route in &state.routes {
        let edges = route
            .edge_ids
            .iter()
            .map(|id| {
                let id = id
                    .parse::<LaneEdgeId>()
                    .expect("state 路线边稳定标识可解析");
                identity.ordinal(id).expect("state 路线边在修订内")
            })
            .collect::<Vec<_>>();
        state_routes.push(
            world
                .register_route(RouteRegisterInput::new(edges))
                .expect("注册 state 路线"),
        );
    }

    let mut active_count = 0_usize;
    let mut parked_count = 0_usize;
    for vehicle in &state.vehicles {
        let route = state_routes[vehicle.route_rank as usize];
        if vehicle.status == "active" {
            let spawn = VehicleSpawnInput::new(
                profile,
                route,
                vehicle.edge_occurrence,
                vehicle.progress_mm,
                vehicle.speed_mm_per_s,
            );
            world
                .spawn_vehicle(
                    if lane_change_routes.contains(&(vehicle.route_rank as usize)) {
                        spawn.with_open_entrance()
                    } else {
                        spawn
                    },
                )
                .expect("spawn active 车辆");
            active_count += 1;
        } else {
            let space_id = vehicle
                .parking_space_id
                .as_deref()
                .expect("parked 车辆有车位标识")
                .parse::<ParkingSpaceId>()
                .expect("车位稳定标识可解析");
            let space = identity.ordinal(space_id).expect("车位在修订内");
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
                .expect("spawn parked 车辆");
            parked_count += 1;
        }
    }
    assert_eq!(active_count + parked_count, state.vehicles.len());
    assert_eq!(world.live_routes().count(), state_route_count);

    for _ in 0..LFSYNTH_WARMUP_TICKS {
        world.step(TickInput::new(fixed_delta_ms)).expect("热身拍");
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
    // 长 = 最长路线；1 边 = 长路线首边；不连通 = 两条不相邻路线首边。
    let typical_len = {
        let mut lengths: Vec<usize> = state.routes.iter().map(|r| r.edge_ids.len()).collect();
        lengths.sort_unstable();
        lengths[lengths.len() / 2]
    };
    let typical_route = state_route_stable_ids(
        &state,
        identity,
        state
            .routes
            .iter()
            .enumerate()
            .find(|(_, route)| route.edge_ids.len() == typical_len)
            .expect("中位边数路线存在")
            .0,
    );
    let (long_index, long_len) = state
        .routes
        .iter()
        .enumerate()
        .map(|(index, route)| (index, route.edge_ids.len()))
        .max_by_key(|&(_, len)| len)
        .expect("state 路线非空");
    let long_route = state_route_stable_ids(&state, identity, long_index);
    let one_route = vec![*long_route.last().expect("长路线非空")];
    let disconnected_route = {
        let other = state
            .routes
            .iter()
            .position(|route| route.edge_ids != state.routes[long_index].edge_ids)
            .expect("存在边序列不同的路线");
        // 两条机动已完成的末端边：机动检查放行，连通性检查必然拒绝。
        let first = *state_route_stable_ids(&state, identity, long_index)
            .last()
            .expect("长路线非空");
        let second = *state_route_stable_ids(&state, identity, other)
            .last()
            .expect("对照路线非空");
        assert_ne!(first, second);
        vec![first, second]
    };
    let workload_id = format!("{LFSYNTH_WORKLOAD_FAMILY}-{identity_name}");
    let workload_manifest = format!(
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

    LfSynthFixture {
        state_digest: committed_state_digest(&world),
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
        long_route,
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
    }
}

/// 第 `index` 条 state 路线的稳定标识序列（候选样本按原样注册）。
fn state_route_stable_ids(
    state: &laneflow_synth_generator::InitialState,
    identity: &laneflow_static_network::SharedIdentityIndex,
    index: usize,
) -> Vec<StableId128> {
    let _ = identity;
    state.routes[index]
        .edge_ids
        .iter()
        .map(|id| {
            id.parse::<LaneEdgeId>()
                .expect("state 路线边稳定标识可解析")
                .into_untyped()
        })
        .collect()
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
