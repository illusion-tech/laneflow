use std::fmt::Write as _;

use super::*;

/// 可移植发射的显式规范 provenance。
///
/// v1 只允许调用方提供 canonical compiler build ID；来源集合、编译选项、几何档位与
/// emitter 版本全部由同一个 `CompilationOutput` 和冻结规则派生。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableEmissionProvenanceV1 {
    pub(super) compiler_build_id: Box<str>,
}

impl PortableEmissionProvenanceV1 {
    /// 建立一份已规范化的 v1 provenance。
    ///
    /// # Errors
    ///
    /// build ID 不是 1..=128-byte ASCII，首字符不是字母/数字，或其余字符不属于
    /// `[A-Za-z0-9._+@-]` 时失败。
    pub fn try_new(compiler_build_id: impl Into<Box<str>>) -> Result<Self, PortableEmissionError> {
        let compiler_build_id = compiler_build_id.into();
        let bytes = compiler_build_id.as_bytes();
        let first_is_valid = bytes
            .first()
            .is_some_and(|byte| byte.is_ascii_alphanumeric());
        let all_are_valid = bytes.iter().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'@' | b'-')
        });
        if !(1..=128).contains(&bytes.len()) || !first_is_valid || !all_are_valid {
            return Err(PortableEmissionError::InvalidCompilerBuildId);
        }
        Ok(Self { compiler_build_id })
    }

    /// 返回 exact-byte 发射输入中的 canonical compiler build ID。
    #[must_use]
    pub fn compiler_build_id(&self) -> &str {
        &self.compiler_build_id
    }
}

/// 一份候选对象的不可覆盖计算绑定。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableObjectCandidate {
    bytes: Box<[u8]>,
    digest: Sha256Digest,
    object_key: Box<str>,
}

impl PortableObjectCandidate {
    /// 返回完整 exact bytes。
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// 返回从 exact bytes 重算的 SHA-256。
    #[must_use]
    pub const fn digest(&self) -> Sha256Digest {
        self.digest
    }

    /// 返回与摘要共同绑定 exact bytes 的强类型长度。
    #[must_use]
    pub fn byte_length(&self) -> ExactByteLength {
        ExactByteLength::new(
            u64::try_from(self.bytes.len()).expect("supported targets have at most 64-bit usize"),
        )
    }

    /// 返回唯一 `sha256/<64 lowercase hex>` object key。
    #[must_use]
    pub fn object_key(&self) -> &str {
        &self.object_key
    }
}

/// 同一次发射原子拥有的三对象未受信发布候选。
///
/// 取得本类型只证明 compiler emitter 已关闭三份 bytes、完成格式预检和内部绑定核对；
/// 它不是独立验证收据，也不授予发布或迁移权限。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortablePublicationCandidate {
    pub(super) canonical_artifact: PortableObjectCandidate,
    pub(super) source_map: PortableObjectCandidate,
    pub(super) semantic_diff: PortableObjectCandidate,
    pub(super) network_revision: NetworkRevisionId,
}

/// LFSD 的显式 base 选择。
///
/// `Artifact` 只接受已经完成格式结构和值域预检的借用。该能力不证明跨表引用、身份闭包、
/// revision 或真实性；emitter 只把它用于诊断性差异，并在分类前额外执行 v1 contract 与
/// 跨修订身份冲突检查。
#[derive(Clone, Copy, Debug)]
pub enum PortableDiffBase<'a> {
    Genesis,
    Artifact(ValueCheckedObjectView<'a>),
}

impl PortablePublicationCandidate {
    #[must_use]
    pub const fn canonical_artifact(&self) -> &PortableObjectCandidate {
        &self.canonical_artifact
    }

    #[must_use]
    pub const fn source_map(&self) -> &PortableObjectCandidate {
        &self.source_map
    }

    #[must_use]
    pub const fn semantic_diff(&self) -> &PortableObjectCandidate {
        &self.semantic_diff
    }

    #[must_use]
    pub const fn network_revision(&self) -> NetworkRevisionId {
        self.network_revision
    }
}

/// 可移植候选发射失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortableEmissionError {
    InvalidCompilerBuildId,
    Format(FormatError),
    ArithmeticOverflow,
    CandidateStagingLimitExceeded { actual: u64, limit: u64 },
    InvalidDiffBaseKind,
    DiffBaseSemanticMismatch,
    UnsupportedSemanticContractTransition,
    CrossRevisionStableIdCollision,
    InternalBindingMismatch,
}

impl From<FormatError> for PortableEmissionError {
    fn from(value: FormatError) -> Self {
        Self::Format(value)
    }
}

pub(super) fn sha256(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
}

fn object_key(digest: Sha256Digest) -> Box<str> {
    let mut key = String::with_capacity(71);
    key.push_str("sha256/");
    write!(&mut key, "{digest:x}").expect("writing to String is infallible");
    key.into_boxed_str()
}

pub(super) fn close_object(bytes: Box<[u8]>) -> PortableObjectCandidate {
    let digest = sha256(&bytes);
    PortableObjectCandidate {
        bytes,
        digest,
        object_key: object_key(digest),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_candidate_bindings_keep_static_contract_types() {
        let object = close_object(vec![1, 2, 3].into_boxed_slice());
        let digest: Sha256Digest = object.digest();
        let byte_length: ExactByteLength = object.byte_length();
        assert_eq!(digest, sha256(object.bytes()));
        assert_eq!(byte_length, ExactByteLength::new(3));

        let publication = PortablePublicationCandidate {
            canonical_artifact: object.clone(),
            source_map: object.clone(),
            semantic_diff: object,
            network_revision: NetworkRevisionId::from_digest(digest),
        };
        let network_revision: NetworkRevisionId = publication.network_revision();
        assert_eq!(network_revision.into_digest(), digest);
    }
}
