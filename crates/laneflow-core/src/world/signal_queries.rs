use super::*;

impl CoreWorld {
    /// 返回当前已提交的 controller snapshot。
    pub fn signal_controller_state(
        &self,
        handle: SignalControllerHandle,
    ) -> Option<SignalControllerState> {
        self.signal_state.controller_state(handle)
    }

    /// 按 controller normalization order 遍历当前 snapshots。
    pub fn signal_controller_states(
        &self,
    ) -> impl ExactSizeIterator<Item = (SignalControllerHandle, SignalControllerState)> + '_ {
        self.signal_state.controller_states()
    }

    /// 返回当前已提交的 SignalGroup snapshot。
    pub fn signal_group_state(&self, handle: SignalGroupHandle) -> Option<SignalGroupSnapshot> {
        self.signal_state.group_state(handle)
    }

    /// 按 SignalGroup normalization order 遍历当前 snapshots。
    pub fn signal_group_states(
        &self,
    ) -> impl ExactSizeIterator<Item = (SignalGroupHandle, SignalGroupSnapshot)> + '_ {
        self.signal_state.group_states()
    }

    /// 返回当前已提交的 ManeuverGate signal-layer snapshot。
    pub fn maneuver_gate_state(&self, gate: ManeuverGateHandle) -> Option<ManeuverGateState> {
        self.signals.maneuver_gate_state(&self.signal_state, gate)
    }

    /// 按 ManeuverGate normalization order 遍历当前 snapshots。
    pub fn maneuver_gate_states(&self) -> impl ExactSizeIterator<Item = ManeuverGateState> + '_ {
        self.signals.maneuver_gates().map(|gate| {
            self.maneuver_gate_state(gate)
                .expect("normalized ManeuverGate must have runtime state")
        })
    }

}
