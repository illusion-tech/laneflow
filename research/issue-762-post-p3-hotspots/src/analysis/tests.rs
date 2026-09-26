use super::*;

fn fixture(detail: bool, workload: u64) -> Value {
    let t = if detail {
        vec![
            90,
            90,
            90,
            90,
            90,
            90,
            90,
            90,
            90,
            90,
            10,
            10,
            10,
            if workload >= 1_024 { 10 } else { 0 },
            if workload >= 1_024 { 10 } else { 0 },
            if workload > 0 && workload < 1_024 {
                10
            } else {
                0
            },
            10,
            0,
        ]
    } else {
        vec![90, 90, 90, 90, 90, 90, 90, 90, 90, 90, 20, 30]
    };
    let c = if detail {
        vec![
            1,
            1,
            1,
            1,
            1,
            1,
            1,
            1,
            1,
            1,
            1,
            1,
            1,
            u64::from(workload >= 1_024),
            u64::from(workload >= 1_024),
            u64::from(workload > 0 && workload < 1_024),
            1,
            workload,
        ]
    } else {
        vec![1; 12]
    };
    json!(
        (1..=256)
            .map(|n| json!({"tick":n,"step_ns":1_000,"stages_ns":t,"calls":c}))
            .collect::<Vec<_>>()
    )
}

fn parse(value: &Value, mode: &str) -> Result<Vec<Row>> {
    rows(
        &value
            .as_array()
            .unwrap()
            .iter()
            .map(|v| format!("LF762 {v}\n"))
            .collect::<String>(),
        mode,
    )
}

#[test]
fn valid_partition_and_nested_children() {
    assert_eq!(parse(&fixture(false, 0), "stages").unwrap().len(), 256);
}

#[test]
fn missing_duplicate_and_reordered_ticks_fail() {
    let original = fixture(false, 0);
    let mut missing = original.clone();
    missing.as_array_mut().unwrap().pop();
    let mut duplicate = original.clone();
    duplicate
        .as_array_mut()
        .unwrap()
        .push(original[255].clone());
    let mut reversed = original;
    reversed.as_array_mut().unwrap().reverse();
    for value in [missing, duplicate, reversed] {
        assert!(parse(&value, "stages").is_err());
    }
}

#[test]
fn worker_cpu_sum_and_nested_double_count_fail() {
    for (i, n) in [(0, 1_001), (10, 91)] {
        let mut value = fixture(false, 0);
        value[100]["stages_ns"][i] = json!(n);
        assert!(parse(&value, "stages").is_err());
    }
}

#[test]
fn missing_clock_and_wrong_arm_fail() {
    let mut value = fixture(false, 0);
    value[200]["calls"][4] = json!(0);
    assert!(parse(&value, "stages").is_err());
    assert!(parse(&fixture(false, 0), "plain").is_err());
}

#[test]
fn negative_fractional_bool_and_bad_shape_fail() {
    for bad in [json!(-1), json!(0.5), json!(true), json!(u64::MAX)] {
        let mut value = fixture(false, 0);
        value[0]["stages_ns"][0] = bad;
        assert!(parse(&value, "stages").is_err());
    }
    let mut value = fixture(false, 0);
    value[0]["stages_ns"] = json!([]);
    assert!(parse(&value, "stages").is_err());
}

#[test]
fn nearest_rank_tail_is_not_average() {
    let mut values = vec![1_000_000; 95];
    values.extend([10_000_000; 5]);
    let value = stats(values);
    assert_eq!(value["mean_ms"], 1.45);
    assert_eq!(value["p95_ms"], 1.0);
    assert_eq!(value["p99_ms"], 10.0);
}

#[test]
fn detail_threshold_and_exclusivity() {
    assert!(parse(&fixture(true, 1_024), "detail").is_ok());
    for (i, n) in [(13, 0), (15, 1), (17, 1_023)] {
        let mut value = fixture(true, 1_024);
        value[100]["calls"][i] = json!(n);
        assert!(parse(&value, "detail").is_err());
    }
}

#[test]
fn zero_and_small_worksets() {
    for workload in [0, 1, 1_023] {
        assert!(parse(&fixture(true, workload), "detail").is_ok());
    }
}

#[test]
fn published_identity_and_real_timing_changes_fail() {
    assert!(same_json(&json!({"sha":"a"}), &json!({"sha":"b"}), "test").is_err());
    assert!(same_json(&json!({"mean":3.4}), &json!({"mean":3.40001}), "test").is_err());
    assert!(same_json(&json!([1, 2]), &json!([1]), "test").is_err());
}

#[test]
fn unsafe_evidence_paths_fail() {
    for path in ["../escape", "/absolute", "C:\\outside", "folder\\child", ""] {
        assert!(io::safe_child(Path::new("raw"), path).is_err());
    }
}
