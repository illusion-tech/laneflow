//! WaitingZone static topology normalization。

use std::ops::Range;

use indexmap::IndexMap;

use crate::{
    error::{CoreError, WaitingZoneError},
    handle::{ManeuverGateHandle, ManeuverPathHandle, WaitingZoneHandle},
    id::validate_external_id,
    junction::{JunctionRegistry, validate_capacity},
    signal::SignalRegistry,
};

/// ManeuverPath 上由 entry/release Gate 界定的 immutable 等待空间。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaitingZone {
    id: String,
    maneuver_path_id: String,
    entry_gate_id: String,
    release_gate_id: String,
    max_occupancy: u32,
}

impl WaitingZone {
    /// 创建待由 `WaitingRegistry` normalization 的 WaitingZone definition。
    pub fn new(
        id: impl Into<String>,
        maneuver_path_id: impl Into<String>,
        entry_gate_id: impl Into<String>,
        release_gate_id: impl Into<String>,
        max_occupancy: u32,
    ) -> Self {
        Self {
            id: id.into(),
            maneuver_path_id: maneuver_path_id.into(),
            entry_gate_id: entry_gate_id.into(),
            release_gate_id: release_gate_id.into(),
            max_occupancy,
        }
    }

    /// 返回 WaitingZone external ID。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回声明的 ManeuverPath external ID。
    pub fn maneuver_path_id(&self) -> &str {
        &self.maneuver_path_id
    }

    /// 返回 entry ManeuverGate external ID。
    pub fn entry_gate_id(&self) -> &str {
        &self.entry_gate_id
    }

    /// 返回 release ManeuverGate external ID。
    pub fn release_gate_id(&self) -> &str {
        &self.release_gate_id
    }

    /// 返回同时允许占用该等待空间的车辆上限。
    pub const fn max_occupancy(&self) -> u32 {
        self.max_occupancy
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedWaitingZone {
    definition: WaitingZone,
    maneuver_path: ManeuverPathHandle,
    entry_gate: ManeuverGateHandle,
    release_gate: ManeuverGateHandle,
}

/// immutable WaitingZone normalized registry。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaitingRegistry {
    waiting_zones: Vec<ResolvedWaitingZone>,
    waiting_zone_handles: IndexMap<String, WaitingZoneHandle>,
    waiting_zone_ranges: Vec<Range<usize>>,
    waiting_zones_by_path: Vec<WaitingZoneHandle>,
}

impl WaitingRegistry {
    /// 创建空 WaitingZone registry。
    pub fn empty() -> Self {
        Self {
            waiting_zones: Vec::new(),
            waiting_zone_handles: IndexMap::new(),
            waiting_zone_ranges: Vec::new(),
            waiting_zones_by_path: Vec::new(),
        }
    }

    /// 创建并校验 WaitingZone definitions。
    pub fn try_new<I>(
        junctions: &JunctionRegistry,
        signals: &SignalRegistry,
        waiting_zones: I,
    ) -> Result<Self, CoreError>
    where
        I: IntoIterator<Item = WaitingZone>,
    {
        let definitions = waiting_zones.into_iter().collect::<Vec<_>>();
        validate_capacity("waitingZones", definitions.len())?;

        let mut waiting_zone_handles = IndexMap::new();
        for (index, zone) in definitions.iter().enumerate() {
            validate_external_id("waitingZones[].id", zone.id())?;
            validate_external_id("waitingZones[].maneuverPathId", zone.maneuver_path_id())?;
            validate_external_id("waitingZones[].entryGateId", zone.entry_gate_id())?;
            validate_external_id("waitingZones[].releaseGateId", zone.release_gate_id())?;
            if waiting_zone_handles.contains_key(zone.id()) {
                return Err(WaitingZoneError::DuplicateId {
                    waiting_zone_id: zone.id().to_owned(),
                }
                .into());
            }
            waiting_zone_handles.insert(zone.id().to_owned(), WaitingZoneHandle::new(index));
        }

        let mut normalized = Vec::with_capacity(definitions.len());
        for definition in definitions {
            let maneuver_path = junctions
                .maneuver_path_handle(definition.maneuver_path_id())
                .ok_or_else(|| WaitingZoneError::UnknownPath {
                    waiting_zone_id: definition.id().to_owned(),
                    maneuver_path_id: definition.maneuver_path_id().to_owned(),
                })?;
            let entry_gate =
                resolve_gate(signals, &definition, "entry", definition.entry_gate_id())?;
            let release_gate = resolve_gate(
                signals,
                &definition,
                "release",
                definition.release_gate_id(),
            )?;

            validate_gate_path(signals, &definition, maneuver_path, entry_gate, "entry")?;
            validate_gate_path(signals, &definition, maneuver_path, release_gate, "release")?;

            let entry_transition_index = signals
                .maneuver_gate(entry_gate)
                .expect("resolved entry gate must exist")
                .transition_index();
            let release_transition_index = signals
                .maneuver_gate(release_gate)
                .expect("resolved release gate must exist")
                .transition_index();
            if entry_transition_index >= release_transition_index {
                return Err(WaitingZoneError::InvalidGateOrder {
                    waiting_zone_id: definition.id().to_owned(),
                    entry_transition_index,
                    release_transition_index,
                }
                .into());
            }
            if definition.max_occupancy() == 0 {
                return Err(WaitingZoneError::InvalidMaxOccupancy {
                    waiting_zone_id: definition.id().to_owned(),
                }
                .into());
            }

            normalized.push(ResolvedWaitingZone {
                definition,
                maneuver_path,
                entry_gate,
                release_gate,
            });
        }

        let mut by_path = (0..normalized.len())
            .map(WaitingZoneHandle::new)
            .collect::<Vec<_>>();
        by_path.sort_by_key(|handle| {
            let zone = &normalized[handle.index()];
            (
                zone.maneuver_path.index(),
                signals
                    .maneuver_gate(zone.entry_gate)
                    .expect("resolved entry gate must exist")
                    .transition_index(),
                signals
                    .maneuver_gate(zone.release_gate)
                    .expect("resolved release gate must exist")
                    .transition_index(),
                handle.index(),
            )
        });

        for pair in by_path.array_windows::<2>() {
            let first = &normalized[pair[0].index()];
            let second = &normalized[pair[1].index()];
            if first.maneuver_path != second.maneuver_path {
                continue;
            }
            let first_release = signals
                .maneuver_gate(first.release_gate)
                .expect("resolved release gate must exist")
                .transition_index();
            let second_entry = signals
                .maneuver_gate(second.entry_gate)
                .expect("resolved entry gate must exist")
                .transition_index();
            if second_entry < first_release {
                return Err(WaitingZoneError::Overlap {
                    maneuver_path_id: first.definition.maneuver_path_id().to_owned(),
                    first_waiting_zone_id: first.definition.id().to_owned(),
                    second_waiting_zone_id: second.definition.id().to_owned(),
                }
                .into());
            }
        }

        let mut waiting_zone_ranges = vec![0..0; junctions.maneuver_paths().len()];
        let mut cursor = 0;
        for path in junctions.maneuver_paths() {
            let start = cursor;
            while cursor < by_path.len()
                && normalized[by_path[cursor].index()].maneuver_path == path
            {
                cursor += 1;
            }
            waiting_zone_ranges[path.index()] = start..cursor;
        }

        Ok(Self {
            waiting_zones: normalized,
            waiting_zone_handles,
            waiting_zone_ranges,
            waiting_zones_by_path: by_path,
        })
    }

    /// 按 retained external definitions 对最终 topology 重新 normalization。
    pub(crate) fn rebind_to_static_topology(
        self,
        junctions: &JunctionRegistry,
        signals: &SignalRegistry,
    ) -> Result<Self, CoreError> {
        Self::try_new(
            junctions,
            signals,
            self.waiting_zones
                .into_iter()
                .map(|resolved| resolved.definition),
        )
    }

    /// 返回 WaitingZone definition。
    pub fn waiting_zone(&self, handle: WaitingZoneHandle) -> Option<&WaitingZone> {
        self.waiting_zones
            .get(handle.index())
            .map(|resolved| &resolved.definition)
    }

    /// 按 external ID 返回 WaitingZone handle。
    pub fn waiting_zone_handle(&self, external_id: &str) -> Option<WaitingZoneHandle> {
        self.waiting_zone_handles.get(external_id).copied()
    }

    /// 返回 WaitingZone external ID。
    pub fn waiting_zone_external_id(&self, handle: WaitingZoneHandle) -> Option<&str> {
        self.waiting_zone(handle).map(WaitingZone::id)
    }

    /// 返回 WaitingZone 的 ManeuverPath。
    pub fn waiting_zone_path(&self, handle: WaitingZoneHandle) -> Option<ManeuverPathHandle> {
        self.waiting_zones
            .get(handle.index())
            .map(|zone| zone.maneuver_path)
    }

    /// 返回 WaitingZone entry Gate。
    pub fn waiting_zone_entry_gate(&self, handle: WaitingZoneHandle) -> Option<ManeuverGateHandle> {
        self.waiting_zones
            .get(handle.index())
            .map(|zone| zone.entry_gate)
    }

    /// 返回 WaitingZone release Gate。
    pub fn waiting_zone_release_gate(
        &self,
        handle: WaitingZoneHandle,
    ) -> Option<ManeuverGateHandle> {
        self.waiting_zones
            .get(handle.index())
            .map(|zone| zone.release_gate)
    }

    /// 按 normalization order 遍历 WaitingZone handles。
    pub fn waiting_zones(&self) -> impl ExactSizeIterator<Item = WaitingZoneHandle> + '_ {
        (0..self.waiting_zones.len()).map(WaitingZoneHandle::new)
    }

    /// 按 entry/release transition 顺序返回某 ManeuverPath 上的 WaitingZones。
    pub fn maneuver_path_waiting_zones(
        &self,
        path: ManeuverPathHandle,
    ) -> Option<impl ExactSizeIterator<Item = WaitingZoneHandle> + '_> {
        let range = self.waiting_zone_ranges.get(path.index())?.clone();
        Some(self.waiting_zones_by_path[range].iter().copied())
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        let Self {
            waiting_zones,
            waiting_zone_handles,
            waiting_zone_ranges,
            waiting_zones_by_path,
        } = self;
        waiting_zones.capacity() * std::mem::size_of::<ResolvedWaitingZone>()
            + waiting_zones
                .iter()
                .map(|zone| {
                    zone.definition.id.capacity()
                        + zone.definition.maneuver_path_id.capacity()
                        + zone.definition.entry_gate_id.capacity()
                        + zone.definition.release_gate_id.capacity()
                })
                .sum::<usize>()
            + waiting_zone_handles.capacity() * std::mem::size_of::<(String, WaitingZoneHandle)>()
            + waiting_zone_handles
                .keys()
                .map(String::capacity)
                .sum::<usize>()
            + waiting_zone_ranges.capacity() * std::mem::size_of::<Range<usize>>()
            + waiting_zones_by_path.capacity() * std::mem::size_of::<WaitingZoneHandle>()
    }
}

fn resolve_gate(
    signals: &SignalRegistry,
    zone: &WaitingZone,
    gate_role: &'static str,
    gate_id: &str,
) -> Result<ManeuverGateHandle, CoreError> {
    Ok(signals
        .maneuver_gate_handle(gate_id)
        .ok_or_else(|| WaitingZoneError::UnknownGate {
            waiting_zone_id: zone.id().to_owned(),
            gate_role,
            maneuver_gate_id: gate_id.to_owned(),
        })?)
}

fn validate_gate_path(
    signals: &SignalRegistry,
    zone: &WaitingZone,
    expected_path: ManeuverPathHandle,
    gate: ManeuverGateHandle,
    gate_role: &'static str,
) -> Result<(), CoreError> {
    if signals.maneuver_gate_path(gate) != Some(expected_path) {
        let gate_id = signals
            .maneuver_gate_external_id(gate)
            .expect("resolved gate must exist");
        return Err(WaitingZoneError::GatePathMismatch {
            waiting_zone_id: zone.id().to_owned(),
            gate_role,
            maneuver_gate_id: gate_id.to_owned(),
            maneuver_path_id: zone.maneuver_path_id().to_owned(),
        }
        .into());
    }
    Ok(())
}
