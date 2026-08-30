//! 空间几何领域 Canonical LIR 记录。

use laneflow_static_contract::{
    CanonicalFrameId, CanonicalFrameOrdinal, ConflictZoneOrdinal, FacilityBandOrdinal,
};

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

/// 按 ConflictZone 规范身份排序的稀疏 2.5D 区域表。
pub(crate) struct LirConflictZoneRegion {
    pub(crate) conflict_zone: ConflictZoneOrdinal,
    pub(crate) canonical_frame: CanonicalFrameOrdinal,
    pub(crate) min_y: f32,
    pub(crate) max_y: f32,
    pub(crate) ring_xz: TableRange<LirCanonicalPoint2F32>,
}

#[derive(Clone, Copy)]
pub(crate) struct LirCanonicalPoint2F32 {
    pub(crate) x: f32,
    pub(crate) z: f32,
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

use super::{FreezeEnv, LirSpatialCounts, push_lir_identity, table_overflow};
use crate::DiagnosticBundle;
use laneflow_static_contract::FieldTag;

pub(super) struct SpatialParts {
    pub canonical_frames: Vec<LirCanonicalFrame>,
    pub lane_edge_geometries: Vec<LirLaneEdgeGeometry>,
    pub facility_band_geometries: Vec<LirFacilityBandGeometry>,
    pub conflict_zone_regions: Vec<LirConflictZoneRegion>,
    pub canonical_points: Vec<LirCanonicalPoint3F32>,
    pub conflict_region_points: Vec<LirCanonicalPoint2F32>,
    pub spatial_segments: Vec<LirSpatialSegment>,
}

pub(super) fn freeze(
    env: &mut FreezeEnv<'_>,
    counts: &LirSpatialCounts,
) -> Result<SpatialParts, DiagnosticBundle> {
    let mut canonical_frames = Vec::with_capacity(env.capacity(counts.canonical_frames)?);
    for mir_key in env
        .orders
        .canonical_frames
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let frame = &env.mir.canonical_frames[mir_key.index()];
        let identity_range = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::CanonicalFrameKey,
            &env.mir.modules[frame.module.index()].authoring_namespace_id,
            &frame.stable_key,
            None,
            env.limits,
            env.primary_span.clone(),
        )?;
        canonical_frames.push(LirCanonicalFrame {
            ordinal: env.orders.canonical_frames.ordinal(mir_key),
            stable_id: frame.stable_id,
            identity_fields: identity_range,
        });
    }

    // HIR 已证明“空间存在时每条 LaneEdge 恰好一条几何”。冻结阶段只按最终
    // LaneEdgeOrdinal 重排，并保持每条中心线内部的点/线段顺序。
    let mut mir_edge_to_geometry = vec![None; env.mir.lane_edges.len()];
    for (index, geometry) in env.mir.lane_edge_geometries.iter().enumerate() {
        debug_assert!(mir_edge_to_geometry[geometry.lane_edge.index()].is_none());
        mir_edge_to_geometry[geometry.lane_edge.index()] = Some(index);
    }
    let mut lane_edge_geometries = Vec::with_capacity(env.capacity(counts.lane_edge_geometries)?);
    let mut canonical_points = Vec::with_capacity(env.capacity(counts.canonical_points)?);
    let mut spatial_segments = Vec::with_capacity(env.capacity(counts.spatial_segments)?);
    for mir_edge in env
        .orders
        .lane_edges
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let Some(geometry_index) = mir_edge_to_geometry[mir_edge.index()] else {
            debug_assert!(env.mir.lane_edge_geometries.is_empty());
            continue;
        };
        let geometry = &env.mir.lane_edge_geometries[geometry_index];
        let point_start = canonical_points.len();
        canonical_points.extend(
            env.mir.canonical_points[geometry.points.as_usize_range()]
                .iter()
                .map(|point| LirCanonicalPoint3F32 {
                    x: point.x,
                    y: point.y,
                    z: point.z,
                }),
        );
        let segment_start = spatial_segments.len();
        spatial_segments.extend(
            env.mir.spatial_segments[geometry.segments.as_usize_range()]
                .iter()
                .map(|segment| LirSpatialSegment {
                    length_meters: segment.length_meters,
                    cumulative_end_meters: segment.cumulative_end_meters,
                    tangent: segment.tangent,
                    up: segment.up,
                }),
        );
        lane_edge_geometries.push(LirLaneEdgeGeometry {
            canonical_frame: env
                .orders
                .canonical_frames
                .ordinal(geometry.canonical_frame),
            points: TableRange::try_from_usize(
                point_start,
                canonical_points.len().saturating_sub(point_start),
            )
            .map_err(|overflow| table_overflow(overflow, env.limits, env.primary_span.clone()))?,
            segments: TableRange::try_from_usize(
                segment_start,
                spatial_segments.len().saturating_sub(segment_start),
            )
            .map_err(|overflow| table_overflow(overflow, env.limits, env.primary_span.clone()))?,
            arc_length_meters: geometry.arc_length_meters,
        });
    }

    // FacilityBand 不进入可通行图，但其可视几何必须和实体使用同一规范顺序。每个
    // 稀疏几何行携带 band ordinal，view 通过有序表查找，避免复制第二份范围索引。
    let mut mir_band_to_geometry = vec![None; env.mir.facility_bands.len()];
    for (index, geometry) in env.mir.facility_band_geometries.iter().enumerate() {
        debug_assert!(mir_band_to_geometry[geometry.facility_band.index()].is_none());
        mir_band_to_geometry[geometry.facility_band.index()] = Some(index);
    }
    let mut facility_band_geometries =
        Vec::with_capacity(env.capacity(counts.facility_band_geometries)?);
    for mir_band in env
        .orders
        .facility_bands
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let Some(geometry_index) = mir_band_to_geometry[mir_band.index()] else {
            continue;
        };
        let geometry = &env.mir.facility_band_geometries[geometry_index];
        let point_start = canonical_points.len();
        canonical_points.extend(
            env.mir.canonical_points[geometry.points.as_usize_range()]
                .iter()
                .map(|point| LirCanonicalPoint3F32 {
                    x: point.x,
                    y: point.y,
                    z: point.z,
                }),
        );
        let facility_band = env.orders.facility_bands.ordinal(mir_band);
        facility_band_geometries.push(LirFacilityBandGeometry {
            facility_band,
            canonical_frame: env
                .orders
                .canonical_frames
                .ordinal(geometry.canonical_frame),
            points: TableRange::try_from_usize(
                point_start,
                canonical_points.len().saturating_sub(point_start),
            )
            .map_err(|overflow| table_overflow(overflow, env.limits, env.primary_span.clone()))?,
        });
    }

    let mut mir_zone_to_region = vec![None; env.mir.conflict_zones.len()];
    for (index, region) in env.mir.conflict_zone_regions.iter().enumerate() {
        debug_assert!(mir_zone_to_region[region.conflict_zone.index()].is_none());
        mir_zone_to_region[region.conflict_zone.index()] = Some(index);
    }
    let mut conflict_zone_regions = Vec::with_capacity(env.capacity(counts.conflict_zone_regions)?);
    let mut conflict_region_points =
        Vec::with_capacity(env.capacity(counts.conflict_region_points)?);
    for mir_zone in env
        .orders
        .conflict_zones
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let Some(region_index) = mir_zone_to_region[mir_zone.index()] else {
            continue;
        };
        let region = &env.mir.conflict_zone_regions[region_index];
        let point_start = conflict_region_points.len();
        conflict_region_points.extend(
            env.mir.conflict_region_points[region.ring_xz.as_usize_range()]
                .iter()
                .map(|point| LirCanonicalPoint2F32 {
                    x: point.x,
                    z: point.z,
                }),
        );
        conflict_zone_regions.push(LirConflictZoneRegion {
            conflict_zone: env.orders.conflict_zones.ordinal(mir_zone),
            canonical_frame: env.orders.canonical_frames.ordinal(region.canonical_frame),
            min_y: region.min_y,
            max_y: region.max_y,
            ring_xz: TableRange::try_from_usize(
                point_start,
                conflict_region_points.len().saturating_sub(point_start),
            )
            .map_err(|overflow| table_overflow(overflow, env.limits, env.primary_span.clone()))?,
        });
    }

    Ok(SpatialParts {
        canonical_frames,
        lane_edge_geometries,
        facility_band_geometries,
        conflict_zone_regions,
        canonical_points,
        conflict_region_points,
        spatial_segments,
    })
}
