//! #1103：固定真实 LuST 拓扑上的恢复覆盖，先列清单再冻结队列。

use super::*;

mod assignments;
mod successors;
mod validation;

fn capture_lust() -> (observe::EmissionObservation, Value, Value) {
    use crate::convert::profiles::{convert_vehicle_profiles, select_passenger_vtypes};
    use crate::convert::topology::{
        TopologyConvertOptions, convert_network_topology_with_tll_and_profiles,
    };
    use crate::source::verify::prepare_verified_lust_inputs;
    use crate::sumo::{parse_sumo_network_xml, parse_tll_static_xml, parse_vtypes_xml};
    let started = Instant::now();
    let root = PathBuf::from(std::env::var("LUST_SOURCE_DIR").expect("LUST_SOURCE_DIR"));
    let prepared = prepare_verified_lust_inputs(&root).expect("verified pinned source");
    let network = parse_sumo_network_xml(prepared.net_xml()).unwrap();
    let tll = parse_tll_static_xml(prepared.tll_xml()).unwrap();
    let vtypes = parse_vtypes_xml(prepared.vtypes_xml()).unwrap();
    let profiles = convert_vehicle_profiles(&select_passenger_vtypes(&vtypes).unwrap()).unwrap();
    let options = TopologyConvertOptions {
        require_lust_location_anchors: true,
        emit_infeasibility_report: true,
        ..TopologyConvertOptions::default()
    };
    let prepare_seconds = started.elapsed().as_secs_f64();
    let convert_started = Instant::now();
    let (original, observation) = observe::capture(|| {
        convert_network_topology_with_tll_and_profiles(
            &network,
            &tll,
            &profiles,
            &options,
            prepared.report_source().clone(),
        )
    });
    let original = original.unwrap();
    let survey = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("evidence/lust-infeasible-survey.md"),
    )
    .unwrap();
    assert_eq!(
        original.infeasibility_report.unwrap().rendered,
        survey.replace("\r\n", "\n")
    );
    (
        observation,
        json!({"lust_commit":crate::source::LUST_COMMIT,"net_digest":prepared.report_source().net_digest()}),
        json!({"source_verify_and_parse_seconds":prepare_seconds,"baseline_conversion_seconds":convert_started.elapsed().as_secs_f64()}),
    )
}

fn original_programs(
    observation: &observe::EmissionObservation,
) -> BTreeMap<String, re::RoadEditingCurveProgram> {
    observation
        .edges
        .iter()
        .filter_map(|(id, result)| match result {
            observe::EdgeEmission::Emitted(program) => Some((id.clone(), program.clone())),
            observe::EdgeEmission::Failed(_) => None,
        })
        .collect()
}

#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR and fresh S1_OUTPUT_DIR; inventory only"]
fn pinned_lust_recovery_inventory() {
    let out = recovery::fresh_output();
    let (observation, source, timing) = capture_lust();
    let traffic = observation.traffic.as_ref().unwrap();
    let programs = original_programs(&observation);
    assert_eq!(programs.len(), 16_265);
    let emitted = programs.keys().collect::<Vec<_>>();
    let failed = observation
        .edges
        .iter()
        .filter_map(|(id, result)| match result {
            observe::EdgeEmission::Failed(error) => Some((id, error)),
            observe::EdgeEmission::Emitted(_) => None,
        })
        .collect::<BTreeMap<_, _>>();
    recovery::write_json(&out, "topology.json", &json!(traffic));
    recovery::write_json(
        &out,
        "inventory.json",
        &json!({"source":source,"emitted_edges":emitted,"failed_edges":failed,
            "paths":traffic.maneuver_paths.len(),"controllers":traffic.signals.controllers.len()}),
    );
    recovery::write_json(&out, "timing.json", &timing);
    println!("S1 recovery inventory saved={}", out.display());
}
