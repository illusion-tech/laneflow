//! v0.4 Signals 静态领域、normalization registry 与 resolver。

use indexmap::{IndexMap, IndexSet};

use crate::{
    error::CoreError,
    graph::LaneGraph,
    handle::{
        EdgeHandle, SignalControllerHandle, SignalGroupHandle, SignalPhaseRef, StopLineHandle,
    },
    id::validate_external_id,
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
    /// 禁止进入受控 MovementGate。
    Red,
    /// v0.4 保守策略下禁止进入受控 MovementGate。
    Yellow,
    /// 允许进入受控 MovementGate，但不代表最终 right-of-way。
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

/// MovementGate 的 external signal binding。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignalControlInput {
    Group(String),
    None,
}

/// directed connection 上的 immutable MovementGate 输入定义。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovementGate {
    from_edge_id: String,
    to_edge_id: String,
    stop_line_id: String,
    signal_control: SignalControlInput,
}

impl MovementGate {
    /// 创建待由 `SignalRegistry` normalization 的 MovementGate definition。
    pub fn new(
        from_edge_id: impl Into<String>,
        to_edge_id: impl Into<String>,
        stop_line_id: impl Into<String>,
        signal_control: SignalControlInput,
    ) -> Self {
        Self {
            from_edge_id: from_edge_id.into(),
            to_edge_id: to_edge_id.into(),
            stop_line_id: stop_line_id.into(),
            signal_control,
        }
    }

    pub fn from_edge_id(&self) -> &str {
        &self.from_edge_id
    }

    pub fn to_edge_id(&self) -> &str {
        &self.to_edge_id
    }

    pub fn stop_line_id(&self) -> &str {
        &self.stop_line_id
    }

    pub const fn signal_control(&self) -> &SignalControlInput {
        &self.signal_control
    }
}

/// normalized MovementGate value identity。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MovementGateKey {
    from_edge: EdgeHandle,
    to_edge: EdgeHandle,
}

impl MovementGateKey {
    pub const fn new(from_edge: EdgeHandle, to_edge: EdgeHandle) -> Self {
        Self { from_edge, to_edge }
    }

    pub const fn from_edge(self) -> EdgeHandle {
        self.from_edge
    }

    pub const fn to_edge(self) -> EdgeHandle {
        self.to_edge
    }
}

/// normalized MovementGate signal binding。
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

/// MovementGate 当前 signal binding snapshot。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MovementGateSignalState {
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

/// 当前已提交的 MovementGate snapshot。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MovementGateState {
    key: MovementGateKey,
    stop_line: StopLineHandle,
    signal: MovementGateSignalState,
}

impl MovementGateState {
    /// 返回 Gate value identity。
    pub const fn key(self) -> MovementGateKey {
        self.key
    }

    /// 返回 Gate 使用的 StopLine。
    pub const fn stop_line(self) -> StopLineHandle {
        self.stop_line
    }

    /// 返回当前 signal binding/aspect/permission snapshot。
    pub const fn signal(self) -> MovementGateSignalState {
        self.signal
    }
}

/// Signals 当前已提交的 compact authority snapshot。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SignalRuntimeState {
    controllers: Vec<SignalControllerState>,
    groups: Vec<SignalGroupSnapshot>,
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
struct ResolvedMovementGate {
    definition: MovementGate,
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
    movement_gates: IndexMap<MovementGateKey, ResolvedMovementGate>,
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
            movement_gates: IndexMap::new(),
        }
    }

    /// 按 canonical 顺序 normalization 全部 static Signals definitions。
    pub fn try_new<SL, SG, SC, MG>(
        lane_graph: &LaneGraph,
        stop_lines: SL,
        groups: SG,
        controllers: SC,
        movement_gates: MG,
    ) -> Result<Self, CoreError>
    where
        SL: IntoIterator<Item = StopLine>,
        SG: IntoIterator<Item = SignalGroup>,
        SC: IntoIterator<Item = SignalController>,
        MG: IntoIterator<Item = MovementGate>,
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

        let mut normalized_gates = IndexMap::new();
        let mut gate_identities = IndexSet::new();
        let mut group_usage = vec![false; group_definitions.len()];
        for gate in movement_gates {
            validate_external_id("signals.movementGates[].fromEdgeId", gate.from_edge_id())?;
            validate_external_id("signals.movementGates[].toEdgeId", gate.to_edge_id())?;
            if !gate_identities
                .insert((gate.from_edge_id().to_owned(), gate.to_edge_id().to_owned()))
            {
                return Err(CoreError::DuplicateMovementGate {
                    from_edge_id: gate.from_edge_id().to_owned(),
                    to_edge_id: gate.to_edge_id().to_owned(),
                });
            }
            let from_edge = lane_graph.edge_handle(gate.from_edge_id()).ok_or_else(|| {
                CoreError::UnknownMovementGateFromEdge {
                    edge_id: gate.from_edge_id().to_owned(),
                }
            })?;
            let to_edge = lane_graph.edge_handle(gate.to_edge_id()).ok_or_else(|| {
                CoreError::UnknownMovementGateToEdge {
                    edge_id: gate.to_edge_id().to_owned(),
                }
            })?;
            if !lane_graph.can_traverse(from_edge, to_edge) {
                return Err(CoreError::DisconnectedMovementGate {
                    from_edge_id: gate.from_edge_id().to_owned(),
                    to_edge_id: gate.to_edge_id().to_owned(),
                });
            }
            let key = MovementGateKey::new(from_edge, to_edge);
            validate_external_id("signals.movementGates[].stopLineId", gate.stop_line_id())?;
            let stop_line = stop_line_handles
                .get(gate.stop_line_id())
                .copied()
                .ok_or_else(|| CoreError::UnknownMovementGateStopLine {
                    stop_line_id: gate.stop_line_id().to_owned(),
                })?;
            let normalized_stop_line = &resolved_stop_lines[stop_line.index()];
            if normalized_stop_line.edge != from_edge {
                return Err(CoreError::MovementGateStopLineMismatch {
                    stop_line_id: gate.stop_line_id().to_owned(),
                    stop_line_edge_id: normalized_stop_line.definition.edge_id().to_owned(),
                    from_edge_id: gate.from_edge_id().to_owned(),
                });
            }
            let control = match gate.signal_control() {
                SignalControlInput::Group(group_id) => {
                    validate_external_id(
                        "signals.movementGates[].signalControl.groupId",
                        group_id,
                    )?;
                    let group = group_handles.get(group_id).copied().ok_or_else(|| {
                        CoreError::UnknownMovementGateSignalGroup {
                            group_id: group_id.clone(),
                        }
                    })?;
                    group_usage[group.index()] = true;
                    SignalControl::Group(group)
                }
                SignalControlInput::None => SignalControl::None,
            };
            normalized_gates.insert(
                key,
                ResolvedMovementGate {
                    definition: gate,
                    stop_line,
                    control,
                },
            );
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
                let key = MovementGateKey::new(resolved.edge, *to_edge);
                if !normalized_gates.contains_key(&key) {
                    return Err(CoreError::MissingMovementGateCoverage {
                        stop_line_id: resolved.definition.id().to_owned(),
                        from_edge_id: resolved.definition.edge_id().to_owned(),
                        to_edge_id: lane_graph
                            .edge_external_id(*to_edge)
                            .expect("resolved connection target must exist")
                            .to_owned(),
                    });
                }
            }
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

        Ok(Self {
            stop_lines: resolved_stop_lines,
            stop_line_handles,
            stop_lines_by_edge,
            groups: resolved_groups,
            group_handles,
            controllers: resolved_controllers,
            controller_handles,
            movement_gates: normalized_gates,
        })
    }

    pub(crate) fn rebind_to_lane_graph(self, lane_graph: &LaneGraph) -> Result<Self, CoreError> {
        Self::try_new(
            lane_graph,
            self.stop_lines
                .into_iter()
                .map(|resolved| resolved.definition),
            self.groups.into_iter().map(|resolved| resolved.definition),
            self.controllers
                .into_iter()
                .map(|resolved| resolved.definition),
            self.movement_gates
                .into_values()
                .map(|resolved| resolved.definition),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.stop_lines.is_empty()
            && self.groups.is_empty()
            && self.controllers.is_empty()
            && self.movement_gates.is_empty()
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

    pub fn movement_gate(&self, key: MovementGateKey) -> Option<&MovementGate> {
        self.movement_gates
            .get(&key)
            .map(|resolved| &resolved.definition)
    }

    pub fn movement_gate_stop_line(&self, key: MovementGateKey) -> Option<StopLineHandle> {
        self.movement_gates
            .get(&key)
            .map(|resolved| resolved.stop_line)
    }

    pub fn movement_gate_control(&self, key: MovementGateKey) -> Option<SignalControl> {
        self.movement_gates
            .get(&key)
            .map(|resolved| resolved.control)
    }

    pub fn movement_gates(&self) -> impl ExactSizeIterator<Item = MovementGateKey> + '_ {
        self.movement_gates.keys().copied()
    }

    pub(crate) fn populate_runtime_state(&self, time_ms: u64, state: &mut SignalRuntimeState) {
        state.controllers.clear();
        state.groups.clear();
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
                state.groups[group.index()] =
                    SignalGroupSnapshot::new(phase.aspects_by_group[group_index]);
            }
        }
    }

    pub(crate) fn movement_gate_state(
        &self,
        runtime: &SignalRuntimeState,
        key: MovementGateKey,
    ) -> Option<MovementGateState> {
        let gate = self.movement_gates.get(&key)?;
        let signal = match gate.control {
            SignalControl::None => MovementGateSignalState::Uncontrolled,
            SignalControl::Group(group) => {
                let aspect = runtime.group_state(group)?.aspect();
                let permission = match aspect {
                    SignalAspect::Green => SignalLayerPermission::ProtectedAllow,
                    SignalAspect::Red | SignalAspect::Yellow => SignalLayerPermission::DenyAndStop,
                };
                MovementGateSignalState::Controlled {
                    group,
                    aspect,
                    permission,
                }
            }
        };
        Some(MovementGateState {
            key,
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
}

impl Default for SignalRegistry {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EdgeLength, LaneEdge};

    #[test]
    fn absolute_time_resolver_is_overflow_safe_at_u64_max() {
        let graph = LaneGraph::try_new([
            LaneEdge::new("entry", EdgeLength::try_new(10.0).unwrap(), ["exit"]),
            LaneEdge::new(
                "exit",
                EdgeLength::try_new(10.0).unwrap(),
                Vec::<String>::new(),
            ),
        ])
        .expect("valid graph");
        let registry = SignalRegistry::try_new(
            &graph,
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
            [MovementGate::new(
                "entry",
                "exit",
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
