//! #1105：固定既有路径与候选，比较后继修复前后的完整几何及控制锚点。

use super::*;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

const REFERENCE: &str =
    include_str!("../../../../../../../research/issue-1103-s1-recovery-budget/results.json");

fn normalized(value: &Value) -> Value {
    serde_json::from_str(&value.to_string()).unwrap()
}

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

pub(super) fn write_geometry(out: &Path, programs: &BTreeMap<String, re::RoadEditingCurveProgram>) {
    let mut writer =
        BufWriter::new(fs::File::create(out.join("successful-programs.jsonl")).unwrap());
    for (id, program) in programs {
        let points = program_points(program)
            .into_iter()
            .map(|p| p.map(|v| (v as f32).to_bits()))
            .collect::<Vec<_>>();
        let row =
            json!({"edge":id,"f64_program":program_bits(program),"canonical_f32_points":points});
        serde_json::to_writer(&mut writer, &row).unwrap();
        writer.write_all(b"\n").unwrap();
    }
    writer.flush().unwrap();
}

pub(super) fn assert_same_file(left: &Path, right: &Path) {
    let mut expected = BufReader::new(fs::File::open(left).unwrap()).lines();
    for actual in BufReader::new(fs::File::open(right).unwrap()).lines() {
        assert_eq!(
            expected.next().unwrap().unwrap(),
            actual.unwrap(),
            "complete baseline row"
        );
    }
    assert!(expected.next().is_none(), "complete baseline file");
}

fn changed_path_ids(before: &[Value], after: &[Value], fixed: bool) -> Vec<String> {
    assert_eq!(before.len(), 84);
    assert_eq!(after.len(), before.len());
    let mut changed = Vec::new();
    for (old, current) in before.iter().zip(after) {
        assert_eq!(current["path"], old["path"]);
        if normalized(current) == *old {
            continue;
        }
        assert!(
            fixed,
            "baseline differs from frozen evidence: {}",
            old["path"]
        );
        assert_eq!(current["edges"], old["edges"]);
        assert_eq!(old["result"]["stage"], "compile");
        assert_eq!(old["result"]["status"], "target_check_not_completed");
        assert!(
            old["result"]["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .all(|d| {
                    d.as_str()
                        .unwrap()
                        .starts_with("LF-COMP-DISCONNECTED-MANEUVER-PATH ")
                })
        );
        assert_eq!(current["result"]["status"], "accepted", "{current}");
        assert!(
            !current["result"]["dependency_paths"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        changed.push(current["path"].as_str().unwrap().to_owned());
    }
    assert_eq!(changed.len(), if fixed { 2 } else { 0 });
    changed
}

fn production_closures(
    out: &Path,
    observation: &observe::EmissionObservation,
    fixed: bool,
) -> Value {
    use crate::output::model::{Centerline, SpatialEdge};

    let traffic = observation.traffic.as_ref().unwrap();
    let mut results = Vec::new();
    let mut all_programs = BTreeMap::new();
    for (index, junction) in ["sumo:-20604", "sumo:-22774"].into_iter().enumerate() {
        let movement_ids = traffic
            .movements
            .iter()
            .filter(|m| m.junction_id == junction)
            .map(|m| m.id.as_str())
            .collect::<HashSet<_>>();
        let path = traffic
            .maneuver_paths
            .iter()
            .find(|p| movement_ids.contains(p.movement_id.as_str()))
            .unwrap();
        let part = clip(traffic, path);
        let edges = part
            .lane_graph
            .edges
            .iter()
            .map(|e| e.id.as_str())
            .collect::<HashSet<_>>();
        let spatial = SpatialPackage {
            format_version: "0.1",
            frame_id: LUST_FRAME_ID.to_owned(),
            edges: observation
                .edges
                .iter()
                .zip(&observation.inputs)
                .filter(|((id, _), _)| edges.contains(id.as_str()))
                .map(|((id, _), input)| SpatialEdge {
                    traffic_edge_id: id.clone(),
                    centerline: Centerline {
                        points: input.points.clone(),
                    },
                })
                .collect(),
        };
        let internal = part
            .maneuver_paths
            .iter()
            .flat_map(|p| p.internal_edge_ids.clone())
            .collect();
        let (result, captured) =
            observe::capture(|| compile_network_lfca(&part, &spatial, &internal));
        let programs = original_programs(&captured);
        assert_eq!(programs.len(), edges.len(), "complete production geometry");
        all_programs.extend(programs);
        let result = if fixed {
            let artifact = result.expect("complete production signal closure");
            assert!(!artifact.network_lfca.is_empty());
            assert!(artifact.infeasibility_report.is_none());
            fs::write(
                out.join(format!("signal-closure-{index}.lfca")),
                &artifact.network_lfca,
            )
            .unwrap();
            json!({"status":"accepted","lane_edges":artifact.counts.lane_edges,
                "maneuver_paths":artifact.counts.maneuver_paths,"controllers":artifact.counts.signal_controllers,
                "groups":artifact.counts.signal_groups,"phases":artifact.counts.signal_phases,
                "stop_lines":artifact.counts.stop_lines,"maneuver_gates":artifact.counts.maneuver_gates,
                "lfca_bytes":artifact.network_lfca.len()})
        } else {
            let error = result.expect_err("pre-fix topology rejection").to_string();
            assert!(
                error.contains("LF-COMP-DISCONNECTED-MANEUVER-PATH"),
                "{error}"
            );
            json!({"status":"topology_rejected","error":error})
        };
        results.push(
            json!({"junction":junction,"dependency_paths":part.maneuver_paths,
            "signals":part.signals,"result":result}),
        );
    }
    let geometry_dir = out.join("production-closures");
    fs::create_dir(&geometry_dir).unwrap();
    write_geometry(&geometry_dir, &all_programs);
    json!(results)
}

#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, fresh DIRECT_SUCCESSOR_OUTPUT_DIR and optional frozen DIRECT_SUCCESSOR_BASELINE_DIR"]
fn pinned_lust_direct_successor_regression() {
    let out = PathBuf::from(std::env::var("DIRECT_SUCCESSOR_OUTPUT_DIR").unwrap());
    assert!(!out.exists(), "fresh evidence output");
    fs::create_dir_all(&out).unwrap();
    let baseline = std::env::var_os("DIRECT_SUCCESSOR_BASELINE_DIR").map(PathBuf::from);
    let fixed = baseline.is_some();
    let reference: Value = serde_json::from_str(REFERENCE).unwrap();
    let (observation, source, timing) = capture_lust();
    assert_eq!(source, reference["source"]);
    let traffic = observation.traffic.as_ref().unwrap();
    let programs = original_programs(&observation);
    assert_eq!(programs.len(), 16_265);
    write_geometry(&out, &programs);
    recovery::write_json(&out, "topology.json", &json!(traffic));
    let failures = observation
        .edges
        .iter()
        .filter_map(|(id, result)| {
            if let observe::EdgeEmission::Failed(error) = result {
                Some((id, error))
            } else {
                None
            }
        })
        .collect::<BTreeMap<_, _>>();
    recovery::write_json(&out, "failed-edges.json", &json!(failures));
    if let Some(baseline) = &baseline {
        for name in [
            "successful-programs.jsonl",
            "topology.json",
            "failed-edges.json",
        ] {
            assert_same_file(&baseline.join(name), &out.join(name));
        }
    }
    let paths = reference["baseline_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            traffic
                .maneuver_paths
                .iter()
                .find(|p| p.id == row["path"].as_str().unwrap())
                .unwrap()
        })
        .collect::<Vec<_>>();
    let audit = paths
        .iter()
        .map(|p| (&p.id, validation::closure_audit(traffic, p)))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        normalized(&json!(audit)),
        reference["closure_audit"],
        "complete control closure"
    );
    let baseline_paths = paths.iter().map(|p| json!({"path":p.id,"edges":path_edges(p),"result":compile_path(traffic,p,&programs)})).collect::<Vec<_>>();
    let changed = changed_path_ids(
        reference["baseline_paths"].as_array().unwrap(),
        &baseline_paths,
        fixed,
    );
    let changed_junctions = paths
        .iter()
        .filter(|p| changed.contains(&p.id))
        .map(|p| {
            &traffic
                .movements
                .iter()
                .find(|m| m.id == p.movement_id)
                .unwrap()
                .junction_id
        })
        .collect::<std::collections::BTreeSet<_>>();
    if fixed {
        assert_eq!(
            json!(changed_junctions),
            json!(["sumo:-20604", "sumo:-22774"])
        );
    }
    let production = production_closures(&out, &observation, fixed);
    if let Some(baseline) = &baseline {
        assert_same_file(
            &baseline.join("production-closures/successful-programs.jsonl"),
            &out.join("production-closures/successful-programs.jsonl"),
        );
        let old: Value =
            serde_json::from_slice(&fs::read(baseline.join("results.json")).unwrap()).unwrap();
        for (before, after) in old["production_closures"]
            .as_array()
            .unwrap()
            .iter()
            .zip(production.as_array().unwrap())
        {
            assert_eq!(
                normalized(&after["signals"]),
                before["signals"],
                "production controls"
            );
            assert_eq!(
                normalized(&after["dependency_paths"]),
                before["dependency_paths"],
                "complete production closure"
            );
        }
    }
    let mut tiers = BTreeMap::new();
    for tier in ["32768", "262144"] {
        let original = &reference["tiers"][tier];
        let pools = original["searches"]
            .as_object()
            .unwrap()
            .iter()
            .filter_map(|(id, row)| {
                let values = row["candidates"].as_array().unwrap();
                (!values.is_empty())
                    .then(|| (id.clone(), values.iter().map(frozen_candidate).collect()))
            })
            .collect::<BTreeMap<_, _>>();
        let current = assignments::evaluate(traffic, &paths, &programs, &pools);
        assert_eq!(
            current["candidate_pool_sizes"],
            original["assignments"]["candidate_pool_sizes"]
        );
        assert_eq!(
            current["best"]["assignment"],
            original["assignments"]["best"]["assignment"]
        );
        let tier_changed = changed_path_ids(
            original["assignments"]["best"]["paths"].as_array().unwrap(),
            current["best"]["paths"].as_array().unwrap(),
            fixed,
        );
        assert_eq!(tier_changed, changed);
        for (old, trial) in original["assignments"]["trials"]
            .as_array()
            .unwrap()
            .iter()
            .zip(current["trials"].as_array().unwrap())
        {
            assert_eq!(
                old["assignment"], trial["assignment"],
                "frozen assignment sequence"
            );
            assert_eq!(
                trial["counts"]["accepted"].as_u64().unwrap(),
                old["counts"]["accepted"].as_u64().unwrap() + if fixed { 2 } else { 0 }
            );
        }
        tiers.insert(tier, current);
    }
    recovery::write_json(
        &out,
        "results.json",
        &json!({
            "schema":"laneflow-1105-direct-successor-regression","source":source,
            "compared_to_pre_fix":fixed,"successful_program_count":programs.len(),"failed_edge_count":failures.len(),
            "all_f64_programs_and_f32_points_unchanged":fixed,"complete_topology_and_controls_unchanged":fixed,
            "target_paths":paths.len(),"changed_paths":changed,"changed_junctions":changed_junctions,
            "closure_audit":audit,"baseline_paths":baseline_paths,"tiers":tiers,"production_closures":production,
            "candidates_from_frozen_evidence":true,"search_executed":false,"accepted_paths_pass_post_emission":true,
        }),
    );
    recovery::write_json(&out, "timing.json", &timing);
    println!("Direct successor regression saved={}", out.display());
}
