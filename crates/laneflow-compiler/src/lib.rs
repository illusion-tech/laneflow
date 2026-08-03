#![doc = include_str!("../README.md")]

// 这些阶段私有模块会在 Canonical LIR/Compiler 公共闭环切片接线；当前切片先以包内
// 测试冻结 Typed AST→HIR→MIR 的正确性，不能为消除暂时未接线而扩大公共 API。
#[allow(dead_code)]
mod arena;
mod declaration;
mod diagnostic;
#[allow(dead_code)]
mod hir;
mod limits;
#[allow(dead_code)]
mod mir;
mod module;
mod source;

pub use declaration::{EntityReference, LaneEdgeInput, LaneEdgeReference, ScalarViolation};
pub use diagnostic::{
    Diagnostic, DiagnosticBundle, DiagnosticCode, DiagnosticPayload, DiagnosticSeverity,
    SourceHeaderField, SourcePosition, SourceSpan, SourceTextViolation,
};
pub use limits::{CompileLimitDimension, CompileLimits};
pub use module::{
    CompilationUnit, CompilationUnitBuilder, SYNTHETIC_FRONTEND_VERSION, SourceLanguage,
    SourceModuleDescriptor, SyntheticModule, SyntheticModuleBuilder,
};
pub use source::{SourceModuleHeader, SourceModuleHeaderInput};
