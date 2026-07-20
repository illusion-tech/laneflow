#![allow(
    dead_code,
    reason = "shared property, allocation, performance, and Criterion fixture"
)]

use std::{collections::HashMap, mem};

use laneflow_core::{
    CoreWorld, EdgeHandle, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge,
    LaneGraph, ParkingRegistry, Route, SignalRegistry, Speed, VehicleHandle, VehicleProfile,
    VehicleProfileRegistry, VehicleSpawnInput,
};
use laneflow_spatial::{
    CanonicalFrameId, CanonicalPoint3F32, CanonicalPoseBatchF32, CanonicalPoseBatchScratch,
    FramePlacementToken, PoseInputRecord, PoseSource, SpatialEdgeInput, SpatialError,
    SpatialRegistry,
};

pub const TEN_THOUSAND: usize = 10_000;
pub const ONE_HUNDRED_THOUSAND: usize = 100_000;

pub struct RuntimeFixture {
    pub spatial: SpatialRegistry,
    pub parking: ParkingRegistry,
    pub edge: EdgeHandle,
    pub vehicle: VehicleHandle,
    pub core_length: f64,
    points: Vec<[f64; 3]>,
}

impl RuntimeFixture {
    pub fn new() -> Self {
        let point_values = [
            [0.0_f32, 0.0, 0.0],
            [250.0, 0.0, 0.0],
            [500.0, 100.0, 0.0],
            [750.0, 100.0, 250.0],
            [1_000.0, 0.0, 250.0],
        ];
        let points: Vec<_> = point_values
            .iter()
            .map(|[x, y, z]| {
                CanonicalPoint3F32::try_new(*x, *y, *z).expect("valid benchmark point")
            })
            .collect();
        let core_length = point_values
            .windows(2)
            .map(|pair| {
                let delta = [
                    pair[1][0] - pair[0][0],
                    pair[1][1] - pair[0][1],
                    pair[1][2] - pair[0][2],
                ];
                delta[0].hypot(delta[1]).hypot(delta[2])
            })
            .sum::<f32>();
        let core_length = f64::from(core_length);

        let graph = LaneGraph::try_new([LaneEdge::new(
            "scale-edge",
            EdgeLength::try_new(core_length).expect("valid benchmark edge length"),
            std::iter::empty::<&str>(),
        )])
        .expect("valid benchmark graph");
        let edge = graph.edge_handle("scale-edge").expect("benchmark edge");
        let parking = ParkingRegistry::empty();

        let profiles = VehicleProfileRegistry::try_new([benchmark_profile()])
            .expect("valid benchmark profiles");
        let profile = profiles
            .profile_handle("scale-profile")
            .expect("benchmark profile");
        let traffic = InitialTrafficData::try_new_with_signals_and_parking(
            graph.clone(),
            [Route::try_new("scale-route", ["scale-edge"]).expect("valid benchmark route")],
            profiles,
            SignalRegistry::empty(),
            parking.clone(),
        )
        .expect("valid benchmark traffic data");
        let world = CoreWorld::with_traffic_data(
            1,
            traffic,
            vec![VehicleSpawnInput::active(
                "scale-vehicle",
                profile,
                "scale-route",
                0,
                EdgeProgress::ZERO,
                Speed::ZERO,
            )],
        )
        .expect("valid benchmark world");
        let vehicle = world
            .vehicle_handle("scale-vehicle")
            .expect("benchmark vehicle");

        let frame_id =
            CanonicalFrameId::try_new("validation/fixed-scale").expect("valid benchmark frame");
        let spatial =
            SpatialRegistry::try_new(&graph, frame_id, [SpatialEdgeInput::new(edge, &points)])
                .expect("valid benchmark spatial registry");

        Self {
            spatial,
            parking,
            edge,
            vehicle,
            core_length,
            points: point_values
                .iter()
                .map(|point| point.map(f64::from))
                .collect(),
        }
    }

    pub fn inputs(&self, count: usize) -> Vec<PoseInputRecord> {
        assert!(count > 1, "scale fixture requires at least two records");
        (0..count)
            .map(|index| {
                // 7919 is coprime with both frozen scales. The permutation visits the full
                // progress range without presenting the sampling branches in sorted order.
                let rank = (index * 7_919) % count;
                let progress = self.core_length * rank as f64 / (count - 1) as f64;
                PoseInputRecord::lane(
                    self.vehicle,
                    self.edge,
                    EdgeProgress::try_new(progress).expect("valid benchmark progress"),
                )
            })
            .collect()
    }

    pub fn output(&self, capacity: usize) -> CanonicalPoseBatchF32 {
        CanonicalPoseBatchF32::with_capacity(
            self.spatial.frame_id().clone(),
            FramePlacementToken::new(0),
            capacity,
        )
    }

    pub fn scratch(&self, capacity: usize) -> CanonicalPoseBatchScratch {
        CanonicalPoseBatchScratch::with_capacity(capacity)
    }

    pub fn f64_candidate(&self) -> F64CandidateRegistry {
        F64CandidateRegistry::new(
            self.spatial.frame_id().clone(),
            self.edge,
            self.core_length,
            &self.points,
        )
    }
}

fn benchmark_profile() -> VehicleProfile {
    VehicleProfile::try_new_iidm(
        "scale-profile",
        IidmProfileSpec {
            length: 4.5,
            desired_speed: 13.9,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 1.4,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 4.0,
        },
    )
    .expect("valid benchmark profile")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct F64Pose {
    pub position: [f64; 3],
    pub tangent: [f64; 3],
    pub up: [f64; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct F64PoseRecord {
    pub vehicle: VehicleHandle,
    pub pose: F64Pose,
}

#[derive(Clone, Debug)]
struct F64Segment {
    length: f64,
    cumulative_end: f64,
    tangent: [f64; 3],
    up: [f64; 3],
}

#[derive(Clone, Debug)]
struct F64Polyline {
    points: Vec<[f64; 3]>,
    segments: Vec<F64Segment>,
    arc_length: f64,
}

impl F64Polyline {
    fn new(points: &[[f64; 3]]) -> Self {
        let mut segments = Vec::with_capacity(points.len() - 1);
        let mut cumulative_end = 0.0;
        for pair in points.windows(2) {
            let delta = sub(pair[1], pair[0]);
            let length = norm(delta);
            let tangent = scale(delta, 1.0 / length);
            let left_length = tangent[0].hypot(tangent[2]);
            let left = [tangent[2] / left_length, 0.0, -tangent[0] / left_length];
            let up = normalize([
                tangent[1] * left[2],
                tangent[2] * left[0] - tangent[0] * left[2],
                -tangent[1] * left[0],
            ]);
            cumulative_end += length;
            segments.push(F64Segment {
                length,
                cumulative_end,
                tangent,
                up,
            });
        }

        Self {
            points: points.to_vec(),
            segments,
            arc_length: cumulative_end,
        }
    }

    fn sample(
        &self,
        edge: EdgeHandle,
        progress: f64,
        core_length: f64,
    ) -> Result<F64Pose, SpatialError> {
        if progress > core_length {
            return Err(SpatialError::ProgressOutOfRange {
                edge,
                progress_meters: progress,
                max_meters: core_length,
            });
        }
        if progress == 0.0 {
            return Ok(self.pose_at(0, 0));
        }
        if progress == core_length {
            return Ok(self.pose_at(self.points.len() - 1, self.segments.len() - 1));
        }

        let geometry_s = progress / core_length * self.arc_length;
        if geometry_s >= self.arc_length {
            return Ok(self.pose_at(self.points.len() - 1, self.segments.len() - 1));
        }
        let segment_index = self
            .segments
            .partition_point(|segment| segment.cumulative_end <= geometry_s);
        let segment = &self.segments[segment_index];
        let cumulative_start = if segment_index == 0 {
            0.0
        } else {
            self.segments[segment_index - 1].cumulative_end
        };
        let ratio = (geometry_s - cumulative_start) / segment.length;
        let start = self.points[segment_index];
        let end = self.points[segment_index + 1];
        let position =
            checked_position(add(start, scale(sub(end, start), ratio))).map_err(|source| {
                SpatialError::SamplePositionComputation {
                    edge,
                    segment_index,
                    source: Box::new(source),
                }
            })?;
        Ok(F64Pose {
            position,
            tangent: segment.tangent,
            up: segment.up,
        })
    }

    fn pose_at(&self, point_index: usize, segment_index: usize) -> F64Pose {
        let segment = &self.segments[segment_index];
        F64Pose {
            position: self.points[point_index],
            tangent: segment.tangent,
            up: segment.up,
        }
    }
}

pub struct F64CandidateRegistry {
    frame_id: CanonicalFrameId,
    core_length: f64,
    edge_handles: Vec<EdgeHandle>,
    edge_slots: HashMap<EdgeHandle, u32>,
    entries: Vec<F64Polyline>,
}

impl F64CandidateRegistry {
    fn new(
        frame_id: CanonicalFrameId,
        edge: EdgeHandle,
        core_length: f64,
        points: &[[f64; 3]],
    ) -> Self {
        let entry = F64Polyline::new(points);
        Self {
            frame_id,
            core_length,
            edge_handles: vec![edge],
            edge_slots: HashMap::from([(edge, 0)]),
            entries: vec![entry],
        }
    }

    pub fn sample(
        &self,
        edge: EdgeHandle,
        progress: EdgeProgress,
    ) -> Result<F64Pose, SpatialError> {
        let slot = self
            .edge_slot(edge)
            .ok_or(SpatialError::UnknownEdgeHandle { edge })?;
        self.sample_resolved(slot, progress)
    }

    pub fn edge_slot(&self, edge: EdgeHandle) -> Option<u32> {
        self.edge_slots.get(&edge).copied()
    }

    pub fn sample_resolved(
        &self,
        slot: u32,
        progress: EdgeProgress,
    ) -> Result<F64Pose, SpatialError> {
        self.entries[slot as usize].sample(
            self.edge_handles[slot as usize],
            progress.value(),
            self.core_length,
        )
    }

    pub fn extract(
        &self,
        placement_token: FramePlacementToken,
        inputs: &[PoseInputRecord],
        output: &mut F64Batch,
        scratch: &mut F64Scratch,
    ) -> Result<(), SpatialError> {
        scratch.records.clear();
        if output.frame_id != self.frame_id {
            return Err(SpatialError::BatchFrameMismatch {
                registry_frame_id: self.frame_id.as_str().to_owned(),
                output_frame_id: output.frame_id.as_str().to_owned(),
            });
        }
        for (input_index, input) in inputs.iter().copied().enumerate() {
            let pose = match input.source() {
                PoseSource::Lane { edge, progress } => self.sample(edge, progress),
                PoseSource::Parking { space } => {
                    Err(SpatialError::UnknownParkingSpaceHandle { space })
                }
                _ => unreachable!("the frozen scale fixture has a known pose source"),
            };
            let pose = match pose.map_err(|source| SpatialError::PoseRecordFailed {
                input_index,
                vehicle: input.vehicle(),
                source: Box::new(source),
            }) {
                Ok(pose) => pose,
                Err(error) => {
                    scratch.records.clear();
                    return Err(error);
                }
            };
            scratch.records.push(F64PoseRecord {
                vehicle: input.vehicle(),
                pose,
            });
        }
        mem::swap(&mut output.records, &mut scratch.records);
        scratch.records.clear();
        output.placement_token = placement_token;
        Ok(())
    }
}

pub struct F64Batch {
    frame_id: CanonicalFrameId,
    placement_token: FramePlacementToken,
    records: Vec<F64PoseRecord>,
}

impl F64Batch {
    pub fn with_capacity(
        frame_id: CanonicalFrameId,
        placement_token: FramePlacementToken,
        capacity: usize,
    ) -> Self {
        Self {
            frame_id,
            placement_token,
            records: Vec::with_capacity(capacity),
        }
    }

    pub fn records(&self) -> &[F64PoseRecord] {
        &self.records
    }

    pub fn capacity(&self) -> usize {
        self.records.capacity()
    }
}

pub struct F64Scratch {
    records: Vec<F64PoseRecord>,
}

impl F64Scratch {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            records: Vec::with_capacity(capacity),
        }
    }

    pub fn capacity(&self) -> usize {
        self.records.capacity()
    }
}

fn add(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn sub(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn scale(vector: [f64; 3], scalar: f64) -> [f64; 3] {
    [vector[0] * scalar, vector[1] * scalar, vector[2] * scalar]
}

fn norm(vector: [f64; 3]) -> f64 {
    vector[0].hypot(vector[1]).hypot(vector[2])
}

fn normalize(vector: [f64; 3]) -> [f64; 3] {
    scale(vector, 1.0 / norm(vector))
}

fn checked_position(mut position: [f64; 3]) -> Result<[f64; 3], SpatialError> {
    for component in &position {
        if !component.is_finite() {
            return Err(SpatialError::ZeroLengthDirection);
        }
    }
    for component in &position {
        if !(-16_384.0..=16_384.0).contains(component) {
            return Err(SpatialError::ZeroLengthDirection);
        }
    }
    for component in &mut position {
        if *component == 0.0 {
            *component = 0.0;
        }
    }
    Ok(position)
}
