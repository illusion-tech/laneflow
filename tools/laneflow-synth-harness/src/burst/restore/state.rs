use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_static_contract::*;
use laneflow_static_network::*;
use laneflow_synth_generator::InitialState;
use serde::{Deserialize, Serialize};

use super::*;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Host {
    pub tick: u64,
    pub after_commands: bool,
    pub command_count: u64,
    pub blocked: u64,
    pub batch_count: usize,
    pub trace_offset: u64,
    pub event_offset: usize,
    pub vehicle_ids: Vec<u64>,
    pub route_ids: Vec<u64>,
    pub projection: Value,
    pub source_digest: [u8; 32],
    pub source_length: u64,
    pub revision: [u8; 32],
    pub crossed: Vec<bool>,
    pub transitions: u64,
    pub max_deceleration: u64,
    pub excessive_decelerations: u64,
    pub burst: Option<BurstHost>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BurstHost {
    live_ranks: Vec<usize>,
    incarnations: Vec<u32>,
    terminal: Vec<bool>,
    arrivals: Vec<Option<u64>>,
    completions: Vec<(u64, u32)>,
    arrival_records: Vec<(u64, u32)>,
    pending: Vec<Command>,
    parked_boundary: Option<u64>,
    left: bool,
    counts: [u32; 3],
    total_crossings: Vec<u32>,
}

pub(super) fn projection(sample: &Sample) -> Result<Value, Error> {
    let world = &sample.world;
    let snapshot = world
        .capture_snapshot()
        .map_err(|e| failure("capture", e))?;
    let ranks: HashMap<_, _> = sample
        .handles
        .iter()
        .copied()
        .enumerate()
        .map(|(r, h)| (h, r))
        .collect();
    let live = world
        .live_vehicles()
        .iter()
        .map(|h| {
            ranks
                .get(h)
                .copied()
                .ok_or_else(|| failure("live mapping", h))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let vehicles = sample
        .handles
        .iter()
        .enumerate()
        .map(|(rank, &handle)| {
            let state = world
                .vehicle(handle)
                .ok_or_else(|| failure("vehicle mapping", rank))?;
            require(state.route() == sample.routes[rank / 25], || {
                "车辆路线归属不符".into()
            })?;
            let binding = match world.parking_binding(handle) {
                None => json!(null),
                Some(ParkingBinding::Occupied(ParkingTarget::ExplicitSpace(space))) => {
                    json!([2, space.raw()])
                }
                Some(ParkingBinding::Reserved(r)) => {
                    require(r.route() == sample.routes[rank / 25], || {
                        "预约路线不符".into()
                    })?;
                    let ParkingTarget::ExplicitSpace(space) = r.target() else {
                        return Err(failure("parking target", rank));
                    };
                    json!([1, space.raw(), r.entry_route_occurrence()])
                }
                _ => return Err(failure("parking binding", rank)),
            };
            Ok(json!([
                rank,
                rank / 25,
                state.profile().raw(),
                state.class().raw(),
                state.length_mm(),
                state.route_edge_index(),
                state.progress_mm(),
                state.carry_um(),
                state.speed_mm_s(),
                status(state.status()),
                binding
            ]))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(
        json!({"tick":snapshot.tick(),"time_ms":snapshot.time_ms(),"command_cursor":snapshot.command_cursor(),"event_cursor":snapshot.event_cursor(),"state_digest":state_digest(world)?,"live":live,"vehicles":vehicles}),
    )
}

pub(super) fn status(value: VehicleStatus) -> u32 {
    match value {
        VehicleStatus::Active => 0,
        VehicleStatus::Parked => 1,
        VehicleStatus::Completed => 2,
    }
}

pub(super) fn capture(
    driver: &Driver,
    records: &Records,
    after_commands: bool,
) -> Result<(Host, Vec<u8>), Error> {
    let sample = driver.sample();
    let snapshot = sample
        .world
        .capture_snapshot()
        .map_err(|e| failure("capture", e))?;
    let by_id: BTreeMap<_, _> = snapshot
        .vehicles()
        .iter()
        .map(|v| (v.snapshot_vehicle_id(), v))
        .collect();
    let ranks: HashMap<_, _> = sample
        .handles
        .iter()
        .copied()
        .enumerate()
        .map(|(r, h)| (h, r))
        .collect();
    let mut vehicle_ids = vec![0; sample.handles.len()];
    let mut route_ids = vec![None; sample.routes.len()];
    require(
        snapshot.live_order().len() == sample.world.live_vehicles().len(),
        || "捕获 live 顺序长度不符".into(),
    )?;
    for (&id, handle) in snapshot
        .live_order()
        .iter()
        .zip(sample.world.live_vehicles())
    {
        let rank = *ranks
            .get(handle)
            .ok_or_else(|| failure("capture rank", id))?;
        vehicle_ids[rank] = id;
        let vehicle = by_id.get(&id).ok_or_else(|| failure("capture ID", id))?;
        let route = &mut route_ids[rank / 25];
        require(
            route.is_none_or(|old| old == vehicle.snapshot_route_id()),
            || "捕获路线映射不一致".into(),
        )?;
        *route = Some(vehicle.snapshot_route_id());
    }
    let route_ids = route_ids
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| failure("route mapping", "incomplete"))?;
    require(
        route_ids.iter().copied().collect::<HashSet<_>>().len() == snapshot.routes().len(),
        || "捕获路线表不完整".into(),
    )?;
    for (rank, &id) in route_ids.iter().enumerate() {
        let route = snapshot
            .routes()
            .iter()
            .find(|r| r.snapshot_route_id() == id)
            .ok_or_else(|| failure("route ID", id))?;
        let edges = sample.state.routes[rank]
            .edge_ids
            .iter()
            .map(|s| s.parse::<LaneEdgeId>().map(|id| id.into_untyped()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| failure("edge IDs", e))?;
        require(route.edges() == edges, || "捕获路线边序列不符".into())?;
    }
    let origin = snapshot.origin();
    let burst = match driver {
        Driver::Simple(_) => None,
        Driver::Burst(b) => Some(BurstHost {
            live_ranks: b.live_ranks.clone(),
            incarnations: b.incarnations.clone(),
            terminal: b.terminal.clone(),
            arrivals: b.arrivals.clone(),
            completions: b.completions.clone(),
            arrival_records: b.arrival_records.clone(),
            pending: b.pending.clone(),
            parked_boundary: b.parked_boundary,
            left: b.left,
            counts: b.counts,
            total_crossings: b.total_crossings.clone(),
        }),
    };
    Ok((
        Host {
            tick: snapshot.tick(),
            after_commands,
            command_count: records.journal.count,
            blocked: records.journal.blocked,
            batch_count: records.batches.len(),
            trace_offset: TRACE_HEADER.len() as u64 + records.trace_count * 40,
            event_offset: records.events.byte_length(),
            vehicle_ids,
            route_ids,
            projection: projection(sample)?,
            source_digest: *origin.canonical_artifact_digest().as_bytes(),
            source_length: origin.canonical_artifact_byte_length().get(),
            revision: *origin.network_revision().as_digest().as_bytes(),
            crossed: sample.crossed.clone(),
            transitions: sample.transitions,
            max_deceleration: sample.max_deceleration_mm_s2,
            excessive_decelerations: sample.deceleration_above_profile_count,
            burst,
        },
        encode_lfrs(&snapshot),
    ))
}

fn options() -> SharedNetworkBuildOptions<'static> {
    SharedNetworkBuildOptions::new(
        SpatialBuildOption::RetainAvailable,
        SharedNetworkBuildLimits::new(2 * 1_024 * 1_024 * 1_024, 2 * 1_024 * 1_024 * 1_024),
    )
}

pub(super) fn authenticate(
    revision: &SharedNetworkRevision,
    reference: &PublishedLfcaReference,
    bytes: &[u8],
) -> Result<(), Error> {
    match attest_published_shared_network_revision(
        revision,
        reference,
        bytes,
        FormatLimits::HARD,
        options(),
        None,
    ) {
        AttestationOutcome::Concluded(report) if report.verdict == AttestationVerdict::Pass => {
            Ok(())
        }
        other => Err(failure("共享路网修订认证非 Pass", other)),
    }
}

pub(super) fn restore(
    bundle: &Bundle,
    plan: &Plan,
    host: &Host,
    bytes: &[u8],
    execution: ExecutionConfig,
    times: &mut Value,
) -> Result<Driver, Error> {
    let start = Instant::now();
    let input = check_canonical_network_input(
        bundle.files()["network.lfca"].as_slice(),
        FormatLimits::HARD,
    )
    .map_err(|e| failure("LFCA", e))?;
    require(
        host.source_digest == *input.canonical_artifact_digest().as_bytes()
            && host.source_length == input.canonical_artifact_byte_length().get(),
        || "F3 检查点必须绑定同一正式输入的 exact LFCA".into(),
    )?;
    let revision =
        build_shared_network_revision(input, options()).map_err(|e| failure("revision", e))?;
    times["revision_build_seconds"] = json!(start.elapsed().as_secs_f64());
    let reference = PublishedLfcaReference::new(
        "fixture://lf-synth-harness",
        Sha256Digest::from_bytes(host.source_digest),
        ExactByteLength::new(host.source_length),
        NetworkRevisionId::from_digest(Sha256Digest::from_bytes(host.revision)),
    )
    .map_err(|e| failure("reference", e))?;
    let start = Instant::now();
    authenticate(&revision, &reference, &bundle.files()["network.lfca"])?;
    times["attestation_seconds"] = json!(start.elapsed().as_secs_f64());
    let state: InitialState = serde_json::from_slice(&bundle.files()["state.json"])?;
    let start = Instant::now();
    let restored = restore_lfrs(
        bytes,
        Arc::clone(&revision),
        CommittedNetworkSource::Published { reference },
        WorldConfig::new(
            state.identity.scale,
            state.routes.len() as u32,
            state.routes.len() as u64 * 2,
            0,
            u64::from(state.identity.fixed_delta_ms),
        ),
        execution,
        SnapshotRestoreLimits::new(512 * 1_024 * 1_024, 4 * 1_024),
    )
    .map_err(|e| failure("restore_lfrs", e))?;
    require(
        restored.vehicle_mappings().len() == host.vehicle_ids.len()
            && restored.route_mappings().len() == host.route_ids.len()
            && host.vehicle_ids.len() == state.vehicles.len()
            && host.route_ids.len() == state.routes.len()
            && host
                .vehicle_ids
                .iter()
                .copied()
                .collect::<HashSet<_>>()
                .len()
                == host.vehicle_ids.len()
            && host.route_ids.iter().copied().collect::<HashSet<_>>().len() == host.route_ids.len(),
        || "恢复映射数量或唯一性不符".into(),
    )?;
    let handles = host
        .vehicle_ids
        .iter()
        .map(|&id| {
            restored
                .vehicle_handle(id)
                .ok_or_else(|| failure("vehicle ID", id))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let routes = host
        .route_ids
        .iter()
        .map(|&id| {
            restored
                .route_handle(id)
                .ok_or_else(|| failure("route ID", id))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let world = restored.into_world();
    require(
        world.execution_config() == execution
            && world.latest_transition_events().is_empty()
            && world.observation_state_sequence() == ObservationStateSequence::INITIAL,
        || "恢复执行配置或新观测流不符".into(),
    )?;
    let session = world
        .open_observation_export(ObservationSelection::AllLaneEdges)
        .map_err(|e| failure("new observation session", e))?;
    require(session.previous().is_none(), || {
        "恢复后的观测 session 有旧基线".into()
    })?;
    let mut groups = Vec::new();
    let mut gates = Vec::new();
    for route in &state.routes {
        let group = revision
            .identity()
            .ordinal(
                route
                    .signal_group_id
                    .parse::<SignalGroupId>()
                    .map_err(|e| failure("group", e))?,
            )
            .ok_or_else(|| failure("group", "missing"))?;
        let view = revision
            .traffic()
            .relations()
            .signal_group(group)
            .ok_or_else(|| failure("group view", "missing"))?;
        require(view.gates().len() == 1, || "Gate 数量不符".into())?;
        groups.push(group);
        gates.push(view.gates()[0]);
    }
    let initial = handles
        .iter()
        .map(|&h| {
            world
                .vehicle(h)
                .ok_or_else(|| failure("restored handle", h))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let bindings = handles.iter().map(|&h| world.parking_binding(h)).collect();
    let sample = Sample {
        cursor: world.command_cursor(),
        world,
        state,
        handles,
        routes,
        groups,
        gates,
        previous: initial.clone(),
        initial,
        bindings,
        crossed: host.crossed.clone(),
        transitions: host.transitions,
        max_deceleration_mm_s2: host.max_deceleration,
        deceleration_above_profile_count: host.excessive_decelerations,
        trace_bytes: Vec::new(),
    };
    require(
        projection(&sample)? == host.projection
            && host.tick == sample.world.tick_index()
            && host.crossed.len() == sample.handles.len(),
        || "恢复后状态、时钟、双游标或身份归属不符".into(),
    )?;
    let driver = if let Some(h) = &host.burst {
        let size = sample.handles.len();
        require(
            h.live_ranks.len() == size
                && h.live_ranks.iter().copied().collect::<HashSet<_>>() == (0..size).collect()
                && h.incarnations.len() == size
                && h.terminal.len() == size
                && h.total_crossings.len() == size
                && h.arrivals.len() == size / 100,
            || "宿主 Burst 记录形状不符".into(),
        )?;
        let spaces = sample
            .state
            .vehicles
            .iter()
            .map(|v| {
                revision
                    .identity()
                    .ordinal(
                        v.parking_space_id
                            .parse::<ParkingSpaceId>()
                            .map_err(|e| failure("space", e))?,
                    )
                    .ok_or_else(|| failure("space", "missing"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut burst = Burst {
            spaces,
            live_ranks: h.live_ranks.clone(),
            live_sequences: vec![0; size],
            handle_ranks: sample
                .handles
                .iter()
                .copied()
                .enumerate()
                .map(|(r, h)| (h, r))
                .collect(),
            incarnations: h.incarnations.clone(),
            expected_status: sample.initial.iter().map(|s| s.status()).collect(),
            terminal: h.terminal.clone(),
            lane_order: Vec::new(),
            arrivals: h.arrivals.clone(),
            completions: h.completions.clone(),
            arrival_records: h.arrival_records.clone(),
            replacements: plan
                .rules
                .iter()
                .find(|r| r["kind"] == "completed_replacement")
                .map(|r| serde_json::from_value(r.clone()))
                .transpose()?,
            pending: h.pending.clone(),
            parked_boundary: h.parked_boundary,
            left: h.left,
            counts: h.counts,
            changed: vec![false; size],
            total_crossings: h.total_crossings.clone(),
            sample,
        };
        burst.refresh_order();
        Driver::Burst(Box::new(burst))
    } else {
        Driver::Simple(Box::new(sample))
    };
    require(
        matches!(driver, Driver::Burst(_)) == matches!(plan.identity.case, Case::W3 | Case::W4),
        || "宿主场景种类不符".into(),
    )?;
    times["restore_and_mapping_seconds"] = json!(start.elapsed().as_secs_f64());
    Ok(driver)
}
