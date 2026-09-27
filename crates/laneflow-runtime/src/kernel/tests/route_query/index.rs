//! #679 路线私有稠密定位原型；只保存拓扑下标，不保存策略解释或资源授权。
use crate::kernel::tables::CompiledRoute;
use std::{hint::black_box, time::Instant};

fn locate(route: &CompiledRoute, hop: u32) -> [u32; 6] {
    [
        route.gate_hops.partition_point(|item| *item < hop) as u32,
        route.gate_hops.partition_point(|item| *item <= hop) as u32,
        route.waiting.partition_point(|item| item.entry_hop < hop) as u32,
        route.waiting.partition_point(|item| item.release_hop < hop) as u32,
        route
            .conflicts
            .partition_point(|item| item.admission_hop < hop) as u32,
        route
            .maneuvers
            .partition_point(|item| item.exit_route_edge_index <= hop) as u32,
    ]
}

pub(super) fn measure(route: &CompiledRoute, case: &str) {
    const QUERIES: usize = 262_144;
    for round in 0..3 {
        let start = Instant::now();
        let index: Vec<_> = (0..=route.edges.len())
            .map(|hop| locate(route, hop as u32))
            .collect();
        let build_ns = start.elapsed().as_nanos();
        for (hop, actual) in index.iter().enumerate() {
            assert_eq!(*actual, locate(route, hop as u32));
        }
        let mut times = [0; 2];
        let mut sums = [0u64; 2];
        for mode in if round % 2 == 0 { [0, 1] } else { [1, 0] } {
            let start = Instant::now();
            for query in 0..QUERIES {
                let hop = black_box(query.wrapping_mul(97) % index.len());
                let value = if mode == 0 {
                    locate(black_box(route), hop as u32)
                } else {
                    black_box(&index)[hop]
                };
                sums[mode] += black_box(value).into_iter().map(u64::from).sum::<u64>();
            }
            times[mode] = start.elapsed().as_nanos();
        }
        assert_eq!(sums[0], sums[1]);
        println!(
            "route-index case={case} round={round} queries={QUERIES} build_ns={build_ns} bytes={} binary_ns={} dense_ns={} checksum={}",
            index.capacity() * size_of::<[u32; 6]>(),
            times[0],
            times[1],
            sums[0]
        );
    }
}
