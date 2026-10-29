use std::{hint::black_box, sync::OnceLock};

use laneflow_format::{
    check_canonical_network_input, check_post_emission_bundle, preflight_object_values,
    without_digest_probe, ExpectedSemanticDiffBase, FormatLimitConfig, FormatLimits,
};
use laneflow_static_contract::PortableObjectKind;
use sha2::{Digest, Sha256};

use crate::{mutate, MAX_INPUT_BYTES};

pub(crate) const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../crates/laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
);
const BUNDLES: [[&[u8]; 3]; 2] = [
    [
        FULL_SPATIAL,
        include_bytes!("../../crates/laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfsm"),
        include_bytes!("../../crates/laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfsd"),
    ],
    [
        include_bytes!("../../crates/laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"),
        include_bytes!("../../crates/laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfsm"),
        include_bytes!("../../crates/laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfsd"),
    ],
];

pub(crate) fn limits() -> FormatLimits {
    FormatLimits::try_new(FormatLimitConfig {
        max_object_bytes: MAX_INPUT_BYTES as u64,
        ..FormatLimitConfig::HARD
    })
    .expect("bounded format limits")
}

fn publication() -> &'static [u8] {
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    BYTES.get_or_init(|| {
        let text = include_str!("../../crates/laneflow-compiler/tests/fixtures/portable/lfcp-min-bindings/expected.lfcp.hex");
        let hex: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(hex.len() % 2, 0, "complete hex pairs");
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("fixture hex"))
            .collect()
    })
}

fn validate_seeds() {
    static CHECKED: OnceLock<()> = OnceLock::new();
    CHECKED.get_or_init(|| {
        for [lfca, lfsm, lfsd] in BUNDLES {
            check_post_emission_bundle(
                lfca,
                lfsm,
                lfsd,
                ExpectedSemanticDiffBase::Genesis,
                limits(),
            )
            .expect("unmutated bundle must admit");
        }
        preflight_object_values(
            publication(),
            PortableObjectKind::CanonicalPublicationDescriptor,
            limits(),
        )
        .expect("unmutated publication must admit");
    });
}

/// 探索四种公开容器的 framing、registry、值域与 LFCA 修订绑定。
///
/// # Panics
///
/// 生产受检视图不能安全消费其已证明字段时触发 oracle。
pub fn check_portable_bytes(bytes: &[u8]) {
    if bytes.len() > MAX_INPUT_BYTES {
        return;
    }
    // 损坏摘要是预期输入；避免生产诊断探针为每个 mutation 写磁盘转储。
    without_digest_probe(|| {
        for kind in PortableObjectKind::ALL {
            if let Ok(view) = preflight_object_values(bytes, kind, limits()) {
                for section in view.registry_view().sections() {
                    for table in section.tables() {
                        for row in table.rows() {
                            for field in row.fields() {
                                black_box(field.value().expect("checked field value"));
                            }
                        }
                    }
                }
            }
        }
        black_box(check_canonical_network_input(bytes, limits())).ok();
    });
}

/// 合法 bundle 定点变异；可重算 chunk 摘要，使变异继续到字段与跨对象绑定检查。
///
/// # Panics
///
/// 合法种子失效或生产读取器违反受检视图约束时触发 oracle。
pub fn check_portable_mutation(data: &[u8]) {
    if data.len() > MAX_INPUT_BYTES {
        return;
    }
    validate_seeds();
    let selector = data.first().copied().unwrap_or(0);
    let object = usize::from(selector & 3);
    let bundle = BUNDLES[usize::from((selector >> 2) & 1)];
    let mode = data.get(1).copied().unwrap_or(0);
    let seed = if object == 3 {
        publication()
    } else {
        bundle[object]
    };
    let mut changed = mutate(seed, data.get(2..).unwrap_or_default());
    if mode & 1 != 0 && object != 3 {
        repair_seed_chunk_digests(seed, &mut changed);
    }
    check_portable_bytes(&changed);
    if object < 3 {
        let mut inputs = bundle;
        inputs[object] = &changed;
        // 独立变化基线绑定，不替调用方把不匹配自动修正。
        let expected = if mode & 2 == 0 {
            ExpectedSemanticDiffBase::Genesis
        } else {
            crate::cutover::expected_base()
        };
        without_digest_probe(|| {
            black_box(check_post_emission_bundle(
                inputs[0],
                inputs[1],
                inputs[2],
                expected,
                limits(),
            ))
            .ok();
        });
    }
}

// 仅解释已由 validate_seeds 验证的种子布局，不从攻击者改过的 offset/count 取范围。
// 算法保持 portable-canonical-artifact §2.2 的 SHA-256 字节合同；不生成受检能力。
pub(crate) fn repair_seed_chunk_digests(seed: &[u8], bytes: &mut [u8]) {
    if seed.len() != bytes.len() {
        return;
    }
    let u32_at = |at| u32::from_le_bytes(seed[at..at + 4].try_into().expect("seed u32"));
    let u64_at = |at| {
        usize::try_from(u64::from_le_bytes(
            seed[at..at + 8].try_into().expect("seed u64"),
        ))
        .expect("seed address")
    };
    for section in 0..u32_at(12) as usize {
        let start = u64_at(32 + section * 24 + 8);
        for chunk in 0..u32_at(start) as usize {
            let entry = start + 16 + chunk * 72;
            let offset = start + u64_at(entry + 24);
            let end = offset + u64_at(entry + 32);
            let digest = Sha256::digest(&bytes[offset..end]);
            bytes[entry + 40..entry + 72].copy_from_slice(&digest);
        }
    }
}

pub(crate) fn seeds() -> Vec<(&'static str, &'static str, Vec<u8>)> {
    validate_seeds();
    let mut seeds = vec![
        ("portable_arbitrary_bytes", "lfca", BUNDLES[0][0].to_vec()),
        ("portable_arbitrary_bytes", "lfsm", BUNDLES[0][1].to_vec()),
        ("portable_arbitrary_bytes", "lfsd", BUNDLES[0][2].to_vec()),
        ("portable_arbitrary_bytes", "lfcp", publication().to_vec()),
    ];
    for (name, selector) in [("lfca", 0), ("lfsm", 1), ("lfsd", 2), ("lfcp", 3)] {
        seeds.push(("portable_bundle_mutation", name, vec![selector, 1]));
    }
    seeds.push(("portable_bundle_mutation", "other-bundle", vec![4, 1]));
    seeds
}

#[cfg(test)]
mod tests {
    use super::*;
    use laneflow_format::preflight_object_registry;

    #[test]
    fn repaired_mutation_reaches_value_validation() {
        validate_seeds();
        let mut bytes = FULL_SPATIAL.to_vec();
        let section = u64::from_le_bytes(bytes[40..48].try_into().unwrap()) as usize;
        let chunk = section
            + u64::from_le_bytes(bytes[section + 40..section + 48].try_into().unwrap()) as usize;
        // 首个 Identity 字段的版本值；保留结构，破坏值域。
        bytes[chunk + 16 + 16 + 12] = 0;
        repair_seed_chunk_digests(FULL_SPATIAL, &mut bytes);
        preflight_object_registry(&bytes, PortableObjectKind::CanonicalArtifact, limits())
            .expect("mutation crossed chunk digest and registry gates");
        assert!(
            preflight_object_values(&bytes, PortableObjectKind::CanonicalArtifact, limits())
                .is_err()
        );
    }
}
