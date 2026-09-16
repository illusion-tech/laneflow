use std::collections::{HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};

/// 当前 scenario-local corridor catalog 版本。
pub const CATALOG_VERSION: &str = "0.4";

/// 走廊生成器明确编制的受保护准入策略键。
pub const PROTECTED_ENTRY_POLICY_KEY: &str = "protected-entry";

/// 走廊编制 Identity v1 的 `AuthoringNamespaceId`。
pub const AUTHORING_NAMESPACE: &str = "laneflow/signalized-corridor";

/// 现行走廊最小路径默认使用的车型编制键。
pub const PASSENGER_CAR_PROFILE_KEY: &str = "passenger-car";

/// 走廊编制中的第二车型键；bind 会解析但不作为默认 spawn profile。
pub const SHUTTLE_BUS_PROFILE_KEY: &str = "shuttle-bus";

/// 本封闭 catalog 的 Traffic Route 数。
pub const ROUTE_COUNT: usize = 28;

/// 本封闭 catalog 要求的最少 physical spawn slot 数。
pub const MIN_SPAWN_SLOT_COUNT: usize = 200;

/// 走廊 portal 的规范顺序。
pub const PORTAL_IDS: [&str; 6] = [
    "portal-main-west",
    "portal-main-east",
    "portal-side-1-north",
    "portal-side-1-south",
    "portal-side-2-north",
    "portal-side-2-south",
];

/// signalized-corridor 使用的 closed TOML catalog。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorridorCatalog {
    /// 内部 catalog 版本。
    pub catalog_version: String,
    /// 宿主必须明确选择策略。
    pub policy_selection: CatalogPolicySelection,
    /// portal entries。
    pub portals: Vec<PortalCatalogEntry>,
    /// Traffic route 到 exit portal 的 cross-reference。
    pub routes: Vec<RouteCatalogEntry>,
    /// route-independent physical spawn slots。
    pub spawn_slots: Vec<SpawnSlotCatalogEntry>,
}

/// 必填、闭合的宿主策略选择；内容来自同一 LFCA。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CatalogPolicySelection {
    NotRequired {},
    Pinned { policy: String },
}

impl CatalogPolicySelection {
    /// 只接受带实体种类的规范 StableId 文本。
    ///
    /// # Errors
    ///
    /// 策略身份文本不符合规范 StableId 语法、缺少实体种类或种类不是路权策略集
    /// （如 LaneEdge ID）时返回 [`CatalogError::InvalidPolicyIdentity`]。
    pub fn resolve(&self) -> Result<laneflow_runtime::WorldPolicySelection, CatalogError> {
        match self {
            Self::NotRequired {} => Ok(laneflow_runtime::WorldPolicySelection::NotRequired),
            Self::Pinned { policy } => policy
                .parse()
                .map(|policy| {
                    laneflow_runtime::WorldPolicySelection::Pinned(laneflow_runtime::PolicyPin {
                        policy,
                    })
                })
                .map_err(|_| CatalogError::InvalidPolicyIdentity),
        }
    }
}

/// corridor portal wire entry。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortalCatalogEntry {
    /// portal 外部 ID。
    pub id: String,
    /// 按 lane index 排序的 entry lanes。
    pub lanes: Vec<PortalLaneCatalogEntry>,
}

/// corridor portal lane wire entry。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortalLaneCatalogEntry {
    /// portal-local lane index。
    pub lane_index: usize,
    /// replacement 使用的共享 entry spawn slot。
    pub entry_spawn_slot_id: String,
    /// 按确定性 cumulative-selection 顺序排列的 route choices。
    pub route_choices: Vec<WeightedRouteChoiceCatalogEntry>,
}

/// corridor weighted route-choice wire entry。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WeightedRouteChoiceCatalogEntry {
    /// production Traffic route ID。
    pub route_id: String,
    /// lane-local 正整数 raw weight。
    pub weight: u64,
}

/// corridor route cross-reference wire entry。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteCatalogEntry {
    /// production Traffic route ID。
    pub route_id: String,
    /// exit portal ID。
    pub exit_portal_id: String,
    /// 有序 `laneEdgeKey`；非空，允许同一边多次出现。
    pub edge_ids: Vec<String>,
}

/// corridor spawn slot wire entry。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnSlotCatalogEntry {
    /// stable slot ID。
    pub slot_id: String,
    /// slot 所属 entry portal。
    pub portal_id: String,
    /// slot 所属 portal-local lane。
    pub lane_index: usize,
    /// production Traffic edge ID。
    pub edge_id: String,
    /// vehicle 前保险杠 edge-local progress。
    pub progress: f64,
}

/// catalog 0.4 线格式或交叉引用不合法。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogError {
    /// `catalog_version` 不等于封闭值 "0.4"，线格式代际不符（`validate`）。
    UnsupportedVersion(String),
    /// Pinned 策略文本不是带实体种类的规范 RightOfWayPolicySet StableId
    /// （`CatalogPolicySelection::resolve`；`validate`/`bind` 首步同样可达）。
    InvalidPolicyIdentity,
    /// portal 数量不是 6，或 portal.id 未按序逐一匹配封闭集合 PORTAL_IDS（`validate`）。
    PortalSet,
    /// portal id 重复（`validate`；portal.id 已逐位匹配互异封闭集合，此检查防御性保留）。
    DuplicatePortal(String),
    /// portal 的 lane 数与封闭要求不符：main portal 为 3、side portal 为 2（`validate`）。
    LaneCount {
        portal_id: String,
        expected: usize,
        actual: usize,
    },
    /// portal lane 的 lane_index 不是从 0 起连续递增的位置下标（`validate`）。
    LaneIndex {
        portal_id: String,
        expected: usize,
        actual: usize,
    },
    /// portal lane 未列任何加权路线选项（`validate`）。
    EmptyRouteChoices {
        portal_id: String,
        lane_index: usize,
    },
    /// portal lane 的某条路线选项 weight 为 0，cumulative selection 需要正整数权重（`validate`）。
    ZeroWeight { portal_id: String, route_id: String },
    /// 单条 portal lane 的全部选项权重按 u64 累加溢出（`validate`）。
    WeightOverflow { portal_id: String },
    /// 同一 portal lane 重复引用同一条路线（`validate`）。
    DuplicateChoice { portal_id: String, route_id: String },
    /// routes 表出现重复 route_id（`validate`）。
    DuplicateRoute(String),
    /// 路线的 edge_ids 序列为空（`validate`）。
    EmptyEdgeIds(String),
    /// 路线的 exit_portal_id 或 slot 的 portal_id 不在封闭集合 PORTAL_IDS 中（`validate`）。
    UnknownPortal(String),
    /// routes 表数量不等于封闭要求的 28 条（`validate`）。
    RouteCount(usize),
    /// portal lane 选项引用的 route_id 未在 routes 表声明（`validate`）。
    UnknownRoute(String),
    /// 路线被某 portal lane 引用，但其 exit_portal_id 与该入口 portal 相同（`validate`）。
    SameEntryExit { route_id: String },
    /// routes 表中的路线没有被任何 portal lane 选项引用（`validate`）。
    UnreferencedRoute(String),
    /// spawn slot 数低于封闭下限 200（`validate`）。
    InsufficientSlots(usize),
    /// slot_id 重复（`validate`）。
    DuplicateSlot(String),
    /// 某个编制键字段为空：portal.id、entry_spawn_slot_id、route_id、exit_portal_id、
    /// edge_ids、slot_id、portal_id、edge_id（`validate`）。
    EmptyId { field: &'static str },
    /// slot progress 非有限或为负（`validate`；毫米级落边由 `bind` 兜底）。
    InvalidProgress { slot_id: String },
    /// 两个 slot 的 (edge_id, progress 的 f64 bit) 完全相同（`validate`；毫米级重复由 `bind` 兜底）。
    DuplicatePosition { slot_id: String },
    /// slot 的 (portal_id, lane_index) 不匹配任何已声明 portal lane（`validate`）。
    SlotLane { slot_id: String },
    /// portal lane 的 entry_spawn_slot_id 在 slot 表中不存在（`validate`）。
    MissingEntrySlot {
        portal_id: String,
        lane_index: usize,
    },
    /// portal lane 的 entry slot 归属不符：其 portal_id 或 lane_index 与该 lane 不一致（`validate`）。
    EntrySlotMismatch {
        portal_id: String,
        lane_index: usize,
    },
}

impl fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPolicyIdentity => write!(
                formatter,
                "policy must be a canonical RightOfWayPolicySet StableId"
            ),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported catalog_version {version:?}")
            }
            Self::PortalSet => write!(
                formatter,
                "catalog portals must be exactly {PORTAL_IDS:?} in that order"
            ),
            Self::DuplicatePortal(id) => write!(formatter, "duplicate portal {id:?}"),
            Self::LaneCount {
                portal_id,
                expected,
                actual,
            } => write!(
                formatter,
                "portal {portal_id:?} must have {expected} lanes, found {actual}"
            ),
            Self::LaneIndex {
                portal_id,
                expected,
                actual,
            } => write!(
                formatter,
                "portal {portal_id:?} lane index {actual} must be {expected}"
            ),
            Self::EmptyRouteChoices {
                portal_id,
                lane_index,
            } => write!(
                formatter,
                "portal {portal_id:?} lane {lane_index} has no route choices"
            ),
            Self::ZeroWeight {
                portal_id,
                route_id,
            } => write!(
                formatter,
                "portal {portal_id:?} route {route_id:?} has zero weight"
            ),
            Self::WeightOverflow { portal_id } => {
                write!(formatter, "portal {portal_id:?} route weights overflow")
            }
            Self::DuplicateChoice {
                portal_id,
                route_id,
            } => write!(
                formatter,
                "portal {portal_id:?} repeats route choice {route_id:?}"
            ),
            Self::DuplicateRoute(id) => write!(formatter, "duplicate catalog route {id:?}"),
            Self::EmptyEdgeIds(id) => {
                write!(formatter, "catalog route {id:?} has empty edge_ids")
            }
            Self::UnknownPortal(id) => write!(formatter, "unknown portal {id:?}"),
            Self::RouteCount(actual) => {
                write!(
                    formatter,
                    "catalog must list {ROUTE_COUNT} routes, found {actual}"
                )
            }
            Self::UnknownRoute(id) => write!(formatter, "unknown catalog route {id:?}"),
            Self::SameEntryExit { route_id } => {
                write!(
                    formatter,
                    "route {route_id:?} has the same entry and exit portal"
                )
            }
            Self::UnreferencedRoute(id) => {
                write!(
                    formatter,
                    "catalog route {id:?} is not used by any portal lane"
                )
            }
            Self::InsufficientSlots(actual) => write!(
                formatter,
                "catalog must provide at least {MIN_SPAWN_SLOT_COUNT} spawn slots, found {actual}"
            ),
            Self::DuplicateSlot(id) => write!(formatter, "duplicate spawn slot {id:?}"),
            Self::EmptyId { field } => write!(formatter, "{field} must not be empty"),
            Self::InvalidProgress { slot_id } => {
                write!(
                    formatter,
                    "spawn slot {slot_id:?} progress is not finite or is negative"
                )
            }
            Self::DuplicatePosition { slot_id } => {
                write!(
                    formatter,
                    "spawn slot {slot_id:?} repeats a physical position"
                )
            }
            Self::SlotLane { slot_id } => {
                write!(
                    formatter,
                    "spawn slot {slot_id:?} has no matching portal lane"
                )
            }
            Self::MissingEntrySlot {
                portal_id,
                lane_index,
            } => write!(
                formatter,
                "portal {portal_id:?} lane {lane_index} entry_spawn_slot_id is missing"
            ),
            Self::EntrySlotMismatch {
                portal_id,
                lane_index,
            } => write!(
                formatter,
                "portal {portal_id:?} lane {lane_index} entry slot is not on that portal lane"
            ),
        }
    }
}

impl std::error::Error for CatalogError {}

/// 校验封闭 catalog 0.4 的版本、重复 ID、portal/lane/weight 与 slot 交叉引用。
///
/// 边是否属于所选 route、progress 是否落在已安装修订的边长内，由 `bind` 对照共享路网修订检查。
///
/// # Errors
///
/// catalog 版本不受支持、portal 集合数量或 ID 序列与封闭集合不符、成员
/// StableId 重复或非法、车道数量/下标非法、路线数量不符、路线无入口边、
/// 出口 portal 未知、入口出口相同、路线未被引用或 slot 不足、选项引用未声明
/// 路线（`UnknownRoute`）、路线选项为空、
/// 权重为零或权重和溢出、选项重复或 slot 交叉引用非法、slot 进度非法（负数/
/// 非有限）或位置重复（`InvalidProgress` / `DuplicatePosition`）时返回相应
/// [`CatalogError`]。
pub fn validate(catalog: &CorridorCatalog) -> Result<(), CatalogError> {
    catalog.policy_selection.resolve()?;
    if catalog.catalog_version != CATALOG_VERSION {
        return Err(CatalogError::UnsupportedVersion(
            catalog.catalog_version.clone(),
        ));
    }
    if catalog.portals.len() != PORTAL_IDS.len() {
        return Err(CatalogError::PortalSet);
    }

    let mut seen_portals = HashSet::new();
    for (index, portal) in catalog.portals.iter().enumerate() {
        if portal.id != PORTAL_IDS[index] {
            return Err(CatalogError::PortalSet);
        }
        require_id("portal.id", &portal.id)?;
        if !seen_portals.insert(portal.id.as_str()) {
            return Err(CatalogError::DuplicatePortal(portal.id.clone()));
        }
        let expected_lanes = if PORTAL_IDS[index].contains("-main-") {
            3
        } else {
            2
        };
        if portal.lanes.len() != expected_lanes {
            return Err(CatalogError::LaneCount {
                portal_id: portal.id.clone(),
                expected: expected_lanes,
                actual: portal.lanes.len(),
            });
        }
        for (lane_index, lane) in portal.lanes.iter().enumerate() {
            if lane.lane_index != lane_index {
                return Err(CatalogError::LaneIndex {
                    portal_id: portal.id.clone(),
                    expected: lane_index,
                    actual: lane.lane_index,
                });
            }
            if lane.route_choices.is_empty() {
                return Err(CatalogError::EmptyRouteChoices {
                    portal_id: portal.id.clone(),
                    lane_index,
                });
            }
            require_id("entry_spawn_slot_id", &lane.entry_spawn_slot_id)?;
            let mut choice_routes = HashSet::new();
            let mut weight_sum = 0_u64;
            for choice in &lane.route_choices {
                require_id("route_id", &choice.route_id)?;
                if choice.weight == 0 {
                    return Err(CatalogError::ZeroWeight {
                        portal_id: portal.id.clone(),
                        route_id: choice.route_id.clone(),
                    });
                }
                weight_sum = weight_sum.checked_add(choice.weight).ok_or_else(|| {
                    CatalogError::WeightOverflow {
                        portal_id: portal.id.clone(),
                    }
                })?;
                if !choice_routes.insert(choice.route_id.as_str()) {
                    return Err(CatalogError::DuplicateChoice {
                        portal_id: portal.id.clone(),
                        route_id: choice.route_id.clone(),
                    });
                }
            }
            let _ = weight_sum;
        }
    }

    if catalog.routes.len() != ROUTE_COUNT {
        return Err(CatalogError::RouteCount(catalog.routes.len()));
    }
    let mut route_ids = HashSet::new();
    let mut route_exit = HashMap::new();
    for route in &catalog.routes {
        require_id("route_id", &route.route_id)?;
        require_id("exit_portal_id", &route.exit_portal_id)?;
        if !route_ids.insert(route.route_id.as_str()) {
            return Err(CatalogError::DuplicateRoute(route.route_id.clone()));
        }
        if route.edge_ids.is_empty() {
            return Err(CatalogError::EmptyEdgeIds(route.route_id.clone()));
        }
        for edge_id in &route.edge_ids {
            require_id("edge_ids", edge_id)?;
        }
        if !PORTAL_IDS.contains(&route.exit_portal_id.as_str()) {
            return Err(CatalogError::UnknownPortal(route.exit_portal_id.clone()));
        }
        route_exit.insert(route.route_id.as_str(), route.exit_portal_id.as_str());
    }

    let mut referenced_routes = HashSet::new();
    for portal in &catalog.portals {
        for lane in &portal.lanes {
            for choice in &lane.route_choices {
                let exit = route_exit
                    .get(choice.route_id.as_str())
                    .ok_or_else(|| CatalogError::UnknownRoute(choice.route_id.clone()))?;
                if *exit == portal.id.as_str() {
                    return Err(CatalogError::SameEntryExit {
                        route_id: choice.route_id.clone(),
                    });
                }
                referenced_routes.insert(choice.route_id.as_str());
            }
        }
    }
    for route in &catalog.routes {
        if !referenced_routes.contains(route.route_id.as_str()) {
            return Err(CatalogError::UnreferencedRoute(route.route_id.clone()));
        }
    }

    if catalog.spawn_slots.len() < MIN_SPAWN_SLOT_COUNT {
        return Err(CatalogError::InsufficientSlots(catalog.spawn_slots.len()));
    }

    let lane_keys = catalog
        .portals
        .iter()
        .flat_map(|portal| {
            portal
                .lanes
                .iter()
                .map(move |lane| (portal.id.as_str(), lane.lane_index))
        })
        .collect::<HashSet<_>>();
    let mut slot_ids = HashSet::new();
    let mut positions = HashSet::new();
    let mut slot_by_id = HashMap::new();
    for slot in &catalog.spawn_slots {
        require_id("slot_id", &slot.slot_id)?;
        require_id("portal_id", &slot.portal_id)?;
        require_id("edge_id", &slot.edge_id)?;
        if !slot_ids.insert(slot.slot_id.as_str()) {
            return Err(CatalogError::DuplicateSlot(slot.slot_id.clone()));
        }
        if !slot.progress.is_finite() || slot.progress < 0.0 {
            return Err(CatalogError::InvalidProgress {
                slot_id: slot.slot_id.clone(),
            });
        }
        let progress_bits = if slot.progress == 0.0 {
            0.0_f64.to_bits()
        } else {
            slot.progress.to_bits()
        };
        if !positions.insert((slot.edge_id.as_str(), progress_bits)) {
            return Err(CatalogError::DuplicatePosition {
                slot_id: slot.slot_id.clone(),
            });
        }
        if !PORTAL_IDS.contains(&slot.portal_id.as_str()) {
            return Err(CatalogError::UnknownPortal(slot.portal_id.clone()));
        }
        if !lane_keys.contains(&(slot.portal_id.as_str(), slot.lane_index)) {
            return Err(CatalogError::SlotLane {
                slot_id: slot.slot_id.clone(),
            });
        }
        slot_by_id.insert(slot.slot_id.as_str(), slot);
    }

    for portal in &catalog.portals {
        for lane in &portal.lanes {
            let Some(entry) = slot_by_id.get(lane.entry_spawn_slot_id.as_str()) else {
                return Err(CatalogError::MissingEntrySlot {
                    portal_id: portal.id.clone(),
                    lane_index: lane.lane_index,
                });
            };
            if entry.portal_id != portal.id || entry.lane_index != lane.lane_index {
                return Err(CatalogError::EntrySlotMismatch {
                    portal_id: portal.id.clone(),
                    lane_index: lane.lane_index,
                });
            }
        }
    }
    Ok(())
}

fn require_id(field: &'static str, value: &str) -> Result<(), CatalogError> {
    if value.is_empty() {
        return Err(CatalogError::EmptyId { field });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN: &str =
        include_str!("../../../../examples/data/v0.2-signalized-corridor.catalog.toml");

    fn golden_catalog() -> CorridorCatalog {
        toml::from_str(GOLDEN).expect("checked-in catalog must parse")
    }

    #[test]
    fn checked_in_catalog_is_closed_0_4() {
        validate(&golden_catalog()).expect("checked-in catalog 0.4");
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut catalog = golden_catalog();
        catalog.catalog_version = "0.1".to_owned();
        assert_eq!(
            validate(&catalog),
            Err(CatalogError::UnsupportedVersion("0.1".to_owned()))
        );
        catalog.catalog_version = "0.2".to_owned();
        assert_eq!(
            validate(&catalog),
            Err(CatalogError::UnsupportedVersion("0.2".to_owned()))
        );
    }

    #[test]
    fn policy_selection_is_mandatory_and_closed() {
        for text in [
            "kind = 'missing'",
            "kind = 'pinned'",
            "kind = 'not_required'\npolicy = 'anything'",
            "kind = 'not_required'\nextra = true",
            "kind = 'pinned'\npolicy = 'anything'\nextra = true",
        ] {
            assert!(
                toml::from_str::<CatalogPolicySelection>(text).is_err(),
                "{text}"
            );
        }
        let mut document: toml::Value = toml::from_str(GOLDEN).unwrap();
        document.as_table_mut().unwrap().remove("policy_selection");
        assert!(toml::from_str::<CorridorCatalog>(&toml::to_string(&document).unwrap()).is_err());
        let mut catalog = golden_catalog();
        catalog.catalog_version = "0.3".into();
        assert_eq!(
            validate(&catalog),
            Err(CatalogError::UnsupportedVersion("0.3".into()))
        );
        for identity in [
            "",
            "lfid1_lane-edge_00000000000000000000000000000001",
            "lfid1_right-of-way-policy-set_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ] {
            catalog.policy_selection = CatalogPolicySelection::Pinned {
                policy: identity.into(),
            };
            assert_eq!(
                catalog.policy_selection.resolve(),
                Err(CatalogError::InvalidPolicyIdentity)
            );
        }
        assert_eq!(
            CatalogPolicySelection::NotRequired {}.resolve().unwrap(),
            laneflow_runtime::WorldPolicySelection::NotRequired
        );
    }

    #[test]
    fn rejects_duplicate_slot_and_zero_weight() {
        let mut catalog = golden_catalog();
        catalog.spawn_slots[1].slot_id = catalog.spawn_slots[0].slot_id.clone();
        assert!(matches!(
            validate(&catalog),
            Err(CatalogError::DuplicateSlot(_))
        ));

        let mut catalog = golden_catalog();
        catalog.portals[0].lanes[0].route_choices[0].weight = 0;
        assert!(matches!(
            validate(&catalog),
            Err(CatalogError::ZeroWeight { .. })
        ));
    }

    #[test]
    fn rejects_non_finite_or_negative_progress_and_empty_ids() {
        let mut catalog = golden_catalog();
        catalog.spawn_slots[0].progress = f64::NAN;
        assert!(matches!(
            validate(&catalog),
            Err(CatalogError::InvalidProgress { .. })
        ));

        let mut catalog = golden_catalog();
        catalog.spawn_slots[0].progress = -1.0;
        assert!(matches!(
            validate(&catalog),
            Err(CatalogError::InvalidProgress { .. })
        ));

        let mut catalog = golden_catalog();
        catalog.spawn_slots[0].slot_id.clear();
        assert_eq!(
            validate(&catalog),
            Err(CatalogError::EmptyId { field: "slot_id" })
        );
    }
}
