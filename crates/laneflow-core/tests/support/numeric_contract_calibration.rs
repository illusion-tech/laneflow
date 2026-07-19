//! #127 研究专用的分层数值契约标定器。
//!
//! 转换判定基准（conversion oracle）把原始 `f64` 校验与目标 `f32` 规范化分开；
//! 运行时判定基准（runtime oracle）再让高精度参照和补偿残差感知 `f32` 候选从
//! 同一份规范化目标权威值出发。本模块不会修改任何生产判定函数。

use std::fmt;

pub const MAX_EDGE_LENGTH_METERS: f64 = 10_000.0;
pub const MAX_EXTENT_OR_OFFSET_METERS: f64 = 128.0;
pub const MIN_VEHICLE_LENGTH_METERS: f64 = 0.1;
pub const MIN_PARKING_EXTENT_METERS: f64 = 0.1;
pub const EDGE_MINIMUM_CANDIDATES_METERS: [f64; 4] = [0.0, 0.01, 0.1, 1.0];
pub const CALIBRATION_SEED: u64 = 0x1270_0140_0141_0144;
const RANDOM_SAMPLE_COUNT: usize = 100_000;

// 以下值只是 #127 研究候选；只有 #144 可以投入生产。
pub const EDGE_BOUNDARY_TOLERANCE_CANDIDATE_METERS: f64 = 0.000_000_01;
pub const LONGITUDINAL_TOLERANCE_CANDIDATE_METERS: f64 = 0.000_05;
pub const PHYSICAL_GAP_TOLERANCE_CANDIDATE_METERS: f64 = 0.000_01;
pub const COMPUTED_SPEED_TOLERANCE_CANDIDATE_METERS_PER_SECOND: f64 = 0.000_05;
pub const PARKING_ANCHOR_CLEARANCE_CANDIDATE_METERS: f64 =
    if EDGE_BOUNDARY_TOLERANCE_CANDIDATE_METERS > LONGITUDINAL_TOLERANCE_CANDIDATE_METERS {
        EDGE_BOUNDARY_TOLERANCE_CANDIDATE_METERS
    } else {
        LONGITUDINAL_TOLERANCE_CANDIDATE_METERS
    };

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConversionDomain {
    EdgeLength { min_exclusive_meters: f64 },
    VehicleLength,
    ParkingExtent,
    ParkingLateralOffset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversionFailure {
    RawNonFinite,
    RawOutOfRange,
    TargetNonFinite,
    TargetOutOfRange,
    TargetZero,
}

pub fn checked_f32(raw: f64) -> Result<f32, ConversionFailure> {
    let converted = raw as f32;
    if !converted.is_finite() {
        return Err(ConversionFailure::TargetNonFinite);
    }
    Ok(if converted == 0.0 { 0.0 } else { converted })
}

/// 镜像 #127 冻结的转换顺序，但不创建第二套生产类型。
pub fn convert_raw_f64(domain: ConversionDomain, raw: f64) -> Result<f32, ConversionFailure> {
    if !raw.is_finite() {
        return Err(ConversionFailure::RawNonFinite);
    }
    let raw_in_range = match domain {
        ConversionDomain::EdgeLength {
            min_exclusive_meters,
        } => raw > min_exclusive_meters && raw <= MAX_EDGE_LENGTH_METERS,
        ConversionDomain::VehicleLength => {
            (MIN_VEHICLE_LENGTH_METERS..=MAX_EXTENT_OR_OFFSET_METERS).contains(&raw)
        }
        ConversionDomain::ParkingExtent => {
            (MIN_PARKING_EXTENT_METERS..=MAX_EXTENT_OR_OFFSET_METERS).contains(&raw)
        }
        ConversionDomain::ParkingLateralOffset => raw.abs() <= MAX_EXTENT_OR_OFFSET_METERS,
    };
    if !raw_in_range {
        return Err(ConversionFailure::RawOutOfRange);
    }

    let normalized = checked_f32(raw)?;
    let target_in_range = match domain {
        ConversionDomain::EdgeLength {
            min_exclusive_meters,
        } => {
            f64::from(normalized) > min_exclusive_meters
                && f64::from(normalized) <= MAX_EDGE_LENGTH_METERS
        }
        ConversionDomain::VehicleLength => (MIN_VEHICLE_LENGTH_METERS
            ..=MAX_EXTENT_OR_OFFSET_METERS)
            .contains(&f64::from(normalized)),
        ConversionDomain::ParkingExtent => (MIN_PARKING_EXTENT_METERS
            ..=MAX_EXTENT_OR_OFFSET_METERS)
            .contains(&f64::from(normalized)),
        ConversionDomain::ParkingLateralOffset => {
            f64::from(normalized).abs() <= MAX_EXTENT_OR_OFFSET_METERS
        }
    };
    if !target_in_range {
        return Err(ConversionFailure::TargetOutOfRange);
    }
    if matches!(domain, ConversionDomain::ParkingLateralOffset) && normalized == 0.0 {
        return Err(ConversionFailure::TargetZero);
    }
    Ok(normalized)
}

pub fn append_converted_batch_atomic(
    authority: &mut Vec<f32>,
    domain: ConversionDomain,
    raw_values: &[f64],
) -> Result<(), (usize, ConversionFailure)> {
    let staged = raw_values
        .iter()
        .copied()
        .enumerate()
        .map(|(index, raw)| convert_raw_f64(domain, raw).map_err(|failure| (index, failure)))
        .collect::<Result<Vec<_>, _>>()?;
    authority.extend(staged);
    Ok(())
}

pub fn parking_anchor_is_strictly_inside(
    edge_length_meters: f32,
    progress_meters: f64,
    clearance_meters: f64,
) -> bool {
    progress_meters > clearance_meters
        && progress_meters < f64::from(edge_length_meters) - clearance_meters
}

#[derive(Clone, Copy, Debug)]
pub struct ResidualProgress {
    high: f32,
    residual: f32,
}

impl ResidualProgress {
    pub fn from_normalized(value: f64) -> Self {
        let high = value as f32;
        let residual = (f64::from(high) - value) as f32;
        Self { high, residual }
    }

    pub fn add_f32(&mut self, delta: f32) {
        let corrected_delta = delta - self.residual;
        let next = self.high + corrected_delta;
        self.residual = (next - self.high) - corrected_delta;
        self.high = next;
    }

    pub fn rebase_after_edge(&mut self, edge_length: f32) {
        *self = Self::from_normalized(self.effective() - f64::from(edge_length));
    }

    pub fn effective(self) -> f64 {
        f64::from(self.high) - f64::from(self.residual)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ErrorObservation {
    pub case: &'static str,
    pub input_a: f64,
    pub input_b: f64,
    pub reference: f64,
    pub candidate: f64,
    pub absolute_error: f64,
    pub local_f32_ulp: f64,
}

impl fmt::Display for ErrorObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "case={} input_a={:.12} input_b={:.12} reference={:.12} candidate={:.12} absolute_error={:.12} local_f32_ulp={:.12}",
            self.case,
            self.input_a,
            self.input_b,
            self.reference,
            self.candidate,
            self.absolute_error,
            self.local_f32_ulp,
        )
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ErrorStats {
    pub samples: usize,
    pub max_absolute_error: f64,
    pub max_error_in_local_ulps: f64,
    pub worst: Option<ErrorObservation>,
}

impl ErrorStats {
    fn observe(
        &mut self,
        case: &'static str,
        input_a: f64,
        input_b: f64,
        reference: f64,
        candidate: f64,
    ) {
        let absolute_error = (reference - candidate).abs();
        let local_f32_ulp = f32_ulp(reference as f32);
        let error_in_ulps = if local_f32_ulp == 0.0 {
            0.0
        } else {
            absolute_error / local_f32_ulp
        };
        self.samples += 1;
        self.max_error_in_local_ulps = self.max_error_in_local_ulps.max(error_in_ulps);
        if self.worst.is_none() || absolute_error > self.max_absolute_error {
            self.max_absolute_error = absolute_error;
            self.worst = Some(ErrorObservation {
                case,
                input_a,
                input_b,
                reference,
                candidate,
                absolute_error,
                local_f32_ulp,
            });
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RuntimeCalibration {
    pub edge_boundary: ErrorStats,
    pub discarded_residual_edge_rebase: ErrorStats,
    pub longitudinal: ErrorStats,
    pub physical_gap: ErrorStats,
    pub computed_speed: ErrorStats,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GapSafetyReport {
    pub samples: usize,
    pub divergences: usize,
    pub first_divergence: Option<String>,
    pub exact_contact_preserved: bool,
    pub positive_gap_preserved: bool,
    pub negative_overlap_rejected: bool,
    pub leader_selection_preserved: bool,
    pub spawn_rejection_preserved: bool,
    pub leave_rejection_preserved: bool,
    pub no_overlap_projection_preserved: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GapClass {
    Overlap,
    Contact,
    Positive,
}

pub fn calibrate_runtime_chains() -> RuntimeCalibration {
    let (edge_boundary, discarded_residual_edge_rebase) = calibrate_edge_boundary();
    RuntimeCalibration {
        edge_boundary,
        discarded_residual_edge_rebase,
        longitudinal: calibrate_longitudinal(),
        physical_gap: calibrate_physical_gap(),
        computed_speed: calibrate_computed_speed(),
    }
}

#[derive(Clone, Copy, Debug)]
struct DeterministicRng(u64);

impl DeterministicRng {
    fn next_u32(&mut self) -> u32 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        (value >> 32) as u32
    }

    fn index(&mut self, upper_exclusive: usize) -> usize {
        self.next_u32() as usize % upper_exclusive
    }

    fn f32_between(&mut self, minimum: f32, maximum: f32) -> f32 {
        let unit = self.next_u32() as f64 / u32::MAX as f64;
        (f64::from(minimum) + unit * f64::from(maximum - minimum)) as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ConstraintAttribution {
    Signal,
    Parking,
    RouteEnd,
    Base,
}

#[derive(Clone, Copy, Debug)]
struct ConstraintSet {
    route_end: Option<f32>,
    signal: Option<f32>,
    parking: Option<f32>,
}

#[derive(Clone, Copy, Debug)]
struct ReducedCandidate {
    attribution: ConstraintAttribution,
    travel: f64,
    speed: f64,
    hard_projection: bool,
    constraint_distance: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscreteOutcome {
    attribution: ConstraintAttribution,
    route_completed: bool,
    parking_reached: bool,
    speed_is_near_zero: bool,
    event_order: Vec<ConstraintAttribution>,
}

#[derive(Clone, Debug, Default)]
pub struct DiscreteCalibration {
    pub samples: usize,
    pub divergences: usize,
    pub first_divergence: Option<String>,
    pub signal_wins_equal_distance_tie: bool,
    pub spatial_event_precedes_leader_event: bool,
}

pub fn calibrate_constraint_cross_matrix() -> DiscreteCalibration {
    let mut report = DiscreteCalibration::default();
    let cases = [
        (1_u64, 100.0_f32, 0.1_f32),
        (16, 13.9, 0.222_4),
        (1_000, 100.0, 100.0),
    ];
    for (delta_time_ms, base_speed, base_travel) in cases {
        let tolerance = LONGITUDINAL_TOLERANCE_CANDIDATE_METERS as f32;
        let distance_probes = [
            base_travel.next_down(),
            base_travel,
            base_travel.next_up(),
            (base_travel - tolerance).max(0.0),
            base_travel + tolerance,
            (base_travel * 0.5).max(f32::MIN_POSITIVE),
        ];
        for distance in distance_probes {
            let constraint_sets = [
                ConstraintSet {
                    route_end: Some(distance),
                    signal: None,
                    parking: None,
                },
                ConstraintSet {
                    route_end: None,
                    signal: Some(distance),
                    parking: None,
                },
                ConstraintSet {
                    route_end: None,
                    signal: None,
                    parking: Some(distance),
                },
                ConstraintSet {
                    route_end: Some(distance),
                    signal: Some(distance),
                    parking: Some(distance),
                },
                ConstraintSet {
                    route_end: Some(distance.next_down()),
                    signal: Some(distance.next_up()),
                    parking: Some(distance),
                },
            ];
            for constraints in constraint_sets {
                for leader_cap in [None, Some((base_travel * 0.25).max(0.0))] {
                    let reference = reduce_constraints(
                        false,
                        delta_time_ms,
                        base_speed,
                        base_travel,
                        base_travel * 0.75,
                        constraints,
                        leader_cap,
                    );
                    let candidate = reduce_constraints(
                        true,
                        delta_time_ms,
                        base_speed,
                        base_travel,
                        base_travel * 0.75,
                        constraints,
                        leader_cap,
                    );
                    report.samples += 1;
                    if candidate != reference {
                        report.divergences += 1;
                        report.first_divergence.get_or_insert_with(|| {
                            format!(
                                "delta_time_ms={delta_time_ms} base_speed={base_speed:?} base_travel={base_travel:?} constraints={constraints:?} leader_cap={leader_cap:?} reference={reference:?} candidate={candidate:?}",
                            )
                        });
                    }
                }
            }
        }
    }

    let equal_tie = reduce_constraints(
        true,
        16,
        13.9,
        0.222_4,
        0.2,
        ConstraintSet {
            route_end: Some(0.1),
            signal: Some(0.1),
            parking: Some(0.1),
        },
        None,
    );
    report.signal_wins_equal_distance_tie = equal_tie.attribution == ConstraintAttribution::Signal;
    let spatial_then_leader = reduce_constraints(
        true,
        16,
        13.9,
        0.222_4,
        0.2,
        ConstraintSet {
            route_end: None,
            signal: Some(0.1),
            parking: None,
        },
        Some(0.05),
    );
    report.spatial_event_precedes_leader_event = spatial_then_leader.event_order
        == [ConstraintAttribution::Signal, ConstraintAttribution::Base];
    report
}

fn reduce_constraints(
    target_f32: bool,
    delta_time_ms: u64,
    base_speed: f32,
    base_travel: f32,
    emergency_min_travel: f32,
    constraints: ConstraintSet,
    leader_cap: Option<f32>,
) -> DiscreteOutcome {
    let exact_delta_time = delta_time_ms as f64 / 1_000.0;
    let target_delta_time = exact_delta_time as f32;
    let mut selected = ReducedCandidate {
        attribution: ConstraintAttribution::Base,
        travel: f64::from(base_travel),
        speed: f64::from(base_speed),
        hard_projection: false,
        constraint_distance: None,
    };
    for (attribution, distance) in [
        (ConstraintAttribution::RouteEnd, constraints.route_end),
        (ConstraintAttribution::Signal, constraints.signal),
        (ConstraintAttribution::Parking, constraints.parking),
    ] {
        let Some(distance) = distance else {
            continue;
        };
        let travel = base_travel.min(distance);
        let speed = if distance < base_travel {
            if target_f32 {
                f64::from(target_speed_after_travel_cap(
                    base_speed,
                    base_travel,
                    distance,
                    target_delta_time,
                ))
            } else {
                exact_speed_after_travel_cap(base_speed, base_travel, distance, exact_delta_time)
            }
        } else {
            f64::from(base_speed)
        };
        let travel = f64::from(travel);
        let distance = f64::from(distance);
        let speed = if travel + LONGITUDINAL_TOLERANCE_CANDIDATE_METERS >= distance {
            0.0
        } else {
            speed
        };
        let hard_projection = matches!(
            attribution,
            ConstraintAttribution::Signal | ConstraintAttribution::Parking
        ) && distance + LONGITUDINAL_TOLERANCE_CANDIDATE_METERS
            < f64::from(emergency_min_travel)
            && distance < f64::from(base_travel);
        let candidate = ReducedCandidate {
            attribution,
            travel,
            speed,
            hard_projection,
            constraint_distance: Some(distance),
        };
        if reduced_candidate_is_stricter(candidate, selected) {
            selected = candidate;
        }
    }

    let mut event_order = Vec::new();
    if selected.hard_projection {
        event_order.push(selected.attribution);
    }
    let travel_before_leader = selected.travel;
    if let Some(leader_cap) = leader_cap {
        let leader_cap = f64::from(leader_cap);
        if leader_cap < selected.travel {
            selected.travel = leader_cap;
            selected.speed = if target_f32 {
                f64::from(target_speed_after_travel_cap(
                    selected.speed as f32,
                    travel_before_leader as f32,
                    leader_cap as f32,
                    target_delta_time,
                ))
            } else {
                exact_speed_after_travel_cap(
                    selected.speed as f32,
                    travel_before_leader as f32,
                    leader_cap as f32,
                    exact_delta_time,
                )
            };
            if leader_cap + PHYSICAL_GAP_TOLERANCE_CANDIDATE_METERS
                < f64::from(emergency_min_travel)
            {
                // 在这份研究专用事件序列中，`Base` 表示跟车安全投影事件。
                event_order.push(ConstraintAttribution::Base);
            }
        }
    }

    let constraint_reached = selected.constraint_distance.is_some_and(|distance| {
        selected.travel + LONGITUDINAL_TOLERANCE_CANDIDATE_METERS >= distance
    });
    let speed_is_near_zero = selected.speed <= COMPUTED_SPEED_TOLERANCE_CANDIDATE_METERS_PER_SECOND;
    DiscreteOutcome {
        attribution: selected.attribution,
        route_completed: selected.attribution == ConstraintAttribution::RouteEnd
            && constraint_reached,
        parking_reached: selected.attribution == ConstraintAttribution::Parking
            && constraint_reached
            && speed_is_near_zero,
        speed_is_near_zero,
        event_order,
    }
}

fn reduced_candidate_is_stricter(left: ReducedCandidate, right: ReducedCandidate) -> bool {
    left.travel
        .total_cmp(&right.travel)
        .then_with(|| left.speed.total_cmp(&right.speed))
        .then_with(|| left.attribution.cmp(&right.attribution))
        .is_lt()
}

fn calibrate_edge_boundary() -> (ErrorStats, ErrorStats) {
    let mut stats = ErrorStats::default();
    let mut discarded_residual_rebase = ErrorStats::default();
    let edge_lengths = [0.01_f32, 0.1, 1.0, 16.0, 128.0, 1_024.0, 8_192.0, 10_000.0];
    let travels = [0.001_f32, 0.016, 0.1, 0.3, 1.0, 13.9, 100.0];

    for edge_length in edge_lengths {
        for travel in travels {
            let initial = f64::from(edge_length) - f64::from(travel.min(edge_length));
            let mut endpoint = ResidualProgress::from_normalized(initial);
            endpoint.add_f32(travel.min(edge_length));
            stats.observe(
                "exact_endpoint",
                f64::from(edge_length),
                f64::from(travel),
                f64::from(edge_length),
                endpoint.effective(),
            );
        }
    }

    // 长时累计与跨 edge 重基线必须在每次跨界后继续保留残差。
    for edge_length in [1.0_f32, 128.0, 10_000.0] {
        for travel in [0.001_f32, 0.016, 0.1, 0.3, 13.9] {
            let mut reference = 0.0_f64;
            let mut candidate = ResidualProgress::from_normalized(0.0);
            let mut discarded_residual_candidate = candidate;
            for _ in 0..36_000 {
                reference += f64::from(travel);
                candidate.add_f32(travel);
                discarded_residual_candidate.add_f32(travel);
                while reference >= f64::from(edge_length) {
                    reference -= f64::from(edge_length);
                    candidate.rebase_after_edge(edge_length);
                    discarded_residual_candidate.add_f32(-edge_length);
                }
                stats.observe(
                    "long_duration_remainder",
                    f64::from(edge_length),
                    f64::from(travel),
                    reference,
                    candidate.effective(),
                );
                discarded_residual_rebase.observe(
                    "discarded_residual_rebase",
                    f64::from(edge_length),
                    f64::from(travel),
                    reference,
                    discarded_residual_candidate.effective(),
                );
            }
        }
    }

    // 一个目标 tick 可以合法消费多个短 edge occurrence（路线中的 edge 出现位置）。
    for edge_length in [0.01_f32, 0.1, 1.0] {
        let travel = 100.0_f32;
        let mut reference = f64::from(travel);
        let mut candidate = ResidualProgress::from_normalized(0.0);
        let mut discarded_residual_candidate = candidate;
        candidate.add_f32(travel);
        discarded_residual_candidate.add_f32(travel);
        while reference >= f64::from(edge_length) {
            reference -= f64::from(edge_length);
            candidate.rebase_after_edge(edge_length);
            discarded_residual_candidate.add_f32(-edge_length);
        }
        stats.observe(
            "single_tick_multi_edge",
            f64::from(edge_length),
            f64::from(travel),
            reference,
            candidate.effective(),
        );
        discarded_residual_rebase.observe(
            "discarded_residual_multi_edge",
            f64::from(edge_length),
            f64::from(travel),
            reference,
            discarded_residual_candidate.effective(),
        );
    }
    (stats, discarded_residual_rebase)
}

fn calibrate_longitudinal() -> ErrorStats {
    let mut stats = ErrorStats::default();
    for delta_time_ms in [1_u64, 16, 1_000] {
        let exact_delta_time = delta_time_ms as f64 / 1_000.0;
        let target_delta_time = exact_delta_time as f32;
        for speed in [0.0_f32, 0.001, 0.1, 1.0, 13.9, 50.0, 100.0] {
            for acceleration in [-50.0_f32, -2.0, 0.0, 1.4, 50.0] {
                let (reference_speed, reference_travel) =
                    exact_ballistic(speed, acceleration, exact_delta_time);
                let (candidate_speed, candidate_travel) =
                    target_ballistic(speed, acceleration, target_delta_time);
                stats.observe(
                    "ballistic_travel",
                    f64::from(speed),
                    delta_time_ms as f64,
                    reference_travel,
                    f64::from(candidate_travel),
                );
                stats.observe(
                    "ballistic_speed_to_distance",
                    f64::from(speed),
                    f64::from(acceleration),
                    reference_speed * exact_delta_time,
                    f64::from(candidate_speed) * f64::from(target_delta_time),
                );
            }
        }
    }

    let mut random = DeterministicRng(CALIBRATION_SEED ^ 0x10_0617_0001);
    let tick_milliseconds = [1_u64, 16, 1_000];
    for _ in 0..RANDOM_SAMPLE_COUNT {
        let delta_time_ms = tick_milliseconds[random.index(tick_milliseconds.len())];
        let exact_delta_time = delta_time_ms as f64 / 1_000.0;
        let target_delta_time = exact_delta_time as f32;
        let speed = random.f32_between(0.0, 100.0);
        let acceleration = random.f32_between(-50.0, 50.0);
        let (reference_speed, reference_travel) =
            exact_ballistic(speed, acceleration, exact_delta_time);
        let (candidate_speed, candidate_travel) =
            target_ballistic(speed, acceleration, target_delta_time);
        stats.observe(
            "random_ballistic_travel",
            f64::from(speed),
            delta_time_ms as f64,
            reference_travel,
            f64::from(candidate_travel),
        );
        stats.observe(
            "random_ballistic_speed_to_distance",
            f64::from(speed),
            f64::from(acceleration),
            reference_speed * exact_delta_time,
            f64::from(candidate_speed) * f64::from(target_delta_time),
        );
    }

    // 位置匹配读取补偿残差后的有效值，并覆盖 10 km 端点。
    for position in [0.0, 0.1, 1.0, 127.999_99, 1_024.0, 8_192.0, 10_000.0] {
        let candidate = ResidualProgress::from_normalized(position);
        stats.observe(
            "effective_position",
            position,
            0.0,
            position,
            candidate.effective(),
        );
    }
    stats
}

fn calibrate_physical_gap() -> ErrorStats {
    let mut stats = ErrorStats::default();
    let gap_probes = [
        -0.001_f64,
        -PHYSICAL_GAP_TOLERANCE_CANDIDATE_METERS,
        -0.000_001,
        0.0,
        0.000_001,
        PHYSICAL_GAP_TOLERANCE_CANDIDATE_METERS,
        0.001,
    ];
    for follower in [0.0_f64, 0.1, 127.0, 1_024.0, 8_000.0, 9_871.0] {
        for vehicle_length in [0.1_f32, 4.5, 32.0, 128.0] {
            for gap in gap_probes {
                let leader_value = follower + f64::from(vehicle_length) + gap;
                if !(0.0..=MAX_EDGE_LENGTH_METERS).contains(&leader_value) {
                    continue;
                }
                let follower_progress = ResidualProgress::from_normalized(follower);
                let leader_progress = ResidualProgress::from_normalized(leader_value);
                let exact_gap = leader_progress.effective()
                    - follower_progress.effective()
                    - f64::from(vehicle_length);
                let target_front_distance =
                    (leader_progress.effective() - follower_progress.effective()) as f32;
                let target_gap = target_front_distance - vehicle_length;
                stats.observe(
                    "same_edge_gap",
                    follower_progress.effective(),
                    f64::from(vehicle_length),
                    exact_gap,
                    f64::from(target_gap),
                );
            }
        }
    }

    let mut random = DeterministicRng(CALIBRATION_SEED ^ 0x6A_4150_0001);
    for _ in 0..RANDOM_SAMPLE_COUNT {
        let vehicle_length = random.f32_between(0.1, 128.0);
        let follower_value = random.f32_between(0.0, 9_871.0) as f64;
        let gap = random.f32_between(-0.001, 0.001) as f64;
        let leader_value = follower_value + f64::from(vehicle_length) + gap;
        if !(0.0..=MAX_EDGE_LENGTH_METERS).contains(&leader_value) {
            continue;
        }
        let follower = ResidualProgress::from_normalized(follower_value);
        let leader = ResidualProgress::from_normalized(leader_value);
        let exact_gap = leader.effective() - follower.effective() - f64::from(vehicle_length);
        let target_gap = (leader.effective() - follower.effective()) as f32 - vehicle_length;
        stats.observe(
            "random_same_edge_gap",
            follower_value,
            f64::from(vehicle_length),
            exact_gap,
            f64::from(target_gap),
        );
    }

    // 跨 edge 路径先在 f64 中消去大坐标，再把局部前距转换为 f32。
    for edge_length in [128.0_f32, 1_024.0, 8_192.0, 10_000.0] {
        for vehicle_length in [0.1_f32, 4.5, 128.0] {
            for gap in gap_probes {
                let follower_progress = f64::from(edge_length) - f64::from(vehicle_length) * 0.5;
                let leader_progress = f64::from(vehicle_length) * 0.5 + gap;
                if leader_progress < 0.0 {
                    continue;
                }
                let follower = ResidualProgress::from_normalized(follower_progress);
                let leader = ResidualProgress::from_normalized(leader_progress);
                let exact_front_distance =
                    (f64::from(edge_length) - follower.effective()) + leader.effective();
                let exact_gap = exact_front_distance - f64::from(vehicle_length);
                let target_gap = exact_front_distance as f32 - vehicle_length;
                stats.observe(
                    "cross_edge_gap",
                    f64::from(edge_length),
                    f64::from(vehicle_length),
                    exact_gap,
                    f64::from(target_gap),
                );
            }
        }
    }
    stats
}

pub fn calibrate_gap_safety_matrix() -> GapSafetyReport {
    let mut report = GapSafetyReport::default();
    let clear_gap = PHYSICAL_GAP_TOLERANCE_CANDIDATE_METERS * 3.0;
    for follower in [0.0_f64, 127.0, 1_024.0, 8_000.0, 9_871.0] {
        for vehicle_length in [0.1_f32, 4.5, 32.0, 128.0] {
            for intended_gap in [-0.001_f64, -clear_gap, 0.0, clear_gap, 0.001] {
                let leader_value = follower + f64::from(vehicle_length) + intended_gap;
                if !(0.0..=MAX_EDGE_LENGTH_METERS).contains(&leader_value) {
                    continue;
                }
                let follower_progress = ResidualProgress::from_normalized(follower);
                let leader_progress = ResidualProgress::from_normalized(leader_value);
                let reference_gap = leader_progress.effective()
                    - follower_progress.effective()
                    - f64::from(vehicle_length);
                let candidate_gap = f64::from(
                    (leader_progress.effective() - follower_progress.effective()) as f32
                        - vehicle_length,
                );
                let reference_class = classify_gap(reference_gap);
                let candidate_class = classify_gap(candidate_gap);
                let reference_leader = reference_gap <= 1.0;
                let candidate_leader = candidate_gap <= 1.0;
                let reference_reject = reference_class == GapClass::Overlap;
                let candidate_reject = candidate_class == GapClass::Overlap;
                let reference_projected = projected_gap(reference_gap, 0.25, 0.0);
                let candidate_projected = projected_gap(candidate_gap, 0.25, 0.0);
                let projection_preserved = reference_reject
                    || (classify_gap(reference_projected) == classify_gap(candidate_projected)
                        && candidate_projected >= -PHYSICAL_GAP_TOLERANCE_CANDIDATE_METERS);
                let matched = reference_class == candidate_class
                    && reference_leader == candidate_leader
                    && reference_reject == candidate_reject
                    && projection_preserved;
                report.samples += 1;
                if !matched {
                    report.divergences += 1;
                    report.first_divergence.get_or_insert_with(|| {
                        format!(
                            "follower={follower:.12} vehicle_length={vehicle_length:.12} intended_gap={intended_gap:.12} reference_gap={reference_gap:.12} candidate_gap={candidate_gap:.12} reference_class={reference_class:?} candidate_class={candidate_class:?} reference_projected={reference_projected:.12} candidate_projected={candidate_projected:.12}",
                        )
                    });
                }
                report.exact_contact_preserved |=
                    intended_gap == 0.0 && candidate_class == GapClass::Contact;
                report.positive_gap_preserved |=
                    intended_gap == clear_gap && candidate_class == GapClass::Positive;
                report.negative_overlap_rejected |=
                    intended_gap == -clear_gap && candidate_class == GapClass::Overlap;
                report.leader_selection_preserved |= reference_leader == candidate_leader;
                report.spawn_rejection_preserved |= reference_reject == candidate_reject;
                report.leave_rejection_preserved |= reference_reject == candidate_reject;
                report.no_overlap_projection_preserved |=
                    !reference_reject && classify_gap(candidate_projected) != GapClass::Overlap;
            }
        }
    }
    report
}

fn classify_gap(gap: f64) -> GapClass {
    if gap < -PHYSICAL_GAP_TOLERANCE_CANDIDATE_METERS {
        GapClass::Overlap
    } else if gap <= PHYSICAL_GAP_TOLERANCE_CANDIDATE_METERS {
        GapClass::Contact
    } else {
        GapClass::Positive
    }
}

fn projected_gap(gap: f64, follower_travel: f64, leader_travel: f64) -> f64 {
    let allowed_follower_travel = (gap + leader_travel).max(0.0);
    gap + leader_travel - follower_travel.min(allowed_follower_travel)
}

fn calibrate_computed_speed() -> ErrorStats {
    let mut stats = ErrorStats::default();
    for delta_time_ms in [1_u64, 16, 1_000] {
        let exact_delta_time = delta_time_ms as f64 / 1_000.0;
        let target_delta_time = exact_delta_time as f32;
        for candidate_speed in [0.001_f32, 0.01, 1.0, 13.9, 50.0, 100.0] {
            let candidate_travel = candidate_speed * target_delta_time;
            for intended_speed in [
                0.0_f32,
                COMPUTED_SPEED_TOLERANCE_CANDIDATE_METERS_PER_SECOND as f32,
                0.001,
                0.01,
                1.0,
            ] {
                if intended_speed > candidate_speed {
                    continue;
                }
                let removed_travel =
                    f64::from(candidate_speed - intended_speed) * exact_delta_time * 0.5;
                let final_travel = (f64::from(candidate_travel) - removed_travel) as f32;
                if final_travel < 0.0 || final_travel > candidate_travel {
                    continue;
                }
                for final_travel in [
                    final_travel.next_down().max(0.0),
                    final_travel,
                    final_travel.next_up().min(candidate_travel),
                ] {
                    let reference = exact_speed_after_travel_cap(
                        candidate_speed,
                        candidate_travel,
                        final_travel,
                        exact_delta_time,
                    );
                    let candidate = target_speed_after_travel_cap(
                        candidate_speed,
                        candidate_travel,
                        final_travel,
                        target_delta_time,
                    );
                    stats.observe(
                        "projection_speed",
                        f64::from(candidate_speed),
                        delta_time_ms as f64,
                        reference,
                        f64::from(candidate),
                    );
                }
            }
        }
    }
    let mut random = DeterministicRng(CALIBRATION_SEED ^ 0x5F_EED0_0001);
    let tick_milliseconds = [1_u64, 16, 1_000];
    for _ in 0..RANDOM_SAMPLE_COUNT {
        let delta_time_ms = tick_milliseconds[random.index(tick_milliseconds.len())];
        let exact_delta_time = delta_time_ms as f64 / 1_000.0;
        let target_delta_time = exact_delta_time as f32;
        let candidate_speed = random.f32_between(0.000_001, 100.0);
        let candidate_travel = candidate_speed * target_delta_time;
        let intended_speed = random.f32_between(0.0, candidate_speed);
        let removed_travel = f64::from(candidate_speed - intended_speed) * exact_delta_time * 0.5;
        let final_travel = (f64::from(candidate_travel) - removed_travel) as f32;
        if final_travel < 0.0 || final_travel > candidate_travel {
            continue;
        }
        let reference = exact_speed_after_travel_cap(
            candidate_speed,
            candidate_travel,
            final_travel,
            exact_delta_time,
        );
        let candidate = target_speed_after_travel_cap(
            candidate_speed,
            candidate_travel,
            final_travel,
            target_delta_time,
        );
        stats.observe(
            "random_projection_speed",
            f64::from(candidate_speed),
            delta_time_ms as f64,
            reference,
            f64::from(candidate),
        );
    }
    stats
}

fn exact_ballistic(speed: f32, acceleration: f32, delta_time: f64) -> (f64, f64) {
    let speed = f64::from(speed);
    let acceleration = f64::from(acceleration);
    if acceleration < 0.0 {
        let stop_time = speed / -acceleration;
        if stop_time < delta_time {
            return (0.0, speed * speed / (-2.0 * acceleration));
        }
    }
    let final_speed = (speed + acceleration * delta_time).max(0.0);
    let travel = 0.5 * speed * delta_time + 0.5 * final_speed * delta_time;
    (final_speed, travel)
}

fn target_ballistic(speed: f32, acceleration: f32, delta_time: f32) -> (f32, f32) {
    if acceleration < 0.0 {
        let stop_time = speed / -acceleration;
        if stop_time < delta_time {
            return (0.0, speed / (-2.0 * acceleration) * speed);
        }
    }
    let final_speed = (speed + acceleration * delta_time).max(0.0);
    let travel = 0.5 * speed * delta_time + 0.5 * final_speed * delta_time;
    (final_speed, travel)
}

fn exact_speed_after_travel_cap(
    candidate_speed: f32,
    candidate_travel: f32,
    final_travel: f32,
    delta_time: f64,
) -> f64 {
    if candidate_speed == 0.0 {
        return 0.0;
    }
    let removed_travel = f64::from(candidate_travel) - f64::from(final_travel);
    let reduction = removed_travel / delta_time * 2.0;
    (f64::from(candidate_speed) - reduction)
        .max(0.0)
        .min(f64::from(candidate_speed))
}

fn target_speed_after_travel_cap(
    candidate_speed: f32,
    candidate_travel: f32,
    final_travel: f32,
    delta_time: f32,
) -> f32 {
    if candidate_speed == 0.0 {
        return 0.0;
    }
    let removed_travel = candidate_travel - final_travel;
    let reduction = removed_travel / delta_time * 2.0;
    (candidate_speed - reduction).max(0.0).min(candidate_speed)
}

fn f32_ulp(value: f32) -> f64 {
    if !value.is_finite() {
        return f64::INFINITY;
    }
    let upper = value.next_up();
    (f64::from(upper) - f64::from(value)).abs()
}
