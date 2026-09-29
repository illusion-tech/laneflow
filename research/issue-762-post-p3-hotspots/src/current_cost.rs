//! #793 当前主干短窗：只记录协调器墙钟，沿用冻结采集与交通一致性校验。
mod current_export;
#[allow(dead_code)]
mod p2_cost;
use p2_cost::io;
use serde_json::{Value, json};
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
fn environment_quality(pre: &Value, post: &Value) -> Value {
    let snapshots = [pre, post];
    let captured = snapshots.iter().all(|s| s["known_contenders"].is_array());
    let observed = snapshots.iter().any(|s| {
        s["known_contenders"]
            .as_array()
            .is_some_and(|p| !p.is_empty())
    });
    json!({
        "usage": "hotspot-screening-only",
        "performance_baseline_accepted": false,
        "pre_post_snapshots_captured": captured,
        "known_contenders_observed": observed,
        "continuous_process_frequency_temperature_observation": false,
        "note": "Snapshots cannot establish a contention-free interval or attribute timing drift. Semantic verification is separate from performance acceptance."
    })
}
fn analyze(raw: &Path) -> Result<Value> {
    let identity = io::read_json(&raw.join("identity.json"))?;
    let scale = identity["matrix_scale"].as_str();
    need(
        identity["matrix_scale"].is_null() || scale == Some("100k"),
        "current matrix scale",
    )?;
    let mut value = p2_cost::analyze_scoped_for(raw, PROTOCOL, scale)?;
    let snapshot = |name: &str| -> Result<Value> {
        let path = raw.join(name);
        if path.try_exists()? {
            io::read_json(&path)
        } else {
            Ok(Value::Null)
        }
    };
    value["quality"] = environment_quality(
        &snapshot("environment-pre.json")?,
        &snapshot("environment-post.json")?,
    );
    if scale.is_some() {
        for run in value["runs"].as_array().ok_or("runs")? {
            for phase in ["before", "after"] {
                let label = run["label"].as_str().ok_or("label")?;
                let record = io::read_json(&raw.join(format!("{label}.environment-{phase}.json")))?;
                need(
                    record["label"] == label
                        && record["phase"] == phase
                        && record["known_contenders"] == json!([]),
                    "per-run contender observation",
                )?;
                need(
                    known_contenders(record["tasklist_csv"].as_str().ok_or("tasklist output")?)?
                        == Vec::<String>::new(),
                    "per-run tasklist disagreement",
                )?;
            }
        }
        value["quality"]["per_run_boundary_snapshots_clear"] = json!(true);
    }
    Ok(value)
}

fn known_contenders(csv: &str) -> Result<Vec<String>> {
    let lines: Vec<_> = csv.lines().filter(|line| !line.trim().is_empty()).collect();
    need(
        !lines.is_empty()
            && lines
                .iter()
                .all(|line| line.starts_with('"') && line.split('"').count() >= 5),
        "unrecognized tasklist output",
    )?;
    Ok(lines
        .iter()
        .filter_map(|line| {
            let name = line.split('"').nth(1)?.to_ascii_lowercase();
            [
                "cargo.exe",
                "rustc.exe",
                "plain.exe",
                "detail.exe",
                "laneflow-urban-harness.exe",
                "wpr.exe",
            ]
            .contains(&name.as_str())
            .then(|| (*line).to_owned())
        })
        .collect())
}

fn observe(raw: &Path, label: &str, phase: &str) -> Result<()> {
    let output = std::process::Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .output()?;
    need(output.status.success(), "tasklist failed")?;
    // 映像名及 CSV 分隔符均为 ASCII；保留本地化输出，不依赖内存列单位。
    let csv = String::from_utf8_lossy(&output.stdout).into_owned();
    let contenders = known_contenders(&csv)?;
    let record = json!({"label":label,"phase":phase,"tasklist_csv":csv,"known_contenders":contenders,"observed_ns":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos().to_string()});
    io::write_new(
        &raw.join(format!("{label}.environment-{phase}.json")),
        &record,
    )?;
    need(
        contenders.is_empty(),
        "known contender observed; partial capture retained",
    )
}
fn run() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    match a.first().map(String::as_str) {
        Some("prepare") if a.len() == 3 => current_export::export(Path::new(&a[2]), &a[1]),
        Some("run") if a.len() == 4 => p2_cost::capture_for(Path::new(&a[1]), Path::new(&a[2]), Path::new(&a[3]), PROTOCOL),
        Some("run-100k") if a.len() == 4 => p2_cost::capture_scoped_for(Path::new(&a[1]), Path::new(&a[2]), Path::new(&a[3]), PROTOCOL, Some("100k"), observe),
        Some("analyze" | "verify") if a.len() == 3 => {
            let raw = Path::new(&a[1]);
            let out = Path::new(&a[2]);
            let value = analyze(raw)?;
            if a[0] == "verify" {
                need(value == io::read_json(out)?, "published mismatch")?;
                println!("verified {} runs; hotspot screening only, see quality", value["runs"].as_array().ok_or("runs")?.len());
                Ok(())
            } else {
                io::outside(raw, out)?;
                io::write_new(out, &value)
            }
        }
        _ => Err("prepare <plain|detail> <root> | run|run-100k <root> <inputs> <new-raw> | analyze|verify <raw> <results>".into()),
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
    fn contender_parser_rejects_unknown_output_and_finds_compiler() {
        assert!(known_contenders("").is_err());
        assert!(known_contenders("ERROR: Access denied").is_err());
        assert!(
            known_contenders("\"System\",\"4\",\"Services\",\"0\",\"12 K\"")
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            known_contenders("\"Rustc.exe\",\"124\",\"Console\",\"1\",\"100 K\"")
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn missing_or_empty_snapshots_do_not_certify_isolation() {
        let empty = json!({"known_contenders": []});
        let busy = json!({"known_contenders": [{"Name": "rustc.exe"}]});
        let missing = environment_quality(&Value::Null, &empty);
        assert_eq!(missing["pre_post_snapshots_captured"], false);
        for (pre, post, observed) in [
            (&empty, &empty, false),
            (&empty, &busy, true),
            (&busy, &empty, true),
        ] {
            let q = environment_quality(pre, post);
            assert_eq!(q["known_contenders_observed"], observed);
            assert_eq!(q["performance_baseline_accepted"], false);
        }
    }
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
