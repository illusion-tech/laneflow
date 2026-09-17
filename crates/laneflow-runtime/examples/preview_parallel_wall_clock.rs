//! #705 非插桩整步墙钟对照：与 cfg(test) 测量探针同场景的
//! 正常 release 构建（无诊断插桩、无统计计数），用于把「融合 vs 多 worker」
//! 的整步墙钟登记为生产形态基线。预览局部收益不等于城市性能通过（#707）。
//!
//! 单独运行：
//! `cargo run --release -p laneflow-runtime --example preview_parallel_wall_clock`

#[path = "../tests/support/multi_gate_scene.rs"]
mod multi_gate_scene;

use std::time::Instant;

const VEHICLES: usize = 1_024;
const WARMUP_TICKS: usize = 24;
const MEASURED_TICKS: usize = 128;
const ROUNDS: usize = 3;

fn percentile(sorted: &[u128], fraction: f64) -> u128 {
    let index = (fraction * sorted.len() as f64).ceil() as usize;
    sorted[index.saturating_sub(1).min(sorted.len() - 1)]
}

fn main() {
    let revision = multi_gate_scene::build_revision(VEHICLES);
    for &workers in &[1_u32, 2, 4, 8, 16] {
        for round in 0..ROUNDS {
            let mut world = multi_gate_scene::install(&revision, workers, 705_500 + round as u64);
            let routes = multi_gate_scene::routes_of(&world);
            let boundaries = multi_gate_scene::route_boundaries(&world, &routes);
            for _ in 0..WARMUP_TICKS {
                multi_gate_scene::replenish(&mut world, &routes, &boundaries);
                multi_gate_scene::step(&mut world);
            }
            let mut samples = Vec::with_capacity(MEASURED_TICKS);
            for _ in 0..MEASURED_TICKS {
                multi_gate_scene::replenish(&mut world, &routes, &boundaries);
                let started = Instant::now();
                multi_gate_scene::step(&mut world);
                samples.push(started.elapsed().as_nanos());
            }
            let mut sorted = samples;
            sorted.sort_unstable();
            println!(
                "preview-wall scene=multi-gate-{VEHICLES} workers={workers} round={round} \
                 whole_p50_ns={} whole_p95_ns={}",
                percentile(&sorted, 0.50),
                percentile(&sorted, 0.95),
            );
        }
    }
    println!("preview-wall-done");
}
