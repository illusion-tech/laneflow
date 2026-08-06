//! 官方合成来源模块及规范模块图的构建。
//!
//! 数据流为 `SourceModuleHeader` → [`SyntheticModuleBuilder`] → [`SyntheticModule`] →
//! [`CompilationUnitBuilder`] → [`CompilationUnit`]。前一构建器校验并拥有 Typed AST
//! 声明，同时生成确定性的 `LFSOURCE` 来源记录；后一构建器闭合显式导入图，并冻结
//! “依赖在前、同层命名空间字节序”模块顺序。所有可失败的增量操作先计算并验证候选
//! 状态，再一次性提交，因而错误不会留下半条导入、声明或累计计数。
//!
//! `HashMap` 只服务唯一性与目标查找。来源记录顺序、诊断顺序和编译单元顺序均来自
//! 显式排序或稳定序列，不能改成遍历哈希表。

mod admission;
mod descriptor;
mod resources;
mod synthetic;
mod synthetic_record;

#[cfg(test)]
mod tests;

pub use admission::{CompilationUnit, CompilationUnitBuilder};
pub use descriptor::{
    SOURCE_DOCUMENT_SET_DIGEST_VERSION, SourceDocumentDescriptor, SourceDocumentOrigin,
    SourceLanguage, SourceModuleDescriptor,
};
pub use synthetic::{SYNTHETIC_FRONTEND_VERSION, SyntheticModule, SyntheticModuleBuilder};

pub(crate) use admission::{ResolvedSourceLocation, SourceDocumentOrdinal};

#[cfg(test)]
use admission::{
    AdmissionSizing, AdmittedOfficialModule, TestOfficialModule, TestSourceDocument,
    TypedAstModule, source_document_index_requested_bytes,
};
#[cfg(test)]
use descriptor::SOURCE_DOCUMENT_DIGEST_CALL_COUNT;
#[cfg(test)]
use resources::{requested_hash_table_bytes, size_bytes};
#[cfg(test)]
use synthetic_record::{encode_source_record, encoded_source_record_len};
