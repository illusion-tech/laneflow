//! #679 未插桩整步墙钟；路线登记在单独计时窗口，编译静态路网和生成车辆不计入。
use laneflow_runtime as runtime_types;
mod fixture;

#[test]
fn route_query_fixture_axes() {
    let root = fixture::revision(false);
    let (mut world, _) = fixture::world(&root, 8);
    for _ in 0..fixture::STEPS {
        fixture::step(&mut world);
    }
    fixture::validate(&world);
}

#[test]
#[ignore = "manual #679 release wall clock; exclusive machine window"]
fn route_query_wall() {
    println!();
    for round in 0..3 {
        for red in [false, true] {
            let root = fixture::revision(red);
            let mut repeats = fixture::REPEATS;
            if round % 2 == 1 {
                repeats.reverse();
            }
            for count in repeats {
                let (mut world, build_ns) = fixture::world(&root, count);
                let mut samples = Vec::with_capacity(fixture::STEPS);
                for _ in 0..fixture::STEPS {
                    let start = std::time::Instant::now();
                    fixture::step(&mut world);
                    samples.push(start.elapsed().as_nanos());
                }
                let state_sum = fixture::validate(&world);
                for (tick, ns) in samples.into_iter().enumerate() {
                    println!(
                        "route-tick round={round} red={red} repeats={count} tick={tick} ns={ns}"
                    );
                }
                println!(
                    "route-end round={round} red={red} repeats={count} vehicles={} warm={} steps={} build_ns={build_ns} state_sum={state_sum} input={:?}",
                    fixture::VEHICLES,
                    fixture::WARM,
                    fixture::STEPS,
                    root.canonical_origin().canonical_artifact_digest()
                );
            }
        }
    }
}
