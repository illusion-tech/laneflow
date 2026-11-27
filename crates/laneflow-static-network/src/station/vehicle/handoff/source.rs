use super::Failure;
use crate::{CanonicalNetworkOrigin, SharedNetworkRevision};
use laneflow_format::{
    BoundedReReadableObjectSource, PostEmissionCheckedBundle, RegistryCheckedFieldValue as Value,
    RegistryCheckedRowIter, RegistryCheckedRowView, ValueCheckedObjectView,
};
use laneflow_static_contract::{EntityKind, FieldTag, RoadCorridorOrdinal, Sha256Digest};
use sha2::{Digest, Sha256};

/// 登记窗口所在走廊的一份完整道路编辑来源文档。
#[derive(Clone, Copy, Debug)]
pub struct HandoffSourceModule<'a> {
    /// LFSM 的 SourceModule.authoringNamespaceId。
    pub namespace: &'a str,
    /// 当前模块的唯一 LFRE 来源文档键。
    pub document_key: &'a str,
    /// 完整的 size-prefixed LFRE，不含外部包装。
    pub source_bytes: &'a [u8],
}

/// 已与指定共享路网的精确 LFCA 来源绑定核对的道路编辑来源集合。
/// 仅在跨修订重绑定期间借用，构造后可对多条已注册窗口复用。
pub struct HandoffSourceEvidence<'a> {
    origin: CanonicalNetworkOrigin,
    artifact: ValueCheckedObjectView<'a>,
    modules: Vec<SourceDocument<'a>>,
}

#[derive(PartialEq, Eq)]
pub(super) struct SourceDocument<'a> {
    namespace: &'a str,
    key: &'a str,
    length: u32,
    digest: Sha256Digest,
}

fn field(row: RegistryCheckedRowView<'_>, tag: u16) -> Result<Value<'_>, Failure> {
    row.field_by_tag(tag)
        .and_then(|v| v.value().ok())
        .ok_or(Failure::Unavailable)
}

type Documents<'a> = std::iter::Peekable<std::iter::Enumerate<RegistryCheckedRowIter<'a>>>;

fn module_digest<'a>(
    row: RegistryCheckedRowView<'a>,
    index: u32,
    docs: &mut Documents<'a>,
    supplied: &[SourceDocument<'_>],
    verified: &mut usize,
) -> Option<(&'a str, u32, [u8; 32])> {
    let (
        Value::U32(ordinal),
        Value::Utf8(namespace),
        Value::U16(language),
        Value::Sha256(expected),
        Value::U32(1),
    ) = (
        field(row, 1).ok()?,
        field(row, 2).ok()?,
        field(row, 3).ok()?,
        field(row, 4).ok()?,
        field(row, 5).ok()?,
    )
    else {
        return None;
    };
    if ordinal != index {
        return None;
    }
    // 文档按 (module ordinal, key) 顺序连续。两次顺序走读同一组，
    // 首次取得摘要原像的 count，第二次核对行序及来源实物并写入摘要。
    let count = docs
        .clone()
        .take_while(|(_, d)| matches!(field(*d,2),Ok(Value::U32(m)) if m == index))
        .count();
    if count == 0 {
        return None;
    }
    let claimed = supplied
        .binary_search_by_key(&namespace, |m| m.namespace)
        .ok();
    if claimed.is_some() && (language != 2 || count != 1) {
        return None;
    }
    let mut prior_key = None;
    let digest = laneflow_format::source_document_set_digest_v1((0..count).map(|_| {
        let (ordinal, row) = docs.next()?;
        let (Value::U32(raw), Value::Utf8(key), Value::Sha256(hash), Value::U32(size)) = (
            field(row, 1).ok()?,
            field(row, 3).ok()?,
            field(row, 4).ok()?,
            field(row, 5).ok()?,
        ) else {
            return None;
        };
        if ordinal != raw as usize || prior_key.is_some_and(|prev| prev >= key) {
            return None;
        }
        prior_key = Some(key);
        if let Some(i) = claimed {
            let m = &supplied[i];
            if m.key != key || m.length != size || m.digest != hash {
                return None;
            }
            *verified += 1;
        }
        Some((key, size, hash.into_bytes()))
    }))?;
    (digest == expected.into_bytes()).then_some((namespace, 1, digest))
}

impl<'a> HandoffSourceEvidence<'a> {
    /// 从后发射受检集合的 LFSM 文档行逐模块重算两级摘要，对照当前根精确 LFCA
    /// 的来源承诺。仅相关走廊需提交道路编辑实物。其他模块可用任何已发布的前端。
    /// 此处验证已采用制品的来源对应关系；制品发布真实性由宿主发布链负责。
    ///
    /// # Errors
    ///
    /// 制品与当前根不匹配、来源集合摘要不符、来源版本或输入大小不受支持时拒绝。
    pub fn check<
        L: BoundedReReadableObjectSource,
        M: BoundedReReadableObjectSource,
        D: BoundedReReadableObjectSource,
    >(
        network: &SharedNetworkRevision,
        bundle: &'a PostEmissionCheckedBundle<L, M, D>,
        modules: &[HandoffSourceModule<'a>],
    ) -> Result<Self, Failure> {
        let origin = *network.canonical_origin();
        if bundle.canonical_artifact_digest() != origin.canonical_artifact_digest()
            || bundle.canonical_artifact_byte_length() != origin.canonical_artifact_byte_length()
        {
            return Err(Failure::Binding);
        }
        let artifact = bundle.canonical_artifact_view();
        if bundle.source_collection_digest_version() != 1 || modules.is_empty() {
            return Err(Failure::Unavailable);
        }
        let sources = bundle
            .source_map_view()
            .registry_view()
            .section(1)
            .ok_or(Failure::Unavailable)?;
        let module_table = sources.table(0).ok_or(Failure::Unavailable)?;
        if modules.len() > module_table.row_count() as usize {
            return Err(Failure::Binding);
        }
        let mut documents = Vec::new();
        documents
            .try_reserve_exact(modules.len())
            .map_err(|_| Failure::WorkLimit)?;
        for m in modules {
            let bytes = m.source_bytes;
            let length = u32::try_from(bytes.len()).map_err(|_| Failure::InvalidInput)?;
            // 每个实物须匹配 LFSM 的 RoadEditing 来源语言、长度和内容摘要。
            // 另一生产前端的规范记录固定以 LFSOURCE 开头，不具有 LFRE 长度与标识。
            let body_length = length.checked_sub(4).ok_or(Failure::Binding)?;
            if bytes.starts_with(b"LFSOURCE")
                || bytes.get(8..12) != Some(b"LFRE")
                || bytes.get(..4) != Some(&body_length.to_le_bytes())
            {
                return Err(Failure::Binding);
            }
            documents.push(SourceDocument {
                namespace: m.namespace,
                key: m.document_key,
                length,
                digest: Sha256Digest::from_bytes(Sha256::digest(bytes).into()),
            });
        }
        documents.sort_unstable_by_key(|m| m.namespace);
        if documents
            .windows(2)
            .any(|pair| pair[0].namespace == pair[1].namespace)
        {
            return Err(Failure::Binding);
        }
        let mut rows = module_table.rows();
        let mut docs = sources
            .table(1)
            .ok_or(Failure::Unavailable)?
            .rows()
            .enumerate()
            .peekable();
        let mut verified = 0;
        let digest = laneflow_format::source_collection_digest_v1(
            (0..module_table.row_count())
                .map(|i| module_digest(rows.next()?, i, &mut docs, &documents, &mut verified)),
        )
        .ok_or(Failure::Binding)?;
        if verified != modules.len()
            || docs.next().is_some()
            || Sha256Digest::from_bytes(digest) != bundle.source_collection_digest()
        {
            return Err(Failure::Binding);
        }
        Ok(Self {
            origin,
            artifact,
            modules: documents,
        })
    }

    pub(super) fn corridor_source(
        &self,
        network: &SharedNetworkRevision,
        ordinal: RoadCorridorOrdinal,
    ) -> Result<&SourceDocument<'a>, Failure> {
        if self.origin != *network.canonical_origin() {
            return Err(Failure::Binding);
        }
        // Identity v1 的第一种实体为 RoadCorridor，其序号等于规范身份表行号。
        let row = self
            .artifact
            .registry_view()
            .section(1)
            .and_then(|s| s.table(0))
            .and_then(|t| t.row(ordinal.raw()))
            .ok_or(Failure::Unavailable)?;
        let (
            Value::U16(kind),
            Value::U32(raw),
            Value::StableId128(stable_id),
            Value::RecordVector(fields),
        ) = (
            field(row, 1)?,
            field(row, 2)?,
            field(row, 3)?,
            field(row, 4)?,
        )
        else {
            return Err(Failure::Unavailable);
        };
        if kind != EntityKind::RoadCorridor.code()
            || raw != ordinal.raw()
            || network
                .identity()
                .stable_id(ordinal)
                .map(|s| *s.as_untyped())
                != Some(stable_id)
        {
            return Err(Failure::Binding);
        }
        let first = fields.rows().next().ok_or(Failure::Unavailable)?;
        let (Value::U16(tag), Value::Bytes(namespace)) = (field(first, 1)?, field(first, 2)?)
        else {
            return Err(Failure::Unavailable);
        };
        if tag != FieldTag::AuthoringNamespaceId.code() {
            return Err(Failure::Binding);
        }
        self.modules
            .binary_search_by(|m| m.namespace.as_bytes().cmp(namespace))
            .map(|i| &self.modules[i])
            .map_err(|_| Failure::Unavailable)
    }
}
