use super::{
    BoundaryExtentKind, BoundaryPayloadInput, BoundaryStation,
    boundary_payload::QuantizedBoundaryPayload, location::RoadEditingLocationFactory,
};
use crate::{
    RoadEditingPropertyStep as Step, RoadEditingStructKind as Struct,
    RoadEditingTableKind as Table, RoadEditingUnionKind as Union,
};
use crate::{RoadEditingRelationKind, RoadEditingRelationOccurrence, SourceLocation};
use laneflow_road_editing_wire::{
    generated::lane_flow::road_editing::v1 as wire,
    runtime::{ForwardsUOffset, Vector},
};
use laneflow_static_contract::EntityKind;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RowSource {
    pub primary: SourceLocation,
    pub contributing: Box<[SourceLocation]>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct NamedExtent {
    pub left: u32,
    pub kind: u8,
    pub key: Arc<str>,
    pub ends: [BoundaryStation; 2],
    pub payload: Option<QuantizedBoundaryPayload>,
    pub name_source: RowSource,
    pub payload_source: Option<RowSource>,
}

impl NamedExtent {
    pub(super) fn station_source(&self, index: usize) -> SourceLocation {
        self.name_source.primary.with_property([
            extent_field(4 + index as u16),
            Step::StructMember {
                structure: Struct::BoundaryStation,
                member_id: u8::from(matches!(self.ends[index], BoundaryStation::Fixed(_))),
            },
        ])
    }
}

/// 临时引用保持在来源所有者自己的有序成员中；规范序号仅在 LIR 排定后解析。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum WidthMember {
    Band(u32),
    Lane { element: u32, lane: u32 },
}

#[derive(Debug, PartialEq)]
pub(crate) struct WidthRow {
    pub member: WidthMember,
    pub lane: Option<(crate::declaration::StationBandProof, bool)>,
    pub source: RowSource,
}

#[derive(Debug, PartialEq)]
pub(crate) struct ExtentRow {
    pub section_element: Option<u32>,
    pub declaration: Arc<NamedExtent>,
    pub stations: [(u64, u32); 2],
}

#[derive(Debug, PartialEq)]
pub(crate) struct SurfaceRow {
    pub stations: [(u64, u32); 2],
    pub jets: [[f32; 9]; 2],
    pub errors: [f32; 5],
    pub source: RowSource,
}

#[derive(Debug, PartialEq)]
pub(crate) struct PublishedCrossSection {
    pub widths: Box<[WidthRow]>,
    pub extents: Box<[ExtentRow]>,
    pub surface: Box<[SurfaceRow]>,
}

impl RowSource {
    fn visit<E>(&self, visit: &mut impl FnMut(&SourceLocation) -> Result<(), E>) -> Result<(), E> {
        visit(&self.primary)?;
        self.contributing.iter().try_for_each(visit)
    }
}
impl NamedExtent {
    pub(crate) fn visit_sources<E>(
        &self,
        visit: &mut impl FnMut(&SourceLocation) -> Result<(), E>,
    ) -> Result<(), E> {
        self.name_source.visit(visit)?;
        if let Some(source) = &self.payload_source {
            source.visit(visit)?;
        }
        Ok(())
    }
}
impl PublishedCrossSection {
    pub(crate) fn visit_sources<E>(
        &self,
        visit: &mut impl FnMut(&SourceLocation) -> Result<(), E>,
    ) -> Result<(), E> {
        for w in &self.widths {
            w.source.visit(visit)?;
        }
        for e in &self.extents {
            e.declaration.visit_sources(visit)?;
        }
        for s in &self.surface {
            s.source.visit(visit)?;
        }
        Ok(())
    }
}

pub(super) type Extents<'a> = Vector<'a, ForwardsUOffset<wire::BoundaryExtent<'a>>>;
pub(super) type MemberEntry<'a> = ((u8, &'a str), u32);

#[derive(Clone, Copy)]
pub(super) enum Owner<'a> {
    Corridor(wire::RoadCorridor<'a>),
    Section(wire::RoadSection<'a>),
}

impl<'a> Owner<'a> {
    pub fn all(root: wire::RoadEditingSource<'a>) -> impl Iterator<Item = Self> {
        root.road_corridors()
            .iter()
            .map(Self::Corridor)
            .chain(root.road_sections().iter().map(Self::Section))
    }
    pub fn extents(self) -> Extents<'a> {
        match self {
            Self::Corridor(c) => c.boundary_extents(),
            Self::Section(s) => s.boundary_extents(),
        }
    }
    pub fn address(self) -> (&'a str, Option<&'a str>) {
        match self {
            Self::Corridor(c) => (c.road_corridor_key(), None),
            Self::Section(s) => (s.road_corridor(), Some(s.road_section_key())),
        }
    }
    pub fn members(self) -> impl ExactSizeIterator<Item = MemberEntry<'a>> {
        let count = match self {
            Self::Corridor(c) => c.elements().len(),
            Self::Section(s) => s.authoring_lanes().len(),
        };
        (0..count).map(move |i| {
            let member = match self {
                Self::Corridor(c) => {
                    let e = c.elements().get(i);
                    (e.kind().0, e.entity_reference())
                }
                Self::Section(s) => (
                    wire::BoundaryMemberKind::AuthoringLane.0,
                    s.authoring_lanes().get(i),
                ),
            };
            (member, i as u32)
        })
    }
    pub fn field(self) -> Step {
        match self {
            Self::Corridor(_) => field(Table::RoadCorridor, 10),
            Self::Section(_) => field(Table::RoadSection, 5),
        }
    }
}

pub(super) fn left_index(members: &[MemberEntry<'_>], e: wire::BoundaryExtent<'_>) -> Option<u32> {
    let lookup = |m: wire::BoundaryMember<'_>| {
        members
            .binary_search_by(|(k, _)| k.cmp(&(m.kind().0, m.entity_reference())))
            .ok()
            .map(|i| members[i].1)
    };
    let left = lookup(e.left())?;
    (lookup(e.right())? == left.checked_add(1)?).then_some(left)
}

pub(super) fn field(table: Table, field_id: u16) -> Step {
    Step::TableField { table, field_id }
}
pub(super) fn extent_field(n: u16) -> Step {
    field(Table::BoundaryExtent, n)
}

/// 在生成和检查来源贡献时共用所选载荷的叶路径；路缘两个端面的四项参数逐项定位。
pub(super) fn payload_paths(variant: u8, mut visit: impl FnMut(&[Step])) {
    let (table, count) = match variant {
        1 => (Table::CrossingRule, 2),
        2 => (Table::Guardrail, 7),
        3 => (Table::Curb, 3),
        _ => return,
    };
    let payload = Step::UnionVariant {
        union: Union::BoundaryPayload,
        discriminant: variant,
    };
    for i in 0..count {
        let path = [extent_field(7), payload, field(table, i)];
        if variant == 3 && i < 2 {
            for member_id in 0..4 {
                visit(&[
                    path[0],
                    path[1],
                    path[2],
                    Step::StructMember {
                        structure: Struct::CurbEnd,
                        member_id,
                    },
                ]);
            }
        } else {
            visit(&path);
        }
    }
}

pub(super) fn lower(owner: Owner<'_>, loc: &RoadEditingLocationFactory) -> Box<[Arc<NamedExtent>]> {
    if owner.extents().is_empty() {
        return Box::new([]);
    }
    let mut members: Vec<_> = owner.members().collect();
    members.sort_unstable();
    let mut rows: Vec<_> = owner
        .extents()
        .iter()
        .map(|e| (left_index(&members, e).expect("checked neighbors"), e))
        .collect();
    rows.sort_unstable_by_key(|(left, e)| (*left, e.extent_kind().0, e.local_key()));
    let (road, section) = owner.address();
    let parents = [road];
    let (kind, owner_keys, key) = if let Some(s) = section {
        (EntityKind::RoadSection, &parents[..], s)
    } else {
        (EntityKind::RoadCorridor, &parents[..0], road)
    };
    rows.into_iter()
        .enumerate()
        .map(|(i, (left, e))| {
            let location = |path: &[Step]| {
                loc.owner_local(
                    kind,
                    owner_keys,
                    key,
                    RoadEditingRelationKind::BoundaryExtent,
                    RoadEditingRelationOccurrence::CanonicalSetOrdinal(i as u32),
                    path,
                    e.canvas_selection(),
                )
            };
            let ends = [e.low(), e.high()]
                .map(|s| BoundaryStation::from_wire(s).expect("checked station"));
            let mut contributing = Vec::with_capacity(9);
            for side in 0..2 {
                for leaf in 0..2 {
                    contributing.push(location(&[
                        extent_field(side),
                        field(Table::BoundaryMember, leaf),
                    ]));
                }
            }
            contributing.push(location(&[extent_field(2)]));
            for (side, s) in ends.iter().enumerate() {
                let path = extent_field(4 + side as u16);
                contributing.push(location(&[
                    path,
                    Step::StructMember {
                        structure: Struct::BoundaryStation,
                        member_id: 0,
                    },
                ]));
                if matches!(s, BoundaryStation::Fixed(_)) {
                    contributing.push(location(&[
                        path,
                        Step::StructMember {
                            structure: Struct::BoundaryStation,
                            member_id: 1,
                        },
                    ]));
                }
            }
            let payload = BoundaryPayloadInput::from_wire(e)
                .expect("checked payload")
                .map(|p| p.quantize().expect("checked parameters"));
            let payload_source = payload.map(|_| {
                let mut contributing = Vec::new();
                payload_paths(e.payload_type().0, |p| contributing.push(location(p)));
                RowSource {
                    primary: location(&[extent_field(7)]),
                    contributing: contributing.into_boxed_slice(),
                }
            });
            Arc::new(NamedExtent {
                left,
                kind: match BoundaryExtentKind::from_wire(e.extent_kind()).expect("checked kind") {
                    BoundaryExtentKind::Rule => 1,
                    BoundaryExtentKind::Obstacle => 2,
                },
                key: e.local_key().into(),
                ends,
                payload,
                name_source: RowSource {
                    primary: location(&[extent_field(3)]),
                    contributing: contributing.into_boxed_slice(),
                },
                payload_source,
            })
        })
        .collect()
}
