//! #1148 的 1–2 cell 交通语义样本；不生成正式规模制品或 Spatial 几何。
use std::sync::Arc;

use laneflow_compiler::*;
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_static_contract::{
    EntityKind, LaneEdgeId, ParkingSpaceId, ParkingSpaceOrdinal, SignalGroupId, SignalGroupOrdinal,
    VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

use crate::*;

const NS: &str = "runtime-fixture-policy";
pub(super) const EDGE_MM: u32 = 5_130_000;
pub(super) const NOMINAL: [u32; 6] = [30_000, 3_000, 1_000, 20_000, 3_000, 1_000];

pub(super) fn durations(dt: u32) -> [u32; 6] {
    NOMINAL.map(|ms| ms.div_ceil(dt) * dt)
}

pub(super) fn cycle(dt: u32) -> u32 {
    durations(dt).iter().sum()
}

pub(super) fn key(cell: usize, route: usize, name: &str) -> String {
    format!("c{cell:04}-r{route}-{name}")
}

pub(super) fn revision(
    cells: usize,
    dt: u32,
    burst: bool,
    nominal: bool,
) -> Arc<SharedNetworkRevision> {
    assert!((1..=2).contains(&cells), "contract probe only");
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: NS,
            source_document_key: "lf-synth-contract.document",
            generator_build_id: "lf-synth-contract-probe",
            parameters_and_inputs_digest: [0x48; 32],
            frontend_options_digest: [0x14; 32],
            random_seed: Some(0),
            provenance: "repository:laneflow-test",
        },
        &limits,
    )
    .unwrap();
    let mut b = SyntheticModuleBuilder::new(header, &limits).unwrap();
    b.add_participant_class(ParticipantClassInput {
        participant_class_key: "car",
        extends: None,
    })
    .unwrap();
    b.add_vehicle_profile(VehicleProfileInput {
        vehicle_profile_key: "car",
        participant_class: ParticipantClassReference::local("car"),
        iidm: IidmVehicleProfileInput {
            length_meters: 4.5,
            desired_speed_meters_per_second: 13.9,
            min_gap_meters: 2.0,
            time_headway_seconds: 1.5,
            max_acceleration_meters_per_second_squared: 1.4,
            comfortable_deceleration_meters_per_second_squared: 2.0,
            emergency_deceleration_meters_per_second_squared: 6.0,
        },
    })
    .unwrap();
    let mut gate_keys = Vec::new();
    for c in 0..cells {
        let facility = key(c, 0, "facility");
        b.add_parking_facility(ParkingFacilityInput {
            parking_facility_key: &facility,
            virtual_capacity: 0,
            virtual_entries: &[],
            virtual_exits: &[],
        })
        .unwrap();
        let groups: Vec<_> = (0..4).map(|r| key(c, r, "group")).collect();
        for (r, group) in groups.iter().enumerate() {
            let entry = key(c, r, "entry");
            let exit = key(c, r, "exit");
            b.add_lane_edge(LaneEdgeInput {
                lane_edge_key: &entry,
                length_meters: 5_130.0,
                speed_limit_meters_per_second: 13.9,
                successors: &[LaneEdgeReference::local(&exit)],
            })
            .unwrap();
            b.add_lane_edge(LaneEdgeInput {
                lane_edge_key: &exit,
                length_meters: 5_130.0,
                speed_limit_meters_per_second: 13.9,
                successors: &[],
            })
            .unwrap();
            let junction = key(c, r, "junction");
            let movement = key(c, r, "movement");
            let path = key(c, r, "path");
            let stop = key(c, r, "stop");
            let gate = key(c, r, "gate");
            b.add_junction(JunctionInput {
                junction_key: &junction,
            })
            .unwrap();
            b.add_movement(MovementInput {
                movement_key: &movement,
                junction: JunctionReference::local(&junction),
                directed_entry_approach_key: "entry",
                directed_exit_approach_key: "exit",
                turn_direction: None,
            })
            .unwrap();
            b.add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: &path,
                movement: MovementReference::local(&movement),
                entry_edge: LaneEdgeReference::local(&entry),
                internal_edges: &[],
                exit_edge: LaneEdgeReference::local(&exit),
            })
            .unwrap();
            b.add_stop_line(StopLineInput {
                stop_line_key: &stop,
                lane_edge: LaneEdgeReference::local(&entry),
            })
            .unwrap();
            b.add_signal_group(SignalGroupInput {
                signal_group_key: group,
            })
            .unwrap();
            b.add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: &gate,
                maneuver_path: ManeuverPathReference::local(&path),
                transition_index: 0,
                stop_line: StopLineReference::local(&stop),
                signal_control: SignalControlInput::Group(SignalGroupReference::local(group)),
            })
            .unwrap();
            gate_keys.push(gate);
            for s in 0..25 {
                let anchor = || ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local(&exit),
                    progress_meters: 5.0 + 7.5 * f64::from(s),
                };
                b.add_parking_space(ParkingSpaceInput {
                    parking_space_key: &key(c, r, &format!("space{s:02}")),
                    parking_facility: Some(ParkingFacilityReference::local(&facility)),
                    entry: anchor(),
                    exit: anchor(),
                    geometry: ParkingSpaceGeometryInput {
                        lateral_offset_meters: 3.0,
                        heading_offset_radians: 0.0,
                        length_meters: 5.0,
                        width_meters: 2.4,
                    },
                })
                .unwrap();
            }
        }
        let phase_keys: Vec<_> = (0..6).map(|p| key(c, 0, &format!("phase{p}"))).collect();
        let states: Vec<Vec<_>> = (0..6)
            .map(|p| {
                groups
                    .iter()
                    .enumerate()
                    .map(|(r, g)| {
                        let aspect = match (p, r % 2) {
                            (0, 0) | (3, 1) => SignalAspect::Green,
                            (1, 0) | (4, 1) => SignalAspect::Yellow,
                            _ => SignalAspect::Red,
                        };
                        SignalGroupStateInput {
                            signal_group: SignalGroupReference::local(g),
                            aspect,
                        }
                    })
                    .collect()
            })
            .collect();
        let ms = if nominal { NOMINAL } else { durations(dt) };
        let phases: Vec<_> = (0..6)
            .map(|p| SignalPhaseInput {
                signal_phase_key: &phase_keys[p],
                duration_ms: u64::from(ms[p]),
                states: &states[p],
            })
            .collect();
        let refs: Vec<_> = groups
            .iter()
            .map(|g| SignalGroupReference::local(g))
            .collect();
        b.add_signal_controller(SignalControllerInput {
            signal_controller_key: &key(c, 0, "controller"),
            offset_ms: u64::from(if burst {
                cycle(dt) - dt
            } else {
                (c as u32 * 997) % cycle(dt)
            }),
            signal_groups: &refs,
            phases: &phases,
        })
        .unwrap();
    }
    let gates: Vec<_> = gate_keys
        .iter()
        .map(|g| (g.as_str(), GateInterpretation::ProtectedGroup))
        .collect();
    crate::test_policy::add_gate_policy(&mut b, "fixture-policy", &gates);
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(b.finish().unwrap()).unwrap();
    let output = Compiler::new().compile(unit.build().unwrap()).unwrap();
    let candidate = emit_portable_candidate(
        &output,
        &PortableEmissionProvenance::try_new("lf-synth-contract-probe").unwrap(),
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    let checked = check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .unwrap();
    build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .unwrap()
}

pub(super) fn install(
    rev: Arc<SharedNetworkRevision>,
    dt: u32,
) -> Result<TrafficWorld, InstallError> {
    let o = *rev.canonical_origin();
    let selection = crate::test_policy::selection(&rev);
    TrafficWorld::install(
        rev,
        WorldConfig::new(256, 16, 4_096, 4_096, u64::from(dt)),
        ExecutionConfig::new(std::num::NonZeroU32::MIN),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://lf-synth-contract",
                o.canonical_artifact_digest(),
                o.canonical_artifact_byte_length(),
                o.network_revision(),
            )
            .unwrap(),
        },
        0,
        selection,
    )
}

pub(super) struct Sample {
    pub world: TrafficWorld,
    pub handles: Vec<VehicleHandle>,
    pub routes: Vec<RouteHandle>,
    pub spaces: Vec<ParkingSpaceOrdinal>,
    pub cells: usize,
    pub dt: u32,
    groups: Vec<SignalGroupOrdinal>,
    case: Case,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Case {
    W1,
    W2,
    W3,
    W4,
}

impl Sample {
    pub fn new(cells: usize, dt: u32, case: Case) -> Self {
        let mut world = install(revision(cells, dt, case == Case::W4, false), dt).unwrap();
        let mut routes = Vec::new();
        let mut spaces = Vec::new();
        let mut handles = Vec::new();
        let mut groups = Vec::new();
        let limits = CompileLimits::p100_initial_v1();
        for c in 0..cells {
            for r in 0..4 {
                let id = derive_canonical_stable_id_v1(
                    EntityKind::SignalGroup,
                    NS,
                    &key(c, r, "group"),
                    &limits,
                )
                .unwrap();
                groups.push(
                    world
                        .revision()
                        .identity()
                        .ordinal(SignalGroupId::from_untyped(id))
                        .unwrap(),
                );
                let ids: Vec<_> = ["entry", "exit"]
                    .iter()
                    .map(|suffix| {
                        let id = derive_canonical_stable_id_v1(
                            EntityKind::LaneEdge,
                            NS,
                            &key(c, r, suffix),
                            &limits,
                        )
                        .unwrap();
                        world
                            .revision()
                            .identity()
                            .ordinal(LaneEdgeId::from_untyped(id))
                            .unwrap()
                    })
                    .collect();
                let route = world.register_route(RouteRegisterInput::new(ids)).unwrap();
                routes.push(route);
                for s in 0..25 {
                    let id = derive_canonical_stable_id_v1(
                        EntityKind::ParkingSpace,
                        NS,
                        &key(c, r, &format!("space{s:02}")),
                        &limits,
                    )
                    .unwrap();
                    let space = world
                        .revision()
                        .identity()
                        .ordinal(ParkingSpaceId::from_untyped(id))
                        .unwrap();
                    spaces.push(space);
                    let active = match case {
                        Case::W1 | Case::W4 => r < 2 || s < if r == 2 { 12 } else { 13 },
                        Case::W2 => true,
                        Case::W3 => s < if r == 3 { 7 } else { 6 },
                    };
                    let handle = if active {
                        let dense = case == Case::W2 || (case != Case::W3 && r < 2);
                        world
                            .spawn_vehicle(VehicleSpawnInput::new(
                                VehicleProfileOrdinal::from_raw(0),
                                route,
                                0,
                                20_000 + s * if dense { 6_500 } else { 28_000 },
                                if dense { 0 } else { 13_900 },
                            ))
                            .unwrap()
                    } else {
                        world
                            .spawn_parked_vehicle(
                                ParkedVehicleSpawnInput::new(
                                    VehicleProfileOrdinal::from_raw(0),
                                    route,
                                    1,
                                    5_000 + 7_500 * s,
                                ),
                                ParkingTarget::ExplicitSpace(space),
                            )
                            .unwrap()
                            .vehicle
                    };
                    handles.push(handle);
                }
            }
        }
        Self {
            world,
            handles,
            routes,
            spaces,
            cells,
            dt,
            groups,
            case,
        }
    }

    pub fn tick(&mut self) -> StepOutcome {
        let result = self.world.step(TickInput::new(u64::from(self.dt))).unwrap();
        self.signals();
        result
    }

    pub fn signals(&self) {
        let observed = self.world.committed_signal_groups();
        assert_eq!(observed.as_slice().len(), self.cells * 4);
        for c in 0..self.cells {
            let offset = if self.case == Case::W4 {
                cycle(self.dt) - self.dt
            } else {
                (c as u32 * 997) % cycle(self.dt)
            };
            let mut position = ((self.world.tick_index() * u64::from(self.dt) + u64::from(offset))
                % u64::from(cycle(self.dt))) as u32;
            let phase = durations(self.dt)
                .into_iter()
                .position(|duration| {
                    if position < duration {
                        true
                    } else {
                        position -= duration;
                        false
                    }
                })
                .unwrap();
            for r in 0..4 {
                let expected = match (phase, r % 2) {
                    (0, 0) | (3, 1) => SignalAspect::Green,
                    (1, 0) | (4, 1) => SignalAspect::Yellow,
                    _ => SignalAspect::Red,
                };
                assert_eq!(
                    observed
                        .as_slice()
                        .iter()
                        .find(|(g, _)| *g == self.groups[c * 4 + r])
                        .unwrap()
                        .1,
                    expected,
                    "signal cell={c} route={r} tick={}",
                    self.world.tick_index()
                );
            }
        }
    }
    pub fn population(&self, active: usize, parked: usize, completed: usize) {
        let mut counts = [0; 3];
        for &h in &self.handles {
            counts[match self.world.vehicle(h).unwrap().status() {
                VehicleStatus::Active => 0,
                VehicleStatus::Parked => 1,
                VehicleStatus::Completed => 2,
            }] += 1;
        }
        assert_eq!(
            counts,
            [
                active * self.cells,
                parked * self.cells,
                completed * self.cells
            ]
        );
    }
}
