#![doc = include_str!("../README.md")]

// arena/HIR/MIR/LIR 会在 Compiler 公共闭环切片中接线。当前仅由包内测试验证
// Typed AST→HIR→MIR→Canonical LIR 的阶段不变量；保持私有可防止临时键和中间表被
// 调用方误当成已验证输出，也不能为了消除暂时的 dead_code 而提前扩大公共兼容面。
#[allow(dead_code)]
mod arena;
mod declaration;
mod diagnostic;
#[allow(dead_code)]
mod hir;
mod identity;
mod limits;
#[allow(dead_code)]
mod lir;
#[allow(dead_code)]
mod mir;
mod module;
mod source;

pub use declaration::{EntityReference, LaneEdgeInput, LaneEdgeReference, ScalarViolation};
pub use diagnostic::{
    Diagnostic, DiagnosticBundle, DiagnosticCode, DiagnosticPayload, DiagnosticSeverity,
    SourceHeaderField, SourcePosition, SourceSpan, SourceTextViolation,
};
pub use identity::CanonicalIdentityViolation;
pub use limits::{CompileLimitDimension, CompileLimits};
pub use module::{
    CompilationUnit, CompilationUnitBuilder, SYNTHETIC_FRONTEND_VERSION, SourceLanguage,
    SourceModuleDescriptor, SyntheticModule, SyntheticModuleBuilder,
};
pub use source::{SourceModuleHeader, SourceModuleHeaderInput};
