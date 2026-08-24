//! 信号领域 Canonical LIR 记录。

use laneflow_static_contract::{
    ManeuverGateOrdinal, SignalAspect, SignalControllerId, SignalControllerOrdinal, SignalGroupId,
    SignalGroupOrdinal, SignalPhaseId, SignalPhaseOrdinal,
};

use crate::arena::TableRange;

use super::LirIdentityField;

#[derive(Clone, Copy)]
pub(crate) enum LirSignalControl {
    Group(SignalGroupOrdinal),
    None,
}

pub(crate) struct LirSignalGroup {
    pub(crate) ordinal: SignalGroupOrdinal,
    pub(crate) stable_id: SignalGroupId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) controller: SignalControllerOrdinal,
    pub(crate) maneuver_gates: TableRange<ManeuverGateOrdinal>,
}

pub(crate) struct LirSignalController {
    pub(crate) ordinal: SignalControllerOrdinal,
    pub(crate) stable_id: SignalControllerId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) offset_ms: u64,
    pub(crate) cycle_duration_ms: u64,
    pub(crate) signal_groups: TableRange<SignalGroupOrdinal>,
    pub(crate) phases: TableRange<SignalPhaseOrdinal>,
}

pub(crate) struct LirSignalPhase {
    pub(crate) ordinal: SignalPhaseOrdinal,
    pub(crate) stable_id: SignalPhaseId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) controller: SignalControllerOrdinal,
    pub(crate) duration_ms: u64,
    pub(crate) states: TableRange<LirSignalPhaseState>,
}

pub(crate) struct LirSignalPhaseState {
    pub(crate) signal_group: SignalGroupOrdinal,
    pub(crate) aspect: SignalAspect,
}
