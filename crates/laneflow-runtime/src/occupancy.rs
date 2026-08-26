use laneflow_static_contract::{LaneEdgeOrdinal, MAX_VEHICLE_LENGTH_MM, MIN_LANE_EDGE_LENGTH_MM};
use laneflow_static_network::SharedNetworkRevision;

use crate::tables::{
    DynamicRouteSlot, VehicleSlot, for_each_occupancy_interval, static_route_ordinal,
};
use crate::{RouteHandle, TrafficWorld, VehicleHandle, VehicleState, VehicleStatus};

#[cfg(test)]
use std::cell::Cell;

/// 占用桶键：物理边序号。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct OccupancyBucketOrdinal(u32);

impl OccupancyBucketOrdinal {
    const fn from_edge(edge: LaneEdgeOrdinal) -> Self {
        Self(edge.raw())
    }

    fn index(self) -> usize {
        usize::try_from(self.0).expect("occupancy bucket fits usize")
    }
}

/// 一条物理边上的占用片段，不是资源声明。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OccupancyRecord {
    vehicle: VehicleHandle,
    bucket: OccupancyBucketOrdinal,
    lo_mm: u32,
    hi_mm: u32,
    update_sequence: u32,
}

impl OccupancyRecord {
    const PLACEHOLDER: Self = Self {
        vehicle: VehicleHandle::new(0, 0),
        bucket: OccupancyBucketOrdinal(0),
        lo_mm: 0,
        hi_mm: 0,
        update_sequence: 0,
    };
}

/// 一辆车在合法最短边上最多覆盖的占用记录数：`ceil(max_length / min_edge)`。
const fn max_records_per_vehicle() -> usize {
    (MAX_VEHICLE_LENGTH_MM / MIN_LANE_EDGE_LENGTH_MM) as usize
}

pub(crate) fn occupancy_record_limit(vehicle_capacity: u32) -> usize {
    usize::try_from(vehicle_capacity)
        .unwrap_or(0)
        .saturating_mul(max_records_per_vehicle())
}

#[derive(Debug)]
pub(crate) struct OccupancyIndex {
    offsets: Vec<usize>,
    scratch: Vec<usize>,
    records: Vec<OccupancyRecord>,
    #[cfg(test)]
    inspections: Cell<u64>,
}

impl OccupancyIndex {
    pub(crate) fn with_capacity(bucket_count: usize, record_capacity: usize) -> Self {
        Self {
            offsets: vec![0; bucket_count.saturating_add(1)],
            scratch: vec![0; bucket_count],
            records: Vec::with_capacity(record_capacity),
            #[cfg(test)]
            inspections: Cell::new(0),
        }
    }

    #[cfg(test)]
    pub(crate) fn inspections(&self) -> u64 {
        self.inspections.get()
    }

    #[cfg(test)]
    pub(crate) fn records_capacity(&self) -> usize {
        self.records.capacity()
    }

    #[cfg(test)]
    pub(crate) fn records_len(&self) -> usize {
        self.records.len()
    }

    #[cfg(test)]
    pub(crate) fn scratch_capacity(&self) -> usize {
        self.scratch.capacity()
    }

    #[cfg(test)]
    pub(crate) fn offsets_capacity(&self) -> usize {
        self.offsets.capacity()
    }

    fn note_inspection(&self) {
        #[cfg(test)]
        self.inspections
            .set(self.inspections.get().saturating_add(1));
    }

    #[cfg(test)]
    fn reset_inspections(&self) {
        self.inspections.set(0);
    }

    #[cfg(test)]
    fn rebuild_from_pending(&mut self, pending: &[OccupancyRecord], bucket_count: usize) {
        self.reset_inspections();
        self.scratch.clear();
        self.scratch.resize(bucket_count, 0);
        for record in pending {
            if let Some(count) = self.scratch.get_mut(record.bucket.index()) {
                *count += 1;
            }
        }
        self.finish_layout(bucket_count);
        for record in pending {
            self.write_record(*record);
        }
        self.sort_buckets(bucket_count);
    }

    fn record_total(&self, bucket_count: usize) -> usize {
        self.scratch.iter().take(bucket_count).copied().sum()
    }

    fn ensure_record_capacity(&mut self, planned: usize) {
        if self.records.capacity() < planned {
            self.records
                .reserve(planned.saturating_sub(self.records.capacity()));
        }
    }

    fn finish_layout(&mut self, bucket_count: usize) {
        self.offsets.clear();
        self.offsets.resize(bucket_count.saturating_add(1), 0);
        for index in 0..bucket_count {
            self.offsets[index + 1] = self.offsets[index].saturating_add(self.scratch[index]);
        }
        let total = self.offsets.get(bucket_count).copied().unwrap_or(0);
        debug_assert!(total <= self.records.capacity());
        self.records.clear();
        self.records.resize(total, OccupancyRecord::PLACEHOLDER);
        self.scratch.clear();
        if bucket_count == 0 {
            return;
        }
        self.scratch
            .extend_from_slice(&self.offsets[..bucket_count]);
    }

    fn write_record(&mut self, record: OccupancyRecord) {
        let bucket = record.bucket.index();
        let Some(head) = self.scratch.get_mut(bucket) else {
            return;
        };
        let slot = *head;
        if let Some(target) = self.records.get_mut(slot) {
            *target = record;
            *head = slot.saturating_add(1);
        }
    }

    fn sort_buckets(&mut self, bucket_count: usize) {
        for bucket in 0..bucket_count {
            let start = self.offsets[bucket];
            let end = self.offsets[bucket + 1];
            self.records[start..end].sort_unstable_by_key(|record| {
                (
                    record.hi_mm,
                    record.lo_mm,
                    record.update_sequence,
                    record.vehicle.index(),
                )
            });
        }
    }

    fn bucket(&self, edge: LaneEdgeOrdinal) -> &[OccupancyRecord] {
        let index = OccupancyBucketOrdinal::from_edge(edge).index();
        let Some(start) = self.offsets.get(index).copied() else {
            return &[];
        };
        let Some(end) = self.offsets.get(index + 1).copied() else {
            return &[];
        };
        self.records.get(start..end).unwrap_or(&[])
    }

    fn nearest_ahead(
        &self,
        edge: LaneEdgeOrdinal,
        self_vehicle: VehicleHandle,
        front_mm: u32,
    ) -> Option<OccupancyRecord> {
        let bucket = self.bucket(edge);
        let mut index = bucket.partition_point(|record| record.hi_mm <= front_mm);
        while let Some(record) = bucket.get(index).copied() {
            self.note_inspection();
            index += 1;
            if record.vehicle == self_vehicle {
                continue;
            }
            return Some(record);
        }
        None
    }

    fn front_most(
        &self,
        edge: LaneEdgeOrdinal,
        self_vehicle: VehicleHandle,
    ) -> Option<OccupancyRecord> {
        for record in self.bucket(edge) {
            self.note_inspection();
            if record.vehicle != self_vehicle {
                return Some(*record);
            }
        }
        None
    }

    /// 前保险杠到最近前车后保险杠的 `i64` 毫米间隙；可负。
    pub(crate) fn leader_gap(
        &self,
        self_vehicle: VehicleHandle,
        follower_edges: &[LaneEdgeOrdinal],
        follower_index: usize,
        follower_progress: u32,
        lengths: &[u32],
    ) -> Option<i64> {
        let current = *follower_edges.get(follower_index)?;
        let mut best = self
            .nearest_ahead(current, self_vehicle, follower_progress)
            .map(|record| i64::from(record.lo_mm) - i64::from(follower_progress));
        let Some(current_length) = lengths.get(current.index()).copied() else {
            return best;
        };
        let mut base_mm = i64::from(current_length) - i64::from(follower_progress);
        for edge in follower_edges
            .iter()
            .copied()
            .skip(follower_index.saturating_add(1))
        {
            if let Some(record) = self.front_most(edge, self_vehicle) {
                if let Some(gap) = base_mm.checked_add(i64::from(record.lo_mm)) {
                    best = Some(best.map_or(gap, |current_gap| current_gap.min(gap)));
                }
            }
            let Some(edge_length) = lengths.get(edge.index()).copied() else {
                return best;
            };
            let Some(next_base) = base_mm.checked_add(i64::from(edge_length)) else {
                return best;
            };
            base_mm = next_base;
        }
        best
    }
}

fn vehicle_state_in(vehicles: &[VehicleSlot], handle: VehicleHandle) -> Option<&VehicleState> {
    let slot = vehicles.get(usize::try_from(handle.index()).ok()?)?;
    if slot.generation != handle.generation() {
        return None;
    }
    slot.state.as_ref()
}

fn route_edges_in<'a>(
    revision: &'a SharedNetworkRevision,
    dynamic_routes: &'a [DynamicRouteSlot],
    route: RouteHandle,
) -> Option<&'a [LaneEdgeOrdinal]> {
    if let Some(ordinal) = static_route_ordinal(route) {
        return revision.traffic().relations().static_route_edges(ordinal);
    }
    let slot = dynamic_routes.get(usize::try_from(route.index()).ok()?)?;
    if slot.generation != route.generation() {
        return None;
    }
    Some(slot.compiled.as_ref()?.edges.as_ref())
}

fn visit_occupancy_records(
    live_order: &[VehicleHandle],
    vehicles: &[VehicleSlot],
    revision: &SharedNetworkRevision,
    dynamic_routes: &[DynamicRouteSlot],
    mut visit: impl FnMut(OccupancyRecord),
) {
    let lengths = revision.traffic().lane_lengths_millimetres();
    for (sequence, handle) in live_order.iter().copied().enumerate() {
        let Some(state) = vehicle_state_in(vehicles, handle) else {
            continue;
        };
        if state.status != VehicleStatus::Active {
            continue;
        }
        let Some(edges) = route_edges_in(revision, dynamic_routes, state.route) else {
            continue;
        };
        let Ok(index) = usize::try_from(state.route_edge_index) else {
            continue;
        };
        let Ok(update_sequence) = u32::try_from(sequence) else {
            continue;
        };
        let _ = for_each_occupancy_interval(
            lengths,
            edges,
            index,
            state.progress_mm,
            state.length_mm,
            |edge, lo_mm, hi_mm| {
                visit(OccupancyRecord {
                    vehicle: handle,
                    bucket: OccupancyBucketOrdinal::from_edge(edge),
                    lo_mm,
                    hi_mm,
                    update_sequence,
                });
            },
        );
    }
}

impl TrafficWorld {
    pub(crate) fn rebuild_occupancy_index(&mut self) -> Result<(), crate::StepError> {
        let bucket_count = usize::try_from(self.revision.traffic().lane_edge_count())
            .expect("lane edge count fits usize");
        let planned = occupancy_record_limit(self.config.vehicle_capacity());
        let occupancy = &mut self.occupancy;
        #[cfg(test)]
        occupancy.reset_inspections();
        occupancy.ensure_record_capacity(planned);
        occupancy.scratch.clear();
        occupancy.scratch.resize(bucket_count, 0);
        visit_occupancy_records(
            &self.live_order,
            &self.vehicles,
            &self.revision,
            &self.dynamic_routes,
            |record| {
                if let Some(count) = occupancy.scratch.get_mut(record.bucket.index()) {
                    *count += 1;
                }
            },
        );
        if occupancy.record_total(bucket_count) > planned {
            return Err(crate::StepError::OccupancyCapacityExceeded);
        }
        occupancy.finish_layout(bucket_count);
        visit_occupancy_records(
            &self.live_order,
            &self.vehicles,
            &self.revision,
            &self.dynamic_routes,
            |record| occupancy.write_record(record),
        );
        occupancy.sort_buckets(bucket_count);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn occupancy_inspections(&self) -> u64 {
        self.occupancy.inspections()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use laneflow_compiler::{
        CompilationUnitBuilder, CompileLimits, Compiler, IidmVehicleProfileInput, LaneEdgeInput,
        ParticipantClassInput, ParticipantClassReference, PortableDiffBase,
        PortableEmissionProvenance, SourceModuleHeader, SourceModuleHeaderInput,
        SyntheticModuleBuilder, VehicleProfileInput, emit_portable_candidate,
    };
    use laneflow_format::{
        FormatLimits, check_canonical_network_input, check_post_emission_bundle,
    };
    use laneflow_static_contract::{ParkingSpaceOrdinal, VehicleProfileOrdinal};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision,
        SpatialBuildOption, build_shared_network_revision,
    };

    use crate::tables::{occupancy_front_gap, remaining_along_route_i64};
    use crate::{RouteRegisterInput, TickInput, VehicleSpawnInput, WorldConfig};

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
    );

    fn iidm() -> IidmVehicleProfileInput {
        IidmVehicleProfileInput {
            length_meters: 4.5,
            desired_speed_meters_per_second: 13.75,
            min_gap_meters: 2.0,
            time_headway_seconds: 1.4,
            max_acceleration_meters_per_second_squared: 1.8,
            comfortable_deceleration_meters_per_second_squared: 2.0,
            emergency_deceleration_meters_per_second_squared: 4.5,
        }
    }

    fn compile_revision(
        configure: impl FnOnce(&mut SyntheticModuleBuilder),
    ) -> Arc<SharedNetworkRevision> {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: "city/occupancy-index",
                source_document_key: "occupancy-index.document",
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: Some(42),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .expect("source header");
        let mut module = SyntheticModuleBuilder::new(header, &limits).expect("synthetic module");
        configure(&mut module);
        let mut unit = CompilationUnitBuilder::new(limits);
        unit.add_synthetic_module(module.finish().expect("finished module"))
            .expect("compilation module");
        let output = Compiler::new()
            .compile(unit.build().expect("compilation unit"))
            .expect("compiled output");
        let provenance = PortableEmissionProvenance::try_new("laneflow-occupancy-index-v1")
            .expect("portable provenance");
        let candidate = emit_portable_candidate(
            &output,
            &provenance,
            FormatLimits::HARD,
            PortableDiffBase::Genesis,
        )
        .expect("portable candidate");
        let checked = check_post_emission_bundle(
            candidate.canonical_artifact().bytes(),
            candidate.source_map().bytes(),
            candidate.semantic_diff().bytes(),
            candidate.expected_semantic_diff_base(),
            FormatLimits::HARD,
        )
        .expect("post-emission checked bundle");
        build_shared_network_revision(
            checked.canonical_network_input(),
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::Omit,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("shared network revision")
    }

    fn add_car_profile(module: &mut SyntheticModuleBuilder) {
        module
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "road-user",
                extends: None,
            })
            .expect("class")
            .add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: "car",
                participant_class: ParticipantClassReference::local("road-user"),
                iidm: iidm(),
            })
            .expect("profile");
    }

    fn two_edge_revision() -> Arc<SharedNetworkRevision> {
        compile_revision(|module| {
            add_car_profile(module);
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "stem",
                    length_meters: 20.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[laneflow_compiler::LaneEdgeReference::local("tail")],
                })
                .expect("stem")
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "tail",
                    length_meters: 20.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[],
                })
                .expect("tail");
        })
    }

    fn loop_revision() -> Arc<SharedNetworkRevision> {
        compile_revision(|module| {
            add_car_profile(module);
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "loop-a",
                    length_meters: 20.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[laneflow_compiler::LaneEdgeReference::local("loop-b")],
                })
                .expect("a")
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "loop-b",
                    length_meters: 20.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[laneflow_compiler::LaneEdgeReference::local("loop-a")],
                })
                .expect("b");
        })
    }

    fn index_gap(world: &TrafficWorld, state: &VehicleState) -> Option<i64> {
        let lengths = world.revision.traffic().lane_lengths_millimetres();
        let edges = world.route_edges(state.route).unwrap();
        let cursor = usize::try_from(state.route_edge_index).unwrap();
        world
            .occupancy
            .leader_gap(state.handle, edges, cursor, state.progress_mm, lengths)
    }

    fn assert_index_matches_scan(world: &TrafficWorld) {
        let lengths = world.revision.traffic().lane_lengths_millimetres();
        for handle in world.live_order.iter().copied() {
            let Some(state) = world.vehicle_state(handle) else {
                continue;
            };
            if state.status != VehicleStatus::Active {
                continue;
            }
            let Some(edges) = world.route_edges(state.route) else {
                continue;
            };
            let cursor = usize::try_from(state.route_edge_index).unwrap();
            let indexed =
                world
                    .occupancy
                    .leader_gap(state.handle, edges, cursor, state.progress_mm, lengths);
            let scanned = world.leader_bumper_gap_scan(state, edges, lengths);
            let wrapped = world.leader_bumper_gap(state, edges, lengths);
            assert_eq!(
                indexed, scanned,
                "occupancy index gap must match scan oracle for {handle:?}"
            );
            assert_eq!(
                wrapped, indexed,
                "leader_bumper_gap must use the occupancy index for {handle:?}"
            );
        }
    }

    fn active_count(world: &TrafficWorld) -> u64 {
        world
            .live_order
            .iter()
            .copied()
            .filter(|handle| {
                world
                    .vehicle_state(*handle)
                    .is_some_and(|state| state.status == VehicleStatus::Active)
            })
            .count() as u64
    }

    #[test]
    fn nearest_ahead_skips_self_and_uses_rear_bumper() {
        let edge = LaneEdgeOrdinal::from_raw(0);
        let follower = VehicleHandle::new(0, 0);
        let leader = VehicleHandle::new(1, 0);
        let mut index = OccupancyIndex::with_capacity(1, 2);
        let pending = vec![
            OccupancyRecord {
                vehicle: follower,
                bucket: OccupancyBucketOrdinal::from_edge(edge),
                lo_mm: 0,
                hi_mm: 1_000,
                update_sequence: 0,
            },
            OccupancyRecord {
                vehicle: leader,
                bucket: OccupancyBucketOrdinal::from_edge(edge),
                lo_mm: 6_000,
                hi_mm: 8_000,
                update_sequence: 1,
            },
        ];
        index.rebuild_from_pending(&pending, 1);
        let gap = index.leader_gap(follower, &[edge], 0, 1_000, &[10_000]);
        assert_eq!(gap, Some(5_000));
    }

    #[test]
    fn later_occurrence_uses_front_most_record() {
        let first = LaneEdgeOrdinal::from_raw(0);
        let second = LaneEdgeOrdinal::from_raw(1);
        let follower = VehicleHandle::new(0, 0);
        let leader = VehicleHandle::new(1, 0);
        let mut index = OccupancyIndex::with_capacity(2, 2);
        let pending = vec![
            OccupancyRecord {
                vehicle: follower,
                bucket: OccupancyBucketOrdinal::from_edge(first),
                lo_mm: 8_000,
                hi_mm: 9_000,
                update_sequence: 0,
            },
            OccupancyRecord {
                vehicle: leader,
                bucket: OccupancyBucketOrdinal::from_edge(second),
                lo_mm: 500,
                hi_mm: 1_500,
                update_sequence: 1,
            },
        ];
        index.rebuild_from_pending(&pending, 2);
        let lengths = [10_000, 5_000];
        let edges = [first, second];
        let gap = index.leader_gap(follower, &edges, 0, 9_000, &lengths);
        assert_eq!(
            gap,
            remaining_along_route_i64(&lengths, &edges, 0, 9_000, 1, 500)
        );
    }

    #[test]
    fn full_spatial_follower_matches_scan_oracle() {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).unwrap();
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .unwrap();
        let mut world = TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 100)).unwrap();
        let route = world
            .static_route(laneflow_static_contract::StaticRouteOrdinal::from_raw(0))
            .unwrap();
        let profile = world
            .revision
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .unwrap();
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000 + profile.length_mm() + profile.min_gap_mm() + 2_000,
                0,
            ))
            .unwrap();
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .unwrap();
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        assert_index_matches_scan(&world);
        world.step(TickInput::new(100)).unwrap();
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        assert_index_matches_scan(&world);
    }

    #[test]
    fn empty_and_solo_vehicle_have_no_leader() {
        let revision = two_edge_revision();
        let stem = LaneEdgeOrdinal::from_raw(0);
        let mut world =
            TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 100)).expect("install");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        world.step(TickInput::new(100)).unwrap();
        let route = world
            .register_route(RouteRegisterInput::new(vec![stem]))
            .expect("route");
        let solo = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("solo");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        let state = world.vehicle_state(solo).copied().unwrap();
        assert_eq!(index_gap(&world, &state), None);
        assert_index_matches_scan(&world);
    }

    #[test]
    fn vehicle_behind_on_current_edge_is_not_leader() {
        let revision = two_edge_revision();
        let stem = LaneEdgeOrdinal::from_raw(0);
        let mut world =
            TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 100)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![stem]))
            .expect("route");
        let profile = world
            .revision
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .unwrap();
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("behind");
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000 + profile.length_mm() + profile.min_gap_mm() + 2_000,
                0,
            ))
            .expect("ahead");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        let state = world.vehicle_state(follower).copied().unwrap();
        assert_eq!(index_gap(&world, &state), None);
        assert_index_matches_scan(&world);
    }

    #[test]
    fn leader_fully_on_next_edge_matches_scan() {
        let revision = two_edge_revision();
        let stem = LaneEdgeOrdinal::from_raw(0);
        let tail = LaneEdgeOrdinal::from_raw(1);
        let mut world =
            TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 100)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![stem, tail]))
            .expect("route");
        let profile = world
            .revision
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .unwrap();
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                1,
                profile.length_mm() + 1_000,
                0,
            ))
            .expect("leader on tail");
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("follower on stem");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        assert_index_matches_scan(&world);
        let state = world.vehicle_state(follower).copied().unwrap();
        assert!(index_gap(&world, &state).is_some());
    }

    #[test]
    fn cycle_wrap_uses_later_occurrence_of_vehicle_behind() {
        let revision = loop_revision();
        let a = LaneEdgeOrdinal::from_raw(0);
        let b = LaneEdgeOrdinal::from_raw(1);
        let mut world =
            TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 100)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![a, b, a]))
            .expect("route");
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("physically behind");
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                9_000,
                0,
            ))
            .expect("near end of first a");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        assert_index_matches_scan(&world);
        let state = world.vehicle_state(follower).copied().unwrap();
        let gap = index_gap(&world, &state).expect("wrap-around leader");
        assert!(
            gap > 20_000,
            "leader behind on the current occurrence must be found via the next a, gap={gap}"
        );
    }

    #[test]
    fn parked_and_completed_are_not_leaders() {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).unwrap();
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .unwrap();
        let mut world = TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 100)).unwrap();
        let route = world
            .static_route(laneflow_static_contract::StaticRouteOrdinal::from_raw(0))
            .unwrap();
        let profile = world
            .revision
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .unwrap();
        let parked = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000 + profile.length_mm() + profile.min_gap_mm() + 2_000,
                0,
            ))
            .unwrap();
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .unwrap();
        world
            .occupy_parking(parked, ParkingSpaceOrdinal::from_raw(0))
            .expect("park");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        let follower_state = world.vehicle_state(follower).copied().unwrap();
        assert_eq!(index_gap(&world, &follower_state), None);
        assert_index_matches_scan(&world);

        let revision = two_edge_revision();
        let stem = LaneEdgeOrdinal::from_raw(0);
        let tail = LaneEdgeOrdinal::from_raw(1);
        let mut world =
            TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 100)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![stem, tail]))
            .expect("route");
        let tail_len = world.revision.traffic().lane_lengths_millimetres()[tail.index()];
        let finishing = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                1,
                tail_len,
                0,
            ))
            .expect("at route end");
        world.step(TickInput::new(100)).unwrap();
        assert_eq!(
            world.vehicle(finishing).unwrap().status(),
            VehicleStatus::Completed
        );
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("follower");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        let follower_state = world.vehicle_state(follower).copied().unwrap();
        assert_eq!(index_gap(&world, &follower_state), None);
        assert_index_matches_scan(&world);
    }

    #[test]
    fn diverge_overhang_matches_scan_and_occupancy_front_gap() {
        let revision = compile_revision(|module| {
            add_car_profile(module);
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "stem",
                    length_meters: 10.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[
                        laneflow_compiler::LaneEdgeReference::local("left"),
                        laneflow_compiler::LaneEdgeReference::local("right"),
                    ],
                })
                .expect("stem")
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "left",
                    length_meters: 20.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[],
                })
                .expect("left")
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "right",
                    length_meters: 20.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[],
                })
                .expect("right");
        });
        let traffic = revision.traffic();
        let count = traffic.lane_edge_count();
        let stem = (0..count)
            .map(LaneEdgeOrdinal::from_raw)
            .find(|edge| {
                traffic
                    .successors(*edge)
                    .is_some_and(|successors| successors.len() == 2)
            })
            .expect("stem");
        let branches = traffic.successors(stem).expect("branches");
        let left = branches[0];
        let right = branches[1];
        let mut world =
            TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 100)).expect("install");
        let leader_route = world
            .register_route(RouteRegisterInput::new(vec![stem, left]))
            .expect("left route");
        let follower_route = world
            .register_route(RouteRegisterInput::new(vec![stem, right]))
            .expect("right route");
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                leader_route,
                1,
                500,
                0,
            ))
            .expect("leader");
        let follower = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                follower_route,
                0,
                5_000,
                10_000,
            ))
            .expect("follower");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        assert_index_matches_scan(&world);
        let follower_state = world.vehicle_state(follower).copied().unwrap();
        let leader_state = world
            .live_order
            .iter()
            .copied()
            .find_map(|handle| {
                let state = world.vehicle_state(handle)?;
                (handle != follower).then_some(*state)
            })
            .expect("leader state");
        let lengths = world.revision.traffic().lane_lengths_millimetres();
        let follower_edges = world.route_edges(follower_state.route).unwrap();
        let leader_edges = world.route_edges(leader_state.route).unwrap();
        let indexed = world.occupancy.leader_gap(
            follower_state.handle,
            follower_edges,
            usize::try_from(follower_state.route_edge_index).unwrap(),
            follower_state.progress_mm,
            lengths,
        );
        let pair = occupancy_front_gap(
            lengths,
            follower_edges,
            usize::try_from(follower_state.route_edge_index).unwrap(),
            follower_state.progress_mm,
            leader_edges,
            usize::try_from(leader_state.route_edge_index).unwrap(),
            leader_state.progress_mm,
            leader_state.length_mm,
        );
        assert_eq!(indexed, pair);
        world.step(TickInput::new(100)).unwrap();
        let follower_after = world.vehicle(follower).unwrap();
        assert!(
            follower_after.progress_mm() < 6_000,
            "follower must not enter leader overhang, progress={}",
            follower_after.progress_mm()
        );
    }

    #[test]
    fn dense_same_edge_inspections_are_not_quadratic() {
        let revision = compile_revision(|module| {
            add_car_profile(module);
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "corridor",
                    length_meters: 400.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[],
                })
                .expect("corridor");
        });
        let edge = LaneEdgeOrdinal::from_raw(0);
        let n = 32_u32;
        let mut world =
            TrafficWorld::install(revision, WorldConfig::new(n, 4, 1, 100)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![edge]))
            .expect("route");
        let profile = world
            .revision
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .unwrap();
        let spacing = profile.length_mm() + profile.min_gap_mm() + 1_000;
        for slot in 0..n {
            let progress = 5_000 + slot * spacing;
            world
                .spawn_vehicle(VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    progress,
                    0,
                ))
                .expect("spawn");
        }
        world.step(TickInput::new(100)).unwrap();
        let n_active = active_count(&world);
        let inspections = world.occupancy_inspections();
        let all_pairs = n_active.saturating_mul(n_active.saturating_sub(1));
        assert_eq!(n_active, u64::from(n));
        assert!(
            inspections >= n_active.saturating_sub(1),
            "index query must inspect at least one claim per follower with a leader, inspections={inspections} n={n_active}"
        );
        assert!(
            inspections < all_pairs,
            "inspections={inspections} must be below all-pairs={all_pairs}"
        );
        assert!(
            inspections <= n_active.saturating_mul(4),
            "single-edge dense query should be near-linear, inspections={inspections} n={n_active}"
        );
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        assert_index_matches_scan(&world);
    }

    #[test]
    fn repeated_edge_prefers_nearer_occurrence() {
        let revision = loop_revision();
        let a = LaneEdgeOrdinal::from_raw(0);
        let b = LaneEdgeOrdinal::from_raw(1);
        let mut world =
            TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 100)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(vec![a, b, a]))
            .expect("route");
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("rear");
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                2,
                6_000,
                0,
            ))
            .expect("ahead on repeated");
        world.rebuild_occupancy_index().expect("occupancy rebuild");
        assert_index_matches_scan(&world);
    }

    #[test]
    fn short_edge_chain_does_not_grow_record_capacity_after_warmup() {
        let revision = compile_revision(|module| {
            add_car_profile(module);
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "stem",
                    length_meters: 20.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[laneflow_compiler::LaneEdgeReference::local("s0")],
                })
                .expect("stem");
            let keys = ["s0", "s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9"];
            for (index, key) in keys.iter().enumerate() {
                if index + 1 < keys.len() {
                    let next = laneflow_compiler::LaneEdgeReference::local(keys[index + 1]);
                    module
                        .add_lane_edge(LaneEdgeInput {
                            lane_edge_key: key,
                            length_meters: 1.0,
                            speed_limit_meters_per_second: 10.0,
                            successors: std::slice::from_ref(&next),
                        })
                        .expect("short");
                } else {
                    module
                        .add_lane_edge(LaneEdgeInput {
                            lane_edge_key: key,
                            length_meters: 1.0,
                            speed_limit_meters_per_second: 10.0,
                            successors: &[],
                        })
                        .expect("short tail");
                }
            }
        });
        let traffic = revision.traffic();
        let mut edges = Vec::new();
        let mut current = (0..traffic.lane_edge_count())
            .map(LaneEdgeOrdinal::from_raw)
            .find(|edge| {
                traffic
                    .successors(*edge)
                    .is_some_and(|successors| !successors.is_empty())
                    && traffic.lane_lengths_millimetres()[edge.index()] >= 20_000
            })
            .expect("stem");
        loop {
            edges.push(current);
            let Some(successors) = traffic.successors(current) else {
                break;
            };
            let Some(next) = successors.first().copied() else {
                break;
            };
            current = next;
        }
        let mut world =
            TrafficWorld::install(revision, WorldConfig::new(1, 4, 1, 1_000)).expect("install");
        let route = world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route");
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                5,
                500,
                0,
            ))
            .expect("spawn spanning five 1 m edges");
        world.step(TickInput::new(1_000)).unwrap();
        let planned = occupancy_record_limit(1);
        let cap = world.occupancy.records_capacity();
        assert!(
            cap >= planned,
            "first rebuild must reserve the legal occupancy record limit, cap={cap} planned={planned}"
        );
        assert!(
            world.occupancy.records_len() > 4,
            "body on the 1 m chain must emit more than four occupancy records, got {}",
            world.occupancy.records_len()
        );
        for _ in 0..8 {
            world.step(TickInput::new(1_000)).unwrap();
            assert_eq!(
                world.occupancy.records_capacity(),
                cap,
                "steady ticks must not grow occupancy record capacity"
            );
        }
    }
}
