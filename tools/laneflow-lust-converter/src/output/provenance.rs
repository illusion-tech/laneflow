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

/// 语义配置子集：随语义 provenance 求摘要的配置字段。
///
/// 只含影响语义产物的 Release asset URL；执行侧字段（converter_commit、
/// source_dir、output_dir 等）属 build provenance，不得进入语义摘要
/// （§3.6：语义 digest 不含 converter commit / toolchain / host）。
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticConfig {
    pub source_bundle_url: Option<String>,
    pub static_bundle_url: Option<String>,
}

/// #253 T3：source tar 的类型级绑定——只能由 verify 路径构造（
/// `VerifiedSourceSet` 背书）：本 provenance 断言 pinned 仓 / commit / 完整
/// 文件表，source 链的可信性必须由验证集承担，不接受任意字节。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedSourceTar {
    pub(crate) bytes: Vec<u8>,
}

impl VerifiedSourceTar {
    /// 由已验证 source 集构建确定性 source tar（嵌入 ODbL / NOTICE）。
    pub fn from_verified_set(verified: &crate::source::VerifiedSourceSet) -> Result<Self> {
        crate::output::pipeline::build_source_tar(verified)
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Inputs for the versioned semantic provenance manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticProvenanceInput {
    pub semantic_config: SemanticConfig,
    pub licenses: LicenseArtifacts,
    pub release_urls: ReleaseAssetUrls,
    pub source_tar: VerifiedSourceTar,
    /// 诊断模式（G1 重划）不交付 static bundle：`None` 时 releaseAssets /
    /// semanticOutputs 的对应字段序列化不出现（#253 N1）。
    pub static_tar: Option<Vec<u8>>,
    /// 同上：诊断模式无 network.lfca（编译另立 G1），认证对象为诊断清单。
    pub network_lfca_bytes: Option<Vec<u8>>,
    /// 诊断交付物（`issue253-infeasible-survey.md`）；fail-fast 路径为 None。
    pub infeasibility_survey_bytes: Option<Vec<u8>>,
    /// 诊断模式为 None（#253 L1：routes.toml 不产出）。
    pub routes_toml_bytes: Option<Vec<u8>>,
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
    /// 诊断模式为 None（该字段序列化不出现）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_lfca: Option<String>,
    /// 诊断模式为 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routes_toml: Option<String>,
    pub manifest_toml: String,
    pub conversion_report: String,
    pub source_tar: String,
    /// 诊断模式为 None（不产出 static tar）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub static_tar: Option<String>,
    /// 诊断交付物摘要（fail-fast 路径为 None）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub infeasibility_survey: Option<String>,
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
    /// 诊断模式不交付 static bundle：字段不出现（#253 N1）。
    #[serde(skip_serializing_if = "Option::is_none")]
    static_bundle: Option<ReleaseAsset>,
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
    /// 诊断模式无 network.lfca：字段不出现，由 infeasibility_survey 接任
    /// 交付物认证（#253 N1）。
    #[serde(skip_serializing_if = "Option::is_none")]
    network_lfca: Option<ArtifactDigest>,
    /// 诊断模式为 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    routes_toml: Option<ArtifactDigest>,
    manifest_toml: ArtifactDigest,
    conversion_report: ArtifactDigest,
    #[serde(skip_serializing_if = "Option::is_none")]
    infeasibility_survey: Option<ArtifactDigest>,
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
        config_digest: sha256_digest(&crate::output::json_bytes(
            "SemanticConfig",
            &input.semantic_config,
        )?),
        licenses: LicenseDigests {
            license_md: artifact("LICENSE.md", &input.licenses.license_md),
            odbl: artifact("ODbL-1.0.txt", &input.licenses.odbl),
            notice: artifact("NOTICE", &input.licenses.notice),
        },
        release_assets: ReleaseAssets {
            source_bundle: release_asset(
                "lust-source.tar",
                input.release_urls.source_bundle_url.clone(),
                input.source_tar.bytes(),
            ),
            static_bundle: input.static_tar.as_ref().map(|tar| {
                release_asset(
                    "lust-static.tar",
                    input.release_urls.static_bundle_url.clone(),
                    tar,
                )
            }),
        },
        semantic_outputs: SemanticOutputs {
            network_lfca: input
                .network_lfca_bytes
                .as_ref()
                .map(|bytes| artifact("network.lfca", bytes)),
            routes_toml: input
                .routes_toml_bytes
                .as_ref()
                .map(|bytes| artifact("routes.toml", bytes)),
            manifest_toml: artifact("manifest.toml", &input.manifest_bytes),
            conversion_report: artifact(
                "lust-conversion-report.json",
                &input.conversion_report_bytes,
            ),
            infeasibility_survey: input
                .infeasibility_survey_bytes
                .as_ref()
                .map(|bytes| artifact("issue253-infeasible-survey.md", bytes)),
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

#[cfg(test)]
mod tests {
    use super::{
        BuildInvocation, BuildProvenanceInput, LicenseArtifacts, RawOutputDigests,
        ReleaseAssetUrls, SemanticConfig, SemanticProvenanceInput, VerifiedSourceTar,
        build_build_provenance, build_semantic_provenance, embedded_notice_bytes,
        embedded_odbl_bytes,
    };
    use crate::{
        output::tar::{TarMember, write_deterministic_ustar},
        source::{PINNED_SOURCE_FILES, VerifiedSourceFile, VerifiedSourceSet},
    };

    /// 合成最小 VerifiedSourceSet（七份 pinned 相对路径摆位；字段私有化后
    /// 走 crate 内 `synthetic_for_tests` 构造——#253 U1 的类型级保证）。
    fn synthetic_verified_source_tar() -> VerifiedSourceTar {
        let root = std::env::temp_dir().join(format!("lust-provenance-src-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for pinned in PINNED_SOURCE_FILES {
            let path = root.join(pinned.relative_path);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("create dir");
            std::fs::write(&path, b"synthetic").expect("write synthetic pinned file");
        }
        // read_verified 消费时重哈希：记录必须对合成字节真实。
        let synthetic_hex = crate::output::digest::hex_sha256(b"synthetic");
        let verified = VerifiedSourceSet::synthetic_for_tests(
            PINNED_SOURCE_FILES
                .iter()
                .map(|pinned| {
                    VerifiedSourceFile::synthetic_for_tests(
                        pinned.relative_path,
                        root.join(pinned.relative_path),
                        pinned.bytes,
                        synthetic_hex.clone(),
                    )
                })
                .collect(),
        );
        let tar = VerifiedSourceTar::from_verified_set(&verified).expect("verified source tar");
        let _ = std::fs::remove_dir_all(&root);
        tar
    }

    #[test]
    fn semantic_and_build_provenance_are_byte_deterministic() {
        let licenses = LicenseArtifacts {
            license_md: b"MIT\\n".to_vec(),
            odbl: embedded_odbl_bytes().to_vec(),
            notice: embedded_notice_bytes().to_vec(),
        };
        let source_tar = synthetic_verified_source_tar();
        let semantic_input = SemanticProvenanceInput {
            semantic_config: SemanticConfig::default(),
            licenses,
            release_urls: ReleaseAssetUrls::default(),
            source_tar: source_tar.clone(),
            static_tar: None,
            network_lfca_bytes: Some(b"LFCA\\n".to_vec()),
            infeasibility_survey_bytes: None,
            routes_toml_bytes: Some(b"format_version = \"0.1\"\n".to_vec()),
            manifest_bytes: b"manifest_version = 1\\n".to_vec(),
            conversion_report_bytes: b"{}\\n".to_vec(),
        };
        let first = build_semantic_provenance(&semantic_input).expect("semantic");
        let second = build_semantic_provenance(&semantic_input).expect("semantic again");
        assert_eq!(first, second);
        assert!(String::from_utf8_lossy(&first).contains("lust-source.tar"));
        assert!(!String::from_utf8_lossy(&first).contains("converterCommit"));

        let build_input = BuildProvenanceInput {
            converter_commit: "abc123".to_owned(),
            rust_version: "1.98.0",
            cargo_lock_sha256: "deadbeef".to_owned(),
            config_digest: "sha256:00".to_owned(),
            semantic_provenance_digest: "sha256:11".to_owned(),
            invocation: BuildInvocation {
                command: "convert",
                require_lust_location_anchors: true,
                require_lust_population_count: true,
            },
            raw_output_digests: RawOutputDigests {
                network_lfca: Some("sha256:a".to_owned()),
                routes_toml: Some("sha256:b".to_owned()),
                manifest_toml: "sha256:c".to_owned(),
                conversion_report: "sha256:d".to_owned(),
                source_tar: "sha256:f".to_owned(),
                static_tar: None,
                infeasibility_survey: None,
            },
        };
        let build_a = build_build_provenance(&build_input).expect("build");
        let build_b = build_build_provenance(&build_input).expect("build again");
        assert_eq!(build_a, build_b);
        assert!(String::from_utf8_lossy(&build_a).contains("converterCommit"));
    }

    #[test]
    fn semantic_digest_tracks_only_semantic_config_subset() {
        let licenses = || LicenseArtifacts {
            license_md: b"MIT\\n".to_vec(),
            odbl: embedded_odbl_bytes().to_vec(),
            notice: embedded_notice_bytes().to_vec(),
        };
        let make_input = |semantic_config: SemanticConfig| SemanticProvenanceInput {
            semantic_config,
            licenses: licenses(),
            release_urls: ReleaseAssetUrls::default(),
            source_tar: synthetic_verified_source_tar(),
            static_tar: Some(
                write_deterministic_ustar(&[TarMember {
                    path: "network.lfca".to_owned(),
                    contents: b"LFCA\\n".to_vec(),
                }])
                .expect("static tar"),
            ),
            network_lfca_bytes: Some(b"LFCA\\n".to_vec()),
            infeasibility_survey_bytes: None,
            routes_toml_bytes: Some(b"format_version = \"0.1\"\n".to_vec()),
            manifest_bytes: b"manifest_version = 1\\n".to_vec(),
            conversion_report_bytes: b"{}\\n".to_vec(),
        };
        let base = make_input(SemanticConfig::default());
        let base_bytes = build_semantic_provenance(&base).expect("base semantic");
        let base_text = String::from_utf8_lossy(&base_bytes);
        assert!(
            !base_text.contains("sourceDir") && !base_text.contains("outputDir"),
            "执行侧字段不得进入语义 manifest"
        );
        // 语义配置子集变化 → digest 变。
        let other_urls = make_input(SemanticConfig {
            source_bundle_url: Some("https://example.invalid/a.tar".to_owned()),
            static_bundle_url: None,
        });
        let other_bytes = build_semantic_provenance(&other_urls).expect("other semantic");
        assert_ne!(
            base_bytes, other_bytes,
            "语义配置变化必须改变 semantic provenance"
        );
        // 同语义配置、不同执行侧产物字节 → digest 不变。
        let same_config_different_outputs = make_input(SemanticConfig::default());
        let same_bytes =
            build_semantic_provenance(&same_config_different_outputs).expect("same semantic");
        assert_eq!(base_bytes, same_bytes, "语义 digest 只随语义配置子集变化");
    }
}
