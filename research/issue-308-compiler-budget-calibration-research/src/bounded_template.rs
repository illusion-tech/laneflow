//! 模板型工作负载的安全有界正式执行路径。
//!
//! 该路径不接受一个与实际容器脱节的总量估算。模块图暂存、声明、字段载荷、待排序
//! 记录、规范记录、阶段值/载荷、排序暂存和最终输出都使用 `ControlledVec`；任何新增
//! 容量路径若未获得硬上限额度，就无法通过可失败增长接口继续执行。

#[cfg(any(
    feature = "candidate-hashbrown-xxh3",
    feature = "candidate-hashbrown-xxh64"
))]
use crate::candidate_matrix::FIXED_HASHER_SEED;
#[cfg(feature = "candidate-hashbrown-fnv1a64")]
use crate::candidate_matrix::Fnv1a64BuildHasher;
use crate::candidate_matrix::{
    CandidateKeyDomain, CandidatePipelineChecksums, CandidatePipelineConfiguration,
};
use crate::controlled_alloc::{
    ControlledAllocationSnapshot, ControlledAllocator, ControlledTransientReservation,
    ControlledVec,
};
use crate::corridor::{
    CorridorTemplate, EntityRef, TemplateGeometryRule, TemplateRelation, UnitEntityRef,
};
use crate::identity::{IdentityContract, IdentityFieldValue};
use crate::stage::{
    HirStageRecord, MirLirStageRecord, SourceSpanRecord, StageContract, StageRetainedCapacityBytes,
    TypedAstStageRecord,
};
use crate::{
    DUPLICATE_OWNER_ERROR_CODE, GeneratorContract, GraphProfileId, ScalableStagePlanSummary,
    ScalableWorkloadId, SequenceKind, StageGenerationError, UNKNOWN_REFERENCE_ERROR_CODE,
    permute_in_place,
};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::RandomState;

const IDENTITY_MAGIC: &[u8; 4] = b"LFID";
const STABLE_ID_DOMAIN: &[u8] = b"laneflow.stable-id.v1\0";
const ABSENT_LOCAL_INDEX: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PayloadRange {
    pub(crate) offset: usize,
    pub(crate) length: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BoundedField {
    tag: u16,
    bytes: PayloadRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BoundedDeclaration {
    owner: UnitEntityRef,
    stable_id: [u8; 16],
    owner_ordinal: u32,
    fields: PayloadRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BoundedFailureCandidate {
    record_index: usize,
    unit: u32,
    relation_sequence_ordinal: u32,
    route_ordinal_within_unit: u32,
    reference_ordinal: u64,
    selected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BoundedDuplicateOwnerCandidate {
    unit: u32,
    relation_sequence_ordinal: u32,
    child_kind: u16,
    child_stable_id: [u8; 16],
    first_owner_stable_id: [u8; 16],
    second_owner_stable_id: [u8; 16],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DuplicateOwnerSpec {
    child: EntityRef,
    first_owner: EntityRef,
    second_owner: EntityRef,
    relation_sequence_ordinal: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingOrder {
    Absent,
    Explicit(u32),
    OwnerPayload,
    Gate {
        transition_index: u32,
        stable_id: [u8; 16],
    },
    Waiting {
        entry_transition_index: u32,
        release_transition_index: u32,
        stable_id: [u8; 16],
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BoundedSemanticRecord {
    pub(crate) record_kind: u16,
    pub(crate) entity_kind_code: u16,
    pub(crate) stable_id: [u8; 16],
    pub(crate) owner_ordinal: u32,
    pub(crate) local_index: u32,
    pub(crate) payload: PayloadRange,
    pending_order: PendingOrder,
}

#[derive(Debug)]
pub(crate) struct BoundedTemplateExecution {
    pub(crate) workload_id: ScalableWorkloadId,
    pub(crate) graph_profile: GraphProfileId,
    pub(crate) n: u32,
    buffers: BoundedTemplateBufferPool,
    plan: ScalableStagePlanSummary,
    candidate_pipeline_checksums: CandidatePipelineChecksums,
}

#[derive(Debug)]
pub(crate) struct BoundedTemplateExecutionFailure {
    source: StageGenerationError,
    buffers: BoundedTemplateBufferPool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoundedTemplateFailureMode {
    MissingReferencePerUnit { maximum_diagnostics: u64 },
    DuplicateOwnerPerUnit,
    DiagnosticCapPlusOne { maximum_diagnostics: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BoundedTemplateFailureObservation {
    pub(crate) diagnostic_count: u64,
    pub(crate) diagnostics_truncated: bool,
    pub(crate) diagnostic_digest_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum BoundedTemplatePopulationOutcome {
    Success(CandidatePipelineChecksums),
    ExpectedFailure(BoundedTemplateFailureObservation),
}

#[derive(Debug)]
struct FixedKeyLookup {
    _reservation: ControlledTransientReservation,
    inner: FixedKeyLookupInner,
}

#[derive(Debug)]
enum FixedKeyLookupInner {
    Std(HashMap<u128, usize, RandomState>),
    Sorted(Vec<(u128, usize)>),
    #[cfg(feature = "candidate-hashbrown-randomstate")]
    HashbrownRandom(hashbrown::HashMap<u128, usize, RandomState>),
    #[cfg(feature = "candidate-hashbrown-xxh3")]
    HashbrownXxh3(hashbrown::HashMap<u128, usize, xxhash_rust::xxh3::Xxh3Builder>),
    #[cfg(feature = "candidate-hashbrown-xxh64")]
    HashbrownXxh64(hashbrown::HashMap<u128, usize, xxhash_rust::xxh64::Xxh64Builder>),
    #[cfg(feature = "candidate-hashbrown-fnv1a64")]
    HashbrownFnv(hashbrown::HashMap<u128, usize, Fnv1a64BuildHasher>),
    #[cfg(feature = "candidate-indexmap-randomstate")]
    IndexMap(indexmap::IndexMap<u128, usize, RandomState>),
}

impl FixedKeyLookup {
    fn get(&self, key: u128) -> Option<usize> {
        match &self.inner {
            FixedKeyLookupInner::Std(values) => values.get(&key).copied(),
            FixedKeyLookupInner::Sorted(values) => values
                .binary_search_by_key(&key, |(candidate, _)| *candidate)
                .ok()
                .map(|index| values[index].1),
            #[cfg(feature = "candidate-hashbrown-randomstate")]
            FixedKeyLookupInner::HashbrownRandom(values) => values.get(&key).copied(),
            #[cfg(feature = "candidate-hashbrown-xxh3")]
            FixedKeyLookupInner::HashbrownXxh3(values) => values.get(&key).copied(),
            #[cfg(feature = "candidate-hashbrown-xxh64")]
            FixedKeyLookupInner::HashbrownXxh64(values) => values.get(&key).copied(),
            #[cfg(feature = "candidate-hashbrown-fnv1a64")]
            FixedKeyLookupInner::HashbrownFnv(values) => values.get(&key).copied(),
            #[cfg(feature = "candidate-indexmap-randomstate")]
            FixedKeyLookupInner::IndexMap(values) => values.get(&key).copied(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct BoundedTemplateBufferPool {
    allocator: ControlledAllocator,
    fields: ControlledVec<BoundedField>,
    field_payload: ControlledVec<u8>,
    declarations: ControlledVec<BoundedDeclaration>,
    canonical_identity_scratch: ControlledVec<u8>,
    owner_ordinal_scratch: ControlledVec<usize>,
    records: ControlledVec<BoundedSemanticRecord>,
    record_payload: ControlledVec<u8>,
    local_index_scratch: ControlledVec<usize>,
    failure_candidates: ControlledVec<BoundedFailureCandidate>,
    duplicate_owner_candidates: ControlledVec<BoundedDuplicateOwnerCandidate>,
    source_permutation_scratch: ControlledVec<u32>,
    source_spans: ControlledVec<SourceSpanRecord>,
    source_records: ControlledVec<TypedAstStageRecord>,
    source_payload: ControlledVec<u8>,
    typed_records: ControlledVec<TypedAstStageRecord>,
    typed_payload: ControlledVec<u8>,
    hir_records: ControlledVec<HirStageRecord>,
    hir_payload: ControlledVec<u8>,
    mir_records: ControlledVec<MirLirStageRecord>,
    mir_payload: ControlledVec<u8>,
    lir_records: ControlledVec<MirLirStageRecord>,
    lir_payload: ControlledVec<u8>,
    diagnostics: ControlledVec<u8>,
    scratch: ControlledVec<u64>,
    semantic_record_stream: ControlledVec<u8>,
    output: ControlledVec<u8>,
}

impl BoundedTemplateExecution {
    pub(crate) fn output_construction(&self) -> &[u8] {
        self.buffers.output.as_slice()
    }

    pub(crate) fn record_payload(&self, record: &BoundedSemanticRecord) -> &[u8] {
        &self.buffers.record_payload.as_slice()
            [record.payload.offset..record.payload.offset + record.payload.length]
    }

    pub(crate) fn records(&self) -> &[BoundedSemanticRecord] {
        self.buffers.records.as_slice()
    }

    pub(crate) fn semantic_record_stream(&self) -> &[u8] {
        self.buffers.semantic_record_stream.as_slice()
    }

    pub(crate) fn peak_live_requested_bytes(&self) -> u64 {
        self.buffers.peak_live_requested_bytes()
    }

    pub(crate) fn candidate_pipeline_checksums(&self) -> CandidatePipelineChecksums {
        self.candidate_pipeline_checksums
    }
}

impl BoundedTemplateBufferPool {
    pub(crate) fn new(allocator: ControlledAllocator) -> Self {
        Self {
            fields: ControlledVec::new("template identity fields", allocator.clone()),
            field_payload: ControlledVec::new("template identity field payload", allocator.clone()),
            declarations: ControlledVec::new("template declarations", allocator.clone()),
            canonical_identity_scratch: ControlledVec::new(
                "template canonical identity scratch",
                allocator.clone(),
            ),
            owner_ordinal_scratch: ControlledVec::new(
                "template owner-ordinal scratch",
                allocator.clone(),
            ),
            records: ControlledVec::new("template semantic records", allocator.clone()),
            record_payload: ControlledVec::new("template semantic payload", allocator.clone()),
            local_index_scratch: ControlledVec::new(
                "template local-index scratch",
                allocator.clone(),
            ),
            failure_candidates: ControlledVec::new(
                "template failure candidates",
                allocator.clone(),
            ),
            duplicate_owner_candidates: ControlledVec::new(
                "template duplicate-owner candidates",
                allocator.clone(),
            ),
            source_permutation_scratch: ControlledVec::new(
                "template source permutation scratch",
                allocator.clone(),
            ),
            source_spans: ControlledVec::new("template source spans", allocator.clone()),
            source_records: ControlledVec::new("template source records", allocator.clone()),
            source_payload: ControlledVec::new("template source payload", allocator.clone()),
            typed_records: ControlledVec::new("template typed AST records", allocator.clone()),
            typed_payload: ControlledVec::new("template typed AST payload", allocator.clone()),
            hir_records: ControlledVec::new("template HIR records", allocator.clone()),
            hir_payload: ControlledVec::new("template HIR payload", allocator.clone()),
            mir_records: ControlledVec::new("template MIR records", allocator.clone()),
            mir_payload: ControlledVec::new("template MIR payload", allocator.clone()),
            lir_records: ControlledVec::new("template canonical LIR records", allocator.clone()),
            lir_payload: ControlledVec::new("template canonical LIR payload", allocator.clone()),
            diagnostics: ControlledVec::new("template diagnostics", allocator.clone()),
            scratch: ControlledVec::new("template scratch", allocator.clone()),
            semantic_record_stream: ControlledVec::new(
                "template semantic record stream",
                allocator.clone(),
            ),
            output: ControlledVec::new("template output construction", allocator.clone()),
            allocator,
        }
    }

    pub(crate) fn begin_request(&self) -> Result<(), StageGenerationError> {
        if !self.all_lengths_are_zero() {
            return Err(StageGenerationError::MaterializedMismatch(
                "template retained semantic state",
            ));
        }
        self.allocator.begin_request()
    }

    pub(crate) fn peak_live_requested_bytes(&self) -> u64 {
        self.allocator.observation().peak_live_requested_bytes
    }

    pub(crate) fn allocation_snapshot(&self) -> ControlledAllocationSnapshot {
        self.allocator.snapshot()
    }

    /// 丢弃一次意外失败中已经增长的阶段容量，同时保留同一受控分配账本和硬上限。
    ///
    /// 正常成功与预期语义失败继续复用容量；只有没有形成结构化预期结果的执行错误才走
    /// 此路径，避免较大规模的部分增长污染随后仍属合法的较小规模请求。
    pub(crate) fn reset_after_unexpected_failure(self) -> Result<Self, StageGenerationError> {
        let allocator = self.allocator.clone();
        drop(self);
        if allocator.observation().live_requested_bytes != 0 {
            return Err(StageGenerationError::MaterializedMismatch(
                "template failed-request live allocation residue",
            ));
        }
        Ok(Self::new(allocator))
    }

    pub(crate) fn retained_capacity_bytes(
        &self,
    ) -> Result<StageRetainedCapacityBytes, StageGenerationError> {
        let source_input = capacity_sum(&[
            self.source_spans.accounted_capacity_bytes(),
            self.source_records.accounted_capacity_bytes(),
            self.source_payload.accounted_capacity_bytes(),
        ])?;
        let typed_ast = capacity_sum(&[
            self.typed_records.accounted_capacity_bytes(),
            self.typed_payload.accounted_capacity_bytes(),
        ])?;
        let hir = capacity_sum(&[
            self.hir_records.accounted_capacity_bytes(),
            self.hir_payload.accounted_capacity_bytes(),
        ])?;
        let mir = capacity_sum(&[
            self.records.accounted_capacity_bytes(),
            self.record_payload.accounted_capacity_bytes(),
            self.mir_records.accounted_capacity_bytes(),
            self.mir_payload.accounted_capacity_bytes(),
        ])?;
        let canonical_lir = capacity_sum(&[
            self.lir_records.accounted_capacity_bytes(),
            self.lir_payload.accounted_capacity_bytes(),
        ])?;
        let diagnostics = self.diagnostics.accounted_capacity_bytes();
        let scratch = capacity_sum(&[
            self.fields.accounted_capacity_bytes(),
            self.field_payload.accounted_capacity_bytes(),
            self.declarations.accounted_capacity_bytes(),
            self.canonical_identity_scratch.accounted_capacity_bytes(),
            self.owner_ordinal_scratch.accounted_capacity_bytes(),
            self.local_index_scratch.accounted_capacity_bytes(),
            self.failure_candidates.accounted_capacity_bytes(),
            self.duplicate_owner_candidates.accounted_capacity_bytes(),
            self.source_permutation_scratch.accounted_capacity_bytes(),
            self.scratch.accounted_capacity_bytes(),
        ])?;
        let output_construction = capacity_sum(&[
            self.semantic_record_stream.accounted_capacity_bytes(),
            self.output.accounted_capacity_bytes(),
        ])?;
        let total = capacity_sum(&[
            source_input,
            typed_ast,
            hir,
            mir,
            canonical_lir,
            diagnostics,
            scratch,
            output_construction,
        ])?;
        Ok(StageRetainedCapacityBytes {
            source_input,
            typed_ast,
            hir,
            mir,
            canonical_lir,
            diagnostics,
            scratch,
            output_construction,
            total,
        })
    }

    fn clear_all(&mut self) {
        self.fields.clear();
        self.field_payload.clear();
        self.declarations.clear();
        self.canonical_identity_scratch.clear();
        self.owner_ordinal_scratch.clear();
        self.records.clear();
        self.record_payload.clear();
        self.local_index_scratch.clear();
        self.failure_candidates.clear();
        self.duplicate_owner_candidates.clear();
        self.source_permutation_scratch.clear();
        self.source_spans.clear();
        self.source_records.clear();
        self.source_payload.clear();
        self.typed_records.clear();
        self.typed_payload.clear();
        self.hir_records.clear();
        self.hir_payload.clear();
        self.mir_records.clear();
        self.mir_payload.clear();
        self.lir_records.clear();
        self.lir_payload.clear();
        self.diagnostics.clear();
        self.scratch.clear();
        self.semantic_record_stream.clear();
        self.output.clear();
    }

    fn all_lengths_are_zero(&self) -> bool {
        self.fields.len() == 0
            && self.field_payload.len() == 0
            && self.declarations.len() == 0
            && self.canonical_identity_scratch.len() == 0
            && self.owner_ordinal_scratch.len() == 0
            && self.records.len() == 0
            && self.record_payload.len() == 0
            && self.local_index_scratch.len() == 0
            && self.failure_candidates.len() == 0
            && self.duplicate_owner_candidates.len() == 0
            && self.source_permutation_scratch.len() == 0
            && self.source_spans.len() == 0
            && self.source_records.len() == 0
            && self.source_payload.len() == 0
            && self.typed_records.len() == 0
            && self.typed_payload.len() == 0
            && self.hir_records.len() == 0
            && self.hir_payload.len() == 0
            && self.mir_records.len() == 0
            && self.mir_payload.len() == 0
            && self.lir_records.len() == 0
            && self.lir_payload.len() == 0
            && self.diagnostics.len() == 0
            && self.scratch.len() == 0
            && self.semantic_record_stream.len() == 0
            && self.output.len() == 0
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_bounded_template_stage_case(
    generator: &GeneratorContract,
    identity: &IdentityContract,
    stage: &StageContract,
    workload_id: ScalableWorkloadId,
    template: &CorridorTemplate,
    graph_profile: GraphProfileId,
    n: u32,
    plan: &ScalableStagePlanSummary,
    allocator: ControlledAllocator,
) -> Result<BoundedTemplateExecution, StageGenerationError> {
    execute_bounded_template_stage_case_with_pool(
        generator,
        identity,
        stage,
        workload_id,
        template,
        graph_profile,
        n,
        plan,
        BoundedTemplateBufferPool::new(allocator),
    )
    .map_err(|failure| failure.into_source())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_bounded_template_stage_case_with_pool(
    generator: &GeneratorContract,
    identity: &IdentityContract,
    stage: &StageContract,
    workload_id: ScalableWorkloadId,
    template: &CorridorTemplate,
    graph_profile: GraphProfileId,
    n: u32,
    plan: &ScalableStagePlanSummary,
    buffers: BoundedTemplateBufferPool,
) -> Result<BoundedTemplateExecution, Box<BoundedTemplateExecutionFailure>> {
    execute_bounded_template_stage_case_with_pool_and_candidate(
        generator,
        identity,
        stage,
        workload_id,
        template,
        graph_profile,
        n,
        plan,
        buffers,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_bounded_template_stage_case_with_pool_and_candidate(
    generator: &GeneratorContract,
    identity: &IdentityContract,
    stage: &StageContract,
    workload_id: ScalableWorkloadId,
    template: &CorridorTemplate,
    graph_profile: GraphProfileId,
    n: u32,
    plan: &ScalableStagePlanSummary,
    mut buffers: BoundedTemplateBufferPool,
    candidate_configuration: Option<&CandidatePipelineConfiguration>,
) -> Result<BoundedTemplateExecution, Box<BoundedTemplateExecutionFailure>> {
    match populate_bounded_template_stage_case(
        generator,
        identity,
        stage,
        workload_id,
        template,
        graph_profile,
        n,
        plan,
        &mut buffers,
        None,
        candidate_configuration,
    ) {
        Ok(BoundedTemplatePopulationOutcome::Success(candidate_pipeline_checksums)) => {
            Ok(BoundedTemplateExecution {
                workload_id,
                graph_profile,
                n,
                buffers,
                plan: plan.clone(),
                candidate_pipeline_checksums,
            })
        }
        Ok(BoundedTemplatePopulationOutcome::ExpectedFailure(_)) => {
            buffers.clear_all();
            Err(Box::new(BoundedTemplateExecutionFailure {
                source: StageGenerationError::MaterializedMismatch(
                    "unexpected bounded template failure outcome",
                ),
                buffers,
            }))
        }
        Err(source) => {
            buffers.clear_all();
            Err(Box::new(BoundedTemplateExecutionFailure {
                source,
                buffers,
            }))
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_bounded_template_failure_case_with_pool(
    generator: &GeneratorContract,
    identity: &IdentityContract,
    stage: &StageContract,
    workload_id: ScalableWorkloadId,
    template: &CorridorTemplate,
    graph_profile: GraphProfileId,
    n: u32,
    plan: &ScalableStagePlanSummary,
    failure_mode: BoundedTemplateFailureMode,
    mut buffers: BoundedTemplateBufferPool,
) -> Result<
    (BoundedTemplateFailureObservation, BoundedTemplateBufferPool),
    Box<BoundedTemplateExecutionFailure>,
> {
    let outcome = populate_bounded_template_stage_case(
        generator,
        identity,
        stage,
        workload_id,
        template,
        graph_profile,
        n,
        plan,
        &mut buffers,
        Some(failure_mode),
        None,
    );
    let observation = match outcome {
        Ok(BoundedTemplatePopulationOutcome::ExpectedFailure(observation)) => observation,
        Ok(BoundedTemplatePopulationOutcome::Success(_)) => {
            buffers.clear_all();
            return Err(Box::new(BoundedTemplateExecutionFailure {
                source: StageGenerationError::MaterializedMismatch(
                    "bounded template failure input unexpectedly succeeded",
                ),
                buffers,
            }));
        }
        Err(source) => {
            buffers.clear_all();
            return Err(Box::new(BoundedTemplateExecutionFailure {
                source,
                buffers,
            }));
        }
    };
    buffers.clear_all();
    if !buffers.all_lengths_are_zero() {
        return Err(Box::new(BoundedTemplateExecutionFailure {
            source: StageGenerationError::MaterializedMismatch(
                "bounded template failure retained semantic state",
            ),
            buffers,
        }));
    }
    Ok((observation, buffers))
}

impl BoundedTemplateExecutionFailure {
    pub(crate) fn into_parts(self) -> (StageGenerationError, BoundedTemplateBufferPool) {
        (self.source, self.buffers)
    }

    fn into_source(self) -> StageGenerationError {
        self.source
    }
}

fn selected_candidate_id(
    configuration: Option<&CandidatePipelineConfiguration>,
    key_domain: CandidateKeyDomain,
) -> &str {
    configuration.map_or_else(
        || match key_domain {
            CandidateKeyDomain::ExternalString | CandidateKeyDomain::ValidatedFixedKey => {
                "std-hashmap-randomstate-v1"
            }
            CandidateKeyDomain::CanonicalOutputOrder => "stable-vec-sort-v1",
            CandidateKeyDomain::FullPipelineBaseline => "baseline-std-randomstate-stable-vec-v1",
        },
        |configuration| configuration.candidate_id(key_domain),
    )
}

fn candidate_transient_bytes(
    item_count: usize,
    bytes_per_item: usize,
    fixed_bytes: usize,
    field: &'static str,
) -> Result<u64, StageGenerationError> {
    let bytes = item_count
        .checked_mul(bytes_per_item)
        .and_then(|bytes| bytes.checked_add(fixed_bytes))
        .ok_or(StageGenerationError::Overflow(field))?;
    u64::try_from(bytes).map_err(|_| StageGenerationError::Overflow(field))
}

fn exercise_external_string_table(
    candidate_id: &str,
    fields: &[BoundedField],
    payload: &[u8],
    allocator: &ControlledAllocator,
) -> Result<u64, StageGenerationError> {
    let reserved_bytes = candidate_transient_bytes(
        fields.len(),
        128,
        payload.len(),
        "candidate external-string reservation",
    )?;
    let _reservation =
        allocator.reserve_transient("candidate external-string container", reserved_bytes)?;
    let key_at = |field: &BoundedField| {
        &payload[field.bytes.offset..field.bytes.offset + field.bytes.length]
    };
    let mut checksum = 0_u64;
    match candidate_id {
        "std-hashmap-randomstate-v1" => {
            let mut table = HashMap::<&[u8], u32, RandomState>::with_capacity_and_hasher(
                fields.len(),
                RandomState::new(),
            );
            for (ordinal, field) in fields.iter().enumerate() {
                let ordinal = u32::try_from(ordinal)
                    .map_err(|_| StageGenerationError::Overflow("external string ordinal"))?;
                table.entry(key_at(field)).or_insert(ordinal);
            }
            for field in fields {
                checksum = mix_candidate_checksum(
                    checksum,
                    u64::from(*table.get(key_at(field)).ok_or(
                        StageGenerationError::MaterializedMismatch("external string lookup"),
                    )?),
                );
            }
        }
        "hashbrown-randomstate-v1" => {
            #[cfg(feature = "candidate-hashbrown-randomstate")]
            {
                let mut table =
                    hashbrown::HashMap::<&[u8], u32, RandomState>::with_capacity_and_hasher(
                        fields.len(),
                        RandomState::new(),
                    );
                for (ordinal, field) in fields.iter().enumerate() {
                    let ordinal = u32::try_from(ordinal)
                        .map_err(|_| StageGenerationError::Overflow("external string ordinal"))?;
                    table.entry(key_at(field)).or_insert(ordinal);
                }
                for field in fields {
                    checksum = mix_candidate_checksum(
                        checksum,
                        u64::from(*table.get(key_at(field)).ok_or(
                            StageGenerationError::MaterializedMismatch("external string lookup"),
                        )?),
                    );
                }
            }
            #[cfg(not(feature = "candidate-hashbrown-randomstate"))]
            return Err(StageGenerationError::MaterializedMismatch(
                "hashbrown random-state feature unavailable",
            ));
        }
        "sorted-vec-binary-search-v1" => {
            let mut table = fields
                .iter()
                .enumerate()
                .map(|(ordinal, field)| {
                    u32::try_from(ordinal)
                        .map(|ordinal| (key_at(field), ordinal))
                        .map_err(|_| StageGenerationError::Overflow("external string ordinal"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            table.sort_by(|left, right| left.0.cmp(right.0).then(left.1.cmp(&right.1)));
            for field in fields {
                let key = key_at(field);
                let index = table.partition_point(|(candidate, _)| *candidate < key);
                let value = table
                    .get(index)
                    .filter(|(candidate, _)| *candidate == key)
                    .map(|(_, value)| *value)
                    .ok_or(StageGenerationError::MaterializedMismatch(
                        "external sorted-vector lookup",
                    ))?;
                checksum = mix_candidate_checksum(checksum, u64::from(value));
            }
        }
        _ => {
            return Err(StageGenerationError::MaterializedMismatch(
                "unsupported external-string candidate",
            ));
        }
    }
    Ok(checksum)
}

fn fixed_key(owner: UnitEntityRef) -> u128 {
    (u128::from(owner.unit) << 64)
        | (u128::from(owner.entity.kind) << 32)
        | u128::from(owner.entity.local)
}

fn build_fixed_key_lookup(
    candidate_id: &str,
    declarations: &[BoundedDeclaration],
    allocator: &ControlledAllocator,
) -> Result<(FixedKeyLookup, u64), StageGenerationError> {
    let reserved_bytes =
        candidate_transient_bytes(declarations.len(), 96, 0, "candidate fixed-key reservation")?;
    let reservation =
        allocator.reserve_transient("candidate validated-fixed-key container", reserved_bytes)?;
    let inner = match candidate_id {
        "std-hashmap-randomstate-v1" => {
            let mut table =
                HashMap::with_capacity_and_hasher(declarations.len(), RandomState::new());
            for (index, declaration) in declarations.iter().enumerate() {
                if table.insert(fixed_key(declaration.owner), index).is_some() {
                    return Err(StageGenerationError::MaterializedMismatch(
                        "duplicate validated fixed key",
                    ));
                }
            }
            FixedKeyLookupInner::Std(table)
        }
        "sorted-vec-binary-search-v1" => {
            let mut values = declarations
                .iter()
                .enumerate()
                .map(|(index, declaration)| (fixed_key(declaration.owner), index))
                .collect::<Vec<_>>();
            values.sort_unstable_by_key(|(key, _)| *key);
            if values.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                return Err(StageGenerationError::MaterializedMismatch(
                    "duplicate validated fixed key",
                ));
            }
            FixedKeyLookupInner::Sorted(values)
        }
        "hashbrown-randomstate-v1" => {
            #[cfg(feature = "candidate-hashbrown-randomstate")]
            {
                let mut table = hashbrown::HashMap::with_capacity_and_hasher(
                    declarations.len(),
                    RandomState::new(),
                );
                for (index, declaration) in declarations.iter().enumerate() {
                    if table.insert(fixed_key(declaration.owner), index).is_some() {
                        return Err(StageGenerationError::MaterializedMismatch(
                            "duplicate validated fixed key",
                        ));
                    }
                }
                FixedKeyLookupInner::HashbrownRandom(table)
            }
            #[cfg(not(feature = "candidate-hashbrown-randomstate"))]
            return Err(StageGenerationError::MaterializedMismatch(
                "hashbrown random-state feature unavailable",
            ));
        }
        "hashbrown-xxh3-fixed-v1" => {
            #[cfg(feature = "candidate-hashbrown-xxh3")]
            {
                let mut table = hashbrown::HashMap::with_capacity_and_hasher(
                    declarations.len(),
                    xxhash_rust::xxh3::Xxh3Builder::new().with_seed(FIXED_HASHER_SEED),
                );
                for (index, declaration) in declarations.iter().enumerate() {
                    if table.insert(fixed_key(declaration.owner), index).is_some() {
                        return Err(StageGenerationError::MaterializedMismatch(
                            "duplicate validated fixed key",
                        ));
                    }
                }
                FixedKeyLookupInner::HashbrownXxh3(table)
            }
            #[cfg(not(feature = "candidate-hashbrown-xxh3"))]
            return Err(StageGenerationError::MaterializedMismatch(
                "xxh3 feature unavailable",
            ));
        }
        "hashbrown-xxh64-fixed-v1" => {
            #[cfg(feature = "candidate-hashbrown-xxh64")]
            {
                let mut table = hashbrown::HashMap::with_capacity_and_hasher(
                    declarations.len(),
                    xxhash_rust::xxh64::Xxh64Builder::new(FIXED_HASHER_SEED),
                );
                for (index, declaration) in declarations.iter().enumerate() {
                    if table.insert(fixed_key(declaration.owner), index).is_some() {
                        return Err(StageGenerationError::MaterializedMismatch(
                            "duplicate validated fixed key",
                        ));
                    }
                }
                FixedKeyLookupInner::HashbrownXxh64(table)
            }
            #[cfg(not(feature = "candidate-hashbrown-xxh64"))]
            return Err(StageGenerationError::MaterializedMismatch(
                "xxh64 feature unavailable",
            ));
        }
        "hashbrown-fnv1a64-v1" => {
            #[cfg(feature = "candidate-hashbrown-fnv1a64")]
            {
                let mut table = hashbrown::HashMap::with_capacity_and_hasher(
                    declarations.len(),
                    Fnv1a64BuildHasher,
                );
                for (index, declaration) in declarations.iter().enumerate() {
                    if table.insert(fixed_key(declaration.owner), index).is_some() {
                        return Err(StageGenerationError::MaterializedMismatch(
                            "duplicate validated fixed key",
                        ));
                    }
                }
                FixedKeyLookupInner::HashbrownFnv(table)
            }
            #[cfg(not(feature = "candidate-hashbrown-fnv1a64"))]
            return Err(StageGenerationError::MaterializedMismatch(
                "fnv1a64 feature unavailable",
            ));
        }
        "indexmap-randomstate-v1" => {
            #[cfg(feature = "candidate-indexmap-randomstate")]
            {
                let mut table = indexmap::IndexMap::with_capacity_and_hasher(
                    declarations.len(),
                    RandomState::new(),
                );
                for (index, declaration) in declarations.iter().enumerate() {
                    if table.insert(fixed_key(declaration.owner), index).is_some() {
                        return Err(StageGenerationError::MaterializedMismatch(
                            "duplicate validated fixed key",
                        ));
                    }
                }
                FixedKeyLookupInner::IndexMap(table)
            }
            #[cfg(not(feature = "candidate-indexmap-randomstate"))]
            return Err(StageGenerationError::MaterializedMismatch(
                "indexmap feature unavailable",
            ));
        }
        _ => {
            return Err(StageGenerationError::MaterializedMismatch(
                "unsupported validated-fixed-key candidate",
            ));
        }
    };
    let lookup = FixedKeyLookup {
        _reservation: reservation,
        inner,
    };
    let mut checksum = 0_u64;
    for declaration in declarations {
        let index = lookup.get(fixed_key(declaration.owner)).ok_or(
            StageGenerationError::MaterializedMismatch("validated fixed-key lookup"),
        )?;
        checksum = mix_candidate_checksum(
            checksum,
            u64::try_from(index)
                .map_err(|_| StageGenerationError::Overflow("fixed-key lookup index"))?,
        );
    }
    Ok((lookup, checksum))
}

fn mix_candidate_checksum(state: u64, value: u64) -> u64 {
    state
        .rotate_left(11)
        .wrapping_add(value ^ 0x9e37_79b9_7f4a_7c15)
        .wrapping_mul(0xbf58_476d_1ce4_e5b9)
}

fn order_canonical_records(
    candidate_id: &str,
    records: &mut [BoundedSemanticRecord],
    payload: &[u8],
    allocator: &ControlledAllocator,
) -> Result<u64, StageGenerationError> {
    let reserved_bytes = candidate_transient_bytes(
        records.len(),
        std::mem::size_of::<BoundedSemanticRecord>() * 2 + std::mem::size_of::<usize>(),
        257 * std::mem::size_of::<usize>(),
        "candidate canonical-order reservation",
    )?;
    let _reservation =
        allocator.reserve_transient("candidate canonical-output-order scratch", reserved_bytes)?;
    match candidate_id {
        "stable-vec-sort-v1" => {
            records.sort_by(|left, right| canonical_record_compare(left, right, payload));
        }
        "deterministic-radix-sort-v1" => {
            deterministic_radix_sort_records(records, payload)?;
        }
        "deterministic-bucket-sort-v1" => {
            deterministic_bucket_sort_records(records, payload)?;
        }
        _ => {
            return Err(StageGenerationError::MaterializedMismatch(
                "unsupported canonical-output-order candidate",
            ));
        }
    }
    if records
        .windows(2)
        .any(|pair| canonical_record_compare(&pair[0], &pair[1], payload) == Ordering::Greater)
    {
        return Err(StageGenerationError::MaterializedMismatch(
            "candidate canonical output order",
        ));
    }
    let mut checksum = 0_u64;
    for record in records {
        checksum = mix_candidate_checksum(checksum, u64::from(record.record_kind));
        checksum = mix_candidate_checksum(checksum, u64::from(record.entity_kind_code));
        checksum = mix_candidate_checksum(
            checksum,
            u64::from_be_bytes(record.stable_id[..8].try_into().expect("stable id half")),
        );
        checksum = mix_candidate_checksum(
            checksum,
            u64::from_be_bytes(record.stable_id[8..].try_into().expect("stable id half")),
        );
        checksum = mix_candidate_checksum(checksum, u64::from(record.owner_ordinal));
        checksum = mix_candidate_checksum(checksum, u64::from(record.local_index));
        checksum = mix_candidate_checksum(
            checksum,
            u64::try_from(record.payload.length)
                .map_err(|_| StageGenerationError::Overflow("canonical payload length"))?,
        );
    }
    Ok(checksum)
}

fn deterministic_radix_sort_records(
    records: &mut [BoundedSemanticRecord],
    payload: &[u8],
) -> Result<(), StageGenerationError> {
    if records.len() < 2 {
        return Ok(());
    }
    let mut scratch = vec![records[0]; records.len()];
    radix_sort_record_range(records, &mut scratch, payload, 0, records.len(), 0)
}

fn radix_sort_record_range(
    records: &mut [BoundedSemanticRecord],
    scratch: &mut [BoundedSemanticRecord],
    payload: &[u8],
    start: usize,
    end: usize,
    depth: usize,
) -> Result<(), StageGenerationError> {
    if end.saturating_sub(start) < 2 {
        return Ok(());
    }
    let mut counts = [0_usize; 257];
    for record in &records[start..end] {
        counts[canonical_record_symbol(record, payload, depth)] += 1;
    }
    let non_empty = counts.iter().filter(|count| **count != 0).count();
    if non_empty == 1 {
        let symbol = counts
            .iter()
            .position(|count| *count != 0)
            .expect("one non-empty radix bucket");
        if symbol != 0 {
            radix_sort_record_range(records, scratch, payload, start, end, depth + 1)?;
        }
        return Ok(());
    }
    let mut offsets = [0_usize; 257];
    let mut cursor = start;
    for (offset, count) in offsets.iter_mut().zip(counts) {
        *offset = cursor;
        cursor = cursor
            .checked_add(count)
            .ok_or(StageGenerationError::Overflow("radix bucket offset"))?;
    }
    let starts = offsets;
    for record in &records[start..end] {
        let symbol = canonical_record_symbol(record, payload, depth);
        scratch[offsets[symbol]] = *record;
        offsets[symbol] += 1;
    }
    records[start..end].copy_from_slice(&scratch[start..end]);
    for symbol in 1..257 {
        let bucket_start = starts[symbol];
        let bucket_end = bucket_start + counts[symbol];
        if bucket_end.saturating_sub(bucket_start) > 1 {
            radix_sort_record_range(
                records,
                scratch,
                payload,
                bucket_start,
                bucket_end,
                depth + 1,
            )?;
        }
    }
    Ok(())
}

fn canonical_record_symbol(record: &BoundedSemanticRecord, payload: &[u8], depth: usize) -> usize {
    const FIXED_BYTES: usize = 28;
    let fixed = if depth < 2 {
        Some(record.record_kind.to_be_bytes()[depth])
    } else if depth < 4 {
        Some(record.entity_kind_code.to_be_bytes()[depth - 2])
    } else if depth < 20 {
        Some(record.stable_id[depth - 4])
    } else if depth < 24 {
        Some(record.owner_ordinal.to_be_bytes()[depth - 20])
    } else if depth < FIXED_BYTES {
        Some(record.local_index.to_be_bytes()[depth - 24])
    } else {
        None
    };
    if let Some(byte) = fixed {
        return usize::from(byte) + 1;
    }
    let payload_index = depth - FIXED_BYTES;
    if payload_index >= record.payload.length {
        0
    } else {
        usize::from(payload[record.payload.offset + payload_index]) + 1
    }
}

fn deterministic_bucket_sort_records(
    records: &mut [BoundedSemanticRecord],
    payload: &[u8],
) -> Result<(), StageGenerationError> {
    if records.len() < 2 {
        return Ok(());
    }
    let mut counts = vec![0_usize; usize::from(u16::MAX) + 1];
    for record in records.iter() {
        counts[usize::from(record.record_kind)] += 1;
    }
    let mut starts = vec![0_usize; counts.len()];
    let mut cursor = 0_usize;
    for (start, count) in starts.iter_mut().zip(&counts) {
        *start = cursor;
        cursor = cursor
            .checked_add(*count)
            .ok_or(StageGenerationError::Overflow("bucket sort offset"))?;
    }
    let mut offsets = starts.clone();
    let mut scratch = vec![records[0]; records.len()];
    for record in records.iter() {
        let bucket = usize::from(record.record_kind);
        scratch[offsets[bucket]] = *record;
        offsets[bucket] += 1;
    }
    records.copy_from_slice(&scratch);
    for (start, count) in starts.into_iter().zip(counts) {
        if count > 1 {
            records[start..start + count]
                .sort_by(|left, right| canonical_record_compare(left, right, payload));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn populate_bounded_template_stage_case(
    generator: &GeneratorContract,
    identity: &IdentityContract,
    stage: &StageContract,
    workload_id: ScalableWorkloadId,
    template: &CorridorTemplate,
    graph_profile: GraphProfileId,
    n: u32,
    plan: &ScalableStagePlanSummary,
    buffers: &mut BoundedTemplateBufferPool,
    failure_mode: Option<BoundedTemplateFailureMode>,
    candidate_configuration: Option<&CandidatePipelineConfiguration>,
) -> Result<BoundedTemplatePopulationOutcome, StageGenerationError> {
    if n == 0 {
        return Err(StageGenerationError::ScaleMustBePositive);
    }
    if plan.workload_id != workload_id || plan.graph_profile != graph_profile || plan.n != n {
        return Err(StageGenerationError::MaterializedMismatch(
            "bounded template plan identity",
        ));
    }

    let entity_count = template.entities.len();
    let declaration_count = usize::try_from(plan.counts.identity_declaration_count)
        .map_err(|_| StageGenerationError::Overflow("bounded declaration count"))?;
    let field_count = usize::try_from(plan.counts.identity_field_occurrence_count)
        .map_err(|_| StageGenerationError::Overflow("bounded field count"))?;
    let semantic_record_count = usize::try_from(plan.counts.semantic_output_record)
        .map_err(|_| StageGenerationError::Overflow("bounded semantic record count"))?;
    let semantic_payload_bytes = usize::try_from(plan.counts.semantic_payload_byte_count)
        .map_err(|_| StageGenerationError::Overflow("bounded semantic payload bytes"))?;
    buffers.clear_all();

    buffers.fields.try_reserve(field_count)?;
    buffers.field_payload.try_reserve(checked_mul_usize(
        field_count,
        32,
        "bounded identity field payload upper",
    )?)?;
    buffers.declarations.try_reserve(declaration_count)?;
    buffers
        .canonical_identity_scratch
        .try_reserve(maximum_canonical_identity_bytes(identity, template)?)?;

    for unit in 0..n {
        let namespace =
            derive_namespace_ascii(generator, workload_id.as_str(), graph_profile, unit)?;
        for entity in &template.entities {
            let binding = identity
                .bindings
                .iter()
                .find(|binding| binding.entity_kind_code == entity.reference.kind)
                .ok_or(StageGenerationError::MissingEntityKind(
                    entity.reference.kind,
                ))?;
            let field_start = buffers.fields.len();
            let profiled_count = u32::try_from(
                binding
                    .fields
                    .iter()
                    .filter(|field| matches!(field.value, IdentityFieldValue::ProfiledKey { .. }))
                    .count(),
            )
            .map_err(|_| StageGenerationError::Overflow("profiled identity field count"))?;
            for field in &binding.fields {
                let payload_start = buffers.field_payload.len();
                match field.value {
                    IdentityFieldValue::Namespace => {
                        buffers.field_payload.try_extend_from_slice(&namespace)?;
                    }
                    IdentityFieldValue::ProfiledKey { kind, local } => {
                        let expanded_local = entity
                            .reference
                            .local
                            .checked_mul(profiled_count)
                            .and_then(|base| base.checked_add(local))
                            .ok_or(StageGenerationError::Overflow(
                                "profiled identity local index",
                            ))?;
                        let mut bytes = [0_u8; 20];
                        write_hex_u16(kind, &mut bytes[0..2]);
                        bytes[2] = b'/';
                        write_hex_u32(unit, &mut bytes[3..11]);
                        bytes[11] = b'/';
                        write_hex_u32(expanded_local, &mut bytes[12..20]);
                        buffers.field_payload.try_extend_from_slice(&bytes)?;
                    }
                    IdentityFieldValue::StableId { kind, .. } => {
                        let target = entity.identity_references.get(&field.tag).copied().ok_or(
                            StageGenerationError::MaterializedMismatch(
                                "template identity reference tag",
                            ),
                        )?;
                        if target.kind != kind {
                            return Err(StageGenerationError::MaterializedMismatch(
                                "template identity reference kind",
                            ));
                        }
                        let target_index = declaration_index(template, unit, target, entity_count)?;
                        let target_declaration =
                            buffers.declarations.as_slice().get(target_index).ok_or(
                                StageGenerationError::MaterializedMismatch(
                                    "template identity dependency order",
                                ),
                            )?;
                        buffers
                            .field_payload
                            .try_extend_from_slice(&target_declaration.stable_id)?;
                    }
                }
                buffers.fields.try_push(BoundedField {
                    tag: field.tag,
                    bytes: PayloadRange {
                        offset: payload_start,
                        length: buffers.field_payload.len() - payload_start,
                    },
                })?;
            }

            buffers.canonical_identity_scratch.clear();
            buffers
                .canonical_identity_scratch
                .try_extend_from_slice(IDENTITY_MAGIC)?;
            append_u16(
                &mut buffers.canonical_identity_scratch,
                identity.identity_encoding_version(),
            )?;
            append_u16(
                &mut buffers.canonical_identity_scratch,
                entity.reference.kind,
            )?;
            let entity_fields = &buffers.fields.as_slice()[field_start..buffers.fields.len()];
            append_u16(
                &mut buffers.canonical_identity_scratch,
                u16::try_from(entity_fields.len())
                    .map_err(|_| StageGenerationError::Overflow("identity field count"))?,
            )?;
            for field in entity_fields {
                append_u16(&mut buffers.canonical_identity_scratch, field.tag)?;
                append_u32(
                    &mut buffers.canonical_identity_scratch,
                    u32::try_from(field.bytes.length)
                        .map_err(|_| StageGenerationError::Overflow("identity field length"))?,
                )?;
                buffers.canonical_identity_scratch.try_extend_from_slice(
                    &buffers.field_payload.as_slice()
                        [field.bytes.offset..field.bytes.offset + field.bytes.length],
                )?;
            }
            let mut hasher = blake3::Hasher::new();
            hasher.update(STABLE_ID_DOMAIN);
            hasher.update(buffers.canonical_identity_scratch.as_slice());
            let mut stable_id = [0_u8; 16];
            stable_id.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
            buffers.declarations.try_push(BoundedDeclaration {
                owner: UnitEntityRef {
                    unit,
                    entity: entity.reference,
                },
                stable_id,
                owner_ordinal: 0,
                fields: PayloadRange {
                    offset: field_start,
                    length: entity_fields.len(),
                },
            })?;
        }
    }
    if buffers.declarations.len() != declaration_count || buffers.fields.len() != field_count {
        return Err(StageGenerationError::MaterializedMismatch(
            "bounded template declarations",
        ));
    }

    let mut candidate_pipeline_checksums = CandidatePipelineChecksums {
        external_string: exercise_external_string_table(
            selected_candidate_id(candidate_configuration, CandidateKeyDomain::ExternalString),
            buffers.fields.as_slice(),
            buffers.field_payload.as_slice(),
            &buffers.allocator,
        )?,
        ..CandidatePipelineChecksums::default()
    };

    assign_owner_ordinals(
        template,
        &mut buffers.declarations,
        &mut buffers.owner_ordinal_scratch,
    )?;
    let (fixed_key_lookup, fixed_key_checksum) = build_fixed_key_lookup(
        selected_candidate_id(
            candidate_configuration,
            CandidateKeyDomain::ValidatedFixedKey,
        ),
        buffers.declarations.as_slice(),
        &buffers.allocator,
    )?;
    candidate_pipeline_checksums.validated_fixed_key = fixed_key_checksum;

    buffers.records.try_reserve(semantic_record_count)?;
    buffers.record_payload.try_reserve(semantic_payload_bytes)?;
    let per_unit_record_count = entity_count
        .checked_add(template.relations.len())
        .and_then(|value| value.checked_add(template.geometry.len()))
        .ok_or(StageGenerationError::Overflow(
            "template per-unit semantic record count",
        ))?;
    buffers
        .local_index_scratch
        .try_reserve(per_unit_record_count)?;

    let duplicate_owner_spec = if matches!(
        failure_mode,
        Some(BoundedTemplateFailureMode::DuplicateOwnerPerUnit)
    ) {
        buffers.duplicate_owner_candidates.try_reserve(
            usize::try_from(n)
                .map_err(|_| StageGenerationError::Overflow("duplicate-owner unit count"))?,
        )?;
        Some(duplicate_owner_spec(template)?)
    } else {
        None
    };
    if matches!(
        failure_mode,
        Some(
            BoundedTemplateFailureMode::MissingReferencePerUnit { .. }
                | BoundedTemplateFailureMode::DiagnosticCapPlusOne { .. }
        )
    ) {
        let route_occurrences_per_unit = template
            .relations
            .iter()
            .filter(|relation| matches!(relation, TemplateRelation::RouteOccurrence { .. }))
            .count();
        buffers.failure_candidates.try_reserve(
            route_occurrences_per_unit
                .checked_mul(
                    usize::try_from(n)
                        .map_err(|_| StageGenerationError::Overflow("failure unit count"))?,
                )
                .ok_or(StageGenerationError::Overflow("failure candidate count"))?,
        )?;
    }
    for unit in 0..n {
        let unit_record_start = buffers.records.len();
        let declaration_start = usize::try_from(unit)
            .ok()
            .and_then(|unit| unit.checked_mul(entity_count))
            .ok_or(StageGenerationError::Overflow(
                "template declaration unit offset",
            ))?;
        for declaration in
            &buffers.declarations.as_slice()[declaration_start..declaration_start + entity_count]
        {
            let payload_start = buffers.record_payload.len();
            let declaration_fields = &buffers.fields.as_slice()
                [declaration.fields.offset..declaration.fields.offset + declaration.fields.length];
            append_u16(
                &mut buffers.record_payload,
                u16::try_from(declaration_fields.len())
                    .map_err(|_| StageGenerationError::Overflow("identity payload field count"))?,
            )?;
            for field in declaration_fields {
                append_u16(&mut buffers.record_payload, field.tag)?;
                append_u32(
                    &mut buffers.record_payload,
                    u32::try_from(field.bytes.length)
                        .map_err(|_| StageGenerationError::Overflow("identity payload field"))?,
                )?;
                buffers.record_payload.try_extend_from_slice(
                    &buffers.field_payload.as_slice()
                        [field.bytes.offset..field.bytes.offset + field.bytes.length],
                )?;
            }
            buffers.records.try_push(BoundedSemanticRecord {
                record_kind: 1,
                entity_kind_code: declaration.owner.entity.kind,
                stable_id: declaration.stable_id,
                owner_ordinal: declaration.owner_ordinal,
                local_index: ABSENT_LOCAL_INDEX,
                payload: PayloadRange {
                    offset: payload_start,
                    length: buffers.record_payload.len() - payload_start,
                },
                pending_order: PendingOrder::Absent,
            })?;
        }
        if let Some(spec) = duplicate_owner_spec {
            buffers
                .duplicate_owner_candidates
                .try_push(BoundedDuplicateOwnerCandidate {
                    unit,
                    relation_sequence_ordinal: spec.relation_sequence_ordinal,
                    child_kind: spec.child.kind,
                    child_stable_id: stable_id(
                        template,
                        buffers.declarations.as_slice(),
                        &fixed_key_lookup,
                        unit,
                        spec.child,
                    )?,
                    first_owner_stable_id: stable_id(
                        template,
                        buffers.declarations.as_slice(),
                        &fixed_key_lookup,
                        unit,
                        spec.first_owner,
                    )?,
                    second_owner_stable_id: stable_id(
                        template,
                        buffers.declarations.as_slice(),
                        &fixed_key_lookup,
                        unit,
                        spec.second_owner,
                    )?,
                })?;
        }
        for (relation_sequence_ordinal, relation) in template.relations.iter().enumerate() {
            let compiled = compile_relation(
                template,
                buffers.declarations.as_slice(),
                &fixed_key_lookup,
                unit,
                relation,
                &mut buffers.record_payload,
            )?;
            let record_index = buffers.records.len();
            buffers.records.try_push(compiled)?;
            if matches!(
                failure_mode,
                Some(
                    BoundedTemplateFailureMode::MissingReferencePerUnit { .. }
                        | BoundedTemplateFailureMode::DiagnosticCapPlusOne { .. }
                )
            ) && matches!(relation, TemplateRelation::RouteOccurrence { .. })
            {
                buffers
                    .failure_candidates
                    .try_push(BoundedFailureCandidate {
                        record_index,
                        unit,
                        relation_sequence_ordinal: u32::try_from(relation_sequence_ordinal)
                            .map_err(|_| {
                                StageGenerationError::Overflow("failure relation ordinal")
                            })?,
                        route_ordinal_within_unit: 0,
                        reference_ordinal: 0,
                        selected: false,
                    })?;
            }
        }
        for point in &template.geometry {
            let payload_start = buffers.record_payload.len();
            let frame = stable_id(
                template,
                buffers.declarations.as_slice(),
                &fixed_key_lookup,
                unit,
                point.frame,
            )?;
            buffers.record_payload.try_extend_from_slice(&frame)?;
            append_u32(&mut buffers.record_payload, point.point_index)?;
            let (x_bits, y_bits, z_bits) = geometry_coordinate_bits(point, unit)?;
            append_u32(&mut buffers.record_payload, x_bits)?;
            append_u32(&mut buffers.record_payload, y_bits)?;
            append_u32(&mut buffers.record_payload, z_bits)?;
            let owner = declaration(
                template,
                buffers.declarations.as_slice(),
                &fixed_key_lookup,
                unit,
                point.edge,
            )?;
            buffers.records.try_push(BoundedSemanticRecord {
                record_kind: 5,
                entity_kind_code: point.edge.kind,
                stable_id: owner.stable_id,
                owner_ordinal: owner.owner_ordinal,
                local_index: point.point_index,
                payload: PayloadRange {
                    offset: payload_start,
                    length: buffers.record_payload.len() - payload_start,
                },
                pending_order: PendingOrder::Explicit(point.point_index),
            })?;
        }
        let unit_record_end = buffers.records.len();
        assign_local_indexes(
            buffers.records.as_mut_slice(),
            buffers.record_payload.as_mut_slice(),
            unit_record_start,
            unit_record_end,
            &mut buffers.local_index_scratch,
        )?;
    }
    if let Some(failure_mode) = failure_mode {
        let observation =
            materialize_failure_diagnostics(template, graph_profile, n, failure_mode, buffers)?;
        let (full_candidate_count, maximum_diagnostics) = match failure_mode {
            BoundedTemplateFailureMode::MissingReferencePerUnit {
                maximum_diagnostics,
            } => (u64::from(n), maximum_diagnostics),
            BoundedTemplateFailureMode::DuplicateOwnerPerUnit => (u64::from(n), u64::from(n)),
            BoundedTemplateFailureMode::DiagnosticCapPlusOne {
                maximum_diagnostics,
            } => (
                u64::from(n)
                    .checked_add(1)
                    .ok_or(StageGenerationError::Overflow(
                        "diagnostic cap full candidate count",
                    ))?,
                maximum_diagnostics,
            ),
        };
        if observation.diagnostic_count != full_candidate_count.min(maximum_diagnostics)
            || observation.diagnostics_truncated != (full_candidate_count > maximum_diagnostics)
            || !buffers.semantic_record_stream.as_slice().is_empty()
            || !buffers.output.as_slice().is_empty()
        {
            return Err(StageGenerationError::MaterializedMismatch(
                "bounded template failure observation",
            ));
        }
        return Ok(BoundedTemplatePopulationOutcome::ExpectedFailure(
            observation,
        ));
    }
    if buffers.records.len() != semantic_record_count
        || buffers.record_payload.len() != semantic_payload_bytes
    {
        return Err(StageGenerationError::MaterializedMismatch(
            "bounded template semantic shape",
        ));
    }

    materialize_bounded_stage_prefix(
        generator,
        stage,
        template,
        graph_profile,
        n,
        plan,
        buffers.records.as_slice(),
        buffers.record_payload.as_slice(),
        &mut buffers.source_permutation_scratch,
        &mut buffers.source_spans,
        &mut buffers.source_records,
        &mut buffers.source_payload,
        &mut buffers.typed_records,
        &mut buffers.typed_payload,
        &mut buffers.hir_records,
        &mut buffers.hir_payload,
        &mut buffers.mir_records,
        &mut buffers.mir_payload,
        &mut buffers.diagnostics,
        &mut buffers.scratch,
    )?;
    candidate_pipeline_checksums.canonical_output_order = order_canonical_records(
        selected_candidate_id(
            candidate_configuration,
            CandidateKeyDomain::CanonicalOutputOrder,
        ),
        buffers.records.as_mut_slice(),
        buffers.record_payload.as_slice(),
        &buffers.allocator,
    )?;
    materialize_bounded_semantic_stage(
        buffers.records.as_slice(),
        buffers.record_payload.as_slice(),
        &mut buffers.lir_records,
        &mut buffers.lir_payload,
    )?;
    let output_bytes = usize::try_from(plan.counts.output_byte_count)
        .map_err(|_| StageGenerationError::Overflow("bounded output bytes"))?;
    buffers.semantic_record_stream.try_reserve(output_bytes)?;
    buffers
        .semantic_record_stream
        .try_extend_from_slice(identity.semantic_record_domain().as_bytes())?;
    buffers.semantic_record_stream.try_push(0)?;
    append_u32(
        &mut buffers.semantic_record_stream,
        identity.semantic_record_stream_version(),
    )?;
    append_u64(
        &mut buffers.semantic_record_stream,
        u64::try_from(buffers.records.len())
            .map_err(|_| StageGenerationError::Overflow("semantic record count"))?,
    )?;
    for record in &buffers.records {
        append_u16(&mut buffers.semantic_record_stream, record.record_kind)?;
        append_u16(&mut buffers.semantic_record_stream, record.entity_kind_code)?;
        buffers
            .semantic_record_stream
            .try_extend_from_slice(&record.stable_id)?;
        append_u32(&mut buffers.semantic_record_stream, record.owner_ordinal)?;
        append_u32(&mut buffers.semantic_record_stream, record.local_index)?;
        append_u64(
            &mut buffers.semantic_record_stream,
            u64::try_from(record.payload.length)
                .map_err(|_| StageGenerationError::Overflow("semantic payload length"))?,
        )?;
        buffers.semantic_record_stream.try_extend_from_slice(
            &buffers.record_payload.as_slice()
                [record.payload.offset..record.payload.offset + record.payload.length],
        )?;
    }
    if buffers.semantic_record_stream.len() != output_bytes {
        return Err(StageGenerationError::MaterializedMismatch(
            "bounded semantic stream bytes",
        ));
    }
    buffers
        .output
        .try_reserve(buffers.semantic_record_stream.len())?;
    buffers
        .output
        .try_extend_from_slice(buffers.semantic_record_stream.as_slice())?;
    Ok(BoundedTemplatePopulationOutcome::Success(
        candidate_pipeline_checksums,
    ))
}

fn duplicate_owner_spec(
    template: &CorridorTemplate,
) -> Result<DuplicateOwnerSpec, StageGenerationError> {
    let child_entity = template
        .entities
        .iter()
        .find(|entity| entity.reference.kind == 17)
        .ok_or(StageGenerationError::MaterializedMismatch(
            "duplicate-owner FacilityBand",
        ))?;
    let first_owner = child_entity
        .identity_references
        .values()
        .copied()
        .find(|target| target.kind == 1)
        .ok_or(StageGenerationError::MaterializedMismatch(
            "duplicate-owner first RoadCorridor",
        ))?;
    let second_owner = template
        .entities
        .iter()
        .filter(|entity| entity.reference.kind == 1)
        .nth(1)
        .map(|entity| entity.reference)
        .ok_or(StageGenerationError::MaterializedMismatch(
            "duplicate-owner second RoadCorridor",
        ))?;
    if first_owner == second_owner {
        return Err(StageGenerationError::MaterializedMismatch(
            "duplicate-owner distinct RoadCorridors",
        ));
    }
    let relation_sequence_ordinal = template
        .relations
        .iter()
        .position(|relation| {
            matches!(
                relation,
                TemplateRelation::Owner { child, parent }
                    if *child == child_entity.reference && *parent == first_owner
            )
        })
        .and_then(|ordinal| u32::try_from(ordinal).ok())
        .ok_or(StageGenerationError::MaterializedMismatch(
            "duplicate-owner source relation",
        ))?;
    Ok(DuplicateOwnerSpec {
        child: child_entity.reference,
        first_owner,
        second_owner,
        relation_sequence_ordinal,
    })
}

fn materialize_failure_diagnostics(
    template: &CorridorTemplate,
    graph_profile: GraphProfileId,
    n: u32,
    failure_mode: BoundedTemplateFailureMode,
    buffers: &mut BoundedTemplateBufferPool,
) -> Result<BoundedTemplateFailureObservation, StageGenerationError> {
    if failure_mode == BoundedTemplateFailureMode::DuplicateOwnerPerUnit {
        return materialize_duplicate_owner_diagnostics(template, graph_profile, n, buffers);
    }
    if buffers.failure_candidates.len() == 0 {
        return Err(StageGenerationError::MaterializedMismatch(
            "failure variant lacks route occurrences",
        ));
    }

    let records = &buffers.records;
    let record_payload = &buffers.record_payload;
    buffers.failure_candidates.sort_unstable_by(|left, right| {
        canonical_record_compare(
            &records.as_slice()[left.record_index],
            &records.as_slice()[right.record_index],
            record_payload.as_slice(),
        )
    });
    for (reference_ordinal, candidate) in buffers
        .failure_candidates
        .as_mut_slice()
        .iter_mut()
        .enumerate()
    {
        candidate.reference_ordinal = u64::try_from(reference_ordinal)
            .map_err(|_| StageGenerationError::Overflow("route reference ordinal"))?;
    }
    let unit_count = usize::try_from(n)
        .map_err(|_| StageGenerationError::Overflow("failure selected-unit count"))?;
    buffers.source_permutation_scratch.try_reserve(unit_count)?;
    buffers
        .source_permutation_scratch
        .try_resize(unit_count, 0)?;
    buffers.source_permutation_scratch.as_mut_slice().fill(0);
    for candidate in buffers.failure_candidates.as_mut_slice() {
        let route_ordinal = buffers
            .source_permutation_scratch
            .as_mut_slice()
            .get_mut(
                usize::try_from(candidate.unit)
                    .map_err(|_| StageGenerationError::Overflow("failure route unit index"))?,
            )
            .ok_or(StageGenerationError::MaterializedMismatch(
                "failure route unit range",
            ))?;
        candidate.route_ordinal_within_unit = *route_ordinal;
        *route_ordinal = route_ordinal
            .checked_add(1)
            .ok_or(StageGenerationError::Overflow(
                "failure route ordinal within unit",
            ))?;
    }

    let maximum_diagnostics = match failure_mode {
        BoundedTemplateFailureMode::MissingReferencePerUnit {
            maximum_diagnostics,
        } => {
            buffers.source_permutation_scratch.as_mut_slice().fill(0);
            for candidate in buffers.failure_candidates.as_mut_slice() {
                let selected = buffers
                    .source_permutation_scratch
                    .as_mut_slice()
                    .get_mut(usize::try_from(candidate.unit).map_err(|_| {
                        StageGenerationError::Overflow("failure selected unit index")
                    })?)
                    .ok_or(StageGenerationError::MaterializedMismatch(
                        "failure selected unit range",
                    ))?;
                if *selected == 0 {
                    candidate.selected = true;
                    *selected = 1;
                }
            }
            maximum_diagnostics
        }
        BoundedTemplateFailureMode::DuplicateOwnerPerUnit => {
            unreachable!("duplicate owner dispatched before route diagnostics")
        }
        BoundedTemplateFailureMode::DiagnosticCapPlusOne {
            maximum_diagnostics,
        } => {
            let candidate_count =
                maximum_diagnostics
                    .checked_add(1)
                    .ok_or(StageGenerationError::Overflow(
                        "diagnostic cap candidate count",
                    ))?;
            if u64::try_from(buffers.failure_candidates.len()).ok() < Some(candidate_count) {
                return Err(StageGenerationError::MaterializedMismatch(
                    "diagnostic cap failure lacks candidates",
                ));
            }
            for candidate in buffers.failure_candidates.as_mut_slice().iter_mut().take(
                usize::try_from(candidate_count).map_err(|_| {
                    StageGenerationError::Overflow("diagnostic cap candidate count")
                })?,
            ) {
                candidate.selected = true;
            }
            maximum_diagnostics
        }
    };

    buffers
        .failure_candidates
        .sort_unstable_by_key(|candidate| {
            (
                candidate.unit,
                candidate.relation_sequence_ordinal,
                candidate.reference_ordinal,
            )
        });
    let full_candidate_count = u64::try_from(
        buffers
            .failure_candidates
            .iter()
            .filter(|candidate| candidate.selected)
            .count(),
    )
    .map_err(|_| StageGenerationError::Overflow("failure full candidate count"))?;
    let retained_count = full_candidate_count.min(maximum_diagnostics);
    let diagnostics_truncated = full_candidate_count > maximum_diagnostics;

    buffers.diagnostics.try_reserve(
        usize::try_from(retained_count)
            .map_err(|_| StageGenerationError::Overflow("failure diagnostic count"))?
            .checked_mul(160)
            .and_then(|bytes| bytes.checked_add(64))
            .ok_or(StageGenerationError::Overflow(
                "failure diagnostic byte reserve",
            ))?,
    )?;
    buffers
        .diagnostics
        .try_extend_from_slice(crate::diagnostic::DIAGNOSTIC_STREAM_DOMAIN)?;
    append_u32(
        &mut buffers.diagnostics,
        crate::diagnostic::DIAGNOSTIC_STREAM_VERSION,
    )?;
    append_u64(&mut buffers.diagnostics, retained_count)?;

    let reference_count = template_source_reference_count(template)?;
    let source_prefix_length = "source/".len()
        + graph_profile.as_str().len()
        + 1
        + "unit/".len()
        + 8
        + ".lfsynthetic".len();
    let source_prefix_length = u32::try_from(source_prefix_length)
        .map_err(|_| StageGenerationError::Overflow("diagnostic source key length"))?;

    let mut emitted = 0_u64;
    for candidate in buffers
        .failure_candidates
        .iter()
        .filter(|candidate| candidate.selected)
    {
        if emitted == retained_count {
            break;
        }
        append_u32(
            &mut buffers.diagnostics,
            u32::try_from(UNKNOWN_REFERENCE_ERROR_CODE.len())
                .map_err(|_| StageGenerationError::Overflow("diagnostic code length"))?,
        )?;
        buffers
            .diagnostics
            .try_extend_from_slice(UNKNOWN_REFERENCE_ERROR_CODE.as_bytes())?;
        buffers.diagnostics.try_push(1)?;
        append_u32(&mut buffers.diagnostics, source_prefix_length)?;
        buffers.diagnostics.try_extend_from_slice(b"source/")?;
        buffers
            .diagnostics
            .try_extend_from_slice(graph_profile.as_str().as_bytes())?;
        buffers.diagnostics.try_extend_from_slice(b"/unit/")?;
        append_hex_u32(&mut buffers.diagnostics, candidate.unit)?;
        buffers.diagnostics.try_extend_from_slice(b".lfsynthetic")?;
        let start_line = u64::try_from(template.entities.len())
            .ok()
            .and_then(|value| value.checked_add(reference_count))
            .and_then(|value| value.checked_add(u64::from(candidate.relation_sequence_ordinal)))
            .and_then(|value| value.checked_add(1))
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(StageGenerationError::Overflow("diagnostic source line"))?;
        append_u32(&mut buffers.diagnostics, start_line)?;
        append_u32(&mut buffers.diagnostics, 1)?;
        append_u32(&mut buffers.diagnostics, start_line)?;
        append_u32(&mut buffers.diagnostics, 18)?;
        append_u32(&mut buffers.diagnostics, 34)?;
        append_u16(&mut buffers.diagnostics, 4)?;
        append_u64(&mut buffers.diagnostics, candidate.reference_ordinal)?;
        append_u32(&mut buffers.diagnostics, 20)?;
        buffers.diagnostics.try_extend_from_slice(b"04/")?;
        append_hex_u32(&mut buffers.diagnostics, candidate.unit)?;
        buffers.diagnostics.try_push(b'/')?;
        let unknown_local = 0x8000_0000_u32
            .checked_add(candidate.route_ordinal_within_unit)
            .ok_or(StageGenerationError::Overflow(
                "unknown LaneEdge local index",
            ))?;
        append_hex_u32(&mut buffers.diagnostics, unknown_local)?;
        emitted = emitted
            .checked_add(1)
            .ok_or(StageGenerationError::Overflow("emitted diagnostic count"))?;
    }
    if emitted != retained_count {
        return Err(StageGenerationError::MaterializedMismatch(
            "retained failure diagnostic count",
        ));
    }

    Ok(BoundedTemplateFailureObservation {
        diagnostic_count: retained_count,
        diagnostics_truncated,
        diagnostic_digest_sha256: diagnostic_buffer_digest(&buffers.diagnostics),
    })
}

fn materialize_duplicate_owner_diagnostics(
    template: &CorridorTemplate,
    graph_profile: GraphProfileId,
    n: u32,
    buffers: &mut BoundedTemplateBufferPool,
) -> Result<BoundedTemplateFailureObservation, StageGenerationError> {
    if buffers.duplicate_owner_candidates.len()
        != usize::try_from(n)
            .map_err(|_| StageGenerationError::Overflow("duplicate-owner unit count"))?
    {
        return Err(StageGenerationError::MaterializedMismatch(
            "duplicate-owner candidate count",
        ));
    }
    buffers
        .duplicate_owner_candidates
        .sort_unstable_by_key(|candidate| (candidate.unit, candidate.relation_sequence_ordinal));
    let diagnostic_count = u64::from(n);
    buffers.diagnostics.try_reserve(
        buffers
            .duplicate_owner_candidates
            .len()
            .checked_mul(192)
            .and_then(|bytes| bytes.checked_add(64))
            .ok_or(StageGenerationError::Overflow(
                "duplicate-owner diagnostic byte reserve",
            ))?,
    )?;
    buffers
        .diagnostics
        .try_extend_from_slice(crate::diagnostic::DIAGNOSTIC_STREAM_DOMAIN)?;
    append_u32(
        &mut buffers.diagnostics,
        crate::diagnostic::DIAGNOSTIC_STREAM_VERSION,
    )?;
    append_u64(&mut buffers.diagnostics, diagnostic_count)?;
    let reference_count = template_source_reference_count(template)?;
    let source_key_length = u32::try_from(
        "source/".len()
            + graph_profile.as_str().len()
            + 1
            + "unit/".len()
            + 8
            + ".lfsynthetic".len(),
    )
    .map_err(|_| StageGenerationError::Overflow("duplicate-owner source key length"))?;
    for candidate in buffers.duplicate_owner_candidates.iter() {
        append_u32(
            &mut buffers.diagnostics,
            u32::try_from(DUPLICATE_OWNER_ERROR_CODE.len())
                .map_err(|_| StageGenerationError::Overflow("duplicate-owner code length"))?,
        )?;
        buffers
            .diagnostics
            .try_extend_from_slice(DUPLICATE_OWNER_ERROR_CODE.as_bytes())?;
        buffers.diagnostics.try_push(1)?;
        append_u32(&mut buffers.diagnostics, source_key_length)?;
        buffers.diagnostics.try_extend_from_slice(b"source/")?;
        buffers
            .diagnostics
            .try_extend_from_slice(graph_profile.as_str().as_bytes())?;
        buffers.diagnostics.try_extend_from_slice(b"/unit/")?;
        append_hex_u32(&mut buffers.diagnostics, candidate.unit)?;
        buffers.diagnostics.try_extend_from_slice(b".lfsynthetic")?;
        let start_line = u64::try_from(template.entities.len())
            .ok()
            .and_then(|value| value.checked_add(reference_count))
            .and_then(|value| value.checked_add(u64::from(candidate.relation_sequence_ordinal)))
            .and_then(|value| value.checked_add(1))
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(StageGenerationError::Overflow(
                "duplicate-owner diagnostic source line",
            ))?;
        append_u32(&mut buffers.diagnostics, start_line)?;
        append_u32(&mut buffers.diagnostics, 1)?;
        append_u32(&mut buffers.diagnostics, start_line)?;
        append_u32(&mut buffers.diagnostics, 18)?;
        append_u32(&mut buffers.diagnostics, 50)?;
        append_u16(&mut buffers.diagnostics, candidate.child_kind)?;
        buffers
            .diagnostics
            .try_extend_from_slice(&candidate.child_stable_id)?;
        buffers
            .diagnostics
            .try_extend_from_slice(&candidate.first_owner_stable_id)?;
        buffers
            .diagnostics
            .try_extend_from_slice(&candidate.second_owner_stable_id)?;
    }
    Ok(BoundedTemplateFailureObservation {
        diagnostic_count,
        diagnostics_truncated: false,
        diagnostic_digest_sha256: diagnostic_buffer_digest(&buffers.diagnostics),
    })
}

fn template_source_reference_count(
    template: &CorridorTemplate,
) -> Result<u64, StageGenerationError> {
    let identity_reference_count = template
        .entities
        .iter()
        .try_fold(0_u64, |total, entity| {
            total.checked_add(u64::try_from(entity.identity_references.len()).ok()?)
        })
        .ok_or(StageGenerationError::Overflow(
            "failure identity reference count",
        ))?;
    let payload_reference_count = template
        .relations
        .iter()
        .try_fold(0_u64, |total, relation| {
            total.checked_add(relation.stable_reference_count())
        })
        .and_then(|total| total.checked_add(u64::try_from(template.geometry.len()).ok()?))
        .ok_or(StageGenerationError::Overflow(
            "failure payload reference count",
        ))?;
    identity_reference_count
        .checked_add(payload_reference_count)
        .ok_or(StageGenerationError::Overflow(
            "failure source reference count",
        ))
}

fn diagnostic_buffer_digest(bytes: &ControlledVec<u8>) -> String {
    let digest = Sha256::digest(bytes.as_slice());
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn append_hex_u32(output: &mut ControlledVec<u8>, value: u32) -> Result<(), StageGenerationError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for shift in (0..8).rev() {
        let nibble = usize::try_from((value >> (shift * 4)) & 0x0f).expect("u32 nibble fits usize");
        output.try_push(HEX[nibble])?;
    }
    Ok(())
}

pub(crate) fn finalize_bounded_template_stage_case(
    execution: BoundedTemplateExecution,
) -> Result<String, StageGenerationError> {
    finalize_bounded_template_stage_case_with_pool(execution).map(|(digest, _)| digest)
}

pub(crate) fn finalize_bounded_template_stage_case_with_pool(
    mut execution: BoundedTemplateExecution,
) -> Result<(String, BoundedTemplateBufferPool), StageGenerationError> {
    verify_bounded_materialization(&execution.buffers, &execution.plan)?;
    if execution.workload_id != execution.plan.workload_id
        || execution.graph_profile != execution.plan.graph_profile
        || execution.n != execution.plan.n
        || execution.buffers.semantic_record_stream.as_slice()
            != execution.buffers.output.as_slice()
    {
        return Err(StageGenerationError::MaterializedMismatch(
            "bounded template finalized identity",
        ));
    }
    let digest = Sha256::digest(execution.buffers.semantic_record_stream.as_slice());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut hex, "{byte:02x}")
            .map_err(|_| StageGenerationError::MaterializedMismatch("semantic digest"))?;
    }
    execution.buffers.clear_all();
    Ok((hex, execution.buffers))
}

#[allow(clippy::too_many_arguments)]
fn materialize_bounded_stage_prefix(
    generator: &GeneratorContract,
    stage: &StageContract,
    template: &CorridorTemplate,
    graph_profile: GraphProfileId,
    n: u32,
    plan: &ScalableStagePlanSummary,
    unsorted_records: &[BoundedSemanticRecord],
    semantic_payload: &[u8],
    source_permutation_scratch: &mut ControlledVec<u32>,
    source_spans: &mut ControlledVec<SourceSpanRecord>,
    source_records: &mut ControlledVec<TypedAstStageRecord>,
    source_payload: &mut ControlledVec<u8>,
    typed_records: &mut ControlledVec<TypedAstStageRecord>,
    typed_payload: &mut ControlledVec<u8>,
    hir_records: &mut ControlledVec<HirStageRecord>,
    hir_payload: &mut ControlledVec<u8>,
    mir_records: &mut ControlledVec<MirLirStageRecord>,
    mir_payload: &mut ControlledVec<u8>,
    diagnostics: &mut ControlledVec<u8>,
    scratch: &mut ControlledVec<u64>,
) -> Result<(), StageGenerationError> {
    exercise_source_permutations(generator, template, n, source_permutation_scratch)?;

    let source_span_count = to_usize(plan.counts.source_span_count, "bounded source span count")?;
    let module_count = u32::try_from(plan.counts.module_count)
        .map_err(|_| StageGenerationError::Overflow("bounded module count"))?;
    source_spans.try_reserve(source_span_count)?;
    for ordinal in 0..source_span_count {
        let ordinal = u32::try_from(ordinal)
            .map_err(|_| StageGenerationError::Overflow("bounded source span ordinal"))?;
        source_spans.try_push(SourceSpanRecord {
            source_document_ordinal: ordinal % module_count,
            start_line: ordinal
                .checked_div(module_count)
                .and_then(|line| line.checked_add(1))
                .ok_or(StageGenerationError::Overflow("bounded source line"))?,
            start_column: 1,
            end_line: ordinal
                .checked_div(module_count)
                .and_then(|line| line.checked_add(1))
                .ok_or(StageGenerationError::Overflow("bounded source line"))?,
            end_column: 21,
        })?;
    }

    let source_record_count = to_usize(
        plan.stages.source_input.record_count,
        "bounded source record count",
    )?;
    source_records.try_reserve(source_record_count)?;
    let module_records = to_usize(plan.counts.module_count, "bounded module records")?;
    let import_records = to_usize(plan.counts.import_edge_count, "bounded import records")?;
    for ordinal in 0..source_record_count {
        let (record_kind, module_ordinal, source_span_ordinal) = if ordinal < module_records {
            (
                stage.record_kind_module,
                u32::try_from(ordinal)
                    .map_err(|_| StageGenerationError::Overflow("module ordinal"))?,
                stage.absent_ordinal,
            )
        } else if ordinal < module_records + import_records {
            (
                stage.record_kind_import,
                u32::try_from(ordinal % module_records)
                    .map_err(|_| StageGenerationError::Overflow("import module ordinal"))?,
                stage.absent_ordinal,
            )
        } else {
            (
                stage.record_kind_declaration,
                u32::try_from(ordinal % module_records)
                    .map_err(|_| StageGenerationError::Overflow("source module ordinal"))?,
                u32::try_from(ordinal - module_records - import_records)
                    .map_err(|_| StageGenerationError::Overflow("source span ordinal"))?,
            )
        };
        source_records.try_push(TypedAstStageRecord {
            record_kind,
            entity_kind: 0,
            module_ordinal,
            source_span_ordinal,
            owner_local_index: u32::try_from(ordinal)
                .map_err(|_| StageGenerationError::Overflow("source local index"))?,
            payload_offset: 0,
            payload_length: 0,
        })?;
    }

    let source_payload_bytes = to_usize(
        plan.stages.source_input.payload_logical_bytes,
        "bounded source payload bytes",
    )?;
    source_payload.try_reserve(source_payload_bytes)?;
    source_payload.try_resize(source_payload_bytes, 0)?;

    let typed_record_count = to_usize(
        plan.stages.typed_ast.record_count,
        "bounded typed AST record count",
    )?;
    typed_records.try_reserve(typed_record_count)?;
    for ordinal in 0..typed_record_count {
        let mut record = source_records
            .as_slice()
            .get(ordinal)
            .copied()
            .unwrap_or_default();
        if ordinal >= source_records.len() {
            record.record_kind = stage.record_kind_declaration;
            record.owner_local_index = u32::try_from(ordinal)
                .map_err(|_| StageGenerationError::Overflow("typed AST local index"))?;
            record.source_span_ordinal = stage.absent_ordinal;
        }
        typed_records.try_push(record)?;
    }
    let typed_payload_bytes = to_usize(
        plan.stages.typed_ast.payload_logical_bytes,
        "bounded typed AST payload bytes",
    )?;
    typed_payload.try_reserve(typed_payload_bytes)?;
    typed_payload.try_extend_from_slice(source_payload.as_slice())?;
    for span in source_spans.iter() {
        append_u32(typed_payload, span.source_document_ordinal)?;
        append_u32(typed_payload, span.start_line)?;
        append_u32(typed_payload, span.start_column)?;
        append_u32(typed_payload, span.end_line)?;
        append_u32(typed_payload, span.end_column)?;
    }
    if typed_payload.len() != typed_payload_bytes {
        return Err(StageGenerationError::MaterializedMismatch(
            "bounded typed AST payload",
        ));
    }

    let hir_record_count = to_usize(plan.stages.hir.record_count, "bounded HIR record count")?;
    hir_records.try_reserve(hir_record_count)?;
    for ordinal in 0..hir_record_count {
        let typed = typed_records
            .as_slice()
            .get(ordinal)
            .copied()
            .unwrap_or_default();
        hir_records.try_push(HirStageRecord {
            record_kind: if typed.record_kind == stage.record_kind_declaration {
                stage.record_kind_symbol
            } else {
                typed.record_kind
            },
            entity_kind: typed.entity_kind,
            module_ordinal: typed.module_ordinal,
            symbol_ordinal: typed.owner_local_index,
            resolved_target_ordinal: stage.absent_ordinal,
            payload_offset: 0,
            payload_length: 0,
        })?;
    }
    let hir_payload_bytes = to_usize(
        plan.stages.hir.payload_logical_bytes,
        "bounded HIR payload bytes",
    )?;
    let source_byte_count = to_usize(plan.counts.source_byte_count, "bounded source byte count")?;
    if source_byte_count > source_payload.len() {
        return Err(StageGenerationError::MaterializedMismatch(
            "bounded HIR string source",
        ));
    }
    hir_payload.try_reserve(hir_payload_bytes)?;
    let string_bytes = &source_payload.as_slice()[source_byte_count..];
    hir_payload
        .try_extend_from_slice(&string_bytes[..string_bytes.len().min(hir_payload_bytes)])?;
    hir_payload.try_resize(hir_payload_bytes, 0)?;

    materialize_bounded_semantic_stage(
        unsorted_records,
        semantic_payload,
        mir_records,
        mir_payload,
    )?;
    let diagnostic_bytes = to_usize(
        plan.stages.diagnostics.logical_bytes,
        "bounded diagnostic bytes",
    )?;
    diagnostics.try_reserve(diagnostic_bytes)?;
    diagnostics.try_resize(diagnostic_bytes, 0)?;
    let scratch_bytes = to_usize(plan.stages.scratch.logical_bytes, "bounded scratch bytes")?;
    if scratch_bytes % std::mem::size_of::<u64>() != 0 {
        return Err(StageGenerationError::MaterializedMismatch(
            "bounded scratch word alignment",
        ));
    }
    let scratch_words = scratch_bytes / std::mem::size_of::<u64>();
    scratch.try_reserve(scratch_words)?;
    scratch.try_resize(scratch_words, 0)?;

    let _ = graph_profile;
    Ok(())
}

fn exercise_source_permutations(
    generator: &GeneratorContract,
    template: &CorridorTemplate,
    n: u32,
    scratch: &mut ControlledVec<u32>,
) -> Result<(), StageGenerationError> {
    let counts = template.stage_input_counts();
    let sequence_counts = [
        (
            SequenceKind::Declarations,
            required_template_count(&counts, "sourceDeclarationCount")?,
        ),
        (
            SequenceKind::References,
            required_template_count(&counts, "sourceReferenceCount")?,
        ),
        (
            SequenceKind::Relations,
            required_template_count(&counts, "sourceRelationCount")?,
        ),
        (
            SequenceKind::Geometry,
            required_template_count(&counts, "sourceGeometryCount")?,
        ),
    ];
    let maximum = sequence_counts
        .iter()
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(0);
    let maximum = to_usize(maximum, "bounded source permutation maximum")?;
    scratch.try_reserve(maximum)?;
    for unit in 0..n {
        for (sequence_kind, count) in sequence_counts {
            scratch.clear();
            for ordinal in 0..count {
                scratch.try_push(
                    u32::try_from(ordinal).map_err(|_| {
                        StageGenerationError::Overflow("source permutation ordinal")
                    })?,
                )?;
            }
            permute_in_place(
                scratch.as_mut_slice(),
                generator,
                sequence_kind,
                generator.base_seed() ^ u64::from(unit),
            );
        }
    }
    Ok(())
}

fn required_template_count(
    counts: &std::collections::BTreeMap<String, u64>,
    field: &'static str,
) -> Result<u64, StageGenerationError> {
    counts
        .get(field)
        .copied()
        .ok_or(StageGenerationError::MaterializedMismatch(field))
}

fn materialize_bounded_semantic_stage(
    records: &[BoundedSemanticRecord],
    semantic_payload: &[u8],
    stage_records: &mut ControlledVec<MirLirStageRecord>,
    payload: &mut ControlledVec<u8>,
) -> Result<(), StageGenerationError> {
    stage_records.try_reserve(records.len())?;
    payload.try_reserve(semantic_payload.len())?;
    for record in records {
        let offset = payload.len();
        let source = record_payload_slice(record, semantic_payload);
        payload.try_extend_from_slice(source)?;
        stage_records.try_push(MirLirStageRecord {
            record_kind: record.record_kind,
            entity_kind: record.entity_kind_code,
            stable_id: record.stable_id,
            owner_ordinal: record.owner_ordinal,
            local_index: record.local_index,
            payload_offset: u64::try_from(offset)
                .map_err(|_| StageGenerationError::Overflow("semantic payload offset"))?,
            payload_length: u64::try_from(source.len())
                .map_err(|_| StageGenerationError::Overflow("semantic payload length"))?,
        })?;
    }
    Ok(())
}

fn verify_bounded_materialization(
    materialization: &BoundedTemplateBufferPool,
    plan: &ScalableStagePlanSummary,
) -> Result<(), StageGenerationError> {
    let checks = [
        (
            materialization.source_records.len(),
            materialization.source_payload.len(),
            plan.stages.source_input.record_count,
            plan.stages.source_input.payload_logical_bytes,
        ),
        (
            materialization.typed_records.len(),
            materialization.typed_payload.len(),
            plan.stages.typed_ast.record_count,
            plan.stages.typed_ast.payload_logical_bytes,
        ),
        (
            materialization.hir_records.len(),
            materialization.hir_payload.len(),
            plan.stages.hir.record_count,
            plan.stages.hir.payload_logical_bytes,
        ),
        (
            materialization.mir_records.len(),
            materialization.mir_payload.len(),
            plan.stages.mir.record_count,
            plan.stages.mir.payload_logical_bytes,
        ),
        (
            materialization.lir_records.len(),
            materialization.lir_payload.len(),
            plan.stages.canonical_lir.record_count,
            plan.stages.canonical_lir.payload_logical_bytes,
        ),
    ];
    for (actual_records, actual_payload, expected_records, expected_payload) in checks {
        if u64::try_from(actual_records).ok() != Some(expected_records)
            || u64::try_from(actual_payload).ok() != Some(expected_payload)
        {
            return Err(StageGenerationError::MaterializedMismatch(
                "bounded template stage shape",
            ));
        }
    }
    if u64::try_from(materialization.source_spans.len()).ok() != Some(plan.counts.source_span_count)
        || u64::try_from(materialization.diagnostics.len()).ok()
            != Some(plan.stages.diagnostics.logical_bytes)
        || materialization
            .scratch
            .len()
            .checked_mul(std::mem::size_of::<u64>())
            .and_then(|bytes| u64::try_from(bytes).ok())
            != Some(plan.stages.scratch.logical_bytes)
        || u64::try_from(materialization.output.len()).ok()
            != Some(plan.stages.output_construction.logical_bytes)
    {
        return Err(StageGenerationError::MaterializedMismatch(
            "bounded template non-primary stage shape",
        ));
    }
    Ok(())
}

fn maximum_canonical_identity_bytes(
    identity: &IdentityContract,
    template: &CorridorTemplate,
) -> Result<usize, StageGenerationError> {
    let mut maximum = 10_usize;
    for entity in &template.entities {
        let binding = identity
            .bindings
            .iter()
            .find(|binding| binding.entity_kind_code == entity.reference.kind)
            .ok_or(StageGenerationError::MissingEntityKind(
                entity.reference.kind,
            ))?;
        let bytes = binding.fields.iter().try_fold(10_usize, |total, field| {
            let value_bytes = match field.value {
                IdentityFieldValue::Namespace => 32,
                IdentityFieldValue::ProfiledKey { .. } => 20,
                IdentityFieldValue::StableId { .. } => 16,
            };
            total
                .checked_add(6 + value_bytes)
                .ok_or(StageGenerationError::Overflow(
                    "maximum canonical identity bytes",
                ))
        })?;
        maximum = maximum.max(bytes);
    }
    Ok(maximum)
}

fn assign_owner_ordinals(
    template: &CorridorTemplate,
    declarations: &mut ControlledVec<BoundedDeclaration>,
    indexes: &mut ControlledVec<usize>,
) -> Result<(), StageGenerationError> {
    indexes.try_reserve(declarations.len())?;
    for entity in &template.entities {
        let kind = entity.reference.kind;
        if template
            .entities
            .iter()
            .take_while(|candidate| candidate.reference != entity.reference)
            .any(|candidate| candidate.reference.kind == kind)
        {
            continue;
        }
        indexes.clear();
        for (index, declaration) in declarations.iter().enumerate() {
            if declaration.owner.entity.kind == kind {
                indexes.try_push(index)?;
            }
        }
        indexes.sort_unstable_by_key(|index| declarations[*index].stable_id);
        for (ordinal, index) in indexes.iter().copied().enumerate() {
            declarations[index].owner_ordinal = u32::try_from(ordinal)
                .map_err(|_| StageGenerationError::Overflow("owner ordinal"))?;
        }
    }
    Ok(())
}

fn compile_relation(
    template: &CorridorTemplate,
    declarations: &[BoundedDeclaration],
    fixed_key_lookup: &FixedKeyLookup,
    unit: u32,
    relation: &TemplateRelation,
    payload: &mut ControlledVec<u8>,
) -> Result<BoundedSemanticRecord, StageGenerationError> {
    let start = payload.len();
    let (record_kind, owner, pending_order) = match relation {
        TemplateRelation::Owner { child, parent } => {
            append_u16(payload, parent.kind)?;
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                fixed_key_lookup,
                unit,
                *parent,
            )?)?;
            (2, *child, PendingOrder::Absent)
        }
        TemplateRelation::EdgeConnection { source, target } => {
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                fixed_key_lookup,
                unit,
                *target,
            )?)?;
            (3, *source, PendingOrder::OwnerPayload)
        }
        TemplateRelation::RouteOccurrence { route, index, edge } => {
            append_u32(payload, *index)?;
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                fixed_key_lookup,
                unit,
                *edge,
            )?)?;
            (4, *route, PendingOrder::Explicit(*index))
        }
        TemplateRelation::Access {
            rule,
            participant,
            target,
            decision,
        } => {
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                fixed_key_lookup,
                unit,
                *participant,
            )?)?;
            append_u16(payload, target.kind)?;
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                fixed_key_lookup,
                unit,
                *target,
            )?)?;
            payload.try_push(*decision)?;
            (6, *rule, PendingOrder::OwnerPayload)
        }
        TemplateRelation::SignalGroup { group, gate } => {
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                fixed_key_lookup,
                unit,
                *gate,
            )?)?;
            (7, *group, PendingOrder::OwnerPayload)
        }
        TemplateRelation::PhaseState {
            phase,
            group,
            state,
        } => {
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                fixed_key_lookup,
                unit,
                *group,
            )?)?;
            payload.try_push(*state)?;
            (8, *phase, PendingOrder::OwnerPayload)
        }
        TemplateRelation::Gate {
            path,
            transition_index,
            gate,
            stop_line,
            edge,
            edge_position_bits,
        } => {
            let gate_id = stable_id(template, declarations, fixed_key_lookup, unit, *gate)?;
            append_u32(payload, 0)?;
            payload.try_extend_from_slice(&gate_id)?;
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                fixed_key_lookup,
                unit,
                *stop_line,
            )?)?;
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                fixed_key_lookup,
                unit,
                *edge,
            )?)?;
            append_u32(payload, *edge_position_bits)?;
            (
                9,
                *path,
                PendingOrder::Gate {
                    transition_index: *transition_index,
                    stable_id: gate_id,
                },
            )
        }
        TemplateRelation::WaitingZone {
            path,
            entry_transition_index,
            release_transition_index,
            zone,
            before_gate,
            after_gate,
            capacity,
        } => {
            let zone_id = stable_id(template, declarations, fixed_key_lookup, unit, *zone)?;
            append_u32(payload, 0)?;
            payload.try_extend_from_slice(&zone_id)?;
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                fixed_key_lookup,
                unit,
                *before_gate,
            )?)?;
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                fixed_key_lookup,
                unit,
                *after_gate,
            )?)?;
            append_u32(payload, *capacity)?;
            (
                10,
                *path,
                PendingOrder::Waiting {
                    entry_transition_index: *entry_transition_index,
                    release_transition_index: *release_transition_index,
                    stable_id: zone_id,
                },
            )
        }
        TemplateRelation::Parking {
            space,
            entry_edge,
            entry_high_bits,
            entry_residual_bits,
            exit_edge,
            exit_high_bits,
            exit_residual_bits,
        } => {
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                fixed_key_lookup,
                unit,
                *space,
            )?)?;
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                fixed_key_lookup,
                unit,
                *entry_edge,
            )?)?;
            append_u32(payload, *entry_high_bits)?;
            append_u32(payload, *entry_residual_bits)?;
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                fixed_key_lookup,
                unit,
                *exit_edge,
            )?)?;
            append_u32(payload, *exit_high_bits)?;
            append_u32(payload, *exit_residual_bits)?;
            (11, *space, PendingOrder::Absent)
        }
        TemplateRelation::LaneCoverage { lane, index, edge } => {
            append_u32(payload, *index)?;
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                fixed_key_lookup,
                unit,
                *edge,
            )?)?;
            (12, *lane, PendingOrder::Explicit(*index))
        }
        TemplateRelation::JunctionInternalEdge { junction, edge } => {
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                fixed_key_lookup,
                unit,
                *edge,
            )?)?;
            (13, *junction, PendingOrder::OwnerPayload)
        }
    };
    let declaration = declaration(template, declarations, fixed_key_lookup, unit, owner)?;
    Ok(BoundedSemanticRecord {
        record_kind,
        entity_kind_code: owner.kind,
        stable_id: declaration.stable_id,
        owner_ordinal: declaration.owner_ordinal,
        local_index: match pending_order {
            PendingOrder::Absent => ABSENT_LOCAL_INDEX,
            PendingOrder::Explicit(index) => index,
            PendingOrder::OwnerPayload
            | PendingOrder::Gate { .. }
            | PendingOrder::Waiting { .. } => 0,
        },
        payload: PayloadRange {
            offset: start,
            length: payload.len() - start,
        },
        pending_order,
    })
}

fn assign_local_indexes(
    records: &mut [BoundedSemanticRecord],
    payload: &mut [u8],
    start: usize,
    end: usize,
    scratch: &mut ControlledVec<usize>,
) -> Result<(), StageGenerationError> {
    for index in start..end {
        let order = records[index].pending_order;
        let is_group_start = (start..index)
            .all(|previous| !same_local_index_group(&records[previous], &records[index]));
        if !is_group_start || matches!(order, PendingOrder::Absent | PendingOrder::Explicit(_)) {
            continue;
        }
        scratch.clear();
        for candidate in start..end {
            if same_local_index_group(&records[candidate], &records[index]) {
                scratch.try_push(candidate)?;
            }
        }
        match order {
            PendingOrder::OwnerPayload => scratch.sort_unstable_by(|left, right| {
                record_payload_slice(&records[*left], payload)
                    .cmp(record_payload_slice(&records[*right], payload))
                    .then_with(|| records[*left].stable_id.cmp(&records[*right].stable_id))
            }),
            PendingOrder::Gate { .. } => {
                scratch.sort_unstable_by_key(|candidate| match records[*candidate].pending_order {
                    PendingOrder::Gate {
                        transition_index,
                        stable_id,
                    } => (transition_index, stable_id),
                    _ => unreachable!("gate group"),
                })
            }
            PendingOrder::Waiting { .. } => {
                scratch.sort_unstable_by_key(|candidate| match records[*candidate].pending_order {
                    PendingOrder::Waiting {
                        entry_transition_index,
                        release_transition_index,
                        stable_id,
                    } => (entry_transition_index, release_transition_index, stable_id),
                    _ => unreachable!("waiting group"),
                })
            }
            PendingOrder::Absent | PendingOrder::Explicit(_) => unreachable!("filtered above"),
        }
        for (ordinal, candidate) in scratch.iter().copied().enumerate() {
            let ordinal = u32::try_from(ordinal)
                .map_err(|_| StageGenerationError::Overflow("local index"))?;
            if matches!(
                records[candidate].pending_order,
                PendingOrder::Gate { .. } | PendingOrder::Waiting { .. }
            ) {
                let offset = records[candidate].payload.offset;
                payload[offset..offset + 4].copy_from_slice(&ordinal.to_le_bytes());
            }
            records[candidate].local_index = ordinal;
            records[candidate].pending_order = PendingOrder::Explicit(ordinal);
        }
    }
    Ok(())
}

fn same_local_index_group(left: &BoundedSemanticRecord, right: &BoundedSemanticRecord) -> bool {
    match (left.pending_order, right.pending_order) {
        (PendingOrder::OwnerPayload, PendingOrder::OwnerPayload) => {
            left.record_kind == right.record_kind
                && left.entity_kind_code == right.entity_kind_code
                && left.stable_id == right.stable_id
        }
        (PendingOrder::Gate { .. }, PendingOrder::Gate { .. })
        | (PendingOrder::Waiting { .. }, PendingOrder::Waiting { .. }) => {
            left.entity_kind_code == right.entity_kind_code && left.stable_id == right.stable_id
        }
        _ => false,
    }
}

fn canonical_record_compare(
    left: &BoundedSemanticRecord,
    right: &BoundedSemanticRecord,
    payload: &[u8],
) -> Ordering {
    (
        left.record_kind,
        left.entity_kind_code,
        left.stable_id,
        left.owner_ordinal,
        left.local_index,
        record_payload_slice(left, payload),
    )
        .cmp(&(
            right.record_kind,
            right.entity_kind_code,
            right.stable_id,
            right.owner_ordinal,
            right.local_index,
            record_payload_slice(right, payload),
        ))
}

fn record_payload_slice<'a>(record: &BoundedSemanticRecord, payload: &'a [u8]) -> &'a [u8] {
    &payload[record.payload.offset..record.payload.offset + record.payload.length]
}

fn declaration<'a>(
    _template: &CorridorTemplate,
    declarations: &'a [BoundedDeclaration],
    fixed_key_lookup: &FixedKeyLookup,
    unit: u32,
    entity: EntityRef,
) -> Result<&'a BoundedDeclaration, StageGenerationError> {
    let index = fixed_key_lookup
        .get(fixed_key(UnitEntityRef { unit, entity }))
        .ok_or(StageGenerationError::MaterializedMismatch(
            "template declaration fixed-key lookup",
        ))?;
    declarations
        .get(index)
        .ok_or(StageGenerationError::MaterializedMismatch(
            "template declaration lookup",
        ))
}

fn stable_id(
    template: &CorridorTemplate,
    declarations: &[BoundedDeclaration],
    fixed_key_lookup: &FixedKeyLookup,
    unit: u32,
    entity: EntityRef,
) -> Result<[u8; 16], StageGenerationError> {
    Ok(declaration(template, declarations, fixed_key_lookup, unit, entity)?.stable_id)
}

fn declaration_index(
    template: &CorridorTemplate,
    unit: u32,
    entity: EntityRef,
    entity_count: usize,
) -> Result<usize, StageGenerationError> {
    let entity_index = template
        .entities
        .iter()
        .position(|candidate| candidate.reference == entity)
        .ok_or(StageGenerationError::MaterializedMismatch(
            "template entity lookup",
        ))?;
    usize::try_from(unit)
        .ok()
        .and_then(|unit| unit.checked_mul(entity_count))
        .and_then(|base| base.checked_add(entity_index))
        .ok_or(StageGenerationError::Overflow("template declaration index"))
}

fn geometry_coordinate_bits(
    point: &crate::corridor::TemplateGeometry,
    unit: u32,
) -> Result<(u32, u32, u32), StageGenerationError> {
    if point.coordinate_rule == TemplateGeometryRule::Fixed {
        return Ok((point.x_bits, point.y_bits, point.z_bits));
    }
    let unit_x = unit % 4_096;
    let unit_y = unit / 4_096;
    let x = unit_x
        .checked_mul(128)
        .and_then(|base| {
            point
                .edge
                .local
                .checked_mul(2)
                .and_then(|edge| base.checked_add(edge))
        })
        .and_then(|base| base.checked_add(point.point_index))
        .ok_or(StageGenerationError::Overflow("junction grid x coordinate"))?;
    let y = unit_y
        .checked_mul(128)
        .ok_or(StageGenerationError::Overflow("junction grid y coordinate"))?;
    Ok((
        (x as f32).to_bits(),
        (y as f32).to_bits(),
        0.0_f32.to_bits(),
    ))
}

fn derive_namespace_ascii(
    generator: &GeneratorContract,
    workload_id: &str,
    graph_profile: GraphProfileId,
    unit: u32,
) -> Result<[u8; 32], StageGenerationError> {
    if generator.namespace_digest_length != 16 {
        return Err(StageGenerationError::MaterializedMismatch(
            "namespace digest length",
        ));
    }
    let (module_name, module_name_len) = unit_module_name(unit);
    let module_name = &module_name[..module_name_len];
    let mut hasher = blake3::Hasher::new();
    hasher.update(generator.namespace_domain.as_bytes());
    hasher.update(&[0]);
    hasher.update(&generator.generator_version.to_le_bytes());
    hasher.update(&generator.base_seed.to_le_bytes());
    hash_length_prefixed(&mut hasher, workload_id.as_bytes())?;
    hash_length_prefixed(&mut hasher, graph_profile.as_str().as_bytes())?;
    hash_length_prefixed(&mut hasher, module_name)?;
    let digest = hasher.finalize();
    let selected = &digest.as_bytes()[generator.namespace_digest_offset
        ..generator.namespace_digest_offset + generator.namespace_digest_length];
    let mut output = [0_u8; 32];
    for (index, byte) in selected.iter().copied().enumerate() {
        output[index * 2] = hex_digit(byte >> 4);
        output[index * 2 + 1] = hex_digit(byte & 0x0f);
    }
    Ok(output)
}

fn hash_length_prefixed(
    hasher: &mut blake3::Hasher,
    value: &[u8],
) -> Result<(), StageGenerationError> {
    let length = u32::try_from(value.len())
        .map_err(|_| StageGenerationError::Overflow("namespace preimage length"))?;
    hasher.update(&length.to_le_bytes());
    hasher.update(value);
    Ok(())
}

fn unit_module_name(unit: u32) -> ([u8; 14], usize) {
    let mut output = [0_u8; 14];
    let prefix = b"unit/".as_slice();
    output[..prefix.len()].copy_from_slice(prefix);
    write_hex_u32(unit, &mut output[prefix.len()..prefix.len() + 8]);
    (output, prefix.len() + 8)
}

fn append_u16(output: &mut ControlledVec<u8>, value: u16) -> Result<(), StageGenerationError> {
    output.try_extend_from_slice(&value.to_le_bytes())?;
    Ok(())
}

fn append_u32(output: &mut ControlledVec<u8>, value: u32) -> Result<(), StageGenerationError> {
    output.try_extend_from_slice(&value.to_le_bytes())?;
    Ok(())
}

fn append_u64(output: &mut ControlledVec<u8>, value: u64) -> Result<(), StageGenerationError> {
    output.try_extend_from_slice(&value.to_le_bytes())?;
    Ok(())
}

fn write_hex_u16(value: u16, output: &mut [u8]) {
    debug_assert_eq!(output.len(), 2);
    output[0] = hex_digit(((value >> 4) & 0x0f) as u8);
    output[1] = hex_digit((value & 0x0f) as u8);
}

fn write_hex_u32(value: u32, output: &mut [u8]) {
    debug_assert_eq!(output.len(), 8);
    for (index, destination) in output.iter_mut().enumerate() {
        let shift = (7 - index) * 4;
        *destination = hex_digit(((value >> shift) & 0x0f) as u8);
    }
}

fn hex_digit(value: u8) -> u8 {
    match value {
        0..=9 => b'0' + value,
        10..=15 => b'a' + value - 10,
        _ => unreachable!("hex nibble"),
    }
}

fn checked_mul_usize(
    left: usize,
    right: usize,
    field: &'static str,
) -> Result<usize, StageGenerationError> {
    left.checked_mul(right)
        .ok_or(StageGenerationError::Overflow(field))
}

fn capacity_sum(values: &[u64]) -> Result<u64, StageGenerationError> {
    values.iter().try_fold(0_u64, |total, value| {
        total
            .checked_add(*value)
            .ok_or(StageGenerationError::Overflow(
                "template retained capacity bytes",
            ))
    })
}

fn to_usize(value: u64, field: &'static str) -> Result<usize, StageGenerationError> {
    usize::try_from(value).map_err(|_| StageGenerationError::Overflow(field))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corridor::{CORRIDOR_WORKLOAD_ID, CorridorContract, build_corridor_stage_case};
    use crate::junction_grid::{
        JUNCTION_GRID_WORKLOAD_ID, JunctionGridContract, build_junction_grid_stage_case,
        build_junction_grid_template,
    };
    use crate::{ScalableStagePlanFactory, TrustedContract, load_repository_contract};

    fn contract() -> TrustedContract {
        load_repository_contract().expect("frozen contract")
    }

    #[test]
    fn bounded_template_semantic_output_matches_existing_producers() {
        let trusted = contract();
        let generator = trusted.generator_contract().unwrap();
        let identity = trusted.identity_contract().unwrap();
        let stage = trusted.stage_contract().unwrap();
        let plans = ScalableStagePlanFactory::from_trusted_contract(&trusted).unwrap();
        let corridor_contract =
            CorridorContract::from_manifest(&trusted.workload_manifest).unwrap();
        let corridor_template = corridor_contract.load_template().unwrap();
        let junction_contract =
            JunctionGridContract::from_manifest(&trusted.workload_manifest).unwrap();
        let junction_template = build_junction_grid_template();
        junction_contract
            .validate_template(&junction_template)
            .unwrap();

        for graph_profile in GraphProfileId::ALL {
            for n in [1, 2] {
                for (workload_id, template) in [
                    (ScalableWorkloadId::Corridor, &corridor_template),
                    (ScalableWorkloadId::JunctionGrid, &junction_template),
                ] {
                    let plan = plans.plan(workload_id, graph_profile, n).unwrap();
                    let allocator = ControlledAllocator::new(u64::MAX);
                    let bounded = execute_bounded_template_stage_case(
                        &generator,
                        &identity,
                        &stage,
                        workload_id,
                        template,
                        graph_profile,
                        n,
                        &plan,
                        allocator,
                    )
                    .unwrap();
                    let existing = match workload_id {
                        ScalableWorkloadId::Corridor => build_corridor_stage_case(
                            &generator,
                            &identity,
                            &stage,
                            &corridor_contract,
                            template,
                            graph_profile,
                            n,
                        )
                        .unwrap(),
                        ScalableWorkloadId::JunctionGrid => build_junction_grid_stage_case(
                            &generator,
                            &identity,
                            &stage,
                            &junction_contract,
                            template,
                            graph_profile,
                            n,
                        )
                        .unwrap(),
                        ScalableWorkloadId::Identity => unreachable!(),
                    };
                    assert_eq!(
                        bounded.semantic_record_stream(),
                        existing.semantic_record_stream,
                        "{}/{graph_profile:?}/{n}",
                        workload_id.as_str()
                    );
                    assert_eq!(bounded.records().len(), existing.records.len());
                    for (bounded_record, existing_record) in
                        bounded.records().iter().zip(&existing.records)
                    {
                        assert_eq!(bounded_record.record_kind, existing_record.record_kind);
                        assert_eq!(
                            bounded_record.entity_kind_code,
                            existing_record.entity_kind_code
                        );
                        assert_eq!(bounded_record.stable_id, existing_record.stable_id);
                        assert_eq!(bounded_record.owner_ordinal, existing_record.owner_ordinal);
                        assert_eq!(bounded_record.local_index, existing_record.local_index);
                        assert_eq!(
                            bounded.record_payload(bounded_record),
                            existing_record.payload
                        );
                    }
                }
            }
        }
        let _ = (CORRIDOR_WORKLOAD_ID, JUNCTION_GRID_WORKLOAD_ID);
    }
}
