//! #793 当前主干短窗：只记录协调器墙钟，沿用冻结采集与交通一致性校验。
mod current_export;
#[allow(dead_code)]
mod p2_cost;
use p2_cost::io;
use std::{error::Error, path::Path};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const BASE: &str = "fadf6a844d9d22869976089f866c364edc6fc261";
const STAGES: [&str; 34] = [
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
    "P5Setup",
    "P5Slots",
    "P5Dispatch",
    "P5Consume",
    "P5Fused",
    "WaitingPreview",
    "WaitingAssembly",
    "P7Eligibility",
    "P7Scan",
    "P7Copy",
    "ConflictMotionClear",
    "EligibilityClear",
    "WaitingClear",
    "P5Active",
    "EligibilityLen",
    "EligibilityEmpty",
];
const PROTOCOL: p2_cost::Protocol = p2_cost::Protocol {
    baseline: BASE,
    name: "current-hotspots-v1",
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
fn validate_detail(t: &[u64], c: &[u64], _tick: u64) -> Result<()> {
    let sum = |v: &[u64]| v.iter().map(|n| u128::from(*n)).sum::<u128>();
    need(
        sum(&t[18..23]) <= u128::from(t[4])
            && sum(&t[23..25]) + u128::from(t[30]) <= u128::from(t[2])
            && sum(&t[26..28]) <= u128::from(t[25])
            && t[25] <= t[9]
            && sum(&t[10..17]) + sum(&t[28..30]) <= u128::from(t[3]),
        "coordinator clock nesting",
    )?;
    need(
        c[18..22].iter().all(|n| *n == 1)
            && c[22] == 0
            && c[23..27].iter().all(|n| *n == 1)
            && c[28..31].iter().all(|n| *n == 1)
            && c[31] >= 1_024
            && c[32] >= c[31]
            && c[33] <= 1
            && c[27] == 1 - c[33]
            && t[17] == 0
            && t[31..].iter().all(|n| *n == 0),
        "city path/counter clocks",
    )
}
fn run() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    match a.first().map(String::as_str) {
        Some("prepare") if a.len() == 3 => current_export::export(Path::new(&a[2]), &a[1]),
        Some("run") if a.len() == 4 => p2_cost::capture_for(Path::new(&a[1]), Path::new(&a[2]), Path::new(&a[3]), PROTOCOL),
        Some("analyze" | "verify") if a.len() == 3 => {
            let raw = Path::new(&a[1]);
            let out = Path::new(&a[2]);
            let value = p2_cost::analyze_for(raw, PROTOCOL)?;
            if a[0] == "verify" {
                need(value == io::read_json(out)?, "published mismatch")?;
                println!("verified 12 runs");
                Ok(())
            } else {
                io::outside(raw, out)?;
                io::write_new(out, &value)
            }
        }
        _ => Err("prepare <plain|detail> <root> | run <root> <inputs> <new-raw> | analyze|verify <raw> <results>".into()),
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
    #[test]
    fn current_path_rejects_missing_clocks_and_invalid_nesting() {
        let t = [0; 34];
        let mut c = [0; 34];
        c[18..22].fill(1);
        c[23..27].fill(1);
        c[28..31].fill(1);
        c[31] = 75_000;
        c[32] = 100_000;
        c[33] = 1;
        validate_detail(&t, &c, 1).unwrap();
        for i in 18..31 {
            let mut bad = c;
            bad[i] = 2;
            assert!(validate_detail(&t, &bad, 1).is_err(), "counter {i}");
        }
        for i in [17, 18, 23, 25, 26, 27, 28, 29, 30, 31, 32, 33] {
            let mut bad = t;
            bad[i] = 1;
            assert!(validate_detail(&bad, &c, 1).is_err(), "clock {i}");
        }
        c[33] = 0;
        assert!(validate_detail(&t, &c, 1).is_err());
        c[27] = 1;
        validate_detail(&t, &c, 1).unwrap();
        c[32] = 10;
        assert!(validate_detail(&t, &c, 1).is_err());
    }
}
