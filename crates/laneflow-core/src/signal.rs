//! v0.4 Signals 静态领域、normalization registry 与 resolver。

use std::ops::Range;

use indexmap::{IndexMap, IndexSet};

use crate::{
    error::CoreError,
    graph::LaneGraph,
    handle::{
        EdgeHandle, ManeuverGateHandle, ManeuverPathHandle, SignalControllerHandle,
        SignalGroupHandle, SignalPhaseRef, StopLineHandle,
    },
    id::validate_external_id,
    junction::JunctionRegistry,
};

/// JSON interoperable integer 能无损表达的最大 signal scheduling 毫秒值（`2^53 - 1`）。
pub const MAX_PORTABLE_SIGNAL_TIME_MS: u64 = 9_007_199_254_740_991;

/// v0.4 StopLine 位置语义。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StopLineLocation {
    /// StopLine 位于 edge 的出口边界。
    EdgeEnd,
}

/// immutable StopLine 输入定义。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopLine {
    id: String,
    edge_id: String,
    location: StopLineLocation,
}

impl StopLine {
    /// 创建待由 `SignalRegistry` normalization 的 StopLine definition。
    pub fn new(
        id: impl Into<String>,
        edge_id: impl Into<String>,
        location: StopLineLocation,
    ) -> Self {
        Self {
            id: id.into(),
            edge_id: edge_id.into(),
            location,
        }
    }

    /// 返回 StopLine external ID。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回 StopLine 所属 edge external ID。
    pub fn edge_id(&self) -> &str {
        &self.edge_id
    }

    /// 返回 StopLine 的逻辑位置。
    pub const fn location(&self) -> StopLineLocation {
        self.location
    }
}

/// immutable SignalGroup 输入定义。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalGroup {
    id: String,
}

impl SignalGroup {
    /// 创建待由 `SignalRegistry` normalization 的 SignalGroup definition。
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }

    /// 返回 SignalGroup external ID。
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// v0.4 SignalGroup indication 闭集。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SignalAspect {
    /// 禁止进入受控 ManeuverGate。
    Red,
    /// v0.4 保守策略下禁止进入受控 ManeuverGate。
    Yellow,
    /// 允许进入受控 ManeuverGate，但不代表最终 right-of-way。
    Green,
}

/// Phase 内单个 SignalGroup 的完整 state record。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalGroupState {
    group_id: String,
    aspect: SignalAspect,
}

impl SignalGroupState {
    /// 创建待由 `SignalRegistry` normalization 的 state record。
    pub fn new(group_id: impl Into<String>, aspect: SignalAspect) -> Self {
        Self {
            group_id: group_id.into(),
            aspect,
        }
    }

    /// 返回 state 引用的 group external ID。
    pub fn group_id(&self) -> &str {
        &self.group_id
    }

    /// 返回 indication。
    pub const fn aspect(&self) -> SignalAspect {
        self.aspect
    }
}

/// controller-local immutable Phase definition。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalPhase {
    id: String,
    duration_ms: u64,
    states: Vec<SignalGroupState>,
}

impl SignalPhase {
    /// 创建待由 `SignalRegistry` normalization 的 phase definition。
    pub fn new<I>(id: impl Into<String>, duration_ms: u64, states: I) -> Self
    where
        I: IntoIterator<Item = SignalGroupState>,
    {
        Self {
            id: id.into(),
            duration_ms,
            states: states.into_iter().collect(),
        }
    }

    /// 返回 controller-local phase external ID。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回 phase duration，单位毫秒。
    pub const fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    /// 返回输入 state records。
    pub fn states(&self) -> &[SignalGroupState] {
        &self.states
    }
}

/// v0.4 controller 类型；当前只支持 fixed-time。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SignalControllerKind {
    FixedTime,
}

/// immutable fixed-time SignalController definition。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalController {
    id: String,
    kind: SignalControllerKind,
    offset_ms: u64,
    group_ids: Vec<String>,
    phases: Vec<SignalPhase>,
}

impl SignalController {
    /// 创建待由 `SignalRegistry` normalization 的 fixed-time controller definition。
    pub fn new_fixed_time<I, S, P>(
        id: impl Into<String>,
        offset_ms: u64,
        group_ids: I,
        phases: P,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
        P: IntoIterator<Item = SignalPhase>,
    {
        let group_ids: Vec<String> = group_ids.into_iter().map(Into::into).collect();
        Self {
            id: id.into(),
            kind: SignalControllerKind::FixedTime,
            offset_ms,
            group_ids,
            phases: phases.into_iter().collect(),
        }
    }

    /// 返回 controller external ID。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回 controller kind。
    pub const fn kind(&self) -> SignalControllerKind {
        self.kind
    }

    /// 返回 canonical offset，单位毫秒。
    pub const fn offset_ms(&self) -> u64 {
        self.offset_ms
    }

    /// 返回输入 group ID 顺序。
    pub fn group_ids(&self) -> &[String] {
        &self.group_ids
    }

    /// 返回 program phase 顺序。
    pub fn phases(&self) -> &[SignalPhase] {
        &self.phases
    }
}

/// ManeuverGate 的 external signal binding。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignalControlInput {
    Group(String),
    None,
}

/// ManeuverPath transition 上的 immutable ManeuverGate 输入定义。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManeuverGate {
    id: String,
    maneuver_path_id: String,
    transition_index: u32,
    stop_line_id: String,
    signal_control: SignalControlInput,
}

impl ManeuverGate {
    /// 创建待由 `SignalRegistry` normalization 的 ManeuverGate definition。
    pub fn new(
        id: impl Into<String>,
        maneuver_path_id: impl Into<String>,
        transition_index: u32,
        stop_line_id: impl Into<String>,
        signal_control: SignalControlInput,
    ) -> Self {
        Self {
            id: id.into(),
            maneuver_path_id: maneuver_path_id.into(),
            transition_index,
            stop_line_id: stop_line_id.into(),
            signal_control,
        }
    }

    /// 返回 ManeuverGate external ID。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回绑定的 ManeuverPath external ID。
    pub fn maneuver_path_id(&self) -> &str {
        &self.maneuver_path_id
    }

    /// 返回 path-local transition index。
    pub const fn transition_index(&self) -> u32 {
        self.transition_index
    }

    /// 返回绑定的 StopLine external ID。
    pub fn stop_line_id(&self) -> &str {
        &self.stop_line_id
    }

    /// 返回 external signal control。
    pub const fn signal_control(&self) -> &SignalControlInput {
        &self.signal_control
    }
}

/// normalized ManeuverGate signal binding。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SignalControl {
    Group(SignalGroupHandle),
    None,
}

/// 当前已提交的 fixed-time controller snapshot。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignalControllerState {
    /// 由 world absolute integer time 推导的 fixed-time controller state。
    FixedTime {
        /// 当前 controller-local phase。
        current_phase: SignalPhaseRef,
        /// 当前 cycle 内的半开区间位置，单位毫秒。
        cycle_position_ms: u64,
        /// 当前 phase 已经过的时间，单位毫秒。
        phase_elapsed_ms: u64,
        /// 当前 phase 尚余的时间，单位毫秒。
        phase_remaining_ms: u64,
    },
}

impl SignalControllerState {
    /// 返回当前 controller-local phase reference。
    pub const fn current_phase(self) -> SignalPhaseRef {
        match self {
            Self::FixedTime { current_phase, .. } => current_phase,
        }
    }

    /// 返回当前 cycle 内的位置。
    pub const fn cycle_position_ms(self) -> u64 {
        match self {
            Self::FixedTime {
                cycle_position_ms, ..
            } => cycle_position_ms,
        }
    }

    /// 返回当前 phase 已经过的时间。
    pub const fn phase_elapsed_ms(self) -> u64 {
        match self {
            Self::FixedTime {
                phase_elapsed_ms, ..
            } => phase_elapsed_ms,
        }
    }

    /// 返回当前 phase 尚余的时间。
    pub const fn phase_remaining_ms(self) -> u64 {
        match self {
            Self::FixedTime {
                phase_remaining_ms, ..
            } => phase_remaining_ms,
        }
    }
}

/// 当前已提交的 SignalGroup snapshot。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SignalGroupSnapshot {
    aspect: SignalAspect,
}

impl SignalGroupSnapshot {
    const fn new(aspect: SignalAspect) -> Self {
        Self { aspect }
    }

    /// 返回当前 indication。
    pub const fn aspect(self) -> SignalAspect {
        self.aspect
    }
}

/// v0.4 signal layer 对 PreGate entry 的判断。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignalLayerPermission {
    /// Signal layer 允许进入 Gate；未来规则层仍可进一步约束。
    ProtectedAllow,
    /// Signal layer 要求车辆在 Gate 前停车。
    DenyAndStop,
}

/// ManeuverGate 当前 signal binding snapshot。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ManeuverGateSignalState {
    /// `signalControl:none`，仅表示 signal layer 不施加约束。
    Uncontrolled,
    /// Gate 由一个 SignalGroup 控制。
    Controlled {
        /// 控制该 Gate 的 SignalGroup。
        group: SignalGroupHandle,
        /// 当前已提交的 indication。
        aspect: SignalAspect,
        /// 由当前 indication 映射出的 signal-layer permission。
        permission: SignalLayerPermission,
    },
}

/// 当前已提交的 ManeuverGate snapshot。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ManeuverGateState {
    gate: ManeuverGateHandle,
    stop_line: StopLineHandle,
    signal: ManeuverGateSignalState,
}

/// Tick-local SignalStop spatial target 与事件归因。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SignalStopConstraint {
    pub(crate) route_distance: f64,
    pub(crate) gate: ManeuverGateHandle,
    pub(crate) stop_line: StopLineHandle,
    pub(crate) group: SignalGroupHandle,
    pub(crate) aspect: SignalAspect,
    pub(crate) from_route_edge_index: usize,
    pub(crate) to_route_edge_index: usize,
}

impl ManeuverGateState {
    /// 返回 Gate identity。
    pub const fn gate(self) -> ManeuverGateHandle {
        self.gate
    }

    /// 返回 Gate 使用的 StopLine。
    pub const fn stop_line(self) -> StopLineHandle {
        self.stop_line
    }

    /// 返回当前 signal binding/aspect/permission snapshot。
    pub const fn signal(self) -> ManeuverGateSignalState {
        self.signal
    }
}

/// Signals 当前已提交的 compact authority snapshot。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SignalRuntimeState {
    controllers: Vec<SignalControllerState>,
    groups: Vec<SignalGroupSnapshot>,
    has_restrictive_group: bool,
}

impl SignalRuntimeState {
    pub(crate) fn controller_state(
        &self,
        handle: SignalControllerHandle,
    ) -> Option<SignalControllerState> {
        self.controllers.get(handle.index()).copied()
    }

    pub(crate) fn controller_states(
        &self,
    ) -> impl ExactSizeIterator<Item = (SignalControllerHandle, SignalControllerState)> + '_ {
        self.controllers
            .iter()
            .copied()
            .enumerate()
            .map(|(index, state)| (SignalControllerHandle::new(index), state))
    }

    pub(crate) fn group_state(&self, handle: SignalGroupHandle) -> Option<SignalGroupSnapshot> {
        self.groups.get(handle.index()).copied()
    }

    pub(crate) fn group_states(
        &self,
    ) -> impl ExactSizeIterator<Item = (SignalGroupHandle, SignalGroupSnapshot)> + '_ {
        self.groups
            .iter()
            .copied()
            .enumerate()
            .map(|(index, state)| (SignalGroupHandle::new(index), state))
    }

    pub(crate) const fn has_restrictive_group(&self) -> bool {
        self.has_restrictive_group
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        let Self {
            controllers,
            groups,
            has_restrictive_group: _,
        } = self;
        controllers.capacity() * std::mem::size_of::<SignalControllerState>()
            + groups.capacity() * std::mem::size_of::<SignalGroupSnapshot>()
    }
}

/// 可跨 tick 复用、但不属于 Core authority state 的 signal candidate scratch。
#[derive(Debug, Default)]
pub(crate) struct SignalRuntimeScratch {
    state: SignalRuntimeState,
}

impl Clone for SignalRuntimeScratch {
    fn clone(&self) -> Self {
        Self {
            state: SignalRuntimeState {
                controllers: Vec::with_capacity(self.state.controllers.capacity()),
                groups: Vec::with_capacity(self.state.groups.capacity()),
                has_restrictive_group: false,
            },
        }
    }
}

impl PartialEq for SignalRuntimeScratch {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl SignalRuntimeScratch {
    pub(crate) const fn state(&self) -> &SignalRuntimeState {
        &self.state
    }

    pub(crate) fn state_mut(&mut self) -> &mut SignalRuntimeState {
        &mut self.state
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        let Self { state } = self;
        state.retained_bytes()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedStopLine {
    definition: StopLine,
    edge: EdgeHandle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedSignalGroup {
    definition: SignalGroup,
    controller: SignalControllerHandle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedSignalPhase {
    definition: SignalPhase,
    aspects_by_group: Vec<SignalAspect>,
    end_offset_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedSignalController {
    definition: SignalController,
    groups: Vec<SignalGroupHandle>,
    phases: Vec<ResolvedSignalPhase>,
    phase_handles: IndexMap<String, SignalPhaseRef>,
    cycle_duration_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedManeuverGate {
    definition: ManeuverGate,
    maneuver_path: ManeuverPathHandle,
    stop_line: StopLineHandle,
    control: SignalControl,
}

/// 已完成全部 v0.4 static Signals domain normalization 的 immutable registry。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalRegistry {
    stop_lines: Vec<ResolvedStopLine>,
    stop_line_handles: IndexMap<String, StopLineHandle>,
    stop_lines_by_edge: IndexMap<EdgeHandle, StopLineHandle>,
    groups: Vec<ResolvedSignalGroup>,
    group_handles: IndexMap<String, SignalGroupHandle>,
    controllers: Vec<ResolvedSignalController>,
    controller_handles: IndexMap<String, SignalControllerHandle>,
    maneuver_gates: Vec<ResolvedManeuverGate>,
    maneuver_gate_handles: IndexMap<String, ManeuverGateHandle>,
    maneuver_gate_by_path_transition: IndexMap<(ManeuverPathHandle, u32), ManeuverGateHandle>,
    maneuver_gate_ranges: Vec<Range<usize>>,
    maneuver_gates_by_path: Vec<ManeuverGateHandle>,
}

impl SignalRegistry {
    /// 创建合法的空 Signals registry。
    pub fn empty() -> Self {
        Self {
            stop_lines: Vec::new(),
            stop_line_handles: IndexMap::new(),
            stop_lines_by_edge: IndexMap::new(),
            groups: Vec::new(),
            group_handles: IndexMap::new(),
            controllers: Vec::new(),
            controller_handles: IndexMap::new(),
            maneuver_gates: Vec::new(),
            maneuver_gate_handles: IndexMap::new(),
            maneuver_gate_by_path_transition: IndexMap::new(),
            maneuver_gate_ranges: Vec::new(),
            maneuver_gates_by_path: Vec::new(),
        }
    }

    /// 按 canonical 顺序 normalization 全部 static Signals definitions。
    ///
    /// `junctions` 可以来自另一张 LaneGraph；构造器会先按 retained external
    /// definitions 对 `lane_graph` 重新绑定，再解析 StopLine 与 ManeuverGate。
    pub fn try_new<SL, SG, SC, MG>(
        lane_graph: &LaneGraph,
        junctions: &JunctionRegistry,
        stop_lines: SL,
        groups: SG,
        controllers: SC,
        maneuver_gates: MG,
    ) -> Result<Self, CoreError>
    where
        SL: IntoIterator<Item = StopLine>,
        SG: IntoIterator<Item = SignalGroup>,
        SC: IntoIterator<Item = SignalController>,
        MG: IntoIterator<Item = ManeuverGate>,
    {
        let rebound_junctions = junctions.rebind_to_lane_graph(lane_graph)?;
        Self::try_new_with_rebound_junctions(
            lane_graph,
            &rebound_junctions,
            stop_lines,
            groups,
            controllers,
            maneuver_gates,
        )
    }

    fn try_new_with_rebound_junctions<SL, SG, SC, MG>(
        lane_graph: &LaneGraph,
        junctions: &JunctionRegistry,
        stop_lines: SL,
        groups: SG,
        controllers: SC,
        maneuver_gates: MG,
    ) -> Result<Self, CoreError>
    where
        SL: IntoIterator<Item = StopLine>,
        SG: IntoIterator<Item = SignalGroup>,
        SC: IntoIterator<Item = SignalController>,
        MG: IntoIterator<Item = ManeuverGate>,
    {
        let mut resolved_stop_lines: Vec<ResolvedStopLine> = Vec::new();
        let mut stop_line_handles = IndexMap::new();
        let mut stop_lines_by_edge: IndexMap<EdgeHandle, StopLineHandle> = IndexMap::new();
        for stop_line in stop_lines {
            validate_external_id("signals.stopLines[].id", stop_line.id())?;
            if stop_line_handles.contains_key(stop_line.id()) {
                return Err(CoreError::DuplicateStopLineId {
                    stop_line_id: stop_line.id().to_owned(),
                });
            }
            validate_external_id("signals.stopLines[].edgeId", stop_line.edge_id())?;
            let edge = lane_graph.edge_handle(stop_line.edge_id()).ok_or_else(|| {
                CoreError::UnknownStopLineEdge {
                    stop_line_id: stop_line.id().to_owned(),
                    edge_id: stop_line.edge_id().to_owned(),
                }
            })?;
            if let Some(first) = stop_lines_by_edge.get(&edge).copied() {
                return Err(CoreError::DuplicateStopLineEdge {
                    edge_id: stop_line.edge_id().to_owned(),
                    first_stop_line_id: resolved_stop_lines[first.index()]
                        .definition
                        .id()
                        .to_owned(),
                    duplicate_stop_line_id: stop_line.id().to_owned(),
                });
            }
            let handle = StopLineHandle::new(resolved_stop_lines.len());
            stop_line_handles.insert(stop_line.id().to_owned(), handle);
            stop_lines_by_edge.insert(edge, handle);
            resolved_stop_lines.push(ResolvedStopLine {
                definition: stop_line,
                edge,
            });
        }

        let mut group_definitions = Vec::new();
        let mut group_handles = IndexMap::new();
        for group in groups {
            validate_external_id("signals.groups[].id", group.id())?;
            if group_handles.contains_key(group.id()) {
                return Err(CoreError::DuplicateSignalGroupId {
                    group_id: group.id().to_owned(),
                });
            }
            let handle = SignalGroupHandle::new(group_definitions.len());
            group_handles.insert(group.id().to_owned(), handle);
            group_definitions.push(group);
        }

        let mut owner_by_group: Vec<Option<SignalControllerHandle>> =
            vec![None; group_definitions.len()];
        let mut resolved_controllers: Vec<ResolvedSignalController> = Vec::new();
        let mut controller_handles = IndexMap::new();
        for controller in controllers {
            validate_external_id("signals.controllers[].id", controller.id())?;
            if controller_handles.contains_key(controller.id()) {
                return Err(CoreError::DuplicateSignalControllerId {
                    controller_id: controller.id().to_owned(),
                });
            }
            if controller.group_ids().is_empty() {
                return Err(CoreError::EmptySignalControllerGroups {
                    controller_id: controller.id().to_owned(),
                });
            }
            if controller.phases().is_empty() {
                return Err(CoreError::EmptySignalControllerPhases {
                    controller_id: controller.id().to_owned(),
                });
            }

            let controller_handle = SignalControllerHandle::new(resolved_controllers.len());
            let mut controller_group_ids = IndexSet::new();
            let mut controller_groups = Vec::with_capacity(controller.group_ids().len());
            for group_id in controller.group_ids() {
                validate_external_id("signals.controllers[].groupIds[]", group_id)?;
                if !controller_group_ids.insert(group_id.as_str()) {
                    return Err(CoreError::DuplicateSignalControllerGroup {
                        controller_id: controller.id().to_owned(),
                        group_id: group_id.clone(),
                    });
                }
                let group = group_handles.get(group_id).copied().ok_or_else(|| {
                    CoreError::UnknownSignalControllerGroup {
                        controller_id: controller.id().to_owned(),
                        group_id: group_id.clone(),
                    }
                })?;
                if let Some(first_controller) = owner_by_group[group.index()] {
                    return Err(CoreError::SignalGroupMultipleControllers {
                        group_id: group_id.clone(),
                        first_controller_id: resolved_controllers[first_controller.index()]
                            .definition
                            .id()
                            .to_owned(),
                        duplicate_controller_id: controller.id().to_owned(),
                    });
                }
                owner_by_group[group.index()] = Some(controller_handle);
                controller_groups.push(group);
            }

            let mut phase_ids = IndexSet::new();
            let mut resolved_phases = Vec::with_capacity(controller.phases().len());
            let mut phase_handles = IndexMap::new();
            let mut cycle_duration_ms = 0_u64;
            for phase in controller.phases() {
                validate_external_id("signals.controllers[].phases[].id", phase.id())?;
                if !phase_ids.insert(phase.id()) {
                    return Err(CoreError::DuplicateSignalPhaseId {
                        controller_id: controller.id().to_owned(),
                        phase_id: phase.id().to_owned(),
                    });
                }
                if !(1..=MAX_PORTABLE_SIGNAL_TIME_MS).contains(&phase.duration_ms()) {
                    return Err(CoreError::InvalidSignalPhaseDuration {
                        controller_id: controller.id().to_owned(),
                        phase_id: phase.id().to_owned(),
                        duration_ms: phase.duration_ms(),
                        max_inclusive: MAX_PORTABLE_SIGNAL_TIME_MS,
                    });
                }
                cycle_duration_ms = cycle_duration_ms
                    .checked_add(phase.duration_ms())
                    .filter(|sum| *sum <= MAX_PORTABLE_SIGNAL_TIME_MS)
                    .ok_or_else(|| CoreError::SignalCycleDurationOverflow {
                        controller_id: controller.id().to_owned(),
                        max_inclusive: MAX_PORTABLE_SIGNAL_TIME_MS,
                    })?;

                let mut aspects_by_group = vec![None; controller_groups.len()];
                for state in phase.states() {
                    validate_external_id(
                        "signals.controllers[].phases[].states[].groupId",
                        state.group_id(),
                    )?;
                    let group_position = controller
                        .group_ids()
                        .iter()
                        .position(|id| id == state.group_id())
                        .ok_or_else(|| CoreError::UnknownSignalPhaseGroup {
                            controller_id: controller.id().to_owned(),
                            phase_id: phase.id().to_owned(),
                            group_id: state.group_id().to_owned(),
                        })?;
                    if aspects_by_group[group_position]
                        .replace(state.aspect())
                        .is_some()
                    {
                        return Err(CoreError::DuplicateSignalPhaseGroup {
                            controller_id: controller.id().to_owned(),
                            phase_id: phase.id().to_owned(),
                            group_id: state.group_id().to_owned(),
                        });
                    }
                }
                for (index, aspect) in aspects_by_group.iter().enumerate() {
                    if aspect.is_none() {
                        return Err(CoreError::MissingSignalPhaseGroup {
                            controller_id: controller.id().to_owned(),
                            phase_id: phase.id().to_owned(),
                            group_id: controller.group_ids()[index].clone(),
                        });
                    }
                }
                let phase_ref = SignalPhaseRef::new(controller_handle, resolved_phases.len());
                phase_handles.insert(phase.id().to_owned(), phase_ref);
                resolved_phases.push(ResolvedSignalPhase {
                    definition: phase.clone(),
                    aspects_by_group: aspects_by_group
                        .into_iter()
                        .map(|aspect| aspect.expect("complete state checked above"))
                        .collect(),
                    end_offset_ms: cycle_duration_ms,
                });
            }

            if controller.offset_ms() > MAX_PORTABLE_SIGNAL_TIME_MS {
                return Err(CoreError::InvalidSignalControllerOffset {
                    controller_id: controller.id().to_owned(),
                    offset_ms: controller.offset_ms(),
                    max_inclusive: MAX_PORTABLE_SIGNAL_TIME_MS,
                });
            }
            if controller.offset_ms() >= cycle_duration_ms {
                return Err(CoreError::SignalControllerOffsetOutOfRange {
                    controller_id: controller.id().to_owned(),
                    offset_ms: controller.offset_ms(),
                    cycle_duration_ms,
                });
            }

            controller_handles.insert(controller.id().to_owned(), controller_handle);
            resolved_controllers.push(ResolvedSignalController {
                definition: controller,
                groups: controller_groups,
                phases: resolved_phases,
                phase_handles,
                cycle_duration_ms,
            });
        }

        let mut gate_definitions = Vec::new();
        let mut maneuver_gate_handles = IndexMap::new();
        for gate in maneuver_gates {
            validate_external_id("signals.maneuverGates[].id", gate.id())?;
            if maneuver_gate_handles.contains_key(gate.id()) {
                return Err(CoreError::DuplicateManeuverGateId {
                    maneuver_gate_id: gate.id().to_owned(),
                });
            }
            if gate_definitions.len() >= u32::MAX as usize {
                return Err(CoreError::StaticDomainCapacityExceeded {
                    domain: "maneuverGates",
                    count: gate_definitions.len().saturating_add(1),
                    max_inclusive: u32::MAX,
                });
            }

            let handle = ManeuverGateHandle::new(gate_definitions.len());
            maneuver_gate_handles.insert(gate.id().to_owned(), handle);
            gate_definitions.push(gate);
        }

        let mut gate_paths = Vec::with_capacity(gate_definitions.len());
        for gate in &gate_definitions {
            validate_external_id(
                "signals.maneuverGates[].maneuverPathId",
                gate.maneuver_path_id(),
            )?;
            let maneuver_path = junctions
                .maneuver_path_handle(gate.maneuver_path_id())
                .ok_or_else(|| CoreError::UnknownManeuverGatePath {
                    maneuver_gate_id: gate.id().to_owned(),
                    maneuver_path_id: gate.maneuver_path_id().to_owned(),
                })?;
            gate_paths.push(maneuver_path);
        }

        let mut gate_from_edges = Vec::with_capacity(gate_definitions.len());
        for (gate, maneuver_path) in gate_definitions.iter().zip(&gate_paths) {
            let path_edges = junctions
                .maneuver_path_edges(*maneuver_path)
                .expect("resolved ManeuverPath handle must exist");
            let transition_count = path_edges.len() - 1;
            let transition_index = gate.transition_index() as usize;
            if transition_index >= transition_count {
                return Err(CoreError::ManeuverGateTransitionOutOfRange {
                    maneuver_gate_id: gate.id().to_owned(),
                    maneuver_path_id: gate.maneuver_path_id().to_owned(),
                    transition_index: gate.transition_index(),
                    transition_count,
                });
            }
            gate_from_edges.push(path_edges[transition_index]);
        }

        let mut gate_by_path_transition: IndexMap<(ManeuverPathHandle, u32), ManeuverGateHandle> =
            IndexMap::new();
        for (index, (gate, maneuver_path)) in gate_definitions.iter().zip(&gate_paths).enumerate() {
            if let Some(first) = gate_by_path_transition
                .get(&(*maneuver_path, gate.transition_index()))
                .copied()
            {
                return Err(CoreError::DuplicateManeuverGatePathTransition {
                    maneuver_path_id: gate.maneuver_path_id().to_owned(),
                    transition_index: gate.transition_index(),
                    first_maneuver_gate_id: gate_definitions[first.index()].id().to_owned(),
                    duplicate_maneuver_gate_id: gate.id().to_owned(),
                });
            }
            gate_by_path_transition.insert(
                (*maneuver_path, gate.transition_index()),
                ManeuverGateHandle::new(index),
            );
        }

        let mut gate_stop_lines = Vec::with_capacity(gate_definitions.len());
        for gate in &gate_definitions {
            validate_external_id("signals.maneuverGates[].stopLineId", gate.stop_line_id())?;
            let stop_line = stop_line_handles
                .get(gate.stop_line_id())
                .copied()
                .ok_or_else(|| CoreError::UnknownManeuverGateStopLine {
                    maneuver_gate_id: gate.id().to_owned(),
                    stop_line_id: gate.stop_line_id().to_owned(),
                })?;
            gate_stop_lines.push(stop_line);
        }

        for ((gate, stop_line), from_edge) in gate_definitions
            .iter()
            .zip(&gate_stop_lines)
            .zip(&gate_from_edges)
        {
            let normalized_stop_line = &resolved_stop_lines[stop_line.index()];
            if normalized_stop_line.edge != *from_edge {
                return Err(CoreError::ManeuverGateStopLineMismatch {
                    maneuver_gate_id: gate.id().to_owned(),
                    stop_line_id: gate.stop_line_id().to_owned(),
                    stop_line_edge_id: normalized_stop_line.definition.edge_id().to_owned(),
                    path_from_edge_id: lane_graph
                        .edge_external_id(*from_edge)
                        .expect("resolved path edge must belong to LaneGraph")
                        .to_owned(),
                });
            }
        }

        let mut group_usage = vec![false; group_definitions.len()];
        let mut gate_controls = Vec::with_capacity(gate_definitions.len());
        for gate in &gate_definitions {
            let control = match gate.signal_control() {
                SignalControlInput::Group(group_id) => {
                    validate_external_id(
                        "signals.maneuverGates[].signalControl.groupId",
                        group_id,
                    )?;
                    let group = group_handles.get(group_id).copied().ok_or_else(|| {
                        CoreError::UnknownManeuverGateSignalGroup {
                            maneuver_gate_id: gate.id().to_owned(),
                            group_id: group_id.clone(),
                        }
                    })?;
                    group_usage[group.index()] = true;
                    SignalControl::Group(group)
                }
                SignalControlInput::None => SignalControl::None,
            };
            gate_controls.push(control);
        }

        let mut resolved_groups = Vec::with_capacity(group_definitions.len());
        for (index, definition) in group_definitions.into_iter().enumerate() {
            let controller =
                owner_by_group[index].ok_or_else(|| CoreError::UnownedSignalGroup {
                    group_id: definition.id().to_owned(),
                })?;
            if !group_usage[index] {
                return Err(CoreError::UnusedSignalGroup {
                    group_id: definition.id().to_owned(),
                });
            }
            resolved_groups.push(ResolvedSignalGroup {
                definition,
                controller,
            });
        }

        for gate in &gate_definitions {
            if gate.transition_index() != 0 {
                return Err(CoreError::UnsupportedManeuverGateTransition {
                    maneuver_gate_id: gate.id().to_owned(),
                    transition_index: gate.transition_index(),
                });
            }
        }

        for resolved in &resolved_stop_lines {
            let next_edges = lane_graph
                .next_edges(resolved.edge)
                .expect("resolved StopLine edge must exist");
            if next_edges.is_empty() {
                return Err(CoreError::OrphanStopLine {
                    stop_line_id: resolved.definition.id().to_owned(),
                    edge_id: resolved.definition.edge_id().to_owned(),
                });
            }
            for to_edge in next_edges {
                let candidates = junctions.entry_transition_candidates(resolved.edge, *to_edge);
                if candidates.is_empty() {
                    return Err(CoreError::MissingManeuverPathCoverage {
                        stop_line_id: resolved.definition.id().to_owned(),
                        from_edge_id: resolved.definition.edge_id().to_owned(),
                        to_edge_id: lane_graph
                            .edge_external_id(*to_edge)
                            .expect("resolved connection target must exist")
                            .to_owned(),
                    });
                }
                for path in candidates {
                    if !gate_by_path_transition.contains_key(&(*path, 0)) {
                        return Err(CoreError::MissingManeuverGateCoverage {
                            stop_line_id: resolved.definition.id().to_owned(),
                            maneuver_path_id: junctions
                                .maneuver_path_external_id(*path)
                                .expect("resolved ManeuverPath handle must exist")
                                .to_owned(),
                        });
                    }
                }
            }
        }

        let mut normalized_gates = Vec::with_capacity(gate_definitions.len());
        let mut gate_counts_by_path = vec![0_usize; junctions.maneuver_paths().len()];
        for (((gate, maneuver_path), stop_line), control) in gate_definitions
            .into_iter()
            .zip(gate_paths)
            .zip(gate_stop_lines)
            .zip(gate_controls)
        {
            gate_counts_by_path[maneuver_path.index()] += 1;
            normalized_gates.push(ResolvedManeuverGate {
                definition: gate,
                maneuver_path,
                stop_line,
                control,
            });
        }

        let mut gate_starts = Vec::with_capacity(gate_counts_by_path.len());
        let mut gate_total = 0_usize;
        for count in &gate_counts_by_path {
            gate_starts.push(gate_total);
            gate_total += count;
        }
        let maneuver_gate_ranges = gate_starts
            .iter()
            .zip(&gate_counts_by_path)
            .map(|(start, count)| *start..*start + *count)
            .collect::<Vec<_>>();
        let mut gate_next = gate_starts;
        let mut maneuver_gates_by_path = vec![None; normalized_gates.len()];
        for (index, gate) in normalized_gates.iter().enumerate() {
            let slot = gate_next[gate.maneuver_path.index()];
            maneuver_gates_by_path[slot] = Some(ManeuverGateHandle::new(index));
            gate_next[gate.maneuver_path.index()] += 1;
        }
        let maneuver_gates_by_path = maneuver_gates_by_path
            .into_iter()
            .map(|gate| gate.expect("gate counts must match normalized paths"))
            .collect();

        Ok(Self {
            stop_lines: resolved_stop_lines,
            stop_line_handles,
            stop_lines_by_edge,
            groups: resolved_groups,
            group_handles,
            controllers: resolved_controllers,
            controller_handles,
            maneuver_gates: normalized_gates,
            maneuver_gate_handles,
            maneuver_gate_by_path_transition: gate_by_path_transition,
            maneuver_gate_ranges,
            maneuver_gates_by_path,
        })
    }

    pub(crate) fn rebind_to_static_topology(
        self,
        lane_graph: &LaneGraph,
        junctions: &JunctionRegistry,
    ) -> Result<Self, CoreError> {
        Self::try_new_with_rebound_junctions(
            lane_graph,
            junctions,
            self.stop_lines
                .into_iter()
                .map(|resolved| resolved.definition),
            self.groups.into_iter().map(|resolved| resolved.definition),
            self.controllers
                .into_iter()
                .map(|resolved| resolved.definition),
            self.maneuver_gates
                .into_iter()
                .map(|resolved| resolved.definition),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.stop_lines.is_empty()
            && self.groups.is_empty()
            && self.controllers.is_empty()
            && self.maneuver_gates.is_empty()
    }

    pub fn stop_line_handle(&self, id: &str) -> Option<StopLineHandle> {
        self.stop_line_handles.get(id).copied()
    }

    pub fn stop_line(&self, handle: StopLineHandle) -> Option<&StopLine> {
        self.stop_lines
            .get(handle.index())
            .map(|resolved| &resolved.definition)
    }

    pub fn stop_line_external_id(&self, handle: StopLineHandle) -> Option<&str> {
        self.stop_line(handle).map(StopLine::id)
    }

    pub fn stop_line_edge(&self, handle: StopLineHandle) -> Option<EdgeHandle> {
        self.stop_lines
            .get(handle.index())
            .map(|resolved| resolved.edge)
    }

    pub fn stop_line_for_edge(&self, edge: EdgeHandle) -> Option<StopLineHandle> {
        self.stop_lines_by_edge.get(&edge).copied()
    }

    pub fn stop_lines(&self) -> impl ExactSizeIterator<Item = &StopLine> {
        self.stop_lines.iter().map(|resolved| &resolved.definition)
    }

    pub fn group_handle(&self, id: &str) -> Option<SignalGroupHandle> {
        self.group_handles.get(id).copied()
    }

    pub fn group(&self, handle: SignalGroupHandle) -> Option<&SignalGroup> {
        self.groups
            .get(handle.index())
            .map(|resolved| &resolved.definition)
    }

    pub fn group_external_id(&self, handle: SignalGroupHandle) -> Option<&str> {
        self.group(handle).map(SignalGroup::id)
    }

    pub fn group_controller(&self, handle: SignalGroupHandle) -> Option<SignalControllerHandle> {
        self.groups
            .get(handle.index())
            .map(|resolved| resolved.controller)
    }

    pub fn groups(&self) -> impl ExactSizeIterator<Item = &SignalGroup> {
        self.groups.iter().map(|resolved| &resolved.definition)
    }

    pub fn controller_handle(&self, id: &str) -> Option<SignalControllerHandle> {
        self.controller_handles.get(id).copied()
    }

    pub fn controller(&self, handle: SignalControllerHandle) -> Option<&SignalController> {
        self.controllers
            .get(handle.index())
            .map(|resolved| &resolved.definition)
    }

    pub fn controller_external_id(&self, handle: SignalControllerHandle) -> Option<&str> {
        self.controller(handle).map(SignalController::id)
    }

    pub fn controller_groups(
        &self,
        handle: SignalControllerHandle,
    ) -> Option<&[SignalGroupHandle]> {
        self.controllers
            .get(handle.index())
            .map(|resolved| resolved.groups.as_slice())
    }

    pub fn controller_cycle_duration_ms(&self, handle: SignalControllerHandle) -> Option<u64> {
        self.controllers
            .get(handle.index())
            .map(|resolved| resolved.cycle_duration_ms)
    }

    pub fn controllers(&self) -> impl ExactSizeIterator<Item = &SignalController> {
        self.controllers.iter().map(|resolved| &resolved.definition)
    }

    pub fn phase_ref(
        &self,
        controller: SignalControllerHandle,
        id: &str,
    ) -> Option<SignalPhaseRef> {
        self.controllers
            .get(controller.index())?
            .phase_handles
            .get(id)
            .copied()
    }

    pub fn phase(&self, phase: SignalPhaseRef) -> Option<&SignalPhase> {
        self.controllers
            .get(phase.controller().index())?
            .phases
            .get(phase.index())
            .map(|resolved| &resolved.definition)
    }

    pub fn phase_external_id(&self, phase: SignalPhaseRef) -> Option<&str> {
        self.phase(phase).map(SignalPhase::id)
    }

    pub fn phase_aspects(&self, phase: SignalPhaseRef) -> Option<&[SignalAspect]> {
        self.controllers
            .get(phase.controller().index())?
            .phases
            .get(phase.index())
            .map(|resolved| resolved.aspects_by_group.as_slice())
    }

    /// 返回 phase 在所属 controller cycle 中的 exclusive end offset。
    pub fn phase_end_offset_ms(&self, phase: SignalPhaseRef) -> Option<u64> {
        self.controllers
            .get(phase.controller().index())?
            .phases
            .get(phase.index())
            .map(|resolved| resolved.end_offset_ms)
    }

    pub fn maneuver_gate(&self, handle: ManeuverGateHandle) -> Option<&ManeuverGate> {
        self.maneuver_gates
            .get(handle.index())
            .map(|resolved| &resolved.definition)
    }

    pub fn maneuver_gate_handle(&self, external_id: &str) -> Option<ManeuverGateHandle> {
        self.maneuver_gate_handles.get(external_id).copied()
    }

    pub fn maneuver_gate_external_id(&self, handle: ManeuverGateHandle) -> Option<&str> {
        self.maneuver_gate(handle).map(ManeuverGate::id)
    }

    pub fn maneuver_gate_path(&self, handle: ManeuverGateHandle) -> Option<ManeuverPathHandle> {
        self.maneuver_gates
            .get(handle.index())
            .map(|resolved| resolved.maneuver_path)
    }

    pub fn maneuver_gate_stop_line(&self, handle: ManeuverGateHandle) -> Option<StopLineHandle> {
        self.maneuver_gates
            .get(handle.index())
            .map(|resolved| resolved.stop_line)
    }

    pub fn maneuver_gate_control(&self, handle: ManeuverGateHandle) -> Option<SignalControl> {
        self.maneuver_gates
            .get(handle.index())
            .map(|resolved| resolved.control)
    }

    pub fn maneuver_gates(&self) -> impl ExactSizeIterator<Item = ManeuverGateHandle> + '_ {
        (0..self.maneuver_gates.len()).map(ManeuverGateHandle::new)
    }

    pub fn maneuver_path_gates(
        &self,
        path: ManeuverPathHandle,
    ) -> Option<impl ExactSizeIterator<Item = ManeuverGateHandle> + '_> {
        let range = self.maneuver_gate_ranges.get(path.index())?.clone();
        Some(self.maneuver_gates_by_path[range].iter().copied())
    }

    pub(crate) fn maneuver_gate_for_path_transition(
        &self,
        path: ManeuverPathHandle,
        transition_index: u32,
    ) -> Option<ManeuverGateHandle> {
        self.maneuver_gate_by_path_transition
            .get(&(path, transition_index))
            .copied()
    }

    pub(crate) fn maneuver_gate_is_signal_controlled(&self, handle: ManeuverGateHandle) -> bool {
        self.maneuver_gates
            .get(handle.index())
            .is_some_and(|gate| matches!(gate.control, SignalControl::Group(_)))
    }

    pub(crate) fn maneuver_gate_state_by_handle(
        &self,
        runtime: &SignalRuntimeState,
        handle: ManeuverGateHandle,
    ) -> Option<ManeuverGateState> {
        let gate = self.maneuver_gates.get(handle.index())?;
        Self::resolved_maneuver_gate_state(runtime, handle, gate)
    }

    pub(crate) fn populate_runtime_state(&self, time_ms: u64, state: &mut SignalRuntimeState) {
        state.controllers.clear();
        state.groups.clear();
        state.has_restrictive_group = false;
        state.controllers.reserve(self.controllers.len());
        state.groups.resize(
            self.groups.len(),
            SignalGroupSnapshot::new(SignalAspect::Red),
        );

        for (controller_index, controller) in self.controllers.iter().enumerate() {
            let cycle = u128::from(controller.cycle_duration_ms);
            let cycle_position_ms = u64::try_from(
                (u128::from(time_ms) + u128::from(controller.definition.offset_ms())) % cycle,
            )
            .expect("cycle position must fit in u64");
            let phase_index = controller
                .phases
                .partition_point(|phase| phase.end_offset_ms <= cycle_position_ms);
            let phase = controller
                .phases
                .get(phase_index)
                .expect("cycle position must resolve to a phase");
            let phase_start_ms = phase_index
                .checked_sub(1)
                .map_or(0, |index| controller.phases[index].end_offset_ms);
            let controller_handle = SignalControllerHandle::new(controller_index);
            state.controllers.push(SignalControllerState::FixedTime {
                current_phase: SignalPhaseRef::new(controller_handle, phase_index),
                cycle_position_ms,
                phase_elapsed_ms: cycle_position_ms - phase_start_ms,
                phase_remaining_ms: phase.end_offset_ms - cycle_position_ms,
            });

            for (group_index, group) in controller.groups.iter().copied().enumerate() {
                let aspect = phase.aspects_by_group[group_index];
                state.groups[group.index()] = SignalGroupSnapshot::new(aspect);
                state.has_restrictive_group |=
                    matches!(aspect, SignalAspect::Red | SignalAspect::Yellow);
            }
        }
    }

    pub(crate) fn maneuver_gate_state(
        &self,
        runtime: &SignalRuntimeState,
        handle: ManeuverGateHandle,
    ) -> Option<ManeuverGateState> {
        let gate = self.maneuver_gates.get(handle.index())?;
        Self::resolved_maneuver_gate_state(runtime, handle, gate)
    }

    fn resolved_maneuver_gate_state(
        runtime: &SignalRuntimeState,
        handle: ManeuverGateHandle,
        gate: &ResolvedManeuverGate,
    ) -> Option<ManeuverGateState> {
        let signal = match gate.control {
            SignalControl::None => ManeuverGateSignalState::Uncontrolled,
            SignalControl::Group(group) => {
                let aspect = runtime.group_state(group)?.aspect();
                let permission = match aspect {
                    SignalAspect::Green => SignalLayerPermission::ProtectedAllow,
                    SignalAspect::Red | SignalAspect::Yellow => SignalLayerPermission::DenyAndStop,
                };
                ManeuverGateSignalState::Controlled {
                    group,
                    aspect,
                    permission,
                }
            }
        };
        Some(ManeuverGateState {
            gate: handle,
            stop_line: gate.stop_line,
            signal,
        })
    }

    pub(crate) fn validate_fixed_delta_time(
        &self,
        fixed_delta_time_ms: u64,
    ) -> Result<(), CoreError> {
        for controller in &self.controllers {
            for phase in &controller.phases {
                if phase.definition.duration_ms() < fixed_delta_time_ms {
                    return Err(CoreError::SignalPhaseShorterThanFixedDelta {
                        controller_id: controller.definition.id().to_owned(),
                        phase_id: phase.definition.id().to_owned(),
                        duration_ms: phase.definition.duration_ms(),
                        fixed_delta_time_ms,
                    });
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        fn phase_heap_bytes(phase: &SignalPhase) -> usize {
            phase.id.capacity()
                + phase.states.capacity() * std::mem::size_of::<SignalGroupState>()
                + phase
                    .states
                    .iter()
                    .map(|state| state.group_id.capacity())
                    .sum::<usize>()
        }

        fn controller_definition_heap_bytes(controller: &SignalController) -> usize {
            controller.id.capacity()
                + controller.group_ids.capacity() * std::mem::size_of::<String>()
                + controller
                    .group_ids
                    .iter()
                    .map(String::capacity)
                    .sum::<usize>()
                + controller.phases.capacity() * std::mem::size_of::<SignalPhase>()
                + controller
                    .phases
                    .iter()
                    .map(phase_heap_bytes)
                    .sum::<usize>()
        }

        fn maneuver_gate_heap_bytes(gate: &ManeuverGate) -> usize {
            gate.id.capacity()
                + gate.maneuver_path_id.capacity()
                + gate.stop_line_id.capacity()
                + match &gate.signal_control {
                    SignalControlInput::Group(group_id) => group_id.capacity(),
                    SignalControlInput::None => 0,
                }
        }

        let Self {
            stop_lines,
            stop_line_handles,
            stop_lines_by_edge,
            groups,
            group_handles,
            controllers,
            controller_handles,
            maneuver_gates,
            maneuver_gate_handles,
            maneuver_gate_by_path_transition,
            maneuver_gate_ranges,
            maneuver_gates_by_path,
        } = self;

        let stop_line_bytes = stop_lines.capacity() * std::mem::size_of::<ResolvedStopLine>()
            + stop_lines
                .iter()
                .map(|stop_line| {
                    stop_line.definition.id.capacity() + stop_line.definition.edge_id.capacity()
                })
                .sum::<usize>();
        let stop_line_handle_bytes = stop_line_handles.capacity()
            * std::mem::size_of::<(String, StopLineHandle)>()
            + stop_line_handles
                .keys()
                .map(String::capacity)
                .sum::<usize>();
        let stop_line_edge_bytes =
            stop_lines_by_edge.capacity() * std::mem::size_of::<(EdgeHandle, StopLineHandle)>();

        let group_bytes = groups.capacity() * std::mem::size_of::<ResolvedSignalGroup>()
            + groups
                .iter()
                .map(|group| group.definition.id.capacity())
                .sum::<usize>();
        let group_handle_bytes = group_handles.capacity()
            * std::mem::size_of::<(String, SignalGroupHandle)>()
            + group_handles.keys().map(String::capacity).sum::<usize>();

        let controller_bytes = controllers.capacity()
            * std::mem::size_of::<ResolvedSignalController>()
            + controllers
                .iter()
                .map(|controller| {
                    controller_definition_heap_bytes(&controller.definition)
                        + controller.groups.capacity() * std::mem::size_of::<SignalGroupHandle>()
                        + controller.phases.capacity() * std::mem::size_of::<ResolvedSignalPhase>()
                        + controller
                            .phases
                            .iter()
                            .map(|phase| {
                                phase_heap_bytes(&phase.definition)
                                    + phase.aspects_by_group.capacity()
                                        * std::mem::size_of::<SignalAspect>()
                            })
                            .sum::<usize>()
                        + controller.phase_handles.capacity()
                            * std::mem::size_of::<(String, SignalPhaseRef)>()
                        + controller
                            .phase_handles
                            .keys()
                            .map(String::capacity)
                            .sum::<usize>()
                })
                .sum::<usize>();
        let controller_handle_bytes = controller_handles.capacity()
            * std::mem::size_of::<(String, SignalControllerHandle)>()
            + controller_handles
                .keys()
                .map(String::capacity)
                .sum::<usize>();

        let maneuver_gate_bytes = maneuver_gates.capacity()
            * std::mem::size_of::<ResolvedManeuverGate>()
            + maneuver_gates
                .iter()
                .map(|gate| maneuver_gate_heap_bytes(&gate.definition))
                .sum::<usize>();
        let maneuver_gate_handle_bytes = maneuver_gate_handles.capacity()
            * std::mem::size_of::<(String, ManeuverGateHandle)>()
            + maneuver_gate_handles
                .keys()
                .map(String::capacity)
                .sum::<usize>();
        let maneuver_gate_lookup_bytes = maneuver_gate_by_path_transition.capacity()
            * std::mem::size_of::<((ManeuverPathHandle, u32), ManeuverGateHandle)>()
            + maneuver_gate_ranges.capacity() * std::mem::size_of::<Range<usize>>()
            + maneuver_gates_by_path.capacity() * std::mem::size_of::<ManeuverGateHandle>();

        stop_line_bytes
            + stop_line_handle_bytes
            + stop_line_edge_bytes
            + group_bytes
            + group_handle_bytes
            + controller_bytes
            + controller_handle_bytes
            + maneuver_gate_bytes
            + maneuver_gate_handle_bytes
            + maneuver_gate_lookup_bytes
    }
}

impl Default for SignalRegistry {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EdgeLength, Junction, JunctionRegistry, LaneEdge, ManeuverPath, Movement};
    use proptest::prelude::*;

    fn aspect_from_seed(seed: u8) -> SignalAspect {
        match seed % 3 {
            0 => SignalAspect::Red,
            1 => SignalAspect::Yellow,
            _ => SignalAspect::Green,
        }
    }

    fn property_registry(
        group_count: usize,
        durations: &[u64],
        aspect_seeds: &[u8],
        offset_ms: u64,
    ) -> SignalRegistry {
        let mut edges = Vec::with_capacity(group_count * 2);
        let mut stop_lines = Vec::with_capacity(group_count);
        let mut groups = Vec::with_capacity(group_count);
        let mut gates = Vec::with_capacity(group_count);
        let mut junctions = Vec::with_capacity(group_count);
        let mut movements = Vec::with_capacity(group_count);
        let mut paths = Vec::with_capacity(group_count);
        let mut group_ids = Vec::with_capacity(group_count);
        for group_index in 0..group_count {
            let entry = format!("entry-{group_index}");
            let exit = format!("exit-{group_index}");
            let stop = format!("stop-{group_index}");
            let group = format!("group-{group_index}");
            let junction = format!("junction-{group_index}");
            let movement = format!("movement-{group_index}");
            let path = format!("path-{group_index}");
            edges.push(LaneEdge::new(
                entry.clone(),
                EdgeLength::try_new(10.0).unwrap(),
                crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                [exit.clone()],
            ));
            edges.push(LaneEdge::new(
                exit.clone(),
                EdgeLength::try_new(10.0).unwrap(),
                crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                Vec::<String>::new(),
            ));
            stop_lines.push(StopLine::new(
                stop.clone(),
                entry.clone(),
                StopLineLocation::EdgeEnd,
            ));
            groups.push(SignalGroup::new(group.clone()));
            junctions.push(Junction::new(junction.clone()));
            movements.push(Movement::new(movement.clone(), junction));
            paths.push(ManeuverPath::new(
                path.clone(),
                movement,
                entry,
                std::iter::empty::<String>(),
                exit,
            ));
            gates.push(ManeuverGate::new(
                format!("gate-{group_index}"),
                path,
                0,
                stop,
                SignalControlInput::Group(group.clone()),
            ));
            group_ids.push(group);
        }

        let phases = durations
            .iter()
            .enumerate()
            .map(|(phase_index, duration)| {
                SignalPhase::new(
                    format!("phase-{phase_index}"),
                    *duration,
                    group_ids.iter().enumerate().map(|(group_index, group_id)| {
                        let seed = aspect_seeds[phase_index * 8 + group_index];
                        SignalGroupState::new(group_id.clone(), aspect_from_seed(seed))
                    }),
                )
            })
            .collect::<Vec<_>>();
        let graph = LaneGraph::try_new(edges).expect("property graph must be valid");
        let junctions = JunctionRegistry::try_new(&graph, junctions, movements, paths)
            .expect("property topology must be valid");
        SignalRegistry::try_new(
            &graph,
            &junctions,
            stop_lines,
            groups,
            [SignalController::new_fixed_time(
                "controller",
                offset_ms,
                group_ids,
                phases,
            )],
            gates,
        )
        .expect("property registry must be valid")
    }

    fn oracle_phase(durations: &[u64], time_ms: u64, offset_ms: u64) -> (usize, u64, u64, u64) {
        let cycle = durations
            .iter()
            .map(|duration| u128::from(*duration))
            .sum::<u128>();
        let position = (u128::from(time_ms) + u128::from(offset_ms)) % cycle;
        let mut phase_start = 0_u128;
        for (phase_index, duration) in durations.iter().enumerate() {
            let phase_end = phase_start + u128::from(*duration);
            if position < phase_end {
                return (
                    phase_index,
                    u64::try_from(position).unwrap(),
                    u64::try_from(position - phase_start).unwrap(),
                    u64::try_from(phase_end - position).unwrap(),
                );
            }
            phase_start = phase_end;
        }
        unreachable!("position modulo cycle must resolve to one phase")
    }

    proptest! {
        #[test]
        fn fixed_time_resolver_matches_independent_u128_oracle(
            group_count in 1_usize..=8,
            durations in prop::collection::vec(1_u64..=10_000, 1..=8),
            aspect_seeds in prop::collection::vec(any::<u8>(), 64),
            raw_offset in any::<u64>(),
            arbitrary_time in any::<u64>(),
            near_max_delta in 0_u64..=1_000_000,
        ) {
            let cycle = durations.iter().sum::<u64>();
            let offset_ms = raw_offset % cycle;
            let registry = property_registry(group_count, &durations, &aspect_seeds, offset_ms);
            let controller = registry.controller_handle("controller").unwrap();
            let times = [
                0,
                arbitrary_time,
                u64::MAX,
                u64::MAX - near_max_delta,
            ];

            for time_ms in times {
                let (phase_index, position, elapsed, remaining) =
                    oracle_phase(&durations, time_ms, offset_ms);
                let mut state = SignalRuntimeState::default();
                registry.populate_runtime_state(time_ms, &mut state);
                let actual = state.controller_state(controller).unwrap();

                prop_assert_eq!(actual.current_phase().index(), phase_index);
                prop_assert_eq!(actual.cycle_position_ms(), position);
                prop_assert_eq!(actual.phase_elapsed_ms(), elapsed);
                prop_assert_eq!(actual.phase_remaining_ms(), remaining);
                for group_index in 0..group_count {
                    let group = registry
                        .group_handle(&format!("group-{group_index}"))
                        .unwrap();
                    prop_assert_eq!(
                        state.group_state(group).unwrap().aspect(),
                        aspect_from_seed(aspect_seeds[phase_index * 8 + group_index])
                    );
                }
                let expected_restrictive = (0..group_count).any(|group_index| {
                    matches!(
                        aspect_from_seed(aspect_seeds[phase_index * 8 + group_index]),
                        SignalAspect::Red | SignalAspect::Yellow
                    )
                });
                prop_assert_eq!(state.has_restrictive_group(), expected_restrictive);
            }
        }
    }

    #[test]
    fn absolute_time_resolver_is_overflow_safe_at_u64_max() {
        let graph = LaneGraph::try_new([
            LaneEdge::new(
                "entry",
                EdgeLength::try_new(10.0).unwrap(),
                crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                ["exit"],
            ),
            LaneEdge::new(
                "exit",
                EdgeLength::try_new(10.0).unwrap(),
                crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                Vec::<String>::new(),
            ),
        ])
        .expect("valid graph");
        let junctions = JunctionRegistry::try_new(
            &graph,
            [Junction::new("junction")],
            [Movement::new("movement", "junction")],
            [ManeuverPath::new(
                "path",
                "movement",
                "entry",
                std::iter::empty::<&str>(),
                "exit",
            )],
        )
        .expect("valid topology");
        let registry = SignalRegistry::try_new(
            &graph,
            &junctions,
            [StopLine::new("stop", "entry", StopLineLocation::EdgeEnd)],
            [SignalGroup::new("group")],
            [SignalController::new_fixed_time(
                "controller",
                17,
                ["group"],
                [
                    SignalPhase::new(
                        "first",
                        11,
                        [SignalGroupState::new("group", SignalAspect::Red)],
                    ),
                    SignalPhase::new(
                        "second",
                        13,
                        [SignalGroupState::new("group", SignalAspect::Green)],
                    ),
                ],
            )],
            [ManeuverGate::new(
                "gate",
                "path",
                0,
                "stop",
                SignalControlInput::Group("group".to_owned()),
            )],
        )
        .expect("valid signals");
        let controller = registry
            .controller_handle("controller")
            .expect("controller handle");
        let expected_position =
            u64::try_from((u128::from(u64::MAX) + 17) % 24).expect("position fits in u64");
        let mut state = SignalRuntimeState::default();

        registry.populate_runtime_state(u64::MAX, &mut state);

        let actual = state
            .controller_state(controller)
            .expect("controller state");
        assert_eq!(actual.cycle_position_ms(), expected_position);
        let expected_phase = if expected_position < 11 {
            registry
                .phase_ref(controller, "first")
                .expect("first phase")
        } else {
            registry
                .phase_ref(controller, "second")
                .expect("second phase")
        };
        assert_eq!(actual.current_phase(), expected_phase);
    }
}
