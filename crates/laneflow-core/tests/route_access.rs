//! (ParticipantClass, Route) 绑定期静态准入校验测试（SSOT §6.5）。
//!
//! 覆盖五个绑定点（spawn / replace / spawn_parked / leave_parking /
//! rebind_reserved_route）的原子拒绝与结构化归因、cursor 可达后缀语义、
//! pending occurrence 确切范围、跨平面合取与 `register_route` 的
//! class-agnostic 语义。

use laneflow_core::{
    AccessEffect, AccessRegistry, AccessRule, AccessTargetId, CoreError, CoreWorld,
    CrossSectionRegistry, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, Junction,
    JunctionRegistry, LaneEdge, LaneGraph, LeaveParkingInput, ManeuverPath, Movement,
    ParkedVehicleSpawnInput, ParkingArea, ParkingCommandEffect, ParkingRegistry, ParkingSpace,
    ParkingSpaceGeometry, ParticipantClass, ParticipantClassRegistry,
    RebindReservedVehicleRouteInput, Route, SignalRegistry, Speed, SpeedLimit, VehicleHandle,
    VehicleProfile, VehicleProfileHandle, VehicleProfileRegistry, VehicleReplaceExternalId,
    VehicleReplaceInput, VehicleReplaceOutcome, VehicleSpawnInput,
};

fn edge(id: &str, next: &[&str]) -> LaneEdge {
    LaneEdge::new(
        id,
        EdgeLength::try_new(100.0).expect("test edge length"),
        SpeedLimit::try_new(f64::MAX).expect("test speed limit"),
        next.iter().copied(),
    )
}

/// 直线链 fixture 图：`e0 -> e1 -> e2 -> e3`。
fn chain_graph() -> LaneGraph {
    LaneGraph::try_new([
        edge("e0", &["e1"]),
        edge("e1", &["e2"]),
        edge("e2", &["e3"]),
        edge("e3", &[]),
    ])
    .expect("test chain graph")
}

/// junction fixture 图：`e0 -> e1 -> j1 -> e2 -> e3`，`j1` 是 internal edge。
fn junction_graph() -> LaneGraph {
    LaneGraph::try_new([
        edge("e0", &["e1"]),
        edge("e1", &["j1"]),
        edge("j1", &["e2"]),
        edge("e2", &["e3"]),
        edge("e3", &[]),
    ])
    .expect("test junction graph")
}

/// `path-1`：entry `e1`、internal [`j1`]、exit `e2`。
fn junctions(graph: &LaneGraph) -> JunctionRegistry {
    JunctionRegistry::try_new(
        graph,
        [Junction::new("junction-1")],
        [Movement::new("movement-1", "junction-1")],
        [ManeuverPath::new(
            "path-1",
            "movement-1",
            "e1",
            ["j1"],
            "e2",
        )],
    )
    .expect("test junctions")
}

fn classes() -> ParticipantClassRegistry {
    ParticipantClassRegistry::try_new(vec![
        ParticipantClass::new("motorVehicle", None),
        ParticipantClass::new("car", Some("motorVehicle")),
        ParticipantClass::new("bus", Some("motorVehicle")),
    ])
    .expect("test classes")
}

fn profile(id: &str, class_id: &str) -> VehicleProfile {
    let classes = classes();
    VehicleProfile::try_new_iidm(
        id,
        classes.class_handle(class_id).expect("class must exist"),
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
    .expect("test profile")
}

fn profiles() -> VehicleProfileRegistry {
    VehicleProfileRegistry::try_new(
        &classes(),
        [profile("car-profile", "car"), profile("bus-profile", "bus")],
    )
    .expect("test profiles")
}

struct TestWorld {
    world: CoreWorld,
    car: VehicleProfileHandle,
    bus: VehicleProfileHandle,
}

fn build_world(
    graph: LaneGraph,
    junctions: JunctionRegistry,
    parking: ParkingRegistry,
    routes: Vec<Route>,
    rules: Vec<AccessRule>,
) -> TestWorld {
    let classes = classes();
    let access = AccessRegistry::try_new(
        &graph,
        &junctions,
        &CrossSectionRegistry::empty(),
        &classes,
        rules,
    )
    .expect("valid access registry");
    let profiles = profiles();
    let car = profiles.profile_handle("car-profile").expect("car profile");
    let bus = profiles.profile_handle("bus-profile").expect("bus profile");
    let traffic = InitialTrafficData::try_new(
        graph,
        routes,
        profiles,
        junctions,
        SignalRegistry::empty(),
        parking,
        classes,
        CrossSectionRegistry::empty(),
        access,
    )
    .expect("valid traffic");
    let world = CoreWorld::with_traffic_data(1_000, traffic, Vec::new()).expect("test world");
    TestWorld { world, car, bus }
}

fn chain_world(routes: Vec<Route>, rules: Vec<AccessRule>) -> TestWorld {
    build_world(
        chain_graph(),
        JunctionRegistry::empty(),
        ParkingRegistry::empty(),
        routes,
        rules,
    )
}

fn junction_world(rules: Vec<AccessRule>) -> TestWorld {
    let graph = junction_graph();
    let junctions = junctions(&graph);
    build_world(
        graph,
        junctions,
        ParkingRegistry::empty(),
        vec![Route::try_new("RJ", ["e0", "e1", "j1", "e2", "e3"]).expect("junction route")],
        rules,
    )
}

/// `path-1` occurrence：entry route edge index 1、exit route edge index 3。
fn deny_path_rule() -> AccessRule {
    AccessRule::new(
        "rule-deny-path",
        AccessTargetId::maneuver_path("path-1"),
        AccessEffect::Deny,
        ["car"],
    )
}

fn spawn_active(
    world: &mut CoreWorld,
    profile: VehicleProfileHandle,
    id: &str,
    route: &str,
    cursor: usize,
) -> Result<VehicleHandle, CoreError> {
    world.spawn_vehicle(VehicleSpawnInput::active(
        id,
        profile,
        route,
        cursor,
        EdgeProgress::ZERO,
        Speed::ZERO,
    ))
}

/// 断言绑定期拒绝的结构化归因：profile/route、平面、edge index 或 occurrence
/// entry index、胜者 rule。
fn assert_denied(
    error: &CoreError,
    profile_id: &str,
    route_id: &str,
    plane: &'static str,
    route_edge_index: usize,
    target_id: &str,
    rule_id: &str,
) {
    std::assert_matches!(
        error,
        CoreError::RouteAccessDenied {
            profile_id: actual_profile,
            route_id: actual_route,
            plane: actual_plane,
            route_edge_index: actual_index,
            target_id: actual_target,
            rule_id: actual_rule,
        } if actual_profile == profile_id
            && actual_route == route_id
            && *actual_plane == plane
            && *actual_index == route_edge_index
            && actual_target == target_id
            && actual_rule == rule_id,
        "unexpected RouteAccessDenied attribution: {error:?}"
    );
}

#[test]
fn spawn_denied_by_edge_rule_is_atomic_with_attribution() {
    let TestWorld {
        mut world,
        car,
        bus,
    } = chain_world(
        vec![Route::try_new("R", ["e0", "e1", "e2", "e3"]).expect("route")],
        vec![AccessRule::new(
            "rule-deny-e1",
            AccessTargetId::lane_edge("e1"),
            AccessEffect::Deny,
            ["car"],
        )],
    );

    let before = world.clone();
    let error = spawn_active(&mut world, car, "v-car", "R", 0).expect_err("car must be denied");
    assert_denied(&error, "car-profile", "R", "edge", 1, "e1", "rule-deny-e1");
    assert_eq!(world, before, "denied spawn must leave world unchanged");

    // 规则只匹配 car：bus 无约束，放行。
    spawn_active(&mut world, bus, "v-bus", "R", 0).expect("bus must pass");
}

#[test]
fn spawn_denied_by_path_rule_covers_future_and_in_progress_occurrences() {
    let TestWorld { mut world, car, .. } = junction_world(vec![deny_path_rule()]);

    let before = world.clone();
    // `cursor < entry`（0 < 1）：未来 occurrence 作为原子整体校验。
    let future = spawn_active(&mut world, car, "v-future", "RJ", 0)
        .expect_err("future occurrence must be denied");
    assert_denied(
        &future,
        "car-profile",
        "RJ",
        "path",
        1,
        "path-1",
        "rule-deny-path",
    );
    // `entry <= cursor < exit`（1 <= 1, 2 < 3）：进行中 occurrence 同样原子校验。
    let at_entry = spawn_active(&mut world, car, "v-entry", "RJ", 1)
        .expect_err("in-progress occurrence at entry must be denied");
    assert_denied(
        &at_entry,
        "car-profile",
        "RJ",
        "path",
        1,
        "path-1",
        "rule-deny-path",
    );
    let inside = spawn_active(&mut world, car, "v-inside", "RJ", 2)
        .expect_err("in-progress occurrence must be denied");
    assert_denied(
        &inside,
        "car-profile",
        "RJ",
        "path",
        1,
        "path-1",
        "rule-deny-path",
    );
    assert_eq!(world, before, "denied spawns must leave world unchanged");
}

#[test]
fn occurrence_with_exit_at_or_before_cursor_is_not_validated() {
    let TestWorld { mut world, car, .. } = junction_world(vec![deny_path_rule()]);

    // `cursor == exit`（3）：traversal 已完成，occurrence 不参与校验；exit edge `e2`
    // 本身在 edge 平面无约束。
    spawn_active(&mut world, car, "v-exit", "RJ", 3).expect("exit cursor must skip occurrence");
    // `cursor > exit`（4）：cursor 之前的 occurrence 不参与校验。
    spawn_active(&mut world, car, "v-after", "RJ", 4).expect("past occurrence must not validate");
}

#[test]
fn edges_before_cursor_are_not_validated() {
    let TestWorld { mut world, car, .. } = chain_world(
        vec![Route::try_new("R", ["e0", "e1", "e2", "e3"]).expect("route")],
        vec![AccessRule::new(
            "rule-deny-e0",
            AccessTargetId::lane_edge("e0"),
            AccessEffect::Deny,
            ["car"],
        )],
    );

    let before = world.clone();
    let error = spawn_active(&mut world, car, "v-denied", "R", 0)
        .expect_err("deny at cursor edge must reject");
    assert_denied(&error, "car-profile", "R", "edge", 0, "e0", "rule-deny-e0");
    assert_eq!(world, before);

    // cursor 之前的 edge 不参与校验。
    spawn_active(&mut world, car, "v-passed", "R", 1)
        .expect("deny before cursor must not validate");
}

#[test]
fn edge_plane_allow_does_not_override_path_plane_deny() {
    let TestWorld { mut world, car, .. } = junction_world(vec![
        AccessRule::new(
            "rule-allow-e0",
            AccessTargetId::lane_edge("e0"),
            AccessEffect::Allow,
            ["car"],
        ),
        deny_path_rule(),
    ]);

    // 跨平面合取：edge 平面 allow 不解除 path 平面 deny。
    let before = world.clone();
    let error = spawn_active(&mut world, car, "v-car", "RJ", 0)
        .expect_err("path deny must win across planes");
    assert_denied(
        &error,
        "car-profile",
        "RJ",
        "path",
        1,
        "path-1",
        "rule-deny-path",
    );
    assert_eq!(world, before);
}

#[test]
fn allow_rule_and_empty_rules_both_pass() {
    let TestWorld { mut world, car, .. } = junction_world(vec![AccessRule::new(
        "rule-allow-path",
        AccessTargetId::maneuver_path("path-1"),
        AccessEffect::Allow,
        ["car"],
    )]);
    spawn_active(&mut world, car, "v-allow", "RJ", 0).expect("explicit allow must pass");

    let TestWorld {
        world: mut unconstrained,
        car,
        ..
    } = junction_world(Vec::new());
    spawn_active(&mut unconstrained, car, "v-default", "RJ", 0)
        .expect("no rules must be unconstrained");
}

#[test]
fn register_route_is_class_agnostic() {
    let TestWorld {
        mut world,
        car,
        bus,
    } = chain_world(
        vec![Route::try_new("R1", ["e1", "e2", "e3"]).expect("initial route")],
        vec![AccessRule::new(
            "rule-deny-e0",
            AccessTargetId::lane_edge("e0"),
            AccessEffect::Deny,
            ["car"],
        )],
    );

    // Route 保持 class-agnostic：带 deny edge 的 route 可注册，绑定时才拒绝。
    let denied_route = world
        .register_route(Route::try_new("R2", ["e0", "e1"]).expect("denied route shape"))
        .expect("register_route performs no access judgment");

    let before = world.clone();
    let error = spawn_active(&mut world, car, "v-car", "R2", 0)
        .expect_err("binding a denied (class, route) must fail");
    assert_denied(&error, "car-profile", "R2", "edge", 0, "e0", "rule-deny-e0");
    assert_eq!(world, before);

    spawn_active(&mut world, bus, "v-bus", "R2", 0).expect("bus binding must pass");
    assert_eq!(
        world.route_external_id(denied_route),
        Some("R2"),
        "denied route remains registered"
    );
}

#[test]
fn replace_denied_by_edge_rule_is_atomic_with_attribution() {
    let TestWorld {
        mut world,
        car,
        bus,
    } = chain_world(
        vec![
            Route::try_new("old-route", ["e0"]).expect("old route"),
            Route::try_new("R", ["e0", "e1", "e2", "e3"]).expect("route"),
        ],
        vec![AccessRule::new(
            "rule-deny-e1",
            AccessTargetId::lane_edge("e1"),
            AccessEffect::Deny,
            ["car"],
        )],
    );
    let old = world
        .spawn_vehicle(VehicleSpawnInput::completed(
            "old",
            car,
            "old-route",
            0,
            EdgeProgress::try_new(100.0).expect("route end"),
        ))
        .expect("completed old vehicle");
    let target = world.route_handle("R").expect("route handle");

    let before = world.clone();
    let input = VehicleReplaceInput::new(
        VehicleReplaceExternalId::Preserve,
        car,
        target,
        0,
        EdgeProgress::ZERO,
        Speed::ZERO,
    );
    let error = world
        .replace_completed_vehicle(old, &input)
        .expect_err("denied replacement binding must fail");
    assert_denied(&error, "car-profile", "R", "edge", 1, "e1", "rule-deny-e1");
    assert_eq!(world, before, "denied replace must leave world unchanged");

    let bus_input = VehicleReplaceInput::new(
        VehicleReplaceExternalId::Preserve,
        bus,
        target,
        0,
        EdgeProgress::ZERO,
        Speed::ZERO,
    );
    let outcome = world
        .replace_completed_vehicle(old, &bus_input)
        .expect("bus replacement binding must pass");
    assert!(matches!(outcome, VehicleReplaceOutcome::Replaced(_)));
}

fn parking_space(id: &str, area: Option<&str>, entry: &str, exit: &str) -> ParkingSpace {
    ParkingSpace::new(
        id,
        area.map(str::to_owned),
        entry,
        20.0,
        exit,
        40.0,
        ParkingSpaceGeometry::new(-3.0, 0.0, 5.0, 2.4),
    )
}

/// parking fixture：`S` 的 entry/exit 都在 `e1`；`R1=[e0,e1]` 干净、
/// `R2=[e1,e2]` 在 suffix 内含被 deny 的 `e2`、`R3=[e1]` 干净。
fn parking_world() -> TestWorld {
    let graph = chain_graph();
    let parking = ParkingRegistry::try_new(
        &graph,
        [ParkingArea::new("lot")],
        [parking_space("S", Some("lot"), "e1", "e1")],
    )
    .expect("test parking");
    build_world(
        graph,
        JunctionRegistry::empty(),
        parking,
        vec![
            Route::try_new("R1", ["e0", "e1"]).expect("clean route"),
            Route::try_new("R2", ["e1", "e2"]).expect("denied route"),
            Route::try_new("R3", ["e1"]).expect("rebind route"),
        ],
        vec![AccessRule::new(
            "rule-deny-e2",
            AccessTargetId::lane_edge("e2"),
            AccessEffect::Deny,
            ["car"],
        )],
    )
}

#[test]
fn spawn_parked_denied_is_atomic_with_attribution() {
    let TestWorld {
        mut world,
        car,
        bus,
    } = parking_world();
    let space = world.parking().space_handle("S").expect("space");

    let before = world.clone();
    let error = world
        .spawn_parked_vehicle(ParkedVehicleSpawnInput {
            id: "parked-car".to_owned(),
            profile: car,
            route_id: "R2".to_owned(),
            route_edge_index: 0,
            space,
        })
        .expect_err("denied parked binding must fail");
    assert_denied(&error, "car-profile", "R2", "edge", 1, "e2", "rule-deny-e2");
    assert_eq!(
        world, before,
        "denied parked spawn must leave world unchanged"
    );

    world
        .spawn_parked_vehicle(ParkedVehicleSpawnInput {
            id: "parked-bus".to_owned(),
            profile: bus,
            route_id: "R2".to_owned(),
            route_edge_index: 0,
            space,
        })
        .expect("bus parked binding must pass");
}

#[test]
fn leave_parking_denied_is_atomic_with_attribution() {
    let TestWorld { mut world, car, .. } = parking_world();
    let space = world.parking().space_handle("S").expect("space");
    let parked = world
        .spawn_parked_vehicle(ParkedVehicleSpawnInput {
            id: "parked-car".to_owned(),
            profile: car,
            route_id: "R1".to_owned(),
            route_edge_index: 1,
            space,
        })
        .expect("clean parked binding must pass")
        .vehicle;
    let denied_route = world.route_handle("R2").expect("denied route");

    let before = world.clone();
    let error = world
        .leave_parking(LeaveParkingInput {
            vehicle: parked,
            space,
            route: denied_route,
            route_edge_index: 0,
        })
        .expect_err("denied leave binding must fail");
    assert_denied(&error, "car-profile", "R2", "edge", 1, "e2", "rule-deny-e2");
    assert_eq!(world, before, "denied leave must leave world unchanged");

    let clean_route = world.route_handle("R1").expect("clean route");
    let record = world
        .leave_parking(LeaveParkingInput {
            vehicle: parked,
            space,
            route: clean_route,
            route_edge_index: 1,
        })
        .expect("clean leave binding must pass");
    assert_eq!(record.effect, ParkingCommandEffect::Applied);
}

#[test]
fn rebind_reserved_route_denied_is_atomic_with_attribution() {
    let TestWorld { mut world, car, .. } = parking_world();
    let space = world.parking().space_handle("S").expect("space");
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::active(
            "v-car",
            car,
            "R1",
            1,
            EdgeProgress::try_new(10.0).expect("progress"),
            Speed::ZERO,
        ))
        .expect("active vehicle");
    world
        .reserve_parking_space(vehicle, space)
        .expect("reservation");
    let denied_route = world.route_handle("R2").expect("denied route");

    let before = world.clone();
    let error = world
        .rebind_reserved_vehicle_route(RebindReservedVehicleRouteInput {
            vehicle,
            space,
            route: denied_route,
            route_edge_index: 0,
        })
        .expect_err("denied rebind must fail");
    assert_denied(&error, "car-profile", "R2", "edge", 1, "e2", "rule-deny-e2");
    assert_eq!(world, before, "denied rebind must leave world unchanged");

    let clean_route = world.route_handle("R3").expect("clean route");
    let record = world
        .rebind_reserved_vehicle_route(RebindReservedVehicleRouteInput {
            vehicle,
            space,
            route: clean_route,
            route_edge_index: 0,
        })
        .expect("clean rebind must pass");
    assert_eq!(record.effect, ParkingCommandEffect::Applied);
}
