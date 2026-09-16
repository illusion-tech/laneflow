use std::fmt::Write as _;

use super::*;

/// 可移植发射的显式规范 provenance。
///
/// v1 只允许调用方提供 canonical compiler build ID；来源集合、编译选项、几何档位与
/// emitter 版本全部由同一个 `CompilationOutput` 和冻结规则派生。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableEmissionProvenance {
    pub(super) compiler_build_id: Box<str>,
}

impl PortableEmissionProvenance {
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
    source: ImmutableObjectSource,
    digest: Sha256Digest,
    object_key: Box<str>,
}

impl PortableObjectCandidate {
    /// 返回完整 exact bytes。
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.source
            .as_bytes()
            .expect("closed candidate source was readable before construction")
    }

    /// 返回从 exact bytes 重算的 SHA-256。
    #[must_use]
    pub const fn digest(&self) -> Sha256Digest {
        self.digest
    }

    /// 返回与摘要共同绑定 exact bytes 的强类型长度。
    #[must_use]
    pub fn byte_length(&self) -> ExactByteLength {
        self.source.exact_byte_length()
    }

    /// 返回唯一 `sha256/<64 lowercase hex>` object key。
    #[must_use]
    pub fn object_key(&self) -> &str {
        &self.object_key
    }

    /// 百万级生产候选是否由 closed file backing 承载。
    #[must_use]
    pub const fn is_file_backed(&self) -> bool {
        self.source.is_file_backed()
    }

    /// 消耗候选并返回其不可变对象来源。
    pub(crate) fn into_source(self) -> ImmutableObjectSource {
        self.source
    }
}

/// 同一次发射原子拥有的三对象未受信发布候选。
///
/// 取得本类型只证明 compiler emitter 已关闭三份 bytes、完成格式预检和内部绑定核对；
/// 它不是发布能力，也不授予发布或迁移权限。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortablePublicationCandidate {
    pub(super) canonical_artifact: PortableObjectCandidate,
    pub(super) source_map: PortableObjectCandidate,
    pub(super) semantic_diff: PortableObjectCandidate,
    pub(super) network_revision: NetworkRevisionId,
    pub(super) compiler_build_id: Box<str>,
    pub(super) source_collection_digest_version: u16,
    pub(super) source_collection_digest: [u8; 32],
    pub(super) expected_semantic_diff_base: ExpectedSemanticDiffBase,
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
    /// 返回 LFCA 可移植规范制品候选。
    #[must_use]
    pub const fn canonical_artifact(&self) -> &PortableObjectCandidate {
        &self.canonical_artifact
    }

    /// 返回 LFSM 源映射封套候选。
    #[must_use]
    pub const fn source_map(&self) -> &PortableObjectCandidate {
        &self.source_map
    }

    /// 返回 LFSD 语义差异封套候选。
    #[must_use]
    pub const fn semantic_diff(&self) -> &PortableObjectCandidate {
        &self.semantic_diff
    }

    /// 返回与三份 exact bytes 共同绑定的网络修订标识。
    #[must_use]
    pub const fn network_revision(&self) -> NetworkRevisionId {
        self.network_revision
    }

    /// 返回与 LFCA/LFSM exact bytes 绑定的 canonical compiler build ID。
    #[must_use]
    pub fn compiler_build_id(&self) -> &str {
        &self.compiler_build_id
    }

    /// 返回 LFSM SourceMapBindings 中的来源集合摘要版本。
    #[must_use]
    pub const fn source_collection_digest_version(&self) -> u16 {
        self.source_collection_digest_version
    }

    /// 返回 LFSM SourceMapBindings 中的来源集合摘要。
    #[must_use]
    pub const fn source_collection_digest(&self) -> [u8; 32] {
        self.source_collection_digest
    }

    /// 返回从实际 `PortableDiffBase` 保存、供后发射检查使用的显式 base binding。
    #[must_use]
    pub const fn expected_semantic_diff_base(&self) -> ExpectedSemanticDiffBase {
        self.expected_semantic_diff_base
    }

    /// 消耗候选并拆解为后发射检查的输入：三份不可变对象来源与显式 diff base binding。
    pub(crate) fn into_check_inputs(
        self,
    ) -> (
        ImmutableObjectSource,
        ImmutableObjectSource,
        ImmutableObjectSource,
        ExpectedSemanticDiffBase,
    ) {
        (
            self.canonical_artifact.into_source(),
            self.source_map.into_source(),
            self.semantic_diff.into_source(),
            self.expected_semantic_diff_base,
        )
    }
}

/// 可移植候选发射失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortableEmissionError {
    /// build ID 不是 1..=128 字节 ASCII、首字符不是字母/数字，或其余字符不在允许
    /// 集合内（`PortableEmissionProvenance::try_new`）。
    InvalidCompilerBuildId,
    /// LFCA/LFSM/LFSD 或被检字节的格式编码、结构预检或值域预检失败；全部发射与
    /// portable policy 检查入口均可达。
    Format(FormatError),
    /// 发射或检查路径上的计数/字节数换算 checked 算术溢出；全部发射与检查入口
    /// 均可达，属超大输入防御。
    ArithmeticOverflow,
    /// 发射或检查路径的暂存投影 `try_reserve_exact` 失败；全部发射与检查入口
    /// 均可达。
    AllocationFailure,
    /// LFSD 路权增量与实际 LFCA、文档描述符或 Entity/StaticRule 表的排他分工
    /// 核对不一致（`check_portable_policy_diff`；两个 emit 入口内嵌同样可达）。
    PolicyDiffMismatch,
    /// LFSM 策略来源或 Movement 方向来源与实际 LFCA、文档描述符或同次受检来源
    /// 输入不闭合（`check_portable_policy_sources`；两个 emit 入口内嵌同样可达）。
    PolicySourceMismatch,
    /// `PortableDiffBase::Artifact` 提供的视图不是 LFCA 对象（两个 emit 入口）。
    InvalidDiffBaseKind,
    /// 提供的 base 制品无法按完整 LFCA 重建索引或校验策略引用，不能作为差异
    /// 基线（两个 emit 入口）。
    DiffBaseSemanticMismatch,
    /// base 与目标的静态契约版本行或执行契约行逐字节不同，跨修订语义契约转换
    /// 不受支持（两个 emit 入口与 `check_portable_policy_diff`）。
    UnsupportedSemanticContractTransition,
    /// 同一 StableId 在 base 与目标间改变了实体种类或规范身份字段
    /// （两个 emit 入口与 `check_portable_policy_diff`）。
    CrossRevisionStableIdCollision,
    /// emitter 自建对象的内部结构读取失败，或 LIR 规范关系与 LFCA 投影不一致；
    /// 属内部绑定防御，正常输出不应触达（两个 emit 入口）。
    InternalBindingMismatch,
    /// 发射到临时目录时的文件 I/O 失败（`emit_portable_candidate_to_staging`）。
    StagedObjectIo,
    /// 封存核对时 staged backing 实际长度与预检 exact length 不一致
    /// （`emit_portable_candidate_to_staging` 的 staged 写入路径；首次只读映射的
    /// 同类漂移经 `ObjectSource` 变体返回）。
    StagedBackingChanged,
    /// 从已封存 staged backing 读取 exact bytes 失败：越界、底层读取失败或
    /// backing 漂移（`emit_portable_candidate_to_staging`）。
    ObjectSource(ObjectSourceError),
    /// 预备对象的 `PortableObjectBytes`、三对象合计的 `PortableBundleBytes` 或
    /// 检查器 scratch 的 `StageScratchBytes` 超出编译资源配置档；全部发射与检查
    /// 入口均可达。
    CompileLimitExceeded {
        dimension: CompileLimitDimension,
        actual: u64,
        limit: u64,
    },
}

impl From<FormatError> for PortableEmissionError {
    fn from(value: FormatError) -> Self {
        Self::Format(value)
    }
}

impl From<StagedObjectError> for PortableEmissionError {
    fn from(value: StagedObjectError) -> Self {
        match value {
            StagedObjectError::Io(_) => Self::StagedObjectIo,
            StagedObjectError::ArithmeticOverflow => Self::ArithmeticOverflow,
            StagedObjectError::BackingChanged => Self::StagedBackingChanged,
        }
    }
}

impl From<ObjectSourceError> for PortableEmissionError {
    fn from(value: ObjectSourceError) -> Self {
        Self::ObjectSource(value)
    }
}

/// 计算 exact bytes 的 SHA-256 摘要。
pub(crate) fn sha256(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
}

/// 从摘要派生唯一的 `sha256/<64 lowercase hex>` object key。
pub(crate) fn object_key(digest: Sha256Digest) -> Box<str> {
    let mut key = String::with_capacity(71);
    key.push_str("sha256/");
    write!(&mut key, "{digest:x}").expect("writing to String is infallible");
    key.into_boxed_str()
}

/// 关闭一份内存 exact bytes：计算摘要与 object key，绑定为 `PortableObjectCandidate`。
pub(crate) fn close_object(bytes: Box<[u8]>) -> PortableObjectCandidate {
    let digest = sha256(&bytes);
    PortableObjectCandidate {
        source: ImmutableObjectSource::from_boxed_bytes(bytes),
        digest,
        object_key: object_key(digest),
    }
}

/// 关闭一份已 staged 的对象来源：从其 exact bytes 计算摘要与 object key 并绑定为候选。
pub(crate) fn close_staged_object(
    source: ClosedStagedObjectSource,
) -> Result<PortableObjectCandidate, PortableEmissionError> {
    let digest = sha256(source.as_bytes()?);
    Ok(PortableObjectCandidate {
        source: ImmutableObjectSource::from_staged(source),
        digest,
        object_key: object_key(digest),
    })
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
            compiler_build_id: Box::from("test-compiler"),
            source_collection_digest_version: 1,
            source_collection_digest: [0; 32],
            expected_semantic_diff_base: ExpectedSemanticDiffBase::Genesis,
        };
        let network_revision: NetworkRevisionId = publication.network_revision();
        assert_eq!(network_revision.into_digest(), digest);
    }
}
