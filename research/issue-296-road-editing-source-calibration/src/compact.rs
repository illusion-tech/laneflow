//! #296 道路编辑校准的紧凑 evidence 与独立重算验证。
//!
//! raw evidence 保留 80 个 fresh-process 样本；本模块只发布 exact source/environment、
//! 交叉验证后的十组摘要与 raw artifact 的 repository-relative SHA-256 绑定。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    AllocatorProbe, EvidenceEnvironment, EvidenceFileBinding, EvidenceProtocol, EvidenceSource,
    EvidenceSummary, RawEvidence, validate_raw_evidence,
};

pub const COMPACT_EVIDENCE_SCHEMA: &str = "laneflow.road-editing-source-calibration-evidence";
pub const COMPACT_EVIDENCE_SCHEMA_VERSION: u32 = 1;
pub const COMPACT_EVIDENCE_SCHEMA_PATH: &str =
    "docs/reference/road-editing-source-calibration-evidence-v1.schema.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompactEvidence {
    pub schema: String,
    pub schema_version: u32,
    pub raw_evidence: EvidenceArtifactBinding,
    pub evidence_schema: EvidenceArtifactBinding,
    pub source: EvidenceSource,
    pub environment: EvidenceEnvironment,
    pub protocol: EvidenceProtocol,
    pub bindings: Vec<EvidenceFileBinding>,
    pub coverage: EvidenceCoverage,
    pub allocator_probes: Vec<AllocatorProbe>,
    pub summaries: Vec<EvidenceSummary>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceArtifactBinding {
    pub path: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceCoverage {
    pub timed_process_invocation_count: u64,
    pub allocator_probe_invocation_count: u64,
    pub total_fresh_process_count: u64,
    pub raw_sample_count: u64,
    pub warmup_sample_count: u64,
    pub formal_sample_count: u64,
    pub summary_count: u64,
    pub base_profile_combination_count: u64,
    pub regularity_companion_count: u64,
    pub rewrite_sample_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompactEvidenceWriteOutcome {
    pub output_path: String,
    pub byte_length: u64,
    pub sha256: String,
    pub coverage: EvidenceCoverage,
}

pub fn write_compact_evidence(
    repository_root: &Path,
    raw_path: &Path,
    output_path: &Path,
) -> Result<CompactEvidenceWriteOutcome, String> {
    if output_path.exists() {
        return Err(format!(
            "compact evidence output already exists: {}",
            output_path.display()
        ));
    }
    let raw_relative = repository_relative(repository_root, raw_path)?;
    let raw_bytes = fs::read(raw_path)
        .map_err(|error| format!("cannot read raw evidence {}: {error}", raw_path.display()))?;
    let raw: RawEvidence = serde_json::from_slice(&raw_bytes)
        .map_err(|error| format!("invalid raw evidence {}: {error}", raw_path.display()))?;
    validate_raw_evidence(repository_root, &raw)?;
    let compact = build_compact(repository_root, &raw, &raw_relative, &raw_bytes)?;
    validate_schema_at_measurement_commit(repository_root, &compact)?;
    let mut bytes = serde_json::to_vec_pretty(&compact)
        .map_err(|error| format!("cannot serialize compact evidence: {error}"))?;
    bytes.push(b'\n');
    write_new(output_path, &bytes)?;
    Ok(CompactEvidenceWriteOutcome {
        output_path: repository_relative(repository_root, output_path)?,
        byte_length: usize_u64(bytes.len()),
        sha256: sha256_hex(&bytes),
        coverage: compact.coverage,
    })
}

pub fn verify_compact_evidence(
    repository_root: &Path,
    compact_path: &Path,
) -> Result<CompactEvidenceWriteOutcome, String> {
    let compact_bytes = fs::read(compact_path).map_err(|error| {
        format!(
            "cannot read compact evidence {}: {error}",
            compact_path.display()
        )
    })?;
    let compact: CompactEvidence = serde_json::from_slice(&compact_bytes).map_err(|error| {
        format!(
            "invalid compact evidence {}: {error}",
            compact_path.display()
        )
    })?;
    validate_schema_at_measurement_commit(repository_root, &compact)?;
    let raw_path = resolve_repository_relative(repository_root, &compact.raw_evidence.path)?;
    let raw_bytes = fs::read(&raw_path).map_err(|error| {
        format!(
            "cannot read bound raw evidence {}: {error}",
            raw_path.display()
        )
    })?;
    if compact.raw_evidence.byte_length != usize_u64(raw_bytes.len())
        || compact.raw_evidence.sha256 != sha256_hex(&raw_bytes)
    {
        return Err("compact evidence raw artifact binding does not match bytes".to_owned());
    }
    let raw: RawEvidence = serde_json::from_slice(&raw_bytes)
        .map_err(|error| format!("invalid bound raw evidence: {error}"))?;
    validate_raw_evidence(repository_root, &raw)?;
    let recomputed = build_compact(
        repository_root,
        &raw,
        &compact.raw_evidence.path,
        &raw_bytes,
    )?;
    if compact != recomputed {
        return Err("compact evidence differs from independent raw recomputation".to_owned());
    }
    Ok(CompactEvidenceWriteOutcome {
        output_path: repository_relative(repository_root, compact_path)?,
        byte_length: usize_u64(compact_bytes.len()),
        sha256: sha256_hex(&compact_bytes),
        coverage: compact.coverage,
    })
}

fn build_compact(
    repository_root: &Path,
    raw: &RawEvidence,
    raw_path: &str,
    raw_bytes: &[u8],
) -> Result<CompactEvidence, String> {
    let schema_bytes = git_blob(
        repository_root,
        &raw.source.measurement_commit,
        COMPACT_EVIDENCE_SCHEMA_PATH,
    )?;
    let warmup_sample_count = raw
        .samples
        .iter()
        .filter(|sample| matches!(sample.sample_kind, crate::EvidenceSampleKind::Warmup))
        .count();
    let formal_sample_count = raw
        .samples
        .iter()
        .filter(|sample| matches!(sample.sample_kind, crate::EvidenceSampleKind::Formal))
        .count();
    let rewrite_sample_count = raw
        .samples
        .iter()
        .filter(|sample| sample.single_module_rewrite.is_some())
        .count();
    Ok(CompactEvidence {
        schema: COMPACT_EVIDENCE_SCHEMA.to_owned(),
        schema_version: COMPACT_EVIDENCE_SCHEMA_VERSION,
        raw_evidence: EvidenceArtifactBinding {
            path: raw_path.to_owned(),
            byte_length: usize_u64(raw_bytes.len()),
            sha256: sha256_hex(raw_bytes),
        },
        evidence_schema: EvidenceArtifactBinding {
            path: COMPACT_EVIDENCE_SCHEMA_PATH.to_owned(),
            byte_length: usize_u64(schema_bytes.len()),
            sha256: sha256_hex(&schema_bytes),
        },
        source: raw.source.clone(),
        environment: raw.environment.clone(),
        protocol: raw.protocol.clone(),
        bindings: raw.bindings.clone(),
        coverage: EvidenceCoverage {
            timed_process_invocation_count: usize_u64(raw.invocations.len()),
            allocator_probe_invocation_count: usize_u64(raw.allocator_probe_invocations.len()),
            total_fresh_process_count: usize_u64(
                raw.invocations
                    .len()
                    .checked_add(raw.allocator_probe_invocations.len())
                    .ok_or_else(|| "fresh-process coverage count overflow".to_owned())?,
            ),
            raw_sample_count: usize_u64(raw.samples.len()),
            warmup_sample_count: usize_u64(warmup_sample_count),
            formal_sample_count: usize_u64(formal_sample_count),
            summary_count: usize_u64(raw.summaries.len()),
            base_profile_combination_count: 9,
            regularity_companion_count: 1,
            rewrite_sample_count: usize_u64(rewrite_sample_count),
        },
        allocator_probes: raw.allocator_probes.clone(),
        summaries: raw.summaries.clone(),
    })
}

fn validate_schema_at_measurement_commit(
    repository_root: &Path,
    compact: &CompactEvidence,
) -> Result<(), String> {
    if compact.schema != COMPACT_EVIDENCE_SCHEMA
        || compact.schema_version != COMPACT_EVIDENCE_SCHEMA_VERSION
        || compact.raw_evidence.schema_path_is_invalid()
        || compact.evidence_schema.path != COMPACT_EVIDENCE_SCHEMA_PATH
        || compact.coverage.timed_process_invocation_count != 80
        || compact.coverage.allocator_probe_invocation_count != 4
        || compact.coverage.total_fresh_process_count != 84
        || compact.coverage.raw_sample_count != 80
        || compact.coverage.warmup_sample_count != 10
        || compact.coverage.formal_sample_count != 70
        || compact.coverage.summary_count != 10
        || compact.coverage.base_profile_combination_count != 9
        || compact.coverage.regularity_companion_count != 1
        || compact.coverage.rewrite_sample_count != 8
        || compact.allocator_probes.len() != 4
    {
        return Err("compact evidence identity or exact coverage is invalid".to_owned());
    }
    let schema_bytes = git_blob(
        repository_root,
        &compact.source.measurement_commit,
        COMPACT_EVIDENCE_SCHEMA_PATH,
    )?;
    if compact.evidence_schema.byte_length != usize_u64(schema_bytes.len())
        || compact.evidence_schema.sha256 != sha256_hex(&schema_bytes)
    {
        return Err("compact evidence schema binding does not match measurement commit".to_owned());
    }
    let schema: Value = serde_json::from_slice(&schema_bytes)
        .map_err(|error| format!("invalid compact evidence schema: {error}"))?;
    let document = serde_json::to_value(compact)
        .map_err(|error| format!("cannot materialize compact evidence JSON: {error}"))?;
    jsonschema::draft202012::validate(&schema, &document).map_err(|error| {
        format!(
            "compact evidence schema violation: {error} (instance {}; schema {})",
            error.instance_path(),
            error.schema_path()
        )
    })
}

impl EvidenceArtifactBinding {
    fn schema_path_is_invalid(&self) -> bool {
        self.path.is_empty()
            || Path::new(&self.path).is_absolute()
            || Path::new(&self.path)
                .components()
                .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    }
}

fn git_blob(repository_root: &Path, commit: &str, path: &str) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .current_dir(repository_root)
        .args(["show", &format!("{commit}:{path}")])
        .output()
        .map_err(|error| format!("cannot read measured Git object {path}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git show failed for measured object {path}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("cannot sync {}: {error}", path.display()))
}

fn repository_relative(repository_root: &Path, path: &Path) -> Result<String, String> {
    let relative = path.strip_prefix(repository_root).map_err(|_| {
        format!(
            "evidence path is outside repository root: {}",
            path.display()
        )
    })?;
    let value = relative.to_string_lossy().replace('\\', "/");
    validate_relative_path(&value)?;
    Ok(value)
}

fn resolve_repository_relative(
    repository_root: &Path,
    path: &str,
) -> Result<std::path::PathBuf, String> {
    validate_relative_path(path)?;
    Ok(repository_root.join(path))
}

fn validate_relative_path(path: &str) -> Result<(), String> {
    let path = Path::new(path);
    if path.is_absolute()
        || path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(
            "evidence artifact path must be repository-relative without traversal".to_owned(),
        );
    }
    Ok(())
}

fn usize_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let mut output = String::with_capacity(64);
    use std::fmt::Write as _;
    for byte in digest {
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/reference/road-editing-source-calibration-evidence-v1.schema.json"
    ));

    #[test]
    fn compact_evidence_schema_is_valid_draft_2020_12() {
        let schema: Value = serde_json::from_str(SCHEMA).expect("schema JSON must parse");
        jsonschema::draft202012::meta::validate(&schema)
            .expect("compact evidence schema must satisfy Draft 2020-12");
    }

    #[test]
    fn artifact_paths_are_repository_relative() {
        assert!(validate_relative_path("target/evidence/raw.json").is_ok());
        assert!(validate_relative_path("../raw.json").is_err());
        assert!(validate_relative_path("/absolute/raw.json").is_err());
        assert!(validate_relative_path("").is_err());
    }
}
