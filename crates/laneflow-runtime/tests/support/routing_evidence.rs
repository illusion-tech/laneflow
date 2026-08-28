use std::sync::Arc;

use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, LaneEdgeInput, LaneEdgeReference,
    PortableDiffBase, PortableEmissionProvenance, SourceModuleHeader, SourceModuleHeaderInput,
    SyntheticModuleBuilder, derive_canonical_stable_id_v1, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_runtime::{
    CandidateRouteInput, CommittedNetworkSource, CommittedTrafficObservationBatch, CostModelKey,
    DYNAMIC_COST_BINDING_VERSION, DynamicCostSnapshotBinding, ObservationExportMode,
    ObservationExportSession, ObservationSelection, PublishedLfcaReference,
    RoutingAdmissionSession, TickInput, TrafficWorld, WorldConfig, bind_observation_set,
};
use laneflow_static_contract::{
    EntityKind, LaneEdgeId, NetworkRevisionId, Sha256Digest, StableId128,
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
        revision,
        WorldConfig::new(0, 1, EDGE_COUNT as u64, 1, DELTA_MS),
        source,
        WORLD_ID,
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
    let cost_model = CostModelKey::new(domain_digest(b"lf-routing-g2-cost-model-v1\0"), 1);
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
