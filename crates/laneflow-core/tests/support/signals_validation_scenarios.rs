use laneflow_core::{
    CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge, LaneGraph,
    MovementGate, Route, SignalAspect, SignalControlInput, SignalController, SignalGroup,
    SignalGroupState, SignalPhase, SignalRegistry, Speed, StopLine, StopLineLocation,
    VehicleProfile, VehicleProfileRegistry, VehicleSpawnInput,
};

pub const SIGNAL_VEHICLE_COUNT: usize = 10_000;
pub const SIGNAL_SCALING_VEHICLE_COUNT: usize = 100_000;
pub const SIGNAL_STEP_COUNT: usize = 60;
pub const SIGNAL_FIXED_DELTA_TIME_MS: u64 = 16;
pub const VEHICLES_PER_ROUTE: usize = 25;
pub const GROUPS_PER_CONTROLLER: usize = 4;

const ENTRY_LENGTH: f64 = 180.0;
const EXIT_LENGTH: f64 = 200.0;
const FIRST_VEHICLE_PROGRESS: f64 = 20.0;
const VEHICLE_SPACING: f64 = 6.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalScenarioMode {
    NoSignals,
    AllNone,
    AllGreen,
    AllRed,
    StopRelease,
    MixedOffsets,
}

impl SignalScenarioMode {
    pub const ALL: [Self; 6] = [
        Self::NoSignals,
        Self::AllNone,
        Self::AllGreen,
        Self::AllRed,
        Self::StopRelease,
        Self::MixedOffsets,
    ];

    pub const fn benchmark_name(self) -> &'static str {
        match self {
            Self::NoSignals => "no_signals",
            Self::AllNone => "all_none",
            Self::AllGreen => "all_green",
            Self::AllRed => "all_red",
            Self::StopRelease => "stop_release",
            Self::MixedOffsets => "mixed_offsets",
        }
    }

    const fn has_stop_lines(self) -> bool {
        !matches!(self, Self::NoSignals)
    }

    const fn has_controllers(self) -> bool {
        !matches!(self, Self::NoSignals | Self::AllNone)
    }
}

pub struct SignalScenario {
    pub world: CoreWorld,
    pub route_count: usize,
    pub controller_count: usize,
    pub group_count: usize,
    pub gate_count: usize,
}

fn edge_length(value: f64) -> EdgeLength {
    EdgeLength::try_new(value).expect("signal scenario edge length must be valid")
}

fn progress(value: f64) -> EdgeProgress {
    EdgeProgress::try_new(value).expect("signal scenario progress must be valid")
}

fn speed(value: f64) -> Speed {
    Speed::try_new(value).expect("signal scenario speed must be valid")
}

fn route_id(index: usize) -> String {
    format!("signal-route-{index:05}")
}

fn entry_id(index: usize) -> String {
    format!("signal-entry-{index:05}")
}

fn exit_id(index: usize) -> String {
    format!("signal-exit-{index:05}")
}

fn stop_line_id(index: usize) -> String {
    format!("signal-stop-{index:05}")
}

fn group_id(index: usize) -> String {
    format!("signal-group-{index:05}")
}

fn phase(id: &str, duration_ms: u64, group_ids: &[String], aspect: SignalAspect) -> SignalPhase {
    SignalPhase::new(
        id,
        duration_ms,
        group_ids
            .iter()
            .map(|group| SignalGroupState::new(group.clone(), aspect)),
    )
}

fn phases_for(mode: SignalScenarioMode, group_ids: &[String]) -> Vec<SignalPhase> {
    match mode {
        SignalScenarioMode::AllGreen => {
            vec![phase("green", 60_000, group_ids, SignalAspect::Green)]
        }
        SignalScenarioMode::AllRed => {
            vec![phase("red", 60_000, group_ids, SignalAspect::Red)]
        }
        SignalScenarioMode::StopRelease => vec![
            phase("red", 480, group_ids, SignalAspect::Red),
            phase("green", 480, group_ids, SignalAspect::Green),
        ],
        SignalScenarioMode::MixedOffsets => vec![
            phase("green", 320, group_ids, SignalAspect::Green),
            phase("yellow", 160, group_ids, SignalAspect::Yellow),
            phase("red", 320, group_ids, SignalAspect::Red),
            phase("all-red", 160, group_ids, SignalAspect::Red),
        ],
        SignalScenarioMode::NoSignals | SignalScenarioMode::AllNone => {
            unreachable!("uncontrolled scenarios do not construct controllers")
        }
    }
}

pub fn signal_scenario(vehicle_count: usize, mode: SignalScenarioMode) -> SignalScenario {
    assert_eq!(vehicle_count % VEHICLES_PER_ROUTE, 0);
    let route_count = vehicle_count / VEHICLES_PER_ROUTE;
    assert_eq!(route_count % GROUPS_PER_CONTROLLER, 0);
    let controller_count = if mode.has_controllers() {
        route_count / GROUPS_PER_CONTROLLER
    } else {
        0
    };
    let group_count = if mode.has_controllers() {
        route_count
    } else {
        0
    };
    let gate_count = if mode.has_stop_lines() {
        route_count
    } else {
        0
    };

    let mut edges = Vec::with_capacity(route_count * 2);
    let mut routes = Vec::with_capacity(route_count);
    let mut stop_lines = Vec::with_capacity(gate_count);
    let mut gates = Vec::with_capacity(gate_count);
    let mut groups = Vec::with_capacity(group_count);
    for route_index in 0..route_count {
        let entry = entry_id(route_index);
        let exit = exit_id(route_index);
        edges.push(LaneEdge::new(
            entry.clone(),
            edge_length(ENTRY_LENGTH),
            [exit.clone()],
        ));
        edges.push(LaneEdge::new(
            exit.clone(),
            edge_length(EXIT_LENGTH),
            Vec::<String>::new(),
        ));
        routes.push(
            Route::try_new(route_id(route_index), [entry.clone(), exit.clone()])
                .expect("signal scenario route must be valid"),
        );

        if mode.has_stop_lines() {
            let stop_line = stop_line_id(route_index);
            stop_lines.push(StopLine::new(
                stop_line.clone(),
                entry.clone(),
                StopLineLocation::EdgeEnd,
            ));
            let signal_control = if mode.has_controllers() {
                let group = group_id(route_index);
                groups.push(SignalGroup::new(group.clone()));
                SignalControlInput::Group(group)
            } else {
                SignalControlInput::None
            };
            gates.push(MovementGate::new(entry, exit, stop_line, signal_control));
        }
    }

    let graph = LaneGraph::try_new(edges).expect("signal scenario graph must be valid");
    let controllers = (0..controller_count)
        .map(|controller_index| {
            let first_group = controller_index * GROUPS_PER_CONTROLLER;
            let group_ids = (first_group..first_group + GROUPS_PER_CONTROLLER)
                .map(group_id)
                .collect::<Vec<_>>();
            let offset_ms = if mode == SignalScenarioMode::MixedOffsets {
                (controller_index as u64 * SIGNAL_FIXED_DELTA_TIME_MS) % 960
            } else {
                0
            };
            SignalController::new_fixed_time(
                format!("signal-controller-{controller_index:04}"),
                offset_ms,
                group_ids.clone(),
                phases_for(mode, &group_ids),
            )
        })
        .collect::<Vec<_>>();
    let signals = SignalRegistry::try_new(&graph, stop_lines, groups, controllers, gates)
        .expect("signal scenario registry must be valid");

    let profiles = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        "signal-benchmark-profile",
        IidmProfileSpec {
            length: 4.5,
            desired_speed: 13.9,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 1.4,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 6.0,
        },
    )
    .expect("signal scenario profile must be valid")])
    .expect("signal scenario profile registry must be valid");
    let profile = profiles
        .profile_handle("signal-benchmark-profile")
        .expect("signal scenario profile must exist");
    let traffic = InitialTrafficData::try_new_with_signals(graph, routes, profiles, signals)
        .expect("signal scenario traffic data must be valid");
    let vehicles = (0..route_count)
        .flat_map(|route_index| {
            (0..VEHICLES_PER_ROUTE).map(move |vehicle_index| {
                VehicleSpawnInput::active(
                    format!("signal-vehicle-{route_index:05}-{vehicle_index:02}"),
                    profile,
                    route_id(route_index),
                    0,
                    progress(FIRST_VEHICLE_PROGRESS + VEHICLE_SPACING * vehicle_index as f64),
                    speed(13.9),
                )
            })
        })
        .collect();
    let world = CoreWorld::with_traffic_data(SIGNAL_FIXED_DELTA_TIME_MS, traffic, vehicles)
        .expect("signal scenario world must be valid");

    SignalScenario {
        world,
        route_count,
        controller_count,
        group_count,
        gate_count,
    }
}
