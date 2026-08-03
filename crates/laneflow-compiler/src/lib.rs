#![doc = include_str!("../README.md")]

mod declaration;
mod diagnostic;
mod limits;
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
