//! 有界证据程序：正式 LFCA/catalog → prepare → 一个 Session → 固定步进与全量消费。

use std::{
    collections::BTreeMap,
    error::Error,
    hint::black_box,
    num::NonZeroU32,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use bevy_app::{App, Update};
use bevy_ecs::{
    entity::Entity,
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{Res, ResMut},
    world::World,
};
use bevy_math::Vec3;
use bevy_time::{TimePlugin, TimeUpdateStrategy};
use bevy_transform::components::Transform;
use laneflow_bevy::{
    LaneFlowCommittedPoseBatch, LaneFlowFixed, LaneFlowFixedSet, LaneFlowPlugin, LaneFlowSession,
    LaneFlowSessionConfig,
};
use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_junction_generator::GridCatalog;
use laneflow_runtime::{
    CommittedNetworkSource, ConflictDecisionOutcome, PublishedLfcaReference, RouteRegisterInput,
    TrafficTransitionKind, TrafficWorld, VehicleHandle, VehicleSpawnInput, VehicleStatus,
    WaitingDecisionOutcome, WorldConfig, deterministic_state_digest, encode_lfrs,
};
use laneflow_scenario::complex_junction::{VEHICLE_PROFILE_KEY, bind};
use laneflow_spatial::{
    CanonicalPoseBatch, FramePlacementToken, PoseInput, PoseRecordId, SpatialSession,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[cfg(feature = "native-example")]
#[path = "junction_scale_native.rs"]
mod native;

#[path = "junction_scale_validation.rs"]
mod validation;
use validation::Validation;

struct RunLimits {
    ticks: u64,
    wall_ms: u64,
    measurement_ms: u64,
}

impl RunLimits {
    fn new(warmup: u64, observation: usize, wall_ms: u64) -> Result<Self, Box<dyn Error>> {
        let ticks = warmup
            .checked_add(u64::try_from(observation)?)
            .ok_or("tick limit overflow")?;
        if ticks == 0 || ticks > 4_096 || observation == 0 || !(1..=600_000).contains(&wall_ms) {
            return Err("total ticks must be 1..=4096 and wall limit must be 1..=600000 ms".into());
        }
        // 墙钟包含初始化和零步渲染准备；给截图、摘要与结果写出留出时间。
        let reserve_ms = 10_000.min(wall_ms / 10);
        Ok(Self {
            ticks,
            wall_ms,
            measurement_ms: wall_ms - reserve_ms,
        })
    }

    fn stop_reason(&self, ticks: u64, elapsed: Duration) -> Option<&'static str> {
        if elapsed >= Duration::from_millis(self.measurement_ms) {
            Some("time-limit")
        } else if ticks >= self.ticks {
            Some("tick-limit")
        } else {
            None
        }
    }
}

#[derive(Resource)]
struct Measurements {
    validation_failure_path: PathBuf,
    warmup: u64,
    allocation: bool,
    started: Instant,
    allocation_start: stats_alloc::Stats,
    tick_ns: Vec<u64>,
    observation_ns: Vec<u64>,
    tick_allocations: usize,
    tick_reallocations: usize,
    max_waiting_members: usize,
    max_reservations: usize,
    waiting_decisions: u64,
    conflict_decisions: u64,
    max_waiting_candidates: usize,
    max_conflict_candidates: usize,
    transitions: u64,
    event_digest: Sha256,
    observed_ticks: usize,
    min_active: usize,
    evidence_ns: Vec<u64>,
    waiting_hold_starts: Vec<Option<u64>>,
    waiting_entries: Vec<u32>,
    reservation_acquisitions: Vec<u32>,
    waiting_requests: Vec<u32>,
    conflict_requests: Vec<u32>,
    waiting_zone_requests: BTreeMap<u32, u64>,
    waiting_vehicle_ticks: u64,
    reservation_vehicle_ticks: u64,
    longest_waiting_hold: u64,
    longest_reservation_age: u64,
}

#[derive(Resource)]
struct Presentation {
    record_frame: bool,
    apply_count: usize,
    identities: Vec<VehicleHandle>,
    entities: Vec<Entity>,
    poses: LaneFlowCommittedPoseBatch,
    reference_spatial: SpatialSession,
    reference_inputs: Vec<PoseInput>,
    reference_poses: CanonicalPoseBatch,
    checked_pose_rows: u64,
    checked_transform_rows: u64,
    evidence_ns: Vec<u64>,
    pose_ns: Vec<u64>,
    apply_ns: Vec<u64>,
    last_error: Option<String>,
}

fn check_presentation(
    world: &World,
    session: &LaneFlowSession,
    p: &mut Presentation,
) -> Result<(), String> {
    p.reference_inputs.clear();
    for (index, &vehicle) in p.identities.iter().enumerate() {
        let state = session
            .world()
            .vehicle(vehicle)
            .ok_or("presentation identity absent")?;
        let route = session
            .world()
            .route_edges(state.route())
            .ok_or("presentation route absent")?;
        p.reference_inputs.push(PoseInput::lane(
            PoseRecordId::new(index as u32),
            route[state.route_edge_index() as usize],
            state.progress_mm(),
        ));
    }
    // Compare the adapter's mapping with direct committed-state inputs to the
    // existing exact Spatial baseline; its geometric oracle remains the finite matrix.
    p.reference_spatial
        .extract_pose_batch(
            FramePlacementToken::new(1),
            &p.reference_inputs,
            &mut p.reference_poses,
        )
        .map_err(|error| format!("reference pose failed: {error:?}"))?;
    if p.poses.vehicles() != p.identities || p.poses.batch() != &p.reference_poses {
        return Err("committed state to pose mismatch".into());
    }
    for ((&vehicle, &entity), record) in p
        .identities
        .iter()
        .zip(&p.entities)
        .zip(p.reference_poses.records())
    {
        if session.vehicle_entity(vehicle) != Some(entity) {
            return Err("stable vehicle to entity mapping changed".into());
        }
        let pose = record.pose();
        let position = pose.position();
        let forward = pose.tangent();
        let up = pose.up();
        let expected = Transform::from_xyz(position.x(), position.y(), position.z()).looking_to(
            Vec3::new(forward.x(), forward.y(), forward.z()),
            Vec3::new(up.x(), up.y(), up.z()),
        );
        if world.get::<Transform>(entity) != Some(&expected) {
            return Err("pose to Transform mismatch".into());
        }
    }
    if p.record_frame {
        p.checked_pose_rows += p.identities.len() as u64;
        p.checked_transform_rows += p.entities.len() as u64;
    }
    Ok(())
}

fn present(world: &mut World) {
    world.resource_scope(
        |world, mut presentation: bevy_ecs::change_detection::Mut<Presentation>| {
            let result = world.resource_scope(
                |world,
                 mut session: bevy_ecs::change_detection::Mut<LaneFlowSession>|
                 -> Result<(), String> {
                    let started = Instant::now();
                    session
                        .extract_committed_pose_batch(
                            FramePlacementToken::new(1),
                            &mut presentation.poses,
                        )
                        .map_err(|error| format!("{error:?}"))?;
                    let pose_elapsed = started.elapsed().as_nanos() as u64;
                    if presentation.poses.vehicles().len() != presentation.identities.len() {
                        return Err("presentable population changed".into());
                    }
                    if !session.consumption_context_is_current(presentation.poses.context()) {
                        return Err("stale pose context".into());
                    }
                    let started = Instant::now();
                    for ((&vehicle, record), &expected) in presentation
                        .poses
                        .vehicles()
                        .iter()
                        .zip(presentation.poses.batch().records())
                        .take(presentation.apply_count)
                        .zip(&presentation.identities)
                    {
                        if vehicle != expected {
                            return Err("stable caller identity order changed".into());
                        }
                        let entity = session.vehicle_entity(vehicle).ok_or("mapping absent")?;
                        let pose = record.pose();
                        let position = pose.position();
                        let forward = pose.tangent();
                        let up = pose.up();
                        *world
                            .get_mut::<Transform>(entity)
                            .ok_or("Transform absent")? =
                            Transform::from_xyz(position.x(), position.y(), position.z())
                                .looking_to(
                                    Vec3::new(forward.x(), forward.y(), forward.z()),
                                    Vec3::new(up.x(), up.y(), up.z()),
                                );
                    }
                    let apply_elapsed = started.elapsed().as_nanos() as u64;
                    let started = Instant::now();
                    check_presentation(world, &session, &mut presentation)?;
                    let evidence_elapsed = started.elapsed().as_nanos() as u64;
                    if presentation.record_frame {
                        presentation.pose_ns.push(pose_elapsed);
                        presentation.apply_ns.push(apply_elapsed);
                        presentation.evidence_ns.push(evidence_elapsed);
                    }
                    Ok(())
                },
            );
            if let Err(error) = result {
                presentation.last_error = Some(error);
            }
        },
    );
}

fn begin_tick(mut samples: ResMut<Measurements>) {
    if samples.allocation {
        samples.allocation_start = stats_alloc::INSTRUMENTED_SYSTEM.stats();
    }
    samples.started = Instant::now();
}

fn observe(
    session: Res<LaneFlowSession>,
    mut samples: ResMut<Measurements>,
    mut validation: ResMut<Validation>,
) {
    let tick_ns = samples.started.elapsed().as_nanos() as u64;
    let stats = samples
        .allocation
        .then(|| stats_alloc::INSTRUMENTED_SYSTEM.stats());
    let tick = session.world().tick_index();
    if tick <= samples.warmup {
        validate_tick(&mut validation, &session, &samples.validation_failure_path);
        return;
    }
    if let Some(stats) = stats {
        samples.tick_allocations += stats.allocations - samples.allocation_start.allocations;
        samples.tick_reallocations += stats.reallocations - samples.allocation_start.reallocations;
    }
    let started = Instant::now();
    let view = session.junction_observation();
    let mut reservations = 0;
    let mut vehicles = 0;
    let mut active = 0;
    for row in view.vehicles() {
        reservations += usize::from(row.conflict_reservation().is_some());
        active += usize::from(row.state().status() == VehicleStatus::Active);
        black_box(row);
        vehicles += 1;
    }
    for zone in view.waiting_zones() {
        black_box(zone);
    }
    for member in view.waiting_zone_members() {
        black_box(member);
    }
    for decision in view.latest_waiting_decisions() {
        black_box(decision);
    }
    for decision in view.latest_conflict_decisions() {
        black_box(decision);
    }
    for event in view.latest_transition_events() {
        black_box(event);
    }
    let observation_ns = started.elapsed().as_nanos() as u64;
    let evidence_started = Instant::now();
    validate_tick(&mut validation, &session, &samples.validation_failure_path);
    for decision in view.latest_waiting_decisions() {
        if matches!(
            decision.outcome(),
            WaitingDecisionOutcome::Granted | WaitingDecisionOutcome::NoGrant(_)
        ) {
            samples.waiting_requests[decision.vehicle_update_sequence() as usize] += 1;
            if let Some(zone) = decision.zone() {
                *samples.waiting_zone_requests.entry(zone.raw()).or_default() += 1;
            }
        }
    }
    for decision in view.latest_conflict_decisions() {
        if matches!(
            decision.outcome(),
            ConflictDecisionOutcome::Granted | ConflictDecisionOutcome::NoGrant(_)
        ) {
            samples.conflict_requests[decision.vehicle_update_sequence() as usize] += 1;
        }
    }
    // 资源负载证据单独计时。Waiting 持有长度在观察边界截断，为实际持续时间下界。
    for (index, row) in view.vehicles().enumerate() {
        if row.state().waiting_membership().is_some() {
            samples.waiting_vehicle_ticks += 1;
            let start = *samples.waiting_hold_starts[index].get_or_insert(tick);
            samples.longest_waiting_hold = samples.longest_waiting_hold.max(tick - start + 1);
        } else {
            samples.waiting_hold_starts[index] = None;
        }
        if let Some(reservation) = row.conflict_reservation() {
            samples.reservation_vehicle_ticks += 1;
            samples.longest_reservation_age = samples
                .longest_reservation_age
                .max(tick - reservation.acquired_tick() + 1);
        }
    }
    for event in view.latest_transition_events() {
        let index = event.vehicle_update_sequence() as usize;
        match event.kind() {
            TrafficTransitionKind::WaitingEntered { .. } => samples.waiting_entries[index] += 1,
            TrafficTransitionKind::ReservationAcquired { .. } => {
                samples.reservation_acquisitions[index] += 1
            }
            _ => {}
        }
    }
    black_box(vehicles);
    samples.tick_ns.push(tick_ns);
    samples.observation_ns.push(observation_ns);
    samples.observed_ticks += 1;
    samples.min_active = samples.min_active.min(active);
    samples.max_reservations = samples.max_reservations.max(reservations);
    samples.max_waiting_members = samples
        .max_waiting_members
        .max(view.waiting_zone_members().len());
    samples.max_waiting_candidates = samples
        .max_waiting_candidates
        .max(view.latest_waiting_decisions().len());
    samples.max_conflict_candidates = samples
        .max_conflict_candidates
        .max(view.latest_conflict_decisions().len());
    samples.waiting_decisions += view.latest_waiting_decisions().len() as u64;
    samples.conflict_decisions += view.latest_conflict_decisions().len() as u64;
    samples.transitions += view.latest_transition_events().len() as u64;
    // 摘要编码在计时区间外；同一提交/工具链内复核逐 tick 输出，非线格式。
    samples.event_digest.update(tick.to_le_bytes());
    for batch in [
        format!("{:?}", view.latest_waiting_decisions()),
        format!("{:?}", view.latest_conflict_decisions()),
        format!("{:?}", view.latest_transition_events()),
    ] {
        samples
            .event_digest
            .update((batch.len() as u64).to_le_bytes());
        samples.event_digest.update(batch.as_bytes());
    }
    samples
        .evidence_ns
        .push(evidence_started.elapsed().as_nanos() as u64);
}

fn validate_tick(
    validation: &mut Validation,
    session: &LaneFlowSession,
    failure_path: &std::path::Path,
) {
    let world = session.world();
    if let Err((kind, detail)) = validation.check_session(session) {
        let report = json!({"pid":std::process::id(),"tick":world.tick_index(),"validation":validation.report()});
        std::fs::write(
            failure_path,
            serde_json::to_vec_pretty(&report).expect("validation report is JSON"),
        )
        .expect("preserve fidelity failure report");
        match world.capture_snapshot() {
            Ok(snapshot) => {
                std::fs::write(failure_path.with_extension("lfrs"), encode_lfrs(&snapshot))
                    .expect("preserve failed candidate state")
            }
            Err(error) => eprintln!("failed candidate snapshot could not be captured: {error:?}"),
        }
        panic!(
            "scale fidelity violation at tick {}: {kind}: {detail}",
            world.tick_index()
        );
    }
}

fn percentiles(samples: &[u64]) -> Value {
    if samples.is_empty() {
        return Value::Null;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let at = |percent: usize| sorted[(sorted.len() * percent).div_ceil(100) - 1];
    json!({"count":sorted.len(), "p50":at(50), "p95":at(95), "p99":at(99), "max":sorted[sorted.len()-1]})
}

fn state_digest(session: &LaneFlowSession) -> String {
    format!(
        "{:x}",
        deterministic_state_digest(&session.world().capture_snapshot().unwrap()).unwrap()
    )
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn run(allocation: bool, rendering: bool) -> Result<(), Box<dyn Error>> {
    let execution_started = Instant::now();
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(7..=8).contains(&args.len()) {
        return Err("usage: junction_scale <lfca> <catalog> <vehicles> <warmup-ticks> <observation-ticks> <normal|catchup|prepare> <output.json> [wall-limit-ms] (total ticks <= 4096, wall limit <= 600000 ms)".into());
    }
    let vehicle_count: usize = args[2].parse()?;
    let warmup: u64 = args[3].parse()?;
    let observation: usize = args[4].parse()?;
    let wall_ms = args
        .get(7)
        .map(|arg| arg.parse())
        .transpose()?
        .unwrap_or(600_000);
    let limits = RunLimits::new(warmup, observation, wall_ms)?;
    if vehicle_count == 0 || vehicle_count > 100_000 || observation == 0 {
        return Err("vehicles must be 1..=100000 and observation must be positive".into());
    }
    let chunks: &[u64] = match args[5].as_str() {
        "normal" | "prepare" => &[1],
        "catchup" => &[0, 1, 2, 4, 0],
        _ => return Err("frame mode must be normal, catchup or prepare".into()),
    };
    let output = PathBuf::from(&args[6]);
    if output.exists() {
        return Err("output already exists; use a new execution file".into());
    }
    let lfca = std::fs::read(&args[0])?;
    let catalog_text = std::fs::read_to_string(&args[1])?;
    let grid: GridCatalog = toml::from_str(&catalog_text)?;
    if grid.layout != "junction-grid-v1"
        || grid.cells.is_empty()
        || grid.cells.len() > 1_000
        || grid.columns == 0
        || grid.columns > grid.cells.len()
        || !grid.pitch_meters.is_finite()
        || grid.pitch_meters <= 0.0
        || vehicle_count < grid.cells.len()
    {
        return Err("grid layout/count does not match the frozen population".into());
    }
    let input = check_canonical_network_input(lfca.as_slice(), FormatLimits::HARD)
        .map_err(|error| format!("{error:?}"))?;
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(256 * 1_024 * 1_024, 64 * 1_024 * 1_024),
        ),
    )
    .map_err(|error| format!("{error:?}"))?;
    let first_bound = bind(&grid.cells[0], &revision)?;
    let origin = revision.canonical_origin();
    let mut world = TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(
            vehicle_count as u32,
            (grid.cells.len() * 11) as u32,
            (grid.cells.len() * 16_384) as u64,
            (grid.cells.len() * 65_536) as u64,
            1,
            16,
        ),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "scenario://complex-junction-scale",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )?,
        },
        0,
        first_bound.policy_selection,
    )?;
    let profile = first_bound.profiles[VEHICLE_PROFILE_KEY];
    let vehicles_per_cell = vehicle_count / grid.cells.len();
    let mut command_digest = Sha256::new();
    let mut identities = Vec::with_capacity(vehicle_count);
    for (cell_index, catalog) in grid.cells.iter().enumerate() {
        let cell_vehicles =
            vehicles_per_cell + usize::from(cell_index < vehicle_count % grid.cells.len());
        let bound = bind(catalog, &revision)?;
        // 64 个有限 catalog 路段依次接续，保留全部真实边、门和策略。
        let mut routes = Vec::new();
        for (initial_index, initial) in bound.route_exits.iter().enumerate() {
            let mut edges = initial.edges.to_vec();
            for leg in 1..64 {
                let end = *edges.last().expect("bound route is non-empty");
                let choices: Vec<_> = bound
                    .route_exits
                    .iter()
                    .filter(|route| route.edges.first() == Some(&end))
                    .collect();
                if choices.is_empty() {
                    return Err("no catalog continuation at loop edge".into());
                }
                let next = choices[(initial_index + leg) % choices.len()];
                edges.extend_from_slice(&next.edges[1..]);
            }
            command_digest.update((cell_index as u64).to_le_bytes());
            command_digest.update((initial_index as u64).to_le_bytes());
            for edge in &edges {
                command_digest.update(edge.raw().to_le_bytes());
            }
            routes.push(world.register_route(RouteRegisterInput::new(edges))?);
        }
        let lane_count = bound.portal_lanes.len();
        let mut lanes = vec![Vec::new(); lane_count];
        for slot in &bound.spawn_slots {
            lanes[slot.portal_lane_index].push(slot);
        }
        for lane in &mut lanes {
            lane.sort_by_key(|slot| std::cmp::Reverse(slot.progress_mm));
        }
        if lanes
            .iter()
            .any(|lane| lane.len() < cell_vehicles.div_ceil(lane_count))
        {
            return Err(
                "insufficient slots in a portal lane for the frozen round-robin plan".into(),
            );
        }
        for local_index in 0..cell_vehicles {
            let index = identities.len();
            let lane = local_index % lane_count;
            let depth = local_index / lane_count;
            let slot = lanes[lane][depth];
            let choices = &bound.portal_lanes[lane].choices;
            let route_index = choices[depth % choices.len()].route_index;
            command_digest.update((index as u64).to_le_bytes());
            command_digest.update((route_index as u64).to_le_bytes());
            command_digest.update((slot.slot_id.len() as u64).to_le_bytes());
            command_digest.update(slot.slot_id.as_bytes());
            command_digest.update(slot.progress_mm.to_le_bytes());
            identities.push(world.spawn_vehicle(VehicleSpawnInput::new(
                profile,
                routes[route_index],
                0,
                slot.progress_mm,
                0,
            ))?);
        }
    }
    let spatial = SpatialSession::bind(revision)
        .map_err(|error| format!("{error:?}"))?
        .ok_or("spatial session missing")?;
    let session = LaneFlowSession::new(
        world,
        Some(spatial),
        LaneFlowSessionConfig::new(NonZeroU32::new(2).unwrap()),
    )?;
    let initial_digest = state_digest(&session);
    let frozen_input = json!({"lfca_sha256":hex(Sha256::digest(&lfca)),"catalog_sha256":hex(Sha256::digest(catalog_text.as_bytes())),
        "command_plan_sha256":hex(command_digest.finalize()),"seed":0,"vehicles":vehicle_count,"profile":VEHICLE_PROFILE_KEY,
        "warmup_ticks":warmup,"observation_ticks":observation,"fixed_delta_ms":16,"cells":grid.cells.len(),
        "vehicles_per_cell_min":vehicles_per_cell,"vehicles_per_cell_max":vehicle_count.div_ceil(grid.cells.len()),"route_legs":64});
    if args[5] == "prepare" {
        let snapshot = encode_lfrs(&session.world().capture_snapshot()?);
        std::fs::write(output.with_extension("initial.lfrs"), &snapshot)?;
        std::fs::write(
            &output,
            serde_json::to_vec_pretty(&json!({"schema":"junction-scale-prepared-v1",
            "input":frozen_input,"initial_state_digest":initial_digest,"snapshot_sha256":hex(Sha256::digest(&snapshot))}))?,
        )?;
        return Ok(());
    }
    let mut app = App::new();
    let validation = Validation::new(session.world())
        .map_err(|(kind, detail)| format!("initial validation: {kind}: {detail}"))?;
    let reference_spatial = SpatialSession::bind(session.world().revision())
        .map_err(|error| format!("{error:?}"))?
        .ok_or("reference spatial missing")?;
    if rendering {
        #[cfg(feature = "native-example")]
        native::plugins(&mut app);
        #[cfg(not(feature = "native-example"))]
        return Err("renderer requires native-example".into());
    } else {
        app.add_plugins(TimePlugin);
    }
    app.add_plugins(LaneFlowPlugin)
        .insert_resource(validation)
        .insert_resource(session)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO))
        .insert_resource(Measurements {
            validation_failure_path: output.with_extension("validation-failure.json"),
            warmup,
            allocation,
            started: Instant::now(),
            allocation_start: stats_alloc::INSTRUMENTED_SYSTEM.stats(),
            tick_ns: Vec::with_capacity(observation),
            observation_ns: Vec::with_capacity(observation),
            tick_allocations: 0,
            tick_reallocations: 0,
            max_waiting_members: 0,
            max_reservations: 0,
            waiting_decisions: 0,
            conflict_decisions: 0,
            max_waiting_candidates: 0,
            max_conflict_candidates: 0,
            transitions: 0,
            event_digest: Sha256::new(),
            observed_ticks: 0,
            min_active: vehicle_count,
            evidence_ns: Vec::with_capacity(observation),
            waiting_hold_starts: vec![None; vehicle_count],
            waiting_entries: vec![0; vehicle_count],
            reservation_acquisitions: vec![0; vehicle_count],
            waiting_requests: vec![0; vehicle_count],
            conflict_requests: vec![0; vehicle_count],
            waiting_zone_requests: BTreeMap::new(),
            waiting_vehicle_ticks: 0,
            reservation_vehicle_ticks: 0,
            longest_waiting_hold: 0,
            longest_reservation_age: 0,
        })
        .add_systems(
            LaneFlowFixed,
            begin_tick
                .after(LaneFlowFixedSet::Lifecycle)
                .before(LaneFlowFixedSet::Step),
        )
        .add_systems(LaneFlowFixed, observe.in_set(LaneFlowFixedSet::Observe))
        .add_systems(Update, present);
    let apply_count = if vehicle_count == 100_000 {
        vehicle_count / 10
    } else {
        vehicle_count
    };
    let entities: Vec<Entity> = identities
        .iter()
        .take(apply_count)
        .map(|_| app.world_mut().spawn(Transform::IDENTITY).id())
        .collect();
    for (&vehicle, &entity) in identities.iter().zip(&entities) {
        app.world_mut()
            .resource_mut::<LaneFlowSession>()
            .bind_vehicle_entity(vehicle, entity)?;
    }
    #[allow(unused_mut)]
    let mut renderer_info = Value::Null;
    #[cfg(feature = "native-example")]
    if rendering {
        renderer_info = native::setup(&mut app, &entities, &grid);
    }
    app.insert_resource(Presentation {
        record_frame: false,
        apply_count,
        identities,
        entities: entities.clone(),
        poses: LaneFlowCommittedPoseBatch::new(),
        reference_spatial,
        reference_inputs: Vec::with_capacity(vehicle_count),
        reference_poses: CanonicalPoseBatch::new(),
        checked_pose_rows: 0,
        checked_transform_rows: 0,
        evidence_ns: Vec::with_capacity(observation + 1),
        pose_ns: Vec::with_capacity(observation + 1),
        apply_ns: Vec::with_capacity(observation + 1),
        last_error: None,
    });
    app.finish();
    app.cleanup();
    #[cfg(feature = "native-example")]
    if rendering {
        renderer_info["adapter"] = native::adapter_info(&app);
    }
    for _ in 0..if rendering { 150 } else { 1 } {
        if limits.stop_reason(0, execution_started.elapsed()).is_some() {
            break;
        }
        app.sub_apps_mut().main.run_default_schedule();
        #[cfg(feature = "native-example")]
        if rendering {
            native::render(&mut app)?;
        }
        app.world_mut().clear_trackers();
    }
    let mut frame_ns = Vec::with_capacity(observation + 1);
    let mut frame_steps = Vec::with_capacity(observation + 1);
    let mut frame_input_quanta = Vec::with_capacity(observation + 1);
    let mut frame_backlog_quanta = Vec::with_capacity(observation + 1);
    let mut backlog_since = None;
    let mut max_backlog_recovery_frames = 0;
    #[allow(unused_mut)]
    let mut renderer_ns: Vec<u64> = Vec::with_capacity(observation + 1);
    let mut checkpoints = Vec::new();
    let mut frame = 0;
    #[allow(unused_mut)]
    let mut minimum_visible = entities.len();
    let total_ticks = limits.ticks;
    eprintln!(
        "prepared {vehicle_count} vehicles in one world; warmup={warmup} observation={observation}"
    );
    let stop_reason;
    let stopped_elapsed;
    loop {
        let before = app
            .world()
            .resource::<LaneFlowSession>()
            .world()
            .tick_index();
        let elapsed = execution_started.elapsed();
        if let Some(reason) = limits.stop_reason(before, elapsed) {
            stop_reason = reason;
            stopped_elapsed = elapsed;
            break;
        }
        let next_boundary = if before < warmup {
            warmup
        } else {
            [1_024, 2_048, 4_096]
                .into_iter()
                .map(|h| warmup + h)
                .find(|&h| h > before)
                .unwrap_or(total_ticks)
                .min(total_ticks)
        };
        let backlog = app
            .world()
            .resource::<LaneFlowSession>()
            .frame_report()
            .backlog()
            .as_millis() as u64
            / 16;
        let available_input = (next_boundary - before)
            .checked_sub(backlog)
            .ok_or("backlog crosses a frozen boundary")?;
        let input_quanta = chunks[frame % chunks.len()].min(available_input);
        let steps = (backlog + input_quanta).min(2);
        let started_frame = Instant::now();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            input_quanta * 16,
        )));
        app.world_mut().resource_mut::<Presentation>().record_frame = before >= warmup;
        app.sub_apps_mut().main.run_default_schedule();
        let session = app.world().resource::<LaneFlowSession>();
        if let Some(error) = session.last_error() {
            return Err(format!("{error:?}").into());
        }
        let after = session.world().tick_index();
        let report = session.frame_report();
        let remaining_backlog = backlog + input_quanta - steps;
        if after != before + steps {
            return Err("fixed-step count mismatch".into());
        }
        if u64::from(report.steps_run()) != steps
            || report.backlog() != Duration::from_millis(remaining_backlog * 16)
        {
            return Err("frame report discarded or invented backlog".into());
        }
        if let Some(error) = &app.world().resource::<Presentation>().last_error {
            return Err(error.clone().into());
        }
        #[cfg(feature = "native-example")]
        if rendering {
            let started = Instant::now();
            native::render(&mut app)?;
            if before >= warmup {
                renderer_ns.push(started.elapsed().as_nanos() as u64);
            }
        }
        app.world_mut().clear_trackers();
        if before >= warmup {
            frame_ns.push(started_frame.elapsed().as_nanos() as u64);
            frame_steps.push(steps);
            frame_input_quanta.push(input_quanta);
            frame_backlog_quanta.push(remaining_backlog);
            if remaining_backlog > 0 {
                backlog_since.get_or_insert(frame);
            } else if let Some(start) = backlog_since.take() {
                max_backlog_recovery_frames = max_backlog_recovery_frames.max(frame - start);
            }
            #[cfg(feature = "native-example")]
            if rendering {
                minimum_visible = minimum_visible.min(native::visible_count(&app, &entities));
            }
        }
        if after == warmup && steps != 0 {
            let snapshot = app
                .world()
                .resource::<LaneFlowSession>()
                .world()
                .capture_snapshot()?;
            std::fs::write(output.with_extension("warm.lfrs"), encode_lfrs(&snapshot))?;
        }
        if after > warmup && [1_024, 2_048, 4_096].contains(&(after - warmup)) && steps != 0 {
            checkpoints.push(json!({"observation_tick":after-warmup, "state_digest":state_digest(app.world().resource::<LaneFlowSession>())}));
        }
        if after.is_multiple_of(256) && steps != 0 {
            eprintln!("tick={after}/{total_ticks}");
        }
        frame += 1;
    }
    let measurement_elapsed_ms = stopped_elapsed.as_millis() as u64;
    let completed_ticks = app
        .world()
        .resource::<LaneFlowSession>()
        .world()
        .tick_index();
    if completed_ticks <= warmup {
        return Err(
            "time budget expired before a measured successful tick; no runnable evidence".into(),
        );
    }
    #[cfg(feature = "native-example")]
    if rendering {
        let preview = output.with_extension("png");
        // 时间边界可能留下 backlog；截图帧只呈现最终状态，不再推进它。
        app.configure_sets(
            laneflow_bevy::LaneFlowOuterFrame,
            laneflow_bevy::LaneFlowOuterFrameSet::Drive.run_if(|| false),
        );
        app.world_mut().resource_mut::<Presentation>().record_frame = false;
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
        native::preview(&mut app, &preview)?;
        if app
            .world()
            .resource::<LaneFlowSession>()
            .world()
            .tick_index()
            != completed_ticks
        {
            return Err("preview advanced beyond the measured window".into());
        }
        renderer_info["preview"] = json!(preview);
    }
    let session = app.world().resource::<LaneFlowSession>();
    let presentation = app.world().resource::<Presentation>();
    let poses = &presentation.poses;
    let pose_ns = &presentation.pose_ns;
    let apply_ns = &presentation.apply_ns;
    let samples = app.world().resource::<Measurements>();
    if samples.observed_ticks as u64 != completed_ticks - warmup {
        return Err("missing successful-tick observation".into());
    }
    if samples.min_active != vehicle_count {
        return Err("active population exhausted during observation".into());
    }
    if pose_ns.len() != frame_ns.len()
        || apply_ns.len() != frame_ns.len()
        || presentation.evidence_ns.len() != frame_ns.len()
    {
        return Err("presentation frame samples are incomplete".into());
    }
    let spatial_adapter_ns: Vec<_> = pose_ns
        .iter()
        .zip(apply_ns)
        .map(|(pose, apply)| pose + apply)
        .collect();
    // 同一帧墙钟扣除该帧实际 renderer 和取证子区间；保留领域观测和 ECS 驱动。
    let mut tick_cursor = 0;
    let mut laneflow_frame_ns = Vec::with_capacity(frame_ns.len());
    for (index, (&elapsed, &steps)) in frame_ns.iter().zip(&frame_steps).enumerate() {
        let next_tick = tick_cursor + steps as usize;
        let evidence: u64 = samples.evidence_ns[tick_cursor..next_tick]
            .iter()
            .sum::<u64>()
            + presentation.evidence_ns[index];
        let render = if rendering { renderer_ns[index] } else { 0 };
        laneflow_frame_ns.push(
            elapsed
                .checked_sub(evidence + render)
                .ok_or("frame subintervals exceed wall clock")?,
        );
        tick_cursor = next_tick;
    }
    if tick_cursor != samples.observed_ticks {
        return Err("frame/tick sample alignment mismatch".into());
    }
    let frame_class = |steps: u64| {
        let select = |values: &[u64]| -> Vec<u64> {
            values
                .iter()
                .zip(&frame_steps)
                .filter_map(|(&value, &count)| (count == steps).then_some(value))
                .collect()
        };
        json!({"integrated_frame_with_evidence":percentiles(&select(&frame_ns)),"laneflow_frame_without_evidence":percentiles(&select(&laneflow_frame_ns)),"spatial_adapter":percentiles(&select(&spatial_adapter_ns)),
            "renderer_submit_and_gpu_wait":if rendering { percentiles(&select(&renderer_ns)) } else { Value::Null }})
    };
    let mut missing = vec![
        "component retained/scratch ledgers are not measured by this bounded run",
        "no steady-state allocation, long-run repeatability or product certification claim",
    ];
    if !rendering {
        missing.push("renderer (headless run)");
    }
    let result = json!({
        "schema":"junction-scale-evidence-v2", "pid":std::process::id(), "allocation_instrumented":allocation,"renderer":renderer_info,
        "run":{"stop_reason":stop_reason,"requested_ticks":total_ticks,"completed_ticks":completed_ticks,
            "simulated_milliseconds":completed_ticks * 16,"wall_limit_ms":limits.wall_ms,
            "measurement_limit_ms":limits.measurement_ms,"measurement_elapsed_ms":measurement_elapsed_ms,
            "sampling":"bounded cold-start run; timing limits include initialization and renderer preparation"},
        "input":frozen_input,"frame_mode":args[5],
        "validation":app.world().resource::<Validation>().report(),
        "presentation_validation":{"checked_pose_rows":presentation.checked_pose_rows,
            "checked_transform_rows":presentation.checked_transform_rows,"violations":0,
            "oracle_scope":"direct committed-state inputs to existing exact Spatial baseline; finite geometry oracle matrix"},
        "initial_state_digest":initial_digest,"final_state_digest":state_digest(session),"checkpoints":checkpoints,
        "domain_event_digest":hex(samples.event_digest.clone().finalize()),
        "counts":{"worlds":1,"presentable":poses.vehicles().len(),"domain_vehicle_rows_per_tick":vehicle_count,
            "pose_rows_per_frame":poses.vehicles().len(),"transform_rows_per_frame":apply_count,
            "observed_ticks":samples.observed_ticks,"frames":frame_ns.len(),"minimum_active":samples.min_active,
            "max_waiting_members":samples.max_waiting_members,"max_conflict_reservations":samples.max_reservations,
            "waiting_decisions":samples.waiting_decisions,"conflict_decisions":samples.conflict_decisions,
            "max_waiting_decision_rows_per_tick":samples.max_waiting_candidates,"max_conflict_decision_rows_per_tick":samples.max_conflict_candidates,"transitions":samples.transitions,
            "minimum_renderer_visible_proxies":rendering.then_some(minimum_visible),
            "maximum_backlog_quanta":frame_backlog_quanta.iter().max(),
            "remaining_backlog_quanta":frame_backlog_quanta.last().copied().unwrap_or(0),
            "two_quantum_backlog_frames":frame_backlog_quanta.iter().filter(|&&n|n==2).count(),
            "max_backlog_recovery_frames":max_backlog_recovery_frames},
        "resource_loads":{"waiting_vehicle_ticks":samples.waiting_vehicle_ticks,"reservation_vehicle_ticks":samples.reservation_vehicle_ticks,
            "longest_observed_waiting_hold_ticks":samples.longest_waiting_hold,"longest_reservation_age_ticks":samples.longest_reservation_age,
            "vehicles_with_repeated_waiting_entries":samples.waiting_entries.iter().filter(|&&n|n>1).count(),
            "vehicles_with_repeated_reservation_acquisitions":samples.reservation_acquisitions.iter().filter(|&&n|n>1).count(),
            "waiting_evaluated_requests":samples.waiting_requests.iter().map(|&n|u64::from(n)).sum::<u64>(),
            "conflict_evaluated_requests":samples.conflict_requests.iter().map(|&n|u64::from(n)).sum::<u64>(),
            "vehicles_with_repeated_waiting_requests":samples.waiting_requests.iter().filter(|&&n|n>1).count(),
            "waiting_zones_with_repeated_requests":samples.waiting_zone_requests.values().filter(|&&n|n>1).count(),
            "waiting_zone_request_counts":samples.waiting_zone_requests,
            "vehicles_with_repeated_conflict_requests":samples.conflict_requests.iter().filter(|&&n|n>1).count()},
        "frame_classes":{"zero_step":frame_class(0),"one_step":frame_class(1),"two_step":frame_class(2)},
        "nanoseconds":{"tick_and_driver":percentiles(&samples.tick_ns),"domain_observation":percentiles(&samples.observation_ns),
            "pose_extraction":percentiles(pose_ns),"mapping_transform_apply":percentiles(apply_ns),"spatial_adapter":percentiles(&spatial_adapter_ns),"laneflow_frame_without_evidence":percentiles(&laneflow_frame_ns),"integrated_frame_with_evidence":percentiles(&frame_ns),"renderer_submit_and_gpu_wait":percentiles(&renderer_ns),"evidence_collection":percentiles(&samples.evidence_ns)},
        "allocation":{"tick_allocations":allocation.then_some(samples.tick_allocations),"tick_reallocations":allocation.then_some(samples.tick_reallocations)},
        "snapshot_payload_bytes":encode_lfrs(&session.world().capture_snapshot()?).len(),
        "pose_output_initialized_bytes":std::mem::size_of_val(poses.batch().records())+std::mem::size_of_val(poses.vehicles()),
        "shared_root_retained_logical_bytes":session.world().revision().retained_logical_bytes(),
        "missing_measurements":missing,"process_memory":"see runner metadata; not a component ledger",
        "samples_ns":{"tick_and_driver":samples.tick_ns,"domain_observation":samples.observation_ns,"pose_extraction":pose_ns,"mapping_transform_apply":apply_ns,"laneflow_frame_without_evidence":laneflow_frame_ns,"integrated_frame_with_evidence":frame_ns,"frame_step_counts":frame_steps,"frame_input_quanta":frame_input_quanta,"frame_backlog_quanta":frame_backlog_quanta,"renderer_submit_and_gpu_wait":renderer_ns,"evidence_collection":samples.evidence_ns,"presentation_validation":presentation.evidence_ns}
    });
    std::fs::write(&output, serde_json::to_vec(&result)?)?;
    if rendering && minimum_visible != apply_count {
        return Err("rendering culled presented proxies; inspect preserved result".into());
    }
    if allocation && (samples.tick_allocations != 0 || samples.tick_reallocations != 0) {
        return Err(
            "post-warmup tick allocation invariant failed; inspect preserved result".into(),
        );
    }
    eprintln!("wrote {}", output.display());
    Ok(())
}

#[cfg(test)]
mod bounded_run_tests {
    use super::*;

    #[test]
    fn total_tick_cap_includes_any_warmup() {
        assert!(RunLimits::new(0, 4_096, 600_000).is_ok());
        assert!(RunLimits::new(512, 4_096, 600_000).is_err());
        assert!(RunLimits::new(u64::MAX, 1, 600_000).is_err());
        assert!(RunLimits::new(0, 1, 600_001).is_err());
        assert!(RunLimits::new(0, 1, 0).is_err());
    }

    #[test]
    fn time_limit_preserves_partial_scope_and_reserves_finalization() {
        let limits = RunLimits::new(0, 4_096, 600_000).unwrap();
        assert_eq!(limits.stop_reason(1_024, Duration::from_secs(589)), None);
        assert_eq!(
            limits.stop_reason(1_024, Duration::from_secs(590)),
            Some("time-limit")
        );
        assert_eq!(
            limits.stop_reason(4_096, Duration::from_secs(100)),
            Some("tick-limit")
        );
        assert_eq!(
            limits.stop_reason(4_096, Duration::from_secs(590)),
            Some("time-limit")
        );
    }
}
