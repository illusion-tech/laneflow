use crate::{Junction, JunctionRegistry, LaneGraph, ManeuverPath, Movement};

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
