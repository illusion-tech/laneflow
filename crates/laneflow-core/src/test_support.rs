use crate::{
    Junction, JunctionRegistry, LaneGraph, ManeuverPath, Movement, ParticipantClass,
    ParticipantClassHandle, ParticipantClassRegistry,
};

/// 默认测试 ParticipantClass registry：`motorVehicle` root + `car` 子类。
pub(crate) fn test_participant_class_registry() -> ParticipantClassRegistry {
    ParticipantClassRegistry::try_new(vec![
        ParticipantClass::new("motorVehicle", None),
        ParticipantClass::new("car", Some("motorVehicle")),
    ])
    .expect("test participant class registry must be valid")
}

/// 默认测试 ParticipantClass handle（`test_participant_class_registry` 中的 `car`）。
pub(crate) fn test_car_participant_class() -> ParticipantClassHandle {
    ParticipantClassHandle::new(1)
}

pub(crate) fn zero_internal_junctions(
    graph: &LaneGraph,
    paths: &[(&str, &str, &str)],
) -> JunctionRegistry {
    JunctionRegistry::try_new(
        graph,
        paths
            .iter()
            .enumerate()
            .map(|(index, _)| Junction::new(format!("test-junction-{index}"))),
        paths.iter().enumerate().map(|(index, _)| {
            Movement::new(
                format!("test-movement-{index}"),
                format!("test-junction-{index}"),
            )
        }),
        paths.iter().enumerate().map(|(index, (id, entry, exit))| {
            ManeuverPath::new(
                *id,
                format!("test-movement-{index}"),
                *entry,
                std::iter::empty::<&str>(),
                *exit,
            )
        }),
    )
    .expect("test topology must be valid")
}
