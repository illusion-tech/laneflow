//! #682 未插桩生产库整步墙钟；原始逐拍样本与摘要在计时外输出。
use laneflow_runtime as runtime_types;
mod fixture;
use std::time::Instant;

#[test]
fn sparse_fixture_axes_remain_valid() {
    for case in [fixture::CASES[0], fixture::CASES[2], fixture::CASES[8]] {
        let case = fixture::Case {
            active: 16,
            live: if case.live > case.active { 32 } else { 16 },
            high_water: 32,
            capacity: 64,
            ..case
        };
        let root = fixture::revision(case);
        let mut world = fixture::world(&root, case);
        for _ in 0..fixture::STEPS {
            fixture::step(&mut world);
        }
        fixture::validate(&world, case);
    }
}

#[test]
#[ignore = "manual #682 release normal wall clock; exclusive machine window"]
fn sparse_cost_wall() {
    println!();
    for case in fixture::selected() {
        let root = fixture::revision(case);
        let mut world = fixture::world(&root, case);
        let mut samples = vec![0; fixture::STEPS];
        for sample in &mut samples {
            let started = Instant::now();
            fixture::step(&mut world);
            *sample = started.elapsed().as_nanos();
        }
        fixture::validate(&world, case);
        for (tick, ns) in samples.iter().enumerate() {
            println!("sparse-tick case={} tick={tick} ns={ns}", case.name);
        }
        println!(
            "sparse-end case={} active={} live={} high_water={} capacity={} edges={} warm={} steps={} digest={} input={:?}",
            case.name,
            case.active,
            case.live,
            case.high_water,
            case.capacity,
            case.edges,
            fixture::WARM,
            fixture::STEPS,
            fixture::digest(&world),
            root.canonical_origin().canonical_artifact_digest()
        );
    }
}
