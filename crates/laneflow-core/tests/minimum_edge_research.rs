use std::mem::size_of;

use laneflow_core::CoreEvent;

#[allow(dead_code)]
#[path = "support/minimum_edge_research.rs"]
mod research;

use research::{
    MIN_EDGE_CANDIDATES_METERS, SELECTED_MIN_EDGE_EXCLUSIVE_METERS, compact_transition_kernel,
    first_valid_edge_length, transition_pressure_estimate,
};

#[test]
fn minimum_edge_candidates_have_explicit_worst_tick_crossing_bounds() {
    let estimates = MIN_EDGE_CANDIDATES_METERS.map(transition_pressure_estimate);
    assert_eq!(estimates[0].crossings_per_vehicle, None);
    assert_eq!(estimates[1].crossings_per_vehicle, Some(9_999));
    assert_eq!(estimates[2].crossings_per_vehicle, Some(999));
    assert_eq!(estimates[3].crossings_per_vehicle, Some(99));
    assert_eq!(estimates[1].crossings_10k, Some(99_990_000));
    assert_eq!(estimates[2].crossings_10k, Some(9_990_000));
    assert_eq!(estimates[3].crossings_10k, Some(990_000));
    assert_eq!(estimates[1].crossings_100k, Some(999_900_000));
    assert_eq!(estimates[2].crossings_100k, Some(99_900_000));
    assert_eq!(estimates[3].crossings_100k, Some(9_900_000));

    for estimate in estimates.into_iter().skip(1) {
        let events = estimate
            .crossings_100k
            .expect("finite candidates have a bounded crossing count");
        eprintln!(
            "minimum_edge_pressure min_exclusive={} first_valid_f32={} crossings_per_vehicle={} crossings_10k={} crossings_100k={} core_event_size={} event_bytes_floor_100k={}",
            estimate.min_exclusive_meters,
            estimate.first_valid_edge_length_meters,
            estimate.crossings_per_vehicle.unwrap(),
            estimate.crossings_10k.unwrap(),
            events,
            size_of::<CoreEvent>(),
            events * size_of::<CoreEvent>() as u128,
        );
    }
}

#[test]
fn one_meter_exclusive_candidate_is_the_only_frozen_option_below_one_hundred_crossings() {
    let selected = transition_pressure_estimate(SELECTED_MIN_EDGE_EXCLUSIVE_METERS);
    assert_eq!(SELECTED_MIN_EDGE_EXCLUSIVE_METERS, 1.0);
    assert!(
        selected
            .crossings_per_vehicle
            .is_some_and(|crossings| crossings < 100),
    );
    for rejected in [0.01, 0.1] {
        assert!(
            transition_pressure_estimate(rejected)
                .crossings_per_vehicle
                .is_some_and(|crossings| crossings >= 100),
        );
    }
    assert!(first_valid_edge_length(SELECTED_MIN_EDGE_EXCLUSIVE_METERS) > 1.0);
}

#[test]
fn compact_transition_kernel_executes_each_crossing_without_event_allocation() {
    for min_exclusive in [0.01, 0.1, 1.0] {
        let estimate = transition_pressure_estimate(min_exclusive);
        let (crossings, checksum) = compact_transition_kernel(100, min_exclusive);
        let lower = u128::from(estimate.crossings_per_vehicle.unwrap()) * 100;
        assert!((lower..=lower + 100).contains(&crossings));
        assert!(checksum.is_finite());
        assert!(checksum >= 0.0);
    }
}

#[test]
#[ignore = "10k/100k compact multi-edge pressure is an explicit #127 performance measurement"]
fn minimum_edge_compact_transition_report_10k_100k() {
    for vehicle_count in [10_000, 100_000] {
        for min_exclusive in [0.01, 0.1, 1.0] {
            let (crossings, checksum) = compact_transition_kernel(vehicle_count, min_exclusive);
            eprintln!(
                "minimum_edge_compact_kernel vehicles={vehicle_count} min_exclusive={min_exclusive} crossings={crossings} checksum={checksum:.12}",
            );
        }
    }
}
