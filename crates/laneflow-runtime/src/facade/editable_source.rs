//! 已提交道路编辑状态与编辑来源差异基线（#1116 G1，`committed-road-editing-state.md`）。

use std::sync::Arc;

use laneflow_static_contract::{ExactByteLength, NetworkRevisionId, Sha256Digest};
use laneflow_static_network::CanonicalNetworkOrigin;
use sha2::Digest as _;
use thiserror::Error;

use crate::kernel::bytes32::bytes32_type;

/// 道路编辑状态摘要的派生版本；更换算法或前像即递增（设计 §2.4）。
pub const ROAD_EDITING_STATE_DIGEST_VERSION: u16 = 1;
/// 道路编辑状态摘要的域分隔前缀；尾随 NUL 属于前缀字节。
pub const ROAD_EDITING_STATE_DIGEST_DOMAIN: &[u8] = b"laneflow:road-editing-state-digest:v1\0";

bytes32_type! {
    /// 32-byte 道路编辑状态摘要（BLAKE3-256）。
    ///
    /// ADR 0033 内部完整性摘要：只由 LaneFlow 实现产生与比对，认证宿主交回的
    /// [`RoadEditingState`] 与快照绑定的是同一份字节；不进入 `NetworkRevisionId`，
    /// 也不替代 LFSM 的 `sourceCollectionDigest`。
    pub struct RoadEditingStateDigest;
}

/// committed [`RoadEditingState`] 构造失败；失败不产生部分对象。
#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum InvalidRoadEditingState {
    /// 模块集合为空。
    #[error("道路编辑状态至少包含一个模块")]
    Empty,
    /// 模块数超出 `u32` 摘要前像字段。
    #[error("道路编辑状态模块数 {count} 超出 u32")]
    TooManyModules {
        /// 实际模块数。
        count: usize,
    },
    /// 某模块的来源文档键为空（按调用方输入顺序编号）。
    #[error("第 {index} 个模块的来源文档键为空")]
    EmptySourceDocumentKey {
        /// 调用方输入顺序中的模块序号。
        index: usize,
    },
    /// 某模块的来源文档键字节数超出 `u32` 摘要前像字段。
    #[error("第 {index} 个模块的来源文档键超出 u32 字节长度")]
    SourceDocumentKeyTooLong {
        /// 调用方输入顺序中的模块序号。
        index: usize,
    },
    /// 某模块的来源字节为空（按调用方输入顺序编号）。
    #[error("第 {index} 个模块的来源字节为空")]
    EmptySourceBytes {
        /// 调用方输入顺序中的模块序号。
        index: usize,
    },
    /// 两个模块的来源文档键相同。
    #[error("来源文档键 `{key}` 重复")]
    DuplicateSourceDocumentKey {
        /// 重复的来源文档键。
        key: String,
    },
    /// 全部来源字节长度之和超出 `u64`。
    #[error("道路编辑状态总字节长度超出 u64")]
    TotalByteLengthOverflow,
}

/// 道路编辑状态中的一个模块：来源文档键 + 完整的 size-prefixed `LFRE` buffer。
///
/// 本类型不读取、不校验 `LFRE` 字节；framing、verifier、版本与语义检查只由编译器
/// 准入负责（设计 §2.3）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoadEditingStateModule {
    source_document_key: String,
    source_bytes: Vec<u8>,
}

impl RoadEditingStateModule {
    /// 接管来源文档键与 `LFRE` 字节；不复制、不校验。
    #[must_use]
    pub fn new(source_document_key: String, source_bytes: Vec<u8>) -> Self {
        Self {
            source_document_key,
            source_bytes,
        }
    }

    /// 来源文档键；与编译器准入时宿主声明的 expected source document key 为同一值。
    #[must_use]
    pub fn source_document_key(&self) -> &str {
        &self.source_document_key
    }

    /// 完整的 size-prefixed `LFRE` 字节。
    #[must_use]
    pub fn source_bytes(&self) -> &[u8] {
        &self.source_bytes
    }
}

#[derive(Debug, Eq, PartialEq)]
struct RoadEditingStateInner {
    modules: Vec<RoadEditingStateModule>,
    digest: RoadEditingStateDigest,
    total_byte_length: ExactByteLength,
}

/// committed 道路编辑状态：与 Runtime 修订共同提交的 `LFRE` 模块规范有序集合。
///
/// Runtime 只做结构校验并计算 [`RoadEditingStateDigest`]，不解码 `LFRE`、不重编译；
/// 状态与共享路网修订的语义对应由完成编译的宿主保证（设计 §4）。构造后不可变，
/// 克隆只增加引用计数、不复制载荷。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoadEditingState {
    inner: Arc<RoadEditingStateInner>,
}

impl RoadEditingState {
    /// 校验结构、按来源文档键 UTF-8 字节序排为规范顺序并计算一次摘要。
    ///
    /// 载荷由调用方移交所有权，构造不复制来源字节；只有引用计数头部是定长分配。
    ///
    /// # Errors
    ///
    /// 集合为空、模块数或键长超出摘要前像字段、键或字节为空、键重复，或总字节长度
    /// 溢出时返回 [`InvalidRoadEditingState`]。
    pub fn try_new(
        mut modules: Vec<RoadEditingStateModule>,
    ) -> Result<Self, InvalidRoadEditingState> {
        if modules.is_empty() {
            return Err(InvalidRoadEditingState::Empty);
        }
        if u32::try_from(modules.len()).is_err() {
            return Err(InvalidRoadEditingState::TooManyModules {
                count: modules.len(),
            });
        }
        let mut total: u64 = 0;
        for (index, module) in modules.iter().enumerate() {
            if module.source_document_key.is_empty() {
                return Err(InvalidRoadEditingState::EmptySourceDocumentKey { index });
            }
            if u32::try_from(module.source_document_key.len()).is_err() {
                return Err(InvalidRoadEditingState::SourceDocumentKeyTooLong { index });
            }
            if module.source_bytes.is_empty() {
                return Err(InvalidRoadEditingState::EmptySourceBytes { index });
            }
            total = u64::try_from(module.source_bytes.len())
                .ok()
                .and_then(|length| total.checked_add(length))
                .ok_or(InvalidRoadEditingState::TotalByteLengthOverflow)?;
        }
        // 键唯一时 unstable 排序即为规范顺序，且不分配。
        modules.sort_unstable_by(|left, right| {
            left.source_document_key
                .as_bytes()
                .cmp(right.source_document_key.as_bytes())
        });
        if let Some(pair) = modules
            .windows(2)
            .find(|pair| pair[0].source_document_key == pair[1].source_document_key)
        {
            return Err(InvalidRoadEditingState::DuplicateSourceDocumentKey {
                key: pair[0].source_document_key.clone(),
            });
        }
        let digest = road_editing_state_digest(&modules);
        Ok(Self {
            inner: Arc::new(RoadEditingStateInner {
                modules,
                digest,
                total_byte_length: ExactByteLength::new(total),
            }),
        })
    }

    /// 规范顺序的模块切片。
    #[must_use]
    pub fn modules(&self) -> &[RoadEditingStateModule] {
        &self.inner.modules
    }

    /// 构造时计算的道路编辑状态摘要。
    #[must_use]
    pub fn digest(&self) -> RoadEditingStateDigest {
        self.inner.digest
    }

    /// 全部来源字节长度之和。
    #[must_use]
    pub fn total_byte_length(&self) -> ExactByteLength {
        self.inner.total_byte_length
    }

    /// 模块数；构造已保证不超过 `u32`。
    #[must_use]
    pub fn module_count(&self) -> u32 {
        u32::try_from(self.inner.modules.len()).unwrap_or(u32::MAX)
    }

    /// 测试专用：载荷实际保留的逻辑字节数（键与来源字节的容量）。
    #[cfg(any(test, feature = "execution-evidence"))]
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        self.inner
            .modules
            .iter()
            .map(|module| {
                (module.source_document_key.capacity() + module.source_bytes.capacity()) as u64
            })
            .sum()
    }
}

/// 按设计 §2.4 前像计算道路编辑状态摘要；`modules` 必须已是规范顺序。
fn road_editing_state_digest(modules: &[RoadEditingStateModule]) -> RoadEditingStateDigest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ROAD_EDITING_STATE_DIGEST_DOMAIN);
    hasher.update(&ROAD_EDITING_STATE_DIGEST_VERSION.to_le_bytes());
    hasher.update(
        &u32::try_from(modules.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    for module in modules {
        let key = module.source_document_key.as_bytes();
        hasher.update(&u32::try_from(key.len()).unwrap_or(u32::MAX).to_le_bytes());
        hasher.update(key);
        hasher.update(&(module.source_bytes.len() as u64).to_le_bytes());
        hasher.update(&module.source_bytes);
    }
    RoadEditingStateDigest::from_bytes(*hasher.finalize().as_bytes())
}

/// 差异基线字节与给定修订 origin 不对应（设计 §3.2）。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[error("差异基线 LFCA 字节的 digest / length 与修订 origin 不一致")]
pub struct EditableDiffBaseMismatch;

#[derive(Debug, Eq, PartialEq)]
struct EditableDiffBaseInner {
    lfca_bytes: Vec<u8>,
    canonical_artifact_digest: Sha256Digest,
    canonical_artifact_byte_length: ExactByteLength,
    network_revision: NetworkRevisionId,
}

/// 可编辑差异基线：构建当前共享路网修订所用的 exact LFCA 字节。
///
/// 下一次 LFSD 发射时 `PortableDiffBase::Artifact` 的唯一合法输入。构造后不可变，
/// 克隆只增加引用计数；不进入快照，恢复时由重编译重新取得。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditableDiffBase {
    inner: Arc<EditableDiffBaseInner>,
}

impl EditableDiffBase {
    /// 接管 LFCA 字节，计算一次 SHA-256，并核对与 `origin` 逐字节对应。
    ///
    /// `origin` 是宿主由这份字节构建出的共享路网修订的来源绑定；记录其修订标识。
    /// 各入口仍会以目标修订 origin 再核对一次（设计 §5）。
    ///
    /// # Errors
    ///
    /// 字节的 SHA-256 或精确长度与 `origin` 不一致时返回 [`EditableDiffBaseMismatch`]。
    pub fn try_new(
        lfca_bytes: Vec<u8>,
        origin: CanonicalNetworkOrigin,
    ) -> Result<Self, EditableDiffBaseMismatch> {
        let digest: [u8; 32] = sha2::Sha256::digest(&lfca_bytes).into();
        let inner = EditableDiffBaseInner {
            canonical_artifact_digest: Sha256Digest::from_bytes(digest),
            canonical_artifact_byte_length: ExactByteLength::new(lfca_bytes.len() as u64),
            network_revision: origin.network_revision(),
            lfca_bytes,
        };
        let base = Self {
            inner: Arc::new(inner),
        };
        if !base.corresponds_to(origin) {
            return Err(EditableDiffBaseMismatch);
        }
        Ok(base)
    }

    /// 差异基线 LFCA 字节的 SHA-256 与精确长度是否分别等于 `origin` 的记录。
    #[must_use]
    pub fn corresponds_to(&self, origin: CanonicalNetworkOrigin) -> bool {
        self.inner.canonical_artifact_digest == origin.canonical_artifact_digest()
            && self.inner.canonical_artifact_byte_length == origin.canonical_artifact_byte_length()
    }

    /// exact LFCA 字节；供宿主取值做下一次发射的 diff base。
    #[must_use]
    pub fn lfca_bytes(&self) -> &[u8] {
        &self.inner.lfca_bytes
    }

    /// LFCA 字节的 SHA-256。
    #[must_use]
    pub fn canonical_artifact_digest(&self) -> Sha256Digest {
        self.inner.canonical_artifact_digest
    }

    /// LFCA 字节的精确长度。
    #[must_use]
    pub fn canonical_artifact_byte_length(&self) -> ExactByteLength {
        self.inner.canonical_artifact_byte_length
    }

    /// 构造时绑定的路网修订标识。
    #[must_use]
    pub fn network_revision(&self) -> NetworkRevisionId {
        self.inner.network_revision
    }

    /// 测试专用：LFCA 字节缓冲实际保留的逻辑字节数。
    #[cfg(any(test, feature = "execution-evidence"))]
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        self.inner.lfca_bytes.capacity() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(key: &str, bytes: &[u8]) -> RoadEditingStateModule {
        RoadEditingStateModule::new(key.to_owned(), bytes.to_vec())
    }

    #[test]
    fn construction_rejects_structural_violations() {
        assert_eq!(
            RoadEditingState::try_new(Vec::new()),
            Err(InvalidRoadEditingState::Empty)
        );
        assert_eq!(
            RoadEditingState::try_new(vec![module("a", b"x"), module("", b"y")]),
            Err(InvalidRoadEditingState::EmptySourceDocumentKey { index: 1 })
        );
        assert_eq!(
            RoadEditingState::try_new(vec![module("a", b"x"), module("b", b"")]),
            Err(InvalidRoadEditingState::EmptySourceBytes { index: 1 })
        );
        assert_eq!(
            RoadEditingState::try_new(vec![
                module("b", b"x"),
                module("a", b"y"),
                module("b", b"z")
            ]),
            Err(InvalidRoadEditingState::DuplicateSourceDocumentKey {
                key: "b".to_owned()
            })
        );
    }

    #[test]
    fn canonical_order_is_independent_of_input_order() {
        let forward = RoadEditingState::try_new(vec![
            module("a.lfre", b"first"),
            module("b.lfre", b"second"),
        ])
        .unwrap();
        let reversed = RoadEditingState::try_new(vec![
            module("b.lfre", b"second"),
            module("a.lfre", b"first"),
        ])
        .unwrap();
        assert_eq!(forward, reversed);
        assert_eq!(forward.digest(), reversed.digest());
        let keys: Vec<_> = reversed
            .modules()
            .iter()
            .map(RoadEditingStateModule::source_document_key)
            .collect();
        assert_eq!(keys, ["a.lfre", "b.lfre"]);
        assert_eq!(forward.module_count(), 2);
        assert_eq!(forward.total_byte_length().get(), 11);
    }

    #[test]
    fn digest_matches_frozen_preimage_and_fixed_vector() {
        let state = RoadEditingState::try_new(vec![
            module("b.lfre", b"second"),
            module("a.lfre", b"first"),
        ])
        .unwrap();
        let mut preimage = Vec::new();
        preimage.extend_from_slice(b"laneflow:road-editing-state-digest:v1\0");
        preimage.extend_from_slice(&1_u16.to_le_bytes());
        preimage.extend_from_slice(&2_u32.to_le_bytes());
        for (key, bytes) in [(&b"a.lfre"[..], &b"first"[..]), (b"b.lfre", b"second")] {
            preimage.extend_from_slice(&(key.len() as u32).to_le_bytes());
            preimage.extend_from_slice(key);
            preimage.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            preimage.extend_from_slice(bytes);
        }
        assert_eq!(
            state.digest().as_bytes(),
            blake3::hash(&preimage).as_bytes()
        );
        assert_eq!(
            format!("{:x}", state.digest()),
            "a1188cdc94f1a74c4e587da656c670937dd0cec4dc65b91b4750d9df52f39048"
        );
    }

    #[test]
    fn clone_shares_payload_without_copying() {
        let state = RoadEditingState::try_new(vec![module("a.lfre", b"payload")]).unwrap();
        let clone = state.clone();
        assert!(std::ptr::eq(
            state.modules().as_ptr(),
            clone.modules().as_ptr()
        ));
    }
}
