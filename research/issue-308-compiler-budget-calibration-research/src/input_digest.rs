//! 失败资格输入的规范 SHA-256 摘要。
//!
//! 研究输入由受信任工作负载清单、完整规模身份、规范计数和输入变体唯一确定；摘要再
//! 绑定值来源、基准运行与完整私有限制集合。显式长度前缀和固定字段顺序避免拼接歧义，
//! 同时避免为了证据封套再次物化城市级输入字节。

use crate::{GraphProfileId, IdentityAggregateCounts, LimitDimensionId, ScalableWorkloadId};
use sha2::{Digest, Sha256};

const FAILURE_INPUT_DIGEST_DOMAIN: &[u8] = b"LANEFLOW-COMPILER-CALIBRATION-INPUT-V1\0";
const CANONICAL_SOURCE_BINDING_RULE: &str =
    "trusted-manifest+full-scale-identity+canonical-counts+input-variant-v1";

#[derive(Clone, Copy, Debug)]
pub struct FailureInputDigestBinding<'a> {
    pub workload_manifest_sha256: &'a str,
    pub workload_id: ScalableWorkloadId,
    pub workload_revision: u32,
    pub graph_profile: GraphProfileId,
    pub string_profile: &'a str,
    pub generator_version: u32,
    pub n: u32,
    pub b: u32,
    pub scale_role: &'a str,
    pub case_id: &'a str,
    pub input_variant_id: &'a str,
    pub counts: &'a IdentityAggregateCounts,
    pub value_basis: &'a str,
    pub basis_run_ids: &'a [String],
}

pub fn failure_input_digest_sha256(
    binding: &FailureInputDigestBinding<'_>,
    parameters: &[(&str, u64)],
) -> String {
    let mut parameters = parameters.to_vec();
    parameters.sort_unstable_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    for pair in parameters.windows(2) {
        assert_ne!(pair[0].0, pair[1].0, "失败输入摘要参数名不得重复");
    }
    let mut basis_run_ids = binding.basis_run_ids.to_vec();
    basis_run_ids.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    for pair in basis_run_ids.windows(2) {
        assert_ne!(pair[0], pair[1], "失败输入摘要基准运行 ID 不得重复");
    }

    let mut hasher = Sha256::new();
    hasher.update(FAILURE_INPUT_DIGEST_DOMAIN);
    update_string(&mut hasher, binding.workload_manifest_sha256);
    update_string(&mut hasher, binding.workload_id.as_str());
    hasher.update(u64::from(binding.workload_revision).to_le_bytes());
    update_string(&mut hasher, binding.graph_profile.as_str());
    update_string(&mut hasher, binding.string_profile);
    hasher.update(u64::from(binding.generator_version).to_le_bytes());
    hasher.update(u64::from(binding.n).to_le_bytes());
    hasher.update(u64::from(binding.b).to_le_bytes());
    update_string(&mut hasher, binding.scale_role);
    update_string(&mut hasher, binding.case_id);
    update_string(&mut hasher, binding.input_variant_id);
    update_string(&mut hasher, CANONICAL_SOURCE_BINDING_RULE);
    update_counts(&mut hasher, binding.counts);
    update_string(&mut hasher, binding.value_basis);
    hasher.update(
        u32::try_from(basis_run_ids.len())
            .expect("失败输入摘要基准运行数量适合 u32")
            .to_le_bytes(),
    );
    for run_id in basis_run_ids {
        update_string(&mut hasher, &run_id);
    }
    hasher.update(
        u32::try_from(parameters.len())
            .expect("失败输入摘要参数数量适合 u32")
            .to_le_bytes(),
    );
    for (name, value) in parameters {
        update_string(&mut hasher, name);
        hasher.update(value.to_le_bytes());
    }
    lower_hex(hasher.finalize().as_slice())
}

pub(crate) fn failure_input_digest_with_private_limits(
    binding: &FailureInputDigestBinding<'_>,
    selected_limit: Option<(LimitDimensionId, u64)>,
    additional_parameters: &[(&str, u64)],
) -> String {
    let parameters = complete_private_limit_parameters(selected_limit, additional_parameters);
    let borrowed = parameters
        .iter()
        .map(|(name, value)| (name.as_str(), *value))
        .collect::<Vec<_>>();
    failure_input_digest_sha256(binding, &borrowed)
}

fn complete_private_limit_parameters(
    selected_limit: Option<(LimitDimensionId, u64)>,
    additional_parameters: &[(&str, u64)],
) -> Vec<(String, u64)> {
    let mut parameters = Vec::with_capacity(
        LimitDimensionId::ALL
            .len()
            .checked_add(additional_parameters.len())
            .expect("失败输入摘要参数数量不会溢出"),
    );
    for dimension in LimitDimensionId::ALL {
        let value = selected_limit
            .filter(|(selected, _)| *selected == dimension)
            .map_or(u64::MAX, |(_, value)| value);
        parameters.push((format!("private-limit/{}", dimension.as_str()), value));
    }
    for (name, value) in additional_parameters {
        assert!(
            !name.starts_with("private-limit/"),
            "额外失败输入参数不得覆盖完整私有限制集合"
        );
        parameters.push(((*name).to_owned(), *value));
    }
    parameters
}

fn update_counts(hasher: &mut Sha256, counts: &IdentityAggregateCounts) {
    let fields = [
        ("module-count", counts.module_count),
        ("import-edge-count", counts.import_edge_count),
        (
            "cross-module-reference-count",
            counts.cross_module_reference_count,
        ),
        ("maximum-import-depth", counts.maximum_import_depth),
        ("source-document-count", counts.source_document_count),
        ("source-byte-count", counts.source_byte_count),
        (
            "identity-declaration-count",
            counts.identity_declaration_count,
        ),
        ("source-declaration-count", counts.source_declaration_count),
        ("source-span-count", counts.source_span_count),
        (
            "identity-field-occurrence-count",
            counts.identity_field_occurrence_count,
        ),
        (
            "profiled-key-occurrence-count",
            counts.profiled_key_occurrence_count,
        ),
        ("source-reference-count", counts.source_reference_count),
        ("source-relation-count", counts.source_relation_count),
        ("source-geometry-count", counts.source_geometry_count),
        ("symbol-count", counts.symbol_count),
        ("string-item-count", counts.string_item_count),
        ("maximum-string-bytes", counts.maximum_string_bytes),
        ("total-string-bytes", counts.total_string_bytes),
        ("diagnostic-count", counts.diagnostic_count),
        ("semantic-output-record", counts.semantic_output_record),
        (
            "semantic-payload-byte-count",
            counts.semantic_payload_byte_count,
        ),
        ("logical-byte-count", counts.logical_byte_count),
        ("output-byte-count", counts.output_byte_count),
    ];
    hasher.update(
        u32::try_from(fields.len())
            .expect("规范计数字段数量适合 u32")
            .to_le_bytes(),
    );
    for (name, value) in fields {
        update_string(hasher, name);
        hasher.update(value.to_le_bytes());
    }
}

fn update_string(hasher: &mut Sha256, value: &str) {
    hasher.update(
        u32::try_from(value.len())
            .expect("失败输入摘要字段长度适合 u32")
            .to_le_bytes(),
    );
    hasher.update(value.as_bytes());
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts() -> IdentityAggregateCounts {
        IdentityAggregateCounts {
            module_count: 2,
            import_edge_count: 1,
            cross_module_reference_count: 3,
            maximum_import_depth: 1,
            source_document_count: 2,
            source_byte_count: 100,
            identity_declaration_count: 4,
            source_declaration_count: 4,
            source_span_count: 8,
            identity_field_occurrence_count: 9,
            profiled_key_occurrence_count: 4,
            source_reference_count: 3,
            source_relation_count: 2,
            source_geometry_count: 1,
            symbol_count: 4,
            string_item_count: 20,
            maximum_string_bytes: 32,
            total_string_bytes: 200,
            diagnostic_count: 0,
            semantic_output_record: 7,
            semantic_payload_byte_count: 64,
            logical_byte_count: 512,
            output_byte_count: 160,
        }
    }

    fn binding<'a>(counts: &'a IdentityAggregateCounts) -> FailureInputDigestBinding<'a> {
        FailureInputDigestBinding {
            workload_manifest_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            workload_id: ScalableWorkloadId::Identity,
            workload_revision: 1,
            graph_profile: GraphProfileId::WideStar,
            string_profile: "short-unique-v1",
            generator_version: 1,
            n: 16,
            b: 8,
            scale_role: "calibration",
            case_id: "limit/source-byte-count/plus-one",
            input_variant_id: "canonical-valid-v1",
            counts,
            value_basis: "canonical-level-exact-value",
            basis_run_ids: &[],
        }
    }

    #[test]
    fn parameter_order_does_not_change_failure_input_digest() {
        let counts = counts();
        let binding = binding(&counts);
        let left = failure_input_digest_sha256(
            &binding,
            &[("selected-limit-value", 99), ("exact-dimension-value", 100)],
        );
        let right = failure_input_digest_sha256(
            &binding,
            &[("exact-dimension-value", 100), ("selected-limit-value", 99)],
        );
        assert_eq!(left, right);
        assert_eq!(left.len(), 64);
    }

    #[test]
    fn changing_a_bound_identity_field_changes_the_digest() {
        let counts = counts();
        let base = binding(&counts);
        let mut changed = base;
        changed.b += 1;
        assert_ne!(
            failure_input_digest_sha256(&base, &[]),
            failure_input_digest_sha256(&changed, &[])
        );
    }

    #[test]
    fn complete_private_limit_parameters_bind_every_dimension_once() {
        let parameters = complete_private_limit_parameters(
            Some((LimitDimensionId::SourceByteCount, 99)),
            &[("exact-dimension-value", 100), ("selected-limit-value", 99)],
        );
        assert_eq!(parameters.len(), LimitDimensionId::ALL.len() + 2);
        for dimension in LimitDimensionId::ALL {
            let name = format!("private-limit/{}", dimension.as_str());
            let matches = parameters
                .iter()
                .filter(|(candidate, _)| candidate == &name)
                .collect::<Vec<_>>();
            assert_eq!(matches.len(), 1, "{name} 必须恰好出现一次");
            let expected = if dimension == LimitDimensionId::SourceByteCount {
                99
            } else {
                u64::MAX
            };
            assert_eq!(matches[0].1, expected);
        }
    }
}
