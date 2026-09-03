//! 以每冲突区的绿灯相位成员判定保护冲突，不枚举门对或反复扫描 controller。
use super::*;
use super::{validation::Coverage, work::WorkBudget};

#[derive(Clone, Copy)]
struct GreenPhase {
    group: MirSignalGroupKey,
    phase: u32,
}

pub(super) struct ProtectedIndex {
    green: Vec<GreenPhase>,
    seen: Vec<Option<(u32, MirConflictZoneKey)>>,
}

impl ProtectedIndex {
    pub(super) fn build(
        unit: &CompilationUnit,
        mir: &MirUnit,
        prior_bytes: u64,
        prior_records: u64,
    ) -> Result<Self, DiagnosticBundle> {
        let count = mir
            .signal_phase_states
            .iter()
            .filter(|v| v.aspect == SignalAspect::Green)
            .count();
        let bytes = (count as u64)
            .saturating_mul(size_of::<GreenPhase>() as u64)
            .saturating_add(
                (mir.signal_phases.len() as u64)
                    .saturating_mul(size_of::<Option<(u32, MirConflictZoneKey)>>() as u64),
            );
        super::validation::budget(
            unit,
            mir,
            prior_bytes.saturating_add(bytes),
            prior_records
                .saturating_add(count as u64)
                .saturating_add(mir.signal_phases.len() as u64),
        )?;
        let mut green = Vec::with_capacity(count);
        for (phase, value) in mir.signal_phases.iter().enumerate() {
            for state in &mir.signal_phase_states[value.states.as_usize_range()] {
                if state.aspect == SignalAspect::Green {
                    green.push(GreenPhase {
                        group: state.signal_group,
                        phase: phase as u32,
                    });
                }
            }
        }
        green.sort_unstable_by_key(|v| (v.group, v.phase));
        Ok(Self {
            green,
            seen: vec![None; mir.signal_phases.len()],
        })
    }

    pub(super) fn bytes(&self) -> u64 {
        (self.green.len() as u64)
            .saturating_mul(size_of::<GreenPhase>() as u64)
            .saturating_add(
                (self.seen.len() as u64)
                    .saturating_mul(size_of::<Option<(u32, MirConflictZoneKey)>>() as u64),
            )
    }

    pub(super) fn records(&self) -> u64 {
        (self.green.len() as u64).saturating_add(self.seen.len() as u64)
    }

    pub(super) fn coherent(
        &mut self,
        mir: &MirUnit,
        policy_index: u32,
        cells: &[Coverage],
        protected: &[bool],
        work: &mut WorkBudget,
    ) -> Result<bool, DiagnosticBundle> {
        let mut zone = None;
        let mut controller = None;
        for cell in cells {
            work.charge(1)?;
            if zone != Some(cell.zone) {
                zone = Some(cell.zone);
                controller = None;
            }
            if !protected[cell.gate.index()] {
                continue;
            }
            let MirSignalControl::Group { signal_group, .. } =
                mir.maneuver_gates[cell.gate.index()].signal_control
            else {
                unreachable!("protected binding validated");
            };
            let start = self.green.partition_point(|v| v.group < signal_group);
            let end = self.green.partition_point(|v| v.group <= signal_group);
            if start == end {
                continue;
            }
            let current = mir.signal_groups[signal_group.index()].controller;
            // 不同控制器的相位没有同步承诺；各有一次绿灯就可能同时放行。
            if controller.is_some_and(|previous| previous != current) {
                return Ok(false);
            }
            controller = Some(current);
            work.charge((end - start) as u64)?;
            for green in &self.green[start..end] {
                let stamp = Some((policy_index, cell.zone));
                let seen = &mut self.seen[green.phase as usize];
                if *seen == stamp {
                    return Ok(false);
                }
                *seen = stamp;
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (CompilationUnit, MirUnit) {
        let unit = crate::compiler::policy_tests::unit_with_control(
            true,
            None,
            Some([SignalAspect::Red; 2]),
            |_, _| {},
        )
        .unwrap();
        let hir = crate::hir::build_hir(&unit).unwrap();
        let mir = super::super::super::lower_to_mir(&unit, &hir).unwrap();
        (unit, mir)
    }

    #[test]
    fn protected_index_matches_phase_pair_oracle_and_isolates_policies_and_zones() {
        let (unit, mut mir) = fixture();
        let a = mir.policies[0].value.gates[0].gate;
        let b = mir.policies[0].value.gates[1].gate;
        let coverage = [
            Coverage {
                gate: a,
                zone: MirConflictZoneKey::from_raw(0),
            },
            Coverage {
                gate: b,
                zone: MirConflictZoneKey::from_raw(0),
            },
        ];
        let group = |gate: MirManeuverGateKey| match mir.maneuver_gates[gate.index()].signal_control
        {
            MirSignalControl::Group { signal_group, .. } => signal_group,
            MirSignalControl::None => unreachable!(),
        };
        let groups = [group(a), group(b)];
        let mut states = Vec::new();
        let template = &mir.signal_phases[0];
        let phases: Vec<_> = (0..2)
            .map(|phase| {
                for group in groups {
                    states.push(super::super::super::MirSignalPhaseState {
                        signal_group: group,
                        aspect: SignalAspect::Red,
                        source_location: mir.signal_phase_states[0].source_location.clone(),
                    });
                }
                super::super::super::MirSignalPhase {
                    module: template.module,
                    stable_key: template.stable_key.clone(),
                    stable_id: template.stable_id,
                    controller: template.controller,
                    duration_ms: template.duration_ms,
                    states: TableRange::try_from_usize(phase * 2, 2).unwrap(),
                    controller_relation_source_location: template
                        .controller_relation_source_location
                        .clone(),
                    source_span: template.source_span.clone(),
                }
            })
            .collect();
        mir.signal_phases = phases.into();
        mir.signal_phase_states = states.into();
        for independent in [false, true] {
            mir.signal_groups[groups[1].index()].controller =
                MirSignalControllerKey::from_raw(u32::from(independent));
            for mask in 0..16 {
                for (i, state) in mir.signal_phase_states.iter_mut().enumerate() {
                    state.aspect = if mask & (1 << i) == 0 {
                        SignalAspect::Red
                    } else {
                        SignalAspect::Green
                    };
                }
                for protected_mask in 0..4 {
                    let mut protected = vec![false; mir.maneuver_gates.len()];
                    protected[a.index()] = protected_mask & 1 != 0;
                    protected[b.index()] = protected_mask & 2 != 0;
                    let green = |phase, gate| mask & (1 << (phase * 2 + gate)) != 0;
                    let conflict = protected_mask == 3
                        && if independent {
                            (0..2).any(|p| green(p, 0)) && (0..2).any(|p| green(p, 1))
                        } else {
                            (0..2).any(|p| green(p, 0) && green(p, 1))
                        };
                    let mut index = ProtectedIndex::build(&unit, &mir, 0, 0).unwrap();
                    for policy in 0..2 {
                        assert_eq!(
                            index
                                .coherent(
                                    &mir,
                                    policy,
                                    &coverage,
                                    &protected,
                                    &mut WorkBudget::new(&unit.limits)
                                )
                                .unwrap(),
                            !conflict
                        );
                    }
                    let separate = [
                        coverage[0],
                        Coverage {
                            gate: b,
                            zone: MirConflictZoneKey::from_raw(1),
                        },
                    ];
                    assert!(
                        index
                            .coherent(
                                &mir,
                                2,
                                &separate,
                                &protected,
                                &mut WorkBudget::new(&unit.limits)
                            )
                            .unwrap()
                    );
                }
            }
        }
    }

    #[test]
    fn large_all_red_zone_does_not_enumerate_gate_pairs() {
        let (unit, mut mir) = fixture();
        let count = 2_000;
        let template = &mir.maneuver_gates[0];
        mir.maneuver_gates = (0..count)
            .map(|_| super::super::super::MirManeuverGate {
                module: template.module,
                stable_key: template.stable_key.clone(),
                stable_id: template.stable_id,
                maneuver_path: template.maneuver_path,
                maneuver_path_source_location: template.maneuver_path_source_location.clone(),
                transition_index: template.transition_index,
                stop_line: template.stop_line,
                stop_line_source_location: template.stop_line_source_location.clone(),
                signal_control: template.signal_control.clone(),
                source_span: template.source_span.clone(),
            })
            .collect();
        let coverage: Vec<_> = (0..count)
            .map(|i| Coverage {
                gate: MirManeuverGateKey::from_raw(i),
                zone: MirConflictZoneKey::from_raw(0),
            })
            .collect();
        let mut index = ProtectedIndex::build(&unit, &mir, 0, 0).unwrap();
        let mut work = WorkBudget::new(&unit.limits);
        assert!(
            index
                .coherent(&mir, 0, &coverage, &vec![true; count as usize], &mut work)
                .unwrap()
        );
        assert_eq!(work.used(), u64::from(count));
    }
}
