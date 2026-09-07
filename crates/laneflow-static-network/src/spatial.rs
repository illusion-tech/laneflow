use core::mem::size_of;

use laneflow_static_contract::{
    CanonicalFrameOrdinal, ConflictZoneOrdinal, FacilityBandOrdinal, LaneEdgeOrdinal,
};

use crate::RangeU32;

/// 规范 `f32` 空间点。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalPoint {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// 规范 `f32` XZ 平面点。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalPointXZ {
    pub x: f32,
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
    /// 几何所属的规范坐标框架。
    #[must_use]
    pub const fn canonical_frame(self) -> CanonicalFrameOrdinal {
        self.canonical_frame
    }

    /// 该 LaneEdge 的总弧长（米）。
    #[must_use]
    pub const fn arc_length_meters(self) -> f32 {
        self.arc_length_meters
    }

    /// 采样点序列。
    #[must_use]
    pub const fn points(self) -> &'a [CanonicalPoint] {
        self.points
    }

    /// 预计算的段几何序列。
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
    /// 由预构建的各连续 payload 组装位姿采样数据。
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

    /// 按 LaneEdge ordinal 借用其只读几何视图。
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

    /// 覆盖的 LaneEdge 总数。
    #[must_use]
    pub fn lane_edge_count(&self) -> u32 {
        u32::try_from(self.canonical_frames.len()).expect("format-bounded lane count fits u32")
    }

    /// 本网络保留的逻辑字节数。
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

/// 单条设施带几何的内部索引记录。
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

/// 单个冲突区空间区域的内部索引记录。
#[derive(Clone, Copy)]
pub(crate) struct ConflictZoneRegionEntry {
    pub(crate) conflict_zone: ConflictZoneOrdinal,
    pub(crate) canonical_frame: CanonicalFrameOrdinal,
    pub(crate) min_y: f32,
    pub(crate) max_y: f32,
    pub(crate) ring_range: RangeU32,
}

/// 单个冲突区空间区域的只读几何借用。
#[derive(Clone, Copy, Debug)]
pub struct ConflictZoneRegionView<'a> {
    canonical_frame: CanonicalFrameOrdinal,
    min_y: f32,
    max_y: f32,
    ring_xz: &'a [CanonicalPointXZ],
}

impl<'a> ConflictZoneRegionView<'a> {
    /// 区域所属的规范坐标框架。
    #[must_use]
    pub const fn canonical_frame(self) -> CanonicalFrameOrdinal {
        self.canonical_frame
    }

    /// 区域的最低与最高高度。
    #[must_use]
    pub const fn height_range(self) -> (f32, f32) {
        (self.min_y, self.max_y)
    }

    /// 区域边界的 XZ 环点序列。
    #[must_use]
    pub const fn ring_xz(self) -> &'a [CanonicalPointXZ] {
        self.ring_xz
    }
}

impl<'a> FacilityGeometryView<'a> {
    /// 几何所属的规范坐标框架。
    #[must_use]
    pub const fn canonical_frame(self) -> CanonicalFrameOrdinal {
        self.canonical_frame
    }

    /// 设施带的采样点序列。
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
    conflict_region_entries: Box<[ConflictZoneRegionEntry]>,
    conflict_region_points: Box<[CanonicalPointXZ]>,
}

impl SharedSpatialNetwork {
    /// 由预构建的方向 profile 与各连续 payload 组装共享空间数据。
    pub(crate) fn new(
        direction_profile: u8,
        lane_pose: Option<LanePoseNetwork>,
        facility_entries: Box<[FacilityGeometryEntry]>,
        facility_points: Box<[CanonicalPoint]>,
        conflict_region_entries: Box<[ConflictZoneRegionEntry]>,
        conflict_region_points: Box<[CanonicalPointXZ]>,
    ) -> Self {
        Self {
            direction_profile,
            lane_pose,
            facility_entries,
            facility_points,
            conflict_region_entries,
            conflict_region_points,
        }
    }

    /// 构建时选定的方向 profile 标识。
    #[must_use]
    pub const fn direction_profile(&self) -> u8 {
        self.direction_profile
    }

    /// 可选的车道位姿采样数据。
    #[must_use]
    pub const fn lane_pose(&self) -> Option<&LanePoseNetwork> {
        self.lane_pose.as_ref()
    }

    /// 按设施带 ordinal 查询其只读几何视图。
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

    /// 携带几何的设施带总数。
    #[must_use]
    pub fn facility_geometry_count(&self) -> u32 {
        u32::try_from(self.facility_entries.len())
            .expect("format-bounded facility geometry count fits u32")
    }

    /// 按冲突区 ordinal 查询其空间区域视图。
    #[must_use]
    pub fn conflict_zone_region(
        &self,
        conflict_zone: ConflictZoneOrdinal,
    ) -> Option<ConflictZoneRegionView<'_>> {
        let index = self
            .conflict_region_entries
            .binary_search_by_key(&conflict_zone.raw(), |entry| entry.conflict_zone.raw())
            .ok()?;
        let entry = self.conflict_region_entries[index];
        Some(ConflictZoneRegionView {
            canonical_frame: entry.canonical_frame,
            min_y: entry.min_y,
            max_y: entry.max_y,
            ring_xz: entry.ring_range.slice(&self.conflict_region_points),
        })
    }

    /// 本网络保留的逻辑字节数（含可选 lane-pose 数据）。
    #[must_use]
    pub fn retained_logical_bytes(&self) -> u64 {
        self.lane_pose
            .as_ref()
            .map_or(0, LanePoseNetwork::retained_logical_bytes)
            + logical_bytes::<FacilityGeometryEntry>(self.facility_entries.len())
            + logical_bytes::<CanonicalPoint>(self.facility_points.len())
            + logical_bytes::<ConflictZoneRegionEntry>(self.conflict_region_entries.len())
            + logical_bytes::<CanonicalPointXZ>(self.conflict_region_points.len())
    }
}

fn logical_bytes<T>(len: usize) -> u64 {
    u64::try_from(
        len.checked_mul(size_of::<T>())
            .expect("retained size fits usize"),
    )
    .expect("retained size fits u64")
}
