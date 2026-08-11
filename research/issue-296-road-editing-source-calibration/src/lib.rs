//! #296 RoadEditingSource P100 的非生产语义种子与校准边界。

mod evidence;
mod seed;

pub use evidence::{
    EvidenceSample, EvidenceSampleKind, EvidenceSampleRequest, EvidenceWorkload, SAMPLE_SCHEMA,
    SAMPLE_SCHEMA_VERSION, run_evidence_sample,
};

pub use seed::{
    EncodedP100Module, GeneratorError, LoadedP100Seed, P100_PROFILE_COMBINATIONS,
    P100CompileStageDurations, P100ProfileCombination, SeedAudit, SeedError, TypedP100Module,
    build_base_modules, build_base_modules_from_seed, build_regularity_probe_modules,
    build_regularity_probe_modules_from_seed, compile_encoded_modules,
    compile_encoded_modules_with_stage_timing, encode_modules, load_bound_seed, load_p100_seed,
};
