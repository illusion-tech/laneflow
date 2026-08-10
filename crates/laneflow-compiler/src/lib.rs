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
pub mod road_editing;
mod source;
mod source_map;

pub use compiler::{
    CanonicalAccessRegulationView, CanonicalAccessRuleView, CanonicalAccessTarget,
    CanonicalAuthoringLaneView, CanonicalCorridorElement, CanonicalFacilityBandGeometryView,
    CanonicalFacilityBandView, CanonicalFrameView, CanonicalGateOccurrenceView,
    CanonicalIdentityFieldView, CanonicalJunctionInternalEdgeView, CanonicalJunctionView,
    CanonicalLaneEdgeGeometryView, CanonicalLaneEdgeView, CanonicalLaneGroupView,
    CanonicalManeuverGateView, CanonicalManeuverOccurrenceView, CanonicalManeuverPathView,
    CanonicalMovementView, CanonicalParkingAreaView, CanonicalParkingLaneAnchor,
    CanonicalParkingSpaceGeometry, CanonicalParkingSpaceView, CanonicalParticipantClassView,
    CanonicalPoint3F32, CanonicalRoadCorridorView, CanonicalRoadSectionView,
    CanonicalSignalControl, CanonicalSignalControllerView, CanonicalSignalGroupView,
    CanonicalSignalPhaseStateView, CanonicalSignalPhaseView, CanonicalSpatialSegment,
    CanonicalStaticRouteOccurrenceRef, CanonicalStaticRouteView, CanonicalStopLineView,
    CanonicalVehicleProfileView, CanonicalWaitingZoneOccurrenceView, CanonicalWaitingZoneView,
    CompilationMetrics, CompilationOutput, Compiler, LirTableCounts, ValidatedCanonicalLir,
};
pub use declaration::{
    AccessRegulationInput, AccessRuleInput, AccessRuleTargetInput, AuthoringLaneInput,
    CanonicalFrameInput, CanonicalPoint3F32Input, CorridorElementReference, EntityReference,
    FacilityBandInput, FacilityBandReference, FacilityKindCategory, FacilityKindViolation,
    IidmVehicleProfileInput, JunctionInput, JunctionReference, LaneEdgeGeometryInput,
    LaneEdgeInput, LaneEdgeReference, LaneGroupInput, LaneGroupReference, ManeuverGateInput,
    ManeuverGateReference, ManeuverPathInput, ManeuverPathReference, MovementInput,
    MovementReference, ParkingAreaInput, ParkingAreaReference, ParkingLaneAnchorInput,
    ParkingSpaceGeometryInput, ParkingSpaceInput, ParticipantClassInput, ParticipantClassReference,
    RoadCorridorInput, RoadSectionInput, RoadSectionReference, ScalarViolation, SignalControlInput,
    SignalControllerInput, SignalGroupInput, SignalGroupReference, SignalGroupStateInput,
    SignalPhaseInput, StaticRouteInput, StopLineInput, StopLineReference, VehicleProfileInput,
    VehicleProfileReference, WaitingZoneInput,
};
pub use diagnostic::{
    AccessCapability, AccessPlane, AccessRegulationField, Diagnostic, DiagnosticBundle,
    DiagnosticCode, DiagnosticPayload, DiagnosticSeverity, GeometryDocumentViolation,
    ParkingAnchorRole, ParkingGeometryField, ParkingGeometryViolation, RoadEditingInputViolation,
    SourceHeaderField, SourcePosition, SourceSpan, SourceTextViolation, SpatialAxis,
    SpatialGeometryViolation, WaitingZoneGateRole,
};
pub use identity::CanonicalIdentityViolation;
pub use laneflow_static_contract::{AccessEffect, SignalAspect};
pub use limits::{CompileLimitDimension, CompileLimits};
pub use module::{
    CompilationUnit, CompilationUnitBuilder, GEOMETRY_FRONTEND_VERSION, GeometryAccuracyProfile,
    GeometryDirectionProfile, GeometryDocumentInput, GeometryModule, GeometryModuleBuilder,
    GeometryModuleCounts, GeometryOffsetCurveBucket, SOURCE_DOCUMENT_SET_DIGEST_VERSION,
    SYNTHETIC_FRONTEND_VERSION, SourceDocumentDescriptor, SourceDocumentOrigin, SourceLanguage,
    SourceModuleDescriptor, SyntheticModule, SyntheticModuleBuilder,
};
pub use source::{SourceModuleHeader, SourceModuleHeaderInput};
pub use source_map::{
    AccessRelationOwner, AccessRelationSourceView, AccessRuleSourceView, AuthoringLaneSourceView,
    CanonicalFrameSourceView, CrossSectionRelationOwner, CrossSectionRelationSourceView,
    FacilityBandSourceView, JunctionRelationOwner, JunctionRelationSourceView, JunctionSourceView,
    LaneEdgeSourceView, LaneEdgeSuccessorSourceView, LaneGroupSourceView, ManeuverGateSourceView,
    ManeuverPathSourceView, MovementSourceView, ParkingAreaSourceView, ParkingRelationSourceView,
    ParkingSpaceSourceView, ParticipantClassSourceView, RoadCorridorSourceView,
    RoadSectionSourceView, RouteRelationSourceView, SignalControllerSourceView,
    SignalGroupSourceView, SignalPhaseSourceView, SignalRelationOwner, SignalRelationSourceView,
    SourceDocumentView, SourceLocationView, SourceModuleSourceView, SourceRelationRole,
    SpatialRelationSourceView, StaticRouteSourceView, StopLineSourceView, ValidatedSourceMapInput,
    VehicleProfileSourceView, WaitingZoneSourceView,
};
