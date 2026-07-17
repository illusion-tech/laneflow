//! 已完成 Core domain normalization 的 immutable static Parking registry。

use std::f64::consts::PI;

use indexmap::IndexMap;

use crate::{
    error::CoreError,
    graph::{EDGE_BOUNDARY_EPSILON, LaneGraph},
    handle::{EdgeHandle, ParkingAreaHandle, ParkingSpaceHandle},
    id::validate_external_id,
    profile::GEOMETRY_GAP_EPSILON,
};

/// Parking anchor 的语义位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParkingAnchorKind {
    Entry,
    Exit,
}

/// optional ParkingSpace 分组。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParkingArea {
    id: String,
}

impl ParkingArea {
    /// 创建尚未 normalization 的 area definition。
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }

    /// 返回 area external ID。
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// ParkingSpace 的 edge-relative rectangular pose。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParkingSpaceGeometry {
    lateral_offset: f64,
    heading_offset_radians: f64,
    length: f64,
    width: f64,
}

impl ParkingSpaceGeometry {
    /// 创建尚未 normalization 的 geometry value。
    pub const fn new(
        lateral_offset: f64,
        heading_offset_radians: f64,
        length: f64,
        width: f64,
    ) -> Self {
        Self {
            lateral_offset,
            heading_offset_radians,
            length,
            width,
        }
    }

    pub const fn lateral_offset(self) -> f64 {
        self.lateral_offset
    }

    pub const fn heading_offset_radians(self) -> f64 {
        self.heading_offset_radians
    }

    pub const fn length(self) -> f64 {
        self.length
    }

    pub const fn width(self) -> f64 {
        self.width
    }
}

/// 已解析到当前 LaneGraph 的 Parking entry/exit anchor。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParkingLaneAnchor {
    edge: EdgeHandle,
    progress: f64,
}

impl ParkingLaneAnchor {
    const fn new(edge: EdgeHandle, progress: f64) -> Self {
        Self { edge, progress }
    }

    pub const fn edge(self) -> EdgeHandle {
        self.edge
    }

    pub const fn progress(self) -> f64 {
        self.progress
    }
}

/// 尚未解析 graph handles 的 static ParkingSpace definition。
#[derive(Clone, Debug, PartialEq)]
pub struct ParkingSpace {
    id: String,
    area_id: Option<String>,
    entry_edge_id: String,
    entry_progress: f64,
    exit_edge_id: String,
    exit_progress: f64,
    geometry: ParkingSpaceGeometry,
}

impl ParkingSpace {
    /// 创建尚未 normalization 的 space definition。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        area_id: Option<String>,
        entry_edge_id: impl Into<String>,
        entry_progress: f64,
        exit_edge_id: impl Into<String>,
        exit_progress: f64,
        geometry: ParkingSpaceGeometry,
    ) -> Self {
        Self {
            id: id.into(),
            area_id,
            entry_edge_id: entry_edge_id.into(),
            entry_progress,
            exit_edge_id: exit_edge_id.into(),
            exit_progress,
            geometry,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn area_id(&self) -> Option<&str> {
        self.area_id.as_deref()
    }

    pub fn entry_edge_id(&self) -> &str {
        &self.entry_edge_id
    }

    pub const fn entry_progress(&self) -> f64 {
        self.entry_progress
    }

    pub fn exit_edge_id(&self) -> &str {
        &self.exit_edge_id
    }

    pub const fn exit_progress(&self) -> f64 {
        self.exit_progress
    }

    pub const fn geometry(&self) -> ParkingSpaceGeometry {
        self.geometry
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ResolvedParkingSpace {
    definition: ParkingSpace,
    area: Option<ParkingAreaHandle>,
    entry: ParkingLaneAnchor,
    exit: ParkingLaneAnchor,
}

/// Static Parking definitions、handles 与 ordered reverse indexes。
#[derive(Clone, Debug, PartialEq)]
pub struct ParkingRegistry {
    areas: Vec<ParkingArea>,
    area_handles: IndexMap<String, ParkingAreaHandle>,
    area_spaces: Vec<Vec<ParkingSpaceHandle>>,
    spaces: Vec<ResolvedParkingSpace>,
    space_handles: IndexMap<String, ParkingSpaceHandle>,
}

impl ParkingRegistry {
    /// 创建合法的空 Parking registry。
    pub fn empty() -> Self {
        Self {
            areas: Vec::new(),
            area_handles: IndexMap::new(),
            area_spaces: Vec::new(),
            spaces: Vec::new(),
            space_handles: IndexMap::new(),
        }
    }

    /// 按固定 fail-fast 顺序 normalization 全部 static Parking definitions。
    pub fn try_new<A, S>(lane_graph: &LaneGraph, areas: A, spaces: S) -> Result<Self, CoreError>
    where
        A: IntoIterator<Item = ParkingArea>,
        S: IntoIterator<Item = ParkingSpace>,
    {
        let mut area_definitions = Vec::new();
        let mut area_handles = IndexMap::new();
        for area in areas {
            validate_external_id("parking.areas[].id", area.id())?;
            if area_handles.contains_key(area.id()) {
                return Err(CoreError::DuplicateParkingAreaId {
                    area_id: area.id().to_owned(),
                });
            }
            let handle = ParkingAreaHandle::new(area_definitions.len());
            area_handles.insert(area.id().to_owned(), handle);
            area_definitions.push(area);
        }

        let mut space_definitions = Vec::new();
        let mut space_handles = IndexMap::new();
        let mut space_areas = Vec::new();
        let mut area_has_member = vec![false; area_definitions.len()];
        for space in spaces {
            validate_external_id("parking.spaces[].id", space.id())?;
            if space_handles.contains_key(space.id()) {
                return Err(CoreError::DuplicateParkingSpaceId {
                    space_id: space.id().to_owned(),
                });
            }
            let area = if let Some(area_id) = space.area_id() {
                validate_external_id("parking.spaces[].areaId", area_id)?;
                let handle = area_handles.get(area_id).copied().ok_or_else(|| {
                    CoreError::UnknownParkingSpaceArea {
                        space_id: space.id().to_owned(),
                        area_id: area_id.to_owned(),
                    }
                })?;
                area_has_member[handle.index()] = true;
                Some(handle)
            } else {
                None
            };
            let handle = ParkingSpaceHandle::new(space_definitions.len());
            space_handles.insert(space.id().to_owned(), handle);
            space_areas.push(area);
            space_definitions.push(space);
        }

        let mut entries = Vec::with_capacity(space_definitions.len());
        let mut exits = Vec::with_capacity(space_definitions.len());
        for space in &space_definitions {
            entries.push(resolve_anchor(
                lane_graph,
                space,
                ParkingAnchorKind::Entry,
                space.entry_edge_id(),
                space.entry_progress(),
            )?);
            exits.push(resolve_anchor(
                lane_graph,
                space,
                ParkingAnchorKind::Exit,
                space.exit_edge_id(),
                space.exit_progress(),
            )?);
        }

        for space in &space_definitions {
            validate_geometry(space)?;
        }

        for (index, has_member) in area_has_member.iter().copied().enumerate() {
            if !has_member {
                return Err(CoreError::OrphanParkingArea {
                    area_id: area_definitions[index].id().to_owned(),
                });
            }
        }

        let mut area_spaces = vec![Vec::new(); area_definitions.len()];
        for (index, area) in space_areas.iter().copied().enumerate() {
            if let Some(area) = area {
                area_spaces[area.index()].push(ParkingSpaceHandle::new(index));
            }
        }

        let spaces = space_definitions
            .into_iter()
            .zip(space_areas)
            .zip(entries)
            .zip(exits)
            .map(|(((definition, area), entry), exit)| ResolvedParkingSpace {
                definition,
                area,
                entry,
                exit,
            })
            .collect();

        Ok(Self {
            areas: area_definitions,
            area_handles,
            area_spaces,
            spaces,
            space_handles,
        })
    }

    pub(crate) fn rebind_to_lane_graph(self, lane_graph: &LaneGraph) -> Result<Self, CoreError> {
        Self::try_new(
            lane_graph,
            self.areas,
            self.spaces.into_iter().map(|resolved| resolved.definition),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.areas.is_empty() && self.spaces.is_empty()
    }

    pub fn area_handle(&self, id: &str) -> Option<ParkingAreaHandle> {
        self.area_handles.get(id).copied()
    }

    pub fn area(&self, handle: ParkingAreaHandle) -> Option<&ParkingArea> {
        self.areas.get(handle.index())
    }

    pub fn area_external_id(&self, handle: ParkingAreaHandle) -> Option<&str> {
        self.area(handle).map(ParkingArea::id)
    }

    pub fn areas(&self) -> impl ExactSizeIterator<Item = &ParkingArea> {
        self.areas.iter()
    }

    pub fn space_handle(&self, id: &str) -> Option<ParkingSpaceHandle> {
        self.space_handles.get(id).copied()
    }

    pub fn space(&self, handle: ParkingSpaceHandle) -> Option<&ParkingSpace> {
        self.spaces
            .get(handle.index())
            .map(|resolved| &resolved.definition)
    }

    pub fn space_external_id(&self, handle: ParkingSpaceHandle) -> Option<&str> {
        self.space(handle).map(ParkingSpace::id)
    }

    pub fn spaces(&self) -> impl ExactSizeIterator<Item = &ParkingSpace> {
        self.spaces.iter().map(|resolved| &resolved.definition)
    }

    pub fn area_spaces(&self, handle: ParkingAreaHandle) -> Option<&[ParkingSpaceHandle]> {
        self.area_spaces.get(handle.index()).map(Vec::as_slice)
    }

    /// 返回 `Some(None)` 表示合法 standalone space，`None` 表示未知 handle。
    pub fn space_area(&self, handle: ParkingSpaceHandle) -> Option<Option<ParkingAreaHandle>> {
        self.spaces
            .get(handle.index())
            .map(|resolved| resolved.area)
    }

    pub fn space_entry(&self, handle: ParkingSpaceHandle) -> Option<ParkingLaneAnchor> {
        self.spaces
            .get(handle.index())
            .map(|resolved| resolved.entry)
    }

    pub fn space_exit(&self, handle: ParkingSpaceHandle) -> Option<ParkingLaneAnchor> {
        self.spaces
            .get(handle.index())
            .map(|resolved| resolved.exit)
    }

    pub fn space_geometry(&self, handle: ParkingSpaceHandle) -> Option<ParkingSpaceGeometry> {
        self.space(handle).map(ParkingSpace::geometry)
    }
}

impl Default for ParkingRegistry {
    fn default() -> Self {
        Self::empty()
    }
}

fn resolve_anchor(
    lane_graph: &LaneGraph,
    space: &ParkingSpace,
    anchor: ParkingAnchorKind,
    edge_id: &str,
    edge_progress: f64,
) -> Result<ParkingLaneAnchor, CoreError> {
    let field = match anchor {
        ParkingAnchorKind::Entry => "parking.spaces[].entry.edgeId",
        ParkingAnchorKind::Exit => "parking.spaces[].exit.edgeId",
    };
    validate_external_id(field, edge_id)?;
    let edge =
        lane_graph
            .edge_handle(edge_id)
            .ok_or_else(|| CoreError::UnknownParkingAnchorEdge {
                space_id: space.id().to_owned(),
                anchor,
                edge_id: edge_id.to_owned(),
            })?;
    let edge_length = lane_graph
        .edge_length(edge)
        .expect("resolved parking anchor edge must have length")
        .value();
    if !edge_progress.is_finite()
        || edge_progress <= EDGE_BOUNDARY_EPSILON
        || edge_progress >= edge_length - EDGE_BOUNDARY_EPSILON
    {
        return Err(CoreError::ParkingAnchorProgressOutOfRange {
            space_id: space.id().to_owned(),
            anchor,
            edge_id: edge_id.to_owned(),
            edge_progress,
            edge_length,
        });
    }
    Ok(ParkingLaneAnchor::new(edge, edge_progress))
}

fn validate_geometry(space: &ParkingSpace) -> Result<(), CoreError> {
    let geometry = space.geometry();
    let values = [
        (
            "lateralOffset",
            geometry.lateral_offset(),
            geometry.lateral_offset().is_finite()
                && geometry.lateral_offset().abs() > GEOMETRY_GAP_EPSILON,
            "必须是 finite 且绝对值大于 GEOMETRY_GAP_EPSILON",
        ),
        (
            "headingOffsetRadians",
            geometry.heading_offset_radians(),
            geometry.heading_offset_radians().is_finite()
                && geometry.heading_offset_radians() >= -PI
                && geometry.heading_offset_radians() < PI,
            "必须是 finite 且位于 [-PI, PI)",
        ),
        (
            "length",
            geometry.length(),
            geometry.length().is_finite() && geometry.length() > GEOMETRY_GAP_EPSILON,
            "必须是 finite 且大于 GEOMETRY_GAP_EPSILON",
        ),
        (
            "width",
            geometry.width(),
            geometry.width().is_finite() && geometry.width() > GEOMETRY_GAP_EPSILON,
            "必须是 finite 且大于 GEOMETRY_GAP_EPSILON",
        ),
    ];
    for (field, value, valid, requirement) in values {
        if !valid {
            return Err(CoreError::InvalidParkingGeometryValue {
                space_id: space.id().to_owned(),
                field,
                value,
                requirement,
            });
        }
    }
    Ok(())
}
