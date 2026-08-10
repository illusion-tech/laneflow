use std::sync::Arc;

use sha2::{Digest, Sha256};

const SOURCE_DOCUMENT_SET_MAGIC: &[u8] = b"LFSOURCE-DOCUMENT-SET";

/// 首版来源文档集摘要前像版本。
pub const SOURCE_DOCUMENT_SET_DIGEST_VERSION: u32 = 1;

/// 官方来源模块使用的来源语言。
///
/// 这是封闭生产前端选择器，不是第三方前端插件登记接口。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u16)]
#[non_exhaustive]
pub enum SourceLanguage {
    SyntheticDsl = 1,
    RoadEditingSource = 3,
}

impl SourceLanguage {
    /// 返回描述符与诊断使用的稳定 ASCII 名称。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SyntheticDsl => "synthetic-dsl",
            Self::RoadEditingSource => "road-editing-source",
        }
    }
}

/// 一份来源文档的冷显示/审计来源记录。
///
/// 它不是内容身份或信任锚，也不参与文档集摘要、稳定标识或 LIR
/// 语义。精确的 current 文档角色与制品引用由 #297 冻结；当前 Synthetic
/// 文档没有额外的宿主来源声明。
pub struct SourceDocumentOrigin {
    pub(super) display_source: Option<Arc<str>>,
}

impl SourceDocumentOrigin {
    /// 返回调用方提供的未认证稳定显示/审计来源（如果存在）。
    #[must_use]
    pub fn display_source(&self) -> Option<&str> {
        self.display_source.as_deref()
    }

    pub(super) const fn synthetic() -> Self {
        Self {
            display_source: None,
        }
    }

    #[cfg(test)]
    pub(super) fn test(display_source: Option<&str>) -> Self {
        Self {
            display_source: display_source.map(Arc::from),
        }
    }
}

/// 由官方前端派生、与所属逻辑模块不可拆分的来源文档描述符。
///
/// 调用方不能绕过官方前端自行配对键、摘要、长度与来源记录：
///
/// ```compile_fail
/// use laneflow_compiler::{SourceDocumentDescriptor, SourceDocumentOrigin};
/// use std::sync::Arc;
/// let _forged = SourceDocumentDescriptor {
///     source_document_key: Arc::from("forged.document"),
///     source_document_digest: [0; 32],
///     source_record_byte_len: 0,
///     authoring_namespace_id: Arc::from("forged/module"),
///     origin: SourceDocumentOrigin { display_source: None },
/// };
/// ```
pub struct SourceDocumentDescriptor {
    pub(super) source_document_key: Arc<str>,
    pub(super) source_document_digest: [u8; 32],
    pub(super) source_record_byte_len: u32,
    pub(super) authoring_namespace_id: Arc<str>,
    pub(super) origin: SourceDocumentOrigin,
}

impl SourceDocumentDescriptor {
    /// 返回与机器路径无关的稳定文档键。
    #[must_use]
    pub fn source_document_key(&self) -> &str {
        &self.source_document_key
    }

    /// 返回官方前端对该文档规范来源记录计算的 SHA-256。
    #[must_use]
    pub const fn source_document_digest(&self) -> &[u8; 32] {
        &self.source_document_digest
    }

    /// 返回参与逐文档摘要的规范来源记录字节数。
    #[must_use]
    pub const fn source_record_byte_len(&self) -> u32 {
        self.source_record_byte_len
    }

    /// 返回拥有该文档的逻辑模块 authoring namespace。
    #[must_use]
    pub fn authoring_namespace_id(&self) -> &str {
        &self.authoring_namespace_id
    }

    /// 返回与文档身份不可分配对的冷显示/审计来源。
    #[must_use]
    pub const fn origin(&self) -> &SourceDocumentOrigin {
        &self.origin
    }

    pub(crate) fn source_document_key_arc(&self) -> Arc<str> {
        Arc::clone(&self.source_document_key)
    }

    pub(crate) fn source_map_logical_bytes(&self) -> u64 {
        32_u64
            .saturating_add(4)
            .saturating_add(4)
            .saturating_add(u64::try_from(self.source_document_key.len()).unwrap_or(u64::MAX))
            .saturating_add(4)
            .saturating_add(u64::try_from(self.authoring_namespace_id.len()).unwrap_or(u64::MAX))
            .saturating_add(self.origin.display_source.as_ref().map_or(1, |source| {
                1_u64
                    .saturating_add(4)
                    .saturating_add(u64::try_from(source.len()).unwrap_or(u64::MAX))
            }))
    }
}

/// 由官方前端派生、调用方无法独立构造的逻辑来源模块描述符。
///
/// 描述符与同一个 [`crate::SyntheticModule`] 内的规范来源记录不可分配对；其中
/// 文档集摘要只用于模块级重放/缓存比较；精确文档摘要、长度、键与来源
/// 记录属于 [`SourceDocumentDescriptor`]。
pub struct SourceModuleDescriptor {
    pub(super) authoring_namespace_id: Arc<str>,
    pub(super) source_language: SourceLanguage,
    pub(super) source_document_set_digest: [u8; 32],
    pub(super) source_document_set_digest_version: u32,
    pub(super) frontend_version: u32,
    pub(super) frontend_options_digest: [u8; 32],
    pub(super) generator_build_id: Arc<str>,
    pub(super) parameters_and_inputs_digest: [u8; 32],
    pub(super) random_seed: Option<u64>,
    pub(super) provenance: Arc<str>,
    pub(super) imports: Box<[Arc<str>]>,
}

impl SourceModuleDescriptor {
    /// 返回拥有本模块声明的稳定 authoring namespace。
    #[must_use]
    pub fn authoring_namespace_id(&self) -> &str {
        &self.authoring_namespace_id
    }

    /// 返回生成本模块的官方来源语言。
    #[must_use]
    pub const fn source_language(&self) -> SourceLanguage {
        self.source_language
    }

    /// 返回对模块内全部来源文档描述符进行版本化聚合的 SHA-256。
    #[must_use]
    pub const fn source_document_set_digest(&self) -> &[u8; 32] {
        &self.source_document_set_digest
    }

    /// 返回文档集摘要前像的编码版本。
    #[must_use]
    pub const fn source_document_set_digest_version(&self) -> u32 {
        self.source_document_set_digest_version
    }

    /// 返回该来源语言记录的编码版本。
    #[must_use]
    pub const fn frontend_version(&self) -> u32 {
        self.frontend_version
    }

    /// 返回调用方登记的前端选项摘要；它不认证来源记录内容。
    #[must_use]
    pub const fn frontend_options_digest(&self) -> &[u8; 32] {
        &self.frontend_options_digest
    }

    /// 返回生成器构建标识。
    #[must_use]
    pub fn generator_build_id(&self) -> &str {
        &self.generator_build_id
    }

    /// 返回调用参数与外部输入集合的登记摘要。
    #[must_use]
    pub const fn parameters_and_inputs_digest(&self) -> &[u8; 32] {
        &self.parameters_and_inputs_digest
    }

    /// 返回生成过程登记的随机种子。
    #[must_use]
    pub const fn random_seed(&self) -> Option<u64> {
        self.random_seed
    }

    /// 返回供审计使用的来源沿袭说明。
    #[must_use]
    pub fn provenance(&self) -> &str {
        &self.provenance
    }

    /// 按命名空间字节序遍历本模块的显式导入集合。
    ///
    /// 该顺序已在 `SyntheticModuleBuilder::finish` 冻结，不反映 `add_import` 调用顺序。
    pub fn imports(&self) -> impl ExactSizeIterator<Item = &str> {
        self.imports.iter().map(AsRef::as_ref)
    }

    pub(crate) fn authoring_namespace_arc(&self) -> Arc<str> {
        Arc::clone(&self.authoring_namespace_id)
    }

    /// 返回源映射伴随数据中此描述符的目标布局中立逻辑字节数。
    pub(crate) fn source_map_logical_bytes(&self) -> u64 {
        let fixed_bytes = 2_u64
            .saturating_add(32)
            .saturating_add(4)
            .saturating_add(4)
            .saturating_add(32)
            .saturating_add(32)
            .saturating_add(1)
            .saturating_add(self.random_seed.map_or(0, |_| 8))
            .saturating_add(4);
        [
            self.authoring_namespace_id.as_ref(),
            self.generator_build_id.as_ref(),
            self.provenance.as_ref(),
        ]
        .into_iter()
        .chain(self.imports.iter().map(AsRef::as_ref))
        .fold(fixed_bytes, |total, value| {
            total
                .saturating_add(4)
                .saturating_add(u64::try_from(value.len()).unwrap_or(u64::MAX))
        })
    }
}

pub(super) fn source_document_set_digest_v1(documents: &[SourceDocumentDescriptor]) -> [u8; 32] {
    assert!(
        !documents.is_empty(),
        "official logical modules must retain at least one source document"
    );
    assert!(
        documents.windows(2).all(|pair| {
            pair[0].source_document_key.as_bytes() <= pair[1].source_document_key.as_bytes()
        }),
        "source documents must be sorted before deriving the set digest"
    );

    let mut hasher = Sha256::new();
    hasher.update(SOURCE_DOCUMENT_SET_MAGIC);
    hasher.update(SOURCE_DOCUMENT_SET_DIGEST_VERSION.to_le_bytes());
    hasher.update(
        u32::try_from(documents.len())
            .expect("official frontend precheck bounds source document count")
            .to_le_bytes(),
    );
    for document in documents {
        let key = document.source_document_key.as_bytes();
        hasher.update(
            u32::try_from(key.len())
                .expect("official frontend precheck bounds source document key bytes")
                .to_le_bytes(),
        );
        hasher.update(key);
        hasher.update(document.source_record_byte_len.to_le_bytes());
        hasher.update(document.source_document_digest);
    }
    hasher.finalize().into()
}

pub(super) fn source_document_digest(source_record: &[u8]) -> [u8; 32] {
    #[cfg(test)]
    SOURCE_DOCUMENT_DIGEST_CALL_COUNT.with(|count| count.set(count.get().saturating_add(1)));
    Sha256::digest(source_record).into()
}

pub(super) fn freeze_source_documents(
    authoring_namespace_id: &Arc<str>,
    mut first: SourceDocumentDescriptor,
    mut remaining: Vec<SourceDocumentDescriptor>,
) -> (Box<[SourceDocumentDescriptor]>, [u8; 32]) {
    first.authoring_namespace_id = Arc::clone(authoring_namespace_id);
    for document in &mut remaining {
        document.authoring_namespace_id = Arc::clone(authoring_namespace_id);
    }
    remaining.push(first);
    remaining.sort_unstable_by(|left, right| {
        left.source_document_key
            .as_bytes()
            .cmp(right.source_document_key.as_bytes())
    });
    let documents = remaining.into_boxed_slice();
    let digest = source_document_set_digest_v1(&documents);
    (documents, digest)
}

#[cfg(test)]
thread_local! {
    pub(super) static SOURCE_DOCUMENT_DIGEST_CALL_COUNT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    use super::SourceLanguage;

    #[test]
    fn source_language_values_keep_the_unpublished_geometry_gap_and_new_exact_code() {
        assert_eq!(SourceLanguage::SyntheticDsl as u16, 1);
        assert_eq!(SourceLanguage::RoadEditingSource as u16, 3);
        assert_eq!(
            SourceLanguage::RoadEditingSource.as_str(),
            "road-editing-source"
        );
    }
}
