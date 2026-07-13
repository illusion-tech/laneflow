//! Tick-local occupancy scratch 与 leader observation 原语。

use crate::{EdgeHandle, VehicleHandle};

/// 单个 physical edge 上的车辆占用记录。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Occupant {
    pub(crate) vehicle: VehicleHandle,
    pub(crate) front_progress: f64,
    pub(crate) vehicle_length: f64,
    pub(crate) update_sequence: u64,
}

/// 沿 follower route 解析出的最近 leader。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LeaderObservation {
    pub(crate) leader: VehicleHandle,
    pub(crate) bumper_gap: f64,
}

/// 可跨 tick 复用、但不属于 Core authority state 的派生 scratch。
#[derive(Clone, Debug, Default)]
pub(crate) struct OccupancyScratch {
    counts: Vec<usize>,
    offsets: Vec<usize>,
    write_positions: Vec<usize>,
    occupants: Vec<Occupant>,
    leaders: Vec<Option<LeaderObservation>>,
    max_vehicle_length: f64,
}

impl PartialEq for OccupancyScratch {
    fn eq(&self, _other: &Self) -> bool {
        // Scratch 的内容和 capacity 取决于查询历史，不参与 CoreWorld 语义相等。
        true
    }
}

impl OccupancyScratch {
    pub(crate) fn begin(&mut self, edge_count: usize, vehicle_slot_count: usize) {
        self.counts.clear();
        self.counts.resize(edge_count, 0);
        self.offsets.clear();
        self.offsets.resize(edge_count + 1, 0);
        self.write_positions.clear();
        self.write_positions.resize(edge_count, 0);
        self.occupants.clear();
        self.leaders.clear();
        self.leaders.resize(vehicle_slot_count, None);
        self.max_vehicle_length = 0.0;
    }

    pub(crate) fn count(&mut self, edge: EdgeHandle, vehicle_length: f64) {
        self.counts[edge.index()] += 1;
        self.max_vehicle_length = self.max_vehicle_length.max(vehicle_length);
    }

    pub(crate) fn allocate_occupants(&mut self) {
        let mut total = 0;
        for (index, count) in self.counts.iter().copied().enumerate() {
            self.offsets[index] = total;
            self.write_positions[index] = total;
            total += count;
        }
        self.offsets[self.counts.len()] = total;
        self.occupants.resize(
            total,
            Occupant {
                vehicle: VehicleHandle::new(0, 0),
                front_progress: 0.0,
                vehicle_length: 0.0,
                update_sequence: 0,
            },
        );
    }

    pub(crate) fn insert(&mut self, edge: EdgeHandle, occupant: Occupant) {
        let cursor = &mut self.write_positions[edge.index()];
        self.occupants[*cursor] = occupant;
        *cursor += 1;
    }

    pub(crate) fn sort_edges(&mut self) {
        for edge_index in 0..self.counts.len() {
            let start = self.offsets[edge_index];
            let end = self.offsets[edge_index + 1];
            self.occupants[start..end].sort_unstable_by(|left, right| {
                left.front_progress
                    .total_cmp(&right.front_progress)
                    .then_with(|| left.update_sequence.cmp(&right.update_sequence))
            });
        }
    }

    pub(crate) fn edge(&self, edge: EdgeHandle) -> &[Occupant] {
        let start = self.offsets[edge.index()];
        let end = self.offsets[edge.index() + 1];
        &self.occupants[start..end]
    }

    pub(crate) fn max_vehicle_length(&self) -> f64 {
        self.max_vehicle_length
    }

    pub(crate) fn occupant_count(&self) -> usize {
        self.occupants.len()
    }

    pub(crate) fn set_leader(
        &mut self,
        vehicle: VehicleHandle,
        observation: Option<LeaderObservation>,
    ) {
        self.leaders[vehicle.index()] = observation;
    }

    pub(crate) fn leader(&self, vehicle: VehicleHandle) -> Option<LeaderObservation> {
        self.leaders.get(vehicle.index()).copied().flatten()
    }
}
