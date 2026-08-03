#![doc = include_str!("../README.md")]

mod diagnostic;
mod limits;
mod source;

pub use diagnostic::{
    Diagnostic, DiagnosticBundle, DiagnosticCode, DiagnosticPayload, DiagnosticSeverity,
    SourceHeaderField, SourcePosition, SourceSpan, SourceTextViolation,
};
pub use limits::{CompileLimitDimension, CompileLimits};
pub use source::{SourceModuleHeader, SourceModuleHeaderInput};
