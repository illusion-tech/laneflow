//! 已提交路网来源：#302 活动聚合的来源指名（发布 / 编辑两种来源，#1116 G1）。

use laneflow_static_contract::{ExactByteLength, NetworkRevisionId, Sha256Digest};
use laneflow_static_network::CanonicalNetworkOrigin;
use thiserror::Error;

use crate::facade::editable_source::{EditableDiffBase, RoadEditingState};

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

#[cfg(any(test, feature = "execution-evidence"))]
impl PublishedLfcaReference {
    /// 测试专用：asset key 缓冲实际保留的逻辑字节数。
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        let Self {
            asset_key,
            canonical_artifact_digest: _,
            canonical_artifact_byte_length: _,
            network_revision: _,
        } = self;
        asset_key.capacity() as u64
    }
}

impl PublishedLfcaReference {
    /// 构造已发布引用。asset key 必须非空；三联值由发布链绑定，本类型
    /// 不重算、不验证其真实性。
    ///
    /// # Errors
    ///
    /// asset key 为空时返回 [`InvalidPublishedLfcaReference::EmptyAssetKey`]。
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

    /// 可失败克隆：asset key 按长度可失败预留（快照捕获消费，#532）。
    ///
    /// 语义与 [`Clone`] 一致，仅在分配压力下返回 `Err` 而不中止进程；
    /// 预留注入点与 capture 侧共用同一快照轴计数器。
    ///
    /// # Errors
    ///
    /// asset key 按长度预留失败时返回
    /// [`crate::SnapshotCaptureError::ReservationFailed`]；语义与 [`Clone`] 一致。
    pub fn try_clone(&self) -> Result<Self, crate::admin::snapshot::SnapshotCaptureError> {
        let mut asset_key = String::new();
        if !self.asset_key.is_empty() {
            #[cfg(test)]
            if crate::admin::snapshot::snapshot_reservation_injected_failure() {
                return Err(crate::admin::snapshot::SnapshotCaptureError::ReservationFailed);
            }
            asset_key
                .try_reserve(self.asset_key.len())
                .map_err(|_| crate::admin::snapshot::SnapshotCaptureError::ReservationFailed)?;
        }
        asset_key.push_str(&self.asset_key);
        Ok(Self {
            asset_key,
            canonical_artifact_digest: self.canonical_artifact_digest,
            canonical_artifact_byte_length: self.canonical_artifact_byte_length,
            network_revision: self.network_revision,
        })
    }
}

/// 已提交路网来源（术语表：committed network source）。
///
/// 已进入活动 Runtime 修订的可重建来源，是 #302 原子晋升清单的一员。
/// 编辑来源同变体携带 [`EditableDiffBase`]，使“编辑来源缺基线”与“发布来源带基线”
/// 不可表示（`committed-road-editing-state.md` §3.1）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommittedNetworkSource {
    /// runtime-only 发布世界：不透明 asset key + LFCA
    /// digest / length / revision（术语表：published LFCA reference）。
    Published {
        /// 宿主持久的发布引用。
        reference: PublishedLfcaReference,
    },
    /// 可编辑世界：committed 道路编辑状态与构建当前修订所用的 exact LFCA。
    ///
    /// 快照只绑定 `state` 的摘要、长度与模块数；`diff_base` 不入档。
    Editable {
        /// 与修订共同提交的道路编辑状态。
        state: RoadEditingState,
        /// 与当前修订逐字节对应的差异基线。
        diff_base: EditableDiffBase,
    },
}

#[cfg(any(test, feature = "execution-evidence"))]
impl CommittedNetworkSource {
    /// 测试专用：来源实际保留的逻辑字节数，按变体透传到引用。
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        match self {
            Self::Published { reference } => reference.retained_logical_bytes(),
            Self::Editable { state, diff_base } => {
                state.retained_logical_bytes() + diff_base.retained_logical_bytes()
            }
        }
    }
}

impl CommittedNetworkSource {
    /// 来源指向的路网修订标识；与活动修订的一致性由安装/切换路径验证。
    ///
    /// 编辑来源取差异基线构造时绑定的修订标识。
    #[must_use]
    pub fn network_revision(&self) -> NetworkRevisionId {
        match self {
            Self::Published { reference } => reference.network_revision(),
            Self::Editable { diff_base, .. } => diff_base.network_revision(),
        }
    }

    /// 编辑来源的差异基线是否与 `origin` 逐字节对应；发布来源没有差异基线，恒为真。
    #[must_use]
    pub fn diff_base_corresponds_to(&self, origin: CanonicalNetworkOrigin) -> bool {
        match self {
            Self::Published { .. } => true,
            Self::Editable { diff_base, .. } => diff_base.corresponds_to(origin),
        }
    }

    /// 可失败克隆（快照捕获消费，#532）；变体形状跟随本枚举演进。
    ///
    /// # Errors
    ///
    /// 成员内容按长度预留失败时返回
    /// [`crate::SnapshotCaptureError::ReservationFailed`]；语义与 [`Clone`] 一致。
    pub fn try_clone(&self) -> Result<Self, crate::admin::snapshot::SnapshotCaptureError> {
        match self {
            Self::Published { reference } => Ok(Self::Published {
                reference: reference.try_clone()?,
            }),
            // 编辑来源成员均为引用计数共享，克隆不分配、不复制载荷。
            Self::Editable { state, diff_base } => Ok(Self::Editable {
                state: state.clone(),
                diff_base: diff_base.clone(),
            }),
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
