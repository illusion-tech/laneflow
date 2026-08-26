use std::cell::Cell;

use laneflow_static_contract::LaneEdgeOrdinal;

use crate::tables::{for_each_occupancy_interval, remaining_along_route_i64};
use crate::{TrafficWorld, VehicleHandle, VehicleStatus};

/// 占用桶。当前与 [`LaneEdgeOrdinal`] 1:1，不映射 `LaneUseSlot`。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct OccupancyBucketOrdinal(u32);

impl OccupancyBucketOrdinal {
    const fn from_edge(edge: LaneEdgeOrdinal) -> Self {
        Self(edge.raw())
    }

    fn index(self) -> usize {
        usize::try_from(self.0).expect("occupancy bucket fits usize")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OccupancyClaim {
    vehicle: VehicleHandle,
    bucket: OccupancyBucketOrdinal,
    lo_mm: u32,
    hi_mm: u32,
    update_sequence: u32,
}

impl OccupancyClaim {
    const PLACEHOLDER: Self = Self {
        vehicle: VehicleHandle::new(0, 0),
        bucket: OccupancyBucketOrdinal(0),
        lo_mm: 0,
        hi_mm: 0,
        update_sequence: 0,
    };
}

/// 无变道时一辆车身跨越的边数上界提示；只用于安装预留，溢出时仍可扩容。
const CLAIM_HINT: usize = 4;

#[derive(Debug)]
pub(crate) struct OccupancyIndex {
    offsets: Vec<usize>,
    scratch: Vec<usize>,
    claims: Vec<OccupancyClaim>,
    inspections: Cell<u64>,
}

impl OccupancyIndex {
    fn empty() -> Self {
        Self {
            offsets: Vec::new(),
            scratch: Vec::new(),
            claims: Vec::new(),
            inspections: Cell::new(0),
        }
    }

    pub(crate) fn with_capacity(bucket_count: usize, vehicle_capacity: usize) -> Self {
        let claim_cap = vehicle_capacity.saturating_mul(CLAIM_HINT);
        Self {
            offsets: vec![0; bucket_count.saturating_add(1)],
            scratch: vec![0; bucket_count],
            claims: Vec::with_capacity(claim_cap),
            inspections: Cell::new(0),
        }
    }

    #[cfg(test)]
    pub(crate) fn inspections(&self) -> u64 {
        self.inspections.get()
    }

    #[cfg(test)]
    pub(crate) fn claims_capacity(&self) -> usize {
        self.claims.capacity()
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
        self.inspections
            .set(self.inspections.get().saturating_add(1));
    }

    #[cfg(test)]
    fn rebuild_from_pending(&mut self, pending: &[OccupancyClaim], bucket_count: usize) {
        self.inspections.set(0);
        self.scratch.clear();
        self.scratch.resize(bucket_count, 0);
        for claim in pending {
            if let Some(count) = self.scratch.get_mut(claim.bucket.index()) {
                *count += 1;
            }
        }
        self.finish_layout(bucket_count);
        for claim in pending {
            self.write_claim(*claim);
        }
        self.sort_buckets(bucket_count);
    }

    fn finish_layout(&mut self, bucket_count: usize) {
        self.offsets.clear();
        self.offsets.resize(bucket_count.saturating_add(1), 0);
        for index in 0..bucket_count {
            self.offsets[index + 1] = self.offsets[index].saturating_add(self.scratch[index]);
        }
        let total = self.offsets.get(bucket_count).copied().unwrap_or(0);
        self.claims.clear();
        self.claims.resize(total, OccupancyClaim::PLACEHOLDER);
        self.scratch.clear();
        if bucket_count == 0 {
            return;
        }
        self.scratch
            .extend_from_slice(&self.offsets[..bucket_count]);
    }

    fn write_claim(&mut self, claim: OccupancyClaim) {
        let bucket = claim.bucket.index();
        let Some(head) = self.scratch.get_mut(bucket) else {
            return;
        };
        let slot = *head;
        if let Some(target) = self.claims.get_mut(slot) {
            *target = claim;
            *head = slot.saturating_add(1);
        }
    }

    fn sort_buckets(&mut self, bucket_count: usize) {
        for bucket in 0..bucket_count {
            let start = self.offsets[bucket];
            let end = self.offsets[bucket + 1];
            self.claims[start..end].sort_unstable_by_key(|claim| {
                (
                    claim.hi_mm,
                    claim.lo_mm,
                    claim.update_sequence,
                    claim.vehicle.index(),
                )
            });
        }
    }

    fn bucket(&self, edge: LaneEdgeOrdinal) -> &[OccupancyClaim] {
        let index = OccupancyBucketOrdinal::from_edge(edge).index();
        let Some(start) = self.offsets.get(index).copied() else {
            return &[];
        };
        let Some(end) = self.offsets.get(index + 1).copied() else {
            return &[];
        };
        self.claims.get(start..end).unwrap_or(&[])
    }

    fn nearest_ahead(
        &self,
        edge: LaneEdgeOrdinal,
        self_vehicle: VehicleHandle,
        front_mm: u32,
    ) -> Option<OccupancyClaim> {
        let bucket = self.bucket(edge);
        let mut index = bucket.partition_point(|claim| claim.hi_mm <= front_mm);
        while let Some(claim) = bucket.get(index).copied() {
            self.note_inspection();
            index += 1;
            if claim.vehicle == self_vehicle {
                continue;
            }
            return Some(claim);
        }
        None
    }

    fn front_most(
        &self,
        edge: LaneEdgeOrdinal,
        self_vehicle: VehicleHandle,
    ) -> Option<OccupancyClaim> {
        for claim in self.bucket(edge) {
            self.note_inspection();
            if claim.vehicle != self_vehicle {
                return Some(*claim);
            }
        }
        None
    }

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
            .map(|claim| i64::from(claim.lo_mm) - i64::from(follower_progress));
        for (found, edge) in follower_edges
            .iter()
            .copied()
            .enumerate()
            .skip(follower_index.saturating_add(1))
        {
            let Some(claim) = self.front_most(edge, self_vehicle) else {
                continue;
            };
            let Some(gap) = remaining_along_route_i64(
                lengths,
                follower_edges,
                follower_index,
                follower_progress,
                found,
                claim.lo_mm,
            ) else {
                continue;
            };
            best = Some(best.map_or(gap, |current_gap| current_gap.min(gap)));
        }
        best
    }
}

impl TrafficWorld {
    pub(crate) fn rebuild_occupancy_index(&mut self) {
        let bucket_count = usize::try_from(self.revision.traffic().lane_edge_count())
            .expect("lane edge count fits usize");
        let mut occupancy = std::mem::replace(&mut self.occupancy, OccupancyIndex::empty());
        occupancy.inspections.set(0);
        occupancy.scratch.clear();
        occupancy.scratch.resize(bucket_count, 0);
        self.visit_occupancy_claims(|claim| {
            if let Some(count) = occupancy.scratch.get_mut(claim.bucket.index()) {
                *count += 1;
            }
        });
        occupancy.finish_layout(bucket_count);
        self.visit_occupancy_claims(|claim| occupancy.write_claim(claim));
        occupancy.sort_buckets(bucket_count);
        self.occupancy = occupancy;
    }

    fn visit_occupancy_claims(&self, mut visit: impl FnMut(OccupancyClaim)) {
        let lengths = self.revision.traffic().lane_lengths_millimetres();
        for (sequence, handle) in self.live_order.iter().copied().enumerate() {
            let Some(state) = self.vehicle_state(handle) else {
                continue;
            };
            if state.status != VehicleStatus::Active {
                continue;
            }
            let Some(edges) = self.route_edges(state.route) else {
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
                    visit(OccupancyClaim {
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
    use laneflow_static_contract::VehicleProfileOrdinal;
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision,
        SpatialBuildOption, build_shared_network_revision,
    };

    use crate::tables::occupancy_front_gap;
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
            assert_eq!(
                world.leader_bumper_gap(state, edges, lengths),
                world.leader_bumper_gap_scan(state, edges, lengths),
                "occupancy index gap must match scan oracle for {handle:?}"
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
            OccupancyClaim {
                vehicle: follower,
                bucket: OccupancyBucketOrdinal::from_edge(edge),
                lo_mm: 0,
                hi_mm: 1_000,
                update_sequence: 0,
            },
            OccupancyClaim {
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
    fn later_occurrence_uses_front_most_claim() {
        let first = LaneEdgeOrdinal::from_raw(0);
        let second = LaneEdgeOrdinal::from_raw(1);
        let follower = VehicleHandle::new(0, 0);
        let leader = VehicleHandle::new(1, 0);
        let mut index = OccupancyIndex::with_capacity(2, 2);
        let pending = vec![
            OccupancyClaim {
                vehicle: follower,
                bucket: OccupancyBucketOrdinal::from_edge(first),
                lo_mm: 8_000,
                hi_mm: 9_000,
                update_sequence: 0,
            },
            OccupancyClaim {
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
        world.rebuild_occupancy_index();
        assert_index_matches_scan(&world);
        world.step(TickInput::new(100)).unwrap();
        world.rebuild_occupancy_index();
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
        world.rebuild_occupancy_index();
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
        let indexed = world.leader_bumper_gap(&follower_state, follower_edges, lengths);
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
            inspections < all_pairs,
            "inspections={inspections} must be below all-pairs={all_pairs}"
        );
        assert!(
            inspections <= n_active.saturating_mul(4),
            "single-edge dense query should be near-linear, inspections={inspections} n={n_active}"
        );
        world.rebuild_occupancy_index();
        assert_index_matches_scan(&world);
    }

    #[test]
    fn repeated_edge_matches_scan_oracle() {
        let revision = compile_revision(|module| {
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
        });
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
        world.rebuild_occupancy_index();
        assert_index_matches_scan(&world);
    }
}
