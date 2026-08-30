//! compiler 后发射检查与 LFCP v2 exact descriptor 构造。
//!
//! LaneFlow 只从同一次 emitter 候选建立受检 bundle，并从该能力构造 LFCP v2 binding。
//! 是否以及如何持久化、认证或发布 exact bytes 由宿主、CI 或打包工具负责。

mod lfcp;

use laneflow_format::{
    FormatError, FormatLimits, ImmutableObjectSource, PostEmissionCheckError,
    PostEmissionCheckedBundle, check_post_emission_bundle,
};

use crate::{PortableObjectCandidate, PortablePublicationCandidate};

pub(crate) use self::lfcp::build_lfcp;

/// LFCP v2 的发布者种类。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortablePublisherKind {
    LocalTool,
    Ci,
    ReleaseService,
}

impl PortablePublisherKind {
    const fn code(self) -> u8 {
        match self {
            Self::LocalTool => 0,
            Self::Ci => 1,
            Self::ReleaseService => 2,
        }
    }
}

/// 显式、受控且进入 LFCP v2 exact bytes 的发布 provenance。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortablePublicationProvenance {
    publisher_kind: PortablePublisherKind,
    publisher_build_id: Box<str>,
    controlled_build_provenance: Option<Box<str>>,
    controlled_timestamp: Option<Box<str>>,
}

impl PortablePublicationProvenance {
    /// 构造完整显式 provenance；本函数不读取环境、时钟或工作目录。
    #[must_use]
    pub fn new(
        publisher_kind: PortablePublisherKind,
        publisher_build_id: impl Into<Box<str>>,
        controlled_build_provenance: Option<Box<str>>,
        controlled_timestamp: Option<Box<str>>,
    ) -> Self {
        Self {
            publisher_kind,
            publisher_build_id: publisher_build_id.into(),
            controlled_build_provenance,
            controlled_timestamp,
        }
    }

    #[must_use]
    pub const fn publisher_kind(&self) -> PortablePublisherKind {
        self.publisher_kind
    }

    #[must_use]
    pub fn publisher_build_id(&self) -> &str {
        &self.publisher_build_id
    }

    #[must_use]
    pub fn controlled_build_provenance(&self) -> Option<&str> {
        self.controlled_build_provenance.as_deref()
    }

    #[must_use]
    pub fn controlled_timestamp(&self) -> Option<&str> {
        self.controlled_timestamp.as_deref()
    }
}

/// checked candidate / LFCP v2 构造失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortablePublicationError {
    PostEmission(PostEmissionCheckError),
    ArithmeticOverflow,
    Format(FormatError),
}

impl From<PostEmissionCheckError> for PortablePublicationError {
    fn from(value: PostEmissionCheckError) -> Self {
        Self::PostEmission(value)
    }
}

impl From<FormatError> for PortablePublicationError {
    fn from(value: FormatError) -> Self {
        Self::Format(value)
    }
}

/// 从同一次 compiler 候选建立来源拥有型后发射能力。
///
/// 调用方不能重新配对 LFCA/LFSM/LFSD 或覆盖 expected diff base binding。
pub fn check_portable_candidate(
    candidate: PortablePublicationCandidate,
    limits: FormatLimits,
) -> Result<
    PostEmissionCheckedBundle<ImmutableObjectSource, ImmutableObjectSource, ImmutableObjectSource>,
    PortablePublicationError,
> {
    let (canonical_artifact, source_map, semantic_diff, expected_base) =
        candidate.into_check_inputs();
    Ok(check_post_emission_bundle(
        canonical_artifact,
        source_map,
        semantic_diff,
        expected_base,
        limits,
    )?)
}

/// 检查候选并从受检 binding 构造 LFCP v2 exact bytes。
///
/// 成功不表示 descriptor 或其对象已经持久化、认证、发布或激活。
pub fn build_portable_publication_descriptor(
    candidate: PortablePublicationCandidate,
    provenance: &PortablePublicationProvenance,
    limits: FormatLimits,
) -> Result<PortableObjectCandidate, PortablePublicationError> {
    let checked = check_portable_candidate(candidate, limits)?;
    build_lfcp(&checked, provenance, limits)
}

#[cfg(test)]
mod tests;
