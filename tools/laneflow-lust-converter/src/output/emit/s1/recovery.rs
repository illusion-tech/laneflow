//! #1103：固定六条截断样本的分档预算与真实路径恢复研究。

use super::*;

pub(super) fn candidates(search: &sampler::EdgeSearch) -> Value {
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

pub(super) fn write_json(out: &std::path::Path, name: &str, value: &Value) {
    fs::write(
        out.join(name),
        format!("{}\n", serde_json::to_string_pretty(value).unwrap()),
    )
    .unwrap();
}
