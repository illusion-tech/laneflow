//! 空间几何领域 Canonical LIR 记录。

use laneflow_static_contract::{CanonicalFrameId, CanonicalFrameOrdinal, FacilityBandOrdinal};

use crate::arena::TableRange;

use super::LirIdentityField;

pub(crate) struct LirCanonicalFrame {
    pub(crate) ordinal: CanonicalFrameOrdinal,
    pub(crate) stable_id: CanonicalFrameId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
}

/// 与 `LaneEdgeOrdinal` 同下标对齐的规范空间几何。
pub(crate) struct LirLaneEdgeGeometry {
    pub(crate) canonical_frame: CanonicalFrameOrdinal,
    pub(crate) points: TableRange<LirCanonicalPoint3F32>,
    pub(crate) segments: TableRange<LirSpatialSegment>,
    pub(crate) arc_length_meters: f32,
}

/// 按 FacilityBand 规范身份排序的稀疏不可遍历中心线表。
pub(crate) struct LirFacilityBandGeometry {
    pub(crate) facility_band: FacilityBandOrdinal,
    pub(crate) canonical_frame: CanonicalFrameOrdinal,
    pub(crate) points: TableRange<LirCanonicalPoint3F32>,
}

#[derive(Clone, Copy)]
pub(crate) struct LirCanonicalPoint3F32 {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
}

#[derive(Clone, Copy)]
pub(crate) struct LirSpatialSegment {
    pub(crate) length_meters: f32,
    pub(crate) cumulative_end_meters: f32,
    pub(crate) tangent: [f32; 3],
    pub(crate) up: [f32; 3],
}
