use laneflow_static_contract::{EntityKind, StableId128};

/// 构建失败涉及的稳定结构分类。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildStructure {
    ContractVersions,
    CanonicalIdentity,
    CanonicalEntityTable,
    LaneEdge,
    LaneSuccessors,
    LanePredecessors,
    ManeuverPath,
    ManeuverCandidates,
    Conflict,
    PlanningHints,
    ExecutionContract,
    SpatialPresence,
    LaneEdgeGeometry,
    FacilityBandGeometry,
    ConflictZoneRegion,
    RelationClosure,
    AccessPlane,
    RetainedOutput,
    BuilderScratch,
}

/// 构建失败的粗粒度稳定分类。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildErrorClass {
    InputInvariant,
    Order,
    Reference,
    Identity,
    Contract,
    Spatial,
    Budget,
    Arithmetic,
    Allocation,
    Cancelled,
}

/// 受检 LFCA 无法闭合为共享静态路网的稳定错误。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BuildError {
    InputInvariant {
        structure: BuildStructure,
    },
    UnexpectedOrdinal {
        structure: BuildStructure,
        expected: u32,
        actual: u32,
    },
    EntityKindOrder {
        previous: EntityKind,
        actual: EntityKind,
    },
    EntityCountMismatch {
        entity_kind: EntityKind,
        identity_count: u32,
        entity_count: u32,
    },
    StableIdMismatch {
        entity_kind: EntityKind,
        ordinal: u32,
    },
    DuplicateStableId {
        stable_id: StableId128,
    },
    ReferenceOutOfBounds {
        structure: BuildStructure,
        ordinal: u32,
        limit: u32,
    },
    NonCanonicalOrder {
        structure: BuildStructure,
        previous: u32,
        actual: u32,
    },
    ContractMismatch {
        structure: BuildStructure,
    },
    SpatialPresenceMismatch,
    SpatialCoverageMismatch {
        lane_edges: u32,
        geometries: u32,
    },
    SpatialLengthMismatch {
        lane_edge: u32,
        traffic_length_mm: u32,
        spatial_length_meters: f32,
    },
    SpatialFrameMismatch {
        predecessor: u32,
        successor: u32,
        predecessor_frame: u32,
        successor_frame: u32,
    },
    SpatialJoinGapMismatch {
        predecessor: u32,
        successor: u32,
        gap_meters: f32,
        tolerance_meters: f32,
    },
    BudgetExceeded {
        structure: BuildStructure,
        required: u64,
        limit: u64,
    },
    ArithmeticOverflow {
        structure: BuildStructure,
    },
    AllocationFailure {
        structure: BuildStructure,
    },
    AccessAmbiguity {
        plane: &'static str,
        unit: u32,
        class: u32,
        first_rule: u32,
        second_rule: u32,
    },
    Cancelled,
}

impl BuildError {
    #[must_use]
    pub const fn class(self) -> BuildErrorClass {
        match self {
            Self::InputInvariant { .. } => BuildErrorClass::InputInvariant,
            Self::UnexpectedOrdinal { .. }
            | Self::EntityKindOrder { .. }
            | Self::NonCanonicalOrder { .. } => BuildErrorClass::Order,
            Self::ReferenceOutOfBounds { .. } => BuildErrorClass::Reference,
            Self::EntityCountMismatch { .. }
            | Self::StableIdMismatch { .. }
            | Self::DuplicateStableId { .. } => BuildErrorClass::Identity,
            Self::ContractMismatch { .. } => BuildErrorClass::Contract,
            Self::SpatialPresenceMismatch
            | Self::SpatialCoverageMismatch { .. }
            | Self::SpatialLengthMismatch { .. }
            | Self::SpatialFrameMismatch { .. }
            | Self::SpatialJoinGapMismatch { .. } => BuildErrorClass::Spatial,
            Self::BudgetExceeded { .. } => BuildErrorClass::Budget,
            Self::ArithmeticOverflow { .. } => BuildErrorClass::Arithmetic,
            Self::AllocationFailure { .. } => BuildErrorClass::Allocation,
            Self::AccessAmbiguity { .. } => BuildErrorClass::Reference,
            Self::Cancelled => BuildErrorClass::Cancelled,
        }
    }
}
