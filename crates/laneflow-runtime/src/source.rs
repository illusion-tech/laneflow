//! 已提交路网来源：#302 活动聚合的来源指名（切片 A 第一步）。

use laneflow_static_contract::{ExactByteLength, NetworkRevisionId, Sha256Digest};
use thiserror::Error;

/// 已发布 LFCA 引用（术语表：published LFCA reference）构造失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum InvalidPublishedLfcaReference {
    /// 宿主 asset key 为空字符串。
    #[error("已发布 LFCA 引用的 asset key 不能为空")]
    EmptyAssetKey,
}

/// 已发布 LFCA 引用（术语表：published LFCA reference）。
///
/// runtime-only 存档来源的宿主持久指名：不透明 asset key 与 LFCA
/// digest / byte length / `NetworkRevisionId` 三联绑定。不复制 LFCA，
/// 也不替代加载时的资产认证；digest / length 只承担来源审计，同修订
/// 判据只要求 `NetworkRevisionId` 与契约版本精确相等（#302 快照合同）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedLfcaReference {
    asset_key: String,
    canonical_artifact_digest: Sha256Digest,
    canonical_artifact_byte_length: ExactByteLength,
    network_revision: NetworkRevisionId,
}

impl PublishedLfcaReference {
    /// 构造已发布引用。asset key 必须非空；三联值由发布链绑定，本类型
    /// 不重算、不验证其真实性。
    pub fn new(
        asset_key: impl Into<String>,
        canonical_artifact_digest: Sha256Digest,
        canonical_artifact_byte_length: ExactByteLength,
        network_revision: NetworkRevisionId,
    ) -> Result<Self, InvalidPublishedLfcaReference> {
        let asset_key = asset_key.into();
        if asset_key.is_empty() {
            return Err(InvalidPublishedLfcaReference::EmptyAssetKey);
        }
        Ok(Self {
            asset_key,
            canonical_artifact_digest,
            canonical_artifact_byte_length,
            network_revision,
        })
    }

    /// 不透明宿主 asset key；语义由宿主资产系统拥有。
    #[must_use]
    pub fn asset_key(&self) -> &str {
        &self.asset_key
    }

    /// 引用 LFCA 的 exact-byte 摘要（来源审计，不参与同修订判据）。
    #[must_use]
    pub const fn canonical_artifact_digest(&self) -> Sha256Digest {
        self.canonical_artifact_digest
    }

    /// 引用 LFCA 的 exact-byte 长度（来源审计，不参与同修订判据）。
    #[must_use]
    pub const fn canonical_artifact_byte_length(&self) -> ExactByteLength {
        self.canonical_artifact_byte_length
    }

    /// 引用 LFCA 绑定的路网修订标识（同修订判据的比对对象）。
    #[must_use]
    pub const fn network_revision(&self) -> NetworkRevisionId {
        self.network_revision
    }
}

/// 已提交路网来源（术语表：committed network source）。
///
/// 已进入活动 Runtime 修订的可重建来源，是 #302 原子晋升清单的一员。
/// v1 只交付发布形态；可编辑形态的载荷是 committed `RoadEditingState`，
/// 该领域类型尚未存在于生产依赖面内，随 editable session 对接切片落位
/// 并回写切换/快照合同——不为其预留空壳变体。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommittedNetworkSource {
    /// runtime-only 发布世界：不透明 asset key + LFCA
    /// digest / length / revision（术语表：published LFCA reference）。
    Published {
        /// 宿主持久的发布引用。
        reference: PublishedLfcaReference,
    },
}

impl CommittedNetworkSource {
    /// 来源指向的路网修订标识；与活动根的一致性由安装/切换路径验证。
    #[must_use]
    pub const fn network_revision(&self) -> NetworkRevisionId {
        match self {
            Self::Published { reference } => reference.network_revision(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(asset_key: &str) -> Result<PublishedLfcaReference, InvalidPublishedLfcaReference> {
        let digest = Sha256Digest::from_bytes([7; 32]);
        PublishedLfcaReference::new(
            asset_key,
            digest,
            ExactByteLength::new(4_096),
            NetworkRevisionId::from_digest(digest),
        )
    }

    #[test]
    fn published_reference_roundtrip_and_rejects_empty_key() {
        let digest = Sha256Digest::from_bytes([7; 32]);
        let value = reference("asset://city/road-v3").expect("non-empty key");
        assert_eq!(value.asset_key(), "asset://city/road-v3");
        assert_eq!(value.canonical_artifact_digest(), digest);
        assert_eq!(value.canonical_artifact_byte_length().get(), 4_096);
        assert_eq!(
            value.network_revision(),
            NetworkRevisionId::from_digest(digest)
        );
        assert_eq!(
            reference(""),
            Err(InvalidPublishedLfcaReference::EmptyAssetKey)
        );
    }

    #[test]
    fn committed_source_exposes_revision() {
        let digest = Sha256Digest::from_bytes([3; 32]);
        let source = CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "asset://a",
                digest,
                ExactByteLength::new(1),
                NetworkRevisionId::from_digest(digest),
            )
            .expect("non-empty key"),
        };
        assert_eq!(
            source.network_revision(),
            NetworkRevisionId::from_digest(digest)
        );
    }
}
