//! #212 P4 controller reduced-rate 研究原型。
//!
//! 本模块及其 seam 只存在于 `laneflow-core` 的测试构建。研究候选只缓存 IIDM
//! controller-intent comfort acceleration；occupancy/leader observation、safe-speed、
//! speed limit、route end、Signal/Parking spatial constraint、全局 projection、事件与
//! committed vehicle state 仍由每个 base tick 的生产 pipeline 重算并提交。

use std::{
    hint::black_box,
    mem::size_of,
    sync::Mutex,
    time::{Duration, Instant},
};

use super::*;
use crate::{
    EdgeLength, IidmProfileSpec, InitialTrafficData, LaneEdge, MovementGate, ParkingRegistry,
    ParkingReleaseReason, ParkingSpace, ParkingSpaceGeometry, Route, SignalAspect,
    SignalControlInput, SignalController, SignalGroup, SignalGroupState, SignalPhase,
    SignalRegistry, SpeedLimit, StopLine, StopLineLocation, VehicleProfileRegistry,
    VehicleSpawnInput,
    longitudinal::{
        ResearchMotionInput, compute_motion_from_controller_intent_for_research,
        evaluate_controller_intent_for_research,
    },
};
use criterion::{Criterion, Throughput};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats};

static REDUCED_RATE_ALLOCATION_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResearchPhaseMode {
    Synchronized,
    StableStaggered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResearchInvalidation {
    Minimal,
    SemanticReactive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResearchCacheMode {
    Transactional,
    SparseTransactional,
    IdealizedInPlace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReducedRateResearchConfig {
    period: u64,
    phase_mode: ResearchPhaseMode,
    invalidation: ResearchInvalidation,
    cache_when_period_one: bool,
    cache_mode: ResearchCacheMode,
}

impl ReducedRateResearchConfig {
    fn new(period: u64, phase_mode: ResearchPhaseMode, invalidation: ResearchInvalidation) -> Self {
        assert!(matches!(period, 1 | 2 | 4 | 8), "研究矩阵只允许 N=1/2/4/8");
        Self {
            period,
            phase_mode,
            invalidation,
            cache_when_period_one: false,
            cache_mode: ResearchCacheMode::Transactional,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResearchSignalStopIdentity {
    gate: MovementGateKey,
    stop_line: crate::StopLineHandle,
    group: SignalGroupHandle,
    from_route_edge_index: usize,
    to_route_edge_index: usize,
}

impl From<SignalStopConstraint> for ResearchSignalStopIdentity {
    fn from(value: SignalStopConstraint) -> Self {
        Self {
            gate: value.gate,
            stop_line: value.stop_line,
            group: value.group,
            from_route_edge_index: value.from_route_edge_index,
            to_route_edge_index: value.to_route_edge_index,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResearchParkingStopIdentity {
    space: crate::ParkingSpaceHandle,
    route: RouteHandle,
    route_edge_index: usize,
}

impl From<ParkingStopConstraint> for ResearchParkingStopIdentity {
    fn from(value: ParkingStopConstraint) -> Self {
        Self {
            space: value.space,
            route: value.route,
            route_edge_index: value.route_edge_index,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ResearchIntentCacheEntry {
    vehicle: VehicleHandle,
    profile: VehicleProfileHandle,
    route: RouteHandle,
    route_edge_index: usize,
    leader: Option<VehicleHandle>,
    signal_stop: Option<ResearchSignalStopIdentity>,
    parking_stop: Option<ResearchParkingStopIdentity>,
    comfort_acceleration: f64,
    refreshed_tick: u64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ResearchControllerContext {
    pub(super) vehicle: VehicleHandle,
    pub(super) update_sequence: u64,
    pub(super) next_tick_index: u64,
    pub(super) profile_handle: VehicleProfileHandle,
    pub(super) route: RouteHandle,
    pub(super) route_edge_index: usize,
    pub(super) current_speed: f64,
    pub(super) profile: crate::IidmProfileSpec,
    pub(super) effective_speed_ceiling: f64,
    pub(super) leader: Option<LeaderKinematics>,
    pub(super) signal_stop: Option<SignalStopConstraint>,
    pub(super) parking_stop: Option<ParkingStopConstraint>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ReducedRateStepMetrics {
    evaluated: usize,
    reused: usize,
    cadence_refreshes: usize,
    common_invalidations: usize,
    semantic_invalidations: usize,
    maximum_age: u64,
    occupancy_nanoseconds: u64,
    longitudinal_proposal_nanoseconds: u64,
    longitudinal_projection_nanoseconds: u64,
    longitudinal_nanoseconds: u64,
    post_longitudinal_nanoseconds: u64,
    research_commit_nanoseconds: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ReducedRateMetrics {
    committed_steps: u64,
    evaluated: u64,
    reused: u64,
    cadence_refreshes: u64,
    common_invalidations: u64,
    semantic_invalidations: u64,
    maximum_age: u64,
}

impl ReducedRateMetrics {
    fn commit(&mut self, step: ReducedRateStepMetrics) {
        self.committed_steps += 1;
        self.evaluated += step.evaluated as u64;
        self.reused += step.reused as u64;
        self.cadence_refreshes += step.cadence_refreshes as u64;
        self.common_invalidations += step.common_invalidations as u64;
        self.semantic_invalidations += step.semantic_invalidations as u64;
        self.maximum_age = self.maximum_age.max(step.maximum_age);
    }
}

#[derive(Clone, Debug)]
pub(super) struct ReducedRateResearchState {
    config: ReducedRateResearchConfig,
    committed: Vec<Option<ResearchIntentCacheEntry>>,
    candidate: Vec<Option<ResearchIntentCacheEntry>>,
    sparse_candidate: Vec<ResearchIntentCacheEntry>,
    sparse_invalidations: Vec<VehicleHandle>,
    candidate_metrics: ReducedRateStepMetrics,
    metrics: ReducedRateMetrics,
    last_committed_step: ReducedRateStepMetrics,
    active_tick: Option<u64>,
}

impl PartialEq for ReducedRateResearchState {
    fn eq(&self, _other: &Self) -> bool {
        // 研究 cache/metrics 不是 Core authority，也不参与失败原子性语义比较。
        true
    }
}

impl ReducedRateResearchState {
    fn new(config: ReducedRateResearchConfig) -> Self {
        Self {
            config,
            committed: Vec::new(),
            candidate: Vec::new(),
            sparse_candidate: Vec::new(),
            sparse_invalidations: Vec::new(),
            candidate_metrics: ReducedRateStepMetrics::default(),
            metrics: ReducedRateMetrics::default(),
            last_committed_step: ReducedRateStepMetrics::default(),
            active_tick: None,
        }
    }

    pub(super) fn begin_step(&mut self, vehicle_slot_count: usize, next_tick_index: u64) {
        let occupancy_nanoseconds = self.candidate_metrics.occupancy_nanoseconds;
        self.candidate_metrics = ReducedRateStepMetrics::default();
        self.candidate_metrics.occupancy_nanoseconds = occupancy_nanoseconds;
        self.active_tick = Some(next_tick_index);
        if !self.uses_cache() {
            return;
        }
        match self.config.cache_mode {
            ResearchCacheMode::Transactional => {
                self.candidate.clear();
                self.candidate
                    .reserve(vehicle_slot_count.saturating_sub(self.candidate.capacity()));
                self.candidate.extend(
                    self.committed
                        .iter()
                        .copied()
                        .chain(std::iter::repeat(None))
                        .take(vehicle_slot_count),
                );
            }
            ResearchCacheMode::SparseTransactional => {
                self.sparse_candidate.clear();
                self.sparse_invalidations.clear();
                self.sparse_candidate
                    .reserve(vehicle_slot_count.saturating_sub(self.sparse_candidate.capacity()));
                self.sparse_invalidations.reserve(
                    vehicle_slot_count.saturating_sub(self.sparse_invalidations.capacity()),
                );
                debug_assert!(
                    self.candidate.is_empty(),
                    "sparse transaction ablation must not retain dense transaction scratch"
                );
            }
            ResearchCacheMode::IdealizedInPlace => {
                self.committed.resize(vehicle_slot_count, None);
                debug_assert!(
                    self.candidate.is_empty()
                        && self.sparse_candidate.is_empty()
                        && self.sparse_invalidations.is_empty(),
                    "idealized in-place ablation must not retain transaction scratch"
                );
            }
        }
    }

    pub(super) fn invalidate(&mut self, vehicle: VehicleHandle) {
        match self.config.cache_mode {
            ResearchCacheMode::Transactional => {
                let Some(entry) = self.candidate.get_mut(vehicle.index()) else {
                    return;
                };
                *entry = None;
            }
            ResearchCacheMode::SparseTransactional => {
                if self
                    .committed
                    .get(vehicle.index())
                    .is_some_and(Option::is_some)
                {
                    self.sparse_invalidations.push(vehicle);
                }
            }
            ResearchCacheMode::IdealizedInPlace => {
                let Some(entry) = self.committed.get_mut(vehicle.index()) else {
                    return;
                };
                *entry = None;
            }
        }
    }

    pub(super) fn discard(&mut self, vehicle: VehicleHandle) {
        if let Some(entry) = self.committed.get_mut(vehicle.index()) {
            *entry = None;
        }
        if let Some(entry) = self.candidate.get_mut(vehicle.index()) {
            *entry = None;
        }
        self.sparse_candidate
            .retain(|entry| entry.vehicle != vehicle);
        self.sparse_invalidations.retain(|entry| *entry != vehicle);
    }

    pub(super) fn tracks_semantic_stop_identity(&self) -> bool {
        self.config.invalidation == ResearchInvalidation::SemanticReactive
    }

    pub(super) fn note_longitudinal_duration(&mut self, duration: Duration) {
        self.candidate_metrics.longitudinal_nanoseconds =
            u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
    }

    pub(super) fn note_occupancy_duration(&mut self, duration: Duration) {
        self.candidate_metrics.occupancy_nanoseconds =
            u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
    }

    pub(super) fn note_longitudinal_breakdown(&mut self, proposal: Duration, projection: Duration) {
        self.candidate_metrics.longitudinal_proposal_nanoseconds =
            u64::try_from(proposal.as_nanos()).unwrap_or(u64::MAX);
        self.candidate_metrics.longitudinal_projection_nanoseconds =
            u64::try_from(projection.as_nanos()).unwrap_or(u64::MAX);
    }

    pub(super) fn note_post_longitudinal_duration(&mut self, duration: Duration) {
        self.candidate_metrics.post_longitudinal_nanoseconds =
            u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
    }

    fn cadence_due(&self, next_tick_index: u64, update_sequence: u64) -> bool {
        let phase = match self.config.phase_mode {
            ResearchPhaseMode::Synchronized => 0,
            ResearchPhaseMode::StableStaggered => update_sequence % self.config.period,
        };
        (next_tick_index + phase).is_multiple_of(self.config.period)
    }

    fn uses_cache(&self) -> bool {
        self.config.period != 1 || self.config.cache_when_period_one
    }

    pub(super) fn controller_intent(
        &mut self,
        context: ResearchControllerContext,
    ) -> Result<f64, CoreError> {
        assert_eq!(
            self.active_tick,
            Some(context.next_tick_index),
            "research cache must begin exactly once before controller evaluation"
        );
        if !self.uses_cache() {
            let comfort_acceleration = evaluate_controller_intent_for_research(
                context.vehicle,
                context.current_speed,
                context.profile,
                context.effective_speed_ceiling,
                context.leader,
            )?;
            self.candidate_metrics.evaluated += 1;
            self.candidate_metrics.cadence_refreshes += 1;
            return Ok(comfort_acceleration);
        }
        let previous = self
            .committed
            .get(context.vehicle.index())
            .and_then(|entry| *entry);
        let common_compatible = previous.is_some_and(|entry| {
            entry.vehicle == context.vehicle
                && entry.profile == context.profile_handle
                && entry.route == context.route
                && entry.route_edge_index == context.route_edge_index
        });
        let leader = context.leader.map(|value| value.observation.leader);
        let signal_stop = context.signal_stop.map(ResearchSignalStopIdentity::from);
        let parking_stop = context.parking_stop.map(ResearchParkingStopIdentity::from);
        let semantic_compatible = previous.is_some_and(|entry| {
            entry.leader == leader
                && entry.signal_stop == signal_stop
                && entry.parking_stop == parking_stop
        });
        let cadence_due = self.cadence_due(context.next_tick_index, context.update_sequence);
        let semantic_refresh = self.config.invalidation == ResearchInvalidation::SemanticReactive
            && !semantic_compatible;
        let must_refresh = cadence_due || !common_compatible || semantic_refresh;

        if !must_refresh {
            let entry = previous.expect("compatible cache entry must exist");
            let age = context.next_tick_index - entry.refreshed_tick;
            self.candidate_metrics.reused += 1;
            self.candidate_metrics.maximum_age = self.candidate_metrics.maximum_age.max(age);
            return Ok(entry.comfort_acceleration);
        }

        let comfort_acceleration = evaluate_controller_intent_for_research(
            context.vehicle,
            context.current_speed,
            context.profile,
            context.effective_speed_ceiling,
            context.leader,
        )?;
        debug_assert!(comfort_acceleration.is_finite());
        let refreshed = ResearchIntentCacheEntry {
            vehicle: context.vehicle,
            profile: context.profile_handle,
            route: context.route,
            route_edge_index: context.route_edge_index,
            leader,
            signal_stop,
            parking_stop,
            comfort_acceleration,
            refreshed_tick: context.next_tick_index,
        };
        match self.config.cache_mode {
            ResearchCacheMode::Transactional => {
                self.candidate[context.vehicle.index()] = Some(refreshed);
            }
            ResearchCacheMode::SparseTransactional => {
                self.sparse_candidate.push(refreshed);
            }
            ResearchCacheMode::IdealizedInPlace => {
                self.committed[context.vehicle.index()] = Some(refreshed);
            }
        }
        self.candidate_metrics.evaluated += 1;
        if cadence_due {
            self.candidate_metrics.cadence_refreshes += 1;
        } else if !common_compatible {
            self.candidate_metrics.common_invalidations += 1;
        } else {
            self.candidate_metrics.semantic_invalidations += 1;
        }
        Ok(comfort_acceleration)
    }

    pub(super) fn commit_step(&mut self, vehicles: &[VehicleSlot], events: &[CoreEvent]) {
        let commit_started = Instant::now();
        let Some(_tick) = self.active_tick.take() else {
            return;
        };
        if self.uses_cache() {
            match self.config.cache_mode {
                ResearchCacheMode::Transactional => {
                    std::mem::swap(&mut self.committed, &mut self.candidate);
                    self.sweep_committed(vehicles);
                }
                ResearchCacheMode::SparseTransactional => {
                    self.commit_sparse(vehicles, events);
                }
                ResearchCacheMode::IdealizedInPlace => {
                    self.sweep_committed(vehicles);
                }
            }
        }
        self.candidate_metrics.research_commit_nanoseconds =
            u64::try_from(commit_started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.last_committed_step = self.candidate_metrics;
        self.metrics.commit(self.candidate_metrics);
        self.candidate_metrics = ReducedRateStepMetrics::default();
    }

    fn sweep_committed(&mut self, vehicles: &[VehicleSlot]) {
        for (index, entry) in self.committed.iter_mut().enumerate() {
            let keep = entry.is_some_and(|entry| {
                vehicles.get(index).is_some_and(|slot| {
                    slot.generation == entry.vehicle.generation()
                        && slot.state.as_ref().is_some_and(|vehicle| {
                            vehicle.handle == entry.vehicle
                                && vehicle.status == VehicleStatus::Active
                        })
                })
            });
            if !keep {
                *entry = None;
            }
        }
    }

    fn commit_sparse(&mut self, vehicles: &[VehicleSlot], events: &[CoreEvent]) {
        self.committed.resize(vehicles.len(), None);
        for entry in self.sparse_candidate.iter().copied() {
            let keep = vehicles.get(entry.vehicle.index()).is_some_and(|slot| {
                slot.generation == entry.vehicle.generation()
                    && slot.state.as_ref().is_some_and(|vehicle| {
                        vehicle.handle == entry.vehicle && vehicle.status == VehicleStatus::Active
                    })
            });
            self.committed[entry.vehicle.index()] = keep.then_some(entry);
        }
        for vehicle in self.sparse_invalidations.iter().copied() {
            let Some(entry) = self.committed.get_mut(vehicle.index()) else {
                continue;
            };
            if entry.is_some_and(|entry| entry.vehicle == vehicle) {
                *entry = None;
            }
        }
        for event in events {
            let CoreEvent::VehicleCompletedRoute(event) = event else {
                continue;
            };
            let Some(entry) = self.committed.get_mut(event.vehicle.index()) else {
                continue;
            };
            if entry.is_some_and(|entry| entry.vehicle == event.vehicle) {
                *entry = None;
            }
        }
        self.sparse_candidate.clear();
        self.sparse_invalidations.clear();
    }

    fn cache_retained_bytes(&self) -> usize {
        self.committed.capacity() * size_of::<Option<ResearchIntentCacheEntry>>()
    }

    fn transaction_scratch_retained_bytes(&self) -> usize {
        self.candidate.capacity() * size_of::<Option<ResearchIntentCacheEntry>>()
    }

    fn sparse_transaction_scratch_retained_bytes(&self) -> usize {
        self.sparse_candidate.capacity() * size_of::<ResearchIntentCacheEntry>()
            + self.sparse_invalidations.capacity() * size_of::<VehicleHandle>()
    }

    fn retained_bytes(&self) -> usize {
        self.cache_retained_bytes()
            + self.transaction_scratch_retained_bytes()
            + self.sparse_transaction_scratch_retained_bytes()
    }
}

fn research_profile() -> (VehicleProfileRegistry, VehicleProfileHandle) {
    let profiles = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        "reduced-rate-profile",
        IidmProfileSpec {
            length: 4.5,
            desired_speed: 20.0,
            min_gap: 2.0,
            time_headway: 1.25,
            max_acceleration: 1.8,
            comfortable_deceleration: 2.4,
            emergency_deceleration: 8.0,
        },
    )
    .expect("research profile")])
    .expect("research profile registry");
    let profile = profiles
        .profile_handle("reduced-rate-profile")
        .expect("research profile handle");
    (profiles, profile)
}

fn convoy_world(vehicle_count: usize) -> CoreWorld {
    let graph = LaneGraph::try_new([LaneEdge::new(
        "reduced-rate-edge",
        EdgeLength::try_new(20_000.0).expect("research edge length"),
        SpeedLimit::try_new(30.0).expect("research speed limit"),
        std::iter::empty::<&str>(),
    )])
    .expect("research graph");
    let (profiles, profile) = research_profile();
    let traffic = InitialTrafficData::try_new(
        graph,
        [Route::try_new("reduced-rate-route", ["reduced-rate-edge"]).expect("research route")],
        profiles,
    )
    .expect("research traffic");
    let vehicles = (0..vehicle_count)
        .map(|index| {
            VehicleSpawnInput::active(
                format!("vehicle-{index:06}"),
                profile,
                "reduced-rate-route",
                0,
                EdgeProgress::try_new(100.0 + index as f64 * 15.0).expect("research progress"),
                Speed::try_new(8.0 + (index % 5) as f64).expect("research speed"),
            )
        })
        .collect();
    CoreWorld::with_traffic_data(16, traffic, vehicles).expect("research world")
}

fn enable_reduced_rate(world: &mut CoreWorld, config: ReducedRateResearchConfig) {
    assert!(
        world.reduced_rate_research.is_none(),
        "research state must be enabled once"
    );
    world.reduced_rate_research = Some(ReducedRateResearchState::new(config));
}

fn assert_longitudinal_scratch_exact(left: &CoreWorld, right: &CoreWorld) {
    for vehicle in left.vehicles() {
        let left_motion = left.longitudinal_scratch.motion(vehicle.handle);
        let right_motion = right.longitudinal_scratch.motion(vehicle.handle);
        assert_eq!(
            left_motion.map(LongitudinalMotion::float_bits_for_research),
            right_motion.map(LongitudinalMotion::float_bits_for_research),
            "longitudinal float fields diverged for {:?}",
            vehicle.handle
        );
        assert_eq!(
            left_motion.and_then(LongitudinalMotion::leader_for_research),
            right_motion.and_then(LongitudinalMotion::leader_for_research),
            "leader identity diverged for {:?}",
            vehicle.handle
        );
    }
}

fn assert_authority_exact(left: &CoreWorld, right: &CoreWorld) {
    let mut left = left.clone();
    let mut right = right.clone();
    left.reduced_rate_research = None;
    right.reduced_rate_research = None;
    assert_eq!(left, right);
}

#[test]
fn n1_harness_is_bit_exact_with_production_pipeline() {
    let mut production = convoy_world(16);
    let mut research = production.clone();
    enable_reduced_rate(
        &mut research,
        ReducedRateResearchConfig::new(
            1,
            ResearchPhaseMode::Synchronized,
            ResearchInvalidation::Minimal,
        ),
    );

    for _ in 0..256 {
        let production_result = production
            .step(TickInput::new(16))
            .expect("production step");
        let research_result = research.step(TickInput::new(16)).expect("research step");
        assert_eq!(research_result, production_result);
        assert_authority_exact(&research, &production);
        assert_longitudinal_scratch_exact(&research, &production);
    }

    let state = research
        .reduced_rate_research
        .as_ref()
        .expect("research state");
    assert_eq!(state.metrics.committed_steps, 256);
    assert_eq!(state.metrics.evaluated, 16 * 256);
    assert_eq!(state.metrics.reused, 0);
    assert_eq!(
        state.retained_bytes(),
        0,
        "N=1 harness must not allocate a cache it cannot reuse"
    );
}

#[test]
fn transactional_cache_n1_is_bit_exact_and_measures_cache_bookkeeping_only() {
    let mut production = convoy_world(16);
    let mut cached = production.clone();
    enable_reduced_rate(
        &mut cached,
        ReducedRateResearchConfig {
            period: 1,
            phase_mode: ResearchPhaseMode::Synchronized,
            invalidation: ResearchInvalidation::Minimal,
            cache_when_period_one: true,
            cache_mode: ResearchCacheMode::Transactional,
        },
    );

    for _ in 0..256 {
        let production_result = production
            .step(TickInput::new(16))
            .expect("production step");
        let cached_result = cached.step(TickInput::new(16)).expect("cached step");
        assert_eq!(cached_result, production_result);
        assert_authority_exact(&cached, &production);
        assert_longitudinal_scratch_exact(&cached, &production);
    }

    let state = cached
        .reduced_rate_research
        .as_ref()
        .expect("cached research state");
    assert_eq!(state.metrics.committed_steps, 256);
    assert_eq!(state.metrics.evaluated, 16 * 256);
    assert_eq!(state.metrics.reused, 0);
    assert!(state.cache_retained_bytes() > 0);
    assert!(state.transaction_scratch_retained_bytes() > 0);
    assert!(state.committed.iter().all(Option::is_some));
}

#[test]
fn idealized_in_place_n8_matches_transactional_n8_on_successful_steps() {
    let mut transactional = convoy_world(32);
    let mut idealized = transactional.clone();
    enable_reduced_rate(
        &mut transactional,
        ReducedRateResearchConfig {
            period: 8,
            phase_mode: ResearchPhaseMode::StableStaggered,
            invalidation: ResearchInvalidation::SemanticReactive,
            cache_when_period_one: false,
            cache_mode: ResearchCacheMode::Transactional,
        },
    );
    enable_reduced_rate(
        &mut idealized,
        ReducedRateResearchConfig {
            period: 8,
            phase_mode: ResearchPhaseMode::StableStaggered,
            invalidation: ResearchInvalidation::SemanticReactive,
            cache_when_period_one: false,
            cache_mode: ResearchCacheMode::IdealizedInPlace,
        },
    );

    for _ in 0..512 {
        let transactional_result = transactional
            .step(TickInput::new(16))
            .expect("transactional step");
        let idealized_result = idealized.step(TickInput::new(16)).expect("idealized step");
        assert_eq!(idealized_result, transactional_result);
        assert_authority_exact(&idealized, &transactional);
        assert_longitudinal_scratch_exact(&idealized, &transactional);
        let transactional_state = transactional
            .reduced_rate_research
            .as_ref()
            .expect("transactional state");
        let idealized_state = idealized
            .reduced_rate_research
            .as_ref()
            .expect("idealized state");
        assert_eq!(idealized_state.committed, transactional_state.committed);
        assert_eq!(idealized_state.metrics, transactional_state.metrics);
    }

    let transactional_state = transactional
        .reduced_rate_research
        .as_ref()
        .expect("transactional state");
    let idealized_state = idealized
        .reduced_rate_research
        .as_ref()
        .expect("idealized state");
    assert!(transactional_state.transaction_scratch_retained_bytes() > 0);
    assert_eq!(idealized_state.transaction_scratch_retained_bytes(), 0);
}

#[test]
fn sparse_transaction_n8_matches_dense_transaction_n8_on_successful_steps() {
    let mut dense = convoy_world(32);
    let mut sparse = dense.clone();
    enable_reduced_rate(
        &mut dense,
        ReducedRateResearchConfig {
            period: 8,
            phase_mode: ResearchPhaseMode::StableStaggered,
            invalidation: ResearchInvalidation::SemanticReactive,
            cache_when_period_one: false,
            cache_mode: ResearchCacheMode::Transactional,
        },
    );
    enable_reduced_rate(
        &mut sparse,
        ReducedRateResearchConfig {
            period: 8,
            phase_mode: ResearchPhaseMode::StableStaggered,
            invalidation: ResearchInvalidation::SemanticReactive,
            cache_when_period_one: false,
            cache_mode: ResearchCacheMode::SparseTransactional,
        },
    );

    for _ in 0..512 {
        let dense_result = dense.step(TickInput::new(16)).expect("dense step");
        let sparse_result = sparse.step(TickInput::new(16)).expect("sparse step");
        assert_eq!(sparse_result, dense_result);
        assert_authority_exact(&sparse, &dense);
        assert_longitudinal_scratch_exact(&sparse, &dense);
        let dense_state = dense.reduced_rate_research.as_ref().expect("dense state");
        let sparse_state = sparse.reduced_rate_research.as_ref().expect("sparse state");
        assert_eq!(sparse_state.committed, dense_state.committed);
        assert_eq!(sparse_state.metrics, dense_state.metrics);
        assert!(sparse_state.sparse_candidate.is_empty());
        assert!(sparse_state.sparse_invalidations.is_empty());
    }
}

#[test]
fn sparse_transaction_n8_preserves_failed_step_atomicity_and_retry() {
    let mut world = convoy_world(16);
    enable_reduced_rate(
        &mut world,
        ReducedRateResearchConfig {
            period: 8,
            phase_mode: ResearchPhaseMode::StableStaggered,
            invalidation: ResearchInvalidation::SemanticReactive,
            cache_when_period_one: false,
            cache_mode: ResearchCacheMode::SparseTransactional,
        },
    );
    world.step(TickInput::new(16)).expect("warm sparse step");
    let mut replay = world.clone();
    let before_tick = world.tick_index();
    let before_state = world
        .reduced_rate_research
        .as_ref()
        .expect("sparse state")
        .clone();
    let failed_vehicle = world.vehicles().nth(3).expect("failed vehicle").handle;

    world.step_failure_after_vehicle = Some(failed_vehicle);
    world
        .step(TickInput::new(16))
        .expect_err("injected step failure");
    world.step_failure_after_vehicle = None;
    let after_failure = world.reduced_rate_research.as_ref().expect("sparse state");
    assert_eq!(world.tick_index(), before_tick);
    assert_eq!(after_failure.committed, before_state.committed);
    assert_eq!(after_failure.metrics, before_state.metrics);

    let retry = world.step(TickInput::new(16)).expect("retry succeeds");
    let fresh = replay
        .step(TickInput::new(16))
        .expect("fresh replay succeeds");
    assert_eq!(retry, fresh);
    assert_authority_exact(&world, &replay);
    assert_longitudinal_scratch_exact(&world, &replay);
    assert_eq!(
        world
            .reduced_rate_research
            .as_ref()
            .expect("retry state")
            .committed,
        replay
            .reduced_rate_research
            .as_ref()
            .expect("replay state")
            .committed
    );
}

#[test]
fn idealized_in_place_ablation_is_explicitly_not_failure_atomic() {
    let mut world = convoy_world(16);
    enable_reduced_rate(
        &mut world,
        ReducedRateResearchConfig {
            period: 8,
            phase_mode: ResearchPhaseMode::StableStaggered,
            invalidation: ResearchInvalidation::SemanticReactive,
            cache_when_period_one: false,
            cache_mode: ResearchCacheMode::IdealizedInPlace,
        },
    );
    world.step(TickInput::new(16)).expect("warm step");
    let before = world
        .reduced_rate_research
        .as_ref()
        .expect("idealized state")
        .committed
        .clone();
    let failed_vehicle = world.vehicles().next().expect("failed vehicle").handle;
    world.step_failure_after_vehicle = Some(failed_vehicle);
    world
        .step(TickInput::new(16))
        .expect_err("injected failure");
    world.step_failure_after_vehicle = None;
    let after = &world
        .reduced_rate_research
        .as_ref()
        .expect("idealized state")
        .committed;
    assert_ne!(
        *after, before,
        "the idealized upper bound must remain visibly ineligible for the failure-atomic gate"
    );
}

#[test]
fn failed_step_does_not_commit_reduced_rate_cache_or_metrics() {
    let mut world = convoy_world(8);
    enable_reduced_rate(
        &mut world,
        ReducedRateResearchConfig::new(
            4,
            ResearchPhaseMode::StableStaggered,
            ResearchInvalidation::SemanticReactive,
        ),
    );
    world.step(TickInput::new(16)).expect("warm research step");
    let mut replay = world.clone();
    let failed_vehicle = world.vehicles().nth(3).expect("failed vehicle").handle;
    let before_tick = world.tick_index();
    let before_state = world
        .reduced_rate_research
        .as_ref()
        .expect("research state")
        .clone();

    world.step_failure_after_vehicle = Some(failed_vehicle);
    world
        .step(TickInput::new(16))
        .expect_err("injected step failure");
    world.step_failure_after_vehicle = None;
    let after_failure = world
        .reduced_rate_research
        .as_ref()
        .expect("research state");
    assert_eq!(world.tick_index(), before_tick);
    assert_eq!(after_failure.committed, before_state.committed);
    assert_eq!(after_failure.metrics, before_state.metrics);

    let retry = world.step(TickInput::new(16)).expect("retry succeeds");
    let fresh = replay
        .step(TickInput::new(16))
        .expect("fresh replay succeeds");
    assert_eq!(retry, fresh);
    assert_eq!(world, replay);
    assert_longitudinal_scratch_exact(&world, &replay);
    assert_eq!(
        world
            .reduced_rate_research
            .as_ref()
            .expect("retry state")
            .committed,
        replay
            .reduced_rate_research
            .as_ref()
            .expect("replay state")
            .committed
    );
}

#[derive(Clone, Copy, Debug, Default)]
struct CandidateErrorSummary {
    maximum_speed_error: f64,
    maximum_progress_error: f64,
    maximum_gap_error: f64,
    maximum_normalized_speed_error: f64,
    maximum_normalized_distance_error: f64,
}

fn update_candidate_errors(
    production: &CoreWorld,
    candidate: &CoreWorld,
    speed_budget: f64,
    distance_budget: f64,
    summary: &mut CandidateErrorSummary,
) {
    let production_vehicles = production.vehicles().collect::<Vec<_>>();
    let candidate_vehicles = candidate.vehicles().collect::<Vec<_>>();
    assert_eq!(candidate_vehicles.len(), production_vehicles.len());

    for (actual, oracle) in candidate_vehicles.iter().zip(&production_vehicles) {
        assert_eq!(actual.handle, oracle.handle);
        assert_eq!(actual.status, oracle.status);
        assert_eq!(actual.route, oracle.route);
        assert_eq!(actual.route_edge_index, oracle.route_edge_index);
        assert!(actual.current_speed.value().is_finite());
        assert!(actual.edge_progress.value().is_finite());
        assert!(actual.applied_acceleration.value().is_finite());
        assert!(actual.current_speed.value() >= 0.0);

        let speed_error = (actual.current_speed.value() - oracle.current_speed.value()).abs();
        let progress_error = (actual.edge_progress.value() - oracle.edge_progress.value()).abs();
        summary.maximum_speed_error = summary.maximum_speed_error.max(speed_error);
        summary.maximum_progress_error = summary.maximum_progress_error.max(progress_error);
        if speed_budget > 0.0 {
            summary.maximum_normalized_speed_error = summary
                .maximum_normalized_speed_error
                .max(speed_error / speed_budget);
        } else {
            assert_eq!(speed_error, 0.0);
        }
        if distance_budget > 0.0 {
            summary.maximum_normalized_distance_error = summary
                .maximum_normalized_distance_error
                .max(progress_error / distance_budget);
        } else {
            assert_eq!(progress_error, 0.0);
        }
    }

    for (actual, oracle) in candidate_vehicles
        .windows(2)
        .zip(production_vehicles.windows(2))
    {
        let actual_gap = actual[1].edge_progress.value() - actual[0].edge_progress.value() - 4.5;
        let oracle_gap = oracle[1].edge_progress.value() - oracle[0].edge_progress.value() - 4.5;
        assert!(
            actual_gap >= -PHYSICAL_GAP_TOLERANCE_METERS,
            "candidate physical overlap: {actual_gap}"
        );
        let gap_error = (actual_gap - oracle_gap).abs();
        summary.maximum_gap_error = summary.maximum_gap_error.max(gap_error);
        if distance_budget > 0.0 {
            summary.maximum_normalized_distance_error = summary
                .maximum_normalized_distance_error
                .max(gap_error / distance_budget);
        } else {
            assert_eq!(gap_error, 0.0);
        }
    }
}

#[test]
fn cadence_and_invalidation_matrix_is_deterministic_and_budget_bounded() {
    const TICKS: usize = 512;
    let profile = research_profile()
        .0
        .profiles()
        .next()
        .expect("profile")
        .iidm();
    let delta_time = 0.016;

    for period in [1, 2, 4, 8] {
        let tau = (period - 1) as f64 * delta_time;
        let acceleration_span = profile.max_acceleration + profile.comfortable_deceleration;
        let speed_budget = acceleration_span * tau;
        let distance_budget = profile.desired_speed * tau + 0.5 * acceleration_span * tau * tau;

        for phase_mode in [
            ResearchPhaseMode::Synchronized,
            ResearchPhaseMode::StableStaggered,
        ] {
            for invalidation in [
                ResearchInvalidation::Minimal,
                ResearchInvalidation::SemanticReactive,
            ] {
                let config = ReducedRateResearchConfig::new(period, phase_mode, invalidation);
                let mut production = convoy_world(16);
                let mut candidate = production.clone();
                let mut replay = production.clone();
                enable_reduced_rate(&mut candidate, config);
                enable_reduced_rate(&mut replay, config);
                let mut summary = CandidateErrorSummary::default();

                for _ in 0..TICKS {
                    let production_result = production
                        .step(TickInput::new(16))
                        .expect("production step");
                    let candidate_result =
                        candidate.step(TickInput::new(16)).expect("candidate step");
                    let replay_result = replay.step(TickInput::new(16)).expect("replay step");
                    assert_eq!(candidate_result, replay_result);
                    assert_eq!(candidate_result.events, production_result.events);
                    assert_authority_exact(&candidate, &replay);
                    assert_longitudinal_scratch_exact(&candidate, &replay);
                    assert_eq!(
                        candidate
                            .reduced_rate_research
                            .as_ref()
                            .expect("candidate state")
                            .committed,
                        replay
                            .reduced_rate_research
                            .as_ref()
                            .expect("replay state")
                            .committed
                    );
                    update_candidate_errors(
                        &production,
                        &candidate,
                        speed_budget,
                        distance_budget,
                        &mut summary,
                    );
                }

                assert!(
                    summary.maximum_normalized_speed_error <= 1.0,
                    "{config:?} exceeded speed budget: {summary:?}"
                );
                assert!(
                    summary.maximum_normalized_distance_error <= 1.0,
                    "{config:?} exceeded distance/gap budget: {summary:?}"
                );
                let state = candidate
                    .reduced_rate_research
                    .as_ref()
                    .expect("candidate state");
                assert_eq!(state.metrics.committed_steps, TICKS as u64);
                assert!(
                    state.metrics.maximum_age < period,
                    "{config:?} exceeded cadence age: {:?}",
                    state.metrics
                );
                if period == 1 {
                    assert_eq!(state.metrics.reused, 0);
                    assert_eq!(summary.maximum_speed_error, 0.0);
                    assert_eq!(summary.maximum_progress_error, 0.0);
                    assert_eq!(summary.maximum_gap_error, 0.0);
                } else {
                    assert!(state.metrics.reused > 0);
                }
            }
        }
    }
}

fn percentile_f64(samples: &mut [f64], numerator: usize, denominator: usize) -> f64 {
    assert!(!samples.is_empty());
    samples.sort_by(f64::total_cmp);
    samples[(samples.len() - 1) * numerator / denominator]
}

fn maximum_progress_error(production: &CoreWorld, candidate: &CoreWorld) -> f64 {
    production
        .vehicles()
        .zip(candidate.vehicles())
        .map(|(oracle, actual)| {
            assert_eq!(actual.handle, oracle.handle);
            (actual.edge_progress.value() - oracle.edge_progress.value()).abs()
        })
        .fold(0.0, f64::max)
}

#[test]
fn stable_semantic_fidelity_report_includes_distributions_and_horizon_drift() {
    const HORIZON: usize = 128;
    const TICKS: usize = HORIZON * 4;
    let profile = research_profile()
        .0
        .profiles()
        .next()
        .expect("profile")
        .iidm();
    let delta_time = 0.016;

    for period in [2, 4, 8] {
        let tau = (period - 1) as f64 * delta_time;
        let acceleration_span = profile.max_acceleration + profile.comfortable_deceleration;
        let speed_budget = acceleration_span * tau;
        let distance_budget = profile.desired_speed * tau + 0.5 * acceleration_span * tau * tau;
        let mut production = convoy_world(16);
        let mut candidate = production.clone();
        enable_reduced_rate(
            &mut candidate,
            ReducedRateResearchConfig::new(
                period,
                ResearchPhaseMode::StableStaggered,
                ResearchInvalidation::SemanticReactive,
            ),
        );
        let mut speed_errors = Vec::with_capacity(TICKS * 16);
        let mut progress_errors = Vec::with_capacity(TICKS * 16);
        let mut gap_errors = Vec::with_capacity(TICKS * 15);
        let mut horizon_drifts = [0.0; 3];

        for tick in 1..=TICKS {
            production
                .step(TickInput::new(16))
                .expect("production step");
            candidate.step(TickInput::new(16)).expect("candidate step");
            let production_vehicles = production.vehicles().collect::<Vec<_>>();
            let candidate_vehicles = candidate.vehicles().collect::<Vec<_>>();
            for (actual, oracle) in candidate_vehicles.iter().zip(&production_vehicles) {
                speed_errors
                    .push((actual.current_speed.value() - oracle.current_speed.value()).abs());
                progress_errors
                    .push((actual.edge_progress.value() - oracle.edge_progress.value()).abs());
            }
            for (actual, oracle) in candidate_vehicles
                .windows(2)
                .zip(production_vehicles.windows(2))
            {
                let actual_gap =
                    actual[1].edge_progress.value() - actual[0].edge_progress.value() - 4.5;
                let oracle_gap =
                    oracle[1].edge_progress.value() - oracle[0].edge_progress.value() - 4.5;
                gap_errors.push((actual_gap - oracle_gap).abs());
            }
            if tick == HORIZON {
                horizon_drifts[0] = maximum_progress_error(&production, &candidate);
            } else if tick == HORIZON * 2 {
                horizon_drifts[1] = maximum_progress_error(&production, &candidate);
            } else if tick == HORIZON * 4 {
                horizon_drifts[2] = maximum_progress_error(&production, &candidate);
            }
        }

        let speed_p50 = percentile_f64(&mut speed_errors.clone(), 50, 100);
        let speed_p95 = percentile_f64(&mut speed_errors.clone(), 95, 100);
        let speed_max = speed_errors.into_iter().fold(0.0, f64::max);
        let progress_p50 = percentile_f64(&mut progress_errors.clone(), 50, 100);
        let progress_p95 = percentile_f64(&mut progress_errors.clone(), 95, 100);
        let progress_max = progress_errors.into_iter().fold(0.0, f64::max);
        let gap_p50 = percentile_f64(&mut gap_errors.clone(), 50, 100);
        let gap_p95 = percentile_f64(&mut gap_errors.clone(), 95, 100);
        let gap_max = gap_errors.into_iter().fold(0.0, f64::max);
        println!(
            "REDUCED_RATE_FIDELITY period={period} speed_budget={speed_budget:.9} \
             distance_budget={distance_budget:.9} speed_p50={speed_p50:.9} \
             speed_p95={speed_p95:.9} speed_max={speed_max:.9} \
             progress_pose_p50={progress_p50:.9} progress_pose_p95={progress_p95:.9} \
             progress_pose_max={progress_max:.9} gap_p50={gap_p50:.9} \
             gap_p95={gap_p95:.9} gap_max={gap_max:.9} \
             drift_h={:.9} drift_2h={:.9} drift_4h={:.9}",
            horizon_drifts[0], horizon_drifts[1], horizon_drifts[2]
        );
        assert!(speed_max <= speed_budget);
        assert!(progress_max <= distance_budget);
        assert!(gap_max <= distance_budget);
    }
}

#[test]
fn semantic_reactive_refreshes_immediately_when_leader_identity_changes() {
    let mut world = convoy_world(2);
    enable_reduced_rate(
        &mut world,
        ReducedRateResearchConfig::new(
            8,
            ResearchPhaseMode::Synchronized,
            ResearchInvalidation::SemanticReactive,
        ),
    );
    world.step(TickInput::new(16)).expect("warm step");
    let follower = world.vehicle_handle("vehicle-000000").expect("follower");
    let leader = world.vehicle_handle("vehicle-000001").expect("leader");
    let state = world
        .reduced_rate_research
        .as_ref()
        .expect("research state");
    assert_eq!(
        state.committed[follower.index()]
            .expect("follower cache")
            .leader,
        Some(leader)
    );
    let semantic_before = state.metrics.semantic_invalidations;

    world.despawn_vehicle(leader).expect("leader despawn");
    assert!(
        world
            .reduced_rate_research
            .as_ref()
            .expect("research state")
            .committed[leader.index()]
        .is_none(),
        "despawn must discard the old generation cache immediately"
    );
    world.step(TickInput::new(16)).expect("leader-change step");

    let state = world
        .reduced_rate_research
        .as_ref()
        .expect("research state");
    assert_eq!(state.metrics.semantic_invalidations, semantic_before + 1);
    assert_eq!(
        state.committed[follower.index()]
            .expect("refreshed follower cache")
            .leader,
        None
    );
}

fn two_edge_world() -> CoreWorld {
    let graph = LaneGraph::try_new([
        LaneEdge::new(
            "transition-a",
            EdgeLength::try_new(50.0).expect("edge A length"),
            SpeedLimit::try_new(30.0).expect("edge A speed limit"),
            ["transition-b"],
        ),
        LaneEdge::new(
            "transition-b",
            EdgeLength::try_new(50.0).expect("edge B length"),
            SpeedLimit::try_new(30.0).expect("edge B speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("transition graph");
    let (profiles, profile) = research_profile();
    let traffic = InitialTrafficData::try_new(
        graph,
        [
            Route::try_new("transition-route", ["transition-a", "transition-b"])
                .expect("transition route"),
        ],
        profiles,
    )
    .expect("transition traffic");
    CoreWorld::with_traffic_data(
        100,
        traffic,
        vec![VehicleSpawnInput::active(
            "transition-vehicle",
            profile,
            "transition-route",
            0,
            EdgeProgress::try_new(49.0).expect("transition progress"),
            Speed::try_new(4.0).expect("transition speed"),
        )],
    )
    .expect("transition world")
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LifecycleEventIdentity {
    ChangedEdge {
        vehicle: VehicleHandle,
        route: RouteHandle,
        from_edge: EdgeHandle,
        to_edge: EdgeHandle,
        from_route_edge_index: usize,
        to_route_edge_index: usize,
    },
    Completed {
        vehicle: VehicleHandle,
        route: RouteHandle,
        edge: EdgeHandle,
        route_edge_index: usize,
    },
}

fn lifecycle_events(result: &StepResult) -> Vec<(u64, LifecycleEventIdentity)> {
    result
        .events
        .iter()
        .filter_map(|event| match event {
            CoreEvent::VehicleChangedEdge(event) => Some((
                event.tick_index,
                LifecycleEventIdentity::ChangedEdge {
                    vehicle: event.vehicle,
                    route: event.route,
                    from_edge: event.from_edge,
                    to_edge: event.to_edge,
                    from_route_edge_index: event.from_route_edge_index,
                    to_route_edge_index: event.to_route_edge_index,
                },
            )),
            CoreEvent::VehicleCompletedRoute(event) => Some((
                event.tick_index,
                LifecycleEventIdentity::Completed {
                    vehicle: event.vehicle,
                    route: event.route,
                    edge: event.edge,
                    route_edge_index: event.route_edge_index,
                },
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn lifecycle_events_preserve_identity_order_and_single_window_tick_shift() {
    let mut production = two_edge_world();
    let mut candidate = production.clone();
    enable_reduced_rate(
        &mut candidate,
        ReducedRateResearchConfig::new(
            8,
            ResearchPhaseMode::StableStaggered,
            ResearchInvalidation::SemanticReactive,
        ),
    );
    let mut production_events = Vec::new();
    let mut candidate_events = Vec::new();

    for _ in 0..256 {
        let production_result = production
            .step(TickInput::new(100))
            .expect("production step");
        let candidate_result = candidate.step(TickInput::new(100)).expect("candidate step");
        production_events.extend(lifecycle_events(&production_result));
        candidate_events.extend(lifecycle_events(&candidate_result));
        if production
            .vehicles()
            .all(|vehicle| vehicle.status == VehicleStatus::Completed)
            && candidate
                .vehicles()
                .all(|vehicle| vehicle.status == VehicleStatus::Completed)
        {
            break;
        }
    }

    assert_eq!(candidate_events.len(), production_events.len());
    assert_eq!(candidate_events.len(), 2);
    for ((candidate_tick, candidate), (production_tick, production)) in
        candidate_events.iter().zip(&production_events)
    {
        assert_eq!(candidate, production);
        assert!(
            candidate_tick.abs_diff(*production_tick) <= 7,
            "event tick shift exceeded N-1: candidate={candidate_tick}, production={production_tick}"
        );
    }
    assert!(matches!(
        candidate_events.as_slice(),
        [
            (_, LifecycleEventIdentity::ChangedEdge { .. }),
            (_, LifecycleEventIdentity::Completed { .. })
        ]
    ));
    assert!(
        candidate
            .reduced_rate_research
            .as_ref()
            .expect("research state")
            .metrics
            .common_invalidations
            > 0,
        "route occurrence transition must force a common invalidation"
    );
    assert!(
        candidate
            .reduced_rate_research
            .as_ref()
            .expect("research state")
            .committed
            .iter()
            .all(Option::is_none),
        "Completed vehicles must not retain controller intent"
    );
}

#[test]
fn sparse_transaction_clears_reused_entry_on_same_tick_route_completion() {
    let mut dense = two_edge_world();
    let mut sparse = dense.clone();
    enable_reduced_rate(
        &mut dense,
        ReducedRateResearchConfig {
            period: 8,
            phase_mode: ResearchPhaseMode::StableStaggered,
            invalidation: ResearchInvalidation::SemanticReactive,
            cache_when_period_one: false,
            cache_mode: ResearchCacheMode::Transactional,
        },
    );
    enable_reduced_rate(
        &mut sparse,
        ReducedRateResearchConfig {
            period: 8,
            phase_mode: ResearchPhaseMode::StableStaggered,
            invalidation: ResearchInvalidation::SemanticReactive,
            cache_when_period_one: false,
            cache_mode: ResearchCacheMode::SparseTransactional,
        },
    );

    for _ in 0..256 {
        let dense_result = dense.step(TickInput::new(100)).expect("dense step");
        let sparse_result = sparse.step(TickInput::new(100)).expect("sparse step");
        assert_eq!(sparse_result, dense_result);
        assert_authority_exact(&sparse, &dense);
        assert_longitudinal_scratch_exact(&sparse, &dense);
        assert_eq!(
            sparse
                .reduced_rate_research
                .as_ref()
                .expect("sparse state")
                .committed,
            dense
                .reduced_rate_research
                .as_ref()
                .expect("dense state")
                .committed
        );
        if sparse
            .vehicles()
            .all(|vehicle| vehicle.status == VehicleStatus::Completed)
        {
            break;
        }
    }

    assert!(
        sparse
            .reduced_rate_research
            .as_ref()
            .expect("sparse state")
            .committed
            .iter()
            .all(Option::is_none)
    );
}

fn signal_phase(id: &str, duration_ms: u64, aspect: SignalAspect) -> SignalPhase {
    SignalPhase::new(
        id,
        duration_ms,
        [SignalGroupState::new("signal-group", aspect)],
    )
}

fn signal_only_world() -> CoreWorld {
    let graph = LaneGraph::try_new([
        LaneEdge::new(
            "signal-entry",
            EdgeLength::try_new(100.0).expect("signal entry length"),
            SpeedLimit::try_new(30.0).expect("signal entry speed limit"),
            ["signal-exit"],
        ),
        LaneEdge::new(
            "signal-exit",
            EdgeLength::try_new(100.0).expect("signal exit length"),
            SpeedLimit::try_new(30.0).expect("signal exit speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("signal graph");
    let signals = SignalRegistry::try_new(
        &graph,
        [StopLine::new(
            "signal-stop",
            "signal-entry",
            StopLineLocation::EdgeEnd,
        )],
        [SignalGroup::new("signal-group")],
        [SignalController::new_fixed_time(
            "signal-controller",
            7,
            ["signal-group"],
            [
                signal_phase("red", 32, SignalAspect::Red),
                signal_phase("green", 48, SignalAspect::Green),
                signal_phase("yellow", 16, SignalAspect::Yellow),
            ],
        )],
        [MovementGate::new(
            "signal-entry",
            "signal-exit",
            "signal-stop",
            SignalControlInput::Group("signal-group".to_owned()),
        )],
    )
    .expect("signal registry");
    let traffic = InitialTrafficData::try_new_with_signals(
        graph,
        [Route::try_new("signal-route", ["signal-entry", "signal-exit"]).expect("signal route")],
        VehicleProfileRegistry::empty(),
        signals,
    )
    .expect("signal traffic");
    CoreWorld::with_traffic_data(16, traffic, Vec::new()).expect("signal world")
}

#[test]
fn signal_phase_and_group_events_remain_exact_on_every_base_tick() {
    let mut production = signal_only_world();
    let mut candidate = production.clone();
    enable_reduced_rate(
        &mut candidate,
        ReducedRateResearchConfig::new(
            8,
            ResearchPhaseMode::StableStaggered,
            ResearchInvalidation::SemanticReactive,
        ),
    );

    for _ in 0..256 {
        let production_result = production
            .step(TickInput::new(16))
            .expect("production step");
        let candidate_result = candidate.step(TickInput::new(16)).expect("candidate step");
        assert_eq!(candidate_result, production_result);
        assert_authority_exact(&candidate, &production);
    }
}

fn reserved_parking_world(start_progress: f64, edge_length: f64) -> CoreWorld {
    let graph = LaneGraph::try_new([LaneEdge::new(
        "parking-edge",
        EdgeLength::try_new(edge_length).expect("parking edge length"),
        SpeedLimit::try_new(30.0).expect("parking speed limit"),
        std::iter::empty::<&str>(),
    )])
    .expect("parking graph");
    let parking = ParkingRegistry::try_new(
        &graph,
        [],
        [ParkingSpace::new(
            "parking-space",
            None,
            "parking-edge",
            20.0,
            "parking-edge",
            25.0,
            ParkingSpaceGeometry::new(-3.0, 0.0, 4.5, 2.4),
        )],
    )
    .expect("parking registry");
    let (profiles, profile) = research_profile();
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        graph,
        [Route::try_new("parking-route", ["parking-edge"]).expect("parking route")],
        profiles,
        SignalRegistry::empty(),
        parking,
    )
    .expect("parking traffic");
    let mut world = CoreWorld::with_traffic_data(
        16,
        traffic,
        vec![VehicleSpawnInput::active(
            "parking-vehicle",
            profile,
            "parking-route",
            0,
            EdgeProgress::try_new(start_progress).expect("parking start progress"),
            Speed::try_new(4.0).expect("parking start speed"),
        )],
    )
    .expect("parking world");
    let vehicle = world.vehicle_handle("parking-vehicle").expect("vehicle");
    let space = world
        .parking()
        .space_handle("parking-space")
        .expect("space");
    world
        .reserve_parking_space(vehicle, space)
        .expect("parking reservation");
    world
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ParkingSemanticEventIdentity {
    Arrival {
        vehicle: VehicleHandle,
        space: crate::ParkingSpaceHandle,
        route: RouteHandle,
        route_edge_index: usize,
    },
    Release {
        vehicle: VehicleHandle,
        space: crate::ParkingSpaceHandle,
        reason: ParkingReleaseReason,
    },
    Completed {
        vehicle: VehicleHandle,
        route: RouteHandle,
        edge: EdgeHandle,
        route_edge_index: usize,
    },
}

type TimedParkingSemanticEvents = Vec<(u64, ParkingSemanticEventIdentity)>;

fn parking_semantic_events(result: &StepResult) -> Vec<(u64, ParkingSemanticEventIdentity)> {
    result
        .events
        .iter()
        .filter_map(|event| match event {
            CoreEvent::VehicleParkingArrivalReached(event) => Some((
                event.tick_index,
                ParkingSemanticEventIdentity::Arrival {
                    vehicle: event.vehicle,
                    space: event.space,
                    route: event.route,
                    route_edge_index: event.route_edge_index,
                },
            )),
            CoreEvent::ParkingReservationReleased(event) => Some((
                event.tick_index,
                ParkingSemanticEventIdentity::Release {
                    vehicle: event.vehicle,
                    space: event.space,
                    reason: event.reason,
                },
            )),
            CoreEvent::VehicleCompletedRoute(event) => Some((
                event.tick_index,
                ParkingSemanticEventIdentity::Completed {
                    vehicle: event.vehicle,
                    route: event.route,
                    edge: event.edge,
                    route_edge_index: event.route_edge_index,
                },
            )),
            _ => None,
        })
        .collect()
}

fn collect_parking_semantic_events(
    production: &mut CoreWorld,
    candidate: &mut CoreWorld,
    maximum_ticks: usize,
) -> (TimedParkingSemanticEvents, TimedParkingSemanticEvents) {
    let mut production_events = Vec::new();
    let mut candidate_events = Vec::new();
    for _ in 0..maximum_ticks {
        let production_result = production
            .step(TickInput::new(16))
            .expect("production step");
        let candidate_result = candidate.step(TickInput::new(16)).expect("candidate step");
        production_events.extend(parking_semantic_events(&production_result));
        candidate_events.extend(parking_semantic_events(&candidate_result));
        if !production_events.is_empty()
            && production_events.last().is_some_and(|(_, event)| {
                matches!(event, ParkingSemanticEventIdentity::Arrival { .. })
            })
            && !candidate_events.is_empty()
            && candidate_events.last().is_some_and(|(_, event)| {
                matches!(event, ParkingSemanticEventIdentity::Arrival { .. })
            })
        {
            break;
        }
        if production
            .vehicles()
            .all(|vehicle| vehicle.status == VehicleStatus::Completed)
            && candidate
                .vehicles()
                .all(|vehicle| vehicle.status == VehicleStatus::Completed)
        {
            break;
        }
    }
    (production_events, candidate_events)
}

fn assert_semantic_event_window(
    production: &[(u64, ParkingSemanticEventIdentity)],
    candidate: &[(u64, ParkingSemanticEventIdentity)],
) {
    assert_eq!(candidate.len(), production.len());
    for ((candidate_tick, candidate), (production_tick, production)) in
        candidate.iter().zip(production)
    {
        assert_eq!(candidate, production);
        assert!(
            candidate_tick.abs_diff(*production_tick) <= 7,
            "Parking semantic event exceeded N-1 window"
        );
    }
}

#[test]
fn parking_arrival_preserves_payload_and_semantic_stop_identity_refresh() {
    let mut production = reserved_parking_world(0.0, 100.0);
    let mut candidate = production.clone();
    enable_reduced_rate(
        &mut candidate,
        ReducedRateResearchConfig::new(
            8,
            ResearchPhaseMode::StableStaggered,
            ResearchInvalidation::SemanticReactive,
        ),
    );

    let (production_events, candidate_events) =
        collect_parking_semantic_events(&mut production, &mut candidate, 2_000);
    assert_semantic_event_window(&production_events, &candidate_events);
    assert!(matches!(
        candidate_events.as_slice(),
        [(_, ParkingSemanticEventIdentity::Arrival { .. })]
    ));
    assert!(
        candidate
            .reduced_rate_research
            .as_ref()
            .expect("research state")
            .metrics
            .semantic_invalidations
            > 0,
        "ParkingStop identity appearance must force semantic refresh"
    );
}

#[test]
fn route_completion_release_precedes_completed_with_exact_payloads() {
    let mut production = reserved_parking_world(30.0, 60.0);
    let mut candidate = production.clone();
    enable_reduced_rate(
        &mut candidate,
        ReducedRateResearchConfig::new(
            8,
            ResearchPhaseMode::StableStaggered,
            ResearchInvalidation::SemanticReactive,
        ),
    );

    let (production_events, candidate_events) =
        collect_parking_semantic_events(&mut production, &mut candidate, 2_000);
    assert_semantic_event_window(&production_events, &candidate_events);
    assert!(matches!(
        candidate_events.as_slice(),
        [
            (_, ParkingSemanticEventIdentity::Release { .. }),
            (_, ParkingSemanticEventIdentity::Completed { .. })
        ]
    ));
    assert_eq!(candidate_events[0].0, candidate_events[1].0);
}

fn mixed_scale_world(vehicle_count: usize) -> CoreWorld {
    const COHORT_SIZE: usize = 16;
    const FOLLOWING_SPACING: f64 = 15.0;
    const COHORT_GAP: f64 = 120.0;

    let cohort_span = COHORT_SIZE as f64 * FOLLOWING_SPACING + COHORT_GAP;
    let cohort_count = vehicle_count.div_ceil(COHORT_SIZE);
    let edge_length = 1_000.0 + cohort_count as f64 * cohort_span;
    let graph = LaneGraph::try_new([LaneEdge::new(
        "mixed-scale-edge",
        EdgeLength::try_new(edge_length).expect("mixed scale edge length"),
        SpeedLimit::try_new(30.0).expect("mixed scale speed limit"),
        std::iter::empty::<&str>(),
    )])
    .expect("mixed scale graph");
    let (profiles, profile) = research_profile();
    let traffic = InitialTrafficData::try_new(
        graph,
        [Route::try_new("mixed-scale-route", ["mixed-scale-edge"]).expect("mixed scale route")],
        profiles,
    )
    .expect("mixed scale traffic");
    let vehicles = (0..vehicle_count)
        .map(|index| {
            let cohort = index / COHORT_SIZE;
            let within_cohort = index % COHORT_SIZE;
            let progress =
                100.0 + cohort as f64 * cohort_span + within_cohort as f64 * FOLLOWING_SPACING;
            VehicleSpawnInput::active(
                format!("mixed-vehicle-{index:06}"),
                profile,
                "mixed-scale-route",
                0,
                EdgeProgress::try_new(progress).expect("mixed scale progress"),
                Speed::try_new(7.0 + (index % 7) as f64).expect("mixed scale speed"),
            )
        })
        .collect();
    CoreWorld::with_traffic_data(16, traffic, vehicles).expect("mixed scale world")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PerformanceCase {
    Production,
    HarnessN1,
    TransactionalCacheN1,
    StableSemanticN4,
    StableSemanticN8,
    SparseTransactionalN8,
    IdealizedInPlaceN8,
    SynchronizedSemanticN8,
}

impl PerformanceCase {
    const TAIL_CASES: [Self; 5] = [
        Self::Production,
        Self::HarnessN1,
        Self::StableSemanticN4,
        Self::StableSemanticN8,
        Self::SynchronizedSemanticN8,
    ];
    const CRITERION_CASES: [Self; 3] = [Self::Production, Self::HarnessN1, Self::StableSemanticN8];
    const ABLATION_CASES: [Self; 5] = [
        Self::Production,
        Self::HarnessN1,
        Self::TransactionalCacheN1,
        Self::StableSemanticN8,
        Self::IdealizedInPlaceN8,
    ];
    const ABLATION_LONGITUDINAL_CASES: [Self; 4] = [
        Self::HarnessN1,
        Self::TransactionalCacheN1,
        Self::StableSemanticN8,
        Self::IdealizedInPlaceN8,
    ];
    const SPARSE_ABLATION_CASES: [Self; 5] = [
        Self::Production,
        Self::HarnessN1,
        Self::StableSemanticN8,
        Self::SparseTransactionalN8,
        Self::IdealizedInPlaceN8,
    ];
    const SPARSE_ABLATION_LONGITUDINAL_CASES: [Self; 4] = [
        Self::HarnessN1,
        Self::StableSemanticN8,
        Self::SparseTransactionalN8,
        Self::IdealizedInPlaceN8,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::Production => "P0-production",
            Self::HarnessN1 => "H1-N1",
            Self::TransactionalCacheN1 => "C1-transactional-cache-N1",
            Self::StableSemanticN4 => "stable-semantic-N4",
            Self::StableSemanticN8 => "C2-transactional-cache-N8",
            Self::SparseTransactionalN8 => "C4-sparse-transaction-N8",
            Self::IdealizedInPlaceN8 => "C3-idealized-in-place-N8",
            Self::SynchronizedSemanticN8 => "synchronized-semantic-N8",
        }
    }

    const fn config(self) -> Option<ReducedRateResearchConfig> {
        match self {
            Self::Production => None,
            Self::HarnessN1 => Some(ReducedRateResearchConfig {
                period: 1,
                phase_mode: ResearchPhaseMode::Synchronized,
                invalidation: ResearchInvalidation::Minimal,
                cache_when_period_one: false,
                cache_mode: ResearchCacheMode::Transactional,
            }),
            Self::TransactionalCacheN1 => Some(ReducedRateResearchConfig {
                period: 1,
                phase_mode: ResearchPhaseMode::Synchronized,
                invalidation: ResearchInvalidation::Minimal,
                cache_when_period_one: true,
                cache_mode: ResearchCacheMode::Transactional,
            }),
            Self::StableSemanticN4 => Some(ReducedRateResearchConfig {
                period: 4,
                phase_mode: ResearchPhaseMode::StableStaggered,
                invalidation: ResearchInvalidation::SemanticReactive,
                cache_when_period_one: false,
                cache_mode: ResearchCacheMode::Transactional,
            }),
            Self::StableSemanticN8 => Some(ReducedRateResearchConfig {
                period: 8,
                phase_mode: ResearchPhaseMode::StableStaggered,
                invalidation: ResearchInvalidation::SemanticReactive,
                cache_when_period_one: false,
                cache_mode: ResearchCacheMode::Transactional,
            }),
            Self::SparseTransactionalN8 => Some(ReducedRateResearchConfig {
                period: 8,
                phase_mode: ResearchPhaseMode::StableStaggered,
                invalidation: ResearchInvalidation::SemanticReactive,
                cache_when_period_one: false,
                cache_mode: ResearchCacheMode::SparseTransactional,
            }),
            Self::IdealizedInPlaceN8 => Some(ReducedRateResearchConfig {
                period: 8,
                phase_mode: ResearchPhaseMode::StableStaggered,
                invalidation: ResearchInvalidation::SemanticReactive,
                cache_when_period_one: false,
                cache_mode: ResearchCacheMode::IdealizedInPlace,
            }),
            Self::SynchronizedSemanticN8 => Some(ReducedRateResearchConfig {
                period: 8,
                phase_mode: ResearchPhaseMode::Synchronized,
                invalidation: ResearchInvalidation::SemanticReactive,
                cache_when_period_one: false,
                cache_mode: ResearchCacheMode::Transactional,
            }),
        }
    }
}

fn world_for_performance_case(base: &CoreWorld, case: PerformanceCase) -> CoreWorld {
    let mut world = base.clone();
    if let Some(config) = case.config() {
        enable_reduced_rate(&mut world, config);
    }
    world
}

#[derive(Clone, Copy, Debug)]
struct LatencySummary {
    p50_ns: u64,
    p95_ns: u64,
    p99_ns: u64,
    maximum_ns: u64,
}

impl LatencySummary {
    fn from_samples(samples: &mut [u64]) -> Self {
        assert!(!samples.is_empty());
        samples.sort_unstable();
        Self {
            p50_ns: percentile(samples, 50, 100),
            p95_ns: percentile(samples, 95, 100),
            p99_ns: percentile(samples, 99, 100),
            maximum_ns: *samples.last().expect("latency sample"),
        }
    }
}

fn percentile(sorted: &[u64], numerator: usize, denominator: usize) -> u64 {
    let index = (sorted.len() - 1) * numerator / denominator;
    sorted[index]
}

#[derive(Clone, Copy, Debug)]
struct PerformanceObservation {
    whole_step: LatencySummary,
    longitudinal: Option<LatencySummary>,
}

fn observe_tail(world: &mut CoreWorld, ticks: usize) -> PerformanceObservation {
    for _ in 0..32 {
        black_box(world.step(TickInput::new(16)).expect("warm step"));
    }
    let mut whole_step = Vec::with_capacity(ticks);
    let mut longitudinal = world
        .reduced_rate_research
        .as_ref()
        .map(|_| Vec::with_capacity(ticks));
    for _ in 0..ticks {
        let started = Instant::now();
        black_box(world.step(TickInput::new(16)).expect("observed step"));
        whole_step.push(u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX));
        if let (Some(samples), Some(research)) = (&mut longitudinal, &world.reduced_rate_research) {
            samples.push(research.last_committed_step.longitudinal_nanoseconds);
        }
    }
    PerformanceObservation {
        whole_step: LatencySummary::from_samples(&mut whole_step),
        longitudinal: longitudinal
            .as_mut()
            .map(|samples| LatencySummary::from_samples(samples)),
    }
}

const DIAGNOSTIC_STAGE_NAMES: [&str; 9] = [
    "whole-step",
    "occupancy-leader",
    "longitudinal-proposal-store",
    "global-projection",
    "longitudinal-unattributed",
    "longitudinal-total",
    "advance-events-authority-commit",
    "research-cache-commit",
    "whole-step-unattributed",
];

fn observe_step_stage_diagnostics(
    world: &mut CoreWorld,
    ticks: usize,
) -> [LatencySummary; DIAGNOSTIC_STAGE_NAMES.len()] {
    for _ in 0..32 {
        black_box(
            world
                .step(TickInput::new(16))
                .expect("diagnostic warm step"),
        );
    }
    let mut samples: [Vec<u64>; DIAGNOSTIC_STAGE_NAMES.len()] =
        std::array::from_fn(|_| Vec::with_capacity(ticks));
    for _ in 0..ticks {
        let whole_started = Instant::now();
        black_box(
            world
                .step(TickInput::new(16))
                .expect("diagnostic observed step"),
        );
        let whole = u64::try_from(whole_started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let metrics = world
            .reduced_rate_research
            .as_ref()
            .expect("diagnostic research state")
            .last_committed_step;
        let longitudinal_unattributed = metrics
            .longitudinal_nanoseconds
            .saturating_sub(metrics.longitudinal_proposal_nanoseconds)
            .saturating_sub(metrics.longitudinal_projection_nanoseconds);
        let whole_unattributed = whole
            .saturating_sub(metrics.occupancy_nanoseconds)
            .saturating_sub(metrics.longitudinal_nanoseconds)
            .saturating_sub(metrics.post_longitudinal_nanoseconds)
            .saturating_sub(metrics.research_commit_nanoseconds);
        for (index, value) in [
            whole,
            metrics.occupancy_nanoseconds,
            metrics.longitudinal_proposal_nanoseconds,
            metrics.longitudinal_projection_nanoseconds,
            longitudinal_unattributed,
            metrics.longitudinal_nanoseconds,
            metrics.post_longitudinal_nanoseconds,
            metrics.research_commit_nanoseconds,
            whole_unattributed,
        ]
        .into_iter()
        .enumerate()
        {
            samples[index].push(value);
        }
    }
    samples.map(|mut samples| LatencySummary::from_samples(&mut samples))
}

#[test]
#[ignore = "explicit release-mode 100k three-round coarse step-stage diagnostics"]
fn release_100k_three_round_step_stage_diagnostics() {
    const DIAGNOSTIC_TICKS: usize = 512;
    let base = mixed_scale_world(100_000);
    for round in 1..=3 {
        for case in [
            PerformanceCase::HarnessN1,
            PerformanceCase::SparseTransactionalN8,
        ] {
            let mut world = world_for_performance_case(&base, case);
            let observations = observe_step_stage_diagnostics(&mut world, DIAGNOSTIC_TICKS);
            for (stage, observation) in DIAGNOSTIC_STAGE_NAMES.into_iter().zip(observations) {
                println!(
                    "REDUCED_RATE_STAGE round={round} case={} stage={stage} \
                     p50_ns={} p95_ns={} p99_ns={} max_ns={}",
                    case.name(),
                    observation.p50_ns,
                    observation.p95_ns,
                    observation.p99_ns,
                    observation.maximum_ns,
                );
            }
            let whole = observations[0].p50_ns;
            let occupancy = observations[1].p50_ns;
            let proposal = observations[2].p50_ns;
            let projection = observations[3].p50_ns;
            let longitudinal = observations[5].p50_ns;
            let post = observations[6].p50_ns;
            let cache_commit = observations[7].p50_ns;
            println!(
                "REDUCED_RATE_STAGE_SHARE round={round} case={} \
                 occupancy_vs_whole_pct={:.3} longitudinal_vs_whole_pct={:.3} \
                 proposal_vs_longitudinal_pct={:.3} projection_vs_longitudinal_pct={:.3} \
                 post_vs_whole_pct={:.3} cache_commit_vs_whole_pct={:.3}",
                case.name(),
                occupancy as f64 / whole as f64 * 100.0,
                longitudinal as f64 / whole as f64 * 100.0,
                proposal as f64 / longitudinal as f64 * 100.0,
                projection as f64 / longitudinal as f64 * 100.0,
                post as f64 / whole as f64 * 100.0,
                cache_commit as f64 / whole as f64 * 100.0,
            );
        }
    }
}

fn percentage_delta(candidate: u64, baseline: u64) -> f64 {
    (candidate as f64 / baseline as f64 - 1.0) * 100.0
}

#[test]
#[ignore = "explicit release-mode 10k/100k, three-round, 1024-tick tail matrix"]
fn release_tail_matrix_reports_p50_p95_p99_max_and_gate_classification() {
    const TAIL_TICKS: usize = 1_024;
    const ROUNDS: usize = 3;

    for vehicle_count in [10_000, 100_000] {
        let base = mixed_scale_world(vehicle_count);
        let mut round_observations = Vec::new();
        for round in 1..=ROUNDS {
            let mut observations = Vec::new();
            for case in PerformanceCase::TAIL_CASES {
                let mut world = world_for_performance_case(&base, case);
                let observation = observe_tail(&mut world, TAIL_TICKS);
                let research = world.reduced_rate_research.as_ref();
                println!(
                    "REDUCED_RATE_TAIL scale={vehicle_count} round={round} case={} \
                     whole_p50_ns={} whole_p95_ns={} whole_p99_ns={} whole_max_ns={} \
                     longitudinal={:?} cache_bytes={} transaction_scratch_bytes={} \
                     cache_entry_bytes={}",
                    case.name(),
                    observation.whole_step.p50_ns,
                    observation.whole_step.p95_ns,
                    observation.whole_step.p99_ns,
                    observation.whole_step.maximum_ns,
                    observation.longitudinal,
                    research.map_or(0, ReducedRateResearchState::cache_retained_bytes),
                    research.map_or(
                        0,
                        ReducedRateResearchState::transaction_scratch_retained_bytes
                    ),
                    size_of::<Option<ResearchIntentCacheEntry>>(),
                );
                observations.push((case, observation));
            }
            let p0 = observations
                .iter()
                .find(|(case, _)| *case == PerformanceCase::Production)
                .expect("P0 observation")
                .1;
            let h1 = observations
                .iter()
                .find(|(case, _)| *case == PerformanceCase::HarnessN1)
                .expect("H1 observation")
                .1;
            println!(
                "REDUCED_RATE_HARNESS scale={vehicle_count} round={round} \
                 whole_p50_delta_pct={:.3}",
                percentage_delta(h1.whole_step.p50_ns, p0.whole_step.p50_ns)
            );
            for case in [
                PerformanceCase::StableSemanticN4,
                PerformanceCase::StableSemanticN8,
            ] {
                let candidate = observations
                    .iter()
                    .find(|(observed, _)| *observed == case)
                    .expect("candidate observation")
                    .1;
                let h1_longitudinal = h1.longitudinal.expect("H1 longitudinal");
                let candidate_longitudinal =
                    candidate.longitudinal.expect("candidate longitudinal");
                let longitudinal_gain =
                    -percentage_delta(candidate_longitudinal.p50_ns, h1_longitudinal.p50_ns);
                let whole_gain =
                    -percentage_delta(candidate.whole_step.p50_ns, h1.whole_step.p50_ns);
                let tail_pass = candidate.whole_step.p95_ns
                    <= h1.whole_step.p95_ns.saturating_mul(105) / 100
                    && candidate.whole_step.p99_ns
                        <= h1.whole_step.p99_ns.saturating_mul(105) / 100;
                let scale_pass = if vehicle_count == 10_000 {
                    candidate.whole_step.p50_ns <= h1.whole_step.p50_ns.saturating_mul(105) / 100
                } else {
                    longitudinal_gain >= 15.0 && whole_gain >= 5.0
                };
                println!(
                    "REDUCED_RATE_GATE scale={vehicle_count} round={round} case={} \
                     longitudinal_gain_pct={longitudinal_gain:.3} \
                     whole_gain_pct={whole_gain:.3} tail_pass={tail_pass} \
                     scale_pass={scale_pass}",
                    case.name(),
                );
            }
            round_observations.push(observations);
        }

        let mut harness_deltas = round_observations
            .iter()
            .map(|observations| {
                let p0 = observations
                    .iter()
                    .find(|(case, _)| *case == PerformanceCase::Production)
                    .expect("P0")
                    .1;
                let h1 = observations
                    .iter()
                    .find(|(case, _)| *case == PerformanceCase::HarnessN1)
                    .expect("H1")
                    .1;
                percentage_delta(h1.whole_step.p50_ns, p0.whole_step.p50_ns)
            })
            .collect::<Vec<_>>();
        harness_deltas.sort_by(f64::total_cmp);
        let harness_median_delta = harness_deltas[harness_deltas.len() / 2];
        println!(
            "REDUCED_RATE_HARNESS_SUMMARY scale={vehicle_count} \
             paired_deltas_pct={harness_deltas:?} median_delta_pct={harness_median_delta:.3}"
        );
        assert!(
            harness_median_delta <= 5.0,
            "H1 three-round whole-step median exceeded the frozen 5% harness guard"
        );
    }
}

fn assert_zero_warm_allocation(vehicle_count: usize, case: PerformanceCase, stats: Stats) {
    assert_eq!(
        stats.allocations,
        0,
        "{vehicle_count} {} allocations",
        case.name()
    );
    assert_eq!(
        stats.reallocations,
        0,
        "{vehicle_count} {} reallocations",
        case.name()
    );
    assert_eq!(
        stats.bytes_allocated,
        0,
        "{vehicle_count} {} allocated bytes",
        case.name()
    );
    assert_eq!(
        stats.bytes_reallocated,
        0,
        "{vehicle_count} {} reallocated bytes",
        case.name()
    );
}

#[test]
#[ignore = "global allocator measurement requires explicit serial release execution"]
fn warm_10k_100k_reduced_rate_step_is_zero_allocation() {
    let _guard = REDUCED_RATE_ALLOCATION_LOCK
        .lock()
        .expect("allocation lock");
    for vehicle_count in [10_000, 100_000] {
        let base = mixed_scale_world(vehicle_count);
        for case in [
            PerformanceCase::HarnessN1,
            PerformanceCase::TransactionalCacheN1,
            PerformanceCase::StableSemanticN8,
            PerformanceCase::SparseTransactionalN8,
            PerformanceCase::IdealizedInPlaceN8,
        ] {
            let mut world = world_for_performance_case(&base, case);
            for _ in 0..32 {
                black_box(world.step(TickInput::new(16)).expect("warm step"));
            }
            let region = Region::new(&INSTRUMENTED_SYSTEM);
            for _ in 0..16 {
                black_box(world.step(TickInput::new(16)).expect("measured step"));
            }
            assert_zero_warm_allocation(vehicle_count, case, region.change());
            let research = world
                .reduced_rate_research
                .as_ref()
                .expect("allocation research state");
            println!(
                "REDUCED_RATE_ALLOCATION scale={vehicle_count} case={} cache_bytes={} \
                 dense_transaction_scratch_bytes={} sparse_transaction_scratch_bytes={} \
                 retained_bytes={}",
                case.name(),
                research.cache_retained_bytes(),
                research.transaction_scratch_retained_bytes(),
                research.sparse_transaction_scratch_retained_bytes(),
                research.retained_bytes(),
            );
        }
    }
}

#[test]
fn cache_and_transaction_scratch_report_exact_linear_retained_bytes() {
    let entry_size = size_of::<Option<ResearchIntentCacheEntry>>();
    let mut previous = None;
    for vehicle_count in [16, 257, 1_024] {
        let mut world = convoy_world(vehicle_count);
        enable_reduced_rate(
            &mut world,
            ReducedRateResearchConfig::new(
                8,
                ResearchPhaseMode::StableStaggered,
                ResearchInvalidation::SemanticReactive,
            ),
        );
        world.step(TickInput::new(16)).expect("warm cache step");
        let research = world
            .reduced_rate_research
            .as_ref()
            .expect("research state");
        assert_eq!(
            research.cache_retained_bytes(),
            research.committed.capacity() * entry_size
        );
        assert_eq!(
            research.transaction_scratch_retained_bytes(),
            research.candidate.capacity() * entry_size
        );
        assert_eq!(
            research.retained_bytes(),
            research.cache_retained_bytes() + research.transaction_scratch_retained_bytes()
        );
        if let Some((previous_count, previous_bytes)) = previous {
            assert!(
                research.retained_bytes() * previous_count <= previous_bytes * vehicle_count * 2,
                "retained memory must stay O(V)"
            );
        }
        previous = Some((vehicle_count, research.retained_bytes()));
    }

    let base = mixed_scale_world(1_024);
    let mut idealized = world_for_performance_case(&base, PerformanceCase::IdealizedInPlaceN8);
    idealized
        .step(TickInput::new(16))
        .expect("idealized warm cache step");
    let research = idealized
        .reduced_rate_research
        .as_ref()
        .expect("idealized research state");
    assert!(research.cache_retained_bytes() >= 1_024 * entry_size);
    assert_eq!(research.transaction_scratch_retained_bytes(), 0);
    assert_eq!(research.retained_bytes(), research.cache_retained_bytes());

    let mut sparse = world_for_performance_case(&base, PerformanceCase::SparseTransactionalN8);
    sparse
        .step(TickInput::new(16))
        .expect("sparse warm cache step");
    let research = sparse
        .reduced_rate_research
        .as_ref()
        .expect("sparse research state");
    assert!(research.cache_retained_bytes() >= 1_024 * entry_size);
    assert_eq!(research.transaction_scratch_retained_bytes(), 0);
    assert_eq!(
        research.sparse_transaction_scratch_retained_bytes(),
        research.sparse_candidate.capacity() * size_of::<ResearchIntentCacheEntry>()
            + research.sparse_invalidations.capacity() * size_of::<VehicleHandle>()
    );
    assert_eq!(
        research.retained_bytes(),
        research.cache_retained_bytes() + research.sparse_transaction_scratch_retained_bytes()
    );
}

fn warm_performance_world(world: &mut CoreWorld) {
    for _ in 0..32 {
        black_box(world.step(TickInput::new(16)).expect("Criterion warm step"));
    }
}

#[test]
#[ignore = "explicit release-mode Criterion median.point_estimate evidence"]
fn criterion_100k_three_round_whole_step_and_longitudinal_matrix() {
    let base = mixed_scale_world(100_000);
    let mut criterion = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .without_plots();

    for round in 1..=3 {
        let mut whole_worlds = PerformanceCase::CRITERION_CASES
            .into_iter()
            .map(|case| {
                let mut world = world_for_performance_case(&base, case);
                warm_performance_world(&mut world);
                (case, world)
            })
            .collect::<Vec<_>>();
        let mut whole_group =
            criterion.benchmark_group(format!("reduced-rate/100k/round-{round}/whole-step"));
        whole_group.throughput(Throughput::Elements(100_000));
        for (case, world) in &mut whole_worlds {
            whole_group.bench_function(case.name(), |bencher| {
                bencher.iter(|| {
                    black_box(world.step(TickInput::new(16)).expect("Criterion step"));
                });
            });
        }
        whole_group.finish();

        let mut longitudinal_worlds = [
            PerformanceCase::HarnessN1,
            PerformanceCase::StableSemanticN8,
        ]
        .into_iter()
        .map(|case| {
            let mut world = world_for_performance_case(&base, case);
            warm_performance_world(&mut world);
            (case, world)
        })
        .collect::<Vec<_>>();
        let mut longitudinal_group =
            criterion.benchmark_group(format!("reduced-rate/100k/round-{round}/longitudinal"));
        longitudinal_group.throughput(Throughput::Elements(100_000));
        for (case, world) in &mut longitudinal_worlds {
            longitudinal_group.bench_function(case.name(), |bencher| {
                bencher.iter_custom(|iterations| {
                    let mut measured = Duration::ZERO;
                    for _ in 0..iterations {
                        black_box(world.step(TickInput::new(16)).expect("Criterion step"));
                        let research = world
                            .reduced_rate_research
                            .as_ref()
                            .expect("Criterion research state");
                        measured = measured.saturating_add(Duration::from_nanos(
                            research.last_committed_step.longitudinal_nanoseconds,
                        ));
                    }
                    measured
                });
            });
        }
        longitudinal_group.finish();
    }
    criterion.final_summary();
}

#[derive(Clone, Copy)]
struct ControllerBenchmarkInput {
    vehicle: VehicleHandle,
    current_speed: f64,
    leader: Option<LeaderKinematics>,
}

#[derive(Clone, Copy)]
struct MotionBenchmarkInput {
    controller: ControllerBenchmarkInput,
    update_sequence: u64,
    comfort_acceleration: f64,
}

fn controller_benchmark_inputs(vehicle_count: usize) -> Vec<ControllerBenchmarkInput> {
    const COHORT_SIZE: usize = 16;
    (0..vehicle_count)
        .map(|index| {
            let current_speed = 7.0 + (index % 7) as f64;
            let leader = (!(index + 1).is_multiple_of(COHORT_SIZE)).then(|| LeaderKinematics {
                observation: LeaderObservation {
                    leader: VehicleHandle::new(index + 1, 0),
                    bumper_gap: 10.5,
                },
                current_speed: 7.0 + ((index + 1) % 7) as f64,
                emergency_deceleration: 8.0,
            });
            ControllerBenchmarkInput {
                vehicle: VehicleHandle::new(index, 0),
                current_speed,
                leader,
            }
        })
        .collect()
}

fn motion_benchmark_inputs(
    controller_inputs: &[ControllerBenchmarkInput],
    profile: IidmProfileSpec,
) -> Vec<MotionBenchmarkInput> {
    controller_inputs
        .iter()
        .copied()
        .map(|controller| MotionBenchmarkInput {
            update_sequence: u64::try_from(controller.vehicle.index())
                .expect("benchmark update sequence must fit in u64"),
            comfort_acceleration: evaluate_controller_intent_for_research(
                controller.vehicle,
                controller.current_speed,
                profile,
                30.0,
                controller.leader,
            )
            .expect("motion benchmark controller intent"),
            controller,
        })
        .collect()
}

#[test]
#[ignore = "explicit release-mode 100k three-round cache/downrate ablation"]
fn criterion_100k_three_round_cache_and_downrate_ablation() {
    const VEHICLE_COUNT: usize = 100_000;
    let base = mixed_scale_world(VEHICLE_COUNT);
    let controller_inputs = controller_benchmark_inputs(VEHICLE_COUNT);
    let profile = research_profile()
        .0
        .profiles()
        .next()
        .expect("profile")
        .iidm();
    let mut criterion = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .without_plots();

    for round in 1..=3 {
        let mut whole_worlds = PerformanceCase::ABLATION_CASES
            .into_iter()
            .map(|case| {
                let mut world = world_for_performance_case(&base, case);
                warm_performance_world(&mut world);
                (case, world)
            })
            .collect::<Vec<_>>();
        let mut whole_group = criterion.benchmark_group(format!(
            "reduced-rate-ablation/100k/round-{round}/whole-step"
        ));
        whole_group.throughput(Throughput::Elements(VEHICLE_COUNT as u64));
        for (case, world) in &mut whole_worlds {
            whole_group.bench_function(case.name(), |bencher| {
                bencher.iter(|| {
                    black_box(world.step(TickInput::new(16)).expect("Criterion step"));
                });
            });
        }
        whole_group.finish();

        let mut longitudinal_worlds = PerformanceCase::ABLATION_LONGITUDINAL_CASES
            .into_iter()
            .map(|case| {
                let mut world = world_for_performance_case(&base, case);
                warm_performance_world(&mut world);
                (case, world)
            })
            .collect::<Vec<_>>();
        let mut longitudinal_group = criterion.benchmark_group(format!(
            "reduced-rate-ablation/100k/round-{round}/longitudinal"
        ));
        longitudinal_group.throughput(Throughput::Elements(VEHICLE_COUNT as u64));
        for (case, world) in &mut longitudinal_worlds {
            longitudinal_group.bench_function(case.name(), |bencher| {
                bencher.iter_custom(|iterations| {
                    let mut measured = Duration::ZERO;
                    for _ in 0..iterations {
                        black_box(world.step(TickInput::new(16)).expect("Criterion step"));
                        let research = world
                            .reduced_rate_research
                            .as_ref()
                            .expect("Criterion research state");
                        measured = measured.saturating_add(Duration::from_nanos(
                            research.last_committed_step.longitudinal_nanoseconds,
                        ));
                    }
                    measured
                });
            });
        }
        longitudinal_group.finish();

        let mut controller_group = criterion.benchmark_group(format!(
            "reduced-rate-ablation/100k/round-{round}/controller-only"
        ));
        controller_group.throughput(Throughput::Elements(VEHICLE_COUNT as u64));
        controller_group.bench_function("IIDM-intent-evaluation", |bencher| {
            bencher.iter(|| {
                let mut checksum = 0.0;
                for input in &controller_inputs {
                    checksum += evaluate_controller_intent_for_research(
                        input.vehicle,
                        input.current_speed,
                        profile,
                        30.0,
                        input.leader,
                    )
                    .expect("controller benchmark evaluation");
                }
                black_box(checksum);
            });
        });
        controller_group.finish();
    }
    criterion.final_summary();
}

#[test]
#[ignore = "explicit release-mode 100k three-round sparse transaction ablation"]
fn criterion_100k_three_round_sparse_transaction_ablation() {
    const VEHICLE_COUNT: usize = 100_000;
    let base = mixed_scale_world(VEHICLE_COUNT);
    let mut criterion = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .without_plots();

    for round in 1..=3 {
        let mut whole_worlds = PerformanceCase::SPARSE_ABLATION_CASES
            .into_iter()
            .map(|case| {
                let mut world = world_for_performance_case(&base, case);
                warm_performance_world(&mut world);
                (case, world)
            })
            .collect::<Vec<_>>();
        let mut whole_group = criterion.benchmark_group(format!(
            "reduced-rate-sparse-ablation/100k/round-{round}/whole-step"
        ));
        whole_group.throughput(Throughput::Elements(VEHICLE_COUNT as u64));
        for (case, world) in &mut whole_worlds {
            whole_group.bench_function(case.name(), |bencher| {
                bencher.iter(|| {
                    black_box(world.step(TickInput::new(16)).expect("Criterion step"));
                });
            });
        }
        whole_group.finish();

        let mut longitudinal_worlds = PerformanceCase::SPARSE_ABLATION_LONGITUDINAL_CASES
            .into_iter()
            .map(|case| {
                let mut world = world_for_performance_case(&base, case);
                warm_performance_world(&mut world);
                (case, world)
            })
            .collect::<Vec<_>>();
        let mut longitudinal_group = criterion.benchmark_group(format!(
            "reduced-rate-sparse-ablation/100k/round-{round}/longitudinal"
        ));
        longitudinal_group.throughput(Throughput::Elements(VEHICLE_COUNT as u64));
        for (case, world) in &mut longitudinal_worlds {
            longitudinal_group.bench_function(case.name(), |bencher| {
                bencher.iter_custom(|iterations| {
                    let mut measured = Duration::ZERO;
                    for _ in 0..iterations {
                        black_box(world.step(TickInput::new(16)).expect("Criterion step"));
                        let research = world
                            .reduced_rate_research
                            .as_ref()
                            .expect("Criterion research state");
                        measured = measured.saturating_add(Duration::from_nanos(
                            research.last_committed_step.longitudinal_nanoseconds,
                        ));
                    }
                    measured
                });
            });
        }
        longitudinal_group.finish();
    }
    criterion.final_summary();
}

#[test]
#[ignore = "explicit release-mode 100k H1 workload for external sampled profiling"]
fn release_100k_h1_external_profile_workload() {
    const PROFILE_TICKS: usize = 1_024;
    let base = mixed_scale_world(100_000);
    let mut world = world_for_performance_case(&base, PerformanceCase::HarnessN1);
    warm_performance_world(&mut world);
    for _ in 0..PROFILE_TICKS {
        black_box(world.step(TickInput::new(16)).expect("profile step"));
    }
}

#[test]
#[ignore = "explicit release-mode 100k three-round longitudinal kernel diagnostics"]
fn criterion_100k_three_round_longitudinal_kernel_diagnostics() {
    const VEHICLE_COUNT: usize = 100_000;
    let base = mixed_scale_world(VEHICLE_COUNT);
    let mut world = world_for_performance_case(&base, PerformanceCase::HarnessN1);
    warm_performance_world(&mut world);
    let update_order = world.vehicle_update_order.clone();
    let motions = update_order
        .iter()
        .map(|vehicle| {
            world
                .longitudinal_scratch
                .motion(vehicle)
                .expect("profile world motion")
        })
        .collect::<Vec<_>>();
    assert_eq!(motions.len(), VEHICLE_COUNT);
    let controller_inputs = controller_benchmark_inputs(VEHICLE_COUNT);
    let profile = research_profile()
        .0
        .profiles()
        .next()
        .expect("profile")
        .iidm();
    let motion_inputs = motion_benchmark_inputs(&controller_inputs, profile);
    let mut criterion = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .without_plots();

    for round in 1..=3 {
        let mut group = criterion.benchmark_group(format!(
            "reduced-rate-diagnostics/100k/round-{round}/longitudinal-kernels"
        ));
        group.throughput(Throughput::Elements(VEHICLE_COUNT as u64));

        group.bench_function("iidm-intent", |bencher| {
            bencher.iter(|| {
                let mut checksum = 0.0;
                for input in &controller_inputs {
                    checksum += evaluate_controller_intent_for_research(
                        input.vehicle,
                        input.current_speed,
                        profile,
                        30.0,
                        input.leader,
                    )
                    .expect("diagnostic controller evaluation");
                }
                black_box(checksum);
            });
        });

        group.bench_function("post-intent-motion", |bencher| {
            bencher.iter(|| {
                let mut checksum = 0.0;
                for input in &motion_inputs {
                    let controller = input.controller;
                    let motion =
                        compute_motion_from_controller_intent_for_research(ResearchMotionInput {
                            vehicle: controller.vehicle,
                            update_sequence: input.update_sequence,
                            current_speed: controller.current_speed,
                            profile,
                            effective_speed_ceiling: 30.0,
                            leader: controller.leader,
                            comfort_acceleration: input.comfort_acceleration,
                            delta_time: 0.016,
                        })
                        .expect("diagnostic post-intent motion");
                    checksum += motion.final_speed() + motion.final_travel();
                }
                black_box(checksum);
            });
        });

        let mut begin_scratch = world.longitudinal_scratch.clone();
        group.bench_function("scratch-begin", |bencher| {
            bencher.iter_custom(|iterations| {
                let mut measured = Duration::ZERO;
                for _ in 0..iterations {
                    let started = Instant::now();
                    begin_scratch.begin(VEHICLE_COUNT);
                    measured = measured.saturating_add(started.elapsed());
                    black_box(&begin_scratch);
                }
                measured
            });
        });

        let mut store_scratch = LongitudinalScratch::default();
        store_scratch.begin(VEHICLE_COUNT);
        group.bench_function("motion-store", |bencher| {
            bencher.iter_custom(|iterations| {
                let mut measured = Duration::ZERO;
                for _ in 0..iterations {
                    store_scratch.begin(VEHICLE_COUNT);
                    let started = Instant::now();
                    for motion in &motions {
                        store_scratch.set(*motion);
                    }
                    measured = measured.saturating_add(started.elapsed());
                    black_box(&store_scratch);
                }
                measured
            });
        });

        let mut projection_scratch = world.longitudinal_scratch.clone();
        group.bench_function("global-projection", |bencher| {
            bencher.iter_custom(|iterations| {
                let mut measured = Duration::ZERO;
                for _ in 0..iterations {
                    projection_scratch.reset_geometry_projection_for_research();
                    let started = Instant::now();
                    projection_scratch
                        .project(update_order.iter(), 0.016)
                        .expect("diagnostic projection");
                    measured = measured.saturating_add(started.elapsed());
                    black_box(&projection_scratch);
                }
                measured
            });
        });

        group.finish();
    }
    criterion.final_summary();
}
