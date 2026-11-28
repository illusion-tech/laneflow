use laneflow_static_contract::{AuthoringLaneId, RoadCorridorId, RoadSectionId, StableId128};
use laneflow_static_network::{
    BoundaryOwner, CorridorElement, SharedNetworkRevision, vehicle::HandoffBinding,
};

use crate::RouteHandoffOccurrence;

/// Routing、快照和摘要共用的物理边界稳定引用。左右按走廊参考方向排列。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StableRouteHandoffWindow {
    /// 横向交接的源出现下标。
    pub source_index: u32,
    /// 边界所有者种类，1 为走廊、2 为区段。
    pub owner_kind: u16,
    /// 边界所有者的稳定身份。
    pub owner: StableId128,
    /// 参考方向左侧成员的稳定身份。
    pub left_member: StableId128,
    /// 参考方向右侧成员的稳定身份。
    pub right_member: StableId128,
    /// 允许交接的走廊局部站位闭区间，单位毫米。
    pub stations_mm: [u64; 2],
}

impl StableRouteHandoffWindow {
    /// 从同修订的已绑定窗口取得持久稳定声明。身份无法解析时返回空。
    #[must_use]
    pub fn from_binding(
        revision: &SharedNetworkRevision,
        source_index: u32,
        binding: HandoffBinding,
    ) -> Option<Self> {
        if binding.revision() != revision.network_revision() {
            return None;
        }
        let window = binding.window();
        let rel = revision.traffic().relations();
        let ids = revision.identity();
        let left = usize::try_from(window.left_index).ok()?;
        let right = left.checked_add(1)?;
        let (owner_kind, owner, left_member, right_member) = match window.owner {
            BoundaryOwner::Corridor(corridor) => {
                let elements = rel.corridor_elements(corridor)?;
                let (CorridorElement::RoadSection(a), CorridorElement::RoadSection(b)) =
                    (*elements.get(left)?, *elements.get(right)?)
                else {
                    return None;
                };
                (
                    1,
                    *ids.stable_id(corridor)?.as_untyped(),
                    *ids.stable_id(a)?.as_untyped(),
                    *ids.stable_id(b)?.as_untyped(),
                )
            }
            BoundaryOwner::Section(section) => {
                let lanes = rel.section_lanes(section)?;
                (
                    2,
                    *ids.stable_id(section)?.as_untyped(),
                    *ids.stable_id(*lanes.get(left)?)?.as_untyped(),
                    *ids.stable_id(*lanes.get(right)?)?.as_untyped(),
                )
            }
        };
        Some(Self {
            source_index,
            owner_kind,
            owner,
            left_member,
            right_member,
            stations_mm: window.stations_mm,
        })
    }

    /// 仅用于同修订读回。跨修订必须先核对两端来源和物理窗口对应关系。
    pub(crate) fn occurrence(
        self,
        revision: &SharedNetworkRevision,
    ) -> Option<RouteHandoffOccurrence> {
        if self.stations_mm[0] > self.stations_mm[1] {
            return None;
        }
        let rel = revision.traffic().relations();
        let ids = revision.identity();
        let (owner, index) = match self.owner_kind {
            1 => {
                let c = ids.ordinal(RoadCorridorId::from_untyped(self.owner))?;
                let a = ids.ordinal(RoadSectionId::from_untyped(self.left_member))?;
                let b = ids.ordinal(RoadSectionId::from_untyped(self.right_member))?;
                let index = rel.corridor_elements(c)?.windows(2).position(|pair| {
                    pair == [
                        CorridorElement::RoadSection(a),
                        CorridorElement::RoadSection(b),
                    ]
                })?;
                (BoundaryOwner::Corridor(c), index)
            }
            2 => {
                let s = ids.ordinal(RoadSectionId::from_untyped(self.owner))?;
                let a = ids.ordinal(AuthoringLaneId::from_untyped(self.left_member))?;
                let b = ids.ordinal(AuthoringLaneId::from_untyped(self.right_member))?;
                let index = rel
                    .section_lanes(s)?
                    .windows(2)
                    .position(|pair| pair == [a, b])?;
                (BoundaryOwner::Section(s), index)
            }
            _ => return None,
        };
        Some(RouteHandoffOccurrence {
            revision: revision.network_revision(),
            source_index: self.source_index,
            owner,
            left_index: u32::try_from(index).ok()?,
            stations_mm: self.stations_mm,
        })
    }
}
