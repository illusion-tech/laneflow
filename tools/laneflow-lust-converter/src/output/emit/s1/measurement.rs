//! #1093：固定真实输入的串行研究计时入口；包含候选诊断，排除全网准备。

use super::*;
use crate::output::geom::ClampSource;

fn source_value(source: &Option<ClampSource>) -> Value {
    match source {
        None => Value::Null,
        Some(ClampSource::PairTarget) => json!({"kind":"pair"}),
        Some(ClampSource::FanOutMean(pairs)) => json!({"kind":"fan_out","pairs":pairs}),
        Some(ClampSource::PartnerFollow(partner)) => json!({"kind":"partner","partner":partner}),
    }
}

fn source(value: &Value) -> Option<ClampSource> {
    match value["kind"].as_str() {
        None => None,
        Some("pair") => Some(ClampSource::PairTarget),
        Some("fan_out") => Some(ClampSource::FanOutMean(
            serde_json::from_value(value["pairs"].clone()).unwrap(),
        )),
        Some("partner") => Some(ClampSource::PartnerFollow(
            value["partner"].as_str().unwrap().to_owned(),
        )),
        _ => panic!("input clamp source"),
    }
}

pub(super) fn write_inputs(
    out: &std::path::Path,
    observation: &observe::EmissionObservation,
    samples: &Value,
) {
    let selected = samples["samples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["edge"].as_str().unwrap())
        .collect::<HashSet<_>>();
    let mut rows = BTreeMap::new();
    for ((id, original), input) in observation.edges.iter().zip(&observation.inputs) {
        if !selected.contains(id.as_str()) {
            continue;
        }
        let original = match original {
            observe::EdgeEmission::Emitted(p) => json!({"program":program_bits(p)}),
            observe::EdgeEmission::Failed(e) => json!({"error":e}),
        };
        rows.insert(
            id,
            json!({"points":input.points.iter().map(|p|p.map(f64::to_bits)).collect::<Vec<_>>(),
            "clamps":input.clamps.map(|c|c.map(|p|p.map(f64::to_bits))),
            "clamp_sources":input.clamp_sources.each_ref().map(source_value),"original":original}),
        );
    }
    assert_eq!(rows.len(), 17);
    fs::write(
        out.join("inputs.json"),
        format!("{}\n", serde_json::to_string_pretty(&rows).unwrap()),
    )
    .unwrap();
}

fn input(row: &Value) -> observe::EdgeInput {
    let points: Vec<[u64; 3]> = serde_json::from_value(row["points"].clone()).unwrap();
    let clamps: [Option<[u64; 3]>; 2] = serde_json::from_value(row["clamps"].clone()).unwrap();
    observe::EdgeInput {
        points: points.into_iter().map(|p| p.map(f64::from_bits)).collect(),
        clamps: clamps.map(|c| c.map(|p| p.map(f64::from_bits))),
        clamp_sources: [
            source(&row["clamp_sources"][0]),
            source(&row["clamp_sources"][1]),
        ],
    }
}

#[test]
#[ignore = "requires frozen S1_BENCH_INPUTS and fresh S1_OUTPUT_DIR"]
fn fixed_sample_search_measurement() {
    let rows: BTreeMap<String, Value> = serde_json::from_slice(
        &fs::read(std::env::var("S1_BENCH_INPUTS").expect("S1_BENCH_INPUTS")).unwrap(),
    )
    .unwrap();
    let samples: Value = serde_json::from_str(include_str!(
        "../../../../../../research/issue-1084-lust-sampling/samples.json"
    ))
    .unwrap();
    let out = PathBuf::from(std::env::var("S1_OUTPUT_DIR").expect("S1_OUTPUT_DIR"));
    assert!(!out.exists(), "fresh output directory");
    fs::create_dir_all(&out).unwrap();
    assert_eq!(rows.len(), 17);
    let mut results = BTreeMap::new();
    let mut times = BTreeMap::new();
    for (id, row) in rows {
        let sample = samples["samples"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["edge"] == id)
            .unwrap();
        assert_eq!(sample["input_points_f64_bits"], row["points"]);
        assert_eq!(sample["clamps_f64_bits"], row["clamps"]);
        let input = input(&row);
        let original = geom::repair_curve(
            &input.points,
            input.clamps[0],
            input.clamps[1],
            input.clamp_sources[0].as_ref(),
            input.clamp_sources[1].as_ref(),
        );
        match &original {
            Ok(p) => assert_eq!(program_bits(&p.program), row["original"]["program"]),
            Err(e) => assert_eq!(e.to_string(), row["original"]["error"].as_str().unwrap()),
        }
        let start = Instant::now();
        let result = match original {
            Ok(p) => json!({"status":"preserved_success","program":program_bits(&p.program)}),
            Err(_) => {
                let search = sampler::search(&input);
                json!({"search":search.report,"candidates":search.candidates.iter().map(|c|
                    json!({"program":program_bits(&c.repaired.program),"arc_m":c.repaired.quantized_arc_meters,"spans":c.spans})).collect::<Vec<_>>()})
            }
        };
        times.insert(id.clone(), start.elapsed().as_secs_f64());
        results.insert(id, result);
    }
    fs::write(
        out.join("results.json"),
        format!("{}\n", serde_json::to_string_pretty(&results).unwrap()),
    )
    .unwrap();
    fs::write(
        out.join("timing.json"),
        format!("{}\n", serde_json::to_string_pretty(&times).unwrap()),
    )
    .unwrap();
}

#[test]
fn clamp_provenance_round_trips() {
    for source in [
        None,
        Some(ClampSource::PairTarget),
        Some(ClampSource::FanOutMean(vec!["a->b".to_owned()])),
        Some(ClampSource::PartnerFollow("b".to_owned())),
    ] {
        assert_eq!(
            source_value(&source),
            source_value(&self::source(&source_value(&source)))
        );
    }
}
