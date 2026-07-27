//! AccessRule 准入 overlay normalization（SSOT §6/§7/§10 phase 9-10）。
//!
//! 两个求值平面（edge / path）互不展平；组合裁决（参与者 specificity >
//! target specificity > priority）在 normalization 期完全消解为
//! `(edge, class)` 与 `(path, class)` resolved 表（稀疏行物化，见
//! [`AccessPlane`]），绑定期只做 O(1) 查表。

use indexmap::IndexMap;

use crate::{
    cross_section::CrossSectionRegistry,
    error::CoreError,
    graph::LaneGraph,
    handle::{
        AccessRuleHandle, EdgeHandle, FacilityBandHandle, LaneGroupHandle, ManeuverPathHandle,
        ParticipantClassHandle, RoadSectionHandle,
    },
    id::validate_external_id,
    junction::{JunctionRegistry, validate_capacity},
    participant_class::ParticipantClassRegistry,
};

/// AccessRule 的准入效果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessEffect {
    /// 显式放行（平面内豁免，不跨平面解除 deny）。
    Allow,
    /// 显式拒绝。
    Deny,
}

/// AccessRule target 引用（恰好一个，五选一）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessTargetId {
    /// edge 平面：单条 LaneEdge。
    LaneEdge(String),
    /// edge 平面：LaneGroup（经 lane 成员展开为 edge 集合）。
    LaneGroup(String),
    /// edge 平面：RoadSection（经 lane 成员展开为 edge 集合）。
    RoadSection(String),
    /// path 平面：ManeuverPath（不展开为 edge）。
    ManeuverPath(String),
    /// FacilityBand（v1 由 capability guard 结构化拒绝）。
    FacilityBand(String),
}

impl AccessTargetId {
    /// 创建 LaneEdge target。
    pub fn lane_edge(id: impl Into<String>) -> Self {
        Self::LaneEdge(id.into())
    }

    /// 创建 LaneGroup target。
    pub fn lane_group(id: impl Into<String>) -> Self {
        Self::LaneGroup(id.into())
    }

    /// 创建 RoadSection target。
    pub fn road_section(id: impl Into<String>) -> Self {
        Self::RoadSection(id.into())
    }

    /// 创建 ManeuverPath target。
    pub fn maneuver_path(id: impl Into<String>) -> Self {
        Self::ManeuverPath(id.into())
    }

    /// 创建 FacilityBand target。
    pub fn facility_band(id: impl Into<String>) -> Self {
        Self::FacilityBand(id.into())
    }

    /// 返回 target external ID。
    pub fn id(&self) -> &str {
        match self {
            Self::LaneEdge(id)
            | Self::LaneGroup(id)
            | Self::RoadSection(id)
            | Self::ManeuverPath(id)
            | Self::FacilityBand(id) => id,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::LaneEdge(_) => "laneEdge",
            Self::LaneGroup(_) => "laneGroup",
            Self::RoadSection(_) => "roadSection",
            Self::ManeuverPath(_) => "maneuverPath",
            Self::FacilityBand(_) => "facilityBand",
        }
    }
}

/// AccessRule 的法规 provenance（审计字段，v1 不参与计算语义）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessRegulation {
    jurisdiction: String,
    version: String,
    source: Option<String>,
}

impl AccessRegulation {
    /// 创建 regulation provenance。
    ///
    /// 三个字段的长度限定为 1 到 128 字符（与 schema 契约一致）：loader 路径不执行
    /// JSON Schema，provenance 是可审计字段，空串或超长串不得绕过校验进入 Core。
    ///
    /// 经 `AccessRule::with_regulation` 进 registry 的规则不经过本构造函数，
    /// 同一 shape 校验由 `AccessRegistry::try_new` phase 9.5（capability guard
    /// 之后、provenance 单一性之前）统一执行。
    pub fn try_new(
        jurisdiction: impl Into<String>,
        version: impl Into<String>,
        source: Option<&str>,
    ) -> Result<Self, CoreError> {
        let regulation = Self {
            jurisdiction: jurisdiction.into(),
            version: version.into(),
            source: source.map(str::to_owned),
        };
        regulation.validate()?;
        Ok(regulation)
    }

    /// 字段长度 shape 校验（1 到 128 字符，与 schema 契约一致）。
    fn validate(&self) -> Result<(), CoreError> {
        for (field, value) in [
            ("jurisdiction", Some(self.jurisdiction.as_str())),
            ("version", Some(self.version.as_str())),
            ("source", self.source.as_deref()),
        ]
        .into_iter()
        .filter_map(|(field, value)| value.map(|value| (field, value)))
        {
            let len = value.chars().count();
            if !(1..=128).contains(&len) {
                return Err(CoreError::InvalidAccessRegulationString { field, len });
            }
        }
        Ok(())
    }

    /// 返回法域。
    pub fn jurisdiction(&self) -> &str {
        &self.jurisdiction
    }

    /// 返回法规版本。
    pub fn version(&self) -> &str {
        &self.version
    }

    /// 返回可选来源。
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }
}

/// AccessRule 输入定义。
///
/// `has_time_windows` 只是 guard 归因标记：wire 层带 timeWindows 的规则进 Core
/// 时置 `true`，timeWindow 数据本身不进 Core（v1 由 capability guard 一律拒绝）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessRule {
    id: String,
    target: AccessTargetId,
    effect: AccessEffect,
    participant_class_ids: Vec<String>,
    regulation: Option<AccessRegulation>,
    priority: String,
    has_time_windows: bool,
}

impl AccessRule {
    /// 创建 AccessRule。ID 语法、唯一性、target/class 引用、capability guard、
    /// priority/regulation shape 与组合歧义由 `AccessRegistry::try_new` 校验。
    pub fn new<I, S>(
        id: impl Into<String>,
        target: AccessTargetId,
        effect: AccessEffect,
        participant_class_ids: I,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            id: id.into(),
            target,
            effect,
            participant_class_ids: participant_class_ids.into_iter().map(Into::into).collect(),
            regulation: None,
            priority: "0".to_owned(),
            has_time_windows: false,
        }
    }

    /// 设置 regulation provenance。
    ///
    /// 不在此处做 shape 校验（与 `id`/`target`/`participant_class_ids` 一致，
    /// definition 一律存原始输入）：字段长度由 `AccessRegistry::try_new`
    /// phase 9.5 统一校验，保证 capability guard 先于 shape 检查的首错顺序。
    pub fn with_regulation(
        mut self,
        jurisdiction: impl Into<String>,
        version: impl Into<String>,
        source: Option<&str>,
    ) -> Self {
        self.regulation = Some(AccessRegulation {
            jurisdiction: jurisdiction.into(),
            version: version.into(),
            source: source.map(str::to_owned),
        });
        self
    }

    /// 设置显式 priority（缺省 0）。
    ///
    /// 与 regulation 同理不在此处做范围校验，也不受任何整数表示边界约束：
    /// definition 存原始数值字面量，整数性与 i32 范围由
    /// `AccessRegistry::try_new` phase 9.5 统一校验，保证 capability guard
    /// 先于 shape 检查的首错顺序。
    pub fn with_priority(mut self, priority: i64) -> Self {
        self.priority = priority.to_string();
        self
    }

    /// 以原始 JSON 数值字面量设置 priority（供已持有字面量的 normalization
    /// 层使用，使任意大小的数值都能先抵达 capability guard）。
    pub fn with_priority_literal(mut self, literal: impl Into<String>) -> Self {
        self.priority = literal.into();
        self
    }

    /// 设置 timeWindows guard 归因标记。
    pub fn with_time_windows(mut self, has_time_windows: bool) -> Self {
        self.has_time_windows = has_time_windows;
        self
    }

    /// 返回 AccessRule external ID。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回 target 引用。
    pub fn target(&self) -> &AccessTargetId {
        &self.target
    }

    /// 返回准入效果。
    pub const fn effect(&self) -> AccessEffect {
        self.effect
    }

    /// 返回参与者 class external IDs（匹配语义：profile class 是任一成员的传递后代或自身）。
    pub fn participant_class_ids(&self) -> &[String] {
        &self.participant_class_ids
    }

    /// 返回可选 regulation provenance。
    pub fn regulation(&self) -> Option<&AccessRegulation> {
        self.regulation.as_ref()
    }

    /// 返回显式 priority 的原始字面量（经 `AccessRegistry::try_new` 校验后
    /// 保证是 i32 范围内的整数）。
    pub fn priority(&self) -> &str {
        &self.priority
    }

    /// 返回 timeWindows guard 归因标记。
    pub const fn has_time_windows(&self) -> bool {
        self.has_time_windows
    }
}

/// resolved 表单元：`(edge, class)` 或 `(path, class)` 的 normalization 裁决结果。
///
/// 保留胜者 rule handle 供错误归因与审计（有意决策：不为省内存只存 deny bit）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessCell {
    /// 没有任何适用规则：准入 overlay 无约束（不解除其他域约束）。
    Unconstrained,
    /// normalization 裁决出的适用规则与效果。
    Decided {
        /// 胜者 rule（同 effect 并列时保留 input order 先声明者作为归因）。
        rule: AccessRuleHandle,
        /// 裁决效果。
        effect: AccessEffect,
    },
}

/// target specificity 轴（仅 edge 平面；laneEdge > laneGroup > roadSection，SSOT §6.2）。
const TARGET_SPECIFICITY_ROAD_SECTION: u8 = 0;
const TARGET_SPECIFICITY_LANE_GROUP: u8 = 1;
const TARGET_SPECIFICITY_LANE_EDGE: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResolvedAccessTarget {
    LaneEdge(EdgeHandle),
    LaneGroup(LaneGroupHandle),
    RoadSection(RoadSectionHandle),
    ManeuverPath(ManeuverPathHandle),
    // FacilityBand target 在 phase 9.4 被 capability guard 一律拒绝，resolved
    // 形态只用于 unknown 检查与 guard 归因，不进入最终 registry。
    FacilityBand(FacilityBandHandle),
}

impl ResolvedAccessTarget {
    /// edge 平面 target specificity（path 平面无此轴，返回最低值不参与比较）。
    const fn target_specificity(self) -> u8 {
        match self {
            Self::LaneEdge(_) => TARGET_SPECIFICITY_LANE_EDGE,
            Self::LaneGroup(_) => TARGET_SPECIFICITY_LANE_GROUP,
            Self::RoadSection(_) => TARGET_SPECIFICITY_ROAD_SECTION,
            Self::ManeuverPath(_) | Self::FacilityBand(_) => 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedAccessRule {
    definition: AccessRule,
    target: ResolvedAccessTarget,
    classes: Vec<ParticipantClassHandle>,
}

/// 单平面 resolved 表：只为有适用规则的单元物化 class 行。
///
/// 无约束单元在 `row_starts` 中标记哨兵、不占 `cells` 存储也不参与
/// normalization 的 (unit, class) 双重循环：内存与初始化时间以
/// O(受约束单元 × classes) 为界，而非全量 units × classes 笛卡尔积
/// （loader 输入不可信，全量物化会让空规则包也分配 units × classes 个
/// cell）。cell 总数经 checked 乘法 + `validate_capacity` 约束在 u32 范围，
/// 与全部静态域的 handle capacity 同口径。
#[derive(Clone, Debug, PartialEq, Eq)]
struct AccessPlane {
    /// unit index → 行起始 cell index；`UNCONSTRAINED_ROW` 表示无约束单元。
    row_starts: Vec<u32>,
    /// 按 (constrained unit, class) 行优先排列的裁决结果。
    cells: Vec<AccessCell>,
}

impl AccessPlane {
    /// 无约束单元的 `row_starts` 哨兵（合法行起点恒小于 cell 总数 ≤ u32::MAX，
    /// 不会与哨兵冲突）。
    const UNCONSTRAINED_ROW: u32 = u32::MAX;

    fn cell(&self, unit_index: usize, class_index: usize) -> AccessCell {
        let Some(&row_start) = self.row_starts.get(unit_index) else {
            return AccessCell::Unconstrained;
        };
        if row_start == Self::UNCONSTRAINED_ROW {
            return AccessCell::Unconstrained;
        }
        self.cells
            .get(row_start as usize + class_index)
            .copied()
            .unwrap_or(AccessCell::Unconstrained)
    }
}

/// AccessRule immutable normalized registry（SSOT §6/§7）。
///
/// 两个平面的组合裁决在 normalization 期完全消解为 `(edge, class)` 与
/// `(path, class)` resolved 表；查询 O(1)。表按稀疏行物化：只有受规则约束的
/// 单元占 class 行存储，无约束单元经哨兵解析（见 [`AccessPlane`]）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessRegistry {
    rules: Vec<ResolvedAccessRule>,
    rule_handles: IndexMap<String, AccessRuleHandle>,
    edge_plane: AccessPlane,
    path_plane: AccessPlane,
    class_count: usize,
}

impl AccessRegistry {
    /// 创建并校验 AccessRule overlay。
    ///
    /// 校验按 SSOT §10 phase 9 顺序进行，同 phase 内按 input order 返回首错，
    /// 任一错误不发布部分 registry：
    /// 1. rule ID syntax/duplicate；
    /// 2. unknown target（laneEdge -> LaneGraph；laneGroup/roadSection/facilityBand
    ///    -> CrossSectionRegistry；maneuverPath -> JunctionRegistry）；
    /// 3. participantClassIds 非空且每个解析到已声明 class；
    /// 4. capability guard（FacilityBand target 或声明 timeWindows 的规则拒绝载入；
    ///    在 unknown 检查之后、shape/组合检查之前）；
    /// 5. shape（priority i32 范围、regulation 字段长度 1 到 128 字符；guard
    ///    整体拒绝后其内部 shape 校验无意义，故在 guard 之后；同 phase 按
    ///    input order 返回首错）；
    /// 6. regulation provenance 单一性（同一 `(jurisdiction, version)`）；
    /// 7. §6.4 确定性组合裁决（残留 allow/deny 并列拒绝载入）。
    ///
    /// phase 10：构造 resolved 表（稀疏行物化）。空 rules 合法（全部 `Unconstrained`）。
    pub fn try_new(
        lane_graph: &LaneGraph,
        junctions: &JunctionRegistry,
        cross_section: &CrossSectionRegistry,
        classes: &ParticipantClassRegistry,
        rules: Vec<AccessRule>,
    ) -> Result<Self, CoreError> {
        validate_capacity("accessRules", rules.len())?;

        // phase 9.1：rule identity（ID syntax/duplicate）。
        let mut rule_handles = IndexMap::new();
        for (index, rule) in rules.iter().enumerate() {
            validate_external_id("accessRules[].id", rule.id())?;
            if rule_handles.contains_key(rule.id()) {
                return Err(CoreError::DuplicateAccessRuleId {
                    rule_id: rule.id().to_owned(),
                });
            }
            rule_handles.insert(rule.id().to_owned(), AccessRuleHandle::new(index));
        }

        // phase 9.2：target 解析（unknown target）。
        let mut resolved_targets = Vec::with_capacity(rules.len());
        for rule in &rules {
            validate_external_id("accessRules[].target.id", rule.target().id())?;
            let unknown = || CoreError::UnknownAccessRuleTarget {
                rule_id: rule.id().to_owned(),
                target_kind: rule.target().kind(),
                target_id: rule.target().id().to_owned(),
            };
            let target = match rule.target() {
                AccessTargetId::LaneEdge(id) => {
                    ResolvedAccessTarget::LaneEdge(lane_graph.edge_handle(id).ok_or_else(unknown)?)
                }
                AccessTargetId::LaneGroup(id) => ResolvedAccessTarget::LaneGroup(
                    cross_section.group_handle(id).ok_or_else(unknown)?,
                ),
                AccessTargetId::RoadSection(id) => ResolvedAccessTarget::RoadSection(
                    cross_section.section_handle(id).ok_or_else(unknown)?,
                ),
                AccessTargetId::ManeuverPath(id) => ResolvedAccessTarget::ManeuverPath(
                    junctions.maneuver_path_handle(id).ok_or_else(unknown)?,
                ),
                AccessTargetId::FacilityBand(id) => ResolvedAccessTarget::FacilityBand(
                    cross_section.band_handle(id).ok_or_else(unknown)?,
                ),
            };
            resolved_targets.push(target);
        }

        // phase 9.3：participant class 解析（非空 + unknown class）。
        let mut resolved_classes = Vec::with_capacity(rules.len());
        for rule in &rules {
            if rule.participant_class_ids().is_empty() {
                return Err(CoreError::EmptyAccessRuleParticipantClasses {
                    rule_id: rule.id().to_owned(),
                });
            }
            let mut handles = Vec::with_capacity(rule.participant_class_ids().len());
            for class_id in rule.participant_class_ids() {
                validate_external_id("accessRules[].participantClassIds[]", class_id)?;
                handles.push(classes.class_handle(class_id).ok_or_else(|| {
                    CoreError::UnknownAccessRuleParticipantClass {
                        rule_id: rule.id().to_owned(),
                        class_id: class_id.to_owned(),
                    }
                })?);
            }
            resolved_classes.push(handles);
        }

        // phase 9.4：capability guard（unknown 检查之后、shape/组合检查之前）。
        for (rule, target) in rules.iter().zip(&resolved_targets) {
            if matches!(target, ResolvedAccessTarget::FacilityBand(_)) {
                return Err(CoreError::AccessCapabilityUnavailable {
                    rule_id: rule.id().to_owned(),
                    capability: "facilityBandTarget",
                });
            }
            if rule.has_time_windows() {
                return Err(CoreError::AccessCapabilityUnavailable {
                    rule_id: rule.id().to_owned(),
                    capability: "timeWindows",
                });
            }
        }

        // phase 9.5：shape（capability guard 之后、provenance 单一性之前；同一
        // phase 内按 input order 逐规则返回首错——priority 整数性/i32 范围先于
        // regulation 字段长度）。
        let mut resolved_priorities = Vec::with_capacity(rules.len());
        for rule in &rules {
            let Some(priority) = parse_access_priority(rule.priority()) else {
                return Err(CoreError::InvalidAccessRulePriority {
                    priority: rule.priority().to_owned(),
                });
            };
            resolved_priorities.push(priority);
            if let Some(regulation) = rule.regulation() {
                regulation.validate()?;
            }
        }

        // phase 9.6：regulation provenance 单一性（source 可不同；未声明者不参与）。
        let mut canonical: Option<(&str, &str, &str)> = None;
        for rule in &rules {
            let Some(regulation) = rule.regulation() else {
                continue;
            };
            match canonical {
                None => {
                    canonical = Some((rule.id(), regulation.jurisdiction(), regulation.version()));
                }
                Some((first_rule_id, jurisdiction, version)) => {
                    if (regulation.jurisdiction(), regulation.version()) != (jurisdiction, version)
                    {
                        return Err(CoreError::AccessRegulationMismatch {
                            first_rule_id: first_rule_id.to_owned(),
                            jurisdiction: jurisdiction.to_owned(),
                            version: version.to_owned(),
                            duplicate_rule_id: rule.id().to_owned(),
                            duplicate_jurisdiction: regulation.jurisdiction().to_owned(),
                            duplicate_version: regulation.version().to_owned(),
                        });
                    }
                }
            }
        }

        // phase 9.7 + 10：target 引用按 kind 分列 + 组合裁决 + resolved 表。
        // section/group 规则不再展开为逐 edge 的规则索引（rules × section edges
        // 的展开在不可信输入下是平方级中间存储）；edge 平面由
        // resolve_edge_cells 经成员关系反查流式收集候选规则并按签名去重共享行。
        let class_count = classes.class_count();
        let edge_count = lane_graph.edges().len();
        let path_count = junctions.maneuver_paths().len();

        let mut edge_target_rules: Vec<Vec<usize>> = vec![Vec::new(); edge_count];
        let mut group_target_rules: Vec<Vec<usize>> =
            vec![Vec::new(); cross_section.groups().len()];
        let mut section_target_rules: Vec<Vec<usize>> =
            vec![Vec::new(); cross_section.sections().len()];
        let mut path_rule_indices: Vec<Vec<usize>> = vec![Vec::new(); path_count];
        for (rule_index, target) in resolved_targets.iter().enumerate() {
            match target {
                ResolvedAccessTarget::LaneEdge(edge) => {
                    edge_target_rules[edge.index()].push(rule_index);
                }
                ResolvedAccessTarget::RoadSection(section) => {
                    section_target_rules[section.index()].push(rule_index);
                }
                ResolvedAccessTarget::LaneGroup(group) => {
                    group_target_rules[group.index()].push(rule_index);
                }
                ResolvedAccessTarget::ManeuverPath(path) => {
                    path_rule_indices[path.index()].push(rule_index);
                }
                ResolvedAccessTarget::FacilityBand(_) => {
                    unreachable!("phase 9.4 capability guard 已拒绝 FacilityBand target")
                }
            }
        }

        let edge_plane = resolve_edge_cells(
            &rules,
            &resolved_targets,
            &resolved_classes,
            &resolved_priorities,
            classes,
            class_count,
            edge_count,
            cross_section,
            &edge_target_rules,
            &group_target_rules,
            &section_target_rules,
            |edge| {
                lane_graph
                    .edge_external_id(edge)
                    .expect("edge index must belong to lane graph")
                    .to_owned()
            },
        )?;
        let path_plane = resolve_cells(
            &rules,
            &resolved_targets,
            &resolved_classes,
            &resolved_priorities,
            classes,
            class_count,
            (0..path_count).map(ManeuverPathHandle::new),
            &path_rule_indices,
            "path",
            "accessPathCells",
            |path| {
                junctions
                    .maneuver_path_external_id(path)
                    .expect("path index must belong to junction registry")
                    .to_owned()
            },
        )?;

        let rules = rules
            .into_iter()
            .zip(resolved_targets)
            .zip(resolved_classes)
            .map(|((definition, target), classes)| ResolvedAccessRule {
                definition,
                target,
                classes,
            })
            .collect();

        Ok(Self {
            rules,
            rule_handles,
            edge_plane,
            path_plane,
            class_count,
        })
    }

    /// 创建不含任何 AccessRule 的空 registry（全部查询返回 `Unconstrained`）。
    pub fn empty() -> Self {
        Self {
            rules: Vec::new(),
            rule_handles: IndexMap::new(),
            edge_plane: AccessPlane {
                row_starts: Vec::new(),
                cells: Vec::new(),
            },
            path_plane: AccessPlane {
                row_starts: Vec::new(),
                cells: Vec::new(),
            },
            class_count: 0,
        }
    }

    /// 按 retained external definitions 对目标 registry 重新 normalization。
    pub fn rebind(
        &self,
        lane_graph: &LaneGraph,
        junctions: &JunctionRegistry,
        cross_section: &CrossSectionRegistry,
        classes: &ParticipantClassRegistry,
    ) -> Result<Self, CoreError> {
        Self::try_new(
            lane_graph,
            junctions,
            cross_section,
            classes,
            self.rules
                .iter()
                .map(|rule| rule.definition.clone())
                .collect(),
        )
    }

    /// 返回 registry 是否为空（无规则）。
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// 返回规则数量。
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// 返回 AccessRule external ID 对应的 handle。
    pub fn rule_handle(&self, external_id: &str) -> Option<AccessRuleHandle> {
        self.rule_handles.get(external_id).copied()
    }

    /// 返回 AccessRule handle 对应的 external ID。
    pub fn rule_external_id(&self, handle: AccessRuleHandle) -> Option<&str> {
        self.rule(handle).map(AccessRule::id)
    }

    /// 返回指定 AccessRule definition。
    pub fn rule(&self, handle: AccessRuleHandle) -> Option<&AccessRule> {
        self.rules
            .get(handle.index())
            .map(|resolved| &resolved.definition)
    }

    /// 按 normalization order 遍历 AccessRule handles。
    pub fn rules(&self) -> impl ExactSizeIterator<Item = AccessRuleHandle> + '_ {
        (0..self.rules.len()).map(AccessRuleHandle::new)
    }

    /// 返回 `(edge, class)` 的 normalization 裁决结果（O(1) 查表）。
    ///
    /// 任一 handle 不属于本 registry 对应的 LaneGraph/ParticipantClassRegistry
    /// 时返回 `Unconstrained`。
    pub fn edge_access(&self, edge: EdgeHandle, class: ParticipantClassHandle) -> AccessCell {
        self.cell(&self.edge_plane, edge.index(), class)
    }

    /// 返回 `(path, class)` 的 normalization 裁决结果（O(1) 查表）。
    ///
    /// 任一 handle 不属于本 registry 对应的 JunctionRegistry/
    /// ParticipantClassRegistry 时返回 `Unconstrained`。
    pub fn path_access(
        &self,
        path: ManeuverPathHandle,
        class: ParticipantClassHandle,
    ) -> AccessCell {
        self.cell(&self.path_plane, path.index(), class)
    }

    fn cell(
        &self,
        plane: &AccessPlane,
        unit_index: usize,
        class: ParticipantClassHandle,
    ) -> AccessCell {
        if class.index() >= self.class_count {
            return AccessCell::Unconstrained;
        }
        plane.cell(unit_index, class.index())
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        let Self {
            rules,
            rule_handles,
            edge_plane,
            path_plane,
            class_count: _,
        } = self;

        let rule_bytes = rules.capacity() * std::mem::size_of::<ResolvedAccessRule>()
            + rules
                .iter()
                .map(|rule| {
                    let definition = &rule.definition;
                    definition.id.capacity()
                        + match &definition.target {
                            AccessTargetId::LaneEdge(id)
                            | AccessTargetId::LaneGroup(id)
                            | AccessTargetId::RoadSection(id)
                            | AccessTargetId::ManeuverPath(id)
                            | AccessTargetId::FacilityBand(id) => id.capacity(),
                        }
                        + definition.participant_class_ids.capacity()
                            * std::mem::size_of::<String>()
                        + definition
                            .participant_class_ids
                            .iter()
                            .map(String::capacity)
                            .sum::<usize>()
                        + definition.regulation.as_ref().map_or(0, |regulation| {
                            regulation.jurisdiction.capacity()
                                + regulation.version.capacity()
                                + regulation.source.as_ref().map_or(0, String::capacity)
                        })
                        + definition.priority.capacity()
                        + rule.classes.capacity() * std::mem::size_of::<ParticipantClassHandle>()
                })
                .sum::<usize>();
        let resolver_bytes = rule_handles.capacity()
            * std::mem::size_of::<(String, AccessRuleHandle)>()
            + rule_handles.keys().map(String::capacity).sum::<usize>();
        let cell_bytes = [edge_plane, path_plane]
            .into_iter()
            .map(|plane| {
                plane.row_starts.capacity() * std::mem::size_of::<u32>()
                    + plane.cells.capacity() * std::mem::size_of::<AccessCell>()
            })
            .sum::<usize>();

        rule_bytes + resolver_bytes + cell_bytes
    }
}

/// 校验 priority 字面量：必须是 i32 范围内的整数（JSON Schema `integer` 语义，
/// 允许零小数/指数表示如 `100.0`/`1e2`）。按十进制字面值精确判定，不做浮点
/// 归一化：`1.00000000000000001` 这类 f64 不可精确表示的小数必须拒绝。
/// 输入由 wire 层保证是合法 JSON number 字面量。
fn parse_access_priority(literal: &str) -> Option<i32> {
    // 快速路径：纯整数字面量。
    if let Ok(value) = literal.parse::<i64>() {
        return i32::try_from(value).ok();
    }
    if let Ok(value) = literal.parse::<u64>() {
        return i32::try_from(value).ok();
    }
    i32::try_from(exact_integer_lexeme(literal)?).ok()
}

/// 若 JSON number 字面量精确表示一个整数则返回其 i128 值，否则 None。
/// 小数位与指数按十进制字面值运算，不经过任何浮点转换。
fn exact_integer_lexeme(literal: &str) -> Option<i128> {
    let (mantissa, exponent) = match literal.find(['e', 'E']) {
        Some(index) => (&literal[..index], literal[index + 1..].parse::<i64>().ok()?),
        None => (literal, 0),
    };
    let (negative, mantissa) = match mantissa.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, mantissa),
    };
    let (integer_digits, fraction_digits) = match mantissa.find('.') {
        Some(index) => (&mantissa[..index], &mantissa[index + 1..]),
        None => (mantissa, ""),
    };
    let digits: String = integer_digits
        .chars()
        .chain(fraction_digits.chars())
        .collect();
    // 0 的任何表示（0e5、0.000、-0.0）都是整数 0。
    if digits.bytes().all(|digit| digit == b'0') {
        return Some(0);
    }
    let fraction_len = i64::try_from(fraction_digits.len()).ok()?;
    let shift = exponent.checked_sub(fraction_len)?;
    let canonical = if shift >= 0 {
        // 整数值 = digits * 10^shift。i128 至多 39 位十进制数字，超长必越界。
        // 输入是不可信 JSON：极端正指数（如 `1e9223372036854775807`）先按上界
        // 拒绝，之后 shift ∈ [0, 39]，usize 转换与加法均不可能溢出。
        if shift > 39 || digits.len() + shift as usize > 39 {
            return None;
        }
        let mut canonical = digits;
        canonical.extend(std::iter::repeat_n('0', shift as usize));
        canonical
    } else {
        // 精确为整数当且仅当被小数点截去的 -shift 位全为 0。
        // `i64::MIN` 取负会溢出；那种量级的负指数本就不可能精确为整数，
        // checked_neg/try_from 失败即按 None 拒绝，保持函数对任意输入全无 panic。
        let keep = digits
            .len()
            .checked_sub(usize::try_from(shift.checked_neg()?).ok()?)?;
        if digits[keep..].bytes().any(|digit| digit != b'0') {
            return None;
        }
        digits[..keep].to_owned()
    };
    let magnitude: i128 = canonical.parse().ok()?;
    Some(if negative { -magnitude } else { magnitude })
}

/// 对一个平面逐单元做 §6.4 裁决并填充稀疏行 resolved 表（path 平面：
/// target 与单元一一对应，单元候选规则集互不相同，无需签名去重）。
///
/// 只为有适用规则的单元物化 class 行（[`AccessPlane`] 文档）：cell 总数经
/// checked 乘法 + validate_capacity 约束在 u32 范围，不可信输入无法让
/// 分配量或初始化时间随 units × classes 全量笛卡尔积爆炸。
#[expect(
    clippy::too_many_arguments,
    reason = "normalization 期一次性裁决需要全部上下文"
)]
fn resolve_cells<H>(
    rules: &[AccessRule],
    resolved_targets: &[ResolvedAccessTarget],
    resolved_classes: &[Vec<ParticipantClassHandle>],
    resolved_priorities: &[i32],
    classes: &ParticipantClassRegistry,
    class_count: usize,
    units: impl Iterator<Item = H>,
    unit_rule_indices: &[Vec<usize>],
    plane: &'static str,
    capacity_domain: &'static str,
    unit_external_id: impl Fn(H) -> String,
) -> Result<AccessPlane, CoreError>
where
    H: Copy,
{
    let constrained_count = unit_rule_indices
        .iter()
        .filter(|indices| !indices.is_empty())
        .count();
    let cell_count = constrained_count.checked_mul(class_count).ok_or(
        CoreError::StaticDomainCapacityExceeded {
            domain: capacity_domain,
            count: usize::MAX,
            max_inclusive: u32::MAX,
        },
    )?;
    validate_capacity(capacity_domain, cell_count)?;

    let mut row_starts = vec![AccessPlane::UNCONSTRAINED_ROW; unit_rule_indices.len()];
    let mut cells = vec![AccessCell::Unconstrained; cell_count];
    let mut contenders: Vec<usize> = Vec::new();
    let mut next_row_start: usize = 0;
    for (unit_index, unit) in units.enumerate() {
        if unit_rule_indices[unit_index].is_empty() {
            continue;
        }
        let row_start = next_row_start;
        next_row_start += class_count;
        row_starts[unit_index] =
            u32::try_from(row_start).expect("cell_count 已经 validate_capacity 约束在 u32 范围");
        resolve_unit_row(
            rules,
            resolved_targets,
            resolved_classes,
            resolved_priorities,
            classes,
            &unit_rule_indices[unit_index],
            plane,
            || unit_external_id(unit),
            &mut cells[row_start..row_start + class_count],
            &mut contenders,
        )?;
    }
    Ok(AccessPlane { row_starts, cells })
}

/// edge 平面 resolved 表构造：经 lane 成员关系反查流式收集每条 edge 的候选
/// 规则（laneEdge 直接规则 + 所属 section 规则 + 所属 lane 的 group 规则），
/// 按候选集合签名去重共享行——同一 section/group 覆盖下的同构 edge 只裁决
/// 一次。中间存储 O(rules + edges + group 成员数)，不物化 rules × edges 的
/// 平方级展开（不可信输入下该展开会先耗尽内存）。
#[expect(
    clippy::too_many_arguments,
    reason = "normalization 期一次性裁决需要全部上下文"
)]
fn resolve_edge_cells(
    rules: &[AccessRule],
    resolved_targets: &[ResolvedAccessTarget],
    resolved_classes: &[Vec<ParticipantClassHandle>],
    resolved_priorities: &[i32],
    classes: &ParticipantClassRegistry,
    class_count: usize,
    edge_count: usize,
    cross_section: &CrossSectionRegistry,
    edge_target_rules: &[Vec<usize>],
    group_target_rules: &[Vec<usize>],
    section_target_rules: &[Vec<usize>],
    edge_external_id: impl Fn(EdgeHandle) -> String,
) -> Result<AccessPlane, CoreError> {
    // lane → 包含它且有规则的 group（只遍历 group 成员关系本身）。
    let mut lane_groups: IndexMap<(usize, usize), Vec<usize>> = IndexMap::new();
    for (group_index, group) in cross_section.groups().enumerate() {
        if group_target_rules[group_index].is_empty() {
            continue;
        }
        let section = cross_section
            .lane_group_section(group)
            .expect("resolved group must have section");
        for lane_index in cross_section
            .group_lanes(group)
            .expect("resolved group must have member lanes")
        {
            lane_groups
                .entry((section.index(), lane_index))
                .or_default()
                .push(group_index);
        }
    }

    let mut candidates: Vec<usize> = Vec::new();
    let mut signature_rows: IndexMap<Box<[usize]>, u32> = IndexMap::new();
    let mut row_starts: Vec<u32> = Vec::with_capacity(edge_count);
    let mut cells: Vec<AccessCell> = Vec::new();
    let mut contenders: Vec<usize> = Vec::new();
    for edge_index in 0..edge_count {
        candidates.clear();
        candidates.extend_from_slice(&edge_target_rules[edge_index]);
        if let Some((section, lane_index)) =
            cross_section.edge_lane_membership(EdgeHandle::new(edge_index))
        {
            candidates.extend_from_slice(&section_target_rules[section.index()]);
            if let Some(groups) = lane_groups.get(&(section.index(), lane_index)) {
                for &group_index in groups {
                    candidates.extend_from_slice(&group_target_rules[group_index]);
                }
            }
        }
        if candidates.is_empty() {
            row_starts.push(AccessPlane::UNCONSTRAINED_ROW);
            continue;
        }
        // 排序使签名与裁决扫描序同为 rule 声明序（并列归因保持先声明者）。
        candidates.sort_unstable();
        if let Some(&row_start) = signature_rows.get(candidates.as_slice()) {
            row_starts.push(row_start);
            continue;
        }
        // 每新增一行 class_count 个 cell；总行数与 path 平面同口径约束。
        let row_start = cells.len();
        let cell_count = (signature_rows.len() + 1).checked_mul(class_count).ok_or(
            CoreError::StaticDomainCapacityExceeded {
                domain: "accessEdgeCells",
                count: usize::MAX,
                max_inclusive: u32::MAX,
            },
        )?;
        validate_capacity("accessEdgeCells", cell_count)?;
        cells.resize(row_start + class_count, AccessCell::Unconstrained);
        resolve_unit_row(
            rules,
            resolved_targets,
            resolved_classes,
            resolved_priorities,
            classes,
            &candidates,
            "edge",
            || edge_external_id(EdgeHandle::new(edge_index)),
            &mut cells[row_start..row_start + class_count],
            &mut contenders,
        )?;
        let row_start =
            u32::try_from(row_start).expect("cell_count 已经 validate_capacity 约束在 u32 范围");
        signature_rows.insert(candidates.as_slice().into(), row_start);
        row_starts.push(row_start);
    }
    Ok(AccessPlane { row_starts, cells })
}

/// 对单个单元的候选规则集合按 §6.4 字典序裁决每个 class 的胜者并填充
/// `cells_row`（len = class_count）。
///
/// 字典序：①参与者 specificity（使匹配成功的最深 class 深度）②target
/// specificity（仅 edge 平面）③priority 数值高者。裁决先求最大 key 的
/// contenders 集合：低 key 规则之间的 allow/deny 并列不构成歧义（它们已被
/// 更高 key 的规则整体击败）；只有最大 key contenders 内部仍 effect 混合才
/// 返回 `CoreError::AccessRuleAmbiguity`。同 effect 并列合法，保留 input
/// order 先声明者作为归因。结果不随输入排列漂移。
#[expect(
    clippy::too_many_arguments,
    reason = "normalization 期一次性裁决需要全部上下文"
)]
fn resolve_unit_row(
    rules: &[AccessRule],
    resolved_targets: &[ResolvedAccessTarget],
    resolved_classes: &[Vec<ParticipantClassHandle>],
    resolved_priorities: &[i32],
    classes: &ParticipantClassRegistry,
    candidate_rules: &[usize],
    plane: &'static str,
    unit_external_id: impl Fn() -> String,
    cells_row: &mut [AccessCell],
    contenders: &mut Vec<usize>,
) -> Result<(), CoreError> {
    for (class_index, cell) in cells_row.iter_mut().enumerate() {
        let profile_class = ParticipantClassHandle::new(class_index);
        // 第一遍：计算每条适用规则的 (depth, target specificity, priority)
        // key，收集最大 key 的 contenders（保持 input order）。
        let mut best_key: Option<(u32, u8, i32)> = None;
        contenders.clear();
        for &rule_index in candidate_rules {
            // 参与者匹配：profile class 是规则任一 participantClassIds 成员的
            // 传递后代或自身；specificity 取使匹配成功的最深 class 深度。
            let mut depth: Option<u32> = None;
            for class_handle in &resolved_classes[rule_index] {
                if classes.is_descendant_or_self(profile_class, *class_handle) {
                    let class_depth = classes
                        .depth(*class_handle)
                        .expect("resolved class must have depth");
                    depth = Some(depth.map_or(class_depth, |best| best.max(class_depth)));
                }
            }
            let Some(depth) = depth else {
                continue;
            };
            let key = (
                depth,
                resolved_targets[rule_index].target_specificity(),
                resolved_priorities[rule_index],
            );
            match best_key {
                Some(best) if key < best => {}
                Some(best) if key == best => contenders.push(rule_index),
                _ => {
                    best_key = Some(key);
                    contenders.clear();
                    contenders.push(rule_index);
                }
            }
        }
        // 第二遍：只检查最大 key contenders 内部的 effect 一致性。
        if let Some(&first) = contenders.first() {
            let effect = rules[first].effect();
            if let Some(&opposite) = contenders
                .iter()
                .skip(1)
                .find(|&&rule_index| rules[rule_index].effect() != effect)
            {
                return Err(CoreError::AccessRuleAmbiguity {
                    plane,
                    target_id: unit_external_id(),
                    class_id: classes
                        .class_external_id(profile_class)
                        .expect("class index must belong to class registry")
                        .to_owned(),
                    first_rule_id: rules[first].id().to_owned(),
                    second_rule_id: rules[opposite].id().to_owned(),
                });
            }
            // 同 effect 并列合法：保留先声明者作为归因。
            *cell = AccessCell::Decided {
                rule: AccessRuleHandle::new(first),
                effect,
            };
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cross_section::{
            CorridorElementId, FacilityBand, LaneGroup, RoadCorridor, RoadSection, SectionLane,
        },
        graph::{EdgeLength, LaneEdge, SpeedLimit},
        participant_class::ParticipantClass,
    };

    fn test_lane_graph() -> LaneGraph {
        LaneGraph::try_new([LaneEdge::new(
            "edge-1",
            EdgeLength::try_new(10.0).expect("test edge length"),
            SpeedLimit::try_new(10.0).expect("test speed limit"),
            Vec::<String>::new(),
        )])
        .expect("test graph")
    }

    #[test]
    fn retained_bytes_tracks_declared_rules() {
        let graph = test_lane_graph();
        let junctions = JunctionRegistry::empty();
        let cross_section = CrossSectionRegistry::try_new(
            &graph,
            Vec::<FacilityBand>::new(),
            Vec::<RoadSection>::new(),
            Vec::<LaneGroup>::new(),
            Vec::<RoadCorridor>::new(),
        )
        .expect("empty cross-section is valid");
        let classes =
            ParticipantClassRegistry::try_new(vec![ParticipantClass::new("motorVehicle", None)])
                .expect("test classes");

        // 零基线：空 graph + 空 class + 空规则，两张 resolved 表均为空。
        let empty_graph = LaneGraph::empty();
        let empty_cross_section = CrossSectionRegistry::try_new(
            &empty_graph,
            Vec::<FacilityBand>::new(),
            Vec::<RoadSection>::new(),
            Vec::<LaneGroup>::new(),
            Vec::<RoadCorridor>::new(),
        )
        .expect("empty cross-section is valid");
        let empty = AccessRegistry::try_new(
            &empty_graph,
            &junctions,
            &empty_cross_section,
            &ParticipantClassRegistry::empty(),
            Vec::new(),
        )
        .expect("empty rules are valid");
        let registry = AccessRegistry::try_new(
            &graph,
            &junctions,
            &cross_section,
            &classes,
            vec![AccessRule::new(
                "rule-1",
                AccessTargetId::lane_edge("edge-1"),
                AccessEffect::Deny,
                ["motorVehicle"],
            )],
        )
        .expect("valid access registry");

        assert_eq!(empty.retained_bytes(), 0);
        assert!(registry.retained_bytes() > 0);
    }

    #[test]
    fn resolved_tables_materialize_only_constrained_rows() {
        let graph = LaneGraph::try_new([
            LaneEdge::new(
                "edge-1",
                EdgeLength::try_new(10.0).expect("test edge length"),
                SpeedLimit::try_new(10.0).expect("test speed limit"),
                Vec::<String>::new(),
            ),
            LaneEdge::new(
                "edge-2",
                EdgeLength::try_new(10.0).expect("test edge length"),
                SpeedLimit::try_new(10.0).expect("test speed limit"),
                Vec::<String>::new(),
            ),
        ])
        .expect("test graph");
        let junctions = JunctionRegistry::empty();
        let cross_section = CrossSectionRegistry::try_new(
            &graph,
            Vec::<FacilityBand>::new(),
            Vec::<RoadSection>::new(),
            Vec::<LaneGroup>::new(),
            Vec::<RoadCorridor>::new(),
        )
        .expect("empty cross-section is valid");
        let classes =
            ParticipantClassRegistry::try_new(vec![ParticipantClass::new("motorVehicle", None)])
                .expect("test classes");
        let registry = AccessRegistry::try_new(
            &graph,
            &junctions,
            &cross_section,
            &classes,
            vec![AccessRule::new(
                "rule-1",
                AccessTargetId::lane_edge("edge-1"),
                AccessEffect::Deny,
                ["motorVehicle"],
            )],
        )
        .expect("valid access registry");

        // 只有 edge-1 受约束：cells 恰物化一行，edge-2 经哨兵解析，不占存储。
        assert_eq!(registry.edge_plane.cells.len(), 1);
        assert_eq!(
            registry.edge_plane.row_starts,
            vec![0, AccessPlane::UNCONSTRAINED_ROW]
        );
        let class = classes.class_handle("motorVehicle").expect("test class");
        assert!(matches!(
            registry.edge_access(EdgeHandle::new(0), class),
            AccessCell::Decided {
                effect: AccessEffect::Deny,
                ..
            }
        ));
        assert!(matches!(
            registry.edge_access(EdgeHandle::new(1), class),
            AccessCell::Unconstrained
        ));
    }

    #[test]
    fn resolved_table_capacity_is_validated_before_allocation() {
        // constrained × classes 超 u32 范围必须按静态域容量拒绝（checked 乘法 +
        // validate_capacity），而不是分配失败 abort；两个非空单元即触发，
        // 无需真实构造超大 graph/class registry。
        let unit_rule_indices = vec![vec![0], vec![0]];
        let error = resolve_cells(
            &[],
            &[],
            &[],
            &[],
            &ParticipantClassRegistry::empty(),
            (u32::MAX as usize) / 2 + 1,
            (0..2).map(EdgeHandle::new),
            &unit_rule_indices,
            "edge",
            "accessEdgeCells",
            |_| "unit".to_owned(),
        )
        .expect_err("units × classes 超 u32 范围必须按容量拒绝");
        assert!(matches!(
            error,
            CoreError::StaticDomainCapacityExceeded {
                domain: "accessEdgeCells",
                ..
            }
        ));
    }

    #[test]
    fn broad_target_rules_share_resolution_rows() {
        // 两条 edge 同属于一个 section 的不同 lane：同一 section 规则对两条
        // edge 的候选集合相同，签名去重后只物化一行（broad target 不做
        // rules × edges 展开）；edge-2 额外有直接规则时候选集不同，另起一行。
        let graph = LaneGraph::try_new([
            LaneEdge::new(
                "edge-1",
                EdgeLength::try_new(10.0).expect("test edge length"),
                SpeedLimit::try_new(10.0).expect("test speed limit"),
                Vec::<String>::new(),
            ),
            LaneEdge::new(
                "edge-2",
                EdgeLength::try_new(10.0).expect("test edge length"),
                SpeedLimit::try_new(10.0).expect("test speed limit"),
                Vec::<String>::new(),
            ),
        ])
        .expect("test graph");
        let junctions = JunctionRegistry::empty();
        let cross_section = CrossSectionRegistry::try_new(
            &graph,
            Vec::<FacilityBand>::new(),
            [RoadSection::new(
                "section-a",
                "motorLane",
                [
                    SectionLane::new(["edge-1"], None),
                    SectionLane::new(["edge-2"], None),
                ],
            )],
            Vec::<LaneGroup>::new(),
            [RoadCorridor::new(
                "corridor-1",
                "section-a",
                [CorridorElementId::section("section-a")],
            )],
        )
        .expect("test cross-section");
        let classes =
            ParticipantClassRegistry::try_new(vec![ParticipantClass::new("motorVehicle", None)])
                .expect("test classes");
        let class = classes.class_handle("motorVehicle").expect("test class");

        let section_rules = || {
            vec![
                AccessRule::new(
                    "rule-section-1",
                    AccessTargetId::road_section("section-a"),
                    AccessEffect::Deny,
                    ["motorVehicle"],
                ),
                AccessRule::new(
                    "rule-section-2",
                    AccessTargetId::road_section("section-a"),
                    AccessEffect::Deny,
                    ["motorVehicle"],
                ),
            ]
        };

        // 只有 section 规则：两条 edge 共享同一签名，恰物化一行且裁决一致。
        let shared = AccessRegistry::try_new(
            &graph,
            &junctions,
            &cross_section,
            &classes,
            section_rules(),
        )
        .expect("valid access registry");
        assert_eq!(shared.edge_plane.cells.len(), 1);
        assert_eq!(shared.edge_plane.row_starts, vec![0, 0]);
        for edge_index in 0..2 {
            assert!(matches!(
                shared.edge_access(EdgeHandle::new(edge_index), class),
                AccessCell::Decided {
                    effect: AccessEffect::Deny,
                    ..
                }
            ));
        }

        // edge-2 追加直接规则后候选集不同：两行，各自裁决正确。
        let mut rules = section_rules();
        rules.push(AccessRule::new(
            "rule-edge-2",
            AccessTargetId::lane_edge("edge-2"),
            AccessEffect::Allow,
            ["motorVehicle"],
        ));
        let distinct = AccessRegistry::try_new(&graph, &junctions, &cross_section, &classes, rules)
            .expect("valid access registry");
        assert_eq!(distinct.edge_plane.cells.len(), 2);
        assert_eq!(distinct.edge_plane.row_starts, vec![0, 1]);
        // laneEdge target specificity 高于 roadSection：edge-2 由直接规则 Allow 胜出。
        assert!(matches!(
            distinct.edge_access(EdgeHandle::new(1), class),
            AccessCell::Decided {
                effect: AccessEffect::Allow,
                ..
            }
        ));
    }
}
