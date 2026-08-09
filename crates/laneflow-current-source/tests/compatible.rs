//! `laneflow-current-source` production-compatible 能力的行为测试。
//!
//! 覆盖：版本闸口（缺失/null/非字符串/重复 `formatVersion`）、
//! `deny_unknown_fields`、digest 词法、size-before-digest、conflicting/
//! duplicate/missing/空 ref、额外制品只查非空/唯一（不哈希、不解析）、
//! 长 ref、>16 个唯一额外制品、128 字节 current ID 接受用例、冻结失败顺序，
//! 以及 Traffic-only 能力不虚构 Manifest/Spatial。

use laneflow_current_source::{
    CURRENT_SCENARIO_MANIFEST_FORMAT_VERSION, CURRENT_SPATIAL_FORMAT_VERSION,
    CURRENT_TRAFFIC_FORMAT_VERSION, CurrentArtifactInput, CurrentArtifactRole, CurrentDocumentRole,
    CurrentSourceError, CurrentSourceErrorPayload, CurrentSourceIssueContext,
    SPATIAL_PACKAGE_MEDIA_TYPE, TRAFFIC_PACKAGE_MEDIA_TYPE, validate_scenario_compatible,
    validate_traffic_compatible,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const TRAFFIC_REF: &str = "v0.10-empty-signals-and-parking.laneflow.json";
const SPATIAL_REF: &str = "v0.1-campus.spatial.json";
const TRAFFIC: &[u8] =
    include_bytes!("../../../examples/data/v0.10-empty-signals-and-parking.laneflow.json");
const SPATIAL: &[u8] = include_bytes!("../../../examples/data/v0.1-campus.spatial.json");
const MANIFEST: &str = include_str!("../../../examples/data/v0.1-campus.scenario.json");

#[test]
fn current_constants_pin_the_frozen_versions_and_media_types() {
    assert_eq!(CURRENT_TRAFFIC_FORMAT_VERSION, "0.10");
    assert_eq!(CURRENT_SCENARIO_MANIFEST_FORMAT_VERSION, "0.1");
    assert_eq!(CURRENT_SPATIAL_FORMAT_VERSION, "0.1");
    assert_eq!(
        TRAFFIC_PACKAGE_MEDIA_TYPE,
        "application/vnd.laneflow.traffic+json"
    );
    assert_eq!(
        SPATIAL_PACKAGE_MEDIA_TYPE,
        "application/vnd.laneflow.spatial+json"
    );
}

#[test]
fn traffic_only_facade_accepts_current_fixture_without_scenario_documents() {
    let validated =
        validate_traffic_compatible(TRAFFIC).expect("current traffic fixture must validate");
    assert!(format!("{validated:?}").contains("ValidatedCurrentTrafficPackage"));
    let parts = validated.into_parts();
    assert_eq!(parts.traffic_wire().units().distance(), "meter");
    assert_eq!(parts.traffic_wire().units().time(), "second");
    let wire = parts.into_traffic_wire();
    assert_eq!(wire.lane_graph().edges().len(), 4);
}

#[test]
fn traffic_version_gate_rejects_missing_null_non_string_and_duplicate_occurrence() {
    let missing = TRAFFIC_TEXT.replacen("\"formatVersion\": \"0.10\",", "", 1);
    let issue = single_issue(
        validate_traffic_compatible(missing.as_bytes()).expect_err("missing formatVersion"),
    );
    assert_eq!(issue.document(), Some(CurrentDocumentRole::Traffic));
    assert_eq!(issue.artifact_ref(), None);
    assert_eq!(issue.path(), Some("$"));
    assert!(matches!(
        issue.payload(),
        CurrentSourceErrorPayload::JsonShape { .. }
    ));
    assert_eq!(issue.stable_code(), "LF-CURRENT-SOURCE-JSON-SHAPE");

    for replacement in ["\"formatVersion\": null", "\"formatVersion\": 0.10"] {
        let mutated = TRAFFIC_TEXT.replacen("\"formatVersion\": \"0.10\"", replacement, 1);
        let issue = single_issue(
            validate_traffic_compatible(mutated.as_bytes()).expect_err("non-string formatVersion"),
        );
        assert_eq!(issue.document(), Some(CurrentDocumentRole::Traffic));
        assert!(
            matches!(issue.payload(), CurrentSourceErrorPayload::JsonShape { .. }),
            "显式 null 与非字符串 formatVersion 必须是 JsonShape"
        );
    }

    let duplicate = TRAFFIC_TEXT.replacen(
        "\"formatVersion\": \"0.10\"",
        "\"formatVersion\": \"0.10\", \"formatVersion\": \"0.10\"",
        1,
    );
    let issue = single_issue(
        validate_traffic_compatible(duplicate.as_bytes()).expect_err("duplicate formatVersion"),
    );
    assert!(
        matches!(issue.payload(), CurrentSourceErrorPayload::JsonShape { .. }),
        "重复 occurrence 不得选择任一值继续版本裁决"
    );
}

#[test]
fn traffic_unsupported_version_is_rejected_before_other_shape_errors() {
    let mut value: Value = serde_json::from_slice(TRAFFIC).expect("fixture JSON");
    value["formatVersion"] = json!("0.9");
    value["futureTopLevelField"] = json!({ "newShape": true });
    let source = serde_json::to_vec(&value).expect("JSON");
    let issue = single_issue(
        validate_traffic_compatible(&source).expect_err("unsupported version must win"),
    );
    assert_eq!(issue.document(), Some(CurrentDocumentRole::Traffic));
    assert_eq!(issue.path(), Some("$"));
    assert_eq!(issue.span(), None);
    assert_eq!(issue.artifact_ref(), None);
    match issue.payload() {
        CurrentSourceErrorPayload::UnsupportedFormatVersion { expected, actual } => {
            assert_eq!(*expected, "0.10");
            assert_eq!(&**actual, "0.9");
        }
        other => panic!(
            "expected UnsupportedFormatVersion, got {}",
            other.stable_code()
        ),
    }
}

#[test]
fn traffic_syntax_error_carries_real_position_and_syntax_category() {
    let issue = single_issue(
        validate_traffic_compatible(b"{\"formatVersion\":\"0.10\",")
            .expect_err("truncated JSON must fail"),
    );
    assert_eq!(issue.document(), Some(CurrentDocumentRole::Traffic));
    // 截断输入的 serde 分类可能是 Eof 或 Syntax，两者都归入 JsonSyntax payload。
    assert!(matches!(
        issue.category(),
        serde_json::error::Category::Eof | serde_json::error::Category::Syntax
    ));
    match issue.payload() {
        CurrentSourceErrorPayload::JsonSyntax { source } => {
            assert!(source.line() > 0 && source.column() > 0);
        }
        other => panic!("expected JsonSyntax, got {}", other.stable_code()),
    }
}

#[test]
fn traffic_wire_denies_unknown_fields() {
    let mut value: Value = serde_json::from_slice(TRAFFIC).expect("fixture JSON");
    value["futureTopLevelField"] = json!(true);
    let source = serde_json::to_vec(&value).expect("JSON");
    let issue =
        single_issue(validate_traffic_compatible(&source).expect_err("unknown field must fail"));
    assert!(matches!(
        issue.payload(),
        CurrentSourceErrorPayload::JsonShape { .. }
    ));
}

#[test]
fn traffic_facade_accepts_128_byte_current_ids() {
    let long_id = "e".repeat(128);
    let mutated = TRAFFIC_TEXT.replace("\"entry\"", &format!("\"{long_id}\""));
    validate_traffic_compatible(mutated.as_bytes()).expect("128-byte current ID must be accepted");
}

#[test]
fn scenario_happy_path_binds_descriptors_and_skips_extra_artifacts() {
    let mut artifacts = vec![
        CurrentArtifactInput::new(TRAFFIC_REF, TRAFFIC, None),
        CurrentArtifactInput::new(SPATIAL_REF, SPATIAL, None),
    ];
    // 20 个唯一额外制品（>16）：非 JSON payload 不哈希、不解析、不复制；
    // 其中含一个 4096 字符长 ref 与一个唯一大 payload。
    for index in 0..18 {
        let bytes = format!("not-json-extra-payload-{index}").into_bytes();
        artifacts.push(CurrentArtifactInput::new(
            Box::leak(format!("extra-{index}.bin").into_boxed_str()),
            Box::leak(bytes.into_boxed_slice()),
            None,
        ));
    }
    let long_ref = "x".repeat(4_096);
    artifacts.push(CurrentArtifactInput::new(
        Box::leak(long_ref.into_boxed_str()),
        b"\x00\x01 unique extra payload",
        None,
    ));
    artifacts.push(CurrentArtifactInput::new(
        "unique-extra-payload.bin",
        b"{definitely not json, never parsed}",
        None,
    ));

    let validated = validate_scenario_compatible(MANIFEST.as_bytes(), &artifacts)
        .expect("canonical scenario with extra unique artifacts must validate");
    assert_eq!(validated.traffic_digest(), sha256(TRAFFIC));
    assert_eq!(validated.spatial_digest(), sha256(SPATIAL));

    let parts = validated.into_parts();
    assert_eq!(parts.manifest().traffic().artifact_ref(), TRAFFIC_REF);
    assert_eq!(parts.manifest().spatial().artifact_ref(), SPATIAL_REF);
    assert_eq!(parts.traffic_digest(), sha256(TRAFFIC));
    assert_eq!(parts.spatial_digest(), sha256(SPATIAL));
    assert_eq!(parts.spatial_wire().frame_id(), "campus-local");
    let (manifest, traffic, spatial) = parts.into_documents();
    assert_eq!(manifest.traffic().artifact_ref(), TRAFFIC_REF);
    assert_eq!(traffic.lane_graph().edges().len(), 4);
    assert_eq!(spatial.edges().len(), 4);
}

#[test]
fn scenario_bundle_carries_exact_manifest_digest() {
    let validated = validate_scenario_compatible(MANIFEST.as_bytes(), &base_artifacts())
        .expect("canonical scenario must validate");
    assert_eq!(validated.manifest_digest(), sha256(MANIFEST.as_bytes()));
    assert_eq!(validated.traffic_digest(), sha256(TRAFFIC));
    assert_eq!(validated.spatial_digest(), sha256(SPATIAL));

    let parts = validated.into_parts();
    assert_eq!(parts.manifest_digest(), sha256(MANIFEST.as_bytes()));
    assert_eq!(parts.traffic_digest(), sha256(TRAFFIC));
    assert_eq!(parts.spatial_digest(), sha256(SPATIAL));
}

#[test]
fn manifest_version_gate_rejects_missing_null_non_string_and_duplicate_occurrence() {
    let missing = MANIFEST.replacen("\"formatVersion\": \"0.1\",", "", 1);
    let issue = single_issue(scenario_error(missing.as_bytes(), &base_artifacts()));
    assert_eq!(issue.document(), Some(CurrentDocumentRole::Manifest));
    assert_eq!(issue.path(), Some("$"));
    assert!(matches!(
        issue.payload(),
        CurrentSourceErrorPayload::JsonShape { .. }
    ));

    for replacement in ["\"formatVersion\": null", "\"formatVersion\": 1"] {
        let mutated = MANIFEST.replacen("\"formatVersion\": \"0.1\"", replacement, 1);
        let issue = single_issue(scenario_error(mutated.as_bytes(), &base_artifacts()));
        assert!(
            matches!(issue.payload(), CurrentSourceErrorPayload::JsonShape { .. }),
            "显式 null 与非字符串 formatVersion 必须是 JsonShape"
        );
    }

    let duplicate = MANIFEST.replacen(
        "\"formatVersion\": \"0.1\"",
        "\"formatVersion\": \"0.1\", \"formatVersion\": \"0.1\"",
        1,
    );
    let issue = single_issue(scenario_error(duplicate.as_bytes(), &base_artifacts()));
    assert!(
        matches!(issue.payload(), CurrentSourceErrorPayload::JsonShape { .. }),
        "重复 occurrence 不得选择任一值继续版本裁决"
    );
}

#[test]
fn manifest_unsupported_version_is_rejected_before_other_shape_errors() {
    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["formatVersion"] = json!("0.2");
    manifest["future"] = json!(true);
    let issue = single_issue(load_value(&manifest, TRAFFIC, SPATIAL));
    assert_eq!(issue.document(), Some(CurrentDocumentRole::Manifest));
    match issue.payload() {
        CurrentSourceErrorPayload::UnsupportedFormatVersion { expected, actual } => {
            assert_eq!(*expected, "0.1");
            assert_eq!(&**actual, "0.2");
        }
        other => panic!(
            "expected UnsupportedFormatVersion, got {}",
            other.stable_code()
        ),
    }
}

#[test]
fn manifest_shape_denies_unknown_fields_with_descriptor_path() {
    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["traffic"]["future"] = json!(true);
    let issue = single_issue(load_value(&manifest, TRAFFIC, SPATIAL));
    assert_eq!(issue.document(), Some(CurrentDocumentRole::Manifest));
    assert!(
        issue
            .path()
            .expect("production-compatible issue 必携带 path")
            .contains("traffic")
    );
    assert!(matches!(
        issue.payload(),
        CurrentSourceErrorPayload::JsonShape { .. }
    ));
}

#[test]
fn descriptor_semantics_enforce_ref_media_type_size_and_digest_lexeme() {
    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["traffic"]["artifactRef"] = json!("");
    let issue = single_issue(load_value(&manifest, TRAFFIC, SPATIAL));
    assert_eq!(issue.path(), Some("traffic.artifactRef"));
    assert!(matches!(
        issue.payload(),
        CurrentSourceErrorPayload::EmptyArtifactReference
    ));

    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["traffic"]["mediaType"] = json!("application/json");
    let issue = single_issue(load_value(&manifest, TRAFFIC, SPATIAL));
    assert_eq!(issue.path(), Some("traffic.mediaType"));
    match issue.payload() {
        CurrentSourceErrorPayload::InvalidMediaType { expected, actual } => {
            assert_eq!(*expected, TRAFFIC_PACKAGE_MEDIA_TYPE);
            assert_eq!(&**actual, "application/json");
        }
        other => panic!("expected InvalidMediaType, got {}", other.stable_code()),
    }

    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["spatial"]["size"] = json!(9_007_199_254_740_992_u64);
    let issue = single_issue(load_value(&manifest, TRAFFIC, SPATIAL));
    assert_eq!(issue.path(), Some("spatial.size"));
    match issue.payload() {
        CurrentSourceErrorPayload::ArtifactSizeOutOfRange { actual, max } => {
            assert_eq!(*actual, 9_007_199_254_740_992_u64);
            assert_eq!(*max, 9_007_199_254_740_991_u64);
        }
        other => panic!(
            "expected ArtifactSizeOutOfRange, got {}",
            other.stable_code()
        ),
    }

    // 边界值（恰好 portable 上限）不触发 out-of-range，而是继续走到 size 校验。
    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["spatial"]["size"] = json!(9_007_199_254_740_991_u64);
    let issue = single_issue(load_value(&manifest, TRAFFIC, SPATIAL));
    assert!(matches!(
        issue.payload(),
        CurrentSourceErrorPayload::ArtifactSizeMismatch {
            role: CurrentArtifactRole::Spatial,
            ..
        }
    ));

    for digest in ["sha256:ABCDEF", "md5:0123456789abcdef", "sha256:abc"] {
        let mut manifest = manifest_value(TRAFFIC, SPATIAL);
        manifest["spatial"]["digest"] = json!(digest);
        let issue = single_issue(load_value(&manifest, TRAFFIC, SPATIAL));
        assert_eq!(issue.path(), Some("spatial.digest"));
        match issue.payload() {
            CurrentSourceErrorPayload::InvalidDigest { actual } => {
                assert_eq!(&**actual, digest);
            }
            other => panic!("expected InvalidDigest, got {}", other.stable_code()),
        }
    }
}

#[test]
fn conflicting_manifest_refs_are_rejected_before_provided_refs() {
    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["spatial"]["artifactRef"] = json!(TRAFFIC_REF);
    let duplicate = [
        CurrentArtifactInput::new(TRAFFIC_REF, TRAFFIC, None),
        CurrentArtifactInput::new(TRAFFIC_REF, TRAFFIC, None),
    ];
    let issue = single_issue(scenario_error(
        serde_json::to_string(&manifest)
            .expect("manifest")
            .as_bytes(),
        &duplicate,
    ));
    match issue.payload() {
        CurrentSourceErrorPayload::ConflictingManifestArtifactReference { artifact_ref } => {
            assert_eq!(&**artifact_ref, TRAFFIC_REF);
        }
        other => panic!(
            "conflicting ref 必须先于 provided refs：{}",
            other.stable_code()
        ),
    }
}

#[test]
fn provided_refs_must_be_non_empty_and_unique() {
    let source = serde_json::to_vec(&manifest_value(TRAFFIC, SPATIAL)).expect("manifest");
    let empty = [
        CurrentArtifactInput::new(TRAFFIC_REF, TRAFFIC, None),
        CurrentArtifactInput::new(SPATIAL_REF, SPATIAL, None),
        CurrentArtifactInput::new("", b"extra", None),
    ];
    let issue = single_issue(scenario_error(&source, &empty));
    assert_eq!(issue.path(), Some("artifacts[2].artifactRef"));
    assert!(matches!(
        issue.payload(),
        CurrentSourceErrorPayload::EmptyArtifactReference
    ));

    let duplicate = [
        CurrentArtifactInput::new(TRAFFIC_REF, TRAFFIC, None),
        CurrentArtifactInput::new(SPATIAL_REF, SPATIAL, None),
        CurrentArtifactInput::new(SPATIAL_REF, SPATIAL, None),
    ];
    let issue = single_issue(scenario_error(&source, &duplicate));
    assert_eq!(issue.path(), Some("artifacts[2].artifactRef"));
    match issue.payload() {
        CurrentSourceErrorPayload::DuplicateProvidedArtifactReference { artifact_ref } => {
            assert_eq!(&**artifact_ref, SPATIAL_REF);
        }
        other => panic!(
            "expected DuplicateProvidedArtifactReference, got {}",
            other.stable_code()
        ),
    }
}

#[test]
fn missing_artifact_reports_role_and_ref() {
    let source = serde_json::to_vec(&manifest_value(TRAFFIC, SPATIAL)).expect("manifest");
    let missing = [CurrentArtifactInput::new(TRAFFIC_REF, TRAFFIC, None)];
    let issue = single_issue(scenario_error(&source, &missing));
    match issue.payload() {
        CurrentSourceErrorPayload::MissingArtifact { role, artifact_ref } => {
            assert_eq!(*role, CurrentArtifactRole::Spatial);
            assert_eq!(&**artifact_ref, SPATIAL_REF);
        }
        other => panic!("expected MissingArtifact, got {}", other.stable_code()),
    }
}

#[test]
fn raw_size_is_checked_before_raw_digest() {
    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["traffic"]["size"] = json!(TRAFFIC.len() + 1);
    manifest["traffic"]["digest"] = json!(format!("sha256:{}", "0".repeat(64)));
    let issue = single_issue(load_value(&manifest, TRAFFIC, SPATIAL));
    match issue.payload() {
        CurrentSourceErrorPayload::ArtifactSizeMismatch {
            role,
            artifact_ref,
            expected,
            actual,
        } => {
            assert_eq!(*role, CurrentArtifactRole::Traffic);
            assert_eq!(&**artifact_ref, TRAFFIC_REF);
            assert_eq!(*expected, TRAFFIC.len() as u64 + 1);
            assert_eq!(*actual, TRAFFIC.len() as u64);
        }
        other => panic!("size 必须先于 digest：{}", other.stable_code()),
    }

    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["traffic"]["digest"] = json!(format!("sha256:{}", "0".repeat(64)));
    let issue = single_issue(load_value(&manifest, TRAFFIC, SPATIAL));
    match issue.payload() {
        CurrentSourceErrorPayload::ArtifactDigestMismatch {
            role,
            artifact_ref,
            expected,
            actual,
        } => {
            assert_eq!(*role, CurrentArtifactRole::Traffic);
            assert_eq!(&**artifact_ref, TRAFFIC_REF);
            assert_eq!(&**expected, format!("sha256:{}", "0".repeat(64)));
            assert_eq!(&**actual, digest_text(TRAFFIC));
        }
        other => panic!(
            "expected ArtifactDigestMismatch, got {}",
            other.stable_code()
        ),
    }
}

#[test]
fn traffic_wire_issues_carry_scenario_traffic_context() {
    let invalid_traffic = br#"{"formatVersion":"0.7"}"#;
    let manifest = manifest_value(invalid_traffic, SPATIAL);
    let issue = single_issue(load_value(&manifest, invalid_traffic, SPATIAL));
    assert_eq!(issue.document(), Some(CurrentDocumentRole::Traffic));
    // artifact_ref() 是 ScenarioTraffic context 的唯一借用视图（:785-787）。
    assert_eq!(issue.artifact_ref(), Some(TRAFFIC_REF));
    match issue.payload() {
        CurrentSourceErrorPayload::UnsupportedFormatVersion { expected, actual } => {
            assert_eq!(*expected, "0.10");
            assert_eq!(&**actual, "0.7");
        }
        other => panic!(
            "expected UnsupportedFormatVersion, got {}",
            other.stable_code()
        ),
    }

    let shape_traffic = br#"{"formatVersion":"0.10"}"#;
    let manifest = manifest_value(shape_traffic, SPATIAL);
    let issue = single_issue(load_value(&manifest, shape_traffic, SPATIAL));
    assert_eq!(issue.document(), Some(CurrentDocumentRole::Traffic));
    assert_eq!(issue.artifact_ref(), Some(TRAFFIC_REF));
    assert!(matches!(
        issue.payload(),
        CurrentSourceErrorPayload::JsonShape { .. }
    ));
}

#[test]
fn spatial_wire_issues_use_spatial_document_without_context() {
    let invalid_spatial = br#"{"formatVersion":"0.2"}"#;
    let manifest = manifest_value(TRAFFIC, invalid_spatial);
    let issue = single_issue(load_value(&manifest, TRAFFIC, invalid_spatial));
    assert_eq!(issue.document(), Some(CurrentDocumentRole::Spatial));
    assert_eq!(issue.artifact_ref(), None);
    match issue.payload() {
        CurrentSourceErrorPayload::UnsupportedFormatVersion { expected, actual } => {
            assert_eq!(*expected, "0.1");
            assert_eq!(&**actual, "0.2");
        }
        other => panic!(
            "expected UnsupportedFormatVersion, got {}",
            other.stable_code()
        ),
    }
}

#[test]
fn failure_order_is_frozen() {
    // Manifest syntax 最先。
    let issue = single_issue(scenario_error(b"{ not json", &base_artifacts()));
    assert!(matches!(
        issue.payload(),
        CurrentSourceErrorPayload::JsonSyntax { .. }
    ));
    assert_eq!(issue.document(), Some(CurrentDocumentRole::Manifest));

    // 版本先于其他 Manifest shape 与 descriptor。
    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["formatVersion"] = json!("0.2");
    manifest["traffic"]["mediaType"] = json!("application/json");
    let issue = single_issue(load_value(&manifest, TRAFFIC, SPATIAL));
    assert!(matches!(
        issue.payload(),
        CurrentSourceErrorPayload::UnsupportedFormatVersion { .. }
    ));

    // 其他 Manifest shape 先于 descriptor。
    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["future"] = json!(true);
    manifest["traffic"]["mediaType"] = json!("application/json");
    let issue = single_issue(load_value(&manifest, TRAFFIC, SPATIAL));
    assert!(matches!(
        issue.payload(),
        CurrentSourceErrorPayload::JsonShape { .. }
    ));

    // Traffic descriptor 先于 Spatial descriptor。
    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["traffic"]["mediaType"] = json!("application/json");
    manifest["spatial"]["mediaType"] = json!("application/json");
    let issue = single_issue(load_value(&manifest, TRAFFIC, SPATIAL));
    assert_eq!(issue.path(), Some("traffic.mediaType"));

    // Traffic size/digest 先于 Spatial size/digest。
    let mut manifest = manifest_value(TRAFFIC, SPATIAL);
    manifest["traffic"]["size"] = json!(TRAFFIC.len() + 1);
    manifest["spatial"]["size"] = json!(SPATIAL.len() + 1);
    let issue = single_issue(load_value(&manifest, TRAFFIC, SPATIAL));
    match issue.payload() {
        CurrentSourceErrorPayload::ArtifactSizeMismatch { role, .. } => {
            assert_eq!(*role, CurrentArtifactRole::Traffic);
        }
        other => panic!(
            "expected traffic ArtifactSizeMismatch, got {}",
            other.stable_code()
        ),
    }

    // Spatial size/digest 先于 Traffic wire。
    let invalid_traffic = br#"{"formatVersion":"0.7"}"#;
    let mut manifest = manifest_value(invalid_traffic, SPATIAL);
    manifest["spatial"]["size"] = json!(SPATIAL.len() + 1);
    let issue = single_issue(load_value(&manifest, invalid_traffic, SPATIAL));
    match issue.payload() {
        CurrentSourceErrorPayload::ArtifactSizeMismatch { role, .. } => {
            assert_eq!(*role, CurrentArtifactRole::Spatial);
        }
        other => panic!(
            "Spatial size/digest 必须先于 Traffic wire：{}",
            other.stable_code()
        ),
    }

    // Traffic wire 先于 Spatial wire。
    let invalid_spatial = br#"{"formatVersion":"0.2"}"#;
    let manifest = manifest_value(invalid_traffic, invalid_spatial);
    let issue = single_issue(load_value(&manifest, invalid_traffic, invalid_spatial));
    assert_eq!(issue.document(), Some(CurrentDocumentRole::Traffic));
}

#[test]
fn error_bundle_is_never_empty_and_codes_are_stable_and_distinct() {
    let error = validate_traffic_compatible(b"{").expect_err("syntax error");
    assert_eq!(error.issues().len(), 1);
    assert!(!format!("{error}").is_empty());

    let syntax = serde_json::from_slice::<Value>(b"{").expect_err("syntax");
    let shape: serde_json::Error = serde::de::Error::custom("shape");
    let payloads = [
        CurrentSourceErrorPayload::JsonSyntax { source: syntax },
        CurrentSourceErrorPayload::JsonShape { source: shape },
        CurrentSourceErrorPayload::UnsupportedFormatVersion {
            expected: "0.10",
            actual: "0.9".into(),
        },
        CurrentSourceErrorPayload::EmptyArtifactReference,
        CurrentSourceErrorPayload::ConflictingManifestArtifactReference {
            artifact_ref: "a".into(),
        },
        CurrentSourceErrorPayload::DuplicateProvidedArtifactReference {
            artifact_ref: "a".into(),
        },
        CurrentSourceErrorPayload::MissingArtifact {
            role: CurrentArtifactRole::Traffic,
            artifact_ref: "a".into(),
        },
        CurrentSourceErrorPayload::InvalidMediaType {
            expected: TRAFFIC_PACKAGE_MEDIA_TYPE,
            actual: "application/json".into(),
        },
        CurrentSourceErrorPayload::InvalidDigest {
            actual: "sha256:x".into(),
        },
        CurrentSourceErrorPayload::ArtifactSizeOutOfRange { actual: 1, max: 0 },
        CurrentSourceErrorPayload::ArtifactSizeMismatch {
            role: CurrentArtifactRole::Spatial,
            artifact_ref: "a".into(),
            expected: 1,
            actual: 2,
        },
        CurrentSourceErrorPayload::ArtifactDigestMismatch {
            role: CurrentArtifactRole::Spatial,
            artifact_ref: "a".into(),
            expected: "sha256:e".into(),
            actual: "sha256:a".into(),
        },
    ];
    let codes = payloads
        .iter()
        .map(CurrentSourceErrorPayload::stable_code)
        .collect::<Vec<_>>();
    // 与 docs/design/current-package-import.md 的 stable issue code 冻结表逐值一致。
    let expected = [
        "LF-CURRENT-SOURCE-JSON-SYNTAX",
        "LF-CURRENT-SOURCE-JSON-SHAPE",
        "LF-CURRENT-SOURCE-FORMAT-VERSION",
        "LF-CURRENT-SOURCE-EMPTY-ARTIFACT-REF",
        "LF-CURRENT-SOURCE-CONFLICTING-ARTIFACT-REF",
        "LF-CURRENT-SOURCE-DUPLICATE-ARTIFACT-REF",
        "LF-CURRENT-SOURCE-MISSING-ARTIFACT",
        "LF-CURRENT-SOURCE-MEDIA-TYPE",
        "LF-CURRENT-SOURCE-DIGEST",
        "LF-CURRENT-SOURCE-ARTIFACT-SIZE-RANGE",
        "LF-CURRENT-SOURCE-ARTIFACT-SIZE-MISMATCH",
        "LF-CURRENT-SOURCE-ARTIFACT-DIGEST-MISMATCH",
    ];
    assert_eq!(codes, expected, "稳定码必须逐值匹配冻结表");
}

#[test]
fn issue_parts_into_components_is_the_only_owned_bridge() {
    let error = validate_traffic_compatible(b"{").expect_err("syntax error");
    let issues = error.into_issues();
    assert_eq!(issues.len(), 1);
    let issue = issues.into_iter().next().expect("one issue");
    assert_eq!(issue.document(), Some(CurrentDocumentRole::Traffic));
    let (payload, document, context, path, span) = issue.into_parts().into_components();
    assert_eq!(document, Some(CurrentDocumentRole::Traffic));
    assert_eq!(context, CurrentSourceIssueContext::None);
    // 新 parse 层为 syntax 错误产出 serde 位置的单点 span（旧实现经
    // serde_path_to_error 链丢弃位置返回 None，属修正而非回归）。
    let span = span.expect("syntax issue 携带单点 span");
    assert_eq!(span.start().line(), 1);
    assert_eq!(span.start().column(), 1);
    assert_eq!(span.start(), span.end());
    // syntax 错误归位为根 path "$"。
    let path = path.expect("production-compatible issue 必携带 path");
    assert!(!path.is_empty());
    match payload {
        CurrentSourceErrorPayload::JsonSyntax { source } => {
            assert!(source.line() > 0, "owned bridge 交出原始 serde 错误");
        }
        other => panic!("expected JsonSyntax, got {}", other.stable_code()),
    }
}

const TRAFFIC_TEXT: &str =
    include_str!("../../../examples/data/v0.10-empty-signals-and-parking.laneflow.json");

fn base_artifacts() -> Vec<CurrentArtifactInput<'static>> {
    vec![
        CurrentArtifactInput::new(TRAFFIC_REF, TRAFFIC, None),
        CurrentArtifactInput::new(SPATIAL_REF, SPATIAL, None),
    ]
}

fn scenario_error(manifest: &[u8], artifacts: &[CurrentArtifactInput<'_>]) -> CurrentSourceError {
    validate_scenario_compatible(manifest, artifacts).expect_err("scenario must fail")
}

fn load_value(manifest: &Value, traffic: &[u8], spatial: &[u8]) -> CurrentSourceError {
    let source = serde_json::to_vec(manifest).expect("manifest JSON");
    let artifacts = [
        CurrentArtifactInput::new(TRAFFIC_REF, traffic, None),
        CurrentArtifactInput::new(SPATIAL_REF, spatial, None),
    ];
    scenario_error(&source, &artifacts)
}

fn single_issue(error: CurrentSourceError) -> laneflow_current_source::CurrentSourceIssue {
    let issues = error.into_issues();
    assert_eq!(issues.len(), 1, "production 立即失败恒为单元素 bundle");
    issues.into_iter().next().expect("one issue")
}

fn manifest_value(traffic: &[u8], spatial: &[u8]) -> Value {
    json!({
        "formatVersion": CURRENT_SCENARIO_MANIFEST_FORMAT_VERSION,
        "traffic": {
            "artifactRef": TRAFFIC_REF,
            "mediaType": TRAFFIC_PACKAGE_MEDIA_TYPE,
            "digest": digest_text(traffic),
            "size": traffic.len(),
        },
        "spatial": {
            "artifactRef": SPATIAL_REF,
            "mediaType": SPATIAL_PACKAGE_MEDIA_TYPE,
            "digest": digest_text(spatial),
            "size": spatial.len(),
        }
    })
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn digest_text(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::from("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(encoded, "{byte:02x}").expect("write to String");
    }
    encoded
}
