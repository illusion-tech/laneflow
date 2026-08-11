//! #296 RoadEditingSource P100 的非生产语义种子与校准边界。

mod allocation;
mod compact;
mod evidence;
mod oracle;
mod protocol;
mod seed;

pub use allocation::{
    ALLOCATOR_PROBE_SCHEMA, ALLOCATOR_PROBE_SCHEMA_VERSION, AllocatorHeapObservation,
    AllocatorProbe, AllocatorProbeRequest, AllocatorProbeRole, PreloadedRevisionObservation,
    run_allocator_probe,
};
pub use compact::{
    COMPACT_EVIDENCE_SCHEMA, COMPACT_EVIDENCE_SCHEMA_PATH, COMPACT_EVIDENCE_SCHEMA_VERSION,
    CompactEvidence, CompactEvidenceWriteOutcome, EvidenceArtifactBinding, EvidenceCoverage,
    verify_compact_evidence, write_compact_evidence,
};
pub use evidence::{
    EvidenceFixture, EvidenceMetrics, EvidenceSample, EvidenceSampleKind, EvidenceSampleRequest,
    EvidenceTimings, EvidenceWorkload, SAMPLE_SCHEMA, SAMPLE_SCHEMA_VERSION,
    SingleModuleRewriteEvidence, SingleModuleRewriteTimings, run_evidence_sample,
};
pub use oracle::{GeometryObservation, PositionErrorStatistics, WorstObservedError};
pub use protocol::{
    AllocatorProbeInvocation, EvidenceEnvironment, EvidenceFileBinding, EvidenceInvocation,
    EvidenceProtocol, EvidenceSource, EvidenceSummary, EvidenceTimingSummary, MedianMad,
    RAW_EVIDENCE_SCHEMA, RAW_EVIDENCE_SCHEMA_VERSION, RawEvidence, SingleModuleRewriteSummary,
    SingleModuleRewriteTimingSummary, run_evidence_protocol, validate_raw_evidence,
};

pub use seed::{
    EncodedP100Module, GeneratorError, LoadedP100Seed, P100_PROFILE_COMBINATIONS,
    P100CompileStageDurations, P100ProfileCombination, SeedAudit, SeedError, TypedP100Module,
    build_base_modules, build_base_modules_from_seed, build_regularity_probe_modules,
    build_regularity_probe_modules_from_seed, build_rewrite_candidate_module_from_seed,
    compile_encoded_modules, compile_encoded_modules_with_stage_timing,
    compile_rewrite_candidate_modules, encode_module, encode_modules, load_bound_seed,
    load_p100_seed,
};
