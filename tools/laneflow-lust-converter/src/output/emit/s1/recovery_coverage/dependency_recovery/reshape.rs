//! #1115：有限几何候选的完整依赖分量验证，保留原成功几何及控制锚点。

use super::*;

mod stream_b_curve_design_c06_c08;

const EDGES: [&str; 2] = ["sumo::-16502_0_0", "sumo::-28860_3_0"];

fn frozen_inputs() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../../tests/fixtures/issue-1107-inputs.json"
    ))
    .unwrap()
}

fn run_searches(inputs: &Value) -> (Value, Value) {
    let mut searches = BTreeMap::new();
    let mut timing = BTreeMap::new();
    for id in EDGES {
        let input = measurement::input(&inputs[id]);
        let started = Instant::now();
        let search = sampler::reshape::search(&input);
        timing.insert(id, started.elapsed().as_secs_f64());
        searches.insert(
            id,
            json!({"search":search.report,"candidates":recovery::candidates(&search)}),
        );
    }
    (json!(searches), json!(timing))
}

#[test]
#[ignore = "requires fresh CURVE_DESIGN_OUTPUT_DIR"]
fn fixed_failed_curve_design() {
    let out = PathBuf::from(std::env::var("CURVE_DESIGN_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh curve design output");
    fs::create_dir_all(&out).unwrap();
    let (searches, timing) = run_searches(&frozen_inputs());
    recovery::write_json(&out, "searches.json", &searches);
    recovery::write_json(&out, "timing.json", &timing);
}
