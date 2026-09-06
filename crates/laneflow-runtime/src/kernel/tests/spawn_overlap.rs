use std::sync::Arc;
use std::time::Instant;

use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, IidmVehicleProfileInput, LaneEdgeInput,
    LaneEdgeReference, ParticipantClassInput, ParticipantClassReference, PortableDiffBase,
    PortableEmissionProvenance, SourceModuleHeader, SourceModuleHeaderInput,
    SyntheticModuleBuilder, VehicleProfileInput, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_static_contract::{LaneEdgeOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

use crate::kernel::world::{overlap_blocker_inspections, reset_overlap_blocker_inspections};
use crate::{
    CommittedNetworkSource, PublishedLfcaReference, RouteHandle, RouteRegisterInput,
    SnapshotRestoreLimits, SpawnError, TickInput, TrafficWorld, VehicleHandle, VehicleSpawnInput,
    VehicleStatus, WorldConfig, encode_lfrs, restore_lfrs,
};

fn lane_revision(edge_count: u32) -> Arc<SharedNetworkRevision> {
    road_revision(edge_count, 8_000.0, false)
}

fn road_revision(edge_count: u32, length_meters: f64, cyclic: bool) -> Arc<SharedNetworkRevision> {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "runtime/overlap-evidence",
            source_document_key: "overlap.document",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x28; 32],
            frontend_options_digest: [0x52; 32],
            random_seed: Some(528),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .expect("header");
    let mut module = SyntheticModuleBuilder::new(header, &limits).expect("module");
    module
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .expect("class")
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "car",
            participant_class: ParticipantClassReference::local("road-user"),
            iidm: IidmVehicleProfileInput {
                length_meters: 4.5,
                desired_speed_meters_per_second: 13.75,
                min_gap_meters: 2.0,
                time_headway_seconds: 1.4,
                max_acceleration_meters_per_second_squared: 1.8,
                comfortable_deceleration_meters_per_second_squared: 2.0,
                emergency_deceleration_meters_per_second_squared: 4.5,
            },
        })
        .expect("profile");
    for edge in 0..edge_count {
        let successor_key = format!("edge-{}", (edge + 1) % edge_count);
        let successors = [LaneEdgeReference::local(&successor_key)];
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: &format!("edge-{edge}"),
                length_meters,
                speed_limit_meters_per_second: 15.0,
                successors: if cyclic { &successors } else { &[] },
            })
            .expect("edge");
    }
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().expect("module"))
        .expect("unit module");
    let output = Compiler::new()
        .compile(unit.build().expect("unit"))
        .expect("compile");
    let provenance =
        PortableEmissionProvenance::try_new("laneflow-overlap-evidence-v1").expect("provenance");
    let candidate = emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .expect("candidate");
    let checked = check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .expect("checked bundle");
    build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("revision")
}

fn empty_world(revision: Arc<SharedNetworkRevision>, capacity: u32) -> TrafficWorld {
    let origin = *revision.canonical_origin();
    let source = CommittedNetworkSource::Published {
        reference: PublishedLfcaReference::new(
            "fixture://overlap-evidence",
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .expect("source"),
    };
    TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(capacity, 512, 2_048, 1_024, 1, 100),
        source,
        528,
        crate::test_policy::selection(&revision),
    )
    .expect("install")
}

fn lane_routes(world: &mut TrafficWorld, count: u32) -> Vec<RouteHandle> {
    (0..count)
        .map(|edge| {
            world
                .register_route(RouteRegisterInput::new(vec![LaneEdgeOrdinal::from_raw(
                    edge,
                )]))
                .expect("route")
        })
        .collect()
}

/// 同机 release 描述性证据；不以跨机器的耗时阈值充当 CI 门禁。
#[test]
#[ignore = "manual release overlap admission and restore evidence"]
fn admission_scale_evidence() {
    let revision = lane_revision(256);
    for count in [1_000u32, 4_000, 10_000] {
        for used_edges in [16u32, 256] {
            for sample in 0..3 {
                let mut world = empty_world(Arc::clone(&revision), count + 1);
                let routes = lane_routes(&mut world, used_edges);
                reset_overlap_blocker_inspections();
                let started = Instant::now();
                for vehicle in 0..count {
                    world
                        .spawn_vehicle(VehicleSpawnInput::new(
                            VehicleProfileOrdinal::from_raw(0),
                            routes[(vehicle % used_edges) as usize],
                            0,
                            (vehicle / used_edges + 1) * 10_000,
                            0,
                        ))
                        .expect("spaced spawn");
                }
                let spawn_us = started.elapsed().as_micros();
                let spawn_candidates = overlap_blocker_inspections();
                let index_bytes = world.derived.spawn_overlap.retained_logical_bytes();
                let captured = world.capture_snapshot().expect("snapshot");
                let bytes = encode_lfrs(&captured);
                reset_overlap_blocker_inspections();
                let started = Instant::now();
                let restored = restore_lfrs(
                    &bytes,
                    Arc::clone(&revision),
                    world.committed_source().clone(),
                    WorldConfig::new(count + 1, 512, 2_048, 1_024, 1, 100),
                    SnapshotRestoreLimits::new(64 * 1_024 * 1_024, 1_024),
                )
                .expect("restore");
                let restore_us = started.elapsed().as_micros();
                let restore_candidates = overlap_blocker_inspections();
                std::hint::black_box(restored);
                println!(
                    "overlap count={count} active={count} used_edges={used_edges} sample={sample} spawn_us={spawn_us} spawn_candidates={spawn_candidates} restore_us={restore_us} restore_candidates={restore_candidates} index_bytes={index_bytes}"
                );
            }
        }
    }
}

fn input(route: RouteHandle, occurrence: u32, progress: u32) -> VehicleSpawnInput {
    VehicleSpawnInput::new(
        VehicleProfileOrdinal::from_raw(0),
        route,
        occurrence,
        progress,
        0,
    )
}

/// 参考路径从已提交 live 表全扫，不借用候选索引或 active_order。
fn linear_blocker(
    world: &TrafficWorld,
    route: RouteHandle,
    occurrence: usize,
    progress: u32,
) -> Option<VehicleHandle> {
    world
        .live_vehicles()
        .iter()
        .copied()
        .filter(|handle| {
            let state = world.vehicle_state(*handle).expect("live state");
            state.status == VehicleStatus::Active
                && crate::kernel::tables::bodies_overlap(
                    world.traffic().lane_lengths_millimetres(),
                    world.route_edges(route).unwrap(),
                    occurrence,
                    progress,
                    4_500,
                    world.route_edges(state.route).unwrap(),
                    state.route_edge_index as usize,
                    state.progress_mm,
                    state.length_mm,
                )
        })
        .min_by_key(|handle| (handle.index(), handle.generation()))
}

#[test]
fn indexed_admission_matches_linear_cross_edge_and_repeated_occurrences() {
    for (edge_count, edge_length, route_len, positions) in [
        (3, 10.0, 3, vec![(1, 1_000), (2, 7_000)]),
        (2, 2.0, 5, vec![(3, 1_000)]),
    ] {
        let mut world = empty_world(road_revision(edge_count, edge_length, true), 8);
        let mut edges = vec![LaneEdgeOrdinal::from_raw(0)];
        while edges.len() < route_len {
            edges.push(world.traffic().successors(*edges.last().unwrap()).unwrap()[0]);
        }
        let route = world
            .register_route(RouteRegisterInput::new(edges.clone()))
            .unwrap();
        for (occurrence, progress) in positions {
            world
                .spawn_vehicle(input(route, occurrence, progress))
                .unwrap();
        }
        for rebuild in [false, true] {
            if rebuild {
                world.derived.spawn_overlap.mark_stale();
            }
            for (occurrence, edge) in edges.iter().enumerate() {
                let end = world.traffic().lane_lengths_millimetres()[edge.index()];
                for progress in [0, 1, 999, 1_000, end - 1, end] {
                    let expected = linear_blocker(&world, route, occurrence, progress);
                    assert_eq!(
                        world.overlap_blocker(route, occurrence, progress, 4_500),
                        expected,
                        "rebuild={rebuild}, occurrence={occurrence}, progress={progress}"
                    );
                    world.derived.spawn_overlap.mark_stale();
                    world.try_refresh_overlap_index().unwrap();
                    assert_eq!(
                        world.indexed_overlap_blocker(route, occurrence, progress, 4_500, None),
                        expected,
                        "fallible rebuild, occurrence={occurrence}, progress={progress}"
                    );
                }
            }
        }
    }
}

#[test]
fn cutover_revalidation_reuses_one_index_and_excludes_self() {
    use crate::admin::cutover_migration::revalidate_migrated_vehicles;
    use crate::kernel::spawn_overlap::overlap_rebuilds;

    let mut world = empty_world(lane_revision(256), 64);
    let routes = lane_routes(&mut world, 16);
    for vehicle in 0..64 {
        world
            .spawn_vehicle(input(
                routes[vehicle % 16],
                0,
                (vehicle / 16 + 1) as u32 * 10_000,
            ))
            .unwrap();
    }
    world.derived.spawn_overlap = Default::default();
    let before = overlap_rebuilds();
    reset_overlap_blocker_inspections();
    revalidate_migrated_vehicles(&mut world).unwrap();
    assert_eq!(overlap_rebuilds() - before, 1);
    assert_eq!(overlap_blocker_inspections(), 64 * 3);
    revalidate_migrated_vehicles(&mut world).unwrap();
    assert_eq!(overlap_rebuilds() - before, 1);
    assert_eq!(overlap_blocker_inspections(), 2 * 64 * 3);
    assert!(world.derived.spawn_overlap.retained_logical_bytes() > 0);
}

#[test]
#[ignore = "manual release cutover candidate count and retained index evidence"]
fn cutover_overlap_scale_evidence() {
    let revision = lane_revision(256);
    for count in [1_000u32, 4_000, 10_000] {
        for used_edges in [16u32, 256] {
            let mut world = empty_world(Arc::clone(&revision), count);
            let routes = lane_routes(&mut world, used_edges);
            for vehicle in 0..count {
                world
                    .spawn_vehicle(input(
                        routes[(vehicle % used_edges) as usize],
                        0,
                        (vehicle / used_edges + 1) * 10_000,
                    ))
                    .unwrap();
            }
            world.derived.spawn_overlap = Default::default();
            reset_overlap_blocker_inspections();
            crate::admin::cutover_migration::revalidate_migrated_vehicles(&mut world).unwrap();
            let candidates = overlap_blocker_inspections();
            let expected: u64 = (0..used_edges)
                .map(|edge| {
                    let n = u64::from(count / used_edges + u32::from(edge < count % used_edges));
                    n * (n - 1)
                })
                .sum();
            assert_eq!(candidates as u64, expected);
            println!(
                "revalidation count={count} edges={used_edges} candidates={candidates} baseline_candidates={} index_bytes={}",
                u64::from(count) * u64::from(count - 1),
                world.derived.spawn_overlap.retained_logical_bytes()
            );
        }
    }
}

#[test]
fn fallible_overlap_rebuild_retries_after_partial_allocation() {
    use crate::kernel::spawn_overlap::with_overlap_allocation_failure_after;
    let mut world = empty_world(lane_revision(4), 4);
    for route in lane_routes(&mut world, 4) {
        world.spawn_vehicle(input(route, 0, 0)).unwrap();
    }
    let before = world.capture_snapshot().unwrap();
    let digest = crate::deterministic_state_digest(&before).unwrap();
    for fail_after in 0..3 {
        world.derived.spawn_overlap = Default::default();
        assert_eq!(
            with_overlap_allocation_failure_after(fail_after, || world.try_refresh_overlap_index()),
            Err(())
        );
        assert!(!world.derived.spawn_overlap.is_current());
        world.try_refresh_overlap_index().unwrap();
        for handle in world.live_vehicles() {
            let state = world.vehicle_state(*handle).unwrap();
            assert_eq!(
                world.indexed_overlap_blocker(state.route, 0, 0, state.length_mm, Some(*handle)),
                None
            );
            assert_eq!(
                world.indexed_overlap_blocker(state.route, 0, 0, state.length_mm, None),
                Some(*handle)
            );
        }
        assert_eq!(
            crate::deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap(),
            digest
        );
    }
}

#[test]
fn cutover_overlap_keeps_live_order_and_entity_error_priority() {
    use crate::admin::cutover_migration::revalidate_migrated_vehicles;
    let mut world = empty_world(lane_revision(1), 2);
    let route = lane_routes(&mut world, 1)[0];
    let first = world.spawn_vehicle(input(route, 0, 0)).unwrap();
    let second = world.spawn_vehicle(input(route, 0, 10_000)).unwrap();
    world.committed.vehicles[second.index() as usize]
        .state
        .as_mut()
        .unwrap()
        .progress_mm = 0;
    world.derived.spawn_overlap.mark_stale();
    assert_eq!(
        revalidate_migrated_vehicles(&mut world),
        Err(crate::CutoverError::VehicleRevalidationFailed {
            vehicle: first.index()
        })
    );
    world.committed.live_order.reverse();
    world.rebuild_active_order();
    assert_eq!(
        revalidate_migrated_vehicles(&mut world),
        Err(crate::CutoverError::VehicleRevalidationFailed {
            vehicle: second.index()
        })
    );
    world.committed.vehicles[second.index() as usize]
        .state
        .as_mut()
        .unwrap()
        .length_mm = 4_499;
    world.derived.spawn_overlap.mark_stale();
    assert_eq!(
        revalidate_migrated_vehicles(&mut world),
        Err(crate::CutoverError::ProfileDerivationMismatch {
            vehicle: second.index()
        })
    );
}

#[test]
fn route_start_is_clipped_and_blocker_selection_survives_slot_reuse() {
    let mut world = empty_world(road_revision(1, 10.0, false), 8);
    let route = lane_routes(&mut world, 1)[0];
    let zero = world.spawn_vehicle(input(route, 0, 0)).unwrap();
    // 路线外的车尾不制造负坐标区间，也不阻止有实际占用的新车。
    let low = world.spawn_vehicle(input(route, 0, 2_000)).unwrap();
    let high = world.spawn_vehicle(input(route, 0, 7_000)).unwrap();
    assert_eq!(world.overlap_blocker(route, 0, 0, 4_500), Some(zero));
    assert_eq!(world.overlap_blocker(route, 0, 6_000, 4_500), Some(low));
    world.despawn_vehicle(zero).unwrap();
    let reused = world.spawn_vehicle(input(route, 0, 0)).unwrap();
    assert_eq!(reused.index(), zero.index());
    world.despawn_vehicle(reused).unwrap();
    world.despawn_vehicle(low).unwrap();
    let low_again = world.spawn_vehicle(input(route, 0, 2_000)).unwrap();
    assert_eq!(low_again.index(), low.index());
    assert_ne!(low_again.generation(), low.generation());
    assert_eq!(world.live_vehicles(), &[high, low_again]);
    for rebuild in [false, true] {
        if rebuild {
            world.derived.spawn_overlap.mark_stale();
        }
        assert_eq!(
            world.overlap_blocker(route, 0, 6_000, 4_500),
            Some(low_again)
        );
    }
}

#[test]
fn ticks_completion_replace_and_failed_commands_keep_admission_current() {
    let mut world = empty_world(road_revision(2, 10.0, false), 8);
    let routes = lane_routes(&mut world, 2);
    let moving = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            routes[0],
            0,
            1_000,
            10_000,
        ))
        .unwrap();
    let completing = world.spawn_vehicle(input(routes[1], 0, 10_000)).unwrap();
    let snapshot = encode_lfrs(&world.capture_snapshot().unwrap());
    assert_eq!(
        world.spawn_vehicle(input(routes[0], 0, 1_000)),
        Err(SpawnError::Overlap)
    );
    assert!(world.step(TickInput::new(1)).is_err());
    assert_eq!(encode_lfrs(&world.capture_snapshot().unwrap()), snapshot);
    for _ in 0..6 {
        world.step(TickInput::new(100)).unwrap();
    }
    assert_eq!(
        world.vehicle_state(completing).unwrap().status,
        VehicleStatus::Completed
    );
    assert_eq!(world.overlap_blocker(routes[1], 0, 10_000, 4_500), None);
    assert_eq!(world.overlap_blocker(routes[0], 0, 1_000, 4_500), None);
    let position = world.vehicle_state(moving).unwrap().progress_mm;
    assert_eq!(
        world.overlap_blocker(routes[0], 0, position, 4_500),
        Some(moving)
    );
    let before = encode_lfrs(&world.capture_snapshot().unwrap());
    assert!(
        matches!(world.replace_completed_vehicle(completing, input(routes[0], 0, position)),
        Err(crate::ReplaceError::Blocked(block)) if block.blocker == moving)
    );
    assert_eq!(encode_lfrs(&world.capture_snapshot().unwrap()), before);
    let replacement = world
        .replace_completed_vehicle(completing, input(routes[1], 0, 5_000))
        .unwrap();
    assert_eq!(
        world.overlap_blocker(routes[1], 0, 5_000, 4_500),
        Some(replacement.new)
    );
    world.despawn_vehicle(replacement.new).unwrap();
    assert_eq!(world.overlap_blocker(routes[1], 0, 5_000, 4_500), None);

    // 重建缓存后才命中命令游标错误；失败不得留下幽灵占用。
    let cursor = world.committed.command_cursor;
    world.committed.command_cursor = u64::MAX;
    world.derived.spawn_overlap.mark_stale();
    assert_eq!(
        world.spawn_vehicle(input(routes[1], 0, 5_000)),
        Err(SpawnError::CommandCursorExhausted)
    );
    world.committed.command_cursor = cursor;
    world
        .spawn_vehicle(input(routes[1], 0, 5_000))
        .expect("retry after late validation failure");
}

#[test]
fn overlap_index_heap_is_counted_once_in_derived_owner() {
    let mut world = empty_world(road_revision(4, 10.0, false), 8);
    let route = lane_routes(&mut world, 1)[0];
    assert_eq!(world.derived.spawn_overlap.retained_logical_bytes(), 0);
    let before = world.derived.retained_logical_bytes();
    assert_eq!(world.overlap_blocker(route, 0, 1_000, 4_500), None);
    let cold_bytes = world.derived.spawn_overlap.retained_logical_bytes();
    assert!(cold_bytes > 0);
    assert_eq!(world.derived.retained_logical_bytes() - before, cold_bytes);
    world.spawn_vehicle(input(route, 0, 5_000)).unwrap();
    assert!(world.derived.spawn_overlap.retained_logical_bytes() > cold_bytes);
}

#[test]
fn restore_and_cutover_share_clipping_and_exclude_completed_vehicles() {
    let revision = road_revision(2, 10.0, false);
    let mut world = empty_world(Arc::clone(&revision), 8);
    let routes = lane_routes(&mut world, 2);
    world.spawn_vehicle(input(routes[1], 0, 10_000)).unwrap();
    world.step(TickInput::new(100)).unwrap();
    world.spawn_vehicle(input(routes[0], 0, 2_000)).unwrap();
    world.spawn_vehicle(input(routes[0], 0, 0)).unwrap();
    let captured = world.capture_snapshot().unwrap();
    let restored = restore_lfrs(
        &encode_lfrs(&captured),
        revision,
        world.committed_source().clone(),
        world.config(),
        SnapshotRestoreLimits::new(1_048_576, 1_024),
    )
    .expect("clipped body and noncoincident entry point coexist through restore");
    let restored_routes: Vec<_> = routes
        .iter()
        .map(|route| {
            restored
                .route_mappings()
                .iter()
                .find(|(_, handle)| {
                    restored.world().route_edges(*handle) == world.route_edges(*route)
                })
                .unwrap()
                .1
        })
        .collect();
    let mut restored = restored.into_world();
    for route in &restored_routes {
        for progress in [0, 1_000, 5_000, 10_000] {
            let expected = linear_blocker(&restored, *route, 0, progress);
            assert_eq!(
                restored.overlap_blocker(*route, 0, progress, 4_500),
                expected
            );
        }
    }
    let target = road_revision(2, 10.0, false);
    let descriptor = crate::NetworkRevisionCutoverDescriptor::new(
        crate::LfcaOriginBinding::from_canonical_origin(*restored.revision().canonical_origin()),
        crate::LfcaOriginBinding::from_canonical_origin(*target.canonical_origin()),
        None,
        crate::MigrationPolicyKind::SameRevisionRestore,
        restored.world_binding(),
    );
    let _commit = restored
        .cutover_same_revision(
            target,
            restored.committed_source().clone(),
            &descriptor,
            &crate::CutoverPreflightLimits::new(1_048_576),
        )
        .expect("cutover full-pair revalidation uses clipped footprints too");
    assert!(
        restored
            .overlap_blocker(restored_routes[0], 0, 0, 4_500)
            .is_some()
    );
    assert_eq!(
        restored.spawn_vehicle(input(restored_routes[0], 0, 5_000)),
        Err(SpawnError::Overlap)
    );
    restored
        .spawn_vehicle(input(restored_routes[1], 0, 10_000))
        .expect("completed is not indexed");
}

#[test]
fn zero_entry_admission_preserves_non_overlap_on_the_next_tick() {
    let mut world = empty_world(road_revision(1, 10.0, false), 8);
    let route = lane_routes(&mut world, 1)[0];
    let first = world.spawn_vehicle(input(route, 0, 0)).unwrap();
    assert_eq!(
        world.spawn_vehicle(input(route, 0, 0)),
        Err(SpawnError::Overlap)
    );
    world.step(TickInput::new(100)).unwrap();
    let second = world.spawn_vehicle(input(route, 0, 0)).unwrap();
    world.step(TickInput::new(100)).unwrap();
    let a = world.vehicle_state(first).unwrap();
    let b = world.vehicle_state(second).unwrap();
    assert!(
        !crate::kernel::tables::bodies_overlap(
            world.traffic().lane_lengths_millimetres(),
            world.route_edges(route).unwrap(),
            0,
            a.progress_mm,
            a.length_mm,
            world.route_edges(route).unwrap(),
            0,
            b.progress_mm,
            b.length_mm,
        ),
        "next tick created overlap: first={a:?}, second={b:?}"
    );

    let mut crossing = empty_world(road_revision(2, 10.0, true), 8);
    let first_edge = LaneEdgeOrdinal::from_raw(0);
    let second_edge = crossing.traffic().successors(first_edge).unwrap()[0];
    let through = crossing
        .register_route(RouteRegisterInput::new(vec![first_edge, second_edge]))
        .unwrap();
    let entry = crossing
        .register_route(RouteRegisterInput::new(vec![second_edge]))
        .unwrap();
    crossing.spawn_vehicle(input(through, 1, 0)).unwrap();
    assert_eq!(
        crossing.spawn_vehicle(input(entry, 0, 0)),
        Err(SpawnError::Overlap),
        "zero progress entry points are physical, not route-occurrence identities"
    );
}
