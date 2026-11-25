//! 道路横断面上的有向结构边界及其编辑参数。
use super::*;

/// 走廊显式声明的横断面覆盖范围。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CrossSectionCoverage {
    /// 普通走廊。
    #[default]
    None,
    /// 开放共享里程和命名区间的位置。
    StationMapping,
    /// 声明全域路幅、支承及规则和实体清单。
    Complete,
}

/// 边界两侧的成员，按走向参考方向从左向右排列。
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum BoundaryMemberInput {
    /// 走廊中的道路区段。
    RoadSection(RoadSectionReference),
    /// 走廊中的设施带。
    FacilityBand(FacilityBandReference),
    /// 区段内的编制车道。
    AuthoringLane(AuthoringLaneReference),
}

/// 命名区间即使只开放里程也具有稳定种类。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum BoundaryExtentKind {
    /// 有向跨越规则。
    Rule,
    /// 有限实体障碍。
    Obstacle,
}

/// 固定端点采用从走向起点计量的路面米数。走廊端点随所在的低端或高端字段绑定。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoundaryStation {
    /// 走向上的固定绝对路面站位。
    Fixed(f64),
    /// 绑定所属走廊的对应端面。
    CorridorEndpoint,
}

/// 路缘某一端的水平左、右偏移及相对路面高程，单位为米。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurbEndInput {
    /// 左界相对边界的位置，向左为正。
    pub left_offset_meters: f64,
    /// 右界相对边界的位置，向左为正。
    pub right_offset_meters: f64,
    /// 相对路面的底高。
    pub bottom_height_meters: f64,
    /// 相对路面的顶高。
    pub top_height_meters: f64,
}

/// 护栏和路缘的几何参数使用米，方向、效果和锚定角色取 LFRE 登记值。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoundaryPayloadInput {
    /// 有向跨越规则。
    CrossingRule {
        /// 1 左到右，2 右到左，3 双向。
        directions: u8,
        /// 1 允许，2 禁止。
        effect: u8,
    },
    /// 从低端或高端向区间内部延伸的竖直护栏。
    Guardrail {
        /// 锚在低端时取 1，高端时取 2。
        anchor_role: u8,
        /// 轴线相对边界的有符号左偏移。
        lateral_offset_meters: f64,
        /// 两端帽中心的水平距离。
        horizontal_length_meters: f64,
        /// 护栏的水平端帽半径。
        radius_meters: f64,
        /// 相对路面的底高。
        bottom_height_meters: f64,
        /// 相对路面的顶高。
        top_height_meters: f64,
        /// 各长度参数统一的编制误差上界。
        parameter_error_meters: f64,
    },
    /// 在两个截面之间线性变化的路缘。
    Curb {
        /// 低站位端。
        low: CurbEndInput,
        /// 高站位端。
        high: CurbEndInput,
        /// 各长度参数统一的编制误差上界。
        parameter_error_meters: f64,
    },
}

/// 走廊接缝或区段内有向边界的一段。身份由所属对象、成员对、种类和局部名组成。
#[derive(Clone, Debug, PartialEq)]
pub struct BoundaryExtentInput {
    /// 参考方向上的左侧成员。
    pub left: BoundaryMemberInput,
    /// 参考方向上的右侧成员。
    pub right: BoundaryMemberInput,
    /// 规则或实体种类。
    pub kind: BoundaryExtentKind,
    /// 在同一有向边界和种类内唯一的局部名。
    pub local_key: Box<str>,
    /// 低站位端。
    pub low: BoundaryStation,
    /// 高站位端。
    pub high: BoundaryStation,
    /// 完整声明使用对应种类的载荷。
    pub payload: Option<BoundaryPayloadInput>,
    /// 可选的编辑器选择标识。
    pub canvas_selection: Option<Box<str>>,
}

fn bad(field: &'static str) -> DiagnosticBundle {
    input_error(field, RoadEditingInputViolation::InvalidCombination)
}

impl BoundaryStation {
    pub(in crate::road_editing) fn from_wire(
        value: &wire::BoundaryStation,
    ) -> Result<Self, &'static str> {
        match (value.kind(), value.station_meters()) {
            (wire::BoundaryStationKind::Fixed, s) if s.is_finite() && s >= 0.0 => {
                Ok(Self::Fixed(s))
            }
            (wire::BoundaryStationKind::CorridorEndpoint, 0.0) => Ok(Self::CorridorEndpoint),
            _ => Err("boundaryExtent.station"),
        }
    }
}

impl CurbEndInput {
    fn from_wire(e: &wire::CurbEnd) -> Self {
        Self {
            left_offset_meters: e.left_offset_meters(),
            right_offset_meters: e.right_offset_meters(),
            bottom_height_meters: e.bottom_height_meters(),
            top_height_meters: e.top_height_meters(),
        }
    }
}

impl BoundaryPayloadInput {
    pub(in crate::road_editing) fn from_wire(
        extent: wire::BoundaryExtent<'_>,
    ) -> Result<Option<Self>, &'static str> {
        let missing = "boundaryExtent.payload";
        Ok(Some(match extent.payload_type() {
            wire::BoundaryPayload::NONE if extent.payload().is_none() => return Ok(None),
            wire::BoundaryPayload::CrossingRule => {
                let p = extent.payload_as_crossing_rule().ok_or(missing)?;
                Self::CrossingRule {
                    directions: p.directions().ok_or("crossingRule.directions")?,
                    effect: p.effect().ok_or("crossingRule.effect")?,
                }
            }
            wire::BoundaryPayload::Guardrail => {
                let p = extent.payload_as_guardrail().ok_or(missing)?;
                Self::Guardrail {
                    anchor_role: p.anchor_role().ok_or("guardrail.anchorRole")?,
                    lateral_offset_meters: p
                        .lateral_offset_meters()
                        .ok_or("guardrail.lateralOffsetMeters")?,
                    horizontal_length_meters: p
                        .horizontal_length_meters()
                        .ok_or("guardrail.horizontalLengthMeters")?,
                    radius_meters: p.radius_meters().ok_or("guardrail.radiusMeters")?,
                    bottom_height_meters: p
                        .bottom_height_meters()
                        .ok_or("guardrail.bottomHeightMeters")?,
                    top_height_meters: p.top_height_meters().ok_or("guardrail.topHeightMeters")?,
                    parameter_error_meters: p
                        .parameter_error_meters()
                        .ok_or("guardrail.parameterErrorMeters")?,
                }
            }
            wire::BoundaryPayload::Curb => {
                let p = extent.payload_as_curb().ok_or(missing)?;
                Self::Curb {
                    low: CurbEndInput::from_wire(p.low()),
                    high: CurbEndInput::from_wire(p.high()),
                    parameter_error_meters: p
                        .parameter_error_meters()
                        .ok_or("curb.parameterErrorMeters")?,
                }
            }
            _ => return Err(missing),
        }))
    }
}

impl BoundaryExtentKind {
    pub(in crate::road_editing) fn from_wire(
        kind: wire::BoundaryExtentKind,
    ) -> Result<Self, &'static str> {
        match kind {
            wire::BoundaryExtentKind::Rule => Ok(Self::Rule),
            wire::BoundaryExtentKind::Obstacle => Ok(Self::Obstacle),
            _ => Err("boundaryExtent.extentKind"),
        }
    }
}

/// 完整的局部所有者地址决定要读回哪个结构边界集合。
pub enum BoundaryOwner<'a> {
    /// 走廊级相邻区段或设施带之间的接缝。
    Corridor(&'a RoadCorridorReference),
    /// 区段级相邻车道之间的边界。
    Section(&'a RoadSectionReference),
}

pub(in crate::road_editing) fn local_key_valid(name: &str) -> bool {
    (1..=53).contains(&name.len())
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._:/-".contains(&c))
}

impl BoundaryExtentInput {
    /// 按所有者读回同一作用域的命名区间，保留其余区间以供局部编辑和重新写出。
    ///
    /// # Errors
    ///
    /// 来源、引用或操作资源无效时返回诊断。
    pub fn read_from(
        input: super::super::RoadEditingModuleInput<'_>,
        owner: BoundaryOwner<'_>,
        limits: &crate::CompileProfile,
    ) -> Result<Option<Box<[Self]>>, DiagnosticBundle> {
        use crate::ledger::{Ledger, LedgerStage, PassLabel, Plan};
        let ledger = Ledger::new(limits.operation_budget());
        let verified = super::super::reader::verify_source_in(input, limits, 0, 0, &ledger)?;
        let root = verified.root();
        let extents = match owner {
            BoundaryOwner::Corridor(r) if r.module_namespace().is_none() => root
                .road_corridors()
                .iter()
                .find(|v| r.local_key() == v.road_corridor_key())
                .map(|v| v.boundary_extents()),
            BoundaryOwner::Section(r) if r.module_namespace().is_none() => root
                .road_sections()
                .iter()
                .find(|v| {
                    v.road_section_key() == r.local_key() && r.owner_keys().eq([v.road_corridor()])
                })
                .map(|v| v.boundary_extents()),
            _ => None,
        };
        let Some(extents) = extents else {
            return Ok(None);
        };
        let mut plan = Plan::ZERO.array::<Self>(extents.len());
        for e in extents {
            plan = plan.array::<u8>(e.local_key().len() + e.canvas_selection().map_or(0, str::len));
            for m in [e.left(), e.right()] {
                plan = plan
                    .array::<u8>(m.entity_reference().len())
                    .array::<Box<str>>(2);
            }
        }
        let pass = ledger.admit_pass(
            PassLabel::new(LedgerStage::Admission, "road-editing-read-boundaries"),
            plan,
        )?;
        let grant = pass.grant_bytes(plan.bytes().unwrap_or(u64::MAX))?;
        let values = extents
            .iter()
            .map(Self::from_wire)
            .collect::<Result<Box<[_]>, _>>()?;
        drop(grant);
        pass.finish(0)?;
        Ok(Some(values))
    }

    fn from_wire(e: wire::BoundaryExtent<'_>) -> Result<Self, DiagnosticBundle> {
        fn reference<K: EntityKindMarker>(
            value: &str,
        ) -> Result<RoadEditingReference<K>, DiagnosticBundle> {
            let mut parts = value.rsplitn(2, '>');
            let local = parts
                .next()
                .ok_or_else(|| bad("boundaryMember.entityReference"))?;
            let owners = parts
                .next()
                .ok_or_else(|| bad("boundaryMember.entityReference"))?;
            RoadEditingReference::owner_scoped(
                owners.split('>').map(str::to_owned).collect(),
                local,
            )
        }
        fn member(m: wire::BoundaryMember<'_>) -> Result<BoundaryMemberInput, DiagnosticBundle> {
            Ok(match m.kind() {
                wire::BoundaryMemberKind::RoadSection => {
                    BoundaryMemberInput::RoadSection(reference(m.entity_reference())?)
                }
                wire::BoundaryMemberKind::FacilityBand => {
                    BoundaryMemberInput::FacilityBand(reference(m.entity_reference())?)
                }
                wire::BoundaryMemberKind::AuthoringLane => {
                    BoundaryMemberInput::AuthoringLane(reference(m.entity_reference())?)
                }
                _ => return Err(bad("boundaryMember.kind")),
            })
        }
        Ok(Self {
            left: member(e.left())?,
            right: member(e.right())?,
            local_key: e.local_key().into(),
            kind: BoundaryExtentKind::from_wire(e.extent_kind()).map_err(bad)?,
            low: BoundaryStation::from_wire(e.low()).map_err(bad)?,
            high: BoundaryStation::from_wire(e.high()).map_err(bad)?,
            payload: BoundaryPayloadInput::from_wire(e).map_err(bad)?,
            canvas_selection: e.canvas_selection().map(Into::into),
        })
    }
}
