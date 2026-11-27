use sha2::{Digest, Sha256};

fn size(n: usize) -> Option<[u8; 4]> {
    Some(u32::try_from(n).ok()?.to_le_bytes())
}

/// 以规范文档键顺序生成版本 1 文档集摘要。各项为文档键、记录字节数及内容 SHA-256。
/// 数量或键长度不可按 u32 编码时返回空。
pub fn source_document_set_digest_v1<'a>(
    documents: impl ExactSizeIterator<Item = Option<(&'a str, u32, [u8; 32])>>,
) -> Option<[u8; 32]> {
    let mut hash = Sha256::new();
    hash.update(b"LFSOURCE-DOCUMENT-SET");
    hash.update(1_u32.to_le_bytes());
    hash.update(size(documents.len())?);
    for document in documents {
        let (key, length, digest) = document?;
        hash.update(size(key.len())?);
        hash.update(key.as_bytes());
        hash.update(length.to_le_bytes());
        hash.update(digest);
    }
    Some(hash.finalize().into())
}

/// 以编译器模块序生成版本 1 来源集合摘要。各项为编制命名空间、文档集摘要版本及摘要。
/// 数量或命名空间长度不可按 u32 编码时返回空。
pub fn source_collection_digest_v1<'a>(
    modules: impl ExactSizeIterator<Item = Option<(&'a str, u32, [u8; 32])>>,
) -> Option<[u8; 32]> {
    let mut hash = Sha256::new();
    hash.update(b"laneflow.source-collection.v1\0");
    hash.update(size(modules.len())?);
    for module in modules {
        let (namespace, version, digest) = module?;
        hash.update(size(namespace.len())?);
        hash.update(namespace.as_bytes());
        hash.update(version.to_le_bytes());
        hash.update(digest);
    }
    Some(hash.finalize().into())
}
