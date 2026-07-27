use laneflow_core::{
    AccessCell, AccessEffect, AccessRule, AccessTargetId, CoreError, CorridorElementId,
    CrossSectionRegistry, EdgeLength, IidmProfileSpec, InitialTrafficData, LaneEdge, LaneGraph,
    RoadCorridor, RoadSection, Route, SectionLane, VehicleProfile, VehicleProfileRegistry,
};

fn edge_length(value: f64) -> EdgeLength {
    EdgeLength::try_new(value).expect("valid edge length")
}

fn edge(id: &str, next: &[&str]) -> LaneEdge {
    LaneEdge::new(
        id,
        edge_length(10.0),
        laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
        next.iter().copied(),
    )
}

fn canonical_graph() -> LaneGraph {
    LaneGraph::try_new([
        LaneEdge::new(
            "A",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["B"],
        ),
        LaneEdge::new(
            "B",
            edge_length(5.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["A"],
        ),
    ])
    .expect("valid lane graph")
}

fn canonical_profiles() -> VehicleProfileRegistry {
    VehicleProfileRegistry::try_new(
        &participant_classes().0,
        [VehicleProfile::try_new_iidm(
            "passenger-car",
            participant_classes().1,
            IidmProfileSpec {
                length: 4.5,
                desired_speed: 13.9,
                min_gap: 2.0,
                time_headway: 1.5,
                max_acceleration: 1.5,
                comfortable_deceleration: 2.0,
                emergency_deceleration: 6.0,
            },
        )
        .expect("valid profile")],
    )
    .expect("valid profile registry")
}

#[test]
fn valid_initial_traffic_data_preserves_input_order_and_registry() {
    let traffic_data = InitialTrafficData::try_new(
        canonical_graph(),
        [
            Route::try_new("loop", ["A", "B", "A"]).expect("valid route"),
            Route::try_new("short", ["A", "B"]).expect("valid route"),
        ],
        canonical_profiles(),
        laneflow_core::JunctionRegistry::empty(),
        laneflow_core::SignalRegistry::empty(),
        laneflow_core::ParkingRegistry::empty(),
        participant_classes().0,
        laneflow_core::CrossSectionRegistry::empty(),
        laneflow_core::AccessRegistry::empty(),
    )
    .expect("valid initial traffic data");

    assert_eq!(
        traffic_data.routes().map(Route::id).collect::<Vec<_>>(),
        ["loop", "short"]
    );
    assert_eq!(traffic_data.lane_graph().edges().len(), 2);
    assert_eq!(traffic_data.vehicle_profiles().len(), 1);

    assert!(traffic_data.junctions().is_empty());
    assert!(traffic_data.signals().is_empty());
    assert!(traffic_data.parking().is_empty());
    assert_eq!(traffic_data.participant_classes().class_count(), 2);
    assert!(traffic_data.cross_section().is_empty());
    assert!(traffic_data.access().is_empty());
}

#[test]
fn duplicate_initial_route_id_is_rejected_before_later_routes() {
    let error = InitialTrafficData::try_new(
        canonical_graph(),
        [
            Route::try_new("route", ["A"]).expect("valid route"),
            Route::try_new("route", ["B"]).expect("valid route"),
            Route::try_new("later", ["missing"]).expect("valid route shape"),
        ],
        VehicleProfileRegistry::empty(),
        laneflow_core::JunctionRegistry::empty(),
        laneflow_core::SignalRegistry::empty(),
        laneflow_core::ParkingRegistry::empty(),
        laneflow_core::ParticipantClassRegistry::empty(),
        laneflow_core::CrossSectionRegistry::empty(),
        laneflow_core::AccessRegistry::empty(),
    )
    .expect_err("duplicate route id must fail first");

    std::assert_matches!(
        error,
        CoreError::DuplicateRouteId { route_id } if route_id == "route"
    );
}

#[test]
fn initial_route_unknown_edge_uses_route_and_edge_input_order() {
    let error = InitialTrafficData::try_new(
        canonical_graph(),
        [
            Route::try_new("first", ["A", "first-missing", "second-missing"])
                .expect("valid route shape"),
            Route::try_new("second", ["third-missing"]).expect("valid route shape"),
        ],
        VehicleProfileRegistry::empty(),
        laneflow_core::JunctionRegistry::empty(),
        laneflow_core::SignalRegistry::empty(),
        laneflow_core::ParkingRegistry::empty(),
        laneflow_core::ParticipantClassRegistry::empty(),
        laneflow_core::CrossSectionRegistry::empty(),
        laneflow_core::AccessRegistry::empty(),
    )
    .expect_err("first unknown route edge must fail");

    std::assert_matches!(
        error,
        CoreError::UnknownRouteEdge { route_id, edge_id }
            if route_id == "first" && edge_id == "first-missing"
    );
}

#[test]
fn initial_route_continuity_uses_same_core_error_as_runtime_registration() {
    let graph = LaneGraph::try_new([
        LaneEdge::new(
            "A",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
        LaneEdge::new(
            "B",
            edge_length(5.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("valid disconnected graph");
    let error = InitialTrafficData::try_new(
        graph,
        [Route::try_new("disconnected", ["A", "B"]).expect("valid route shape")],
        VehicleProfileRegistry::empty(),
        laneflow_core::JunctionRegistry::empty(),
        laneflow_core::SignalRegistry::empty(),
        laneflow_core::ParkingRegistry::empty(),
        laneflow_core::ParticipantClassRegistry::empty(),
        laneflow_core::CrossSectionRegistry::empty(),
        laneflow_core::AccessRegistry::empty(),
    )
    .expect_err("disconnected route must fail");

    std::assert_matches!(
        error,
        CoreError::DisconnectedRouteEdge {
            route_id,
            from_edge_id,
            to_edge_id,
        } if route_id == "disconnected" && from_edge_id == "A" && to_edge_id == "B"
    );
}

/// 覆盖 edge `A` 的最小合法 cross-section（corridor/section/lane 完备 owner 树）。
fn cross_section_for(graph: &LaneGraph) -> CrossSectionRegistry {
    CrossSectionRegistry::try_new(
        graph,
        [],
        [RoadSection::new(
            "section-a",
            "motorLane",
            [SectionLane::new(["A"], None)],
        )],
        [],
        [RoadCorridor::new(
            "corridor-1",
            "section-a",
            [CorridorElementId::section("section-a")],
        )],
    )
    .expect("valid cross-section registry")
}

#[test]
fn assembly_rebinds_cross_section_and_access_to_final_lane_graph() {
    // caller 侧 registry 围绕 handle 排列相反的等价 graph 构造：edge `A` 在 caller
    // graph 中是 handle 1，在 final graph 中是 handle 0。若 try_new 不用最终
    // LaneGraph rebind cross_section/access，resolved 表会把 deny 错挂到 edge `B`。
    let caller_graph =
        LaneGraph::try_new([edge("B", &["A"]), edge("A", &["B"])]).expect("valid permuted graph");
    let caller_cross_section = cross_section_for(&caller_graph);
    let classes = participant_classes().0;
    let caller_access = laneflow_core::AccessRegistry::try_new(
        &caller_graph,
        &laneflow_core::JunctionRegistry::empty(),
        &caller_cross_section,
        &classes,
        vec![AccessRule::new(
            "rule-deny-a",
            AccessTargetId::lane_edge("A"),
            AccessEffect::Deny,
            ["car"],
        )],
    )
    .expect("valid access registry");

    let traffic_data = InitialTrafficData::try_new(
        canonical_graph(),
        [],
        VehicleProfileRegistry::empty(),
        laneflow_core::JunctionRegistry::empty(),
        laneflow_core::SignalRegistry::empty(),
        laneflow_core::ParkingRegistry::empty(),
        classes,
        caller_cross_section,
        caller_access,
    )
    .expect("valid initial traffic data");

    assert!(
        traffic_data
            .cross_section()
            .section_handle("section-a")
            .is_some()
    );
    assert!(traffic_data.access().rule_handle("rule-deny-a").is_some());
    let car = traffic_data
        .participant_classes()
        .class_handle("car")
        .expect("car class must exist");
    let edge_a = traffic_data
        .lane_graph()
        .edge_handle("A")
        .expect("edge A must exist");
    let edge_b = traffic_data
        .lane_graph()
        .edge_handle("B")
        .expect("edge B must exist");
    assert_eq!(
        traffic_data.access().edge_access(edge_a, car),
        AccessCell::Decided {
            rule: traffic_data
                .access()
                .rule_handle("rule-deny-a")
                .expect("rule must exist"),
            effect: AccessEffect::Deny,
        }
    );
    assert_eq!(
        traffic_data.access().edge_access(edge_b, car),
        AccessCell::Unconstrained
    );
}

fn participant_classes() -> (
    laneflow_core::ParticipantClassRegistry,
    laneflow_core::ParticipantClassHandle,
) {
    let classes = laneflow_core::ParticipantClassRegistry::try_new(vec![
        laneflow_core::ParticipantClass::new("motorVehicle", None),
        laneflow_core::ParticipantClass::new("car", Some("motorVehicle")),
    ])
    .expect("participant classes must be valid");
    let car = classes.class_handle("car").expect("car class must exist");
    (classes, car)
}

/// 声明顺序相反的等价 class registry（`car` 的 dense index 从 1 变为 0）。
fn reversed_participant_classes() -> laneflow_core::ParticipantClassRegistry {
    laneflow_core::ParticipantClassRegistry::try_new(vec![
        laneflow_core::ParticipantClass::new("car", Some("motorVehicle")),
        laneflow_core::ParticipantClass::new("motorVehicle", None),
    ])
    .expect("reversed class registry must be valid")
}

#[test]
fn assembly_rebinds_profiles_to_final_participant_classes() {
    // profiles 按 caller 侧 class registry（[motorVehicle, car]）构造，final
    // assembly 收到声明顺序相反的 registry。若不做 profiles rebind，旧 dense
    // index 1 会把 passenger-car 错挂到 final registry 的 `motorVehicle`。
    let traffic_data = InitialTrafficData::try_new(
        canonical_graph(),
        [],
        canonical_profiles(),
        laneflow_core::JunctionRegistry::empty(),
        laneflow_core::SignalRegistry::empty(),
        laneflow_core::ParkingRegistry::empty(),
        reversed_participant_classes(),
        laneflow_core::CrossSectionRegistry::empty(),
        laneflow_core::AccessRegistry::empty(),
    )
    .expect("valid initial traffic data");

    let profiles = traffic_data.vehicle_profiles();
    let handle = profiles
        .profile_handle("passenger-car")
        .expect("profile handle exists");
    let class = profiles
        .profile(handle)
        .map(VehicleProfile::participant_class)
        .expect("profile exists");
    assert_eq!(
        traffic_data.participant_classes().class_external_id(class),
        Some("car"),
        "final assembly 后 profile 的 class 语义必须仍是 car"
    );
}

#[test]
fn assembly_rejects_profile_class_missing_in_final_participant_classes() {
    // final class registry 缺 `car`：profiles rebind 返回结构化错误，装配失败。
    let without_car = laneflow_core::ParticipantClassRegistry::try_new(vec![
        laneflow_core::ParticipantClass::new("motorVehicle", None),
    ])
    .expect("valid class registry");
    let error = InitialTrafficData::try_new(
        canonical_graph(),
        [],
        canonical_profiles(),
        laneflow_core::JunctionRegistry::empty(),
        laneflow_core::SignalRegistry::empty(),
        laneflow_core::ParkingRegistry::empty(),
        without_car,
        laneflow_core::CrossSectionRegistry::empty(),
        laneflow_core::AccessRegistry::empty(),
    )
    .expect_err("profile class missing in final registry must fail");

    std::assert_matches!(
        error,
        CoreError::UnknownVehicleProfileParticipantClass { profile_id, class_id }
            if profile_id == "passenger-car" && class_id == "car"
    );
}
