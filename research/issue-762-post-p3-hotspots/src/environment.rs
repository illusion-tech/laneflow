use crate::{Result, io, need};
use serde_json::json;
use std::{
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn contenders(csv: &str) -> Result<Vec<String>> {
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
                "base-plain.exe",
                "candidate-plain.exe",
                "layout-plain.exe",
                "base-detail.exe",
                "candidate-detail.exe",
                "laneflow-urban-harness.exe",
                "wpr.exe",
            ]
            .contains(&name.as_str())
            .then(|| (*line).to_owned())
        })
        .collect())
}
pub(crate) fn observe(raw: &Path, label: &str, phase: &str) -> Result<()> {
    let output = Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .output()?;
    need(output.status.success(), "tasklist failed")?;
    let csv = String::from_utf8_lossy(&output.stdout).into_owned();
    let known = contenders(&csv)?;
    io::write_new(
        &raw.join(format!("{label}.environment-{phase}.json")),
        &json!({
            "label":label,"phase":phase,"tasklist_csv":csv,"known_contenders":known,
            "observed_ns":SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos().to_string()
        }),
    )?;
    need(
        known.is_empty(),
        "known contender observed; partial capture retained",
    )
}
pub(crate) fn verify(raw: &Path, label: &str) -> Result<()> {
    let process = io::read_json(&raw.join(format!("{label}.process.json")))?;
    let parse =
        |v: &serde_json::Value| -> Result<u128> { Ok(v.as_str().ok_or("snapshot time")?.parse()?) };
    let started = parse(&process["started"])?;
    let ended = parse(&process["ended"])?;
    need(started < ended, "process time order")?;
    for phase in ["before", "after"] {
        let record = io::read_json(&raw.join(format!("{label}.environment-{phase}.json")))?;
        need(
            record["label"] == label
                && record["phase"] == phase
                && record["known_contenders"] == json!([]),
            "boundary snapshot",
        )?;
        need(
            contenders(record["tasklist_csv"].as_str().ok_or("tasklist output")?)?.is_empty(),
            "snapshot contender disagreement",
        )?;
        let observed = parse(&record["observed_ns"])?;
        need(
            if phase == "before" {
                observed <= started
            } else {
                observed >= ended
            },
            "snapshot time order",
        )?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn localized_columns_are_irrelevant_but_compilers_and_harnesses_are_detected() {
        assert!(contenders("ERROR: denied").is_err());
        assert!(contenders("").is_err());
        assert!(
            contenders("\"System\",\"4\",\"服务\",\"0\",\"12 K\"")
                .unwrap()
                .is_empty()
        );
        for name in ["Rustc.exe", "base-plain.exe", "candidate-detail.exe"] {
            assert_eq!(
                contenders(&format!("\"{name}\",\"123\",\"Console\",\"1\",\"12 K\""))
                    .unwrap()
                    .len(),
                1
            );
        }
    }
}
