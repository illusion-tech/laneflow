use super::*;

fn control() -> Control {
    [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [3.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
    ]
}

#[test]
fn geometry_family_preserves_endpoints_and_end_tangents() {
    let first = control();
    let second = first.map(|p| add(p, [3.0, 0.0, 0.0]));
    let second = [
        second[0], second[1], second[2], second[3], first[4], first[5],
    ];
    let original = [first, second];
    for geometry in geometries(&original) {
        assert_eq!(geometry.curves[0][0], first[0]);
        assert_eq!(geometry.curves[0][4], first[4]);
        assert_eq!(geometry.curves.last().unwrap()[3], second[3]);
        assert_eq!(geometry.curves.last().unwrap()[5], second[5]);
        assert!(geometry.curves.windows(2).all(|w| w[0][3] == w[1][0]));
    }
}

#[test]
fn backtracking_control_is_rejected() {
    let mut c = control();
    assert!(monotone(&[c]));
    c[1] = [4.0, 0.0, 0.0];
    assert!(!monotone(&[c]));
}

#[test]
fn original_sink_and_end_checks_remain_active() {
    let c = control();
    let mut input = EdgeInput {
        points: vec![c[0], c[3]],
        clamps: [Some(c[4]), Some(c[5])],
        clamp_sources: [None, None],
    };
    let run =
        |input: &EdgeInput, n| attempt(&[c], &[None], false, n, input, &mut Budget::default());
    assert!(matches!(run(&input, 20), Attempt::Accepted(_)));
    assert!(matches!(run(&input, 31), Attempt::Rejected("sink")));
    input.clamps[0] = Some([0.0, 0.0, 1.0]);
    assert!(matches!(
        run(&input, 20),
        Attempt::Rejected("complete_end_direction")
    ));
}

#[test]
fn sampling_budget_stops_before_mutating_next_partition() {
    let mut budget = Budget {
        partitions: PARTITION_LIMIT,
        ..Budget::default()
    };
    assert!(!budget.partition());
    assert_eq!(budget.partitions, PARTITION_LIMIT);
    assert_eq!(budget.stop, Some("partitions"));
    let mut budget = Budget {
        sample_work: WORK_LIMIT,
        ..Budget::default()
    };
    assert!(!budget.sample(1));
    assert_eq!(budget.sample_work, WORK_LIMIT);
    let c = control();
    let input = EdgeInput {
        points: vec![c[0], c[3]],
        clamps: [Some(c[4]), Some(c[5])],
        clamp_sources: [None, None],
    };
    assert!(matches!(
        attempt(&[c], &[None], false, 20, &input, &mut budget),
        Attempt::Budget
    ));
    assert_eq!(budget.sample_work, WORK_LIMIT);
}

#[test]
fn reference_capture_recovers_the_frozen_real_controls_without_a_program() {
    let inputs: Value = serde_json::from_str(include_str!(
        "../../../../../../../research/issue-1107-dependency-recovery/inputs.json"
    ))
    .unwrap();
    let endpoints: Value = serde_json::from_str(include_str!(
        "../../../../../../../research/issue-1113-closure-feasibility/endpoint-feasibility.json"
    ))
    .unwrap();
    for (id, count, index) in [("sumo::-16502_0_0", 4, 1), ("sumo::-28860_3_0", 6, 0)] {
        let points: Vec<[u64; 3]> = serde_json::from_value(inputs[id]["points"].clone()).unwrap();
        let clamps: [Option<[u64; 3]>; 2] =
            serde_json::from_value(inputs[id]["clamps"].clone()).unwrap();
        let input = EdgeInput {
            points: points.into_iter().map(|p| p.map(f64::from_bits)).collect(),
            clamps: clamps.map(|p| p.map(|d| d.map(f64::from_bits))),
            clamp_sources: [None, None],
        };
        let curves = reference(&input);
        assert_eq!(curves.len(), count);
        assert_eq!(
            json!(curves[index].map(|p| p.map(f64::to_bits))),
            endpoints[id]["control_f64_bits"]
        );
        assert!(!capture_if_active(control()), "capture state reset");
        assert!(
            repair_curve(&input.points, input.clamps[0], input.clamps[1], None, None).is_err(),
            "original rejection retained"
        );
    }
}
