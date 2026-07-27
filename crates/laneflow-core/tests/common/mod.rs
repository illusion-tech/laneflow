use laneflow_core::{
    CoreError, CoreWorld, IidmProfileSpec, InitialTrafficData, LaneGraph, ParticipantClass,
    ParticipantClassHandle, ParticipantClassRegistry, Route, VehicleProfile, VehicleProfileHandle,
    VehicleProfileRegistry, VehicleSpawnInput,
};

/// 默认测试 ParticipantClass registry：`motorVehicle` root + `car` 子类。
pub fn test_participant_classes() -> ParticipantClassRegistry {
    ParticipantClassRegistry::try_new(vec![
        ParticipantClass::new("motorVehicle", None),
        ParticipantClass::new("car", Some("motorVehicle")),
    ])
    .expect("test participant class registry must be valid")
}

/// 返回 `classes` 中 `car` 子类的 handle。
pub fn test_car_class(classes: &ParticipantClassRegistry) -> ParticipantClassHandle {
    classes.class_handle("car").expect("car class must exist")
}

pub fn test_profile(external_id: &str) -> VehicleProfile {
    let classes = test_participant_classes();
    VehicleProfile::try_new_iidm(
        external_id,
        test_car_class(&classes),
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
    .expect("test profile must be valid")
}

pub fn test_profile_registry() -> (VehicleProfileRegistry, VehicleProfileHandle) {
    let registry = VehicleProfileRegistry::try_new(
        &test_participant_classes(),
        [test_profile("test-profile")],
    )
    .expect("test profile registry must be valid");
    let profile = registry
        .profile_handle("test-profile")
        .expect("test profile handle must exist");
    (registry, profile)
}

pub fn world_with_test_profile<I, F>(
    fixed_delta_time_ms: u64,
    lane_graph: LaneGraph,
    routes: I,
    vehicle_inputs: F,
) -> Result<CoreWorld, CoreError>
where
    I: IntoIterator<Item = Route>,
    F: FnOnce(VehicleProfileHandle) -> Vec<VehicleSpawnInput>,
{
    let (registry, profile) = test_profile_registry();
    let traffic_data = InitialTrafficData::try_new(
        lane_graph,
        routes,
        registry,
        laneflow_core::JunctionRegistry::empty(),
        laneflow_core::SignalRegistry::empty(),
        laneflow_core::ParkingRegistry::empty(),
        test_participant_classes(),
        laneflow_core::CrossSectionRegistry::empty(),
        laneflow_core::AccessRegistry::empty(),
    )?;
    CoreWorld::with_traffic_data(fixed_delta_time_ms, traffic_data, vehicle_inputs(profile))
}
