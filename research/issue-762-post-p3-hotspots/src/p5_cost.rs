//! #772 P5 成本归因：共享采集器，工作线程抽样累计与协调器墙钟分列。
#[allow(dead_code)]
mod p2_cost;
mod p5_export;
use p2_cost::io;
use std::{error::Error, path::Path};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const BASE: &str = "05c505dde3b76fde0a5302a567bf46fdb236146f";
const STAGES: [&str; 55] = [
    "Preflight",
    "Occupancy",
    "WaitingPrepare",
    "ConflictPrepare",
    "MotionLoop",
    "WaitingFinalize",
    "Signals",
    "ConflictFinalize",
    "WaitingOutputs",
    "Commit",
    "Frontier",
    "P4",
    "P3Discover",
    "P3Dispatch",
    "P3Consume",
    "P3Fused",
    "P3Tail",
    "P3Workload",
    "P5Discover",
    "P5Slots",
    "P5Dispatch",
    "P5Consume",
    "P5Fused",
    "SampleTotal",
    "SampleInputs",
    "SampleStops",
    "SampleReuse",
    "SampleRecompute",
    "SampleArrival",
    "MotionInputs",
    "MotionHorizon",
    "LeaderGap",
    "RouteStops",
    "GateStop",
    "Solve",
    "HardRoom",
    "NextState",
    "ClockFloor",
    "P5Vehicles",
    "P5Reused",
    "P5Recomputed",
    "P5Sampled",
    "SampleReused",
    "SampleRecomputed",
    "P5InputCount",
    "P5MotionCalls",
    "P5HorizonReuse",
    "P5HorizonComputed",
    "P5SignalGates",
    "P5RestrictiveGateVisits",
    "P5HardStopped",
    "P5ZeroTravel",
    "P5SameEdge",
    "P5CrossEdge",
    "P5Arrivals",
];
const PROTOCOL: p2_cost::Protocol = p2_cost::Protocol {
    baseline: BASE,
    name: "p5-cost-v1",
    stages: &STAGES,
    validate: validate_detail,
};
fn need(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(message.to_owned().into())
    }
}
fn samples(workload: u64, tick: u64) -> u64 {
    let offset = (tick - 1) % 64;
    if workload > offset {
        1 + (workload - 1 - offset) / 64
    } else {
        0
    }
}
fn validate_detail(t: &[u64], c: &[u64], tick: u64) -> Result<()> {
    let sum = |v: &[u64]| v.iter().map(|n| u128::from(*n)).sum::<u128>();
    need(sum(&t[18..23]) <= u128::from(t[4]), "P5 wall nesting")?;
    need(
        c[18..22].iter().all(|n| *n == 1) && c[22] == 0 && c[44] >= 1_024,
        "P5 city dispatch path",
    )?;
    need(
        c[38] == c[44]
            && u128::from(c[39]) + u128::from(c[40]) == u128::from(c[38])
            && c[40] == c[45]
            && u128::from(c[46]) + u128::from(c[47]) == u128::from(c[45])
            && sum(&c[50..54]) == u128::from(c[45])
            && c[54] <= c[38],
        "P5 path partition",
    )?;
    need(
        c[41] == samples(c[44], tick)
            && u128::from(c[42]) + u128::from(c[43]) == u128::from(c[41])
            && c[42] <= c[39]
            && c[43] <= c[40],
        "sample rotation/partition",
    )?;
    need(
        [23, 24, 25, 26, 28, 37].iter().all(|i| c[*i] == c[41])
            && c[27] == c[43]
            && c[29..36].iter().all(|n| *n == c[43])
            && c[36] <= c[43],
        "sample span counts",
    )?;
    need(
        sum(&t[24..29]) + u128::from(t[37]) <= u128::from(t[23])
            && sum(&t[29..37]) <= u128::from(t[27])
            && t[38..].iter().all(|n| *n == 0),
        "sample nesting/counter clocks",
    )
}
fn run() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    match a.first().map(String::as_str) {
        Some("prepare") if a.len()==3 => p5_export::export(Path::new(&a[2]),&a[1]),
        Some("run") if a.len()==4 => p2_cost::capture_for(Path::new(&a[1]),Path::new(&a[2]),Path::new(&a[3]),PROTOCOL),
        Some("analyze"|"verify") if a.len()==3 => {let raw=Path::new(&a[1]);let out=Path::new(&a[2]);let v=p2_cost::analyze_for(raw,PROTOCOL)?;if a[0]=="verify" {need(v==io::read_json(out)?,"published mismatch")?;println!("verified 12 runs");Ok(())} else {io::outside(raw,out)?;io::write_new(out,&v)}},
        _=>Err("prepare <plain|detail> <root> | run <root> <inputs> <new-raw> | analyze|verify <raw> <results>".into())
    }
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    fn fixture() -> (Vec<Value>, Vec<Value>) {
        let ticks = (1..=256)
            .map(|tick| json!({"tick":tick,"N_active":2_048}))
            .collect();
        let rows = (1..=256)
            .map(|tick| {
                let mut c = vec![0; 55];
                c[..13].fill(1);
                c[16] = 1;
                c[18..22].fill(1);
                c[38] = 2_048;
                c[39] = 1_024;
                c[40] = 1_024;
                c[44] = 2_048;
                c[45] = 1_024;
                c[46] = 1_024;
                c[52] = 1_024;
                c[41] = samples(2_048, tick);
                c[42] = 16;
                c[43] = 16;
                for i in [23, 24, 25, 26, 28, 37] {
                    c[i] = 32;
                }
                c[27] = 16;
                c[29..37].fill(16);
                json!({"tick":tick,"step_ns":100,"stages_ns":vec![0;55],"calls":c})
            })
            .collect();
        (rows, ticks)
    }
    #[test]
    fn rotation_covers_each_position_once_in_64_ticks() {
        for workload in [0, 1, 63, 64, 65, 1_024, 7_500, 75_000] {
            assert_eq!(
                (1..=64).map(|tick| samples(workload, tick)).sum::<u64>(),
                workload
            );
        }
    }
    #[test]
    fn counters_sampling_and_nested_clocks_fail_closed() {
        let (rows, ticks) = fixture();
        p2_cost::validate_rows_for(&rows, &ticks, "detail", PROTOCOL).unwrap();
        for index in 18..55 {
            if [48, 49].contains(&index) {
                continue;
            }
            let mut bad = rows.clone();
            bad[0]["calls"][index] = json!(u64::MAX);
            assert!(
                p2_cost::validate_rows_for(&bad, &ticks, "detail", PROTOCOL).is_err(),
                "counter {index}"
            );
        }
        for index in [18, 24, 29, 38] {
            let mut bad = rows.clone();
            bad[0]["stages_ns"][index] = json!(101);
            assert!(
                p2_cost::validate_rows_for(&bad, &ticks, "detail", PROTOCOL).is_err(),
                "span {index}"
            );
        }
        // 门访问次数无法由单拍车辆数给出上界；跨行汇总仍必须拒绝溢出。
        for index in [48, 49] {
            let mut bad = rows.clone();
            bad[0]["calls"][index] = json!(u64::MAX);
            bad[1]["calls"][index] = json!(1);
            assert!(p2_cost::validate_rows_for(&bad, &ticks, "detail", PROTOCOL).is_err());
        }
        assert!(p2_cost::validate_rows_for(&rows[..255], &ticks, "detail", PROTOCOL).is_err());
    }
}
