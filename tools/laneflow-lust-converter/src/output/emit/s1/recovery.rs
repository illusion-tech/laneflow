//! #1103：固定六条截断样本的分档预算与真实路径恢复研究。

use super::*;

const TRUNCATED_EDGES: [&str; 6] = [
    "sumo:--30316_0",
    "sumo:--30358#12_1",
    "sumo:--30452_0",
    "sumo::-10004_3_0",
    "sumo::-1000_2_0",
    "sumo::-10046_0_0",
];
const PARTITION_TIERS: [u64; 4] = [32_768, 65_536, 131_072, 262_144];

fn candidates(search: &sampler::EdgeSearch) -> Value {
    json!(
        search
            .candidates
            .iter()
            .map(|c| json!({
                "program":program_bits(&c.repaired.program),
                "arc_m":c.repaired.quantized_arc_meters,"spans":c.spans,
            }))
            .collect::<Vec<_>>()
    )
}

fn fresh_output() -> PathBuf {
    let out = PathBuf::from(std::env::var("S1_OUTPUT_DIR").expect("S1_OUTPUT_DIR"));
    assert!(!out.exists(), "fresh output directory");
    fs::create_dir_all(&out).unwrap();
    out
}

fn write_json(out: &std::path::Path, name: &str, value: &Value) {
    fs::write(
        out.join(name),
        format!("{}\n", serde_json::to_string_pretty(value).unwrap()),
    )
    .unwrap();
}

#[test]
#[ignore = "requires frozen S1_BENCH_INPUTS, S1_PARTITION_LIMIT and fresh S1_OUTPUT_DIR"]
fn fixed_sample_budget_measurement() {
    let partitions: u64 = std::env::var("S1_PARTITION_LIMIT")
        .expect("S1_PARTITION_LIMIT")
        .parse()
        .unwrap();
    assert!(PARTITION_TIERS.contains(&partitions), "frozen budget tier");
    let rows: BTreeMap<String, Value> = serde_json::from_slice(
        &fs::read(std::env::var("S1_BENCH_INPUTS").expect("S1_BENCH_INPUTS")).unwrap(),
    )
    .unwrap();
    assert_eq!(rows.len(), 17);
    let baseline: Value = serde_json::from_str(include_str!(
        "../../../../../../research/issue-1093-s1-efficiency/results.json"
    ))
    .unwrap();
    let frozen: Value = serde_json::from_str(include_str!(
        "../../../../../../research/issue-1093-s1-efficiency/inputs.json"
    ))
    .unwrap();
    assert_eq!(json!(rows), frozen, "complete frozen inputs");
    let out = fresh_output();
    let mut results = BTreeMap::new();
    let mut timing = BTreeMap::new();
    for (id, row) in rows {
        let input = measurement::input(&row);
        let old = geom::repair_curve(
            &input.points,
            input.clamps[0],
            input.clamps[1],
            input.clamp_sources[0].as_ref(),
            input.clamp_sources[1].as_ref(),
        );
        let selected = TRUNCATED_EDGES.contains(&id.as_str());
        if selected {
            assert_eq!(
                baseline["samples"][&id]["search"]["stop_reason"],
                "partitions"
            );
        }
        match &old {
            Ok(program) => {
                assert!(!selected);
                assert_eq!(program_bits(&program.program), row["original"]["program"]);
            }
            Err(error) => assert_eq!(error.to_string(), row["original"]["error"]),
        }
        let started = Instant::now();
        let result = match old {
            Ok(program) => {
                json!({"status":"preserved_success","program":program_bits(&program.program)})
            }
            Err(_) => {
                let budget = sampler::SearchBudget {
                    partitions: if selected {
                        partitions
                    } else {
                        sampler::PARTITION_LIMIT
                    },
                };
                let search = sampler::search(&input, budget);
                let values = candidates(&search);
                json!({"search":search.report,"candidates":values})
            }
        };
        timing.insert(id.clone(), started.elapsed().as_secs_f64());
        if result.get("search").is_some() && (!selected || partitions == sampler::PARTITION_LIMIT) {
            assert_eq!(result["search"], baseline["samples"][&id]["search"]);
            assert_eq!(result["candidates"], baseline["samples"][&id]["candidates"]);
        }
        results.insert(id, result);
    }
    write_json(
        &out,
        "results.json",
        &json!({
            "schema":"laneflow-1103-budget-sweep","partition_limit":partitions,
            "budgeted_edges":TRUNCATED_EDGES,"policy":sampler::policy(sampler::SearchBudget { partitions }),
            "other_edges_partition_limit":sampler::PARTITION_LIMIT,"samples":results,
        }),
    );
    write_json(&out, "timing.json", &json!(timing));
}
