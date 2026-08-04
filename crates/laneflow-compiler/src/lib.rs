#![doc = include_str!("../README.md")]

// arena/HIR/MIR/LIR 是生产 Compiler 闭环的私有实现层。保持这些模块私有，可防止临时
// 键、阶段表和未验证中间态被调用方误当作稳定输出；只有 compiler 模块能把完整成功
// 管线封装为对外的 ValidatedCanonicalLir 与配对来源伴随数据。
#[allow(dead_code)]
mod arena;
mod compiler;
mod declaration;
mod diagnostic;
mod hir;
mod identity;
mod limits;
mod lir;
mod mir;
mod module;
mod source;
mod source_map;

pub use compiler::{
    CanonicalAuthoringLaneView, CanonicalCorridorElement, CanonicalFacilityBandView,
    CanonicalGateOccurrenceView, CanonicalIdentityFieldView, CanonicalJunctionInternalEdgeView,
    CanonicalJunctionView, CanonicalLaneEdgeView, CanonicalLaneGroupView,
    CanonicalManeuverGateView, CanonicalManeuverOccurrenceView, CanonicalManeuverPathView,
    CanonicalMovementView, CanonicalRoadCorridorView, CanonicalRoadSectionView,
    CanonicalSignalControl, CanonicalSignalControllerView, CanonicalSignalGroupView,
    CanonicalSignalPhaseStateView, CanonicalSignalPhaseView, CanonicalStaticRouteOccurrenceRef,
    CanonicalStaticRouteView, CanonicalStopLineView, CanonicalWaitingZoneOccurrenceView,
    CanonicalWaitingZoneView, CompilationOutput, Compiler, ValidatedCanonicalLir,
};
pub use declaration::{
    AuthoringLaneInput, CorridorElementReference, EntityReference, FacilityBandInput,
    FacilityBandReference, FacilityKindCategory, FacilityKindViolation, JunctionInput,
    JunctionReference, LaneEdgeInput, LaneEdgeReference, LaneGroupInput, LaneGroupReference,
    ManeuverGateInput, ManeuverGateReference, ManeuverPathInput, ManeuverPathReference,
    MovementInput, MovementReference, RoadCorridorInput, RoadSectionInput, RoadSectionReference,
    ScalarViolation, SignalControlInput, SignalControllerInput, SignalGroupInput,
    SignalGroupReference, SignalGroupStateInput, SignalPhaseInput, StaticRouteInput, StopLineInput,
    StopLineReference, WaitingZoneInput,
};
pub use diagnostic::{
    Diagnostic, DiagnosticBundle, DiagnosticCode, DiagnosticPayload, DiagnosticSeverity,
    SourceHeaderField, SourcePosition, SourceSpan, SourceTextViolation, WaitingZoneGateRole,
};
pub use identity::CanonicalIdentityViolation;
pub use laneflow_static_contract::SignalAspect;
pub use limits::{CompileLimitDimension, CompileLimits};
pub use module::{
    CompilationUnit, CompilationUnitBuilder, SYNTHETIC_FRONTEND_VERSION, SourceLanguage,
    SourceModuleDescriptor, SyntheticModule, SyntheticModuleBuilder,
};
pub use source::{SourceModuleHeader, SourceModuleHeaderInput};
pub use source_map::{
    AuthoringLaneSourceView, CrossSectionRelationOwner, CrossSectionRelationSourceView,
    FacilityBandSourceView, JunctionRelationOwner, JunctionRelationSourceView, JunctionSourceView,
    LaneEdgeSourceView, LaneEdgeSuccessorSourceView, LaneGroupSourceView, ManeuverGateSourceView,
    ManeuverPathSourceView, MovementSourceView, RoadCorridorSourceView, RoadSectionSourceView,
    RouteRelationSourceView, SignalControllerSourceView, SignalGroupSourceView,
    SignalPhaseSourceView, SignalRelationOwner, SignalRelationSourceView, SourceDocumentView,
    SourceLocationView, SourceRelationRole, StaticRouteSourceView, StopLineSourceView,
    ValidatedSourceMapInput, WaitingZoneSourceView,
};
