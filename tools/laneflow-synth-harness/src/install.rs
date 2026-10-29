use std::num::NonZeroU32;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::*;
use laneflow_static_contract::*;
use laneflow_static_network::*;
use laneflow_synth_generator::{Bundle, InitialState};

use crate::{Error, failure};

pub(crate) struct Sample {
    pub world: TrafficWorld,
    pub state: InitialState,
    pub handles: Vec<VehicleHandle>,
    pub routes: Vec<RouteHandle>,
    pub groups: Vec<SignalGroupOrdinal>,
    pub gates: Vec<ManeuverGateOrdinal>,
    pub initial: Vec<VehicleState>,
    pub previous: Vec<VehicleState>,
    pub bindings: Vec<Option<ParkingBinding>>,
    pub crossed: Vec<bool>,
    pub transitions: u64,
    pub max_deceleration_mm_s2: u64,
    pub deceleration_above_profile_count: u64,
    pub cursor: u64,
    pub trace_bytes: Vec<u8>,
}

impl Sample {
    pub fn new(bundle: &Bundle) -> Result<Self, Error> {
        let input = check_canonical_network_input(
            bundle.files()["network.lfca"].as_slice(),
            FormatLimits::HARD,
        )
        .map_err(|e| failure("LFCA", e))?;
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(2 * 1_024 * 1_024 * 1_024, 2 * 1_024 * 1_024 * 1_024),
            ),
        )
        .map_err(|e| failure("shared revision", e))?;
        let state: InitialState = serde_json::from_slice(&bundle.files()["state.json"])?;
        let identity = revision.identity();
        let profile = identity
            .ordinal(
                state
                    .vehicle_profile_id
                    .parse::<VehicleProfileId>()
                    .map_err(|e| failure("profile id", e))?,
            )
            .ok_or_else(|| failure("profile", "missing"))?;
        let origin = *revision.canonical_origin();
        let source = PublishedLfcaReference::new(
            "fixture://lf-synth-harness",
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .map_err(|e| failure("source", e))?;
        let mut world = TrafficWorld::install(
            revision.clone(),
            WorldConfig::new(
                state.identity.scale,
                state.routes.len() as u32,
                state.routes.len() as u64 * 2,
                0,
                u64::from(state.identity.fixed_delta_ms),
            ),
            ExecutionConfig::new(NonZeroU32::MIN),
            CommittedNetworkSource::Published { reference: source },
            0,
            WorldPolicySelection::Pinned(PolicyPin {
                policy: state
                    .policy_id
                    .parse()
                    .map_err(|e| failure("policy id", e))?,
            }),
        )
        .map_err(|e| failure("install", e))?;
        let mut routes = Vec::new();
        let mut groups = Vec::new();
        let mut gates = Vec::new();
        for route in &state.routes {
            let edges = route
                .edge_ids
                .iter()
                .map(|id| {
                    let id = id
                        .parse::<LaneEdgeId>()
                        .map_err(|e| failure("edge id", e))?;
                    identity
                        .ordinal(id)
                        .ok_or_else(|| failure("edge", "missing"))
                })
                .collect::<Result<Vec<_>, Error>>()?;
            let group = identity
                .ordinal(
                    route
                        .signal_group_id
                        .parse::<SignalGroupId>()
                        .map_err(|e| failure("group id", e))?,
                )
                .ok_or_else(|| failure("group", "missing"))?;
            let group_view = revision
                .traffic()
                .relations()
                .signal_group(group)
                .ok_or_else(|| failure("group relations", "missing"))?;
            crate::require(group_view.gates().len() == 1, || {
                "每条路线必须有唯一信号 Gate".into()
            })?;
            gates.push(group_view.gates()[0]);
            routes.push(
                world
                    .register_route(RouteRegisterInput::new(edges))
                    .map_err(|e| failure("route", e))?,
            );
            groups.push(group);
        }
        let mut handles = Vec::new();
        for vehicle in &state.vehicles {
            let route = routes[vehicle.route_rank as usize];
            let handle = if vehicle.status == "active" {
                world
                    .spawn_vehicle(VehicleSpawnInput::new(
                        profile,
                        route,
                        vehicle.edge_occurrence,
                        vehicle.progress_mm,
                        vehicle.speed_mm_per_s,
                    ))
                    .map_err(|e| failure("spawn", e))?
            } else {
                let space = identity
                    .ordinal(
                        vehicle
                            .parking_space_id
                            .parse::<ParkingSpaceId>()
                            .map_err(|e| failure("parking id", e))?,
                    )
                    .ok_or_else(|| failure("parking space", "missing"))?;
                world
                    .spawn_parked_vehicle(
                        ParkedVehicleSpawnInput::new(
                            profile,
                            route,
                            vehicle.edge_occurrence,
                            vehicle.progress_mm,
                        ),
                        ParkingTarget::ExplicitSpace(space),
                    )
                    .map_err(|e| failure("parked spawn", e))?
                    .vehicle
            };
            handles.push(handle);
        }
        let initial: Vec<_> = handles.iter().map(|&h| world.vehicle(h).unwrap()).collect();
        let bindings = handles.iter().map(|&h| world.parking_binding(h)).collect();
        let cursor = world.command_cursor();
        Ok(Self {
            world,
            state,
            handles,
            routes,
            groups,
            gates,
            previous: initial.clone(),
            crossed: vec![false; initial.len()],
            trace_bytes: Vec::with_capacity(initial.len() * 32),
            initial,
            bindings,
            transitions: 0,
            max_deceleration_mm_s2: 0,
            deceleration_above_profile_count: 0,
            cursor,
        })
    }
}
