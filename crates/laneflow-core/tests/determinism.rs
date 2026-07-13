mod common;

use common::world_with_test_profile;
use laneflow_core::{
    CoreWorld, EdgeLength, EdgeProgress, LaneEdge, LaneGraph, Route, Speed, TickInput,
    VehicleSpawnInput,
};

fn edge_length(value: f64) -> EdgeLength {
    EdgeLength::try_new(value).expect("valid edge length")
}

fn progress(value: f64) -> EdgeProgress {
    EdgeProgress::try_new(value).expect("valid progress")
}

fn speed(value: f64) -> Speed {
    Speed::try_new(value).expect("valid speed")
}

fn deterministic_world() -> CoreWorld {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new("A", edge_length(10.0), ["B"]),
        LaneEdge::new("B", edge_length(5.0), std::iter::empty::<&str>()),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R", ["A", "B"]).expect("valid route");

    world_with_test_profile(1_000, lane_graph, [route], |profile| {
        vec![
            VehicleSpawnInput::active("V2", profile, "R", 0, progress(6.0), speed(2.0)),
            VehicleSpawnInput::active("V1", profile, "R", 0, progress(0.0), speed(6.0)),
        ]
    })
    .expect("valid world")
}

#[test]
fn same_initial_world_and_tick_sequence_produces_same_results() {
    let mut first = deterministic_world();
    let mut second = deterministic_world();

    for _ in 0..4 {
        let first_result = first.step(TickInput::new(1_000)).expect("step succeeds");
        let second_result = second.step(TickInput::new(1_000)).expect("step succeeds");

        assert_eq!(first_result, second_result);
        assert_eq!(first.fixed_delta_time_ms(), second.fixed_delta_time_ms());
        assert_eq!(first.tick_index(), second.tick_index());
        assert_eq!(first.time_ms(), second.time_ms());
        assert_eq!(
            first.vehicles().cloned().collect::<Vec<_>>(),
            second.vehicles().cloned().collect::<Vec<_>>()
        );
    }

    assert_eq!(first, second);
}
