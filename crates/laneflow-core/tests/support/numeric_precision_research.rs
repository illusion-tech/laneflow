//! #122 research-only precision candidates.
//!
//! This module intentionally stays outside production Core. It mirrors the current one-dimensional
//! IIDM, safe-speed, ballistic integration and acyclic no-overlap projection pipeline for the
//! representative platoon workloads. `F64Mode` is checked against `CoreWorld` before the f32 modes
//! are used as comparative evidence.

use std::{fmt::Debug, marker::PhantomData, mem::size_of, ops};

pub const FIXED_DELTA_TIME_MS: u64 = 16;
pub const STEP_COUNT: usize = 60;
pub const VEHICLE_COUNT: usize = 10_000;
pub const SCALING_VEHICLE_COUNT: usize = 100_000;
pub const LOCALITY_EDGE_LENGTH: f64 = 10_000.0;

const MILLISECONDS_PER_SECOND: f64 = 1_000.0;
const VEHICLE_LENGTH: f64 = 4.5;
const DESIRED_SPEED: f64 = 13.9;
const MIN_GAP: f64 = 2.0;
const TIME_HEADWAY: f64 = 1.5;
const MAX_ACCELERATION: f64 = 1.4;
const COMFORTABLE_DECELERATION: f64 = 2.0;
const EMERGENCY_DECELERATION: f64 = 4.0;
const MECHANICAL_EPSILON: f64 = 1.0e-9;

pub trait ResearchFloat:
    Copy
    + Debug
    + Default
    + PartialEq
    + PartialOrd
    + ops::Add<Output = Self>
    + ops::Sub<Output = Self>
    + ops::Mul<Output = Self>
    + ops::Div<Output = Self>
    + ops::Neg<Output = Self>
    + 'static
{
    fn from_f64(value: f64) -> Self;
    fn to_f64(self) -> f64;
    fn abs(self) -> Self;
    fn sqrt(self) -> Self;
    fn powi(self, exponent: i32) -> Self;
    fn powf(self, exponent: Self) -> Self;
    fn hypot(self, other: Self) -> Self;
    fn next_down(self) -> Self;
    fn is_finite(self) -> bool;
    fn max_value() -> Self;
}

macro_rules! impl_research_float {
    ($type:ty) => {
        impl ResearchFloat for $type {
            fn from_f64(value: f64) -> Self {
                value as Self
            }

            fn to_f64(self) -> f64 {
                self as f64
            }

            fn abs(self) -> Self {
                self.abs()
            }

            fn sqrt(self) -> Self {
                self.sqrt()
            }

            fn powi(self, exponent: i32) -> Self {
                self.powi(exponent)
            }

            fn powf(self, exponent: Self) -> Self {
                self.powf(exponent)
            }

            fn hypot(self, other: Self) -> Self {
                self.hypot(other)
            }

            fn next_down(self) -> Self {
                self.next_down()
            }

            fn is_finite(self) -> bool {
                self.is_finite()
            }

            fn max_value() -> Self {
                Self::MAX
            }
        }
    };
}

impl_research_float!(f32);
impl_research_float!(f64);

pub trait PrecisionMode: Copy + Debug + 'static {
    type Compute: ResearchFloat;
    type Progress: ResearchFloat;
    type Residual: Copy + Debug + Default + PartialEq;

    const NAME: &'static str;
    const RESIDUAL_AWARE: bool = false;

    fn compute_from_f64(value: f64) -> Self::Compute {
        Self::Compute::from_f64(value)
    }

    fn progress_from_f64(value: f64) -> Self::Progress {
        Self::Progress::from_f64(value)
    }

    fn compute_from_progress(value: Self::Progress) -> Self::Compute {
        Self::Compute::from_f64(value.to_f64())
    }

    fn progress_from_compute(value: Self::Compute) -> Self::Progress {
        Self::Progress::from_f64(value.to_f64())
    }

    fn progress_to_f64(progress: Self::Progress, _residual: Self::Residual) -> f64 {
        progress.to_f64()
    }

    fn compute_progress_value(progress: Self::Progress, residual: Self::Residual) -> Self::Compute {
        let _ = residual;
        Self::compute_from_progress(progress)
    }

    fn compute_progress_difference(
        leader_progress: Self::Progress,
        leader_residual: Self::Residual,
        follower_progress: Self::Progress,
        follower_residual: Self::Residual,
    ) -> Self::Compute {
        let _ = (leader_residual, follower_residual);
        Self::compute_from_progress(leader_progress - follower_progress)
    }

    fn compute_progress_remaining(
        edge_length: Self::Progress,
        progress: Self::Progress,
        residual: Self::Residual,
    ) -> Self::Compute {
        let _ = residual;
        Self::compute_from_progress(edge_length - progress)
    }

    fn progress_reaches_boundary(
        progress: Self::Progress,
        residual: Self::Residual,
        edge_length: Self::Progress,
        epsilon: Self::Progress,
    ) -> bool {
        let _ = residual;
        progress + epsilon >= edge_length
    }

    fn progress_is_near_zero(
        progress: Self::Progress,
        residual: Self::Residual,
        epsilon: Self::Progress,
    ) -> bool {
        let _ = residual;
        progress.abs() < epsilon
    }

    fn add_progress(
        progress: &mut Self::Progress,
        residual: &mut Self::Residual,
        delta: Self::Progress,
    );

    fn clear_residual(residual: &mut Self::Residual) {
        *residual = Self::Residual::default();
    }
}

#[derive(Clone, Copy, Debug)]
pub struct F64Mode;

impl PrecisionMode for F64Mode {
    type Compute = f64;
    type Progress = f64;
    type Residual = ();

    const NAME: &'static str = "f64";

    fn add_progress(progress: &mut f64, _residual: &mut (), delta: f64) {
        *progress += delta;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RawF32Mode;

impl PrecisionMode for RawF32Mode {
    type Compute = f32;
    type Progress = f32;
    type Residual = ();

    const NAME: &'static str = "raw_f32";

    fn add_progress(progress: &mut f32, _residual: &mut (), delta: f32) {
        *progress += delta;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CompensatedF32Mode;

impl PrecisionMode for CompensatedF32Mode {
    type Compute = f32;
    type Progress = f32;
    type Residual = f32;

    const NAME: &'static str = "compensated_f32";

    fn add_progress(progress: &mut f32, residual: &mut f32, delta: f32) {
        let corrected_delta = delta - *residual;
        let next = *progress + corrected_delta;
        *residual = (next - *progress) - corrected_delta;
        *progress = next;
    }
}

macro_rules! residual_aware_progress_methods {
    () => {
        fn progress_to_f64(progress: f32, residual: f32) -> f64 {
            f64::from(progress) - f64::from(residual)
        }

        fn compute_progress_value(progress: f32, residual: f32) -> Self::Compute {
            Self::Compute::from_f64(Self::progress_to_f64(progress, residual))
        }

        fn compute_progress_difference(
            leader_progress: f32,
            leader_residual: f32,
            follower_progress: f32,
            follower_residual: f32,
        ) -> Self::Compute {
            Self::Compute::from_f64(
                (f64::from(leader_progress) - f64::from(follower_progress))
                    - (f64::from(leader_residual) - f64::from(follower_residual)),
            )
        }

        fn compute_progress_remaining(
            edge_length: f32,
            progress: f32,
            residual: f32,
        ) -> Self::Compute {
            Self::Compute::from_f64(
                (f64::from(edge_length) - f64::from(progress)) + f64::from(residual),
            )
        }

        fn progress_reaches_boundary(
            progress: f32,
            residual: f32,
            edge_length: f32,
            epsilon: f32,
        ) -> bool {
            Self::progress_to_f64(progress, residual) + f64::from(epsilon) >= f64::from(edge_length)
        }

        fn progress_is_near_zero(progress: f32, residual: f32, epsilon: f32) -> bool {
            Self::progress_to_f64(progress, residual).abs() < f64::from(epsilon)
        }
    };
}

#[derive(Clone, Copy, Debug)]
pub struct ResidualAwareF32Mode;

impl PrecisionMode for ResidualAwareF32Mode {
    type Compute = f32;
    type Progress = f32;
    type Residual = f32;

    const NAME: &'static str = "residual_aware_f32";
    const RESIDUAL_AWARE: bool = true;

    residual_aware_progress_methods!();

    fn add_progress(progress: &mut f32, residual: &mut f32, delta: f32) {
        let corrected_delta = delta - *residual;
        let next = *progress + corrected_delta;
        *residual = (next - *progress) - corrected_delta;
        *progress = next;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SensitiveControlMixedMode;

impl PrecisionMode for SensitiveControlMixedMode {
    type Compute = f64;
    type Progress = f32;
    type Residual = f32;

    const NAME: &'static str = "sensitive_f64_control_f32_progress";
    const RESIDUAL_AWARE: bool = true;

    residual_aware_progress_methods!();

    fn add_progress(progress: &mut f32, residual: &mut f32, delta: f32) {
        let corrected_delta = delta - *residual;
        let next = *progress + corrected_delta;
        *residual = (next - *progress) - corrected_delta;
        *progress = next;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MixedF32Mode;

impl PrecisionMode for MixedF32Mode {
    type Compute = f32;
    type Progress = f64;
    type Residual = ();

    const NAME: &'static str = "mixed_f32_compute_f64_progress";

    fn add_progress(progress: &mut f64, _residual: &mut (), delta: f64) {
        *progress += delta;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateScenario {
    FreeFlow,
    DensePlatoon,
    StopAndGo,
}

impl CandidateScenario {
    pub const fn benchmark_name(self) -> &'static str {
        match self {
            Self::FreeFlow => "free_flow",
            Self::DensePlatoon => "dense_platoon",
            Self::StopAndGo => "stop_and_go",
        }
    }

    const fn spacing(self) -> f64 {
        match self {
            Self::FreeFlow => 250.0,
            Self::DensePlatoon | Self::StopAndGo => 6.5,
        }
    }

    const fn initial_speed(self) -> f64 {
        match self {
            Self::FreeFlow => 10.0,
            Self::DensePlatoon => 1.0,
            Self::StopAndGo => 8.0,
        }
    }

    const fn stopped_front(self) -> bool {
        matches!(self, Self::StopAndGo)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateLayout {
    LegacySingleEdge,
    LocalityPreserving,
    EdgeCap4Km,
    EdgeCap1Km,
    EdgeCap100M,
}

impl CandidateLayout {
    pub const EDGE_CAP_MATRIX: [Self; 5] = [
        Self::LegacySingleEdge,
        Self::LocalityPreserving,
        Self::EdgeCap4Km,
        Self::EdgeCap1Km,
        Self::EdgeCap100M,
    ];

    pub const fn benchmark_name(self) -> &'static str {
        match self {
            Self::LegacySingleEdge => "legacy",
            Self::LocalityPreserving => "cap_10km",
            Self::EdgeCap4Km => "cap_4km",
            Self::EdgeCap1Km => "cap_1km",
            Self::EdgeCap100M => "cap_100m",
        }
    }

    pub const fn edge_cap(self) -> Option<f64> {
        match self {
            Self::LegacySingleEdge => None,
            Self::LocalityPreserving => Some(LOCALITY_EDGE_LENGTH),
            Self::EdgeCap4Km => Some(4_000.0),
            Self::EdgeCap1Km => Some(1_000.0),
            Self::EdgeCap100M => Some(100.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateStatus {
    Active,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CandidateProfile<T: ResearchFloat> {
    length: T,
    desired_speed: T,
    min_gap: T,
    time_headway: T,
    max_acceleration: T,
    comfortable_deceleration: T,
    emergency_deceleration: T,
}

impl<T: ResearchFloat> CandidateProfile<T> {
    fn standard() -> Self {
        Self {
            length: T::from_f64(VEHICLE_LENGTH),
            desired_speed: T::from_f64(DESIRED_SPEED),
            min_gap: T::from_f64(MIN_GAP),
            time_headway: T::from_f64(TIME_HEADWAY),
            max_acceleration: T::from_f64(MAX_ACCELERATION),
            comfortable_deceleration: T::from_f64(COMFORTABLE_DECELERATION),
            emergency_deceleration: T::from_f64(EMERGENCY_DECELERATION),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CandidateVehicle<M: PrecisionMode> {
    route_edge_index: usize,
    edge_progress: M::Progress,
    progress_residual: M::Residual,
    current_speed: M::Compute,
    applied_acceleration: M::Compute,
    status: CandidateStatus,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CandidateMotion<T: ResearchFloat> {
    leader: Option<usize>,
    bumper_gap: Option<T>,
    current_speed: T,
    candidate_speed: T,
    candidate_travel: T,
    emergency_min_travel: T,
    final_speed: T,
    final_travel: T,
    safety_projection: bool,
}

impl<T: ResearchFloat> CandidateMotion<T> {
    fn stationary() -> Self {
        Self {
            leader: None,
            bumper_gap: None,
            current_speed: T::default(),
            candidate_speed: T::default(),
            candidate_travel: T::default(),
            emergency_min_travel: T::default(),
            final_speed: T::default(),
            final_travel: T::default(),
            safety_projection: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateStepSummary {
    pub safety_projections: Vec<(usize, usize)>,
    pub edge_changes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CandidateSnapshot {
    pub route_edge_index: usize,
    pub edge_progress: f64,
    pub route_progress: f64,
    pub current_speed: f64,
    pub applied_acceleration: f64,
    pub status: CandidateStatus,
    pub leader: Option<usize>,
    pub bumper_gap: Option<f64>,
    pub candidate_speed: f64,
    pub candidate_travel: f64,
    pub emergency_min_travel: f64,
    pub final_speed: f64,
    pub final_travel: f64,
    pub safety_projection: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CandidateMemoryStats {
    pub vehicle_size: usize,
    pub motion_size: usize,
    pub retained_bytes: usize,
    pub edge_count: usize,
    pub route_occurrence_count: usize,
    pub topology_scalar_floor_bytes: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CandidateWorld<M: PrecisionMode> {
    vehicles: Vec<CandidateVehicle<M>>,
    motions: Vec<CandidateMotion<M::Compute>>,
    profile: CandidateProfile<M::Compute>,
    layout: CandidateLayout,
    route_length: f64,
    nominal_edge_length: M::Progress,
    edge_count: usize,
    delta_time: M::Compute,
    _mode: PhantomData<M>,
}

impl<M: PrecisionMode> CandidateWorld<M> {
    pub fn new(vehicle_count: usize, scenario: CandidateScenario, layout: CandidateLayout) -> Self {
        assert!(vehicle_count > 0);
        let spacing = scenario.spacing();
        let route_length = spacing * vehicle_count as f64 + 1_000.0;
        let nominal_edge_length = layout.edge_cap().unwrap_or(route_length);
        let edge_count = (route_length / nominal_edge_length).ceil() as usize;
        let vehicles = (0..vehicle_count)
            .map(|index| {
                let route_progress = spacing * index as f64;
                let route_edge_index = (route_progress / nominal_edge_length).floor() as usize;
                let edge_progress = route_progress - nominal_edge_length * route_edge_index as f64;
                let stopped = scenario.stopped_front() && index + 1 == vehicle_count;
                CandidateVehicle {
                    route_edge_index,
                    edge_progress: M::progress_from_f64(edge_progress),
                    progress_residual: M::Residual::default(),
                    current_speed: M::compute_from_f64(if stopped {
                        0.0
                    } else {
                        scenario.initial_speed()
                    }),
                    applied_acceleration: M::Compute::default(),
                    status: if stopped {
                        CandidateStatus::Stopped
                    } else {
                        CandidateStatus::Active
                    },
                }
            })
            .collect::<Vec<_>>();
        let motions = vec![CandidateMotion::stationary(); vehicle_count];
        Self {
            vehicles,
            motions,
            profile: CandidateProfile::standard(),
            layout,
            route_length,
            nominal_edge_length: M::progress_from_f64(nominal_edge_length),
            edge_count,
            delta_time: M::compute_from_f64(FIXED_DELTA_TIME_MS as f64 / MILLISECONDS_PER_SECOND),
            _mode: PhantomData,
        }
    }

    pub fn step(&mut self) -> CandidateStepSummary {
        for index in 0..self.vehicles.len() {
            let vehicle = self.vehicles[index];
            self.motions[index] = if vehicle.status == CandidateStatus::Stopped {
                CandidateMotion::stationary()
            } else {
                let leader = self.candidate_leader(index);
                compute_motion(
                    vehicle.current_speed,
                    self.profile,
                    leader.map(|(leader_index, bumper_gap)| {
                        (
                            leader_index,
                            self.vehicles[leader_index].current_speed,
                            bumper_gap,
                        )
                    }),
                    self.delta_time,
                )
            };
        }

        for index in (0..self.vehicles.len()).rev() {
            let Some(leader) = self.motions[index].leader else {
                continue;
            };
            let leader_final_travel = self.motions[leader].final_travel;
            apply_geometry_cap(
                &mut self.motions[index],
                leader_final_travel,
                self.delta_time,
            );
        }

        let mut summary = CandidateStepSummary {
            safety_projections: Vec::new(),
            edge_changes: 0,
        };
        for index in 0..self.vehicles.len() {
            if self.vehicles[index].status != CandidateStatus::Active {
                continue;
            }
            let motion = self.motions[index];
            if motion.safety_projection {
                summary.safety_projections.push((
                    index,
                    motion.leader.expect("safety projection must have a leader"),
                ));
            }
            self.vehicles[index].current_speed = motion.final_speed;
            self.vehicles[index].applied_acceleration =
                (motion.final_speed - motion.current_speed) / self.delta_time;
            summary.edge_changes += self.advance(index, motion.final_travel);
        }
        summary
    }

    pub fn run_steps(&mut self, steps: usize) -> usize {
        let mut observable_events = 0;
        for _ in 0..steps {
            let summary = self.step();
            observable_events += summary.safety_projections.len() + summary.edge_changes;
        }
        observable_events
    }

    pub fn snapshot(&self, index: usize) -> CandidateSnapshot {
        let vehicle = self.vehicles[index];
        let motion = self.motions[index];
        let edge_progress = M::progress_to_f64(vehicle.edge_progress, vehicle.progress_residual);
        let route_progress =
            vehicle.route_edge_index as f64 * self.nominal_edge_length.to_f64() + edge_progress;
        CandidateSnapshot {
            route_edge_index: vehicle.route_edge_index,
            edge_progress,
            route_progress,
            current_speed: vehicle.current_speed.to_f64(),
            applied_acceleration: vehicle.applied_acceleration.to_f64(),
            status: vehicle.status,
            leader: motion.leader,
            bumper_gap: motion.bumper_gap.map(ResearchFloat::to_f64),
            candidate_speed: motion.candidate_speed.to_f64(),
            candidate_travel: motion.candidate_travel.to_f64(),
            emergency_min_travel: motion.emergency_min_travel.to_f64(),
            final_speed: motion.final_speed.to_f64(),
            final_travel: motion.final_travel.to_f64(),
            safety_projection: motion.safety_projection,
        }
    }

    pub fn len(&self) -> usize {
        self.vehicles.len()
    }

    pub fn memory_stats(&self) -> CandidateMemoryStats {
        CandidateMemoryStats {
            vehicle_size: size_of::<CandidateVehicle<M>>(),
            motion_size: size_of::<CandidateMotion<M::Compute>>(),
            retained_bytes: size_of::<Self>()
                + self.vehicles.capacity() * size_of::<CandidateVehicle<M>>()
                + self.motions.capacity() * size_of::<CandidateMotion<M::Compute>>(),
            edge_count: self.edge_count,
            route_occurrence_count: self.edge_count,
            topology_scalar_floor_bytes: self.edge_count * (size_of::<f64>() + size_of::<usize>()),
        }
    }

    pub const fn mode_name() -> &'static str {
        M::NAME
    }

    fn candidate_leader(&self, follower_index: usize) -> Option<(usize, M::Compute)> {
        let leader_index = follower_index.checked_add(1)?;
        let leader = self.vehicles.get(leader_index)?;
        let follower = self.vehicles[follower_index];
        let front_distance = self.front_distance(follower, *leader);
        let bumper_gap = front_distance - self.profile.length;
        (bumper_gap <= self.leader_horizon(follower.current_speed))
            .then_some((leader_index, normalize_gap(bumper_gap)))
    }

    fn front_distance(
        &self,
        follower: CandidateVehicle<M>,
        leader: CandidateVehicle<M>,
    ) -> M::Compute {
        if follower.route_edge_index == leader.route_edge_index {
            return M::compute_progress_difference(
                leader.edge_progress,
                leader.progress_residual,
                follower.edge_progress,
                follower.progress_residual,
            );
        }

        if !M::RESIDUAL_AWARE {
            let mut distance = self.edge_length(follower.route_edge_index) - follower.edge_progress;
            for edge_index in follower.route_edge_index + 1..leader.route_edge_index {
                distance = distance + self.edge_length(edge_index);
            }
            distance = distance + leader.edge_progress;
            return M::compute_from_progress(distance);
        }

        let mut distance = M::compute_progress_remaining(
            self.edge_length(follower.route_edge_index),
            follower.edge_progress,
            follower.progress_residual,
        );
        for edge_index in follower.route_edge_index + 1..leader.route_edge_index {
            distance = distance + M::compute_from_progress(self.edge_length(edge_index));
        }
        distance + M::compute_progress_value(leader.edge_progress, leader.progress_residual)
    }

    fn leader_horizon(&self, speed: M::Compute) -> M::Compute {
        let upper_speed = speed + self.profile.max_acceleration * self.delta_time;
        let travel_upper =
            half_product(speed, self.delta_time) + half_product(upper_speed, self.delta_time);
        let braking_distance = braking_distance(upper_speed, self.profile.emergency_deceleration);
        let hard_horizon = travel_upper + braking_distance;
        let comfort_horizon = self.profile.min_gap + speed * self.profile.time_headway;
        maximum(hard_horizon, comfort_horizon)
    }

    fn advance(&mut self, index: usize, travel: M::Compute) -> usize {
        let epsilon = M::Progress::from_f64(MECHANICAL_EPSILON);
        let travel = M::progress_from_compute(travel);
        if travel <= epsilon {
            return 0;
        }

        let vehicle = &mut self.vehicles[index];
        M::add_progress(
            &mut vehicle.edge_progress,
            &mut vehicle.progress_residual,
            travel,
        );
        let mut edge_changes = 0;
        loop {
            let edge_length = if vehicle.route_edge_index + 1 == self.edge_count {
                M::progress_from_f64(
                    self.route_length
                        - self.nominal_edge_length.to_f64() * (self.edge_count - 1) as f64,
                )
            } else {
                self.nominal_edge_length
            };
            if !M::progress_reaches_boundary(
                vehicle.edge_progress,
                vehicle.progress_residual,
                edge_length,
                epsilon,
            ) {
                break;
            }
            M::add_progress(
                &mut vehicle.edge_progress,
                &mut vehicle.progress_residual,
                -edge_length,
            );
            if M::progress_is_near_zero(vehicle.edge_progress, vehicle.progress_residual, epsilon) {
                vehicle.edge_progress = M::Progress::default();
                M::clear_residual(&mut vehicle.progress_residual);
            }
            vehicle.route_edge_index += 1;
            edge_changes += 1;
            assert!(
                vehicle.route_edge_index < self.edge_count,
                "fixture route is long enough"
            );
        }
        edge_changes
    }

    fn edge_length(&self, edge_index: usize) -> M::Progress {
        if edge_index + 1 == self.edge_count {
            M::progress_from_f64(
                self.route_length
                    - self.nominal_edge_length.to_f64() * (self.edge_count - 1) as f64,
            )
        } else {
            self.nominal_edge_length
        }
    }
}

fn compute_motion<T: ResearchFloat>(
    current_speed: T,
    profile: CandidateProfile<T>,
    leader: Option<(usize, T, T)>,
    delta_time: T,
) -> CandidateMotion<T> {
    let comfort_acceleration = iidm_acceleration(current_speed, profile, leader);
    let comfort = ballistic_motion(current_speed, comfort_acceleration, delta_time);
    let emergency_min_travel =
        emergency_min_travel(current_speed, profile.emergency_deceleration, delta_time);
    let candidate = if let Some((_, leader_speed, bumper_gap)) = leader {
        let safe_speed = safe_speed(
            current_speed,
            profile.emergency_deceleration,
            leader_speed,
            profile.emergency_deceleration,
            bumper_gap,
            delta_time,
        );
        let emergency_floor = maximum(
            current_speed - profile.emergency_deceleration * delta_time,
            T::default(),
        );
        let candidate_speed = maximum(minimum(comfort.0, safe_speed), emergency_floor);
        if candidate_speed == comfort.0 {
            comfort
        } else {
            ballistic_motion(
                current_speed,
                (candidate_speed - current_speed) / delta_time,
                delta_time,
            )
        }
    } else {
        comfort
    };

    CandidateMotion {
        leader: leader.map(|value| value.0),
        bumper_gap: leader.map(|value| value.2),
        current_speed,
        candidate_speed: candidate.0,
        candidate_travel: candidate.1,
        emergency_min_travel,
        final_speed: candidate.0,
        final_travel: candidate.1,
        safety_projection: false,
    }
}

fn apply_geometry_cap<T: ResearchFloat>(
    motion: &mut CandidateMotion<T>,
    leader_final_travel: T,
    delta_time: T,
) {
    let epsilon = T::from_f64(MECHANICAL_EPSILON);
    let geometry_cap = maximum(
        motion
            .bumper_gap
            .expect("leader motion must retain its gap")
            + leader_final_travel,
        T::default(),
    );
    let travel_before_projection = motion.candidate_travel;
    let final_travel = minimum(travel_before_projection, geometry_cap);
    motion.final_speed = if final_travel < travel_before_projection {
        speed_after_travel_cap(
            motion.candidate_speed,
            motion.candidate_travel,
            final_travel,
            delta_time,
        )
    } else {
        motion.candidate_speed
    };
    motion.final_travel = final_travel;
    motion.safety_projection = final_travel + epsilon < travel_before_projection
        && final_travel + epsilon < motion.emergency_min_travel;
}

fn iidm_acceleration<T: ResearchFloat>(
    current_speed: T,
    profile: CandidateProfile<T>,
    leader: Option<(usize, T, T)>,
) -> T {
    let one = T::from_f64(1.0);
    let four = T::from_f64(4.0);
    let free_acceleration = if current_speed <= profile.desired_speed {
        let speed_term = (current_speed / profile.desired_speed).powi(4);
        profile.max_acceleration * (one - speed_term)
    } else {
        let exponent = profile.max_acceleration * four / profile.comfortable_deceleration;
        let speed_term = (profile.desired_speed / current_speed).powf(exponent);
        -profile.comfortable_deceleration * (one - speed_term)
    };
    let Some((_, leader_speed, bumper_gap)) = leader else {
        return clamp(
            free_acceleration,
            -profile.comfortable_deceleration,
            profile.max_acceleration,
        );
    };
    if bumper_gap <= T::from_f64(MECHANICAL_EPSILON) {
        return -profile.comfortable_deceleration;
    }

    let closing_speed = current_speed - leader_speed;
    let sqrt_acceleration_product =
        profile.max_acceleration.sqrt() * profile.comfortable_deceleration.sqrt();
    let dynamic_gap = half_product(current_speed, closing_speed) / sqrt_acceleration_product;
    let desired_gap = profile.min_gap
        + maximum(
            current_speed * profile.time_headway + dynamic_gap,
            T::default(),
        );
    let gap_ratio = desired_gap / bumper_gap;
    let acceleration = if gap_ratio >= one {
        profile.max_acceleration * (one - gap_ratio * gap_ratio)
    } else if free_acceleration > T::default() {
        let exponent = T::from_f64(2.0) * profile.max_acceleration / free_acceleration;
        free_acceleration * (one - gap_ratio.powf(exponent))
    } else {
        free_acceleration
    };
    clamp(
        acceleration,
        -profile.comfortable_deceleration,
        profile.max_acceleration,
    )
}

fn safe_speed<T: ResearchFloat>(
    follower_speed: T,
    follower_deceleration: T,
    leader_speed: T,
    leader_deceleration: T,
    bumper_gap: T,
    delta_time: T,
) -> T {
    let gap_term = multiply_factors([
        T::from_f64(2.0),
        follower_deceleration,
        maximum(bumper_gap, T::default()),
    ]);
    let leader_term = follower_deceleration / leader_deceleration * leader_speed * leader_speed;
    let rhs = gap_term + leader_term;
    let follower_step_term = multiply_factors([follower_deceleration, follower_speed, delta_time]);
    let c = follower_step_term - rhs;
    if c >= T::default() {
        return T::default();
    }
    let b = follower_deceleration * delta_time;
    let sqrt_discriminant = b.hypot(T::from_f64(2.0) * (-c).sqrt());
    let denominator = T::from_f64(0.5) * b + T::from_f64(0.5) * sqrt_discriminant;
    let root = maximum((-c) / denominator, T::default());
    if root > T::default() {
        root.next_down()
    } else {
        T::default()
    }
}

fn ballistic_motion<T: ResearchFloat>(current_speed: T, acceleration: T, delta_time: T) -> (T, T) {
    if acceleration < T::default() {
        let stop_time = current_speed / -acceleration;
        if stop_time < delta_time {
            return (T::default(), braking_distance(current_speed, -acceleration));
        }
    }
    let speed = maximum(current_speed + acceleration * delta_time, T::default());
    let travel = half_product(current_speed, delta_time) + half_product(speed, delta_time);
    (speed, travel)
}

fn emergency_min_travel<T: ResearchFloat>(
    current_speed: T,
    emergency_deceleration: T,
    delta_time: T,
) -> T {
    let speed_step = emergency_deceleration * delta_time;
    if current_speed <= speed_step {
        braking_distance(current_speed, emergency_deceleration)
    } else {
        (current_speed - T::from_f64(0.5) * speed_step) * delta_time
    }
}

fn braking_distance<T: ResearchFloat>(speed: T, deceleration: T) -> T {
    if speed == T::default() {
        return T::default();
    }
    if deceleration > T::max_value() / T::from_f64(2.0) {
        return speed / deceleration * (T::from_f64(0.5) * speed);
    }
    let denominator = T::from_f64(2.0) * deceleration;
    if speed < T::from_f64(1.0) {
        speed / (denominator / speed)
    } else {
        speed / denominator * speed
    }
}

fn speed_after_travel_cap<T: ResearchFloat>(
    candidate_speed: T,
    candidate_travel: T,
    final_travel: T,
    delta_time: T,
) -> T {
    if candidate_speed == T::default() {
        return T::default();
    }
    let speed_reduction = (candidate_travel - final_travel) / delta_time * T::from_f64(2.0);
    minimum(
        maximum(candidate_speed - speed_reduction, T::default()),
        candidate_speed,
    )
}

fn normalize_gap<T: ResearchFloat>(value: T) -> T {
    if value.abs() <= T::from_f64(MECHANICAL_EPSILON) {
        T::default()
    } else {
        value
    }
}

fn half_product<T: ResearchFloat>(left: T, right: T) -> T {
    if left.abs() >= right.abs() {
        T::from_f64(0.5) * left * right
    } else {
        left * (T::from_f64(0.5) * right)
    }
}

fn multiply_factors<T: ResearchFloat, const N: usize>(mut factors: [T; N]) -> T {
    factors.sort_unstable_by(|left, right| {
        left.abs()
            .partial_cmp(&right.abs())
            .expect("candidate factors must be finite")
    });
    factors
        .into_iter()
        .fold(T::from_f64(1.0), |product, factor| product * factor)
}

fn minimum<T: ResearchFloat>(left: T, right: T) -> T {
    if left <= right { left } else { right }
}

fn maximum<T: ResearchFloat>(left: T, right: T) -> T {
    if left >= right { left } else { right }
}

fn clamp<T: ResearchFloat>(value: T, minimum: T, maximum: T) -> T {
    maximum_value(minimum_value(value, maximum), minimum)
}

fn minimum_value<T: ResearchFloat>(left: T, right: T) -> T {
    minimum(left, right)
}

fn maximum_value<T: ResearchFloat>(left: T, right: T) -> T {
    maximum(left, right)
}

pub fn constant_addition<M: PrecisionMode>(
    initial_progress: f64,
    travel_per_tick: f64,
    ticks: usize,
) -> f64 {
    let mut progress = M::progress_from_f64(initial_progress);
    let mut residual = M::Residual::default();
    let travel = M::progress_from_compute(M::compute_from_f64(travel_per_tick));
    for _ in 0..ticks {
        M::add_progress(&mut progress, &mut residual, travel);
    }
    M::progress_to_f64(progress, residual)
}

pub fn finite_candidate_value<T: ResearchFloat>(value: f64) -> Option<T> {
    let candidate = T::from_f64(value);
    candidate
        .is_finite()
        .then_some(if candidate == T::default() {
            T::default()
        } else {
            candidate
        })
}
