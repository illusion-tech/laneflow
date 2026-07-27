//! AccessRule 准入 overlay normalization（SSOT §6/§7/§10 phase 9-10）。
//!
//! 两个求值平面（edge / path）互不展平；组合裁决（参与者 specificity >
//! target specificity > priority）在 normalization 期完全消解为
//! `(edge, class)` 与 `(path, class)` 的 dense resolved 表，绑定期只做 O(1) 查表。

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
    pub fn try_new(
        jurisdiction: impl Into<String>,
        version: impl Into<String>,
        source: Option<&str>,
    ) -> Result<Self, CoreError> {
        let jurisdiction = jurisdiction.into();
        let version = version.into();
        let source = source.map(str::to_owned);
        for (field, value) in [
            ("jurisdiction", Some(jurisdiction.as_str())),
            ("version", Some(version.as_str())),
            ("source", source.as_deref()),
        ]
        .into_iter()
        .filter_map(|(field, value)| value.map(|value| (field, value)))
        {
            let len = value.chars().count();
            if !(1..=128).contains(&len) {
                return Err(CoreError::InvalidAccessRegulationString { field, len });
            }
        }
        Ok(Self {
            jurisdiction,
            version,
            source,
        })
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
    priority: i32,
    has_time_windows: bool,
}

impl AccessRule {
    /// 创建 AccessRule。ID 语法、唯一性、target/class 引用、capability guard
    /// 与组合歧义由 `AccessRegistry::try_new` 校验。
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
            priority: 0,
            has_time_windows: false,
        }
    }

    /// 设置 regulation provenance。
    pub fn with_regulation(mut self, regulation: AccessRegulation) -> Self {
        self.regulation = Some(regulation);
        self
    }

    /// 设置显式 priority（缺省 0）。
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
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

    /// 返回显式 priority。
    pub const fn priority(&self) -> i32 {
        self.priority
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

/// AccessRule immutable normalized registry（SSOT §6/§7）。
///
/// 两个平面的组合裁决在 normalization 期完全消解：edge 平面为
/// `edges.len() × class_count` 行优先 dense 表，path 平面为
/// `paths × class_count` 行优先 dense 表；查询 O(1)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessRegistry {
    rules: Vec<ResolvedAccessRule>,
    rule_handles: IndexMap<String, AccessRuleHandle>,
    edge_cells: Vec<AccessCell>,
    path_cells: Vec<AccessCell>,
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
    /// 5. regulation provenance 单一性（同一 `(jurisdiction, version)`）；
    /// 6. §6.4 确定性组合裁决（残留 allow/deny 并列拒绝载入）。
    ///
    /// phase 10：构造 dense resolved 表。空 rules 合法（全部 `Unconstrained`）。
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

        // phase 9.5：regulation provenance 单一性（source 可不同；未声明者不参与）。
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

        // phase 9.6 + 10：target 展开 + 组合裁决 + dense resolved 表。
        let class_count = classes.class_count();
        let edge_count = lane_graph.edges().len();
        let path_count = junctions.maneuver_paths().len();

        let mut edge_rule_indices: Vec<Vec<usize>> = vec![Vec::new(); edge_count];
        let mut path_rule_indices: Vec<Vec<usize>> = vec![Vec::new(); path_count];
        for (rule_index, target) in resolved_targets.iter().enumerate() {
            match target {
                ResolvedAccessTarget::LaneEdge(edge) => {
                    edge_rule_indices[edge.index()].push(rule_index);
                }
                ResolvedAccessTarget::RoadSection(section) => {
                    let lanes = cross_section
                        .section_lanes(*section)
                        .expect("resolved section must have lanes");
                    for (_, edges) in lanes {
                        for edge in edges {
                            edge_rule_indices[edge.index()].push(rule_index);
                        }
                    }
                }
                ResolvedAccessTarget::LaneGroup(group) => {
                    let section = cross_section
                        .lane_group_section(*group)
                        .expect("resolved group must have section");
                    let lanes: Vec<&[EdgeHandle]> = cross_section
                        .section_lanes(section)
                        .expect("resolved section must have lanes")
                        .map(|(_, edges)| edges)
                        .collect();
                    let members = cross_section
                        .group_lanes(*group)
                        .expect("resolved group must have member lanes");
                    for lane_index in members {
                        for edge in lanes[lane_index] {
                            edge_rule_indices[edge.index()].push(rule_index);
                        }
                    }
                }
                ResolvedAccessTarget::ManeuverPath(path) => {
                    path_rule_indices[path.index()].push(rule_index);
                }
                ResolvedAccessTarget::FacilityBand(_) => {
                    unreachable!("phase 9.4 capability guard 已拒绝 FacilityBand target")
                }
            }
        }

        let edge_cells = resolve_cells(
            &rules,
            &resolved_targets,
            &resolved_classes,
            classes,
            class_count,
            (0..edge_count).map(EdgeHandle::new),
            &edge_rule_indices,
            "edge",
            |edge| {
                lane_graph
                    .edge_external_id(edge)
                    .expect("edge index must belong to lane graph")
                    .to_owned()
            },
        )?;
        let path_cells = resolve_cells(
            &rules,
            &resolved_targets,
            &resolved_classes,
            classes,
            class_count,
            (0..path_count).map(ManeuverPathHandle::new),
            &path_rule_indices,
            "path",
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
            edge_cells,
            path_cells,
            class_count,
        })
    }

    /// 创建不含任何 AccessRule 的空 registry（全部查询返回 `Unconstrained`）。
    pub fn empty() -> Self {
        Self {
            rules: Vec::new(),
            rule_handles: IndexMap::new(),
            edge_cells: Vec::new(),
            path_cells: Vec::new(),
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
        self.cell(&self.edge_cells, edge.index(), class)
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
        self.cell(&self.path_cells, path.index(), class)
    }

    fn cell(
        &self,
        cells: &[AccessCell],
        unit_index: usize,
        class: ParticipantClassHandle,
    ) -> AccessCell {
        if class.index() >= self.class_count {
            return AccessCell::Unconstrained;
        }
        let Some(cell_index) = unit_index
            .checked_mul(self.class_count)
            .and_then(|row| row.checked_add(class.index()))
        else {
            return AccessCell::Unconstrained;
        };
        cells
            .get(cell_index)
            .copied()
            .unwrap_or(AccessCell::Unconstrained)
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        let Self {
            rules,
            rule_handles,
            edge_cells,
            path_cells,
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
                        + rule.classes.capacity() * std::mem::size_of::<ParticipantClassHandle>()
                })
                .sum::<usize>();
        let resolver_bytes = rule_handles.capacity()
            * std::mem::size_of::<(String, AccessRuleHandle)>()
            + rule_handles.keys().map(String::capacity).sum::<usize>();
        let cell_bytes = edge_cells.capacity() * std::mem::size_of::<AccessCell>()
            + path_cells.capacity() * std::mem::size_of::<AccessCell>();

        rule_bytes + resolver_bytes + cell_bytes
    }
}

/// 对一个平面的全部 `(unit, class)` 组合做 §6.4 字典序裁决并填充 dense 表。
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
fn resolve_cells<H>(
    rules: &[AccessRule],
    resolved_targets: &[ResolvedAccessTarget],
    resolved_classes: &[Vec<ParticipantClassHandle>],
    classes: &ParticipantClassRegistry,
    class_count: usize,
    units: impl Iterator<Item = H>,
    unit_rule_indices: &[Vec<usize>],
    plane: &'static str,
    unit_external_id: impl Fn(H) -> String,
) -> Result<Vec<AccessCell>, CoreError>
where
    H: Copy,
{
    let mut cells = vec![AccessCell::Unconstrained; unit_rule_indices.len() * class_count];
    let mut contenders: Vec<usize> = Vec::new();
    for (unit_index, unit) in units.enumerate() {
        for class_index in 0..class_count {
            let profile_class = ParticipantClassHandle::new(class_index);
            // 第一遍：计算每条适用规则的 (depth, target specificity, priority)
            // key，收集最大 key 的 contenders（保持 input order）。
            let mut best_key: Option<(u32, u8, i32)> = None;
            contenders.clear();
            for &rule_index in &unit_rule_indices[unit_index] {
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
                    rules[rule_index].priority(),
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
                        target_id: unit_external_id(unit),
                        class_id: classes
                            .class_external_id(profile_class)
                            .expect("class index must belong to class registry")
                            .to_owned(),
                        first_rule_id: rules[first].id().to_owned(),
                        second_rule_id: rules[opposite].id().to_owned(),
                    });
                }
                // 同 effect 并列合法：保留先声明者作为归因。
                cells[unit_index * class_count + class_index] = AccessCell::Decided {
                    rule: AccessRuleHandle::new(first),
                    effect,
                };
            }
        }
    }
    Ok(cells)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cross_section::{FacilityBand, LaneGroup, RoadCorridor, RoadSection},
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
}
