//! #679 已归档原始样本的完整性与统计复核；不需要采样机器或 Windows。
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::PathBuf};

type Key = (u32, bool, usize);
type Samples = BTreeMap<Key, Vec<u64>>;

fn directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../research/issue-679-route-query/evidence")
}

fn fields(line: &str) -> BTreeMap<&str, &str> {
    line.split_whitespace()
        .skip(1)
        .filter_map(|item| item.split_once('='))
        .collect()
}

fn wall(text: &str) -> Result<Samples, String> {
    let mut samples: Samples = BTreeMap::new();
    let mut ends = BTreeMap::new();
    let mut inputs = BTreeMap::new();
    for line in text.lines().filter(|line| line.starts_with("route-")) {
        let row = fields(line);
        let value = |name| {
            row.get(name)
                .copied()
                .ok_or_else(|| format!("missing {name}"))
        };
        let number = |name| {
            value(name)?
                .parse::<u64>()
                .map_err(|_| format!("invalid {name}"))
        };
        let round = number("round")?;
        let red = match value("red")? {
            "true" => true,
            "false" => false,
            _ => return Err("red".into()),
        };
        let repeats = number("repeats")?;
        if round >= 3 || ![8, 128, 2_048].contains(&repeats) {
            return Err("matrix axis".into());
        }
        let key = (round as u32, red, repeats as usize);
        if line.starts_with("route-tick ") {
            let values = samples.entry(key).or_default();
            if number("tick")? != values.len() as u64 || values.len() >= 128 || number("ns")? == 0 {
                return Err("sample order/count/time".into());
            }
            values.push(number("ns")?);
        } else if line.starts_with("route-end ") {
            if ends.insert(key, number("build_ns")?).is_some() {
                return Err("duplicate end".into());
            }
            if number("vehicles")? != 512
                || number("warm")? != 16
                || number("steps")? != 128
                || number("state_sum")? != 1_332_736_691
            {
                return Err("workload identity".into());
            }
            let input = value("input")?;
            if inputs
                .insert(red, input)
                .is_some_and(|previous| previous != input)
            {
                return Err("input drift".into());
            }
        } else {
            return Err("unknown row".into());
        }
    }
    if samples.len() != 18
        || ends.len() != 18
        || samples.values().any(|values| values.len() != 128)
        || samples.keys().ne(ends.keys())
        || !text.contains("test result: ok. 1 passed; 0 failed;")
    {
        return Err("incomplete matrix".into());
    }
    Ok(samples)
}

#[test]
fn archived_route_query_evidence_is_complete() {
    let root = directory();
    let manifest: toml::Value =
        toml::from_str(&fs::read_to_string(root.join("manifest.toml")).unwrap()).unwrap();
    assert_eq!(manifest["schema"].as_integer(), Some(1));
    for key in ["wall_exit", "diagnostic_exit"] {
        assert_eq!(manifest[key].as_integer(), Some(0));
    }
    let files = manifest["files"].as_table().unwrap();
    assert_eq!(files.len(), 7);
    for name in [
        "cpu-baseline.log",
        "diagnostic-monitor.log",
        "diagnostic.log",
        "diagnostic.stderr",
        "wall-monitor.log",
        "wall.log",
        "wall.stderr",
    ] {
        assert!(files.contains_key(name), "missing manifest entry {name}");
    }
    assert_eq!(
        manifest["source_commit"].as_str().unwrap(),
        "5c93c0413ec6b591ec8d4972358cc70e4872bf39"
    );
    for key in ["source_clean_before", "source_clean_after", "source_pushed"] {
        assert_eq!(manifest[key].as_bool(), Some(true));
    }
    for (name, expected) in manifest["files"].as_table().unwrap() {
        assert!(!name.contains(['/', '\\']));
        let actual: String = Sha256::digest(fs::read(root.join(name)).unwrap())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(actual, expected.as_str().unwrap(), "hash {name}");
    }
    let samples = wall(&fs::read_to_string(root.join("wall.log")).unwrap()).unwrap();
    for ((round, red, repeats), values) in samples {
        let mut sorted = values.clone();
        sorted.sort_unstable();
        println!(
            "verified round={round} red={red} repeats={repeats} mean_us={:.3} p95_us={:.3}",
            values.iter().sum::<u64>() as f64 / 128_000.0,
            sorted[121] as f64 / 1_000.0
        );
    }
    let diagnostic = fs::read_to_string(root.join("diagnostic.log")).unwrap();
    assert!(diagnostic.contains("test result: ok. 1 passed; 0 failed;"));
    for red in [false, true] {
        for repeats in [8, 128, 2_048] {
            let case = format!("red-{red}-repeats-{repeats}");
            let rows: Vec<_> = diagnostic
                .lines()
                .filter(|line| fields(line).get("case") == Some(&case.as_str()))
                .collect();
            assert_eq!(
                rows.iter()
                    .filter(|line| line.starts_with("route-memory "))
                    .count(),
                1
            );
            let barrier: Vec<_> = rows
                .iter()
                .filter(|line| line.starts_with("route-barrier "))
                .collect();
            assert_eq!(barrier.len(), 1);
            for field in ["signal_gates", "conflict_scans", "waiting_scans"] {
                assert_eq!(fields(barrier[0])[field], "0");
            }
            let index: Vec<_> = rows
                .iter()
                .filter(|line| line.starts_with("route-index "))
                .collect();
            assert_eq!(index.len(), 3);
            for (round, line) in index.iter().enumerate() {
                let row = fields(line);
                assert_eq!(row["round"], round.to_string());
                assert_eq!(row["queries"], "262144");
                assert_eq!(row["bytes"], (24 * (3 * repeats + 1)).to_string());
            }
            let searches: Vec<_> = rows
                .iter()
                .filter(|line| line.starts_with("route-search "))
                .collect();
            assert_eq!(searches.len(), 16);
            for line in searches {
                assert_eq!(fields(line)["calls"], "65536");
            }
        }
    }
}

#[test]
fn route_query_verifier_rejects_missing_duplicate_and_bad_samples() {
    let valid = fs::read_to_string(directory().join("wall.log")).unwrap();
    let row = valid
        .lines()
        .find(|line| line.starts_with("route-tick "))
        .unwrap();
    assert!(wall(&valid.replacen(row, "", 1)).is_err());
    assert!(wall(&valid.replacen(row, &format!("{row}\n{row}"), 1)).is_err());
    assert!(wall(&valid.replacen("repeats=8", "repeats=9", 1)).is_err());
    assert!(wall(&valid.replace("state_sum=1332736691", "state_sum=0")).is_err());
}
