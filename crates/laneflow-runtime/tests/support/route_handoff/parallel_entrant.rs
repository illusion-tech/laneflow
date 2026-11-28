use super::*;
use laneflow_runtime::{
    RouteHandoffEntrantReason, RouteHandoffRequest, TickInput, TrafficWorld, VehicleHandle,
    VehicleSpawnInput,
};

struct Scene {
    world: TrafficWorld,
    request: RouteHandoffRequest,
    straight: VehicleHandle,
}

enum Arrival {
    StraightFirst,
    ChangerFirst,
}

fn scene(arrival: Arrival) -> Scene {
    let (straight_index, straight_front_mm, straight_speed_mm_s, changer_speed_mm_s) = match arrival
    {
        Arrival::StraightFirst => (1, 11_800, 8_000, 8_000),
        Arrival::ChangerFirst => (0, 4_200, 2_000, 3_000),
    };
    let revision = compile_road_editing_revision(road_module(
        0.,
        None,
        RoadEditingLaneDirection::Forward,
        Some(stop_line::SourceStop::ParallelFork),
    ));
    let config = WorldConfig::try_new(WorldConfigInput {
        vehicle_capacity: 2,
        route_capacity: 2,
        route_edge_occurrence_capacity: 16,
        route_conflict_occurrence_capacity: 8,
        fixed_delta_time_ms: 100,
    })
    .unwrap()
    .with_handoff_driving(registration::driving());
    let mut world = install_fixture(Arc::clone(&revision), config).unwrap();
    let (section, target, source) = stop_line::road_lanes(&revision);
    let changer_edges = vec![source[0], source[1], target[2], target[3]];
    let fork = revision
        .traffic()
        .successors(target[1])
        .unwrap()
        .iter()
        .copied()
        .find(|&e| e != target[2])
        .unwrap();
    let straight_edges = vec![target[0], target[1], fork];
    assert!(straight_edges.iter().all(|e| !changer_edges.contains(e)));
    let occurrence = RouteHandoffOccurrence {
        revision: revision.network_revision(),
        source_index: 1,
        owner: BoundaryOwner::Section(section),
        left_index: 0,
        stations_mm: [24_000, 26_000],
    };
    let route_input = RouteRegisterInput::new(changer_edges).with_handoffs(vec![occurrence]);
    let binding = world.bind_route_handoff(&route_input, occurrence).unwrap();
    let route = world.register_route(route_input).unwrap();
    let straight_route = world
        .register_route(RouteRegisterInput::new(straight_edges))
        .unwrap();
    let profile = conflict_profile(&revision, "car");
    let straight = world
        .spawn_vehicle(VehicleSpawnInput::new(
            profile,
            straight_route,
            straight_index,
            straight_front_mm,
            straight_speed_mm_s,
        ))
        .unwrap();
    let mut trial = stop_line::car_trial(&revision, [10_000, 40_000]);
    trial.speed_m_s = f64::from(changer_speed_mm_s) / 1_000.;
    let approaches = world
        .preview_route_handoff_approaches(
            &binding,
            1,
            &trial,
            0..=30_000,
            TRAVEL_TOLERANCE_MM,
            u32::MAX,
        )
        .unwrap();
    let swept = approaches.swept_sides[1].as_ref().unwrap();
    assert!(swept.addresses().any(|a| a.edge == target[1]));
    if matches!(arrival, Arrival::StraightFirst) {
        // 前杠交接前，目标侧已有一个实体点落在平行直行车当前的车身长度内。
        let preview = world.preview_route_handoff(&binding, &trial).unwrap();
        let sample = preview
            .sample_after_travel(0, 8_500, TRAVEL_TOLERANCE_MM, trial.limits)
            .unwrap()
            .sample
            .unwrap();
        let witness = sample.footprint.sides[1]
            .as_ref()
            .unwrap()
            .witness
            .stations_mm();
        let front = 10_000 + u64::from(straight_front_mm);
        let length = u64::from(world.vehicle(straight).unwrap().length_mm());
        assert!(
            front - length <= witness[0] && witness[1] <= front,
            "{witness:?}"
        );
        let blocker = approaches.entrants.side_blockers[1].unwrap();
        assert_eq!(
            (blocker.vehicle, blocker.reason),
            (straight, RouteHandoffEntrantReason::Occupied)
        );
    } else {
        assert!(approaches.entrants.side_blockers[1].is_none());
    }
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            profile,
            route,
            1,
            3_400,
            changer_speed_mm_s,
        ))
        .unwrap();
    Scene {
        world,
        request: RouteHandoffRequest {
            vehicle,
            route,
            revision: revision.network_revision(),
            source_index: 1,
            station_window_mm: trial.station_window_mm,
            rear_stations_mm: trial.rear_stations_mm,
            speed_m_s: trial.speed_m_s,
            limits: trial.limits,
        },
        straight,
    }
}

#[test]
fn parallel_fork_occupant_prevents_handoff_until_its_tail_leaves() {
    let Scene {
        mut world,
        mut request,
        straight,
    } = scene(Arrival::StraightFirst);
    let mut held = false;
    for _ in 0..120 {
        let parallel = world.vehicle(straight).unwrap();
        let clear =
            parallel.route_edge_index() == 2 && parallel.progress_mm() >= parallel.length_mm();
        if !held {
            // 每个未获准的申请仅在当拍有效。以本拍前杠重算后轴起点，
            // 收短终点，使新的候选仍以已登记的 25 m 交接窗口中心对称。
            let changer = world.vehicle(request.vehicle).unwrap();
            assert_eq!(changer.route_edge_index(), 1);
            let rear = 10_000 + u64::from(changer.rounded_front_progress_mm()) - 3_400;
            request.rear_stations_mm = [rear, 50_000 - rear];
            world
                .step_with_route_handoffs(TickInput::new(100), &[request])
                .unwrap();
        } else {
            world.step(TickInput::new(100)).unwrap();
        }
        if world
            .vehicle_route_handoffs(request.vehicle)
            .next()
            .is_some()
        {
            assert!(clear);
            held = true;
        } else if held {
            assert!(world.vehicle(request.vehicle).unwrap().route_edge_index() >= 2);
            return;
        }
    }
    panic!("直行车驶出分岔后，换道车未在 12 秒内完成交接");
}

// 从上一拍存档独立读取目标侧剩余的实际保护范围。尾部后方的保护会逐拍释放。
fn first_target_protection(world: &TrafficWorld) -> Option<u32> {
    let bytes = laneflow_runtime::encode_lfrs(&world.capture_snapshot().unwrap());
    let wire = crate::snapshot_wire::size_prefixed_root_as_runtime_snapshot(&bytes).unwrap();
    let held = wire
        .vehicles()
        .iter()
        .find(|v| !v.handoffs().is_empty())?
        .handoffs()
        .get(0);
    let rev = world.revision();
    let (_, target, _) = stop_line::road_lanes(&rev);
    target.iter().zip([0, 10_000]).find_map(|(&edge, offset)| {
        let id = crate::snapshot_wire::StableId128::new(
            rev.identity()
                .stable_id(edge)
                .unwrap()
                .as_untyped()
                .as_bytes(),
        );
        held.protections()
            .iter()
            .filter(|p| {
                p.role() == crate::snapshot_wire::HandoffProtectionRole::Target && *p.edge() == id
            })
            .map(|p| p.start_mm() + offset)
            .min()
    })
}

#[test]
fn parallel_fork_approach_waits_outside_the_reserved_sweep_then_passes() {
    let Scene {
        mut world,
        request,
        straight,
    } = scene(Arrival::ChangerFirst);
    world
        .step_with_route_handoffs(TickInput::new(100), &[request])
        .unwrap();
    assert!(
        world
            .vehicle_route_handoffs(request.vehicle)
            .next()
            .is_some()
    );
    assert!(first_target_protection(&world).is_some());
    let mut braked = false;
    let mut passed = false;
    for _ in 0..180 {
        let prior_speed = world.vehicle(straight).unwrap().speed_mm_s();
        let protected = first_target_protection(&world);
        world.step(TickInput::new(100)).unwrap();
        let parallel = world.vehicle(straight).unwrap();
        if let Some(start) = protected {
            assert!(parallel.route_edge_index() < 2);
            let station =
                [0, 10_000][parallel.route_edge_index() as usize] + parallel.progress_mm();
            assert!(station < start);
            braked |= parallel.speed_mm_s() < prior_speed;
        } else if parallel.route_edge_index() == 2 && parallel.progress_mm() > parallel.length_mm()
        {
            passed = true;
        }
        if passed
            && world
                .vehicle_route_handoffs(request.vehicle)
                .next()
                .is_none()
        {
            assert!(braked);
            return;
        }
    }
    panic!("分岔保护解除后，两车未在 18 秒内通过");
}
