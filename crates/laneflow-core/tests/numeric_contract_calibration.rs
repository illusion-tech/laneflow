#[allow(dead_code)]
#[path = "support/numeric_contract_calibration.rs"]
mod calibration;
#[allow(dead_code)]
#[path = "support/numeric_precision_research.rs"]
mod runtime_candidates;

use calibration::{
    COMPUTED_SPEED_TOLERANCE_CANDIDATE_METERS_PER_SECOND, ConversionDomain, ConversionFailure,
    EDGE_BOUNDARY_TOLERANCE_CANDIDATE_METERS, EDGE_MINIMUM_CANDIDATES_METERS,
    LONGITUDINAL_TOLERANCE_CANDIDATE_METERS, MAX_EDGE_LENGTH_METERS, MAX_EXTENT_OR_OFFSET_METERS,
    MIN_PARKING_EXTENT_METERS, MIN_VEHICLE_LENGTH_METERS,
    PARKING_ANCHOR_CLEARANCE_CANDIDATE_METERS, PHYSICAL_GAP_TOLERANCE_CANDIDATE_METERS,
    append_converted_batch_atomic, calibrate_constraint_cross_matrix, calibrate_runtime_chains,
    checked_f32, convert_raw_f64, parking_anchor_is_strictly_inside,
};

#[test]
fn conversion_oracle_locks_raw_then_target_validation_order() {
    for min_exclusive_meters in EDGE_MINIMUM_CANDIDATES_METERS {
        let domain = ConversionDomain::EdgeLength {
            min_exclusive_meters,
        };
        assert_eq!(
            convert_raw_f64(domain, min_exclusive_meters),
            Err(ConversionFailure::RawOutOfRange),
        );

        let rounded_boundary = min_exclusive_meters as f32;
        let first_valid_target = if f64::from(rounded_boundary) > min_exclusive_meters {
            rounded_boundary
        } else {
            rounded_boundary.next_up()
        };
        assert_eq!(
            convert_raw_f64(domain, f64::from(first_valid_target)),
            Ok(first_valid_target),
        );
        assert_eq!(
            convert_raw_f64(domain, MAX_EDGE_LENGTH_METERS),
            Ok(MAX_EDGE_LENGTH_METERS as f32),
        );
        assert_eq!(
            convert_raw_f64(domain, MAX_EDGE_LENGTH_METERS.next_up()),
            Err(ConversionFailure::RawOutOfRange),
        );
    }

    let exact_positive = ConversionDomain::EdgeLength {
        min_exclusive_meters: 0.0,
    };
    assert_eq!(
        convert_raw_f64(exact_positive, f64::from_bits(1)),
        Err(ConversionFailure::TargetOutOfRange),
        "raw positive value that underflows to target zero must not become a valid edge",
    );

    for domain in [
        ConversionDomain::VehicleLength,
        ConversionDomain::ParkingExtent,
    ] {
        assert!(convert_raw_f64(domain, 0.1).is_ok());
        assert_eq!(
            convert_raw_f64(domain, 0.1_f64.next_down()),
            Err(ConversionFailure::RawOutOfRange),
        );
        assert_eq!(
            convert_raw_f64(domain, MAX_EXTENT_OR_OFFSET_METERS),
            Ok(MAX_EXTENT_OR_OFFSET_METERS as f32),
        );
        assert_eq!(
            convert_raw_f64(domain, MAX_EXTENT_OR_OFFSET_METERS.next_up()),
            Err(ConversionFailure::RawOutOfRange),
        );
    }

    for raw in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            convert_raw_f64(ConversionDomain::VehicleLength, raw),
            Err(ConversionFailure::RawNonFinite),
        );
    }
}

#[test]
fn checked_conversion_and_batch_normalization_fail_atomically() {
    assert_eq!(
        checked_f32(-0.0).expect("signed zero converts").to_bits(),
        0
    );
    assert_eq!(
        checked_f32(f64::MAX),
        Err(ConversionFailure::TargetNonFinite),
    );

    let mut authority = vec![7.0_f32];
    let before = authority.clone();
    let failure = append_converted_batch_atomic(
        &mut authority,
        ConversionDomain::VehicleLength,
        &[4.5, 0.1_f64.next_down(), 8.0],
    );
    assert_eq!(failure, Err((1, ConversionFailure::RawOutOfRange)));
    assert_eq!(authority, before);

    let failure = append_converted_batch_atomic(
        &mut authority,
        ConversionDomain::ParkingLateralOffset,
        &[1.0, f64::from_bits(1), 2.0],
    );
    assert_eq!(failure, Err((1, ConversionFailure::TargetZero)));
    assert_eq!(authority, before);

    append_converted_batch_atomic(
        &mut authority,
        ConversionDomain::ParkingExtent,
        &[0.1, 4.0, 128.0],
    )
    .expect("valid batch commits only after every conversion succeeds");
    assert_eq!(authority, [7.0, 0.1_f64 as f32, 4.0, 128.0]);
}

#[test]
fn conversion_oracle_keeps_lateral_exact_nonzero_and_canonical_zero_distinct() {
    let domain = ConversionDomain::ParkingLateralOffset;
    for zero in [0.0_f64, -0.0] {
        assert_eq!(
            convert_raw_f64(domain, zero),
            Err(ConversionFailure::TargetZero),
        );
    }
    for underflow in [f64::from_bits(1), -f64::from_bits(1)] {
        assert_eq!(
            convert_raw_f64(domain, underflow),
            Err(ConversionFailure::TargetZero),
        );
    }

    let minimum_target = f32::from_bits(1);
    assert_eq!(
        convert_raw_f64(domain, f64::from(minimum_target)),
        Ok(minimum_target),
    );
    assert_eq!(
        convert_raw_f64(domain, -f64::from(minimum_target)),
        Ok(-minimum_target),
    );
    assert_eq!(
        convert_raw_f64(domain, MAX_EXTENT_OR_OFFSET_METERS),
        Ok(MAX_EXTENT_OR_OFFSET_METERS as f32),
    );
    assert_eq!(
        convert_raw_f64(domain, -MAX_EXTENT_OR_OFFSET_METERS),
        Ok(-(MAX_EXTENT_OR_OFFSET_METERS as f32)),
    );
}

#[test]
fn parking_anchor_clearance_is_derived_and_strict_on_both_endpoints() {
    assert_eq!(
        PARKING_ANCHOR_CLEARANCE_CANDIDATE_METERS,
        EDGE_BOUNDARY_TOLERANCE_CANDIDATE_METERS.max(LONGITUDINAL_TOLERANCE_CANDIDATE_METERS),
    );
    let edge_length = 10.0_f32;
    let clearance = PARKING_ANCHOR_CLEARANCE_CANDIDATE_METERS;
    assert!(!parking_anchor_is_strictly_inside(
        edge_length,
        clearance,
        clearance,
    ));
    assert!(parking_anchor_is_strictly_inside(
        edge_length,
        clearance.next_up(),
        clearance,
    ));
    let upper = f64::from(edge_length) - clearance;
    assert!(!parking_anchor_is_strictly_inside(
        edge_length,
        upper,
        clearance,
    ));
    assert!(parking_anchor_is_strictly_inside(
        edge_length,
        upper.next_down(),
        clearance,
    ));
}

#[test]
fn runtime_oracle_calibrates_four_independent_arithmetic_chains() {
    let report = calibrate_runtime_chains();
    for (domain, stats, threshold) in [
        (
            "edge_boundary",
            report.edge_boundary,
            EDGE_BOUNDARY_TOLERANCE_CANDIDATE_METERS,
        ),
        (
            "longitudinal",
            report.longitudinal,
            LONGITUDINAL_TOLERANCE_CANDIDATE_METERS,
        ),
        (
            "physical_gap",
            report.physical_gap,
            PHYSICAL_GAP_TOLERANCE_CANDIDATE_METERS,
        ),
        (
            "computed_speed",
            report.computed_speed,
            COMPUTED_SPEED_TOLERANCE_CANDIDATE_METERS_PER_SECOND,
        ),
    ] {
        let worst = stats
            .worst
            .expect("every arithmetic chain must have samples");
        let safety_ratio = if stats.max_absolute_error == 0.0 {
            f64::INFINITY
        } else {
            threshold / stats.max_absolute_error
        };
        eprintln!(
            "numeric_contract_calibration domain={domain} samples={} max_absolute_error={:.12} max_error_in_local_ulps={:.6} candidate_threshold={:.12} safety_ratio={safety_ratio:.6} worst=({worst})",
            stats.samples, stats.max_absolute_error, stats.max_error_in_local_ulps, threshold,
        );
        assert!(
            stats.max_absolute_error <= threshold,
            "domain={domain} threshold={threshold} worst={worst}",
        );
    }

    let discarded = report
        .discarded_residual_edge_rebase
        .worst
        .expect("discarded-residual control must have samples");
    eprintln!(
        "numeric_contract_failure domain=edge_boundary chain=ordinary_negative_edge_add max_absolute_error={:.12} repaired_max_absolute_error={:.12} worst=({discarded})",
        report.discarded_residual_edge_rebase.max_absolute_error,
        report.edge_boundary.max_absolute_error,
    );
    assert!(
        report.discarded_residual_edge_rebase.max_absolute_error
            > EDGE_BOUNDARY_TOLERANCE_CANDIDATE_METERS,
        "the preserved control must prove why ordinary negative-edge addition is rejected",
    );
    assert!(
        report.edge_boundary.max_absolute_error * 100.0
            < report.discarded_residual_edge_rebase.max_absolute_error,
        "edge rebase must materially remove the discarded-residual chain",
    );

    assert!(report.edge_boundary.max_absolute_error < 0.01);
    assert!(report.longitudinal.max_absolute_error < 0.01);
    assert!(report.physical_gap.max_absolute_error < 0.01);
    assert!(report.computed_speed.max_absolute_error < 0.01);
}

#[test]
fn arithmetic_predicates_lock_exact_and_adjacent_comparison_directions() {
    let edge = EDGE_BOUNDARY_TOLERANCE_CANDIDATE_METERS;
    assert!(edge.next_down() < edge);
    assert!(edge >= edge);
    assert!(edge.next_up() >= edge);

    let longitudinal = LONGITUDINAL_TOLERANCE_CANDIDATE_METERS;
    assert!(0.0 + longitudinal >= longitudinal.next_down());
    assert!(0.0 + longitudinal >= longitudinal);
    assert!(longitudinal < longitudinal.next_up());

    let gap = PHYSICAL_GAP_TOLERANCE_CANDIDATE_METERS;
    assert!(gap <= gap);
    assert!(gap.next_up() > gap);
    assert!(-gap >= -gap);
    assert!((-gap).next_down() < -gap);
    assert!((-gap..=gap).contains(&0.0));

    let speed = COMPUTED_SPEED_TOLERANCE_CANDIDATE_METERS_PER_SECOND;
    assert!(0.0 <= speed);
    assert!(speed <= speed);
    assert!(speed.next_up() > speed);
}

#[test]
fn constraint_cross_matrix_preserves_attribution_and_event_order() {
    let report = calibrate_constraint_cross_matrix();
    eprintln!(
        "numeric_contract_constraint_matrix samples={} divergences={} first_divergence={:?} signal_tie={} spatial_before_leader={}",
        report.samples,
        report.divergences,
        report.first_divergence,
        report.signal_wins_equal_distance_tie,
        report.spatial_event_precedes_leader_event,
    );
    assert_eq!(report.divergences, 0, "{:?}", report.first_divergence);
    assert!(report.signal_wins_equal_distance_tie);
    assert!(report.spatial_event_precedes_leader_event);
}

#[test]
fn normalized_authority_runtime_oracle_preserves_control_flow() {
    use runtime_candidates::{
        CandidateLayout, CandidateScenario, CandidateWorld, ResidualAwareF32Mode, STEP_COUNT,
        SensitiveControlMixedMode,
    };

    let mut max_progress_error = 0.0_f64;
    let mut max_gap_error = 0.0_f64;
    let mut max_speed_error = 0.0_f64;
    let mut max_acceleration_error = 0.0_f64;
    let mut max_travel_error = 0.0_f64;
    for layout in CandidateLayout::EDGE_CAP_MATRIX.into_iter().skip(1) {
        for scenario in [
            CandidateScenario::FreeFlow,
            CandidateScenario::DensePlatoon,
            CandidateScenario::StopAndGo,
        ] {
            let mut reference =
                CandidateWorld::<SensitiveControlMixedMode>::new(256, scenario, layout);
            let mut candidate = CandidateWorld::<ResidualAwareF32Mode>::new(256, scenario, layout);
            for tick in 1..=STEP_COUNT {
                let reference_summary = reference.step();
                let candidate_summary = candidate.step();
                assert_eq!(
                    candidate_summary, reference_summary,
                    "layout={layout:?} scenario={scenario:?} tick={tick}",
                );
                for index in 0..reference.len() {
                    let reference = reference.snapshot(index);
                    let candidate = candidate.snapshot(index);
                    assert_eq!(candidate.route_edge_index, reference.route_edge_index);
                    assert_eq!(candidate.status, reference.status);
                    assert_eq!(candidate.leader, reference.leader);
                    assert_eq!(candidate.safety_projection, reference.safety_projection);
                    max_progress_error = max_progress_error
                        .max((candidate.route_progress - reference.route_progress).abs());
                    max_speed_error = max_speed_error
                        .max((candidate.current_speed - reference.current_speed).abs());
                    max_acceleration_error = max_acceleration_error.max(
                        (candidate.applied_acceleration - reference.applied_acceleration).abs(),
                    );
                    max_travel_error = max_travel_error
                        .max((candidate.final_travel - reference.final_travel).abs());
                    match (candidate.bumper_gap, reference.bumper_gap) {
                        (Some(candidate), Some(reference)) => {
                            max_gap_error = max_gap_error.max((candidate - reference).abs());
                        }
                        (None, None) => {}
                        unexpected => panic!(
                            "leader gap presence diverged: layout={layout:?} scenario={scenario:?} tick={tick} vehicle={index} gaps={unexpected:?}",
                        ),
                    }
                }
            }
        }
    }

    eprintln!(
        "numeric_contract_normalized_runtime max_progress_error={max_progress_error:.12} max_gap_error={max_gap_error:.12} max_speed_error={max_speed_error:.12} max_acceleration_error={max_acceleration_error:.12} max_travel_error={max_travel_error:.12}",
    );
    assert!(max_progress_error <= 0.01);
    assert!(max_gap_error <= 0.01);
    assert!(max_speed_error <= 0.01);
    assert!(max_acceleration_error <= 0.02);
    assert!(max_travel_error <= 0.01);
}

#[test]
fn target_input_candidates_remain_within_frozen_product_ranges() {
    assert_eq!(MIN_VEHICLE_LENGTH_METERS, 0.1);
    assert_eq!(MIN_PARKING_EXTENT_METERS, 0.1);
    assert_eq!(MAX_EXTENT_OR_OFFSET_METERS, 128.0);
    assert_eq!(MAX_EDGE_LENGTH_METERS, 10_000.0);
    assert_eq!(EDGE_MINIMUM_CANDIDATES_METERS, [0.0, 0.01, 0.1, 1.0]);
}
