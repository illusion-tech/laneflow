//! 语义预检后的本地所有权闭合；排序只影响查询，不改变诊断遍历顺序。

use super::scratch::{PreflightScratch, ScratchVec};
use super::{DiagnosticBundle, invalid_combination, wire};

type ChildKey<'a> = (&'a str, &'a str);

struct OwnerIndex<'scratch, 'limits, 'source> {
    entries: ScratchVec<'scratch, 'limits, (ChildKey<'source>, u8)>,
}

impl<'scratch, 'limits, 'source> OwnerIndex<'scratch, 'limits, 'source> {
    fn new(
        keys: impl ExactSizeIterator<Item = ChildKey<'source>>,
        scratch: &'scratch PreflightScratch<'limits>,
    ) -> Result<Self, DiagnosticBundle> {
        let mut entries = scratch.collect(keys.map(|key| (key, 0)))?;
        entries.sort_unstable_by_key(|entry| entry.0);
        Ok(Self { entries })
    }

    fn find(&self, reference: &str) -> Option<usize> {
        let key = local_child_key(reference)?;
        self.entries
            .binary_search_by_key(&key, |entry| entry.0)
            .ok()
    }

    fn claim(&mut self, reference: &str) -> bool {
        let Some(index) = self.find(reference) else {
            return false;
        };
        let count = &mut self.entries[index].1;
        *count = count.saturating_add(1);
        true
    }

    fn all_owned_once(&self) -> bool {
        self.entries.iter().all(|entry| entry.1 == 1)
    }
}

fn local_child_key(reference: &str) -> Option<ChildKey<'_>> {
    // 调用点已经完成精确深度与 token 语法校验；本地所有权仍须拒绝导入限定。
    if reference.contains("::") {
        return None;
    }
    reference.rsplit_once('>')
}

pub(super) fn validate(
    root: wire::RoadEditingSource<'_>,
    scratch: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    let mut junction_keys =
        scratch.collect(root.junctions().iter().map(|value| value.junction_key()))?;
    junction_keys.sort_unstable();
    for movement in root.movements() {
        if junction_keys.binary_search(&movement.junction()).is_err() {
            return Err(invalid_combination("movement.junction", expected_key));
        }
    }
    drop(junction_keys);

    let movement_keys = OwnerIndex::new(
        root.movements()
            .iter()
            .map(|value| (value.junction(), value.movement_key())),
        scratch,
    )?;
    for path in root.maneuver_paths() {
        if movement_keys.find(path.movement()).is_none() {
            return Err(invalid_combination("maneuverPath.movement", expected_key));
        }
    }
    drop(movement_keys);

    let path_keys = OwnerIndex::new(
        root.maneuver_paths()
            .iter()
            .map(|value| (value.movement(), value.maneuver_path_key())),
        scratch,
    )?;
    for gate in root.maneuver_gates() {
        if path_keys.find(gate.maneuver_path()).is_none() {
            return Err(invalid_combination(
                "maneuverGate.maneuverPath",
                expected_key,
            ));
        }
    }
    for zone in root.waiting_zones() {
        if path_keys.find(zone.maneuver_path()).is_none() {
            return Err(invalid_combination(
                "waitingZone.maneuverPath",
                expected_key,
            ));
        }
    }
    drop(path_keys);

    let mut sections = OwnerIndex::new(
        root.road_sections()
            .iter()
            .map(|value| (value.road_corridor(), value.road_section_key())),
        scratch,
    )?;
    for group in root.lane_groups() {
        if sections.find(group.road_section()).is_none() {
            return Err(invalid_combination("laneGroup.roadSection", expected_key));
        }
    }
    for corridor in root.road_corridors() {
        for element in corridor.elements() {
            // corridor-owned reference 的首组件已由字段预检证明等于当前 corridor。
            if element.kind() == wire::CorridorElementKind::RoadSection
                && !sections.claim(element.entity_reference())
            {
                return Err(invalid_combination("roadCorridor.elements", expected_key));
            }
        }
    }
    if !sections.all_owned_once() {
        return Err(invalid_combination("roadCorridor.elements", expected_key));
    }
    drop(sections);

    let mut bands = OwnerIndex::new(
        root.facility_bands()
            .iter()
            .map(|value| (value.road_corridor(), value.facility_band_key())),
        scratch,
    )?;
    for corridor in root.road_corridors() {
        for element in corridor.elements() {
            if element.kind() == wire::CorridorElementKind::FacilityBand
                && !bands.claim(element.entity_reference())
            {
                return Err(invalid_combination("roadCorridor.elements", expected_key));
            }
        }
    }
    if !bands.all_owned_once() {
        return Err(invalid_combination("roadCorridor.elements", expected_key));
    }
    drop(bands);

    let mut lanes = OwnerIndex::new(
        root.authoring_lanes()
            .iter()
            .map(|value| (value.road_section(), value.authoring_lane_key())),
        scratch,
    )?;
    for section in root.road_sections() {
        for reference in section.authoring_lanes() {
            let matches_owner = local_child_key(reference)
                .and_then(|(parent, _)| local_child_key(parent))
                == Some((section.road_corridor(), section.road_section_key()));
            if !matches_owner || !lanes.claim(reference) {
                return Err(invalid_combination(
                    "roadSection.authoringLanes",
                    expected_key,
                ));
            }
        }
    }
    if !lanes.all_owned_once() {
        return Err(invalid_combination(
            "roadSection.authoringLanes",
            expected_key,
        ));
    }
    drop(lanes);

    validate_signals(root, scratch, expected_key)
}

fn validate_signals(
    root: wire::RoadEditingSource<'_>,
    scratch: &PreflightScratch<'_>,
    expected_key: &str,
) -> Result<(), DiagnosticBundle> {
    let groups = root.signal_groups();
    let phases = root.signal_phases();
    let controllers = root.signal_controllers();
    let mut group_order = scratch.collect(0..groups.len())?;
    group_order.sort_unstable_by_key(|index| groups.get(*index).signal_group_key());
    let mut group_owner_counts = scratch.collect((0..groups.len()).map(|_| 0_u8))?;
    for controller in controllers {
        for reference in controller.signal_groups() {
            if reference.contains("::") {
                return Err(invalid_combination(
                    "signalController.signalGroups",
                    expected_key,
                ));
            }
            let Ok(position) = group_order
                .binary_search_by_key(&reference, |index| groups.get(*index).signal_group_key())
            else {
                return Err(invalid_combination(
                    "signalController.signalGroups",
                    expected_key,
                ));
            };
            let count = &mut group_owner_counts[position];
            *count = count.saturating_add(1);
            if *count != 1 {
                return Err(invalid_combination(
                    "signalController.signalGroups",
                    expected_key,
                ));
            }
        }
    }
    if group_owner_counts.iter().any(|count| *count != 1) {
        return Err(invalid_combination(
            "signalController.signalGroups",
            expected_key,
        ));
    }
    drop(group_owner_counts);
    drop(group_order);

    let mut phase_order = scratch.collect(0..phases.len())?;
    phase_order.sort_unstable_by_key(|index| {
        let phase = phases.get(*index);
        (phase.signal_controller(), phase.signal_phase_key())
    });
    let mut referenced_phase_count = 0_usize;
    for controller in controllers {
        for reference in controller.signal_phases() {
            let Some((owner_key, phase_key)) = local_child_key(reference) else {
                return Err(invalid_combination(
                    "signalController.signalPhases",
                    expected_key,
                ));
            };
            if owner_key != controller.signal_controller_key()
                || phase_order
                    .binary_search_by_key(&(owner_key, phase_key), |index| {
                        let phase = phases.get(*index);
                        (phase.signal_controller(), phase.signal_phase_key())
                    })
                    .is_err()
            {
                return Err(invalid_combination(
                    "signalController.signalPhases",
                    expected_key,
                ));
            }
            referenced_phase_count = referenced_phase_count.saturating_add(1);
        }
    }
    if referenced_phase_count != phases.len() {
        return Err(invalid_combination(
            "signalController.signalPhases",
            expected_key,
        ));
    }
    drop(phase_order);

    let mut controller_order = scratch.collect(0..controllers.len())?;
    controller_order.sort_unstable_by_key(|index| controllers.get(*index).signal_controller_key());
    for phase in phases {
        let Ok(position) = controller_order
            .binary_search_by_key(&phase.signal_controller(), |index| {
                controllers.get(*index).signal_controller_key()
            })
        else {
            return Err(invalid_combination(
                "signalPhase.signalController",
                expected_key,
            ));
        };
        let controller = controllers.get(controller_order[position]);
        let mut expected_groups = scratch.collect(controller.signal_groups().iter())?;
        expected_groups.sort_unstable();
        let states = phase.states();
        if states.len() != expected_groups.len()
            || states.iter().any(|state| {
                expected_groups
                    .binary_search(&state.signal_group())
                    .is_err()
            })
        {
            return Err(invalid_combination(
                "signalPhase.states.signalGroup",
                expected_key,
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CompileLimits;

    #[test]
    fn owner_index_distinguishes_full_addresses_and_counts_claims() {
        let limits = CompileLimits::p100_initial_v1();
        let scratch = PreflightScratch::new(&limits, 0);
        let mut index = OwnerIndex::new(
            [("a>section", "lane"), ("b>section", "lane")].into_iter(),
            &scratch,
        )
        .unwrap();
        assert!(index.find("a>section>lane").is_some());
        assert!(index.find("b>section>lane").is_some());
        assert!(index.find("c>section>lane").is_none());
        assert!(index.find("a>other>lane").is_none());
        assert!(index.find("city::a>section>lane").is_none());
        assert!(!index.all_owned_once());
        assert!(index.claim("a>section>lane"));
        assert!(!index.all_owned_once());
        assert!(index.claim("b>section>lane"));
        assert!(index.all_owned_once());
        assert!(index.claim("a>section>lane"));
        assert!(!index.all_owned_once());
        for _ in 0..256 {
            assert!(index.claim("a>section>lane"));
        }
        assert!(!index.all_owned_once());
    }
}
