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

pub(super) fn input(row: &Value) -> observe::EdgeInput {
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
