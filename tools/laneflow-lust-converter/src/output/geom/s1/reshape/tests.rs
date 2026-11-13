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
    assert!(matches!(run(&input, 31), Attempt::Rejected("sink", _)));
    input.clamps[0] = Some([0.0, 0.0, 1.0]);
    assert!(matches!(
        run(&input, 20),
        Attempt::Rejected("complete_end_direction", _)
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
        "../../../../../tests/fixtures/issue-1107-inputs.json"
    ))
    .unwrap();
    // 收缩控制点正本（原出自 #1113 endpoint-feasibility.json 的两边缘参考控制位）。
    const FROZEN_CONTROLS: [(&str, usize, usize, [[u64; 3]; 6]); 2] = [
        (
            "sumo::-16502_0_0",
            4,
            1,
            [
                [13_862_387_164_458_443_080, 0, 4_655_015_308_771_993_846],
                [13_862_384_193_822_325_522, 0, 4_655_016_326_933_566_396],
                [13_862_381_141_529_851_931, 0, 4_655_017_349_990_150_296],
                [13_862_376_960_990_537_318, 0, 4_655_018_299_443_621_396],
                [4_599_843_899_495_991_925, 0, 4_606_637_215_741_038_519],
                [4_602_357_792_311_913_445, 0, 4_606_066_178_973_635_054],
            ],
        ),
        (
            "sumo::-28860_3_0",
            6,
            0,
            [
                [13_875_855_830_054_978_191, 0, 4_655_233_144_015_688_827],
                [13_875_858_281_081_106_028, 0, 4_655_233_656_986_549_693],
                [13_875_860_838_204_137_095, 0, 4_655_233_110_628_027_768],
                [13_875_863_342_278_716_547, 0, 4_655_233_093_934_197_238],
                [13_830_363_443_550_450_828, 0, 4_596_548_516_256_258_305],
                [13_830_554_255_501_481_530, 0, 13_797_708_031_575_004_769],
            ],
        ),
    ];
    for (id, count, index, expected_control) in FROZEN_CONTROLS {
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
        assert_eq!(curves[index].map(|p| p.map(f64::to_bits)), expected_control);
        assert!(!capture_if_active(control()), "capture state reset");
        assert!(
            repair_curve(&input.points, input.clamps[0], input.clamps[1], None, None).is_err(),
            "original rejection retained"
        );
    }
}
