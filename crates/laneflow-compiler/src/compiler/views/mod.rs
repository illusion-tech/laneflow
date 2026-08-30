//! Canonical LIR 公共只读视图。
//!
//! 字段保持私有，仅 `compiler` 模块可通过 `from_lir` / `from_record` 构造。

use laneflow_static_contract::{FieldTag, LaneEdgeId, LaneEdgeOrdinal};

use crate::lir::{LirIdentityField, LirLaneEdge, LirUnit};

mod access;
mod cross_section;
mod junction;
mod parking;
mod route;
mod signal;
mod spatial;

pub use access::{
    CanonicalAccessRegulationView, CanonicalAccessRuleView, CanonicalAccessTarget,
    CanonicalParticipantClassView, CanonicalVehicleProfileView,
};
pub use cross_section::{
    CanonicalAuthoringLaneView, CanonicalCorridorElement, CanonicalFacilityBandView,
    CanonicalLaneGroupView, CanonicalRoadCorridorView, CanonicalRoadSectionView,
};
pub use junction::{
    CanonicalJunctionInternalEdgeView, CanonicalJunctionView, CanonicalManeuverPathView,
    CanonicalMovementView,
};
pub use parking::{
    CanonicalParkingFacilityView, CanonicalParkingLaneAnchor, CanonicalParkingSpaceGeometry,
    CanonicalParkingSpaceView,
};
pub use route::{CanonicalManeuverGateView, CanonicalStopLineView, CanonicalWaitingZoneView};
pub use signal::{
    CanonicalSignalControl, CanonicalSignalControllerView, CanonicalSignalGroupView,
    CanonicalSignalPhaseStateView, CanonicalSignalPhaseView,
};
pub use spatial::{
    CanonicalFacilityBandGeometryView, CanonicalFrameView, CanonicalLaneEdgeGeometryView,
    CanonicalPoint3F32, CanonicalSpatialSegment,
};

macro_rules! impl_stable_entity_view {
    ($view:ident, $record:ty, $ordinal:ty, $id:ty) => {
        /// Canonical LIR 中一个已验证稳定实体的借用视图。
        #[derive(Clone, Copy)]
        pub struct $view<'a> {
            lir: &'a LirUnit,
            record: &'a $record,
        }

        impl<'a> $view<'a> {
            pub(in crate::compiler) const fn from_lir(
                lir: &'a LirUnit,
                record: &'a $record,
            ) -> Self {
                Self { lir, record }
            }
        }

        impl $view<'_> {
            /// 返回当前实体表中的有类型逻辑序号。
            #[must_use]
            pub const fn ordinal(&self) -> $ordinal {
                self.record.ordinal
            }

            /// 返回由完整 Identity v1 前像派生的有类型稳定标识。
            #[must_use]
            pub const fn stable_id(&self) -> $id {
                self.record.stable_id
            }

            /// 按 Identity v1 登记顺序遍历完整规范身份字段。
            pub fn identity_fields(
                &self,
            ) -> impl ExactSizeIterator<Item = CanonicalIdentityFieldView<'_>> {
                self.lir.identity_fields[self.record.identity_fields.as_usize_range()]
                    .iter()
                    .map(|field| {
                        CanonicalIdentityFieldView::from_lir(&self.lir.identity_field_bytes, field)
                    })
            }
        }
    };
}

pub(crate) use impl_stable_entity_view;

/// Canonical LIR 中一条 `LaneEdge` 记录的借用视图。
#[derive(Clone, Copy)]
pub struct CanonicalLaneEdgeView<'a> {
    lir: &'a LirUnit,
    edge: &'a LirLaneEdge,
}

impl<'a> CanonicalLaneEdgeView<'a> {
    pub(in crate::compiler) const fn from_lir(lir: &'a LirUnit, edge: &'a LirLaneEdge) -> Self {
        Self { lir, edge }
    }
}

impl CanonicalLaneEdgeView<'_> {
    /// 返回当前表中的有类型逻辑序号。
    #[must_use]
    pub const fn ordinal(&self) -> LaneEdgeOrdinal {
        self.edge.ordinal
    }

    /// 返回由完整 Identity v1 前像派生的稳定标识。
    #[must_use]
    pub const fn stable_id(&self) -> LaneEdgeId {
        self.edge.stable_id
    }

    /// 按 Identity v1 登记顺序遍历完整规范身份字段。
    pub fn identity_fields(&self) -> impl ExactSizeIterator<Item = CanonicalIdentityFieldView<'_>> {
        self.lir.identity_fields[self.edge.identity_fields.as_usize_range()]
            .iter()
            .map(|field| {
                CanonicalIdentityFieldView::from_lir(&self.lir.identity_field_bytes, field)
            })
    }

    /// 返回交通权威长度，单位为毫米。
    #[must_use]
    pub const fn length_mm(&self) -> u32 {
        self.edge.length_mm
    }

    /// 返回基础道路限速，单位为毫米每秒。
    #[must_use]
    pub const fn speed_limit_mm_s(&self) -> u32 {
        self.edge.speed_limit_mm_s
    }

    /// 返回按领域顺序冻结的下游边有类型序号。
    #[must_use]
    pub fn successors(&self) -> &[LaneEdgeOrdinal] {
        &self.lir.lane_edge_successors[self.edge.successors.as_usize_range()]
    }

    /// 返回与本边同 ordinal 对齐的规范空间几何；headless LIR 返回 `None`。
    #[must_use]
    pub fn spatial_geometry(&self) -> Option<CanonicalLaneEdgeGeometryView<'_>> {
        self.lir
            .lane_edge_geometries
            .get(self.edge.ordinal.index())
            .map(|geometry| {
                CanonicalLaneEdgeGeometryView::from_lir(self.lir, self.edge.ordinal, geometry)
            })
    }
}

/// Canonical LIR 共享身份字段池中的一项借用视图。
#[derive(Clone, Copy)]
pub struct CanonicalIdentityFieldView<'a> {
    identity_field_bytes: &'a [u8],
    field: &'a LirIdentityField,
}

impl<'a> CanonicalIdentityFieldView<'a> {
    pub(in crate::compiler) const fn from_lir(
        identity_field_bytes: &'a [u8],
        field: &'a LirIdentityField,
    ) -> Self {
        Self {
            identity_field_bytes,
            field,
        }
    }
}

impl CanonicalIdentityFieldView<'_> {
    /// 返回 Identity v1 登记字段标签。
    #[must_use]
    pub const fn tag(&self) -> FieldTag {
        self.field.tag
    }

    /// 返回字段的完整规范值字节，不包含标签和长度前缀。
    #[must_use]
    pub fn value_bytes(&self) -> &[u8] {
        &self.identity_field_bytes[self.field.value_bytes.as_usize_range()]
    }
}
