use core::mem::size_of;

use laneflow_static_contract::{CanonicalFrameOrdinal, FacilityBandOrdinal, LaneEdgeOrdinal};

use crate::RangeU32;

/// 规范 `f32` 空间点。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalPoint {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// 一段预计算的规范采样几何。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SegmentGeometry {
    pub length_meters: f32,
    pub cumulative_end_meters: f32,
    pub tangent: [f32; 3],
    pub up: [f32; 3],
}

/// 单条 LaneEdge 的只读连续几何借用。
#[derive(Clone, Copy, Debug)]
pub struct LaneGeometryView<'a> {
    canonical_frame: CanonicalFrameOrdinal,
    arc_length_meters: f32,
    points: &'a [CanonicalPoint],
    segments: &'a [SegmentGeometry],
}

impl<'a> LaneGeometryView<'a> {
    #[must_use]
    pub const fn canonical_frame(self) -> CanonicalFrameOrdinal {
        self.canonical_frame
    }

    #[must_use]
    pub const fn arc_length_meters(self) -> f32 {
        self.arc_length_meters
    }

    #[must_use]
    pub const fn points(self) -> &'a [CanonicalPoint] {
        self.points
    }

    #[must_use]
    pub const fn segments(self) -> &'a [SegmentGeometry] {
        self.segments
    }
}

/// 完整覆盖 LaneEdge ordinal 的位姿采样静态数据。
pub struct LanePoseNetwork {
    canonical_frames: Box<[CanonicalFrameOrdinal]>,
    arc_lengths_meters: Box<[f32]>,
    point_ranges: Box<[RangeU32]>,
    points: Box<[CanonicalPoint]>,
    segment_ranges: Box<[RangeU32]>,
    segments: Box<[SegmentGeometry]>,
}

impl LanePoseNetwork {
    pub(crate) fn new(
        canonical_frames: Box<[CanonicalFrameOrdinal]>,
        arc_lengths_meters: Box<[f32]>,
        point_ranges: Box<[RangeU32]>,
        points: Box<[CanonicalPoint]>,
        segment_ranges: Box<[RangeU32]>,
        segments: Box<[SegmentGeometry]>,
    ) -> Self {
        Self {
            canonical_frames,
            arc_lengths_meters,
            point_ranges,
            points,
            segment_ranges,
            segments,
        }
    }

    #[must_use]
    pub fn lane_geometry(&self, lane_edge: LaneEdgeOrdinal) -> Option<LaneGeometryView<'_>> {
        let index = lane_edge.index();
        Some(LaneGeometryView {
            canonical_frame: *self.canonical_frames.get(index)?,
            arc_length_meters: *self.arc_lengths_meters.get(index)?,
            points: self.point_ranges.get(index)?.slice(&self.points),
            segments: self.segment_ranges.get(index)?.slice(&self.segments),
        })
    }

    #[must_use]
    pub fn lane_edge_count(&self) -> u32 {
        u32::try_from(self.canonical_frames.len()).expect("format-bounded lane count fits u32")
    }

    #[must_use]
    pub fn retained_logical_bytes(&self) -> u64 {
        logical_bytes::<CanonicalFrameOrdinal>(self.canonical_frames.len())
            + logical_bytes::<f32>(self.arc_lengths_meters.len())
            + logical_bytes::<RangeU32>(self.point_ranges.len())
            + logical_bytes::<CanonicalPoint>(self.points.len())
            + logical_bytes::<RangeU32>(self.segment_ranges.len())
            + logical_bytes::<SegmentGeometry>(self.segments.len())
    }
}

#[derive(Clone, Copy)]
pub(crate) struct FacilityGeometryEntry {
    pub(crate) facility_band: FacilityBandOrdinal,
    pub(crate) canonical_frame: CanonicalFrameOrdinal,
    pub(crate) point_range: RangeU32,
}

/// 单条 FacilityBand 的只读几何借用。
#[derive(Clone, Copy, Debug)]
pub struct FacilityGeometryView<'a> {
    canonical_frame: CanonicalFrameOrdinal,
    points: &'a [CanonicalPoint],
}

impl<'a> FacilityGeometryView<'a> {
    #[must_use]
    pub const fn canonical_frame(self) -> CanonicalFrameOrdinal {
        self.canonical_frame
    }

    #[must_use]
    pub const fn points(self) -> &'a [CanonicalPoint] {
        self.points
    }
}

/// 可选的共享规范空间数据；presence 不自动授予 lane-pose capability。
pub struct SharedSpatialNetwork {
    direction_profile: u8,
    lane_pose: Option<LanePoseNetwork>,
    facility_entries: Box<[FacilityGeometryEntry]>,
    facility_points: Box<[CanonicalPoint]>,
}

impl SharedSpatialNetwork {
    pub(crate) fn new(
        direction_profile: u8,
        lane_pose: Option<LanePoseNetwork>,
        facility_entries: Box<[FacilityGeometryEntry]>,
        facility_points: Box<[CanonicalPoint]>,
    ) -> Self {
        Self {
            direction_profile,
            lane_pose,
            facility_entries,
            facility_points,
        }
    }

    #[must_use]
    pub const fn direction_profile(&self) -> u8 {
        self.direction_profile
    }

    #[must_use]
    pub const fn lane_pose(&self) -> Option<&LanePoseNetwork> {
        self.lane_pose.as_ref()
    }

    #[must_use]
    pub fn facility_geometry(
        &self,
        facility_band: FacilityBandOrdinal,
    ) -> Option<FacilityGeometryView<'_>> {
        let index = self
            .facility_entries
            .binary_search_by_key(&facility_band.raw(), |entry| entry.facility_band.raw())
            .ok()?;
        let entry = self.facility_entries[index];
        Some(FacilityGeometryView {
            canonical_frame: entry.canonical_frame,
            points: entry.point_range.slice(&self.facility_points),
        })
    }

    #[must_use]
    pub fn facility_geometry_count(&self) -> u32 {
        u32::try_from(self.facility_entries.len())
            .expect("format-bounded facility geometry count fits u32")
    }

    #[must_use]
    pub fn retained_logical_bytes(&self) -> u64 {
        self.lane_pose
            .as_ref()
            .map_or(0, LanePoseNetwork::retained_logical_bytes)
            + logical_bytes::<FacilityGeometryEntry>(self.facility_entries.len())
            + logical_bytes::<CanonicalPoint>(self.facility_points.len())
    }
}

fn logical_bytes<T>(len: usize) -> u64 {
    u64::try_from(
        len.checked_mul(size_of::<T>())
            .expect("retained size fits usize"),
    )
    .expect("retained size fits u64")
}
