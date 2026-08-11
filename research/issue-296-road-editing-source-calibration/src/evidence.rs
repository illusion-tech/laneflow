//! #296 道路编辑正式校准的单 fresh-process 样本。

use std::path::Path;
use std::time::{Duration, Instant};

use laneflow_compiler::{
    CompilationMetrics, CompileLimits, GeometryAccuracyProfile, GeometryDirectionProfile,
};
use serde::{Deserialize, Serialize};

use crate::{
    EncodedP100Module, GeneratorError, build_base_modules_from_seed,
    build_regularity_probe_modules_from_seed, compile_encoded_modules_with_stage_timing,
    encode_modules, load_p100_seed,
};

pub const SAMPLE_SCHEMA: &str = "laneflow.road-editing-source-calibration-sample";
pub const SAMPLE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceWorkload {
    Base,
    Regularity,
}

impl EvidenceWorkload {
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Base => "LF-ROAD-EDITING-P100-v1",
            Self::Regularity => "LF-ROAD-EDITING-P100-REGULARITY-v1",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceSampleKind {
    Warmup,
    Formal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceSampleRequest {
    pub workload: EvidenceWorkload,
    pub accuracy: GeometryAccuracyProfile,
    pub direction: GeometryDirectionProfile,
    pub sample_kind: EvidenceSampleKind,
    pub sample_index: u8,
    pub argv: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceSample {
    pub schema: String,
    pub schema_version: u32,
    pub workload: String,
    pub accuracy_profile_code: u8,
    pub direction_profile_code: u8,
    pub sample_kind: EvidenceSampleKind,
    pub sample_index: u8,
    pub argv: Vec<String>,
    pub timings_ns: EvidenceTimings,
    pub fixtures: Vec<EvidenceFixture>,
    pub metrics: EvidenceMetrics,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceTimings {
    pub typed_model_build: u64,
    pub encode: u64,
    pub size_prefix_and_identifier_preflight: u64,
    pub flatbuffers_verifier: u64,
    pub semantic_preflight_and_typed_ast_lowering: u64,
    pub complete_compile: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceFixture {
    pub module_index: u8,
    pub namespace: String,
    pub source_document_key: String,
    pub byte_length: u64,
    pub retained_capacity_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceMetrics {
    pub source_bytes_total: u64,
    pub verified_table_occurrence_count: u64,
    pub geometry_point_count: u64,
    pub total_horizontal_regularity_node_visits: u64,
    pub maximum_horizontal_regularity_node_visits_per_offset_bearing_source_segment: u32,
    pub frontend_controlled_peak_bytes: u64,
    pub lir_record_count: u64,
    pub output_logical_bytes: u64,
    pub compiler_controlled_peak_bytes: u64,
    pub semantic_fingerprint: String,
}

/// 构造一次正式协议样本；调用方负责保证每次调用发生在新的进程中。
///
/// semantic seed 在第一个计时边界前读取和闭合。资源计数只读取同一次成功 production
/// compile 的 [`CompilationMetrics`]，research harness 不遍历 IR 重算第二份账本。
pub fn run_evidence_sample(
    repository_root: &Path,
    request: &EvidenceSampleRequest,
) -> Result<EvidenceSample, GeneratorError> {
    validate_request(request)?;
    let limits = CompileLimits::p100_initial_v2();
    let seed = load_p100_seed(repository_root)?;

    let typed_model_started = Instant::now();
    let modules = match request.workload {
        EvidenceWorkload::Base => {
            build_base_modules_from_seed(seed, request.accuracy, request.direction, &limits)?
        }
        EvidenceWorkload::Regularity => build_regularity_probe_modules_from_seed(seed, &limits)?,
    };
    let typed_model_build = typed_model_started.elapsed();

    let encode_started = Instant::now();
    let encoded = encode_modules(modules, &limits)?;
    let encode = encode_started.elapsed();
    let fixtures = encoded.iter().map(fixture_record).collect::<Vec<_>>();

    let (output, compile) = compile_encoded_modules_with_stage_timing(&encoded, limits)?;
    let metrics = metrics_record(output.metrics());
    Ok(EvidenceSample {
        schema: SAMPLE_SCHEMA.to_owned(),
        schema_version: SAMPLE_SCHEMA_VERSION,
        workload: request.workload.id().to_owned(),
        accuracy_profile_code: request.accuracy as u8,
        direction_profile_code: request.direction as u8,
        sample_kind: request.sample_kind,
        sample_index: request.sample_index,
        argv: request.argv.clone(),
        timings_ns: EvidenceTimings {
            typed_model_build: duration_ns(typed_model_build),
            encode: duration_ns(encode),
            size_prefix_and_identifier_preflight: duration_ns(
                compile.size_prefix_and_identifier_preflight(),
            ),
            flatbuffers_verifier: duration_ns(compile.flatbuffers_verifier()),
            semantic_preflight_and_typed_ast_lowering: duration_ns(
                compile.semantic_preflight_and_typed_ast_lowering(),
            ),
            complete_compile: duration_ns(compile.complete_compile()),
        },
        fixtures,
        metrics,
    })
}

fn validate_request(request: &EvidenceSampleRequest) -> Result<(), GeneratorError> {
    match request.sample_kind {
        EvidenceSampleKind::Warmup if request.sample_index != 0 => {
            return Err(GeneratorError::Contract(
                "warmup sample index must be exactly zero".to_owned(),
            ));
        }
        EvidenceSampleKind::Formal if !(1..=7).contains(&request.sample_index) => {
            return Err(GeneratorError::Contract(
                "formal sample index must be in 1..=7".to_owned(),
            ));
        }
        _ => {}
    }
    if request.workload == EvidenceWorkload::Regularity
        && (request.accuracy != GeometryAccuracyProfile::Fine2Cm
            || request.direction != GeometryDirectionProfile::Smooth1Deg)
    {
        return Err(GeneratorError::Contract(
            "regularity workload requires Fine2Cm and Smooth1Deg".to_owned(),
        ));
    }
    Ok(())
}

fn fixture_record(module: &EncodedP100Module) -> EvidenceFixture {
    EvidenceFixture {
        module_index: module.module_index(),
        namespace: module.namespace().to_owned(),
        source_document_key: module.source_document_key().to_owned(),
        byte_length: usize_u64(module.as_bytes().len()),
        retained_capacity_bytes: usize_u64(module.retained_capacity_bytes()),
        sha256: hex(&module.sha256()),
    }
}

fn metrics_record(metrics: CompilationMetrics) -> EvidenceMetrics {
    EvidenceMetrics {
        source_bytes_total: metrics.source_bytes_total(),
        verified_table_occurrence_count: metrics.verified_table_occurrence_count(),
        geometry_point_count: metrics.geometry_point_count(),
        total_horizontal_regularity_node_visits: metrics.total_horizontal_regularity_node_visits(),
        maximum_horizontal_regularity_node_visits_per_offset_bearing_source_segment: metrics
            .maximum_horizontal_regularity_node_visits_per_offset_bearing_source_segment(),
        frontend_controlled_peak_bytes: metrics.frontend_controlled_peak_bytes(),
        lir_record_count: metrics.lir_record_count(),
        output_logical_bytes: metrics.output_logical_bytes(),
        compiler_controlled_peak_bytes: metrics.compiler_controlled_peak_bytes(),
        semantic_fingerprint: hex(&metrics.semantic_fingerprint()),
    }
}

fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn usize_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(workload: EvidenceWorkload) -> EvidenceSampleRequest {
        EvidenceSampleRequest {
            workload,
            accuracy: GeometryAccuracyProfile::Fine2Cm,
            direction: GeometryDirectionProfile::Smooth1Deg,
            sample_kind: EvidenceSampleKind::Formal,
            sample_index: 1,
            argv: vec!["calibrate".to_owned()],
        }
    }

    #[test]
    fn sample_contract_rejects_noncanonical_indices_and_probe_profiles() {
        let mut invalid = request(EvidenceWorkload::Base);
        invalid.sample_index = 0;
        assert!(validate_request(&invalid).is_err());
        invalid.sample_kind = EvidenceSampleKind::Warmup;
        invalid.sample_index = 1;
        assert!(validate_request(&invalid).is_err());

        let mut probe = request(EvidenceWorkload::Regularity);
        probe.accuracy = GeometryAccuracyProfile::Balanced5Cm;
        assert!(validate_request(&probe).is_err());
    }

    #[test]
    fn base_sample_reads_production_metrics_and_fixture_identity() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap();
        let sample =
            run_evidence_sample(repository_root, &request(EvidenceWorkload::Base)).unwrap();

        assert_eq!(sample.schema, SAMPLE_SCHEMA);
        assert_eq!(sample.fixtures.len(), 5);
        assert_eq!(sample.metrics.verified_table_occurrence_count, 3_165);
        assert_eq!(sample.metrics.total_horizontal_regularity_node_visits, 0);
        assert!(sample.metrics.compiler_controlled_peak_bytes > 0);
        assert!(sample.timings_ns.complete_compile > 0);
    }

    #[test]
    fn regularity_sample_preserves_the_exact_visit_count() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap();
        let sample =
            run_evidence_sample(repository_root, &request(EvidenceWorkload::Regularity)).unwrap();

        assert_eq!(sample.metrics.total_horizontal_regularity_node_visits, 3);
        assert_eq!(
            sample
                .metrics
                .maximum_horizontal_regularity_node_visits_per_offset_bearing_source_segment,
            3
        );
    }
}
