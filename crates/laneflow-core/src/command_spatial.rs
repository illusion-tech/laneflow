//! Lifecycle command 使用的 non-authoritative physical-edge local index。

use std::{cmp::Ordering, collections::BinaryHeap};

use crate::{EdgeHandle, LaneGraph, VehicleHandle, profile::VehicleProfileRegistry};

const SPATIAL_CHUNK_TARGET: usize = 64;
const SPATIAL_CHUNK_MAX: usize = SPATIAL_CHUNK_TARGET * 2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CommandOccupant {
    pub(crate) vehicle: VehicleHandle,
    pub(crate) front_progress: f64,
}

fn compare_occupants(left: &CommandOccupant, right: &CommandOccupant) -> std::cmp::Ordering {
    left.front_progress
        .total_cmp(&right.front_progress)
        .then_with(|| left.vehicle.index().cmp(&right.vehicle.index()))
        .then_with(|| left.vehicle.generation().cmp(&right.vehicle.generation()))
}

fn indexed_progress(occupant: CommandOccupant, front_progress: &[f64]) -> f64 {
    front_progress[occupant.vehicle.index()]
}

fn compare_indexed(
    left: CommandOccupant,
    right: CommandOccupant,
    front_progress: &[f64],
) -> std::cmp::Ordering {
    indexed_progress(left, front_progress)
        .total_cmp(&indexed_progress(right, front_progress))
        .then_with(|| left.vehicle.index().cmp(&right.vehicle.index()))
        .then_with(|| left.vehicle.generation().cmp(&right.vehicle.generation()))
}

#[derive(Clone, Copy, Debug)]
struct CommandSpeedEntry {
    speed: f64,
    vehicle: VehicleHandle,
}

impl PartialEq for CommandSpeedEntry {
    fn eq(&self, other: &Self) -> bool {
        self.speed == other.speed && self.vehicle == other.vehicle
    }
}

impl Eq for CommandSpeedEntry {}

impl PartialOrd for CommandSpeedEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CommandSpeedEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.speed
            .total_cmp(&other.speed)
            .then_with(|| self.vehicle.index().cmp(&other.vehicle.index()))
            .then_with(|| self.vehicle.generation().cmp(&other.vehicle.generation()))
    }
}

#[derive(Clone, Debug, Default)]
struct SpatialChunk {
    occupants: Vec<CommandOccupant>,
}

#[derive(Clone, Debug, Default)]
struct SpatialBucket {
    chunks: Vec<SpatialChunk>,
    active_chunks: usize,
}

impl SpatialBucket {
    fn active(&self) -> &[SpatialChunk] {
        &self.chunks[..self.active_chunks]
    }

    fn insertion_chunk_for_new(&self, occupant: CommandOccupant, front_progress: &[f64]) -> usize {
        if self.active_chunks == 0 {
            return 0;
        }
        self.active()
            .partition_point(|chunk| {
                let last = *chunk
                    .occupants
                    .last()
                    .expect("active spatial chunk must not be empty");
                indexed_progress(last, front_progress)
                    .total_cmp(&occupant.front_progress)
                    .then_with(|| last.vehicle.index().cmp(&occupant.vehicle.index()))
                    .then_with(|| {
                        last.vehicle
                            .generation()
                            .cmp(&occupant.vehicle.generation())
                    })
                    .is_lt()
            })
            .min(self.active_chunks - 1)
    }

    fn insertion_chunk_indexed(&self, occupant: CommandOccupant, front_progress: &[f64]) -> usize {
        if self.active_chunks == 0 {
            return 0;
        }
        self.active()
            .partition_point(|chunk| {
                compare_indexed(
                    *chunk
                        .occupants
                        .last()
                        .expect("active spatial chunk must not be empty"),
                    occupant,
                    front_progress,
                )
                .is_lt()
            })
            .min(self.active_chunks - 1)
    }

    fn prepare_insert(&mut self, occupant: CommandOccupant, front_progress: &[f64]) {
        self.chunks.reserve(1);
        if self.active_chunks == self.chunks.len() {
            self.chunks.push(SpatialChunk::default());
        }
        let target = self.insertion_chunk_for_new(occupant, front_progress);
        if self.active_chunks == 0 {
            self.chunks[0].occupants.reserve(SPATIAL_CHUNK_TARGET);
        } else if self.chunks[target].occupants.len() < SPATIAL_CHUNK_MAX {
            self.chunks[target].occupants.reserve(1);
        } else {
            self.chunks[self.active_chunks]
                .occupants
                .reserve(SPATIAL_CHUNK_MAX);
        }
    }

    fn insert(&mut self, occupant: CommandOccupant, front_progress: &[f64]) {
        if self.active_chunks == 0 {
            self.active_chunks = 1;
            self.chunks[0].occupants.push(occupant);
            return;
        }

        let mut target = self.insertion_chunk_indexed(occupant, front_progress);
        if self.chunks[target].occupants.len() >= SPATIAL_CHUNK_MAX {
            self.chunks[target + 1..=self.active_chunks].rotate_right(1);
            self.active_chunks += 1;
            let (left, right) = self.chunks.split_at_mut(target + 1);
            let source = &mut left[target].occupants;
            let destination = &mut right[0].occupants;
            destination.clear();
            destination.extend(source.drain(SPATIAL_CHUNK_TARGET..));
            if compare_indexed(
                *destination
                    .first()
                    .expect("split spatial chunk must have upper half"),
                occupant,
                front_progress,
            )
            .is_le()
            {
                target += 1;
            }
        }

        let chunk = &mut self.chunks[target].occupants;
        let position = chunk
            .binary_search_by(|existing| compare_indexed(*existing, occupant, front_progress))
            .unwrap_or_else(|position| position);
        chunk.insert(position, occupant);
    }

    fn remove(&mut self, occupant: CommandOccupant, front_progress: &[f64]) -> bool {
        let mut chunk_index = self.active().partition_point(|chunk| {
            indexed_progress(
                *chunk
                    .occupants
                    .last()
                    .expect("active spatial chunk must not be empty"),
                front_progress,
            ) < occupant.front_progress
        });
        while chunk_index < self.active_chunks {
            let chunk = &mut self.chunks[chunk_index].occupants;
            if chunk.first().is_some_and(|first| {
                indexed_progress(*first, front_progress) > occupant.front_progress
            }) {
                break;
            }
            let start = chunk.partition_point(|existing| {
                indexed_progress(*existing, front_progress) < occupant.front_progress
            });
            let end = chunk.partition_point(|existing| {
                indexed_progress(*existing, front_progress) <= occupant.front_progress
            });
            if let Some(relative) = chunk[start..end]
                .iter()
                .position(|existing| existing.vehicle == occupant.vehicle)
            {
                chunk.remove(start + relative);
                if chunk.is_empty() {
                    self.chunks[chunk_index..self.active_chunks].rotate_left(1);
                    self.active_chunks -= 1;
                }
                return true;
            }
            chunk_index += 1;
        }
        false
    }

    fn insertion_chunk_resolved<F>(
        &self,
        occupant: CommandOccupant,
        resolve_progress: &mut F,
    ) -> usize
    where
        F: FnMut(VehicleHandle) -> f64,
    {
        if self.active_chunks == 0 {
            return 0;
        }
        self.active()
            .partition_point(|chunk| {
                let last = chunk
                    .occupants
                    .last()
                    .expect("active spatial chunk must not be empty");
                resolve_progress(last.vehicle)
                    .total_cmp(&occupant.front_progress)
                    .then_with(|| last.vehicle.index().cmp(&occupant.vehicle.index()))
                    .then_with(|| {
                        last.vehicle
                            .generation()
                            .cmp(&occupant.vehicle.generation())
                    })
                    .is_lt()
            })
            .min(self.active_chunks - 1)
    }

    fn prepare_insert_resolved<F>(&mut self, occupant: CommandOccupant, resolve_progress: &mut F)
    where
        F: FnMut(VehicleHandle) -> f64,
    {
        self.chunks.reserve(1);
        if self.active_chunks == self.chunks.len() {
            self.chunks.push(SpatialChunk::default());
        }
        let target = self.insertion_chunk_resolved(occupant, resolve_progress);
        if self.active_chunks == 0 {
            self.chunks[0].occupants.reserve(SPATIAL_CHUNK_TARGET);
        } else if self.chunks[target].occupants.len() < SPATIAL_CHUNK_MAX {
            self.chunks[target].occupants.reserve(1);
        } else {
            self.chunks[self.active_chunks]
                .occupants
                .reserve(SPATIAL_CHUNK_MAX);
        }
    }

    fn insert_resolved<F>(&mut self, occupant: CommandOccupant, resolve_progress: &mut F)
    where
        F: FnMut(VehicleHandle) -> f64,
    {
        if self.active_chunks == 0 {
            self.active_chunks = 1;
            self.chunks[0].occupants.push(occupant);
            return;
        }
        let mut target = self.insertion_chunk_resolved(occupant, resolve_progress);
        if self.chunks[target].occupants.len() >= SPATIAL_CHUNK_MAX {
            self.chunks[target + 1..=self.active_chunks].rotate_right(1);
            self.active_chunks += 1;
            let (left, right) = self.chunks.split_at_mut(target + 1);
            let source = &mut left[target].occupants;
            let destination = &mut right[0].occupants;
            destination.clear();
            destination.extend(source.drain(SPATIAL_CHUNK_TARGET..));
            let first = destination
                .first()
                .expect("split spatial chunk must have upper half");
            if resolve_progress(first.vehicle)
                .total_cmp(&occupant.front_progress)
                .then_with(|| first.vehicle.index().cmp(&occupant.vehicle.index()))
                .then_with(|| {
                    first
                        .vehicle
                        .generation()
                        .cmp(&occupant.vehicle.generation())
                })
                .is_le()
            {
                target += 1;
            }
        }

        let chunk = &mut self.chunks[target].occupants;
        let position = chunk
            .binary_search_by(|existing| {
                resolve_progress(existing.vehicle)
                    .total_cmp(&occupant.front_progress)
                    .then_with(|| existing.vehicle.index().cmp(&occupant.vehicle.index()))
                    .then_with(|| {
                        existing
                            .vehicle
                            .generation()
                            .cmp(&occupant.vehicle.generation())
                    })
            })
            .unwrap_or_else(|position| position);
        chunk.insert(position, occupant);
    }

    fn remove_resolved<F>(&mut self, occupant: CommandOccupant, resolve_progress: &mut F) -> bool
    where
        F: FnMut(VehicleHandle) -> f64,
    {
        let mut chunk_index = self.active().partition_point(|chunk| {
            resolve_progress(
                chunk
                    .occupants
                    .last()
                    .expect("active spatial chunk must not be empty")
                    .vehicle,
            ) < occupant.front_progress
        });
        while chunk_index < self.active_chunks {
            let chunk = &mut self.chunks[chunk_index].occupants;
            if chunk
                .first()
                .is_some_and(|first| resolve_progress(first.vehicle) > occupant.front_progress)
            {
                break;
            }
            let start = chunk.partition_point(|existing| {
                resolve_progress(existing.vehicle) < occupant.front_progress
            });
            let end = chunk.partition_point(|existing| {
                resolve_progress(existing.vehicle) <= occupant.front_progress
            });
            if let Some(relative) = chunk[start..end]
                .iter()
                .position(|existing| existing.vehicle == occupant.vehicle)
            {
                chunk.remove(start + relative);
                if chunk.is_empty() {
                    self.chunks[chunk_index..self.active_chunks].rotate_left(1);
                    self.active_chunks -= 1;
                }
                return true;
            }
            chunk_index += 1;
        }
        false
    }

    fn for_each_in_range<F>(
        &self,
        min_progress: f64,
        max_progress: f64,
        resolve_progress: &mut F,
        mut visit: impl FnMut(VehicleHandle),
    ) where
        F: FnMut(VehicleHandle) -> f64,
    {
        let mut chunk_index = self.active().partition_point(|chunk| {
            resolve_progress(
                chunk
                    .occupants
                    .last()
                    .expect("active spatial chunk must not be empty")
                    .vehicle,
            ) < min_progress
        });
        while chunk_index < self.active_chunks {
            let chunk = &self.chunks[chunk_index].occupants;
            if chunk
                .first()
                .is_some_and(|first| resolve_progress(first.vehicle) > max_progress)
            {
                break;
            }
            let start =
                chunk.partition_point(|occupant| resolve_progress(occupant.vehicle) < min_progress);
            let end = chunk
                .partition_point(|occupant| resolve_progress(occupant.vehicle) <= max_progress);
            for occupant in &chunk[start..end] {
                visit(occupant.vehicle);
            }
            chunk_index += 1;
        }
    }

    fn rebuild_from_sorted(&mut self, occupants: &[CommandOccupant]) {
        let required_chunks = occupants.len().div_ceil(SPATIAL_CHUNK_TARGET);
        self.chunks.resize_with(
            required_chunks.max(self.chunks.len()),
            SpatialChunk::default,
        );
        for chunk in &mut self.chunks {
            chunk.occupants.clear();
        }
        self.active_chunks = required_chunks;
        for (chunk, source) in self
            .chunks
            .iter_mut()
            .zip(occupants.chunks(SPATIAL_CHUNK_TARGET))
        {
            chunk.occupants.reserve(source.len());
            chunk.occupants.extend_from_slice(source);
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CommandSpatialIndex {
    buckets: Vec<SpatialBucket>,
    incoming_edges: Vec<Vec<EdgeHandle>>,
    edge_lengths: Vec<f64>,
    max_vehicle_length: f64,
    staging: Vec<Vec<CommandOccupant>>,
    front_progress: Vec<f64>,
    membership_edges: Vec<Option<EdgeHandle>>,
    membership_vehicles: Vec<Option<VehicleHandle>>,
    candidate_marks: Vec<u32>,
    candidate_epoch: u32,
    candidates: Vec<VehicleHandle>,
    reverse_best: Vec<f64>,
    reverse_touched: Vec<EdgeHandle>,
    reverse_queue: Vec<(EdgeHandle, f64)>,
    max_vehicle_speed: f64,
    speed_heap: BinaryHeap<CommandSpeedEntry>,
    speed_heap_valid: bool,
    min_emergency_deceleration: f64,
    #[cfg(test)]
    query_stats: SpatialQueryStats,
    #[cfg(test)]
    speed_heap_rebuilds: usize,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SpatialQueryStats {
    pub(crate) edge_ranges: usize,
    pub(crate) occupants_visited: usize,
}

impl PartialEq for CommandSpatialIndex {
    fn eq(&self, _other: &Self) -> bool {
        // Cache history/capacity 不属于 Core authority state。
        true
    }
}

impl CommandSpatialIndex {
    pub(crate) fn new(graph: &LaneGraph, profiles: &VehicleProfileRegistry) -> Self {
        let edge_count = graph.edges().len();
        let mut incoming_edges = vec![Vec::new(); edge_count];
        for from_index in 0..edge_count {
            let from = EdgeHandle::new(from_index);
            for to in graph.next_edges(from).expect("normalized edge must exist") {
                incoming_edges[to.index()].push(from);
            }
        }
        let edge_lengths = (0..edge_count)
            .map(|index| {
                graph
                    .edge_length(EdgeHandle::new(index))
                    .expect("normalized edge must exist")
                    .value()
            })
            .collect();
        let max_vehicle_length = profiles
            .profiles()
            .map(|profile| profile.iidm().length)
            .fold(0.0, f64::max);
        let min_emergency_deceleration = profiles
            .profiles()
            .map(|profile| profile.iidm().emergency_deceleration)
            .fold(f64::MAX, f64::min);

        Self {
            buckets: vec![SpatialBucket::default(); edge_count],
            incoming_edges,
            edge_lengths,
            max_vehicle_length,
            staging: vec![Vec::new(); edge_count],
            front_progress: Vec::new(),
            membership_edges: Vec::new(),
            membership_vehicles: Vec::new(),
            candidate_marks: Vec::new(),
            candidate_epoch: 0,
            candidates: Vec::new(),
            reverse_best: vec![-1.0; edge_count],
            reverse_touched: Vec::with_capacity(edge_count),
            reverse_queue: Vec::with_capacity(edge_count),
            max_vehicle_speed: 0.0,
            speed_heap: BinaryHeap::new(),
            speed_heap_valid: false,
            min_emergency_deceleration,
            #[cfg(test)]
            query_stats: SpatialQueryStats::default(),
            #[cfg(test)]
            speed_heap_rebuilds: 0,
        }
    }

    pub(crate) fn prepare_insert<F>(
        &mut self,
        edge: EdgeHandle,
        occupant: CommandOccupant,
        resolve_progress: &mut F,
    ) where
        F: FnMut(VehicleHandle) -> f64,
    {
        self.candidate_marks.resize(
            self.candidate_marks.len().max(occupant.vehicle.index() + 1),
            0,
        );
        self.front_progress
            .reserve((occupant.vehicle.index() + 1).saturating_sub(self.front_progress.len()));
        self.membership_edges
            .reserve((occupant.vehicle.index() + 1).saturating_sub(self.membership_edges.len()));
        self.membership_vehicles
            .reserve((occupant.vehicle.index() + 1).saturating_sub(self.membership_vehicles.len()));
        self.buckets[edge.index()].prepare_insert_resolved(occupant, resolve_progress);
    }

    pub(crate) fn insert<F>(
        &mut self,
        edge: EdgeHandle,
        occupant: CommandOccupant,
        resolve_progress: &mut F,
    ) where
        F: FnMut(VehicleHandle) -> f64,
    {
        self.front_progress.resize(
            self.front_progress.len().max(occupant.vehicle.index() + 1),
            0.0,
        );
        self.membership_edges.resize(
            self.membership_edges
                .len()
                .max(occupant.vehicle.index() + 1),
            None,
        );
        self.membership_vehicles.resize(
            self.membership_vehicles
                .len()
                .max(occupant.vehicle.index() + 1),
            None,
        );
        assert_eq!(self.membership_edges[occupant.vehicle.index()], None);
        assert_eq!(self.membership_vehicles[occupant.vehicle.index()], None);
        self.front_progress[occupant.vehicle.index()] = occupant.front_progress;
        self.membership_edges[occupant.vehicle.index()] = Some(edge);
        self.membership_vehicles[occupant.vehicle.index()] = Some(occupant.vehicle);
        self.buckets[edge.index()].insert_resolved(occupant, resolve_progress);
    }

    pub(crate) fn remove<F>(
        &mut self,
        edge: EdgeHandle,
        occupant: CommandOccupant,
        resolve_progress: &mut F,
    ) where
        F: FnMut(VehicleHandle) -> f64,
    {
        assert_eq!(
            self.membership_edges[occupant.vehicle.index()],
            Some(edge),
            "command spatial membership edge must match"
        );
        assert!(
            self.buckets[edge.index()].remove_resolved(occupant, resolve_progress),
            "command spatial occupant must exist"
        );
        self.membership_edges[occupant.vehicle.index()] = None;
        self.membership_vehicles[occupant.vehicle.index()] = None;
        self.refresh_max_vehicle_speed();
    }

    pub(crate) fn sync_vehicle(
        &mut self,
        old_membership: Option<(EdgeHandle, CommandOccupant)>,
        new_membership: Option<(EdgeHandle, CommandOccupant)>,
    ) {
        match (old_membership, new_membership) {
            (Some((old_edge, old)), Some((new_edge, new))) if old_edge == new_edge => {
                assert_eq!(old.vehicle, new.vehicle);
                assert_eq!(self.membership_edges[old.vehicle.index()], Some(old_edge));
                assert_eq!(
                    self.membership_vehicles[old.vehicle.index()],
                    Some(old.vehicle)
                );
            }
            (old_membership, new_membership) => {
                if let Some((edge, occupant)) = old_membership {
                    assert_eq!(self.membership_edges[occupant.vehicle.index()], Some(edge));
                    let indexed_occupant = CommandOccupant {
                        vehicle: occupant.vehicle,
                        front_progress: self.front_progress[occupant.vehicle.index()],
                    };
                    assert!(
                        self.buckets[edge.index()].remove(indexed_occupant, &self.front_progress),
                        "command spatial occupant must exist"
                    );
                    self.membership_edges[occupant.vehicle.index()] = None;
                    self.membership_vehicles[occupant.vehicle.index()] = None;
                }
                if let Some((edge, occupant)) = new_membership {
                    self.candidate_marks.resize(
                        self.candidate_marks.len().max(occupant.vehicle.index() + 1),
                        0,
                    );
                    self.buckets[edge.index()].prepare_insert(occupant, &self.front_progress);
                    self.front_progress[occupant.vehicle.index()] = occupant.front_progress;
                    self.membership_edges[occupant.vehicle.index()] = Some(edge);
                    self.membership_vehicles.resize(
                        self.membership_vehicles
                            .len()
                            .max(occupant.vehicle.index() + 1),
                        None,
                    );
                    self.membership_vehicles[occupant.vehicle.index()] = Some(occupant.vehicle);
                    self.buckets[edge.index()].insert(occupant, &self.front_progress);
                }
            }
        }
    }

    pub(crate) fn begin_rebuild(&mut self, vehicle_slot_count: usize) {
        for staging in &mut self.staging {
            staging.clear();
        }
        self.front_progress.resize(vehicle_slot_count, 0.0);
        self.membership_edges.resize(vehicle_slot_count, None);
        self.membership_edges.fill(None);
        self.membership_vehicles.resize(vehicle_slot_count, None);
        self.membership_vehicles.fill(None);
        self.speed_heap_valid = false;
        self.candidate_marks
            .resize(self.candidate_marks.len().max(vehicle_slot_count), 0);
    }

    pub(crate) fn stage(&mut self, edge: EdgeHandle, occupant: CommandOccupant) {
        self.front_progress[occupant.vehicle.index()] = occupant.front_progress;
        self.membership_edges[occupant.vehicle.index()] = Some(edge);
        self.membership_vehicles[occupant.vehicle.index()] = Some(occupant.vehicle);
        self.staging[edge.index()].push(occupant);
    }

    pub(crate) fn finish_rebuild(&mut self) {
        for (bucket, staging) in self.buckets.iter_mut().zip(&mut self.staging) {
            staging.sort_unstable_by(compare_occupants);
            bucket.rebuild_from_sorted(staging);
        }
    }

    pub(crate) fn gather_overlap_candidates<F>(
        &mut self,
        route_edges: &[EdgeHandle],
        route_edge_index: usize,
        front_progress: f64,
        candidate_length: f64,
        vehicle_slot_count: usize,
        resolve_progress: &mut F,
    ) where
        F: FnMut(VehicleHandle) -> f64,
    {
        self.begin_candidates(vehicle_slot_count);
        self.gather_forward(
            route_edges,
            route_edge_index,
            front_progress,
            self.max_vehicle_length,
            resolve_progress,
        );
        self.gather_reverse(
            route_edges[route_edge_index],
            front_progress,
            candidate_length,
            resolve_progress,
        );
    }

    pub(crate) fn sort_candidates_by_key(&mut self, mut key: impl FnMut(VehicleHandle) -> usize) {
        self.candidates.sort_unstable_by_key(|handle| key(*handle));
    }

    pub(crate) fn candidates(&self) -> &[VehicleHandle] {
        &self.candidates
    }

    #[cfg(test)]
    pub(crate) fn query_stats(&self) -> SpatialQueryStats {
        self.query_stats
    }

    pub(crate) fn gather_direct_follower_candidates<F>(
        &mut self,
        candidate_edge: EdgeHandle,
        candidate_progress: f64,
        reverse_horizon: f64,
        vehicle_slot_count: usize,
        resolve_progress: &mut F,
    ) where
        F: FnMut(VehicleHandle) -> f64,
    {
        self.begin_candidates(vehicle_slot_count);
        self.gather_reverse(
            candidate_edge,
            candidate_progress,
            reverse_horizon,
            resolve_progress,
        );
    }

    pub(crate) const fn max_vehicle_speed(&self) -> f64 {
        self.max_vehicle_speed
    }

    pub(crate) const fn min_emergency_deceleration(&self) -> f64 {
        self.min_emergency_deceleration
    }

    pub(crate) fn note_vehicle_speed(&mut self, vehicle: VehicleHandle, value: f64) {
        debug_assert!(value.is_finite() && value >= 0.0);
        self.max_vehicle_speed = self.max_vehicle_speed.max(value);
        if self.speed_heap_valid && value > 0.0 {
            self.speed_heap.push(CommandSpeedEntry {
                speed: value,
                vehicle,
            });
        }
    }

    pub(crate) fn set_max_vehicle_speed(&mut self, value: f64) {
        debug_assert!(value.is_finite() && value >= 0.0);
        self.max_vehicle_speed = value;
        self.speed_heap_valid = false;
    }

    /// 在 command phase 首次移除当前最快 Active vehicle 前建立一次 exact max heap。
    ///
    /// 正常 tick 只维护 scalar max；同一 command batch 后续移除通过 lazy deletion 保持
    /// `O(log V)`，避免每个 command 重扫全部 vehicles。
    pub(crate) fn prepare_speed_removal<I>(&mut self, removed_speed: f64, active_speeds: I)
    where
        I: IntoIterator<Item = (VehicleHandle, f64)>,
    {
        debug_assert!(removed_speed.is_finite() && removed_speed >= 0.0);
        if removed_speed < self.max_vehicle_speed
            || self.max_vehicle_speed == 0.0
            || self.speed_heap_valid
        {
            return;
        }

        let mut entries = std::mem::take(&mut self.speed_heap).into_vec();
        entries.clear();
        entries.extend(
            active_speeds
                .into_iter()
                .filter(|(_, speed)| *speed > 0.0)
                .map(|(vehicle, speed)| CommandSpeedEntry { speed, vehicle }),
        );
        self.speed_heap = BinaryHeap::from(entries);
        self.speed_heap_valid = true;
        #[cfg(test)]
        {
            self.speed_heap_rebuilds += 1;
        }
    }

    fn refresh_max_vehicle_speed(&mut self) {
        if !self.speed_heap_valid {
            return;
        }
        while self.speed_heap.peek().is_some_and(|entry| {
            self.membership_vehicles
                .get(entry.vehicle.index())
                .copied()
                .flatten()
                != Some(entry.vehicle)
        }) {
            self.speed_heap.pop();
        }
        self.max_vehicle_speed = self.speed_heap.peek().map_or(0.0, |entry| entry.speed);
    }

    #[cfg(test)]
    pub(crate) fn occupants(&self) -> impl Iterator<Item = (EdgeHandle, CommandOccupant)> + '_ {
        let front_progress = &self.front_progress;
        self.buckets
            .iter()
            .enumerate()
            .flat_map(move |(edge_index, bucket)| {
                bucket.active().iter().flat_map(move |chunk| {
                    chunk.occupants.iter().copied().map(move |mut occupant| {
                        occupant.front_progress = front_progress[occupant.vehicle.index()];
                        (EdgeHandle::new(edge_index), occupant)
                    })
                })
            })
    }

    #[cfg(test)]
    pub(crate) fn occupant_count(&self) -> usize {
        self.buckets
            .iter()
            .map(|bucket| {
                bucket
                    .active()
                    .iter()
                    .map(|chunk| chunk.occupants.len())
                    .sum::<usize>()
            })
            .sum()
    }

    #[cfg(test)]
    pub(crate) const fn speed_heap_rebuilds(&self) -> usize {
        self.speed_heap_rebuilds
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        let bucket_bytes = self.buckets.capacity() * std::mem::size_of::<SpatialBucket>()
            + self
                .buckets
                .iter()
                .map(|bucket| {
                    bucket.chunks.capacity() * std::mem::size_of::<SpatialChunk>()
                        + bucket
                            .chunks
                            .iter()
                            .map(|chunk| {
                                chunk.occupants.capacity() * std::mem::size_of::<CommandOccupant>()
                            })
                            .sum::<usize>()
                })
                .sum::<usize>();
        let incoming_bytes = self.incoming_edges.capacity()
            * std::mem::size_of::<Vec<EdgeHandle>>()
            + self
                .incoming_edges
                .iter()
                .map(|incoming| incoming.capacity() * std::mem::size_of::<EdgeHandle>())
                .sum::<usize>();
        let staging_bytes = self.staging.capacity() * std::mem::size_of::<Vec<CommandOccupant>>()
            + self
                .staging
                .iter()
                .map(|staging| staging.capacity() * std::mem::size_of::<CommandOccupant>())
                .sum::<usize>();
        bucket_bytes
            + incoming_bytes
            + staging_bytes
            + self.edge_lengths.capacity() * std::mem::size_of::<f64>()
            + self.front_progress.capacity() * std::mem::size_of::<f64>()
            + self.membership_edges.capacity() * std::mem::size_of::<Option<EdgeHandle>>()
            + self.membership_vehicles.capacity() * std::mem::size_of::<Option<VehicleHandle>>()
            + self.candidate_marks.capacity() * std::mem::size_of::<u32>()
            + self.candidates.capacity() * std::mem::size_of::<VehicleHandle>()
            + self.reverse_best.capacity() * std::mem::size_of::<f64>()
            + self.reverse_touched.capacity() * std::mem::size_of::<EdgeHandle>()
            + self.reverse_queue.capacity() * std::mem::size_of::<(EdgeHandle, f64)>()
            + self.speed_heap.capacity() * std::mem::size_of::<CommandSpeedEntry>()
    }

    fn begin_candidates(&mut self, vehicle_slot_count: usize) {
        self.candidate_marks
            .resize(self.candidate_marks.len().max(vehicle_slot_count), 0);
        self.candidates.clear();
        self.candidate_epoch = self.candidate_epoch.wrapping_add(1);
        if self.candidate_epoch == 0 {
            self.candidate_marks.fill(0);
            self.candidate_epoch = 1;
        }
        for edge in self.reverse_touched.drain(..) {
            self.reverse_best[edge.index()] = -1.0;
        }
        self.reverse_queue.clear();
        #[cfg(test)]
        {
            self.query_stats = SpatialQueryStats::default();
        }
    }

    fn add_range<F>(
        &mut self,
        edge: EdgeHandle,
        min_progress: f64,
        max_progress: f64,
        resolve_progress: &mut F,
    ) where
        F: FnMut(VehicleHandle) -> f64,
    {
        #[cfg(test)]
        {
            self.query_stats.edge_ranges += 1;
        }
        let epoch = self.candidate_epoch;
        let marks = &mut self.candidate_marks;
        let candidates = &mut self.candidates;
        #[cfg(test)]
        let mut visited = 0;
        self.buckets[edge.index()].for_each_in_range(
            min_progress,
            max_progress,
            resolve_progress,
            |vehicle| {
                #[cfg(test)]
                {
                    visited += 1;
                }
                let mark = &mut marks[vehicle.index()];
                if *mark != epoch {
                    *mark = epoch;
                    candidates.push(vehicle);
                }
            },
        );
        #[cfg(test)]
        {
            self.query_stats.occupants_visited += visited;
        }
    }

    fn gather_forward<F>(
        &mut self,
        route_edges: &[EdgeHandle],
        mut occurrence: usize,
        mut start_progress: f64,
        mut remaining: f64,
        resolve_progress: &mut F,
    ) where
        F: FnMut(VehicleHandle) -> f64,
    {
        while let Some(edge) = route_edges.get(occurrence).copied() {
            let edge_length = self.edge_lengths[edge.index()];
            let distance_to_end = edge_length - start_progress;
            let end_progress = if remaining >= distance_to_end {
                edge_length
            } else {
                start_progress + remaining
            };
            self.add_range(edge, start_progress, end_progress, resolve_progress);
            if distance_to_end > remaining {
                break;
            }
            remaining -= distance_to_end;
            occurrence += 1;
            start_progress = 0.0;
        }
    }

    fn gather_reverse<F>(
        &mut self,
        candidate_edge: EdgeHandle,
        candidate_progress: f64,
        candidate_length: f64,
        resolve_progress: &mut F,
    ) where
        F: FnMut(VehicleHandle) -> f64,
    {
        let same_edge_min = (candidate_progress - candidate_length).max(0.0);
        self.add_range(
            candidate_edge,
            same_edge_min,
            candidate_progress,
            resolve_progress,
        );
        if candidate_length < candidate_progress {
            return;
        }

        let available = candidate_length - candidate_progress;
        self.reverse_best[candidate_edge.index()] = available;
        self.reverse_touched.push(candidate_edge);
        self.reverse_queue.push((candidate_edge, available));

        while let Some((edge, available)) = self.reverse_queue.pop() {
            for incoming_index in 0..self.incoming_edges[edge.index()].len() {
                let predecessor = self.incoming_edges[edge.index()][incoming_index];
                let predecessor_length = self.edge_lengths[predecessor.index()];
                let min_progress = (predecessor_length - available).max(0.0);
                self.add_range(
                    predecessor,
                    min_progress,
                    predecessor_length,
                    resolve_progress,
                );
                if predecessor_length > available {
                    continue;
                }
                let next_available = available - predecessor_length;
                let best = &mut self.reverse_best[predecessor.index()];
                if next_available > *best {
                    if *best < 0.0 {
                        self.reverse_touched.push(predecessor);
                    }
                    *best = next_available;
                    self.reverse_queue.push((predecessor, next_available));
                }
            }
        }
    }
}
