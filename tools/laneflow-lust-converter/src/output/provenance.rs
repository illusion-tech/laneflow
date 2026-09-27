//! Semantic and build provenance records (§3.6 / §8).

use serde::Serialize;

use crate::{
    Result,
    output::{digest::sha256_digest, json_bytes},
    source::{LUST_COMMIT, LUST_REPOSITORY, LUST_TAG, PINNED_SOURCE_FILES},
};

/// License / NOTICE bytes included in source and static bundles.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LicenseArtifacts {
    pub license_md: Vec<u8>,
    pub odbl: Vec<u8>,
    pub notice: Vec<u8>,
}

/// Optional pinned Release URLs for generated tar assets.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReleaseAssetUrls {
    pub source_bundle_url: Option<String>,
    pub static_bundle_url: Option<String>,
}

/// Inputs for the versioned semantic provenance manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticProvenanceInput {
    pub config_toml_bytes: Vec<u8>,
    pub licenses: LicenseArtifacts,
    pub release_urls: ReleaseAssetUrls,
    pub source_tar: Vec<u8>,
    pub static_tar: Vec<u8>,
    pub network_lfca_bytes: Vec<u8>,
    pub routes_toml_bytes: Vec<u8>,
    pub manifest_bytes: Vec<u8>,
    pub conversion_report_bytes: Vec<u8>,
}

/// Inputs for the per-build provenance record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildProvenanceInput {
    pub converter_commit: String,
    pub rust_version: &'static str,
    pub cargo_lock_sha256: String,
    pub config_digest: String,
    pub semantic_provenance_digest: String,
    pub invocation: BuildInvocation,
    pub raw_output_digests: RawOutputDigests,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInvocation {
    pub command: &'static str,
    pub require_lust_location_anchors: bool,
    pub require_lust_population_count: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawOutputDigests {
    pub network_lfca: String,
    pub routes_toml: String,
    pub manifest_toml: String,
    pub conversion_report: String,
    pub source_tar: String,
    pub static_tar: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SemanticProvenanceManifest {
    format_version: &'static str,
    source_chain: SourceChain,
    config_digest: String,
    licenses: LicenseDigests,
    release_assets: ReleaseAssets,
    semantic_outputs: SemanticOutputs,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceChain {
    repository: &'static str,
    tag: &'static str,
    commit: &'static str,
    files: Vec<PinnedFileDigest>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PinnedFileDigest {
    relative_path: &'static str,
    bytes: u64,
    digest: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LicenseDigests {
    license_md: ArtifactDigest,
    odbl: ArtifactDigest,
    notice: ArtifactDigest,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseAssets {
    source_bundle: ReleaseAsset,
    static_bundle: ReleaseAsset,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseAsset {
    artifact_ref: &'static str,
    media_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    bytes: u64,
    digest: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SemanticOutputs {
    network_lfca: ArtifactDigest,
    routes_toml: ArtifactDigest,
    manifest_toml: ArtifactDigest,
    conversion_report: ArtifactDigest,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactDigest {
    artifact_ref: &'static str,
    bytes: u64,
    digest: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BuildProvenanceRecord {
    format_version: &'static str,
    converter_commit: String,
    rust_version: &'static str,
    cargo_lock_digest: String,
    config_digest: String,
    semantic_provenance_digest: String,
    invocation: BuildInvocation,
    raw_output_digests: RawOutputDigests,
}

/// Embedded ODbL 1.0 full text shipped with the converter.
pub fn embedded_odbl_bytes() -> &'static [u8] {
    include_bytes!("../../licenses/ODbL-1.0.txt")
}

/// Embedded NOTICE text shipped with the converter.
pub fn embedded_notice_bytes() -> &'static [u8] {
    include_bytes!("../../licenses/NOTICE")
}

/// Build semantic provenance JSON bytes.
pub fn build_semantic_provenance(input: &SemanticProvenanceInput) -> Result<Vec<u8>> {
    let manifest = SemanticProvenanceManifest {
        format_version: "0.2",
        source_chain: SourceChain {
            repository: LUST_REPOSITORY,
            tag: LUST_TAG,
            commit: LUST_COMMIT,
            files: PINNED_SOURCE_FILES
                .iter()
                .map(|file| PinnedFileDigest {
                    relative_path: file.relative_path,
                    bytes: file.bytes,
                    digest: format!("sha256:{}", file.sha256_hex),
                })
                .collect(),
        },
        config_digest: sha256_digest(&input.config_toml_bytes),
        licenses: LicenseDigests {
            license_md: artifact("LICENSE.md", &input.licenses.license_md),
            odbl: artifact("ODbL-1.0.txt", &input.licenses.odbl),
            notice: artifact("NOTICE", &input.licenses.notice),
        },
        release_assets: ReleaseAssets {
            source_bundle: release_asset(
                "lust-source.tar",
                input.release_urls.source_bundle_url.clone(),
                &input.source_tar,
            ),
            static_bundle: release_asset(
                "lust-static.tar",
                input.release_urls.static_bundle_url.clone(),
                &input.static_tar,
            ),
        },
        semantic_outputs: SemanticOutputs {
            network_lfca: artifact("network.lfca", &input.network_lfca_bytes),
            routes_toml: artifact("routes.toml", &input.routes_toml_bytes),
            manifest_toml: artifact("manifest.toml", &input.manifest_bytes),
            conversion_report: artifact("lust-conversion-report.json", &input.conversion_report_bytes),
        },
    };
    json_bytes("SemanticProvenanceManifest", &manifest)
}

/// Build build-provenance JSON bytes.
pub fn build_build_provenance(input: &BuildProvenanceInput) -> Result<Vec<u8>> {
    let record = BuildProvenanceRecord {
        format_version: "0.2",
        converter_commit: input.converter_commit.clone(),
        rust_version: input.rust_version,
        cargo_lock_digest: format!("sha256:{}", input.cargo_lock_sha256),
        config_digest: input.config_digest.clone(),
        semantic_provenance_digest: input.semantic_provenance_digest.clone(),
        invocation: input.invocation.clone(),
        raw_output_digests: input.raw_output_digests.clone(),
    };
    json_bytes("BuildProvenanceRecord", &record)
}

fn artifact(artifact_ref: &'static str, bytes: &[u8]) -> ArtifactDigest {
    ArtifactDigest {
        artifact_ref,
        bytes: u64::try_from(bytes.len()).expect("artifact size fits u64"),
        digest: sha256_digest(bytes),
    }
}

fn release_asset(artifact_ref: &'static str, url: Option<String>, bytes: &[u8]) -> ReleaseAsset {
    ReleaseAsset {
        artifact_ref,
        media_type: "application/x-tar",
        url,
        bytes: u64::try_from(bytes.len()).expect("artifact size fits u64"),
        digest: sha256_digest(bytes),
    }
}
