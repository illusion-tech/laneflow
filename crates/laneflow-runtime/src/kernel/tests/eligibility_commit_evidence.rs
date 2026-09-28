//! #777 正常库整拍；所有诊断钩子均不进入该库构建。
use laneflow_runtime as runtime_types;
#[path = "eligibility_commit/input.rs"]
mod input;
#[allow(dead_code)]
#[path = "sparse_cost/fixture.rs"]
mod sparse;

#[test]
#[ignore = "manual #777 release balanced A/B"]
fn eligibility_commit_wall() {
    println!();
    let case = input::selected();
    let mut samples = Vec::with_capacity(input::TICKS);
    for window in 0..input::windows(&case) {
        let (mut world, digest) = input::create(&case);
        for _ in 0..input::TICKS / input::windows(&case) {
            let start = std::time::Instant::now();
            input::step(&mut world, &case);
            samples.push(start.elapsed().as_nanos());
        }
        input::validate(&world, &case);
        input::end(&case, window, &world, &digest);
    }
    for (tick, ns) in samples.iter().enumerate() {
        println!("elig-tick case={case} tick={tick} ns={ns}");
    }
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn eligibility_resource_windows_remain_valid() {
    for case in ["conflict-small", "conflict-large"] {
        let (mut world, _) = input::create(case);
        for _ in 0..16 {
            input::step(&mut world, case);
        }
        input::validate(&world, case);
    }
}
