//! #1105：固定既有路径与候选，比较后继修复前后的完整几何及控制锚点。

use super::*;

pub(super) fn frozen_candidate(value: &Value) -> Candidate {
    let bits: [u64; 3] = serde_json::from_value(value["program"]["start"].clone()).unwrap();
    let segments = value["program"]["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|segment| {
            assert!(segment["canvas_selection"].is_null());
            let bits: [u64; 3] = serde_json::from_value(segment["end"].clone()).unwrap();
            re::RoadEditingCurveSegment::line(fixture_point(bits.map(f64::from_bits)))
        })
        .collect();
    let program =
        re::RoadEditingCurveProgram::try_new(fixture_point(bits.map(f64::from_bits)), segments)
            .unwrap();
    assert_eq!(
        program_bits(&program),
        value["program"],
        "frozen candidate bits"
    );
    Candidate {
        repaired: geom::RepairedCurve {
            program,
            quantized_arc_meters: value["arc_m"].as_f64().unwrap(),
        },
        spans: Vec::new(),
    }
}
