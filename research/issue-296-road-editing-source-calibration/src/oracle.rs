//! ADR 0022 B1 的独立离线 4097 点观测器。
//!
//! 本模块只消费公开 authoring model 与最终 Canonical LIR。它不调用 production 私有
//! geometry evaluator，也不参与 accept/reject；重复实现冻结运算图是为了让 G3 观测能
//! 发现 production 细分或参数归属漂移。

use std::collections::BTreeMap;

use laneflow_compiler::road_editing::{
    LinearWidthProfile, RoadEditingCorridorElement, RoadEditingCurveProgram,
    RoadEditingCurveSegmentGeometry, RoadEditingDeclaration, RoadEditingLaneDirection,
    RoadEditingPoint3, RoadEditingStationEnd,
};
use laneflow_compiler::{
    CanonicalIdentityFieldView, CanonicalPoint3F32, CompilationOutput, GeometryAccuracyProfile,
    GeometryDirectionProfile,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{GeneratorError, TypedP100Module};

const GRID_DENOMINATOR: f64 = 4096.0;
const GRID_POINT_COUNT: u64 = 4097;
const MAX_DEPTH: u8 = 20;
const MAX_SOURCE_JOIN_GAP_METERS: f64 = 0.005;
const MODULE_NAMESPACE_TAG: u16 = 1;
const LANE_EDGE_KEY_TAG: u16 = 5;
const FACILITY_BAND_KEY_TAG: u16 = 26;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeometryObservation {
    pub evaluator_interval_count: u64,
    pub observed_sample_count: u64,
    pub evaluator_interval_identity_sha256: String,
    pub position_error: PositionErrorStatistics,
    pub worst_observed_error: WorstObservedError,
    pub final_f32_direction_jump_maximum_degrees_bits: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PositionErrorStatistics {
    pub p50_meters_bits: String,
    pub p95_meters_bits: String,
    pub p99_meters_bits: String,
    pub maximum_meters_bits: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorstObservedError {
    pub module: String,
    pub source_address: String,
    pub source_segment_ordinal: u32,
    pub station_row_ordinal: Option<u32>,
    pub evaluator_interval_ordinal: u64,
    pub parameter_bits: String,
}

pub(crate) struct ObservationPlan {
    tasks: Vec<ObservationTask>,
}

struct ObservationTask {
    module: String,
    source_address: String,
    target: ObservationTarget,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    intervals: Vec<OracleInterval>,
}

enum ObservationTarget {
    InternalAlignment,
    LaneEdge {
        key: String,
        direction: RoadEditingLaneDirection,
    },
    FacilityBand {
        key: String,
    },
}

#[derive(Clone, Copy)]
struct OracleInterval {
    evaluator: Evaluator,
    parameter_start: f64,
    parameter_end: f64,
    source_segment_ordinal: u32,
    station_row_ordinal: Option<u32>,
    source_boundary: bool,
}

#[derive(Clone, Copy)]
enum Evaluator {
    Reference(Segment),
    Offset {
        segment: Segment,
        station: StationInterval,
        offset: OffsetInterval,
    },
}

#[derive(Clone, Copy)]
struct StationInterval {
    parameter_start: f64,
    parameter_end: f64,
    cumulative_start_meters: f64,
    cumulative_end_meters: f64,
}

#[derive(Clone, Copy)]
struct OffsetInterval {
    station_start_meters: f64,
    station_end_meters: f64,
    offset_start_meters: f64,
    offset_end_meters: f64,
}

#[derive(Clone, Copy)]
enum Segment {
    Line {
        start: Point,
        end: Point,
    },
    Cubic {
        start: Point,
        control_1: Point,
        control_2: Point,
        end: Point,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Point {
    x: f64,
    y: f64,
    z: f64,
}

#[derive(Clone, Copy)]
struct Sample {
    point: Point,
    first: Point,
}

#[derive(Clone, Copy)]
struct Vertex {
    parameter: f64,
    point: F32Point,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct F32Point {
    x: u32,
    y: u32,
    z: u32,
}

#[derive(Clone, Copy)]
struct StationRow {
    segment_ordinal: u32,
    parameter_start: f64,
    parameter_end: f64,
    cumulative_start_meters: f64,
    cumulative_end_meters: f64,
}

#[derive(Clone)]
struct DistanceAttribution {
    module: String,
    source_address: String,
    source_segment_ordinal: u32,
    station_row_ordinal: Option<u32>,
    interval_ordinal: u64,
    parameter: f64,
}

pub(crate) fn prepare_observation(
    modules: &[TypedP100Module],
) -> Result<ObservationPlan, GeneratorError> {
    let mut tasks = Vec::new();
    for module in modules {
        prepare_module(module, &mut tasks)?;
    }
    Ok(ObservationPlan { tasks })
}

pub(crate) fn observe(
    plan: &ObservationPlan,
    output: &CompilationOutput,
) -> Result<GeometryObservation, GeneratorError> {
    let lane_points = lane_points(output)?;
    let facility_points = facility_points(output)?;
    let mut distances = Vec::new();
    let mut worst: Option<(f64, DistanceAttribution)> = None;
    let mut identities = Sha256::new();
    let mut interval_ordinal = 0_u64;

    for task in &plan.tasks {
        let (combined, interval_vertices) = approximate_task(task)?;
        match &task.target {
            ObservationTarget::InternalAlignment => {}
            ObservationTarget::LaneEdge { key, direction } => {
                let mut expected = combined
                    .iter()
                    .map(|vertex| vertex.point)
                    .collect::<Vec<_>>();
                if *direction == RoadEditingLaneDirection::Backward {
                    expected.reverse();
                }
                require_final_points(
                    &task.module,
                    key,
                    &expected,
                    lane_points.get(&(task.module.clone(), key.clone())),
                )?;
            }
            ObservationTarget::FacilityBand { key } => require_final_points(
                &task.module,
                key,
                &combined
                    .iter()
                    .map(|vertex| vertex.point)
                    .collect::<Vec<_>>(),
                facility_points.get(&(task.module.clone(), key.clone())),
            )?,
        }

        for (interval, vertices) in task.intervals.iter().zip(interval_vertices) {
            let identity = format!(
                "{}|{}|{}|{}|{}|{:016x}|{:016x}\n",
                task.module,
                task.source_address,
                interval.source_segment_ordinal,
                interval
                    .station_row_ordinal
                    .map_or("-".to_owned(), |value| value.to_string()),
                interval_ordinal,
                interval.parameter_start.to_bits(),
                interval.parameter_end.to_bits(),
            );
            identities.update(identity.as_bytes());
            for k in 0_u32..=4096 {
                let parameter = grid_parameter(interval.parameter_start, interval.parameter_end, k);
                let point = interval.evaluator.evaluate(parameter)?.point;
                let distance = distance_to_parameter_segment(point, parameter, &vertices)?;
                let attribution = DistanceAttribution {
                    module: task.module.clone(),
                    source_address: task.source_address.clone(),
                    source_segment_ordinal: interval.source_segment_ordinal,
                    station_row_ordinal: interval.station_row_ordinal,
                    interval_ordinal,
                    parameter,
                };
                if worst
                    .as_ref()
                    .is_none_or(|(maximum, _)| distance > *maximum)
                {
                    worst = Some((distance, attribution.clone()));
                }
                distances.push(distance);
            }
            interval_ordinal = interval_ordinal
                .checked_add(1)
                .ok_or_else(|| contract("evaluator interval count overflow"))?;
        }
    }
    let expected_samples = interval_ordinal
        .checked_mul(GRID_POINT_COUNT)
        .ok_or_else(|| contract("observed sample count overflow"))?;
    if usize_u64(distances.len()) != expected_samples || distances.is_empty() {
        return Err(contract("observed sample completeness mismatch"));
    }
    distances.sort_by(f64::total_cmp);
    let p50 = percentile(&distances, 50)?;
    let p95 = percentile(&distances, 95)?;
    let p99 = percentile(&distances, 99)?;
    let maximum = *distances
        .last()
        .ok_or_else(|| contract("observed distance population is empty"))?;
    let (worst_distance, worst) = worst.ok_or_else(|| contract("worst attribution is missing"))?;
    if worst_distance.to_bits() != maximum.to_bits() {
        return Err(contract("worst attribution does not match sorted maximum"));
    }
    Ok(GeometryObservation {
        evaluator_interval_count: interval_ordinal,
        observed_sample_count: expected_samples,
        evaluator_interval_identity_sha256: hex(&identities.finalize()),
        position_error: PositionErrorStatistics {
            p50_meters_bits: bits(p50),
            p95_meters_bits: bits(p95),
            p99_meters_bits: bits(p99),
            maximum_meters_bits: bits(maximum),
        },
        worst_observed_error: WorstObservedError {
            module: worst.module,
            source_address: worst.source_address,
            source_segment_ordinal: worst.source_segment_ordinal,
            station_row_ordinal: worst.station_row_ordinal,
            evaluator_interval_ordinal: worst.interval_ordinal,
            parameter_bits: bits(worst.parameter),
        },
        final_f32_direction_jump_maximum_degrees_bits: bits(max_direction_jump_degrees(
            lane_points.values().chain(facility_points.values()),
        )?),
    })
}

fn prepare_module(
    module: &TypedP100Module,
    tasks: &mut Vec<ObservationTask>,
) -> Result<(), GeneratorError> {
    let source = module.module();
    let namespace = module.namespace().to_owned();
    let accuracy = source.geometry_accuracy_profile();
    let direction = source.geometry_direction_profile();
    let alignments = source
        .road_alignments()
        .iter()
        .map(|alignment| (alignment.road_alignment_key(), alignment))
        .collect::<BTreeMap<_, _>>();
    let declarations = source.declarations();
    let corridors = declarations
        .iter()
        .filter_map(|declaration| match declaration {
            RoadEditingDeclaration::RoadCorridor(value) => Some((value.road_corridor_key(), value)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let sections = declarations
        .iter()
        .filter_map(|declaration| match declaration {
            RoadEditingDeclaration::RoadSection(value) => Some((value.road_section_key(), value)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let lanes = declarations
        .iter()
        .filter_map(|declaration| match declaration {
            RoadEditingDeclaration::AuthoringLane(value) => {
                Some((value.authoring_lane_key(), value))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let bands = declarations
        .iter()
        .filter_map(|declaration| match declaration {
            RoadEditingDeclaration::FacilityBand(value) => Some((value.facility_band_key(), value)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();

    for alignment in alignments.values() {
        tasks.push(ObservationTask {
            module: namespace.clone(),
            source_address: format!("RoadAlignment:{}", alignment.road_alignment_key()),
            target: ObservationTarget::InternalAlignment,
            accuracy: GeometryAccuracyProfile::Fine2Cm,
            direction: GeometryDirectionProfile::Smooth1Deg,
            intervals: reference_intervals(alignment.reference_line())?,
        });
    }

    let mut offset_tasks = Vec::new();
    for corridor in corridors.values() {
        let alignment = alignments
            .get(corridor.road_alignment().key())
            .ok_or_else(|| contract("corridor alignment is missing from module"))?;
        let station_rows = station_rows(alignment.reference_line())?;
        let station_end = match corridor.end_station() {
            RoadEditingStationEnd::Finite(value) => value,
            RoadEditingStationEnd::AlignmentEnd => {
                station_rows
                    .last()
                    .ok_or_else(|| contract("alignment station rows are empty"))?
                    .cumulative_end_meters
            }
        };
        let mut members = Vec::new();
        for element in corridor.elements() {
            match element {
                RoadEditingCorridorElement::RoadSection(reference) => {
                    let section = sections
                        .get(reference.local_key())
                        .ok_or_else(|| contract("corridor section is missing"))?;
                    for lane_reference in section.authoring_lanes() {
                        let lane = lanes
                            .get(lane_reference.local_key())
                            .ok_or_else(|| contract("section lane is missing"))?;
                        members.push(Member {
                            source_address: format!(
                                "AuthoringLane:{}>{}",
                                section.road_section_key(),
                                lane.authoring_lane_key()
                            ),
                            local_key: lane.authoring_lane_key(),
                            width: lane.width_profile(),
                            target: ObservationTarget::LaneEdge {
                                key: lane.lane_edge().local_key().to_owned(),
                                direction: lane.direction(),
                            },
                        });
                    }
                }
                RoadEditingCorridorElement::FacilityBand(reference) => {
                    let band = bands
                        .get(reference.local_key())
                        .ok_or_else(|| contract("corridor facility band is missing"))?;
                    members.push(Member {
                        source_address: format!(
                            "FacilityBand:{}>{}",
                            corridor.road_corridor_key(),
                            band.facility_band_key()
                        ),
                        local_key: band.facility_band_key(),
                        width: band.width_profile(),
                        target: ObservationTarget::FacilityBand {
                            key: band.facility_band_key().to_owned(),
                        },
                    });
                }
            }
        }
        let reference_ordinal = members
            .iter()
            .position(|member| member.local_key == corridor.reference_lane().local_key())
            .ok_or_else(|| contract("corridor reference lane is missing from members"))?;
        let offsets = member_offsets(
            &members
                .iter()
                .map(|member| member.width)
                .collect::<Vec<_>>(),
            reference_ordinal,
        )?;
        let segments = program_segments(alignment.reference_line())?;
        for (member, (offset_start, offset_end)) in members.into_iter().zip(offsets) {
            let intervals = offset_intervals(
                &segments,
                &station_rows,
                corridor.start_station_meters(),
                station_end,
                offset_start,
                offset_end,
            )?;
            offset_tasks.push(ObservationTask {
                module: namespace.clone(),
                source_address: member.source_address,
                target: member.target,
                accuracy,
                direction,
                intervals,
            });
        }
    }
    offset_tasks.sort_by(|left, right| left.source_address.cmp(&right.source_address));
    tasks.extend(offset_tasks);

    let mut explicit_tasks = declarations
        .iter()
        .filter_map(|declaration| match declaration {
            RoadEditingDeclaration::LaneEdge(edge) => edge.explicit_geometry().map(|geometry| {
                Ok(ObservationTask {
                    module: namespace.clone(),
                    source_address: format!("LaneEdge:{}", edge.lane_edge_key()),
                    target: ObservationTarget::LaneEdge {
                        key: edge.lane_edge_key().to_owned(),
                        direction: RoadEditingLaneDirection::Forward,
                    },
                    accuracy,
                    direction,
                    intervals: reference_intervals(geometry)?,
                })
            }),
            _ => None,
        })
        .collect::<Result<Vec<_>, GeneratorError>>()?;
    explicit_tasks.sort_by(|left, right| left.source_address.cmp(&right.source_address));
    tasks.extend(explicit_tasks);
    Ok(())
}

struct Member<'a> {
    source_address: String,
    local_key: &'a str,
    width: LinearWidthProfile,
    target: ObservationTarget,
}

fn reference_intervals(
    program: &RoadEditingCurveProgram,
) -> Result<Vec<OracleInterval>, GeneratorError> {
    program_segments(program).map(|segments| {
        segments
            .into_iter()
            .enumerate()
            .map(|(ordinal, segment)| OracleInterval {
                evaluator: Evaluator::Reference(segment),
                parameter_start: 0.0,
                parameter_end: 1.0,
                source_segment_ordinal: usize_u32(ordinal),
                station_row_ordinal: None,
                source_boundary: ordinal != 0,
            })
            .collect()
    })
}

fn program_segments(program: &RoadEditingCurveProgram) -> Result<Vec<Segment>, GeneratorError> {
    let mut start = point(program.start());
    let mut segments = Vec::with_capacity(program.segments().len());
    for source in program.segments() {
        let (segment, end) = match source.geometry() {
            RoadEditingCurveSegmentGeometry::Line { end } => {
                let end = point(end);
                (Segment::Line { start, end }, end)
            }
            RoadEditingCurveSegmentGeometry::CubicBezier {
                control_1,
                control_2,
                end,
            } => {
                let control_1 = point(control_1);
                let control_2 = point(control_2);
                let end = point(end);
                (
                    Segment::Cubic {
                        start,
                        control_1,
                        control_2,
                        end,
                    },
                    end,
                )
            }
        };
        segments.push(segment);
        start = end;
    }
    if segments.is_empty() {
        return Err(contract("curve program has no segments"));
    }
    Ok(segments)
}

fn station_rows(program: &RoadEditingCurveProgram) -> Result<Vec<StationRow>, GeneratorError> {
    let segments = program_segments(program)?;
    let mut rows = Vec::new();
    let mut cumulative = 0.0;
    for (segment_ordinal, segment) in segments.into_iter().enumerate() {
        let vertices = approximate(
            Evaluator::Reference(segment),
            0.0,
            1.0,
            None,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
        )?;
        let mut previous_parameter = vertices[0].parameter;
        let mut previous_point = segment.evaluate(previous_parameter)?.point;
        for vertex in vertices.iter().skip(1) {
            let current = segment.evaluate(vertex.parameter)?.point;
            let length = distance(previous_point, current)?;
            if length == 0.0 {
                return Err(contract("station row is degenerate"));
            }
            let end = finite(cumulative + length)?;
            rows.push(StationRow {
                segment_ordinal: usize_u32(segment_ordinal),
                parameter_start: previous_parameter,
                parameter_end: vertex.parameter,
                cumulative_start_meters: cumulative,
                cumulative_end_meters: end,
            });
            cumulative = end;
            previous_parameter = vertex.parameter;
            previous_point = current;
        }
    }
    Ok(rows)
}

fn offset_intervals(
    segments: &[Segment],
    rows: &[StationRow],
    corridor_start: f64,
    corridor_end: f64,
    offset_start: f64,
    offset_end: f64,
) -> Result<Vec<OracleInterval>, GeneratorError> {
    let mut intervals = Vec::new();
    let mut previous_segment = None;
    for (row_ordinal, row) in rows.iter().enumerate() {
        let station_start = row.cumulative_start_meters.max(corridor_start);
        let station_end = row.cumulative_end_meters.min(corridor_end);
        if station_start >= station_end {
            continue;
        }
        let parameter_start = parameter_in_row(*row, station_start)?;
        let parameter_end = parameter_in_row(*row, station_end)?;
        let source_boundary = previous_segment.is_some_and(|value| value != row.segment_ordinal);
        intervals.push(OracleInterval {
            evaluator: Evaluator::Offset {
                segment: *segments
                    .get(row.segment_ordinal as usize)
                    .ok_or_else(|| contract("station row segment is missing"))?,
                station: StationInterval {
                    parameter_start: row.parameter_start,
                    parameter_end: row.parameter_end,
                    cumulative_start_meters: row.cumulative_start_meters,
                    cumulative_end_meters: row.cumulative_end_meters,
                },
                offset: OffsetInterval {
                    station_start_meters: corridor_start,
                    station_end_meters: corridor_end,
                    offset_start_meters: offset_start,
                    offset_end_meters: offset_end,
                },
            },
            parameter_start,
            parameter_end,
            source_segment_ordinal: row.segment_ordinal,
            station_row_ordinal: Some(usize_u32(row_ordinal)),
            source_boundary,
        });
        previous_segment = Some(row.segment_ordinal);
    }
    if intervals.is_empty() {
        return Err(contract("offset evaluator has no intervals"));
    }
    Ok(intervals)
}

fn member_offsets(
    profiles: &[LinearWidthProfile],
    reference: usize,
) -> Result<Vec<(f64, f64)>, GeneratorError> {
    if profiles.is_empty() || reference >= profiles.len() {
        return Err(contract("invalid corridor reference member"));
    }
    let mut offsets = vec![(0.0, 0.0); profiles.len()];
    let mut left_start = 0.5 * profiles[reference].start_width_meters();
    let mut left_end = 0.5 * profiles[reference].end_width_meters();
    for ordinal in (0..reference).rev() {
        let width = profiles[ordinal];
        left_start = finite(left_start + 0.5 * width.start_width_meters())?;
        left_end = finite(left_end + 0.5 * width.end_width_meters())?;
        offsets[ordinal] = (left_start, left_end);
        left_start = finite(left_start + 0.5 * width.start_width_meters())?;
        left_end = finite(left_end + 0.5 * width.end_width_meters())?;
    }
    let mut right_start = -(0.5 * profiles[reference].start_width_meters());
    let mut right_end = -(0.5 * profiles[reference].end_width_meters());
    for ordinal in (reference + 1)..profiles.len() {
        let width = profiles[ordinal];
        right_start = finite(right_start - 0.5 * width.start_width_meters())?;
        right_end = finite(right_end - 0.5 * width.end_width_meters())?;
        offsets[ordinal] = (right_start, right_end);
        right_start = finite(right_start - 0.5 * width.start_width_meters())?;
        right_end = finite(right_end - 0.5 * width.end_width_meters())?;
    }
    Ok(offsets)
}

fn approximate_task(
    task: &ObservationTask,
) -> Result<(Vec<Vertex>, Vec<Vec<Vertex>>), GeneratorError> {
    let mut combined = Vec::new();
    let mut intervals = Vec::with_capacity(task.intervals.len());
    let mut previous = None;
    for interval in &task.intervals {
        let welded = if interval.source_boundary {
            let previous =
                previous.ok_or_else(|| contract("source boundary has no prior point"))?;
            let actual = quantize(interval.evaluator.evaluate(interval.parameter_start)?.point);
            if f32_distance(previous, actual)? > MAX_SOURCE_JOIN_GAP_METERS {
                return Err(contract("independent source join exceeds five millimeters"));
            }
            Some(previous)
        } else {
            None
        };
        let vertices = approximate(
            interval.evaluator,
            interval.parameter_start,
            interval.parameter_end,
            welded,
            task.accuracy,
            task.direction,
        )?;
        if combined.is_empty() {
            combined.extend(vertices.iter().copied());
        } else {
            if combined.last().map(|value| value.point) != vertices.first().map(|value| value.point)
            {
                return Err(contract(
                    "adjacent evaluator intervals do not share a canonical point",
                ));
            }
            combined.extend(vertices.iter().skip(1).copied());
        }
        previous = combined.last().map(|vertex| vertex.point);
        intervals.push(vertices);
    }
    Ok((combined, intervals))
}

fn approximate(
    evaluator: Evaluator,
    parameter_start: f64,
    parameter_end: f64,
    welded_start: Option<F32Point>,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
) -> Result<Vec<Vertex>, GeneratorError> {
    let mut start = Endpoint::evaluate(evaluator, parameter_start)?;
    if let Some(welded) = welded_start {
        start.point = welded;
    }
    let end = Endpoint::evaluate(evaluator, parameter_end)?;
    let mut output = vec![Vertex {
        parameter: parameter_start,
        point: start.point,
    }];
    let mut stack = vec![Node {
        parameter_start,
        parameter_end,
        start,
        end,
        depth: 0,
    }];
    while let Some(node) = stack.pop() {
        if candidate_accepts(evaluator, node, accuracy, direction)? {
            output.push(Vertex {
                parameter: node.parameter_end,
                point: node.end.point,
            });
            continue;
        }
        if node.depth == MAX_DEPTH {
            return Err(contract("independent approximation did not converge"));
        }
        let mid = finite(node.parameter_start + (node.parameter_end - node.parameter_start) / 2.0)?;
        let midpoint = Endpoint::evaluate(evaluator, mid)?;
        let depth = node.depth + 1;
        stack.push(Node {
            parameter_start: mid,
            parameter_end: node.parameter_end,
            start: midpoint,
            end: node.end,
            depth,
        });
        stack.push(Node {
            parameter_start: node.parameter_start,
            parameter_end: mid,
            start: node.start,
            end: midpoint,
            depth,
        });
    }
    Ok(output)
}

#[derive(Clone, Copy)]
struct Endpoint {
    point: F32Point,
    first: Point,
}

impl Endpoint {
    fn evaluate(evaluator: Evaluator, parameter: f64) -> Result<Self, GeneratorError> {
        let sample = evaluator.evaluate(parameter)?;
        Ok(Self {
            point: quantize(sample.point),
            first: sample.first,
        })
    }
}

#[derive(Clone, Copy)]
struct Node {
    parameter_start: f64,
    parameter_end: f64,
    start: Endpoint,
    end: Endpoint,
    depth: u8,
}

fn candidate_accepts(
    evaluator: Evaluator,
    node: Node,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
) -> Result<bool, GeneratorError> {
    let mid = finite(node.parameter_start + (node.parameter_end - node.parameter_start) / 2.0)?;
    let q1 = finite(node.parameter_start + (mid - node.parameter_start) / 2.0)?;
    let q3 = finite(mid + (node.parameter_end - mid) / 2.0)?;
    let start = node.start.point.promote();
    let end = node.end.point.promote();
    let chord = end.sub(start)?;
    let target = position_target(accuracy);
    let target_squared = finite(target * target)?;
    for (parameter, chord_parameter) in [(q1, 0.25), (mid, 0.5), (q3, 0.75)] {
        let sample = evaluator.evaluate(parameter)?.point;
        let chord_point = start.lerp(end, chord_parameter)?;
        if sample.sub(chord_point)?.norm_squared()? > target_squared {
            return Ok(false);
        }
    }
    let cosine_squared = half_angle_cosine_squared(direction);
    Ok(direction_accepts(node.start.first, chord, cosine_squared)?
        && direction_accepts(chord, node.end.first, cosine_squared)?)
}

impl Evaluator {
    fn evaluate(self, parameter: f64) -> Result<Sample, GeneratorError> {
        match self {
            Self::Reference(segment) => segment.evaluate(parameter),
            Self::Offset {
                segment,
                station,
                offset,
            } => segment.evaluate_offset(parameter, station, offset),
        }
    }
}

impl Segment {
    fn evaluate(self, parameter: f64) -> Result<Sample, GeneratorError> {
        self.evaluate_dual(Dual::parameter(parameter)?)?.sample()
    }

    fn evaluate_offset(
        self,
        parameter: f64,
        station: StationInterval,
        offset: OffsetInterval,
    ) -> Result<Sample, GeneratorError> {
        let parameter = Dual::parameter(parameter)?;
        let base = self.evaluate_dual(parameter)?;
        let (hx, hz) = self.horizontal_derivative(parameter)?;
        let norm = hx.mul(hx)?.add(hz.mul(hz)?)?.sqrt()?;
        let left = DualPoint {
            x: hz.div(norm)?,
            y: Dual::constant(0.0)?,
            z: hx.neg()?.div(norm)?,
        };
        let t0 = Dual::constant(station.parameter_start)?;
        let t1 = Dual::constant(station.parameter_end)?;
        let local = parameter.sub(t0)?.div(t1.sub(t0)?)?;
        let station_value = Dual::constant(station.cumulative_start_meters)?.add(
            Dual::constant(station.cumulative_end_meters)?
                .sub(Dual::constant(station.cumulative_start_meters)?)?
                .mul(local)?,
        )?;
        let offset_parameter = station_value
            .sub(Dual::constant(offset.station_start_meters)?)?
            .div(
                Dual::constant(offset.station_end_meters)?
                    .sub(Dual::constant(offset.station_start_meters)?)?,
            )?;
        let offset_value = Dual::constant(offset.offset_start_meters)?.add(
            Dual::constant(offset.offset_end_meters)?
                .sub(Dual::constant(offset.offset_start_meters)?)?
                .mul(offset_parameter)?,
        )?;
        base.add(left.mul(offset_value)?)?.sample()
    }

    fn evaluate_dual(self, parameter: Dual) -> Result<DualPoint, GeneratorError> {
        match self {
            Self::Line { start, end } => {
                DualPoint::constant(start)?.lerp(DualPoint::constant(end)?, parameter)
            }
            Self::Cubic {
                start,
                control_1,
                control_2,
                end,
            } => {
                let p0 = DualPoint::constant(start)?;
                let p1 = DualPoint::constant(control_1)?;
                let p2 = DualPoint::constant(control_2)?;
                let p3 = DualPoint::constant(end)?;
                let a = p0.lerp(p1, parameter)?;
                let b = p1.lerp(p2, parameter)?;
                let c = p2.lerp(p3, parameter)?;
                let d = a.lerp(b, parameter)?;
                let e = b.lerp(c, parameter)?;
                d.lerp(e, parameter)
            }
        }
    }

    fn horizontal_derivative(self, parameter: Dual) -> Result<(Dual, Dual), GeneratorError> {
        match self {
            Self::Line { start, end } => Ok((
                Dual::constant(finite(end.x - start.x)?)?,
                Dual::constant(finite(end.z - start.z)?)?,
            )),
            Self::Cubic {
                start,
                control_1,
                control_2,
                end,
            } => {
                let q0 = derivative(start, control_1)?;
                let q1 = derivative(control_1, control_2)?;
                let q2 = derivative(control_2, end)?;
                let x01 = Dual::constant(q0.x)?.lerp(Dual::constant(q1.x)?, parameter)?;
                let x12 = Dual::constant(q1.x)?.lerp(Dual::constant(q2.x)?, parameter)?;
                let z01 = Dual::constant(q0.z)?.lerp(Dual::constant(q1.z)?, parameter)?;
                let z12 = Dual::constant(q1.z)?.lerp(Dual::constant(q2.z)?, parameter)?;
                Ok((x01.lerp(x12, parameter)?, z01.lerp(z12, parameter)?))
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Dual {
    value: f64,
    first: f64,
}

impl Dual {
    fn constant(value: f64) -> Result<Self, GeneratorError> {
        Ok(Self {
            value: finite(value)?,
            first: 0.0,
        })
    }
    fn parameter(value: f64) -> Result<Self, GeneratorError> {
        Ok(Self {
            value: finite(value)?,
            first: 1.0,
        })
    }
    fn add(self, other: Self) -> Result<Self, GeneratorError> {
        Ok(Self {
            value: finite(self.value + other.value)?,
            first: finite(self.first + other.first)?,
        })
    }
    fn sub(self, other: Self) -> Result<Self, GeneratorError> {
        Ok(Self {
            value: finite(self.value - other.value)?,
            first: finite(self.first - other.first)?,
        })
    }
    fn neg(self) -> Result<Self, GeneratorError> {
        Ok(Self {
            value: finite(-self.value)?,
            first: finite(-self.first)?,
        })
    }
    fn mul(self, other: Self) -> Result<Self, GeneratorError> {
        Ok(Self {
            value: finite(self.value * other.value)?,
            first: finite(finite(self.first * other.value)? + finite(self.value * other.first)?)?,
        })
    }
    fn div(self, other: Self) -> Result<Self, GeneratorError> {
        if other.value == 0.0 {
            return Err(contract("dual division by zero"));
        }
        let value = finite(self.value / other.value)?;
        let numerator =
            finite(finite(self.first * other.value)? - finite(self.value * other.first)?)?;
        let denominator = finite(other.value * other.value)?;
        if denominator == 0.0 {
            return Err(contract("dual denominator underflowed to zero"));
        }
        Ok(Self {
            value,
            first: finite(numerator / denominator)?,
        })
    }
    fn sqrt(self) -> Result<Self, GeneratorError> {
        if self.value <= 0.0 {
            return Err(contract("dual sqrt domain error"));
        }
        let value = finite(self.value.sqrt())?;
        Ok(Self {
            value,
            first: finite(self.first / finite(2.0 * value)?)?,
        })
    }
    fn lerp(self, other: Self, parameter: Self) -> Result<Self, GeneratorError> {
        self.add(parameter.mul(other.sub(self)?)?)
    }
}

#[derive(Clone, Copy)]
struct DualPoint {
    x: Dual,
    y: Dual,
    z: Dual,
}

impl DualPoint {
    fn constant(point: Point) -> Result<Self, GeneratorError> {
        Ok(Self {
            x: Dual::constant(point.x)?,
            y: Dual::constant(point.y)?,
            z: Dual::constant(point.z)?,
        })
    }
    fn add(self, other: Self) -> Result<Self, GeneratorError> {
        Ok(Self {
            x: self.x.add(other.x)?,
            y: self.y.add(other.y)?,
            z: self.z.add(other.z)?,
        })
    }
    fn mul(self, scalar: Dual) -> Result<Self, GeneratorError> {
        Ok(Self {
            x: self.x.mul(scalar)?,
            y: self.y.mul(scalar)?,
            z: self.z.mul(scalar)?,
        })
    }
    fn lerp(self, other: Self, parameter: Dual) -> Result<Self, GeneratorError> {
        Ok(Self {
            x: self.x.lerp(other.x, parameter)?,
            y: self.y.lerp(other.y, parameter)?,
            z: self.z.lerp(other.z, parameter)?,
        })
    }
    fn sample(self) -> Result<Sample, GeneratorError> {
        Ok(Sample {
            point: Point {
                x: finite(self.x.value)?,
                y: finite(self.y.value)?,
                z: finite(self.z.value)?,
            },
            first: Point {
                x: finite(self.x.first)?,
                y: finite(self.y.first)?,
                z: finite(self.z.first)?,
            },
        })
    }
}

impl Point {
    fn sub(self, other: Self) -> Result<Self, GeneratorError> {
        Ok(Self {
            x: finite(self.x - other.x)?,
            y: finite(self.y - other.y)?,
            z: finite(self.z - other.z)?,
        })
    }
    fn add(self, other: Self) -> Result<Self, GeneratorError> {
        Ok(Self {
            x: finite(self.x + other.x)?,
            y: finite(self.y + other.y)?,
            z: finite(self.z + other.z)?,
        })
    }
    fn scale(self, value: f64) -> Result<Self, GeneratorError> {
        Ok(Self {
            x: finite(self.x * value)?,
            y: finite(self.y * value)?,
            z: finite(self.z * value)?,
        })
    }
    fn lerp(self, other: Self, parameter: f64) -> Result<Self, GeneratorError> {
        self.add(other.sub(self)?.scale(parameter)?)
    }
    fn norm_squared(self) -> Result<f64, GeneratorError> {
        finite(
            finite(finite(self.x * self.x)? + finite(self.y * self.y)?)? + finite(self.z * self.z)?,
        )
    }
}

impl F32Point {
    fn promote(self) -> Point {
        Point {
            x: f64::from(f32::from_bits(self.x)),
            y: f64::from(f32::from_bits(self.y)),
            z: f64::from(f32::from_bits(self.z)),
        }
    }
}

#[allow(
    clippy::manual_clamp,
    reason = "the workload freezes the r<=0 / r>=1 branch graph and forbids replacing it"
)]
fn distance_to_parameter_segment(
    point: Point,
    parameter: f64,
    vertices: &[Vertex],
) -> Result<f64, GeneratorError> {
    let segment_index = vertices
        .windows(2)
        .position(|pair| pair[1].parameter >= parameter)
        .ok_or_else(|| contract("parameter has no associated final segment"))?;
    let q0 = vertices[segment_index].point.promote();
    let q1 = vertices[segment_index + 1].point.promote();
    let v = q1.sub(q0)?;
    let w = point.sub(q0)?;
    let vv = v.norm_squared()?;
    let ratio = if vv == 0.0 {
        0.0
    } else {
        finite(dot(w, v)? / vv)?
    };
    let u = if ratio <= 0.0 {
        0.0
    } else if ratio >= 1.0 {
        1.0
    } else {
        ratio
    };
    let closest = q0.add(v.scale(u)?)?;
    finite(point.sub(closest)?.norm_squared()?.sqrt())
}

fn grid_parameter(start: f64, end: f64, k: u32) -> f64 {
    if k == 4096 {
        end
    } else {
        start + (end - start) * (f64::from(k) / GRID_DENOMINATOR)
    }
}

fn parameter_in_row(row: StationRow, station: f64) -> Result<f64, GeneratorError> {
    let fraction = finite(
        finite(station - row.cumulative_start_meters)?
            / finite(row.cumulative_end_meters - row.cumulative_start_meters)?,
    )?;
    finite(
        row.parameter_start + finite(finite(row.parameter_end - row.parameter_start)? * fraction)?,
    )
}

fn require_final_points(
    module: &str,
    key: &str,
    expected: &[F32Point],
    actual: Option<&Vec<F32Point>>,
) -> Result<(), GeneratorError> {
    let actual = actual.ok_or_else(|| contract(format!("missing LIR geometry {module}::{key}")))?;
    if expected != actual {
        return Err(contract(format!(
            "independent oracle differs from LIR geometry {module}::{key}: expected={} actual={}",
            expected.len(),
            actual.len()
        )));
    }
    Ok(())
}

fn lane_points(
    output: &CompilationOutput,
) -> Result<BTreeMap<(String, String), Vec<F32Point>>, GeneratorError> {
    let mut map = BTreeMap::new();
    for edge in output.lir().lane_edges() {
        let namespace = identity_text(edge.identity_fields(), MODULE_NAMESPACE_TAG)?;
        let key = identity_text(edge.identity_fields(), LANE_EDGE_KEY_TAG)?;
        let geometry = edge
            .spatial_geometry()
            .ok_or_else(|| contract("P100 lane edge is missing spatial geometry"))?;
        let points = geometry.points().map(f32_point).collect::<Vec<_>>();
        if map.insert((namespace, key), points).is_some() {
            return Err(contract("duplicate LIR lane edge identity"));
        }
    }
    Ok(map)
}

fn facility_points(
    output: &CompilationOutput,
) -> Result<BTreeMap<(String, String), Vec<F32Point>>, GeneratorError> {
    let mut map = BTreeMap::new();
    for band in output.lir().facility_bands() {
        let namespace = identity_text(band.identity_fields(), MODULE_NAMESPACE_TAG)?;
        let key = identity_text(band.identity_fields(), FACILITY_BAND_KEY_TAG)?;
        let geometry = band
            .spatial_geometry()
            .ok_or_else(|| contract("P100 facility band is missing spatial geometry"))?;
        let points = geometry.points().map(f32_point).collect::<Vec<_>>();
        if map.insert((namespace, key), points).is_some() {
            return Err(contract("duplicate LIR facility band identity"));
        }
    }
    Ok(map)
}

fn identity_text<'a>(
    fields: impl Iterator<Item = CanonicalIdentityFieldView<'a>>,
    tag: u16,
) -> Result<String, GeneratorError> {
    let matches = fields
        .filter(|field| field.tag().code() == tag)
        .map(|field| std::str::from_utf8(field.value_bytes()).map(str::to_owned))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| contract("LIR identity field is not UTF-8"))?;
    let [value] = matches.as_slice() else {
        return Err(contract("LIR identity field is missing or repeated"));
    };
    Ok(value.clone())
}

fn max_direction_jump_degrees<'a>(
    polylines: impl Iterator<Item = &'a Vec<F32Point>>,
) -> Result<f64, GeneratorError> {
    let mut maximum = 0.0_f64;
    for points in polylines {
        let chords = points
            .windows(2)
            .map(|pair| pair[1].promote().sub(pair[0].promote()))
            .collect::<Result<Vec<_>, _>>()?;
        for pair in chords.windows(2) {
            let cross = Point {
                x: finite(pair[0].y * pair[1].z - pair[0].z * pair[1].y)?,
                y: finite(pair[0].z * pair[1].x - pair[0].x * pair[1].z)?,
                z: finite(pair[0].x * pair[1].y - pair[0].y * pair[1].x)?,
            };
            let angle = finite(cross.norm_squared()?.sqrt().atan2(dot(pair[0], pair[1])?))?;
            maximum = maximum.max(finite(angle * (180.0 / std::f64::consts::PI))?);
        }
    }
    Ok(maximum)
}

fn direction_accepts(
    left: Point,
    right: Point,
    cosine_squared: f64,
) -> Result<bool, GeneratorError> {
    let dot = dot(left, right)?;
    let lhs = finite(dot * dot)?;
    let rhs = finite(finite(cosine_squared * left.norm_squared()?)? * right.norm_squared()?)?;
    Ok(dot > 0.0 && lhs >= rhs)
}

fn dot(left: Point, right: Point) -> Result<f64, GeneratorError> {
    finite(
        finite(finite(left.x * right.x)? + finite(left.y * right.y)?)? + finite(left.z * right.z)?,
    )
}

fn derivative(start: Point, end: Point) -> Result<Point, GeneratorError> {
    Ok(Point {
        x: finite(3.0 * finite(end.x - start.x)?)?,
        y: finite(3.0 * finite(end.y - start.y)?)?,
        z: finite(3.0 * finite(end.z - start.z)?)?,
    })
}

fn distance(left: Point, right: Point) -> Result<f64, GeneratorError> {
    finite(left.sub(right)?.norm_squared()?.sqrt())
}

fn f32_distance(left: F32Point, right: F32Point) -> Result<f64, GeneratorError> {
    distance(left.promote(), right.promote())
}

fn quantize(point: Point) -> F32Point {
    F32Point {
        x: canonical_f32(point.x as f32).to_bits(),
        y: canonical_f32(point.y as f32).to_bits(),
        z: canonical_f32(point.z as f32).to_bits(),
    }
}

fn f32_point(point: CanonicalPoint3F32) -> F32Point {
    F32Point {
        x: point.x.to_bits(),
        y: point.y.to_bits(),
        z: point.z.to_bits(),
    }
}

fn canonical_f32(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}

fn point(value: RoadEditingPoint3) -> Point {
    Point {
        x: value.x(),
        y: value.y(),
        z: value.z(),
    }
}

fn position_target(profile: GeometryAccuracyProfile) -> f64 {
    f64::from_bits(match profile {
        GeometryAccuracyProfile::Fine2Cm => 0x3f84_7ae1_47ae_147b,
        GeometryAccuracyProfile::Balanced5Cm => 0x3f99_9999_9999_999a,
        GeometryAccuracyProfile::Compact10Cm => 0x3fa9_9999_9999_999a,
    })
}

fn half_angle_cosine_squared(profile: GeometryDirectionProfile) -> f64 {
    f64::from_bits(match profile {
        GeometryDirectionProfile::Smooth1Deg => 0x3fef_ff60_4bfa_d7c5,
        GeometryDirectionProfile::Balanced2Deg => 0x3fef_fd81_3c5f_82b4,
        GeometryDirectionProfile::Compact5Deg => 0x3fef_f069_da0c_0ad2,
    })
}

fn percentile(values: &[f64], percentile: usize) -> Result<f64, GeneratorError> {
    let numerator = percentile
        .checked_mul(values.len())
        .and_then(|value| value.checked_add(99))
        .ok_or_else(|| contract("percentile rank overflow"))?;
    let rank = numerator
        .checked_div(100)
        .and_then(|value| value.checked_sub(1))
        .ok_or_else(|| contract("percentile rank underflow"))?;
    values
        .get(rank)
        .copied()
        .ok_or_else(|| contract("percentile rank is out of range"))
}

fn finite(value: f64) -> Result<f64, GeneratorError> {
    if value.is_finite() {
        Ok(if value == 0.0 { 0.0 } else { value })
    } else {
        Err(contract("non-finite independent geometry observation"))
    }
}

fn contract(message: impl Into<String>) -> GeneratorError {
    GeneratorError::Contract(message.into())
}

fn usize_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn usize_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn bits(value: f64) -> String {
    format!("0x{:016x}", value.to_bits())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{build_base_modules, compile_encoded_modules, encode_modules};
    use laneflow_compiler::CompileLimits;
    use std::path::Path;

    #[test]
    fn independent_observer_matches_base_production_lir() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap();
        let limits = CompileLimits::p100_initial_v2();
        let modules = build_base_modules(
            root,
            GeometryAccuracyProfile::Compact10Cm,
            GeometryDirectionProfile::Compact5Deg,
            &limits,
        )
        .unwrap();
        let plan = prepare_observation(&modules).unwrap();
        let encoded = encode_modules(modules, &limits).unwrap();
        let output = compile_encoded_modules(&encoded, limits).unwrap();
        let observation = observe(&plan, &output).unwrap();

        assert_eq!(
            observation.observed_sample_count,
            observation.evaluator_interval_count * GRID_POINT_COUNT
        );
        assert!(!observation.evaluator_interval_identity_sha256.is_empty());
        for value in [
            &observation.position_error.p50_meters_bits,
            &observation.position_error.p95_meters_bits,
            &observation.position_error.p99_meters_bits,
            &observation.position_error.maximum_meters_bits,
            &observation.worst_observed_error.parameter_bits,
            &observation.final_f32_direction_jump_maximum_degrees_bits,
        ] {
            assert_eq!(value.len(), 18);
            assert!(value.starts_with("0x"));
        }
    }
}
