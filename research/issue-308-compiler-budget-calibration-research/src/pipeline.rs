//! `LF-COMP-ID-v1` 的受测八阶段因果管线。
//!
//! 本模块只在计时边界内物化与规模成比例的来源、阶段值和载荷。摘要、十六进制展示、
//! 独立预言机比较与证据 JSON 均由调用方在管线返回后完成。计时角色与归因角色通过
//! 编译期常量生成两条单态化容量路径：计时路径只执行受检容量规划和
//! `try_reserve_exact`，归因路径才执行逐容量请求记账与硬上限预占。

use crate::identity::{
    ABSENT_LOCAL_INDEX, IDENTITY_MAGIC, IdentityBinding, IdentityContract, IdentityFieldValue,
    STABLE_ID_DOMAIN,
};
use crate::stage::{
    HirStageRecord, IdentityStageCaseOutput, IdentityStagePlan, IdentityStageSummary,
    MirLirStageRecord, SourceSpanRecord, StageContract, StageGenerationError,
    StageRetainedCapacityBytes, TypedAstStageRecord, as_u64, to_usize,
};
use crate::{GeneratorContract, GraphProfileId, SequenceKind, permute_in_place};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

const ENTITY_KIND_ABSENT: u16 = 0;
const SHARED_CONSTANT_ENTITY_KIND: u16 = 0x00ff;

#[derive(Clone, Copy, Debug)]
struct StringLayout {
    string_start: usize,
    namespace_base: usize,
    profiled_key_base: usize,
    reference_base: usize,
    string_end: usize,
}

#[derive(Debug)]
struct SourceStage {
    spans: Vec<SourceSpanRecord>,
    records: Vec<TypedAstStageRecord>,
    payload: Vec<u8>,
    strings: StringLayout,
    scratch: Vec<u64>,
    namespace_preimage_scratch: Vec<u8>,
}

#[derive(Debug)]
struct TypedStage {
    records: Vec<TypedAstStageRecord>,
    payload: Vec<u8>,
}

#[derive(Debug)]
struct HirStage {
    records: Vec<HirStageRecord>,
    payload: Vec<u8>,
}

#[derive(Debug)]
struct MirStage {
    records: Vec<MirLirStageRecord>,
    payload: Vec<u8>,
    stable_id_scratch: Vec<[u8; 16]>,
    canonical_identity_scratch: Vec<u8>,
    identity_payload_scratch: Vec<u8>,
}

#[derive(Debug)]
struct LirStage {
    records: Vec<MirLirStageRecord>,
    payload: Vec<u8>,
    scratch_capacity_bytes: u64,
    sort_scratch: Vec<usize>,
    owner_ordinal_scratch: Vec<u32>,
}

#[derive(Clone, Copy, Debug)]
enum ControlledBufferSlot {
    SourceSpans,
    SourceRecords,
    SourcePayload,
    SourceScratch,
    NamespacePreimageScratch,
    TypedAstRecords,
    TypedAstPayload,
    HirRecords,
    HirPayload,
    MirRecords,
    MirPayload,
    MirStableIdScratch,
    MirCanonicalIdentityScratch,
    MirIdentityPayloadScratch,
    CanonicalLirRecords,
    CanonicalLirPayload,
    LirSortScratch,
    LirOwnerOrdinalScratch,
    OutputConstruction,
}

impl ControlledBufferSlot {
    const COUNT: usize = 19;

    const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug)]
struct ControlledAllocationTracker<const TRACK_ALLOCATIONS: bool> {
    hard_ceiling_bytes: u64,
    live_requested_bytes: AtomicU64,
    peak_live_requested_bytes: AtomicU64,
    requested_bytes_by_slot: [u64; ControlledBufferSlot::COUNT],
    allocation_count: u64,
    reallocation_count: u64,
    allocated_bytes: u64,
    reallocated_bytes: u64,
    freed_bytes: u64,
}

impl<const TRACK_ALLOCATIONS: bool> ControlledAllocationTracker<TRACK_ALLOCATIONS> {
    fn new(hard_ceiling_bytes: u64) -> Self {
        Self {
            hard_ceiling_bytes,
            live_requested_bytes: AtomicU64::new(0),
            peak_live_requested_bytes: AtomicU64::new(0),
            requested_bytes_by_slot: [0; ControlledBufferSlot::COUNT],
            allocation_count: 0,
            reallocation_count: 0,
            allocated_bytes: 0,
            reallocated_bytes: 0,
            freed_bytes: 0,
        }
    }

    fn preoccupy(
        &self,
        field: &'static str,
        requested_bytes: u64,
    ) -> Result<u64, StageGenerationError> {
        self.live_requested_bytes
            .fetch_update(
                AtomicOrdering::SeqCst,
                AtomicOrdering::SeqCst,
                |live_requested_bytes| {
                    live_requested_bytes
                        .checked_add(requested_bytes)
                        .filter(|next| *next <= self.hard_ceiling_bytes)
                },
            )
            .map(|previous| previous + requested_bytes)
            .map_err(
                |live_requested_bytes| StageGenerationError::ControlledAllocationHardCeiling {
                    field,
                    hard_ceiling_bytes: self.hard_ceiling_bytes,
                    live_requested_bytes,
                    requested_bytes,
                },
            )
    }

    fn cancel_preoccupation(&self, requested_bytes: u64) {
        let previous = self
            .live_requested_bytes
            .fetch_sub(requested_bytes, AtomicOrdering::SeqCst);
        debug_assert!(previous >= requested_bytes);
    }

    fn commit_replacement(
        &mut self,
        slot: ControlledBufferSlot,
        requested_bytes: u64,
        preoccupied_live_bytes: u64,
    ) -> Result<(), StageGenerationError> {
        self.peak_live_requested_bytes
            .fetch_max(preoccupied_live_bytes, AtomicOrdering::SeqCst);
        let previous_requested_bytes = std::mem::replace(
            &mut self.requested_bytes_by_slot[slot.index()],
            requested_bytes,
        );
        if previous_requested_bytes == 0 {
            self.allocation_count =
                self.allocation_count
                    .checked_add(1)
                    .ok_or(StageGenerationError::Overflow(
                        "controlled allocation count",
                    ))?;
            self.allocated_bytes = self
                .allocated_bytes
                .checked_add(requested_bytes)
                .ok_or(StageGenerationError::Overflow("controlled allocated bytes"))?;
        } else {
            self.reallocation_count =
                self.reallocation_count
                    .checked_add(1)
                    .ok_or(StageGenerationError::Overflow(
                        "controlled reallocation count",
                    ))?;
            self.reallocated_bytes = self.reallocated_bytes.checked_add(requested_bytes).ok_or(
                StageGenerationError::Overflow("controlled reallocated bytes"),
            )?;
            self.freed_bytes = self
                .freed_bytes
                .checked_add(previous_requested_bytes)
                .ok_or(StageGenerationError::Overflow("controlled freed bytes"))?;
        }
        self.cancel_preoccupation(previous_requested_bytes);
        Ok(())
    }

    fn peak_live_requested_bytes(&self) -> u64 {
        self.peak_live_requested_bytes.load(AtomicOrdering::SeqCst)
    }

    fn hard_ceiling_bytes(&self) -> u64 {
        self.hard_ceiling_bytes
    }

    fn snapshot(&self) -> IdentityAllocationSnapshot {
        IdentityAllocationSnapshot {
            allocation_count: self.allocation_count,
            reallocation_count: self.reallocation_count,
            allocated_bytes: self.allocated_bytes,
            reallocated_bytes: self.reallocated_bytes,
            freed_bytes: self.freed_bytes,
            live_requested_bytes: self.live_requested_bytes.load(AtomicOrdering::SeqCst),
            peak_live_requested_bytes: self.peak_live_requested_bytes(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityAllocationSnapshot {
    pub allocation_count: u64,
    pub reallocation_count: u64,
    pub allocated_bytes: u64,
    pub reallocated_bytes: u64,
    pub freed_bytes: u64,
    pub live_requested_bytes: u64,
    pub peak_live_requested_bytes: u64,
}

#[derive(Debug)]
pub(crate) struct IdentityStageBufferPool<const TRACK_ALLOCATIONS: bool> {
    allocations: ControlledAllocationTracker<TRACK_ALLOCATIONS>,
    source_spans: Vec<SourceSpanRecord>,
    source_records: Vec<TypedAstStageRecord>,
    source_payload: Vec<u8>,
    source_scratch: Vec<u64>,
    namespace_preimage_scratch: Vec<u8>,
    typed_ast_records: Vec<TypedAstStageRecord>,
    typed_ast_payload: Vec<u8>,
    hir_records: Vec<HirStageRecord>,
    hir_payload: Vec<u8>,
    mir_records: Vec<MirLirStageRecord>,
    mir_payload: Vec<u8>,
    mir_stable_id_scratch: Vec<[u8; 16]>,
    mir_canonical_identity_scratch: Vec<u8>,
    mir_identity_payload_scratch: Vec<u8>,
    canonical_lir_records: Vec<MirLirStageRecord>,
    canonical_lir_payload: Vec<u8>,
    lir_sort_scratch: Vec<usize>,
    lir_owner_ordinal_scratch: Vec<u32>,
    diagnostics: Vec<u8>,
    output_construction: Vec<u8>,
}

impl Default for IdentityStageBufferPool<false> {
    fn default() -> Self {
        Self::new_for_mode(u64::MAX)
    }
}

impl IdentityStageBufferPool<true> {
    pub(crate) fn controlled_allocation_hard_ceiling_bytes(&self) -> u64 {
        self.allocations.hard_ceiling_bytes()
    }

    pub(crate) fn peak_live_requested_bytes(&self) -> u64 {
        self.allocations.peak_live_requested_bytes()
    }

    pub(crate) fn allocation_snapshot(&self) -> IdentityAllocationSnapshot {
        self.allocations.snapshot()
    }
}

impl<const TRACK_ALLOCATIONS: bool> IdentityStageBufferPool<TRACK_ALLOCATIONS> {
    pub(crate) fn new_for_mode(hard_ceiling_bytes: u64) -> Self {
        Self {
            allocations: ControlledAllocationTracker::new(hard_ceiling_bytes),
            source_spans: Vec::new(),
            source_records: Vec::new(),
            source_payload: Vec::new(),
            source_scratch: Vec::new(),
            namespace_preimage_scratch: Vec::new(),
            typed_ast_records: Vec::new(),
            typed_ast_payload: Vec::new(),
            hir_records: Vec::new(),
            hir_payload: Vec::new(),
            mir_records: Vec::new(),
            mir_payload: Vec::new(),
            mir_stable_id_scratch: Vec::new(),
            mir_canonical_identity_scratch: Vec::new(),
            mir_identity_payload_scratch: Vec::new(),
            canonical_lir_records: Vec::new(),
            canonical_lir_payload: Vec::new(),
            lir_sort_scratch: Vec::new(),
            lir_owner_ordinal_scratch: Vec::new(),
            diagnostics: Vec::new(),
            output_construction: Vec::new(),
        }
    }

    pub(crate) fn retained_capacity_bytes(
        &self,
    ) -> Result<StageRetainedCapacityBytes, StageGenerationError> {
        let source_input = capacity_sum(&[
            capacity_bytes(&self.source_spans)?,
            capacity_bytes(&self.source_records)?,
            capacity_bytes(&self.source_payload)?,
        ])?;
        let typed_ast = capacity_sum(&[
            capacity_bytes(&self.typed_ast_records)?,
            capacity_bytes(&self.typed_ast_payload)?,
        ])?;
        let hir = capacity_sum(&[
            capacity_bytes(&self.hir_records)?,
            capacity_bytes(&self.hir_payload)?,
        ])?;
        let mir = capacity_sum(&[
            capacity_bytes(&self.mir_records)?,
            capacity_bytes(&self.mir_payload)?,
        ])?;
        let canonical_lir = capacity_sum(&[
            capacity_bytes(&self.canonical_lir_records)?,
            capacity_bytes(&self.canonical_lir_payload)?,
        ])?;
        let diagnostics = capacity_bytes(&self.diagnostics)?;
        let scratch = capacity_sum(&[
            capacity_bytes(&self.source_scratch)?,
            capacity_bytes(&self.namespace_preimage_scratch)?,
            capacity_bytes(&self.mir_stable_id_scratch)?,
            capacity_bytes(&self.mir_canonical_identity_scratch)?,
            capacity_bytes(&self.mir_identity_payload_scratch)?,
            capacity_bytes(&self.lir_sort_scratch)?,
            capacity_bytes(&self.lir_owner_ordinal_scratch)?,
        ])?;
        let output_construction = capacity_bytes(&self.output_construction)?;
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

    pub(crate) fn all_lengths_are_zero(&self) -> bool {
        self.source_spans.is_empty()
            && self.source_records.is_empty()
            && self.source_payload.is_empty()
            && self.source_scratch.is_empty()
            && self.namespace_preimage_scratch.is_empty()
            && self.typed_ast_records.is_empty()
            && self.typed_ast_payload.is_empty()
            && self.hir_records.is_empty()
            && self.hir_payload.is_empty()
            && self.mir_records.is_empty()
            && self.mir_payload.is_empty()
            && self.mir_stable_id_scratch.is_empty()
            && self.mir_canonical_identity_scratch.is_empty()
            && self.mir_identity_payload_scratch.is_empty()
            && self.canonical_lir_records.is_empty()
            && self.canonical_lir_payload.is_empty()
            && self.lir_sort_scratch.is_empty()
            && self.lir_owner_ordinal_scratch.is_empty()
            && self.diagnostics.is_empty()
            && self.output_construction.is_empty()
    }
}

#[derive(Debug)]
pub(crate) struct IdentityStageMaterialization {
    source: SourceStage,
    typed: TypedStage,
    hir: HirStage,
    mir: MirStage,
    lir: LirStage,
    pub(crate) output_construction: Vec<u8>,
}

pub(crate) fn prepare_identity_stage_case(
    identity: &IdentityContract,
    stage: &StageContract,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<IdentityStagePlan, StageGenerationError> {
    IdentityStagePlan::prepare(identity, stage, graph_profile, n)
}

pub(crate) fn execute_identity_stage_case(
    generator: &GeneratorContract,
    identity: &IdentityContract,
    stage: &StageContract,
    plan: &IdentityStagePlan,
) -> Result<IdentityStageMaterialization, StageGenerationError> {
    let mut buffers = IdentityStageBufferPool::default();
    execute_identity_stage_case_with_buffers(generator, identity, stage, plan, &mut buffers)
}

pub(crate) fn execute_identity_stage_case_with_buffers<const TRACK_ALLOCATIONS: bool>(
    generator: &GeneratorContract,
    identity: &IdentityContract,
    stage: &StageContract,
    plan: &IdentityStagePlan,
    buffers: &mut IdentityStageBufferPool<TRACK_ALLOCATIONS>,
) -> Result<IdentityStageMaterialization, StageGenerationError> {
    let source = materialize_source_input(generator, identity, stage, plan, buffers)?;
    let typed = lower_source_to_typed_ast(identity, stage, plan, &source, buffers)?;
    let hir = lower_typed_ast_to_hir(identity, stage, plan, &typed, &source.strings, buffers)?;
    let mir = lower_hir_to_mir(identity, stage, plan, &hir, &source.strings, buffers)?;
    let lir = canonicalize_mir(stage, plan, &mir, buffers)?;
    let output_construction = construct_output(identity, plan, &lir, buffers)?;

    Ok(IdentityStageMaterialization {
        source,
        typed,
        hir,
        mir,
        lir,
        output_construction,
    })
}

pub(crate) fn recycle_identity_stage_case<const TRACK_ALLOCATIONS: bool>(
    buffers: &mut IdentityStageBufferPool<TRACK_ALLOCATIONS>,
    output: IdentityStageCaseOutput,
) -> IdentityStageSummary {
    let IdentityStageCaseOutput {
        summary,
        string_bytes: _,
        source_spans,
        source_input_records,
        source_input_payload,
        typed_ast_records,
        typed_ast_payload,
        hir_records,
        hir_payload,
        mir_records,
        mir_payload,
        canonical_lir_records,
        canonical_lir_payload,
        diagnostics,
        scratch_capacity_bytes: _,
        output_construction,
        source_scratch,
        namespace_preimage_scratch,
        mir_stable_id_scratch,
        mir_canonical_identity_scratch,
        mir_identity_payload_scratch,
        lir_sort_scratch,
        lir_owner_ordinal_scratch,
    } = output;
    store_cleared(&mut buffers.source_spans, source_spans);
    store_cleared(&mut buffers.source_records, source_input_records);
    store_cleared(&mut buffers.source_payload, source_input_payload);
    store_cleared(&mut buffers.source_scratch, source_scratch);
    store_cleared(
        &mut buffers.namespace_preimage_scratch,
        namespace_preimage_scratch,
    );
    store_cleared(&mut buffers.typed_ast_records, typed_ast_records);
    store_cleared(&mut buffers.typed_ast_payload, typed_ast_payload);
    store_cleared(&mut buffers.hir_records, hir_records);
    store_cleared(&mut buffers.hir_payload, hir_payload);
    store_cleared(&mut buffers.mir_records, mir_records);
    store_cleared(&mut buffers.mir_payload, mir_payload);
    store_cleared(&mut buffers.mir_stable_id_scratch, mir_stable_id_scratch);
    store_cleared(
        &mut buffers.mir_canonical_identity_scratch,
        mir_canonical_identity_scratch,
    );
    store_cleared(
        &mut buffers.mir_identity_payload_scratch,
        mir_identity_payload_scratch,
    );
    store_cleared(&mut buffers.canonical_lir_records, canonical_lir_records);
    store_cleared(&mut buffers.canonical_lir_payload, canonical_lir_payload);
    store_cleared(&mut buffers.lir_sort_scratch, lir_sort_scratch);
    store_cleared(
        &mut buffers.lir_owner_ordinal_scratch,
        lir_owner_ordinal_scratch,
    );
    store_cleared(&mut buffers.diagnostics, diagnostics);
    store_cleared(&mut buffers.output_construction, output_construction);
    summary
}

pub(crate) fn finalize_identity_stage_case(
    plan: &IdentityStagePlan,
    materialized: IdentityStageMaterialization,
) -> Result<IdentityStageCaseOutput, StageGenerationError> {
    let IdentityStageMaterialization {
        source,
        typed,
        hir,
        mir,
        lir,
        output_construction,
    } = materialized;
    // 以下摘要与验证工作必须位于未来正式外层计时区之外。
    let semantic_digest_sha256 = encode_lower_hex(&Sha256::digest(&output_construction));
    let summary = plan.summary(semantic_digest_sha256);
    verify_materialized_shapes(
        &summary,
        &source,
        &typed,
        &hir,
        &mir,
        &lir,
        &output_construction,
    )?;
    let string_bytes =
        source.payload[source.strings.string_start..source.strings.string_end].to_vec();
    let scratch_capacity_bytes = lir
        .scratch_capacity_bytes
        .max(as_u64(source.scratch.capacity(), "source scratch capacity")? * 8);

    Ok(IdentityStageCaseOutput {
        summary,
        string_bytes,
        source_spans: source.spans,
        source_input_records: source.records,
        source_input_payload: source.payload,
        typed_ast_records: typed.records,
        typed_ast_payload: typed.payload,
        hir_records: hir.records,
        hir_payload: hir.payload,
        mir_records: mir.records,
        mir_payload: mir.payload,
        canonical_lir_records: lir.records,
        canonical_lir_payload: lir.payload,
        diagnostics: Vec::new(),
        scratch_capacity_bytes,
        output_construction,
        source_scratch: source.scratch,
        namespace_preimage_scratch: source.namespace_preimage_scratch,
        mir_stable_id_scratch: mir.stable_id_scratch,
        mir_canonical_identity_scratch: mir.canonical_identity_scratch,
        mir_identity_payload_scratch: mir.identity_payload_scratch,
        lir_sort_scratch: lir.sort_scratch,
        lir_owner_ordinal_scratch: lir.owner_ordinal_scratch,
    })
}

pub(crate) fn build_identity_stage_case(
    generator: &GeneratorContract,
    identity: &IdentityContract,
    stage: &StageContract,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<IdentityStageCaseOutput, StageGenerationError> {
    let plan = prepare_identity_stage_case(identity, stage, graph_profile, n)?;
    let materialized = execute_identity_stage_case(generator, identity, stage, &plan)?;
    finalize_identity_stage_case(&plan, materialized)
}

fn materialize_source_input<const TRACK_ALLOCATIONS: bool>(
    generator: &GeneratorContract,
    identity: &IdentityContract,
    stage: &StageContract,
    plan: &IdentityStagePlan,
    buffers: &mut IdentityStageBufferPool<TRACK_ALLOCATIONS>,
) -> Result<SourceStage, StageGenerationError> {
    let source_payload_capacity = to_usize(
        plan.stages.source_input.payload_logical_bytes,
        "source input payload",
    )?;
    let source_record_capacity = to_usize(
        plan.stages.source_input.record_count,
        "source input records",
    )?;
    let span_capacity = to_usize(plan.counts.source_span_count, "source spans")?;
    let scratch_capacity = to_usize(
        plan.counts
            .module_count
            .max(plan.counts.source_reference_count)
            .max(plan.counts.semantic_output_record),
        "source scratch",
    )?;

    let mut payload = take_reusable(
        &mut buffers.source_payload,
        source_payload_capacity,
        "source input payload",
        &mut buffers.allocations,
        ControlledBufferSlot::SourcePayload,
    )?;
    let mut spans = take_reusable(
        &mut buffers.source_spans,
        span_capacity,
        "source spans",
        &mut buffers.allocations,
        ControlledBufferSlot::SourceSpans,
    )?;
    append_source_documents(&mut payload, &mut spans, identity, stage, plan)?;
    if as_u64(payload.len(), "source bytes")? != plan.counts.source_byte_count {
        return Err(StageGenerationError::MaterializedMismatch("source bytes"));
    }
    let string_start = payload.len();

    let mut records = take_reusable(
        &mut buffers.source_records,
        source_record_capacity,
        "source input records",
        &mut buffers.allocations,
        ControlledBufferSlot::SourceRecords,
    )?;
    append_module_name_strings(&mut payload, &mut records, stage, plan)?;
    append_source_document_key_strings(&mut payload, plan)?;
    let mut scratch = take_reusable(
        &mut buffers.source_scratch,
        scratch_capacity,
        "source scratch",
        &mut buffers.allocations,
        ControlledBufferSlot::SourceScratch,
    )?;
    append_import_strings_and_records(
        &mut payload,
        &mut records,
        &mut scratch,
        generator,
        stage,
        plan,
    )?;

    let namespace_base = payload.len();
    let mut namespace_preimage_scratch = take_reusable(
        &mut buffers.namespace_preimage_scratch,
        128,
        "namespace preimage scratch",
        &mut buffers.allocations,
        ControlledBufferSlot::NamespacePreimageScratch,
    )?;
    append_namespace_strings(
        &mut payload,
        &mut namespace_preimage_scratch,
        generator,
        identity,
        plan,
    )?;
    let profiled_key_base = payload.len();
    append_profiled_key_strings(&mut payload, identity, plan)?;
    let reference_base = payload.len();
    append_reference_strings(&mut payload, identity, plan)?;
    if plan.graph_profile == GraphProfileId::SharedFaninDag {
        payload.extend_from_slice(stage.shared_constant_name.as_bytes());
        payload.extend_from_slice(stage.shared_constant_value.as_bytes());
    }
    let string_end = payload.len();
    if as_u64(string_end - string_start, "string bytes")? != plan.counts.total_string_bytes {
        return Err(StageGenerationError::MaterializedMismatch("string bytes"));
    }

    let strings = StringLayout {
        string_start,
        namespace_base,
        profiled_key_base,
        reference_base,
        string_end,
    };
    append_permuted_source_records(
        &mut records,
        &mut scratch,
        generator,
        identity,
        stage,
        plan,
        strings,
    )?;
    if records.len() != source_record_capacity || spans.len() != span_capacity {
        return Err(StageGenerationError::MaterializedMismatch(
            "source input record count",
        ));
    }
    scratch.clear();
    namespace_preimage_scratch.clear();

    Ok(SourceStage {
        spans,
        records,
        payload,
        strings,
        scratch,
        namespace_preimage_scratch,
    })
}

fn append_source_documents(
    payload: &mut Vec<u8>,
    spans: &mut Vec<SourceSpanRecord>,
    identity: &IdentityContract,
    stage: &StageContract,
    plan: &IdentityStagePlan,
) -> Result<(), StageGenerationError> {
    for module_ordinal in 0..u32_from_u64(plan.counts.module_count, "module count")? {
        let mut line = 1_u32;
        let declaration_count = module_declaration_count(identity, plan, module_ordinal)?;
        let reference_count = module_reference_count(plan, module_ordinal)?;
        let relation_count = module_relation_count(plan, module_ordinal)?;
        for local in 0..declaration_count {
            append_source_token(
                payload,
                spans,
                module_ordinal,
                &mut line,
                &stage.declaration_token,
                local,
            )?;
        }
        for local in 0..reference_count {
            append_source_token(
                payload,
                spans,
                module_ordinal,
                &mut line,
                &stage.reference_token,
                local,
            )?;
        }
        for local in 0..relation_count {
            append_source_token(
                payload,
                spans,
                module_ordinal,
                &mut line,
                &stage.relation_token,
                local,
            )?;
        }
    }
    Ok(())
}

fn append_source_token(
    payload: &mut Vec<u8>,
    spans: &mut Vec<SourceSpanRecord>,
    module_ordinal: u32,
    line: &mut u32,
    token: &str,
    local: u32,
) -> Result<(), StageGenerationError> {
    let token_start = payload.len();
    payload.extend_from_slice(token.as_bytes());
    payload.push(b'/');
    append_hex_u32(payload, local);
    payload.push(b'\n');
    let token_length = payload
        .len()
        .checked_sub(token_start)
        .ok_or(StageGenerationError::Overflow("source token length"))?;
    spans.push(SourceSpanRecord {
        source_document_ordinal: module_ordinal,
        start_line: *line,
        start_column: 1,
        end_line: *line,
        end_column: u32::try_from(token_length)
            .map_err(|_| StageGenerationError::Overflow("source token length"))?,
    });
    *line = line
        .checked_add(1)
        .ok_or(StageGenerationError::Overflow("source line"))?;
    Ok(())
}

fn append_module_name_strings(
    payload: &mut Vec<u8>,
    records: &mut Vec<TypedAstStageRecord>,
    stage: &StageContract,
    plan: &IdentityStagePlan,
) -> Result<(), StageGenerationError> {
    for module_ordinal in 0..u32_from_u64(plan.counts.module_count, "module count")? {
        let start = payload.len();
        append_module_name(payload, plan, module_ordinal)?;
        records.push(TypedAstStageRecord {
            record_kind: stage.record_kind_module,
            entity_kind: ENTITY_KIND_ABSENT,
            module_ordinal,
            source_span_ordinal: stage.absent_ordinal,
            owner_local_index: stage.absent_ordinal,
            payload_offset: u64_from_usize(start, "module name offset")?,
            payload_length: u64_from_usize(payload.len() - start, "module name length")?,
        });
    }
    Ok(())
}

fn append_source_document_key_strings(
    payload: &mut Vec<u8>,
    plan: &IdentityStagePlan,
) -> Result<(), StageGenerationError> {
    for module_ordinal in 0..u32_from_u64(plan.counts.module_count, "module count")? {
        payload.extend_from_slice(b"source/");
        payload.extend_from_slice(plan.graph_profile.as_str().as_bytes());
        payload.push(b'/');
        append_module_name(payload, plan, module_ordinal)?;
        payload.extend_from_slice(b".lfsynthetic");
    }
    Ok(())
}

fn append_import_strings_and_records(
    payload: &mut Vec<u8>,
    records: &mut Vec<TypedAstStageRecord>,
    scratch: &mut Vec<u64>,
    generator: &GeneratorContract,
    stage: &StageContract,
    plan: &IdentityStagePlan,
) -> Result<(), StageGenerationError> {
    for source_module in 0..u32_from_u64(plan.counts.module_count, "module count")? {
        fill_import_targets(scratch, plan, source_module)?;
        permute_in_place(
            scratch,
            generator,
            SequenceKind::Imports,
            module_seed_ordinal(plan, source_module)?,
        );
        for (input_ordinal, target_module) in scratch.iter().copied().enumerate() {
            let target_module = u32::try_from(target_module)
                .map_err(|_| StageGenerationError::Overflow("target module ordinal"))?;
            let start = payload.len();
            append_module_name(payload, plan, target_module)?;
            records.push(TypedAstStageRecord {
                record_kind: stage.record_kind_import,
                entity_kind: ENTITY_KIND_ABSENT,
                module_ordinal: source_module,
                source_span_ordinal: stage.absent_ordinal,
                owner_local_index: u32::try_from(input_ordinal)
                    .map_err(|_| StageGenerationError::Overflow("import input ordinal"))?,
                payload_offset: u64_from_usize(start, "import string offset")?,
                payload_length: u64_from_usize(payload.len() - start, "import string length")?,
            });
        }
    }
    Ok(())
}

fn append_namespace_strings(
    payload: &mut Vec<u8>,
    preimage: &mut Vec<u8>,
    generator: &GeneratorContract,
    identity: &IdentityContract,
    plan: &IdentityStagePlan,
) -> Result<(), StageGenerationError> {
    for unit_index in 0..plan.n {
        let module_ordinal = plan
            .unit_module_base
            .checked_add(unit_index)
            .ok_or(StageGenerationError::Overflow("unit module ordinal"))?;
        let module_name = module_name_buffer(plan, module_ordinal)?;
        let namespace = derive_namespace_ascii(
            generator,
            plan.graph_profile,
            module_name.as_slice(),
            preimage,
        );
        for _ in &identity.bindings {
            payload.extend_from_slice(&namespace);
        }
    }
    Ok(())
}

fn append_profiled_key_strings(
    payload: &mut Vec<u8>,
    identity: &IdentityContract,
    plan: &IdentityStagePlan,
) -> Result<(), StageGenerationError> {
    for unit_index in 0..plan.n {
        for binding in &identity.bindings {
            for field in &binding.fields {
                if let IdentityFieldValue::ProfiledKey { kind, local } = field.value {
                    append_hex_u16(payload, kind);
                    payload.push(b'/');
                    append_hex_u32(payload, unit_index);
                    payload.push(b'/');
                    append_hex_u32(payload, local);
                }
            }
        }
    }
    Ok(())
}

fn append_reference_strings(
    payload: &mut Vec<u8>,
    identity: &IdentityContract,
    plan: &IdentityStagePlan,
) -> Result<(), StageGenerationError> {
    for unit_index in 0..plan.n {
        let module = unit_module_ordinal(plan, unit_index)?;
        for binding in &identity.bindings {
            for field in &binding.fields {
                if let IdentityFieldValue::StableId { kind, .. } = field.value {
                    append_reference_spelling(payload, kind, module, 0);
                }
            }
        }
    }
    for unit_index in 0..plan.n {
        let module = unit_module_ordinal(plan, unit_index)?;
        for relation in &identity.owner_relations {
            append_reference_spelling(payload, relation.parent_kind, module, 0);
        }
    }
    for cross_ordinal in 0..u32_from_u64(
        plan.counts.cross_module_reference_count,
        "cross module references",
    )? {
        let target = cross_reference_target(plan, cross_ordinal)?;
        append_reference_spelling(payload, target.0, target.1, 0);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn append_permuted_source_records(
    records: &mut Vec<TypedAstStageRecord>,
    scratch: &mut Vec<u64>,
    generator: &GeneratorContract,
    identity: &IdentityContract,
    stage: &StageContract,
    plan: &IdentityStagePlan,
    strings: StringLayout,
) -> Result<(), StageGenerationError> {
    let mut span_base = 0_u32;
    for module_ordinal in 0..u32_from_u64(plan.counts.module_count, "module count")? {
        let declaration_count = module_declaration_count(identity, plan, module_ordinal)?;
        let reference_count = module_reference_count(plan, module_ordinal)?;
        let relation_count = module_relation_count(plan, module_ordinal)?;
        let reference_span_base = span_base
            .checked_add(declaration_count)
            .ok_or(StageGenerationError::Overflow("reference span base"))?;
        let relation_span_base = reference_span_base
            .checked_add(reference_count)
            .ok_or(StageGenerationError::Overflow("relation span base"))?;
        let module_seed = module_seed_ordinal(plan, module_ordinal)?;

        fill_ordinals(scratch, declaration_count);
        permute_in_place(scratch, generator, SequenceKind::Declarations, module_seed);
        for canonical_ordinal in scratch.iter().copied() {
            let canonical_ordinal = u32::try_from(canonical_ordinal)
                .map_err(|_| StageGenerationError::Overflow("declaration ordinal"))?;
            let (entity_kind, payload_offset, payload_length) = declaration_source_value(
                identity,
                stage,
                plan,
                strings,
                module_ordinal,
                canonical_ordinal,
            )?;
            records.push(TypedAstStageRecord {
                record_kind: stage.record_kind_declaration,
                entity_kind,
                module_ordinal,
                source_span_ordinal: span_base
                    .checked_add(canonical_ordinal)
                    .ok_or(StageGenerationError::Overflow("declaration span"))?,
                owner_local_index: 0,
                payload_offset,
                payload_length,
            });
        }

        fill_ordinals(scratch, reference_count);
        permute_in_place(scratch, generator, SequenceKind::References, module_seed);
        for canonical_ordinal in scratch.iter().copied() {
            let canonical_ordinal = u32::try_from(canonical_ordinal)
                .map_err(|_| StageGenerationError::Overflow("reference ordinal"))?;
            let descriptor =
                source_reference_descriptor(identity, plan, module_ordinal, canonical_ordinal)?;
            records.push(TypedAstStageRecord {
                record_kind: stage.record_kind_reference,
                entity_kind: descriptor.source_entity_kind,
                module_ordinal,
                source_span_ordinal: reference_span_base
                    .checked_add(canonical_ordinal)
                    .ok_or(StageGenerationError::Overflow("reference span"))?,
                owner_local_index: canonical_ordinal,
                payload_offset: u64_from_usize(
                    strings
                        .reference_base
                        .checked_add(
                            usize::try_from(descriptor.global_string_ordinal)
                                .map_err(|_| {
                                    StageGenerationError::Overflow("reference string ordinal")
                                })?
                                .checked_mul(30)
                                .ok_or(StageGenerationError::Overflow("reference string offset"))?,
                        )
                        .ok_or(StageGenerationError::Overflow("reference string offset"))?,
                    "reference string offset",
                )?,
                payload_length: 30,
            });
        }

        fill_ordinals(scratch, relation_count);
        permute_in_place(scratch, generator, SequenceKind::Relations, module_seed);
        for relation_ordinal in scratch.iter().copied() {
            let relation_ordinal = u32::try_from(relation_ordinal)
                .map_err(|_| StageGenerationError::Overflow("relation ordinal"))?;
            let unit_index = module_to_unit(plan, module_ordinal)
                .ok_or(StageGenerationError::InvalidModuleOrdinal(module_ordinal))?;
            let relation = identity
                .owner_relations
                .get(
                    usize::try_from(relation_ordinal)
                        .map_err(|_| StageGenerationError::Overflow("relation ordinal"))?,
                )
                .ok_or(StageGenerationError::MaterializedMismatch(
                    "relation binding",
                ))?;
            let global_reference = u64::from(plan.n)
                .checked_mul(u64::from(plan.stable_fields_per_unit))
                .and_then(|base| {
                    base.checked_add(
                        u64::from(unit_index)
                            .checked_mul(u64::from(plan.relation_count_per_unit))?,
                    )
                })
                .and_then(|base| base.checked_add(u64::from(relation_ordinal)))
                .ok_or(StageGenerationError::Overflow("relation reference ordinal"))?;
            records.push(TypedAstStageRecord {
                record_kind: stage.record_kind_relation,
                entity_kind: relation.child_kind,
                module_ordinal,
                source_span_ordinal: relation_span_base
                    .checked_add(relation_ordinal)
                    .ok_or(StageGenerationError::Overflow("relation span"))?,
                owner_local_index: relation_ordinal,
                payload_offset: u64_from_usize(
                    strings
                        .reference_base
                        .checked_add(
                            usize::try_from(global_reference)
                                .map_err(|_| {
                                    StageGenerationError::Overflow("relation reference ordinal")
                                })?
                                .checked_mul(30)
                                .ok_or(StageGenerationError::Overflow(
                                    "relation reference offset",
                                ))?,
                        )
                        .ok_or(StageGenerationError::Overflow("relation reference offset"))?,
                    "relation reference offset",
                )?,
                payload_length: 30,
            });
        }

        span_base = relation_span_base
            .checked_add(relation_count)
            .ok_or(StageGenerationError::Overflow("module span count"))?;
    }
    Ok(())
}

fn lower_source_to_typed_ast<const TRACK_ALLOCATIONS: bool>(
    identity: &IdentityContract,
    stage: &StageContract,
    plan: &IdentityStagePlan,
    source: &SourceStage,
    buffers: &mut IdentityStageBufferPool<TRACK_ALLOCATIONS>,
) -> Result<TypedStage, StageGenerationError> {
    let mut records = take_reusable(
        &mut buffers.typed_ast_records,
        to_usize(plan.stages.typed_ast.record_count, "typed AST records")?,
        "typed AST records",
        &mut buffers.allocations,
        ControlledBufferSlot::TypedAstRecords,
    )?;
    for record in &source.records {
        records.push(*record);
        if record.record_kind != stage.record_kind_declaration
            || record.entity_kind == SHARED_CONSTANT_ENTITY_KIND
        {
            continue;
        }
        let binding_index = binding_index(identity, record.entity_kind)?;
        let binding = &identity.bindings[binding_index];
        let unit_index = module_to_unit(plan, record.module_ordinal).ok_or(
            StageGenerationError::InvalidModuleOrdinal(record.module_ordinal),
        )?;
        let mut profiled_before = 0_u32;
        let mut stable_before = 0_u32;
        for preceding in identity.bindings.iter().take(binding_index) {
            for field in &preceding.fields {
                match field.value {
                    IdentityFieldValue::ProfiledKey { .. } => {
                        profiled_before = profiled_before
                            .checked_add(1)
                            .ok_or(StageGenerationError::Overflow("profiled field ordinal"))?;
                    }
                    IdentityFieldValue::StableId { .. } => {
                        stable_before = stable_before
                            .checked_add(1)
                            .ok_or(StageGenerationError::Overflow("stable field ordinal"))?;
                    }
                    IdentityFieldValue::Namespace => {}
                }
            }
        }
        let mut binding_profiled = 0_u32;
        let mut binding_stable = 0_u32;
        for field in &binding.fields {
            let (payload_offset, payload_length) = match field.value {
                IdentityFieldValue::Namespace => (record.payload_offset, 32),
                IdentityFieldValue::ProfiledKey { .. } => {
                    let ordinal = u64::from(unit_index)
                        .checked_mul(u64::from(plan.profiled_fields_per_unit))
                        .and_then(|base| base.checked_add(u64::from(profiled_before)))
                        .and_then(|base| base.checked_add(u64::from(binding_profiled)))
                        .ok_or(StageGenerationError::Overflow("profiled key ordinal"))?;
                    binding_profiled = binding_profiled
                        .checked_add(1)
                        .ok_or(StageGenerationError::Overflow("profiled field ordinal"))?;
                    (
                        u64_from_usize(
                            source
                                .strings
                                .profiled_key_base
                                .checked_add(
                                    usize::try_from(ordinal)
                                        .map_err(|_| {
                                            StageGenerationError::Overflow("profiled key ordinal")
                                        })?
                                        .checked_mul(20)
                                        .ok_or(StageGenerationError::Overflow(
                                            "profiled key offset",
                                        ))?,
                                )
                                .ok_or(StageGenerationError::Overflow("profiled key offset"))?,
                            "profiled key offset",
                        )?,
                        20,
                    )
                }
                IdentityFieldValue::StableId { .. } => {
                    let ordinal = u64::from(unit_index)
                        .checked_mul(u64::from(plan.stable_fields_per_unit))
                        .and_then(|base| base.checked_add(u64::from(stable_before)))
                        .and_then(|base| base.checked_add(u64::from(binding_stable)))
                        .ok_or(StageGenerationError::Overflow("stable reference ordinal"))?;
                    binding_stable = binding_stable
                        .checked_add(1)
                        .ok_or(StageGenerationError::Overflow("stable field ordinal"))?;
                    (
                        u64_from_usize(
                            source
                                .strings
                                .reference_base
                                .checked_add(
                                    usize::try_from(ordinal)
                                        .map_err(|_| {
                                            StageGenerationError::Overflow(
                                                "stable reference ordinal",
                                            )
                                        })?
                                        .checked_mul(30)
                                        .ok_or(StageGenerationError::Overflow(
                                            "stable reference offset",
                                        ))?,
                                )
                                .ok_or(StageGenerationError::Overflow("stable reference offset"))?,
                            "stable reference offset",
                        )?,
                        30,
                    )
                }
            };
            records.push(TypedAstStageRecord {
                record_kind: stage.record_kind_identity_field,
                entity_kind: binding.entity_kind_code,
                module_ordinal: record.module_ordinal,
                source_span_ordinal: record.source_span_ordinal,
                owner_local_index: u32::from(field.tag),
                payload_offset,
                payload_length,
            });
        }
    }

    let mut payload = take_reusable(
        &mut buffers.typed_ast_payload,
        to_usize(
            plan.stages.typed_ast.payload_logical_bytes,
            "typed AST payload",
        )?,
        "typed AST payload",
        &mut buffers.allocations,
        ControlledBufferSlot::TypedAstPayload,
    )?;
    payload.extend_from_slice(&source.payload);
    encode_source_spans(&mut payload, &source.spans);
    records.sort_by(|left, right| typed_record_order(left, right, &payload, stage.absent_ordinal));
    Ok(TypedStage { records, payload })
}

fn lower_typed_ast_to_hir<const TRACK_ALLOCATIONS: bool>(
    identity: &IdentityContract,
    stage: &StageContract,
    plan: &IdentityStagePlan,
    typed: &TypedStage,
    source_strings: &StringLayout,
    buffers: &mut IdentityStageBufferPool<TRACK_ALLOCATIONS>,
) -> Result<HirStage, StageGenerationError> {
    let string_bytes = source_strings.string_end - source_strings.string_start;
    let mut payload = take_reusable(
        &mut buffers.hir_payload,
        to_usize(plan.stages.hir.payload_logical_bytes, "HIR payload")?,
        "HIR payload",
        &mut buffers.allocations,
        ControlledBufferSlot::HirPayload,
    )?;
    payload
        .extend_from_slice(&typed.payload[source_strings.string_start..source_strings.string_end]);
    let mut records = take_reusable(
        &mut buffers.hir_records,
        to_usize(plan.stages.hir.record_count, "HIR records")?,
        "HIR records",
        &mut buffers.allocations,
        ControlledBufferSlot::HirRecords,
    )?;

    for record in &typed.records {
        let mut hir_record = HirStageRecord {
            record_kind: record.record_kind,
            entity_kind: record.entity_kind,
            module_ordinal: record.module_ordinal,
            symbol_ordinal: stage.absent_ordinal,
            resolved_target_ordinal: stage.absent_ordinal,
            payload_offset: 0,
            payload_length: 0,
        };
        if record.record_kind == stage.record_kind_import {
            let target = parse_module_name(
                plan,
                payload_slice(&typed.payload, record.payload_offset, record.payload_length)?,
            )?;
            hir_record.resolved_target_ordinal = target;
            set_hir_operands(&mut hir_record, &mut payload, string_bytes, &[target])?;
        } else if record.record_kind == stage.record_kind_declaration {
            hir_record.record_kind = stage.record_kind_symbol;
            hir_record.symbol_ordinal =
                symbol_ordinal(plan, identity, record.module_ordinal, record.entity_kind)?;
        } else if record.record_kind == stage.record_kind_identity_field {
            hir_record.symbol_ordinal =
                symbol_ordinal(plan, identity, record.module_ordinal, record.entity_kind)?;
            let field = identity_field(identity, record.entity_kind, record.owner_local_index)?;
            if matches!(field.value, IdentityFieldValue::StableId { .. }) {
                let target = parse_reference_spelling(payload_slice(
                    &typed.payload,
                    record.payload_offset,
                    record.payload_length,
                )?)?;
                hir_record.resolved_target_ordinal =
                    symbol_ordinal(plan, identity, target.module_ordinal, target.entity_kind)?;
            }
            set_hir_operands(
                &mut hir_record,
                &mut payload,
                string_bytes,
                &[record.owner_local_index],
            )?;
        } else if record.record_kind == stage.record_kind_reference {
            let target = parse_reference_spelling(payload_slice(
                &typed.payload,
                record.payload_offset,
                record.payload_length,
            )?)?;
            let target_symbol =
                symbol_ordinal(plan, identity, target.module_ordinal, target.entity_kind)?;
            hir_record.resolved_target_ordinal = target_symbol;
            set_hir_operands(
                &mut hir_record,
                &mut payload,
                string_bytes,
                &[target_symbol],
            )?;
        } else if record.record_kind == stage.record_kind_relation {
            let target = parse_reference_spelling(payload_slice(
                &typed.payload,
                record.payload_offset,
                record.payload_length,
            )?)?;
            let child_symbol =
                symbol_ordinal(plan, identity, record.module_ordinal, record.entity_kind)?;
            let parent_symbol =
                symbol_ordinal(plan, identity, target.module_ordinal, target.entity_kind)?;
            hir_record.symbol_ordinal = child_symbol;
            hir_record.resolved_target_ordinal = parent_symbol;
            set_hir_operands(
                &mut hir_record,
                &mut payload,
                string_bytes,
                &[child_symbol, parent_symbol],
            )?;
        }
        records.push(hir_record);
    }
    records.sort_by(|left, right| hir_record_order(left, right, &payload, stage.absent_ordinal));
    Ok(HirStage { records, payload })
}

fn lower_hir_to_mir<const TRACK_ALLOCATIONS: bool>(
    identity: &IdentityContract,
    stage: &StageContract,
    plan: &IdentityStagePlan,
    hir: &HirStage,
    source_strings: &StringLayout,
    buffers: &mut IdentityStageBufferPool<TRACK_ALLOCATIONS>,
) -> Result<MirStage, StageGenerationError> {
    let mut records = take_reusable(
        &mut buffers.mir_records,
        to_usize(plan.stages.mir.record_count, "MIR records")?,
        "MIR records",
        &mut buffers.allocations,
        ControlledBufferSlot::MirRecords,
    )?;
    let mut payload = take_reusable(
        &mut buffers.mir_payload,
        to_usize(plan.stages.mir.payload_logical_bytes, "MIR payload")?,
        "MIR payload",
        &mut buffers.allocations,
        ControlledBufferSlot::MirPayload,
    )?;
    let mut stable_ids = take_reusable(
        &mut buffers.mir_stable_id_scratch,
        usize::try_from(plan.binding_count)
            .map_err(|_| StageGenerationError::Overflow("binding count"))?,
        "MIR stable ID scratch",
        &mut buffers.allocations,
        ControlledBufferSlot::MirStableIdScratch,
    )?;
    let mut canonical_identity = take_reusable(
        &mut buffers.mir_canonical_identity_scratch,
        256,
        "MIR canonical identity scratch",
        &mut buffers.allocations,
        ControlledBufferSlot::MirCanonicalIdentityScratch,
    )?;
    let mut identity_payload = take_reusable(
        &mut buffers.mir_identity_payload_scratch,
        256,
        "MIR identity payload scratch",
        &mut buffers.allocations,
        ControlledBufferSlot::MirIdentityPayloadScratch,
    )?;

    for unit_index in 0..plan.n {
        stable_ids.clear();
        let module_ordinal = unit_module_ordinal(plan, unit_index)?;
        let module_records = hir_module_records(&hir.records, module_ordinal);
        for (current_binding_index, binding) in identity.bindings.iter().enumerate() {
            canonical_identity.clear();
            identity_payload.clear();
            canonical_identity.extend_from_slice(IDENTITY_MAGIC);
            canonical_identity
                .extend_from_slice(&identity.identity_encoding_version().to_le_bytes());
            canonical_identity.extend_from_slice(&binding.entity_kind_code.to_le_bytes());
            identity_payload.extend_from_slice(
                &u16::try_from(binding.fields.len())
                    .map_err(|_| StageGenerationError::Overflow("identity field count"))?
                    .to_le_bytes(),
            );

            let mut profiled_in_binding = 0_u32;
            for field in &binding.fields {
                let hir_field = find_hir_identity_field(
                    module_records,
                    &hir.payload,
                    stage,
                    binding.entity_kind_code,
                    field.tag,
                )?;
                let value = match field.value {
                    IdentityFieldValue::Namespace => {
                        let declaration_ordinal = u64::from(unit_index)
                            .checked_mul(u64::from(plan.binding_count))
                            .and_then(|base| {
                                base.checked_add(u64::try_from(current_binding_index).ok()?)
                            })
                            .ok_or(StageGenerationError::Overflow("namespace ordinal"))?;
                        string_at(
                            &hir.payload,
                            source_strings.namespace_base - source_strings.string_start,
                            declaration_ordinal,
                            32,
                        )?
                    }
                    IdentityFieldValue::ProfiledKey { .. } => {
                        let before = profiled_fields_before(identity, current_binding_index)?;
                        let ordinal = u64::from(unit_index)
                            .checked_mul(u64::from(plan.profiled_fields_per_unit))
                            .and_then(|base| base.checked_add(u64::from(before)))
                            .and_then(|base| base.checked_add(u64::from(profiled_in_binding)))
                            .ok_or(StageGenerationError::Overflow("profiled key ordinal"))?;
                        profiled_in_binding = profiled_in_binding
                            .checked_add(1)
                            .ok_or(StageGenerationError::Overflow("profiled key ordinal"))?;
                        string_at(
                            &hir.payload,
                            source_strings.profiled_key_base - source_strings.string_start,
                            ordinal,
                            20,
                        )?
                    }
                    IdentityFieldValue::StableId { kind, .. } => {
                        let target_index = binding_index(identity, kind)?;
                        let expected_symbol = symbol_ordinal(plan, identity, module_ordinal, kind)?;
                        if hir_field.resolved_target_ordinal != expected_symbol
                            || target_index >= stable_ids.len()
                        {
                            return Err(StageGenerationError::InvalidSymbol {
                                module_ordinal,
                                entity_kind: kind,
                            });
                        }
                        stable_ids[target_index].as_slice()
                    }
                };
                identity_payload.extend_from_slice(&field.tag.to_le_bytes());
                identity_payload.extend_from_slice(
                    &u32::try_from(value.len())
                        .map_err(|_| StageGenerationError::Overflow("identity field length"))?
                        .to_le_bytes(),
                );
                identity_payload.extend_from_slice(value);
            }
            canonical_identity.extend_from_slice(&identity_payload);
            let mut hasher = blake3::Hasher::new();
            hasher.update(STABLE_ID_DOMAIN);
            hasher.update(&canonical_identity);
            let digest = hasher.finalize();
            let mut stable_id = [0_u8; 16];
            stable_id.copy_from_slice(&digest.as_bytes()[..16]);
            stable_ids.push(stable_id);

            let payload_offset = u64_from_usize(payload.len(), "MIR payload offset")?;
            payload.extend_from_slice(&identity_payload);
            records.push(MirLirStageRecord {
                record_kind: 1,
                entity_kind: binding.entity_kind_code,
                stable_id,
                owner_ordinal: stage.absent_ordinal,
                local_index: ABSENT_LOCAL_INDEX,
                payload_offset,
                payload_length: u64_from_usize(identity_payload.len(), "MIR payload length")?,
            });
        }

        for relation in &identity.owner_relations {
            let child_index = binding_index(identity, relation.child_kind)?;
            let parent_index = binding_index(identity, relation.parent_kind)?;
            let child_symbol = symbol_ordinal(plan, identity, module_ordinal, relation.child_kind)?;
            let parent_symbol =
                symbol_ordinal(plan, identity, module_ordinal, relation.parent_kind)?;
            let relation_record = module_records
                .iter()
                .find(|record| {
                    record.record_kind == stage.record_kind_relation
                        && record.entity_kind == relation.child_kind
                        && record.symbol_ordinal == child_symbol
                        && record.resolved_target_ordinal == parent_symbol
                })
                .ok_or(StageGenerationError::InvalidSymbol {
                    module_ordinal,
                    entity_kind: relation.child_kind,
                })?;
            validate_relation_operands(relation_record, &hir.payload, child_symbol, parent_symbol)?;
            let payload_offset = u64_from_usize(payload.len(), "MIR relation payload offset")?;
            payload.extend_from_slice(&relation.parent_kind.to_le_bytes());
            payload.extend_from_slice(&stable_ids[parent_index]);
            records.push(MirLirStageRecord {
                record_kind: 2,
                entity_kind: relation.child_kind,
                stable_id: stable_ids[child_index],
                owner_ordinal: stage.absent_ordinal,
                local_index: ABSENT_LOCAL_INDEX,
                payload_offset,
                payload_length: 18,
            });
        }
    }
    stable_ids.clear();
    canonical_identity.clear();
    identity_payload.clear();
    Ok(MirStage {
        records,
        payload,
        stable_id_scratch: stable_ids,
        canonical_identity_scratch: canonical_identity,
        identity_payload_scratch: identity_payload,
    })
}

fn canonicalize_mir<const TRACK_ALLOCATIONS: bool>(
    stage: &StageContract,
    plan: &IdentityStagePlan,
    mir: &MirStage,
    buffers: &mut IdentityStageBufferPool<TRACK_ALLOCATIONS>,
) -> Result<LirStage, StageGenerationError> {
    let record_count = to_usize(plan.stages.canonical_lir.record_count, "LIR records")?;
    let mut scratch = take_reusable(
        &mut buffers.lir_sort_scratch,
        record_count,
        "canonical LIR sort scratch",
        &mut buffers.allocations,
        ControlledBufferSlot::LirSortScratch,
    )?;
    let mut owner_ordinals = take_reusable(
        &mut buffers.lir_owner_ordinal_scratch,
        record_count,
        "canonical LIR owner ordinal scratch",
        &mut buffers.allocations,
        ControlledBufferSlot::LirOwnerOrdinalScratch,
    )?;
    owner_ordinals.resize(record_count, stage.absent_ordinal);

    for entity_kind in 1..=plan.binding_count {
        scratch.clear();
        scratch.extend(
            mir.records
                .iter()
                .enumerate()
                .filter(|(_, record)| {
                    record.record_kind == 1 && record.entity_kind == entity_kind as u16
                })
                .map(|(index, _)| index),
        );
        scratch.sort_unstable_by_key(|index| mir.records[*index].stable_id);
        for (ordinal, record_index) in scratch.iter().copied().enumerate() {
            owner_ordinals[record_index] = u32::try_from(ordinal)
                .map_err(|_| StageGenerationError::Overflow("owner ordinal"))?;
        }
    }
    for (record_index, record) in mir.records.iter().enumerate() {
        if record.record_kind == 1 {
            continue;
        }
        owner_ordinals[record_index] = find_owner_ordinal(record, mir, &owner_ordinals)?;
    }

    scratch.clear();
    scratch.extend(0..mir.records.len());
    scratch.sort_unstable_by(|left, right| {
        canonical_record_order(
            &mir.records[*left],
            owner_ordinals[*left],
            &mir.records[*right],
            owner_ordinals[*right],
            &mir.payload,
        )
    });
    let scratch_capacity_bytes = u64::try_from(scratch.capacity())
        .map_err(|_| StageGenerationError::Overflow("scratch capacity"))?
        .checked_mul(8)
        .ok_or(StageGenerationError::Overflow("scratch capacity"))?;
    if scratch_capacity_bytes > plan.stages.scratch.logical_bytes {
        return Err(StageGenerationError::MaterializedMismatch(
            "scratch capacity",
        ));
    }

    let mut records = take_reusable(
        &mut buffers.canonical_lir_records,
        record_count,
        "canonical LIR records",
        &mut buffers.allocations,
        ControlledBufferSlot::CanonicalLirRecords,
    )?;
    let mut payload = take_reusable(
        &mut buffers.canonical_lir_payload,
        to_usize(
            plan.stages.canonical_lir.payload_logical_bytes,
            "LIR payload",
        )?,
        "canonical LIR payload",
        &mut buffers.allocations,
        ControlledBufferSlot::CanonicalLirPayload,
    )?;
    for source_index in scratch.iter().copied() {
        let source = mir.records[source_index];
        let source_payload =
            payload_slice(&mir.payload, source.payload_offset, source.payload_length)?;
        let payload_offset = u64_from_usize(payload.len(), "LIR payload offset")?;
        payload.extend_from_slice(source_payload);
        records.push(MirLirStageRecord {
            owner_ordinal: owner_ordinals[source_index],
            payload_offset,
            ..source
        });
    }
    scratch.clear();
    owner_ordinals.clear();
    Ok(LirStage {
        records,
        payload,
        scratch_capacity_bytes,
        sort_scratch: scratch,
        owner_ordinal_scratch: owner_ordinals,
    })
}

fn construct_output<const TRACK_ALLOCATIONS: bool>(
    identity: &IdentityContract,
    plan: &IdentityStagePlan,
    lir: &LirStage,
    buffers: &mut IdentityStageBufferPool<TRACK_ALLOCATIONS>,
) -> Result<Vec<u8>, StageGenerationError> {
    let mut output = take_reusable(
        &mut buffers.output_construction,
        to_usize(
            plan.stages.output_construction.logical_bytes,
            "output construction",
        )?,
        "output construction",
        &mut buffers.allocations,
        ControlledBufferSlot::OutputConstruction,
    )?;
    output.extend_from_slice(identity.semantic_record_domain().as_bytes());
    output.push(0);
    output.extend_from_slice(&identity.semantic_record_stream_version().to_le_bytes());
    output.extend_from_slice(
        &u64::try_from(lir.records.len())
            .map_err(|_| StageGenerationError::Overflow("output record count"))?
            .to_le_bytes(),
    );
    for record in &lir.records {
        output.extend_from_slice(&record.record_kind.to_le_bytes());
        output.extend_from_slice(&record.entity_kind.to_le_bytes());
        output.extend_from_slice(&record.stable_id);
        output.extend_from_slice(&record.owner_ordinal.to_le_bytes());
        output.extend_from_slice(&record.local_index.to_le_bytes());
        output.extend_from_slice(&record.payload_length.to_le_bytes());
        output.extend_from_slice(payload_slice(
            &lir.payload,
            record.payload_offset,
            record.payload_length,
        )?);
    }
    Ok(output)
}

fn verify_materialized_shapes(
    summary: &crate::stage::IdentityStageSummary,
    source: &SourceStage,
    typed: &TypedStage,
    hir: &HirStage,
    mir: &MirStage,
    lir: &LirStage,
    output: &[u8],
) -> Result<(), StageGenerationError> {
    let actual = [
        (
            "sourceInput",
            source.records.len(),
            source.payload.len(),
            summary.stages.source_input,
        ),
        (
            "typedAst",
            typed.records.len(),
            typed.payload.len(),
            summary.stages.typed_ast,
        ),
        (
            "hir",
            hir.records.len(),
            hir.payload.len(),
            summary.stages.hir,
        ),
        (
            "mir",
            mir.records.len(),
            mir.payload.len(),
            summary.stages.mir,
        ),
        (
            "canonicalLir",
            lir.records.len(),
            lir.payload.len(),
            summary.stages.canonical_lir,
        ),
    ];
    for (name, records, payload, expected) in actual {
        if as_u64(records, "stage records")? != expected.record_count
            || as_u64(payload, "stage payload")? != expected.payload_logical_bytes
        {
            return Err(StageGenerationError::MaterializedMismatch(name));
        }
    }
    if as_u64(output.len(), "output construction")?
        != summary.stages.output_construction.logical_bytes
    {
        return Err(StageGenerationError::MaterializedMismatch(
            "output construction",
        ));
    }
    Ok(())
}

fn declaration_source_value(
    identity: &IdentityContract,
    stage: &StageContract,
    plan: &IdentityStagePlan,
    strings: StringLayout,
    module_ordinal: u32,
    canonical_ordinal: u32,
) -> Result<(u16, u64, u64), StageGenerationError> {
    if plan.graph_profile == GraphProfileId::SharedFaninDag && module_ordinal == 1 {
        if canonical_ordinal != 0 {
            return Err(StageGenerationError::MaterializedMismatch(
                "shared declaration ordinal",
            ));
        }
        return Ok((
            SHARED_CONSTANT_ENTITY_KIND,
            u64_from_usize(
                strings.string_end
                    - stage.shared_constant_value.len()
                    - stage.shared_constant_name.len(),
                "shared name offset",
            )?,
            u64_from_usize(stage.shared_constant_name.len(), "shared name length")?,
        ));
    }
    let unit_index = module_to_unit(plan, module_ordinal)
        .ok_or(StageGenerationError::InvalidModuleOrdinal(module_ordinal))?;
    let binding = identity
        .bindings
        .get(
            usize::try_from(canonical_ordinal)
                .map_err(|_| StageGenerationError::Overflow("declaration ordinal"))?,
        )
        .ok_or(StageGenerationError::MaterializedMismatch(
            "declaration binding",
        ))?;
    let declaration_ordinal = u64::from(unit_index)
        .checked_mul(u64::from(plan.binding_count))
        .and_then(|base| base.checked_add(u64::from(canonical_ordinal)))
        .ok_or(StageGenerationError::Overflow("declaration ordinal"))?;
    let offset = strings
        .namespace_base
        .checked_add(
            usize::try_from(declaration_ordinal)
                .map_err(|_| StageGenerationError::Overflow("namespace ordinal"))?
                .checked_mul(32)
                .ok_or(StageGenerationError::Overflow("namespace offset"))?,
        )
        .ok_or(StageGenerationError::Overflow("namespace offset"))?;
    Ok((
        binding.entity_kind_code,
        u64_from_usize(offset, "namespace offset")?,
        32,
    ))
}

#[derive(Clone, Copy, Debug)]
struct SourceReferenceDescriptor {
    source_entity_kind: u16,
    global_string_ordinal: u64,
}

fn source_reference_descriptor(
    identity: &IdentityContract,
    plan: &IdentityStagePlan,
    module_ordinal: u32,
    canonical_ordinal: u32,
) -> Result<SourceReferenceDescriptor, StageGenerationError> {
    if let Some(unit_index) = module_to_unit(plan, module_ordinal) {
        if canonical_ordinal < plan.stable_fields_per_unit {
            let (binding, _) = stable_field_by_ordinal(identity, canonical_ordinal)?;
            return Ok(SourceReferenceDescriptor {
                source_entity_kind: binding.entity_kind_code,
                global_string_ordinal: u64::from(unit_index)
                    .checked_mul(u64::from(plan.stable_fields_per_unit))
                    .and_then(|base| base.checked_add(u64::from(canonical_ordinal)))
                    .ok_or(StageGenerationError::Overflow("stable reference ordinal"))?,
            });
        }
        let relation_ordinal = canonical_ordinal - plan.stable_fields_per_unit;
        if relation_ordinal < plan.relation_count_per_unit {
            let relation = identity
                .owner_relations
                .get(
                    usize::try_from(relation_ordinal)
                        .map_err(|_| StageGenerationError::Overflow("relation ordinal"))?,
                )
                .ok_or(StageGenerationError::MaterializedMismatch(
                    "relation reference",
                ))?;
            return Ok(SourceReferenceDescriptor {
                source_entity_kind: relation.child_kind,
                global_string_ordinal: u64::from(plan.n)
                    .checked_mul(u64::from(plan.stable_fields_per_unit))
                    .and_then(|base| {
                        base.checked_add(
                            u64::from(unit_index)
                                .checked_mul(u64::from(plan.relation_count_per_unit))?,
                        )
                    })
                    .and_then(|base| base.checked_add(u64::from(relation_ordinal)))
                    .ok_or(StageGenerationError::Overflow("relation reference ordinal"))?,
            });
        }
        let cross_local = relation_ordinal - plan.relation_count_per_unit;
        let cross_global = cross_reference_ordinal_for_module(plan, module_ordinal, cross_local)?;
        return Ok(SourceReferenceDescriptor {
            source_entity_kind: ENTITY_KIND_ABSENT,
            global_string_ordinal: base_cross_reference_ordinal(plan)?
                .checked_add(u64::from(cross_global))
                .ok_or(StageGenerationError::Overflow("cross reference ordinal"))?,
        });
    }
    let cross_global = cross_reference_ordinal_for_module(plan, module_ordinal, canonical_ordinal)?;
    Ok(SourceReferenceDescriptor {
        source_entity_kind: ENTITY_KIND_ABSENT,
        global_string_ordinal: base_cross_reference_ordinal(plan)?
            .checked_add(u64::from(cross_global))
            .ok_or(StageGenerationError::Overflow("cross reference ordinal"))?,
    })
}

fn module_declaration_count(
    identity: &IdentityContract,
    plan: &IdentityStagePlan,
    module_ordinal: u32,
) -> Result<u32, StageGenerationError> {
    if module_to_unit(plan, module_ordinal).is_some() {
        u32::try_from(identity.bindings.len())
            .map_err(|_| StageGenerationError::Overflow("declaration count"))
    } else if plan.graph_profile == GraphProfileId::SharedFaninDag && module_ordinal == 1 {
        Ok(1)
    } else {
        Ok(0)
    }
}

fn module_reference_count(
    plan: &IdentityStagePlan,
    module_ordinal: u32,
) -> Result<u32, StageGenerationError> {
    let local = if module_to_unit(plan, module_ordinal).is_some() {
        let cross_reference_count = cross_reference_count_for_module(plan, module_ordinal)?;
        plan.stable_fields_per_unit
            .checked_add(plan.relation_count_per_unit)
            .and_then(|base| base.checked_add(cross_reference_count))
            .ok_or(StageGenerationError::Overflow("module reference count"))?
    } else {
        cross_reference_count_for_module(plan, module_ordinal)?
    };
    Ok(local)
}

fn module_relation_count(
    plan: &IdentityStagePlan,
    module_ordinal: u32,
) -> Result<u32, StageGenerationError> {
    Ok(if module_to_unit(plan, module_ordinal).is_some() {
        plan.relation_count_per_unit
    } else {
        0
    })
}

fn cross_reference_count_for_module(
    plan: &IdentityStagePlan,
    module_ordinal: u32,
) -> Result<u32, StageGenerationError> {
    Ok(match plan.graph_profile {
        GraphProfileId::WideStar if module_ordinal == 0 => plan.n,
        GraphProfileId::DeepChain => {
            if let Some(unit) = module_to_unit(plan, module_ordinal) {
                u32::from(unit + 1 < plan.n)
            } else {
                0
            }
        }
        GraphProfileId::SharedFaninDag => u32::from(module_to_unit(plan, module_ordinal).is_some()),
        _ => 0,
    })
}

fn cross_reference_ordinal_for_module(
    plan: &IdentityStagePlan,
    module_ordinal: u32,
    local: u32,
) -> Result<u32, StageGenerationError> {
    let ordinal = match plan.graph_profile {
        GraphProfileId::WideStar if module_ordinal == 0 && local < plan.n => local,
        GraphProfileId::DeepChain => {
            let unit = module_to_unit(plan, module_ordinal)
                .ok_or(StageGenerationError::InvalidModuleOrdinal(module_ordinal))?;
            if local != 0 || unit + 1 >= plan.n {
                return Err(StageGenerationError::MaterializedMismatch(
                    "deep-chain cross reference",
                ));
            }
            unit
        }
        GraphProfileId::SharedFaninDag => {
            let unit = module_to_unit(plan, module_ordinal)
                .ok_or(StageGenerationError::InvalidModuleOrdinal(module_ordinal))?;
            if local != 0 {
                return Err(StageGenerationError::MaterializedMismatch(
                    "shared-fanin cross reference",
                ));
            }
            unit
        }
        _ => {
            return Err(StageGenerationError::MaterializedMismatch(
                "cross reference module",
            ));
        }
    };
    Ok(ordinal)
}

fn cross_reference_target(
    plan: &IdentityStagePlan,
    ordinal: u32,
) -> Result<(u16, u32), StageGenerationError> {
    match plan.graph_profile {
        GraphProfileId::WideStar => Ok((1, unit_module_ordinal(plan, ordinal)?)),
        GraphProfileId::DeepChain => {
            let target = ordinal
                .checked_add(1)
                .ok_or(StageGenerationError::Overflow("deep-chain target"))?;
            Ok((1, unit_module_ordinal(plan, target)?))
        }
        GraphProfileId::SharedFaninDag => Ok((SHARED_CONSTANT_ENTITY_KIND, 1)),
    }
}

fn base_cross_reference_ordinal(plan: &IdentityStagePlan) -> Result<u64, StageGenerationError> {
    u64::from(plan.n)
        .checked_mul(u64::from(
            plan.stable_fields_per_unit
                .checked_add(plan.relation_count_per_unit)
                .ok_or(StageGenerationError::Overflow("reference count"))?,
        ))
        .ok_or(StageGenerationError::Overflow("cross reference base"))
}

fn fill_import_targets(
    output: &mut Vec<u64>,
    plan: &IdentityStagePlan,
    source_module: u32,
) -> Result<(), StageGenerationError> {
    output.clear();
    match plan.graph_profile {
        GraphProfileId::WideStar => {
            if source_module == 0 {
                for unit in 0..plan.n {
                    output.push(u64::from(unit_module_ordinal(plan, unit)?));
                }
            }
        }
        GraphProfileId::DeepChain => {
            if source_module == 0 {
                output.push(u64::from(unit_module_ordinal(plan, 0)?));
            } else if let Some(unit) = module_to_unit(plan, source_module)
                && unit + 1 < plan.n
            {
                output.push(u64::from(unit_module_ordinal(plan, unit + 1)?));
            }
        }
        GraphProfileId::SharedFaninDag => {
            if source_module == 0 {
                for group in 0..plan.group_count {
                    output.push(u64::from(
                        group
                            .checked_add(2)
                            .ok_or(StageGenerationError::Overflow("group module ordinal"))?,
                    ));
                }
            } else if source_module >= 2 && source_module < plan.unit_module_base {
                let group = source_module - 2;
                let start = group
                    .checked_mul(64)
                    .ok_or(StageGenerationError::Overflow("group unit start"))?;
                let end = start.saturating_add(64).min(plan.n);
                for unit in start..end {
                    output.push(u64::from(unit_module_ordinal(plan, unit)?));
                }
            } else if module_to_unit(plan, source_module).is_some() {
                output.push(1);
            }
        }
    }
    Ok(())
}

fn module_seed_ordinal(
    plan: &IdentityStagePlan,
    module_ordinal: u32,
) -> Result<u64, StageGenerationError> {
    if module_ordinal == 0 {
        return Ok(0);
    }
    if plan.graph_profile == GraphProfileId::SharedFaninDag {
        if module_ordinal == 1 {
            return Ok(1);
        }
        if module_ordinal < plan.unit_module_base {
            return Ok((1_u64 << 40) | u64::from(module_ordinal - 2));
        }
    }
    let unit = module_to_unit(plan, module_ordinal)
        .ok_or(StageGenerationError::InvalidModuleOrdinal(module_ordinal))?;
    Ok((2_u64 << 40) | u64::from(unit))
}

fn unit_module_ordinal(
    plan: &IdentityStagePlan,
    unit_index: u32,
) -> Result<u32, StageGenerationError> {
    if unit_index >= plan.n {
        return Err(StageGenerationError::InvalidModuleOrdinal(unit_index));
    }
    plan.unit_module_base
        .checked_add(unit_index)
        .ok_or(StageGenerationError::Overflow("unit module ordinal"))
}

fn module_to_unit(plan: &IdentityStagePlan, module_ordinal: u32) -> Option<u32> {
    module_ordinal
        .checked_sub(plan.unit_module_base)
        .filter(|unit| *unit < plan.n)
}

fn symbol_ordinal(
    plan: &IdentityStagePlan,
    identity: &IdentityContract,
    module_ordinal: u32,
    entity_kind: u16,
) -> Result<u32, StageGenerationError> {
    if plan.graph_profile == GraphProfileId::SharedFaninDag
        && module_ordinal == 1
        && entity_kind == SHARED_CONSTANT_ENTITY_KIND
    {
        return Ok(0);
    }
    let unit = module_to_unit(plan, module_ordinal).ok_or(StageGenerationError::InvalidSymbol {
        module_ordinal,
        entity_kind,
    })?;
    let binding = u32::try_from(binding_index(identity, entity_kind)?)
        .map_err(|_| StageGenerationError::Overflow("binding index"))?;
    u32::from(plan.graph_profile == GraphProfileId::SharedFaninDag)
        .checked_add(
            unit.checked_mul(plan.binding_count)
                .ok_or(StageGenerationError::Overflow("symbol ordinal"))?,
        )
        .and_then(|base| base.checked_add(binding))
        .ok_or(StageGenerationError::Overflow("symbol ordinal"))
}

fn binding_index(
    identity: &IdentityContract,
    entity_kind: u16,
) -> Result<usize, StageGenerationError> {
    identity
        .bindings
        .binary_search_by_key(&entity_kind, |binding| binding.entity_kind_code)
        .map_err(|_| StageGenerationError::MissingEntityKind(entity_kind))
}

fn identity_field(
    identity: &IdentityContract,
    entity_kind: u16,
    tag: u32,
) -> Result<&crate::identity::IdentityFieldBinding, StageGenerationError> {
    let tag =
        u16::try_from(tag).map_err(|_| StageGenerationError::MaterializedMismatch("field tag"))?;
    identity.bindings[binding_index(identity, entity_kind)?]
        .fields
        .iter()
        .find(|field| field.tag == tag)
        .ok_or(StageGenerationError::MaterializedMismatch("identity field"))
}

fn stable_field_by_ordinal(
    identity: &IdentityContract,
    ordinal: u32,
) -> Result<(&IdentityBinding, u16), StageGenerationError> {
    let mut current = 0_u32;
    for binding in &identity.bindings {
        for field in &binding.fields {
            if matches!(field.value, IdentityFieldValue::StableId { .. }) {
                if current == ordinal {
                    return Ok((binding, field.tag));
                }
                current = current
                    .checked_add(1)
                    .ok_or(StageGenerationError::Overflow("stable field ordinal"))?;
            }
        }
    }
    Err(StageGenerationError::MaterializedMismatch(
        "stable field ordinal",
    ))
}

fn profiled_fields_before(
    identity: &IdentityContract,
    binding_index: usize,
) -> Result<u32, StageGenerationError> {
    identity
        .bindings
        .iter()
        .take(binding_index)
        .try_fold(0_u32, |total, binding| {
            let count = u32::try_from(
                binding
                    .fields
                    .iter()
                    .filter(|field| matches!(field.value, IdentityFieldValue::ProfiledKey { .. }))
                    .count(),
            )
            .map_err(|_| StageGenerationError::Overflow("profiled field count"))?;
            total
                .checked_add(count)
                .ok_or(StageGenerationError::Overflow("profiled field count"))
        })
}

fn find_hir_identity_field<'a>(
    module_records: &'a [HirStageRecord],
    payload: &[u8],
    stage: &StageContract,
    entity_kind: u16,
    field_tag: u16,
) -> Result<&'a HirStageRecord, StageGenerationError> {
    module_records
        .iter()
        .find(|record| {
            record.record_kind == stage.record_kind_identity_field
                && record.entity_kind == entity_kind
                && read_single_u32(payload, record).ok() == Some(u32::from(field_tag))
        })
        .ok_or(StageGenerationError::MaterializedMismatch(
            "HIR identity field",
        ))
}

fn hir_module_records(records: &[HirStageRecord], module_ordinal: u32) -> &[HirStageRecord] {
    let start = records.partition_point(|record| record.module_ordinal < module_ordinal);
    let end = records.partition_point(|record| record.module_ordinal <= module_ordinal);
    &records[start..end]
}

fn validate_relation_operands(
    record: &HirStageRecord,
    payload: &[u8],
    child: u32,
    parent: u32,
) -> Result<(), StageGenerationError> {
    let bytes = payload_slice(payload, record.payload_offset, record.payload_length)?;
    if bytes.len() != 8
        || u32::from_le_bytes(bytes[0..4].try_into().expect("four bytes")) != child
        || u32::from_le_bytes(bytes[4..8].try_into().expect("four bytes")) != parent
    {
        return Err(StageGenerationError::MaterializedMismatch(
            "HIR relation operands",
        ));
    }
    Ok(())
}

fn read_single_u32(payload: &[u8], record: &HirStageRecord) -> Result<u32, StageGenerationError> {
    let bytes = payload_slice(payload, record.payload_offset, record.payload_length)?;
    if bytes.len() != 4 {
        return Err(StageGenerationError::MaterializedMismatch(
            "HIR u32 operand",
        ));
    }
    Ok(u32::from_le_bytes(
        bytes.try_into().expect("validated four bytes"),
    ))
}

fn find_owner_ordinal(
    record: &MirLirStageRecord,
    mir: &MirStage,
    owner_ordinals: &[u32],
) -> Result<u32, StageGenerationError> {
    mir.records
        .iter()
        .enumerate()
        .find(|(_, candidate)| {
            candidate.record_kind == 1
                && candidate.entity_kind == record.entity_kind
                && candidate.stable_id == record.stable_id
        })
        .map(|(index, _)| owner_ordinals[index])
        .filter(|ordinal| *ordinal != u32::MAX)
        .ok_or(StageGenerationError::MaterializedMismatch(
            "record owner ordinal",
        ))
}

fn typed_record_order(
    left: &TypedAstStageRecord,
    right: &TypedAstStageRecord,
    payload: &[u8],
    absent: u32,
) -> Ordering {
    typed_record_prefix(left, absent)
        .cmp(&typed_record_prefix(right, absent))
        .then_with(|| {
            payload_slice(payload, left.payload_offset, left.payload_length)
                .expect("typed payload range verified during construction")
                .cmp(
                    payload_slice(payload, right.payload_offset, right.payload_length)
                        .expect("typed payload range verified during construction"),
                )
        })
}

fn typed_record_prefix(record: &TypedAstStageRecord, absent: u32) -> (u32, u16, u8, u32, u16, u32) {
    let (span_presence, span) = if record.source_span_ordinal == absent {
        (0, 0)
    } else {
        (1, record.source_span_ordinal)
    };
    (
        record.module_ordinal,
        record.record_kind,
        span_presence,
        span,
        record.entity_kind,
        record.owner_local_index,
    )
}

fn hir_record_order(
    left: &HirStageRecord,
    right: &HirStageRecord,
    payload: &[u8],
    absent: u32,
) -> Ordering {
    let left_target = (
        u8::from(left.resolved_target_ordinal != absent),
        left.resolved_target_ordinal,
    );
    let right_target = (
        u8::from(right.resolved_target_ordinal != absent),
        right.resolved_target_ordinal,
    );
    (
        left.module_ordinal,
        left.record_kind,
        left.entity_kind,
        left.symbol_ordinal,
        left_target,
    )
        .cmp(&(
            right.module_ordinal,
            right.record_kind,
            right.entity_kind,
            right.symbol_ordinal,
            right_target,
        ))
        .then_with(|| {
            payload_slice(payload, left.payload_offset, left.payload_length)
                .expect("HIR payload range verified during construction")
                .cmp(
                    payload_slice(payload, right.payload_offset, right.payload_length)
                        .expect("HIR payload range verified during construction"),
                )
        })
}

fn canonical_record_order(
    left: &MirLirStageRecord,
    left_owner: u32,
    right: &MirLirStageRecord,
    right_owner: u32,
    payload: &[u8],
) -> Ordering {
    (
        left.record_kind,
        left.entity_kind,
        left.stable_id,
        left_owner,
        left.local_index,
    )
        .cmp(&(
            right.record_kind,
            right.entity_kind,
            right.stable_id,
            right_owner,
            right.local_index,
        ))
        .then_with(|| {
            payload_slice(payload, left.payload_offset, left.payload_length)
                .expect("MIR payload range verified during construction")
                .cmp(
                    payload_slice(payload, right.payload_offset, right.payload_length)
                        .expect("MIR payload range verified during construction"),
                )
        })
}

fn set_hir_operands(
    record: &mut HirStageRecord,
    payload: &mut Vec<u8>,
    string_bytes: usize,
    values: &[u32],
) -> Result<(), StageGenerationError> {
    let offset = payload.len();
    for value in values {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    record.payload_offset = u64_from_usize(offset, "HIR operand offset")?;
    record.payload_length = u64_from_usize(values.len() * 4, "HIR operand length")?;
    if offset < string_bytes {
        return Err(StageGenerationError::MaterializedMismatch(
            "HIR operand partition",
        ));
    }
    Ok(())
}

fn encode_source_spans(payload: &mut Vec<u8>, spans: &[SourceSpanRecord]) {
    for span in spans {
        payload.extend_from_slice(&span.source_document_ordinal.to_le_bytes());
        payload.extend_from_slice(&span.start_line.to_le_bytes());
        payload.extend_from_slice(&span.start_column.to_le_bytes());
        payload.extend_from_slice(&span.end_line.to_le_bytes());
        payload.extend_from_slice(&span.end_column.to_le_bytes());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ParsedReference {
    entity_kind: u16,
    module_ordinal: u32,
    local_ordinal: u32,
}

fn parse_reference_spelling(bytes: &[u8]) -> Result<ParsedReference, StageGenerationError> {
    if bytes.len() != 30 || &bytes[..10] != b"reference/" || bytes[12] != b'/' || bytes[21] != b'/'
    {
        return Err(StageGenerationError::InvalidSourceReference);
    }
    Ok(ParsedReference {
        entity_kind: parse_hex_u16(&bytes[10..12])?,
        module_ordinal: parse_hex_u32(&bytes[13..21])?,
        local_ordinal: parse_hex_u32(&bytes[22..30])?,
    })
}

fn parse_module_name(plan: &IdentityStagePlan, bytes: &[u8]) -> Result<u32, StageGenerationError> {
    if bytes == b"root" {
        return Ok(0);
    }
    if plan.graph_profile == GraphProfileId::SharedFaninDag && bytes == b"shared/common" {
        return Ok(1);
    }
    if let Some(hex) = bytes.strip_prefix(b"group/") {
        let group = parse_hex_u32(hex)?;
        if plan.graph_profile == GraphProfileId::SharedFaninDag && group < plan.group_count {
            return group
                .checked_add(2)
                .ok_or(StageGenerationError::Overflow("group module ordinal"));
        }
    }
    if let Some(hex) = bytes.strip_prefix(b"unit/") {
        return unit_module_ordinal(plan, parse_hex_u32(hex)?);
    }
    Err(StageGenerationError::InvalidSourceReference)
}

fn append_module_name(
    output: &mut Vec<u8>,
    plan: &IdentityStagePlan,
    module_ordinal: u32,
) -> Result<(), StageGenerationError> {
    output.extend_from_slice(module_name_buffer(plan, module_ordinal)?.as_slice());
    Ok(())
}

#[derive(Clone, Copy)]
struct ModuleNameBuffer {
    bytes: [u8; 14],
    len: usize,
}

impl ModuleNameBuffer {
    fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

fn module_name_buffer(
    plan: &IdentityStagePlan,
    module_ordinal: u32,
) -> Result<ModuleNameBuffer, StageGenerationError> {
    let mut buffer = ModuleNameBuffer {
        bytes: [0; 14],
        len: 0,
    };
    if module_ordinal == 0 {
        buffer.bytes[..4].copy_from_slice(b"root");
        buffer.len = 4;
        return Ok(buffer);
    }
    if plan.graph_profile == GraphProfileId::SharedFaninDag {
        if module_ordinal == 1 {
            buffer.bytes[..13].copy_from_slice(b"shared/common");
            buffer.len = 13;
            return Ok(buffer);
        }
        if module_ordinal < plan.unit_module_base {
            buffer.bytes[..6].copy_from_slice(b"group/");
            write_hex_u32(&mut buffer.bytes[6..14], module_ordinal - 2);
            buffer.len = 14;
            return Ok(buffer);
        }
    }
    let unit = module_to_unit(plan, module_ordinal)
        .ok_or(StageGenerationError::InvalidModuleOrdinal(module_ordinal))?;
    buffer.bytes[..5].copy_from_slice(b"unit/");
    write_hex_u32(&mut buffer.bytes[5..13], unit);
    buffer.len = 13;
    Ok(buffer)
}

fn derive_namespace_ascii(
    generator: &GeneratorContract,
    graph_profile: GraphProfileId,
    module_name: &[u8],
    preimage: &mut Vec<u8>,
) -> [u8; 32] {
    preimage.clear();
    preimage.extend_from_slice(generator.namespace_domain.as_bytes());
    preimage.push(0);
    preimage.extend_from_slice(&generator.generator_version.to_le_bytes());
    preimage.extend_from_slice(&generator.base_seed.to_le_bytes());
    append_length_prefixed(preimage, b"LF-COMP-ID-v1");
    append_length_prefixed(preimage, graph_profile.as_str().as_bytes());
    append_length_prefixed(preimage, module_name);
    let digest = blake3::hash(preimage);
    let selected = &digest.as_bytes()[generator.namespace_digest_offset
        ..generator.namespace_digest_offset + generator.namespace_digest_length];
    let mut ascii = [0_u8; 32];
    for (index, byte) in selected.iter().copied().enumerate() {
        ascii[index * 2] = hex_digit(byte >> 4);
        ascii[index * 2 + 1] = hex_digit(byte & 0x0f);
    }
    ascii
}

fn append_length_prefixed(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(
        &u32::try_from(value.len())
            .expect("research identifier must fit u32")
            .to_le_bytes(),
    );
    output.extend_from_slice(value);
}

fn append_reference_spelling(
    output: &mut Vec<u8>,
    kind: u16,
    module_ordinal: u32,
    local_ordinal: u32,
) {
    output.extend_from_slice(b"reference/");
    append_hex_u16(output, kind);
    output.push(b'/');
    append_hex_u32(output, module_ordinal);
    output.push(b'/');
    append_hex_u32(output, local_ordinal);
}

fn append_hex_u16(output: &mut Vec<u8>, value: u16) {
    output.push(hex_digit(((value >> 4) & 0x0f) as u8));
    output.push(hex_digit((value & 0x0f) as u8));
}

fn append_hex_u32(output: &mut Vec<u8>, value: u32) {
    let start = output.len();
    output.resize(start + 8, 0);
    write_hex_u32(&mut output[start..start + 8], value);
}

fn write_hex_u32(output: &mut [u8], value: u32) {
    for (index, byte) in output.iter_mut().enumerate() {
        let shift = (7 - index) * 4;
        *byte = hex_digit(((value >> shift) & 0x0f) as u8);
    }
}

fn hex_digit(value: u8) -> u8 {
    match value {
        0..=9 => b'0' + value,
        _ => b'a' + (value - 10),
    }
}

fn parse_hex_u16(bytes: &[u8]) -> Result<u16, StageGenerationError> {
    if bytes.len() != 2 {
        return Err(StageGenerationError::InvalidSourceReference);
    }
    Ok((u16::from(parse_hex_digit(bytes[0])?) << 4) | u16::from(parse_hex_digit(bytes[1])?))
}

fn parse_hex_u32(bytes: &[u8]) -> Result<u32, StageGenerationError> {
    if bytes.len() != 8 {
        return Err(StageGenerationError::InvalidSourceReference);
    }
    bytes.iter().try_fold(0_u32, |value, byte| {
        value
            .checked_shl(4)
            .and_then(|value| value.checked_add(u32::from(parse_hex_digit(*byte).ok()?)))
            .ok_or(StageGenerationError::InvalidSourceReference)
    })
}

fn parse_hex_digit(value: u8) -> Result<u8, StageGenerationError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(StageGenerationError::InvalidSourceReference),
    }
}

fn fill_ordinals(output: &mut Vec<u64>, count: u32) {
    output.clear();
    output.extend((0..count).map(u64::from));
}

fn payload_slice(payload: &[u8], offset: u64, length: u64) -> Result<&[u8], StageGenerationError> {
    let start =
        usize::try_from(offset).map_err(|_| StageGenerationError::Overflow("payload offset"))?;
    let length =
        usize::try_from(length).map_err(|_| StageGenerationError::Overflow("payload length"))?;
    let end = start
        .checked_add(length)
        .ok_or(StageGenerationError::Overflow("payload range"))?;
    payload
        .get(start..end)
        .ok_or(StageGenerationError::MaterializedMismatch("payload range"))
}

fn string_at(
    payload: &[u8],
    base: usize,
    ordinal: u64,
    width: usize,
) -> Result<&[u8], StageGenerationError> {
    let ordinal =
        usize::try_from(ordinal).map_err(|_| StageGenerationError::Overflow("string ordinal"))?;
    let start = base
        .checked_add(
            ordinal
                .checked_mul(width)
                .ok_or(StageGenerationError::Overflow("string offset"))?,
        )
        .ok_or(StageGenerationError::Overflow("string offset"))?;
    payload
        .get(start..start + width)
        .ok_or(StageGenerationError::MaterializedMismatch("string range"))
}

fn u32_from_u64(value: u64, field: &'static str) -> Result<u32, StageGenerationError> {
    u32::try_from(value).map_err(|_| StageGenerationError::Overflow(field))
}

fn u64_from_usize(value: usize, field: &'static str) -> Result<u64, StageGenerationError> {
    u64::try_from(value).map_err(|_| StageGenerationError::Overflow(field))
}

fn take_reusable<T, const TRACK_ALLOCATIONS: bool>(
    slot: &mut Vec<T>,
    required_capacity: usize,
    field: &'static str,
    allocations: &mut ControlledAllocationTracker<TRACK_ALLOCATIONS>,
    controlled_slot: ControlledBufferSlot,
) -> Result<Vec<T>, StageGenerationError> {
    let mut values = std::mem::take(slot);
    values.clear();
    if values.capacity() < required_capacity {
        if TRACK_ALLOCATIONS {
            let requested_bytes = u64::try_from(required_capacity)
                .ok()
                .and_then(|capacity| {
                    capacity.checked_mul(
                        u64::try_from(std::mem::size_of::<T>()).expect("type size must fit u64"),
                    )
                })
                .ok_or(StageGenerationError::Overflow(
                    "controlled allocation request bytes",
                ))?;
            let preoccupied_live_bytes = allocations.preoccupy(field, requested_bytes)?;
            if let Err(source) = values.try_reserve_exact(required_capacity) {
                allocations.cancel_preoccupation(requested_bytes);
                return Err(StageGenerationError::AllocationFailed { field, source });
            }
            allocations.commit_replacement(
                controlled_slot,
                requested_bytes,
                preoccupied_live_bytes,
            )?;
        } else if let Err(source) = values.try_reserve_exact(required_capacity) {
            return Err(StageGenerationError::AllocationFailed { field, source });
        }
    }
    Ok(values)
}

fn store_cleared<T>(slot: &mut Vec<T>, mut values: Vec<T>) {
    values.clear();
    *slot = values;
}

fn capacity_bytes<T>(values: &Vec<T>) -> Result<u64, StageGenerationError> {
    u64::try_from(values.capacity())
        .ok()
        .and_then(|capacity| {
            capacity.checked_mul(
                u64::try_from(std::mem::size_of::<T>()).expect("type size must fit u64"),
            )
        })
        .ok_or(StageGenerationError::Overflow("retained capacity bytes"))
}

fn capacity_sum(values: &[u64]) -> Result<u64, StageGenerationError> {
    values.iter().try_fold(0_u64, |total, value| {
        total
            .checked_add(*value)
            .ok_or(StageGenerationError::Overflow("retained capacity bytes"))
    })
}

fn encode_lower_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(hex_digit(byte >> 4)));
        encoded.push(char::from(hex_digit(byte & 0x0f)));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_repository_contract;

    #[test]
    fn n2_pipeline_materializes_real_distinct_mir_and_lir() {
        let trusted = load_repository_contract().expect("frozen contract");
        let generator = trusted.generator_contract().expect("generator");
        let identity = trusted.identity_contract().expect("identity");
        let stage = trusted.stage_contract().expect("stage");
        for profile in GraphProfileId::ALL {
            let output = build_identity_stage_case(&generator, &identity, &stage, profile, 2)
                .expect("N=2 stage output");
            assert_eq!(output.summary.counts.identity_declaration_count, 44);
            assert_eq!(output.summary.counts.semantic_output_record, 64);
            assert_eq!(output.summary.counts.semantic_payload_byte_count, 3_852);
            assert!(
                output
                    .mir_records
                    .iter()
                    .all(|record| record.owner_ordinal == u32::MAX)
            );
            assert!(
                output
                    .canonical_lir_records
                    .iter()
                    .all(|record| record.owner_ordinal != u32::MAX)
            );
            assert_ne!(output.mir_records, output.canonical_lir_records);
        }
    }

    #[test]
    fn source_spans_are_frozen_before_wide_star_reference_permutation() {
        let trusted = load_repository_contract().expect("frozen contract");
        let generator = trusted.generator_contract().expect("generator");
        let identity = trusted.identity_contract().expect("identity");
        let stage = trusted.stage_contract().expect("stage");
        let output =
            build_identity_stage_case(&generator, &identity, &stage, GraphProfileId::WideStar, 2)
                .expect("wide-star N=2");
        let root_references = output
            .source_input_records
            .iter()
            .filter(|record| {
                record.module_ordinal == 0 && record.record_kind == stage.record_kind_reference
            })
            .collect::<Vec<_>>();
        assert_eq!(root_references.len(), 2);
        assert!(
            root_references[0].source_span_ordinal > root_references[1].source_span_ordinal,
            "input order is permuted while pre-permutation spans retain canonical order"
        );
        assert_eq!(
            output.source_spans[root_references[1].source_span_ordinal as usize].start_line,
            1
        );
        assert_eq!(
            output.source_spans[root_references[0].source_span_ordinal as usize].start_line,
            2
        );
    }

    #[test]
    fn same_length_wrong_stable_reference_is_rejected_by_the_causal_pipeline() {
        let trusted = load_repository_contract().expect("frozen contract");
        let generator = trusted.generator_contract().expect("generator");
        let identity = trusted.identity_contract().expect("identity");
        let stage = trusted.stage_contract().expect("stage");
        let plan = IdentityStagePlan::prepare(&identity, &stage, GraphProfileId::WideStar, 2)
            .expect("N=2 plan");
        let mut buffers = IdentityStageBufferPool::default();
        let source = materialize_source_input(&generator, &identity, &stage, &plan, &mut buffers)
            .expect("source");
        let mut typed = lower_source_to_typed_ast(&identity, &stage, &plan, &source, &mut buffers)
            .expect("typed AST");
        let stable_reference = typed
            .records
            .iter()
            .find(|record| {
                record.record_kind == stage.record_kind_identity_field
                    && record.module_ordinal == plan.unit_module_base
                    && record.payload_length == 30
            })
            .expect("unit 0 StableId field");
        let module_last_hex =
            usize::try_from(stable_reference.payload_offset).expect("payload offset") + 20;
        assert_eq!(typed.payload[module_last_hex], b'1');
        typed.payload[module_last_hex] = b'2';

        let hir = lower_typed_ast_to_hir(
            &identity,
            &stage,
            &plan,
            &typed,
            &source.strings,
            &mut buffers,
        )
        .expect("the same-length spelling still parses and resolves");
        assert!(matches!(
            lower_hir_to_mir(
                &identity,
                &stage,
                &plan,
                &hir,
                &source.strings,
                &mut buffers,
            ),
            Err(StageGenerationError::InvalidSymbol {
                module_ordinal,
                ..
            }) if module_ordinal == plan.unit_module_base
        ));
    }

    #[test]
    fn work_counters_remain_linear_or_n_log_n_by_construction() {
        let trusted = load_repository_contract().expect("frozen contract");
        let identity = trusted.identity_contract().expect("identity");
        let stage = trusted.stage_contract().expect("stage");
        for profile in GraphProfileId::ALL {
            let small =
                IdentityStagePlan::prepare(&identity, &stage, profile, 8).expect("small plan");
            let large =
                IdentityStagePlan::prepare(&identity, &stage, profile, 16).expect("large plan");
            assert!(large.counts.semantic_output_record <= small.counts.semantic_output_record * 2);
            assert!(large.counts.source_span_count <= small.counts.source_span_count * 3);
            assert!(large.stages.scratch.logical_bytes <= small.stages.scratch.logical_bytes * 2);
        }
    }

    #[test]
    fn reusable_capacity_overflow_is_reported_without_panicking() {
        let mut values = Vec::<u8>::new();
        let mut allocations = ControlledAllocationTracker::<true>::new(u64::MAX);
        let error = take_reusable(
            &mut values,
            usize::MAX,
            "test buffer",
            &mut allocations,
            ControlledBufferSlot::SourcePayload,
        )
        .expect_err("an impossible capacity must fail");
        assert!(matches!(
            error,
            StageGenerationError::AllocationFailed {
                field: "test buffer",
                ..
            }
        ));
        assert!(values.is_empty());
    }
}
