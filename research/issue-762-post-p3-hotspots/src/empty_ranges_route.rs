//! 长路线普通 integration 构建的三组 ABBA；严格复用 #679 输入和矩阵校验。
#[path = "route_wall.rs"]
mod evidence;

use crate::{BASE, Result, io, need};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

pub(crate) fn capture(root: &Path, raw: &Path) -> Result<()> {
    let repo = std::env::current_dir()?;
    let head = io::git(&repo, &["rev-parse", "HEAD"])?;
    need(
        io::git(&repo, &["status", "--porcelain"])?.is_empty(),
        "dirty collector",
    )?;
    io::ensure_new(raw)?;
    fs::create_dir_all(raw)?;
    let mut identity = json!({"protocol":"empty-ranges-route-abba-v1", "head":head, "sources":{}, "binaries":{}, "completed":false});
    for arm in ["base", "candidate"] {
        let source = io::read_json(&root.join(format!("{arm}-plain-source.json")))?;
        need(
            source["source_files"] == io::source_index(&root.join(format!("{arm}-plain-source")))?,
            "source drift",
        )?;
        need(arm != "base" || source["base"] == BASE, "baseline drift")?;
        io::git(
            &repo,
            &[
                "merge-base",
                "--is-ancestor",
                source["base"].as_str().ok_or("source commit")?,
                &head,
            ],
        )?;
        identity["sources"][arm] = source;
        let binary = root
            .join(format!("{arm}-route{}", std::env::consts::EXE_SUFFIX))
            .canonicalize()?;
        identity["binaries"][arm] = json!({"path":binary, "sha256":io::sha(&binary)?});
    }
    io::write_new(&raw.join("identity.json"), &identity)?;
    for round in 0..3 {
        for (position, arm) in ["base", "candidate", "candidate", "base"]
            .iter()
            .enumerate()
        {
            let name = format!("{round}-{position}-{arm}");
            let binary = Path::new(
                identity["binaries"][arm]["path"]
                    .as_str()
                    .ok_or("binary path")?,
            );
            need(
                io::sha(binary)? == identity["binaries"][arm]["sha256"],
                "binary drift",
            )?;
            let output = Command::new(binary)
                .args([
                    "--exact",
                    "route_query_wall",
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .output()?;
            fs::write(raw.join(format!("{name}.stdout")), &output.stdout)?;
            fs::write(raw.join(format!("{name}.stderr")), &output.stderr)?;
            need(output.status.success(), "route process failed")?;
            evidence::wall(std::str::from_utf8(&output.stdout)?)?;
            need(
                io::sha(binary)? == identity["binaries"][arm]["sha256"],
                "binary drift",
            )?;
            need(
                io::git(&repo, &["rev-parse", "HEAD"])? == head
                    && io::git(&repo, &["status", "--porcelain"])?.is_empty(),
                "collector drift",
            )?;
            println!("route {name} complete");
        }
    }
    for arm in ["base", "candidate"] {
        need(
            identity["sources"][arm]["source_files"]
                == io::source_index(&root.join(format!("{arm}-plain-source")))?,
            "source drift",
        )?;
    }
    identity["completed"] = json!(true);
    io::replace_owned(&raw.join("identity.json"), &identity)
}

pub(crate) fn analyze(raw: &Path) -> Result<Value> {
    let identity = io::read_json(&raw.join("identity.json"))?;
    need(
        identity["completed"] == true
            && identity["protocol"] == "empty-ranges-route-abba-v1"
            && identity["sources"]["base"]["base"] == BASE,
        "route identity",
    )?;
    let mut rows = Vec::new();
    for round in 0..3 {
        for (position, arm) in ["base", "candidate", "candidate", "base"]
            .iter()
            .enumerate()
        {
            let name = format!("{round}-{position}-{arm}");
            let samples = evidence::wall(&fs::read_to_string(raw.join(format!("{name}.stdout")))?)?;
            for ((inner, red, repeats), values) in samples {
                let mut sorted = values.clone();
                sorted.sort_unstable();
                rows.push(json!({"round":round,"position":position,"arm":arm,"inner":inner,"red":red,"repeats":repeats,"mean_ns":values.iter().sum::<u64>() as f64/values.len() as f64,"p95_ns":sorted[(values.len()*95).div_ceil(100)-1],"samples_ns":values}));
            }
        }
    }
    Ok(json!({"identity":identity,"files":io::file_index(raw)?,"rows":rows}))
}
