//! harness 内部完整性摘要（ADR 0033，#1042）。
//!
//! harness 自己产生的摘要只由 harness 重算，唯一算法为 BLAKE3-256；前像以
//! `laneflow:<名称>:v<n>\0` 域分隔前缀开头，前缀版本即派生版本。合同见
//! `docs/design/urban-demand-harness.md` §6「摘要角色与算法」。生成器清单中的
//! 文件摘要不属于本模块（见 [`crate::artifacts::ArtifactFileDigest`]）。

use serde::{Deserialize, Serialize};

/// 摘要角色：每个角色一个域分隔前缀。
#[derive(Clone, Copy, Debug)]
pub(crate) enum Domain {
    /// 文件全部字节。
    File,
    /// 观测块内编码。
    ObservationLeaf,
    /// 按块序的观测叶摘要与尾部编码。
    Observation,
    /// 本拍命令的规范 JSON 数组。
    TickCommands,
    /// 本拍事件的规范 JSON 数组。
    TickEvents,
    /// 宿主耐久状态的规范形式。
    CallerState,
    /// 已应用 Transform 的身份与位姿编码。
    #[cfg_attr(not(feature = "adapter"), allow(dead_code))]
    PresentationApplied,
    /// gate / transitions 逐拍记录的规范 JSON。
    #[cfg_attr(not(feature = "adapter"), allow(dead_code))]
    TrialTick,
    /// gate / transitions 保留状态编码。
    #[cfg_attr(not(feature = "adapter"), allow(dead_code))]
    RetainedState,
    /// 完整性浸泡的逐拍摘要（仅测试，不持久）。
    #[cfg(test)]
    SoakTicks,
}

impl Domain {
    const fn prefix(self) -> &'static [u8] {
        match self {
            Self::File => b"laneflow:urban-harness-file:v1\0",
            Self::ObservationLeaf => b"laneflow:urban-observation-leaf:v4\0",
            Self::Observation => b"laneflow:urban-observation:v4\0",
            Self::TickCommands => b"laneflow:urban-tick-commands:v1\0",
            Self::TickEvents => b"laneflow:urban-tick-events:v1\0",
            Self::CallerState => b"laneflow:urban-caller-digest:v2\0",
            Self::PresentationApplied => b"laneflow:urban-presentation-applied:v2\0",
            Self::TrialTick => b"laneflow:urban-trial-tick:v1\0",
            Self::RetainedState => b"laneflow:urban-retained-state:v1\0",
            #[cfg(test)]
            Self::SoakTicks => b"laneflow:urban-integrity-soak-ticks:v1\0",
        }
    }

    /// 已写入域分隔前缀的增量哈希。
    pub(crate) fn hasher(self) -> blake3::Hasher {
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.prefix());
        hasher
    }

    /// 一次性摘要的 32 字节值。
    pub(crate) fn bytes(self, preimage: &[u8]) -> [u8; 32] {
        *self.hasher().update(preimage).finalize().as_bytes()
    }

    /// 一次性摘要的小写十六进制形式。
    pub(crate) fn hex(self, preimage: &[u8]) -> String {
        crate::hex(&self.bytes(preimage))
    }
}

/// harness 产出文件的字节数与文件字节摘要。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileDigest {
    pub bytes: u64,
    pub digest: String,
}

impl FileDigest {
    pub(crate) fn of(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.len() as u64,
            digest: Domain::File.hex(bytes),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个前缀的 known vector：以 `blake3::hash(前缀 ‖ 前像)` 独立计算核对，
    /// 前缀字面值同时固定在本测试中，改动前缀即改动派生版本。
    #[test]
    fn domain_prefixes_are_fixed_and_digests_match_independent_blake3() {
        let cases: [(Domain, &[u8]); 10] = [
            (Domain::File, b"laneflow:urban-harness-file:v1\0"),
            (
                Domain::ObservationLeaf,
                b"laneflow:urban-observation-leaf:v4\0",
            ),
            (Domain::Observation, b"laneflow:urban-observation:v4\0"),
            (Domain::TickCommands, b"laneflow:urban-tick-commands:v1\0"),
            (Domain::TickEvents, b"laneflow:urban-tick-events:v1\0"),
            (Domain::CallerState, b"laneflow:urban-caller-digest:v2\0"),
            (
                Domain::PresentationApplied,
                b"laneflow:urban-presentation-applied:v2\0",
            ),
            (Domain::TrialTick, b"laneflow:urban-trial-tick:v1\0"),
            (Domain::RetainedState, b"laneflow:urban-retained-state:v1\0"),
            (
                Domain::SoakTicks,
                b"laneflow:urban-integrity-soak-ticks:v1\0",
            ),
        ];
        let preimage = b"laneflow urban-harness known vector";
        let mut seen = std::collections::BTreeSet::new();
        for (domain, prefix) in cases {
            assert_eq!(domain.prefix(), prefix);
            let mut independent = prefix.to_vec();
            independent.extend_from_slice(preimage);
            let expected = blake3::hash(&independent);
            assert_eq!(domain.bytes(preimage), *expected.as_bytes(), "{domain:?}");
            assert_eq!(domain.hex(preimage), expected.to_hex().as_str());
            assert!(seen.insert(domain.hex(preimage)), "{domain:?} collides");
        }
    }

    #[test]
    fn file_digest_known_vector() {
        // BLAKE3-256("laneflow:urban-harness-file:v1\0" ‖ "abc")
        let digest = FileDigest::of(b"abc");
        assert_eq!(digest.bytes, 3);
        let mut independent = b"laneflow:urban-harness-file:v1\0".to_vec();
        independent.extend_from_slice(b"abc");
        assert_eq!(digest.digest, blake3::hash(&independent).to_hex().as_str());
        assert_ne!(digest.digest, crate::hex(blake3::hash(b"abc").as_bytes()));
        // 固定字面值：任何改动前缀或算法的变更都会在此显形，须同步递增派生版本。
        assert_eq!(
            digest.digest,
            "8df0d1f938ee2ebd8a954a3bd832ea10d1a7b8288f9ea43fdcbfef37c7e38607"
        );
    }
}
