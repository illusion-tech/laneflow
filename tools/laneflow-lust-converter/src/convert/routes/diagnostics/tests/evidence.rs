use std::{fs, path::PathBuf, process::Command};

use serde::Serialize;
use serde_json::{Value, json};

use super::*;
use crate::convert::{
    TopologyConvertOptions,
    population::PopulationOrdinal,
    routes::test_support::{PathEvidence, diagnostic_options, expected_final_chain, path_evidence},
};

#[derive(Serialize)]
struct CaseEvidence {
    name: &'static str,
    test: &'static str,
    input: String,
    source_paths: Vec<PathEvidence>,
    report: Value,
    combined: Option<CombinedObservation>,
    not_executed: Vec<&'static str>,
}

#[derive(Serialize)]
struct CombinedObservation {
    stage: &'static str,
    stage_evidence: &'static str,
    outcome: &'static str,
    direct_route_succeeded: bool,
    network_lfca_bytes: Option<usize>,
    routes_toml_bytes: Option<usize>,
    route_count: Option<usize>,
    population_record_count: Option<usize>,
    error: Option<Value>,
    ordinary_diagnostic_calls: usize,
}

#[derive(Serialize)]
struct CostObservation {
    ordinary_diagnostic_calls_before: usize,
    diagnostic_calls_after: usize,
    metrics: Value,
}

#[derive(Serialize)]
struct ComparisonCheck {
    name: &'static str,
    equal: bool,
    baseline: Value,
    current: Value,
}

#[derive(Serialize)]
struct A0Comparison {
    case: &'static str,
    equal: bool,
    checks: Vec<ComparisonCheck>,
}

#[derive(Serialize)]
struct EvidenceReport {
    code: Value,
    baseline_code: Value,
    export_command: String,
    a0_comparisons: Vec<A0Comparison>,
    cases: Vec<CaseEvidence>,
    cost: CostObservation,
    unexecuted: Vec<&'static str>,
}

#[test]
fn a1_route_evidence() {
    let cases = observed_cases();
    validate_cases(&cases);
    let cost = observe_cost();
    let Some(output) = std::env::var_os("LANEFLOW_806_A1_RESULTS").map(PathBuf::from) else {
        return;
    };
    let baseline_path = std::env::var_os("LANEFLOW_806_A0_BASELINE")
        .map(PathBuf::from)
        .expect("A0 baseline is required for A1 evidence export");
    let baseline: Value =
        serde_json::from_slice(&fs::read(&baseline_path).expect("read A0 baseline results"))
            .expect("parse A0 baseline results");
    let baseline_code = baseline["code"].clone();
    let a0_comparisons = compare_a0(&baseline, &cases);
    assert!(a0_comparisons.iter().all(|comparison| comparison.equal));
    let code = export_metadata();
    let report = EvidenceReport {
        export_command: export_command(&output, &baseline_path),
        code,
        baseline_code,
        a0_comparisons,
        cases,
        cost,
        unexecuted: vec![
            "Runtime route registration and vehicle admission",
            "fixed-tick and multi-vehicle safety",
            "ignored pinned full-LuST, migration, TOPO/DEMAND and Runtime simulation",
        ],
    };
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).expect("create A1 evidence directory");
    }
    let bytes = crate::output::json_bytes("1085 route diagnostics", &report)
        .expect("serialize A1 evidence");
    fs::write(&output, bytes).expect("write A1 results.json");
    println!(
        "A1_RESULTS={} CASES={} HEAD={}",
        output.display(),
        report.cases.len(),
        report.code["head"].as_str().expect("metadata head")
    );
}

fn observed_cases() -> Vec<CaseEvidence> {
    vec![
        fixture_case(
            "direct-gap",
            "a1_gap_and_missing_hop_have_distinct_diagnostics",
            RouteFixture::new(RouteFixtureKind::Gap),
            false,
        ),
        fixture_case(
            "missing-hop",
            "a1_gap_and_missing_hop_have_distinct_diagnostics",
            RouteFixture::new(RouteFixtureKind::MissingHop),
            false,
        ),
        record_case(
            "branch-success",
            "a1_branch_success_does_not_report_first_dead_branch",
            RouteFixtureKind::Gap,
            population_record("branch-success", &["B", "C"]),
        ),
        fixture_case(
            "two-lane-origin",
            "a1_branch_success_does_not_report_first_dead_branch",
            RouteFixture::new(RouteFixtureKind::TwoLaneOrigin),
            false,
        ),
        record_case(
            "all-failed-start-lanes",
            "a1_all_failed_start_lanes_use_full_reachable_set",
            RouteFixtureKind::MissingHop,
            population_record("all-fail", &["B", "C"]),
        ),
        fixture_case(
            "repeated-success",
            "a1_repeated_occurrence_is_located_by_index",
            RouteFixture::new(RouteFixtureKind::LoopBack).with_road_sequence(&["A", "B", "C", "B"]),
            false,
        ),
        fixture_case(
            "repeated-gap",
            "a1_repeated_occurrence_is_located_by_index",
            RouteFixture::new(RouteFixtureKind::LoopBack)
                .with_road_sequence(&["A", "B", "C", "B", "C"]),
            false,
        ),
        record_case(
            "unknown-road-direct",
            "a1_source_errors_and_unclassified_boundaries_are_distinct",
            RouteFixtureKind::Gap,
            population_record("unknown-road", &["A", "B", "GHOST"]),
        ),
        record_case(
            "internal-road-direct",
            "a1_source_errors_and_unclassified_boundaries_are_distinct",
            RouteFixtureKind::Gap,
            population_record("internal-road", &["A", "B", ":J2_0"]),
        ),
        normalization_case(),
        record_case(
            "empty-unclassified",
            "a1_source_errors_and_unclassified_boundaries_are_distinct",
            RouteFixtureKind::Gap,
            population_record("empty", &[]),
        ),
        record_case(
            "single-road",
            "a1_source_errors_and_unclassified_boundaries_are_distinct",
            RouteFixtureKind::Gap,
            population_record("single", &["B"]),
        ),
        injection_case(),
        fixture_case(
            "continuous-direct",
            "a1_continuous_combined_static_result_is_recorded",
            RouteFixture::new(RouteFixtureKind::Continuous),
            true,
        ),
    ]
}

fn fixture_case(
    name: &'static str,
    test: &'static str,
    fixture: RouteFixture,
    combined: bool,
) -> CaseEvidence {
    let network = fixture.network();
    let topology = fixture.normalized_topology(&network);
    let population = fixture.selected_population();
    let report = diagnose_population(&network, &topology, &population);
    let combined = combined.then(|| combined_observation(&fixture, &report));
    CaseEvidence {
        name,
        test,
        input: format!("{:?}", fixture.kind),
        source_paths: normalized_paths(&topology),
        report: diagnostic_json(&report),
        combined,
        not_executed: unexecuted_stages(),
    }
}

fn record_case(
    name: &'static str,
    test: &'static str,
    fixture_kind: RouteFixtureKind,
    population: PopulationRecord,
) -> CaseEvidence {
    let fixture = RouteFixture::new(fixture_kind);
    let network = fixture.network();
    let topology = fixture.normalized_topology(&network);
    let report = diagnose_population(&network, &topology, &[population]);
    CaseEvidence {
        name,
        test,
        input: format!("{fixture_kind:?}; direct population record"),
        source_paths: normalized_paths(&topology),
        report: diagnostic_json(&report),
        combined: None,
        not_executed: unexecuted_stages(),
    }
}

fn normalization_case() -> CaseEvidence {
    let fixture = RouteFixture::new(RouteFixtureKind::UnknownLane);
    let network = fixture.network();
    let error = normalize_junctions(
        &network,
        &crate::convert::junction::StubWeldGate::Unrestricted,
    )
    .expect_err("unknown lane normalization");
    let record = diagnose_normalization_failure(error);
    let formal = Err(record.original_error.clone().expect("normalization error"));
    let report = RouteDiagnosticReport {
        formal,
        records: vec![record],
        metrics: DiagnosticMetrics::default(),
    };
    CaseEvidence {
        name: "normalization-unknown-lane",
        test: "a1_source_errors_and_unclassified_boundaries_are_distinct",
        input: "UnknownLane; normalization has no population context".to_owned(),
        source_paths: Vec::new(),
        report: diagnostic_json(&report),
        combined: None,
        not_executed: unexecuted_stages(),
    }
}

fn injection_case() -> CaseEvidence {
    let fixture = RouteFixture::new(RouteFixtureKind::Continuous);
    let network = fixture.network();
    let topology = fixture.normalized_topology(&network);
    let population = fixture.selected_population();
    let report = diagnose_population_with_materialize_injection(&network, &topology, &population);
    CaseEvidence {
        name: "materialization-injection",
        test: "a1_materialization_failure_is_limited_to_actual_entry_and_injection",
        input: "Continuous; test-only materialize-entry failure injection".to_owned(),
        source_paths: normalized_paths(&topology),
        report: diagnostic_json(&report),
        combined: None,
        not_executed: unexecuted_stages(),
    }
}

fn unexecuted_stages() -> Vec<&'static str> {
    vec![
        "runtime-route-registration",
        "vehicle-placement-and-admission",
        "fixed-tick-execution",
        "full-population-diagnostics",
    ]
}

fn normalized_paths(topology: &NormalizedTopology) -> Vec<PathEvidence> {
    let mut connections = topology.path_by_connection.keys().collect::<Vec<_>>();
    connections.sort();
    connections
        .into_iter()
        .map(|(from, from_lane, to, to_lane)| {
            path_evidence(topology, from, *from_lane, to, *to_lane)
        })
        .collect()
}

fn combined_observation(
    fixture: &RouteFixture,
    direct_report: &RouteDiagnosticReport,
) -> CombinedObservation {
    reset_diagnostic_entry_count();
    let result = fixture.combined(&TopologyConvertOptions::default());
    let ordinary_diagnostic_calls = diagnostic_entry_count();
    let direct_route_succeeded = direct_report
        .records
        .iter()
        .all(|record| record.kind == RouteDiagnosticKind::Success);
    match result {
        Ok(artifacts) => CombinedObservation {
            stage: "static-conversion",
            stage_evidence: "direct route succeeded; combined static conversion returned artifacts",
            outcome: "returned",
            direct_route_succeeded,
            network_lfca_bytes: Some(artifacts.topology.network_lfca.len()),
            routes_toml_bytes: artifacts.routes_toml.as_ref().map(Vec::len),
            route_count: Some(artifacts.route_count),
            population_record_count: Some(artifacts.population_record_count),
            error: None,
            ordinary_diagnostic_calls,
        },
        Err(error) => {
            let geometry = error.to_string().contains("repaired curve");
            CombinedObservation {
                stage: if geometry {
                    "geometry-repair"
                } else {
                    "post-route-static-conversion"
                },
                stage_evidence: if geometry {
                    "direct route succeeded; output/emit.rs geometry repair returned this error"
                } else {
                    "direct route succeeded; combined conversion returned after the route stage"
                },
                outcome: "post-route-error",
                direct_route_succeeded,
                network_lfca_bytes: None,
                routes_toml_bytes: None,
                route_count: None,
                population_record_count: None,
                error: Some(error_json(&DiagnosticError::from_error(error))),
                ordinary_diagnostic_calls,
            }
        }
    }
}

fn observe_cost() -> CostObservation {
    reset_diagnostic_entry_count();
    let fixture = RouteFixture::new(RouteFixtureKind::Continuous);
    let network = fixture.network();
    let topology = fixture.normalized_topology(&network);
    let population = fixture.selected_population();
    build_routes_and_bind_population(&network, &topology, &population)
        .expect("ordinary build without diagnostics");
    let ordinary_diagnostic_calls_before = diagnostic_entry_count();
    let report = diagnose_population(&network, &topology, &population);
    CostObservation {
        ordinary_diagnostic_calls_before,
        diagnostic_calls_after: diagnostic_entry_count(),
        metrics: json!({
            "records": report.metrics.records,
            "expansionCalls": report.metrics.expansion_calls,
            "stateEvaluations": report.metrics.state_evaluations,
            "memoHits": report.metrics.memo_hits,
            "materializeCalls": report.metrics.materialize_calls,
            "elapsedNs": report.metrics.elapsed_ns,
        }),
    }
}

fn validate_cases(cases: &[CaseEvidence]) {
    assert_eq!(cases.len(), 14);
    let case = |name| cases.iter().find(|case| case.name == name).expect("case");
    assert_eq!(
        case("direct-gap").report["records"][0]["firstGap"]["kind"],
        json!("LaneContinuityGap")
    );
    assert_eq!(
        case("missing-hop").report["records"][0]["firstGap"]["kind"],
        json!("RoadHopMissing")
    );
    assert_eq!(
        case("branch-success").report["records"][0]["route"]["edgeIds"][0],
        json!("sumo:B_1")
    );
    assert_eq!(
        case("two-lane-origin").report["records"][0]["route"]["edgeIds"][0],
        json!("sumo:A_1")
    );
    assert_eq!(
        case("all-failed-start-lanes").report["records"][0]["firstGap"]["reachableLanes"],
        json!([0, 1])
    );
    assert_eq!(
        case("repeated-success").report["records"][0]["route"]["edgeIds"]
            .as_array()
            .expect("success chain")
            .len(),
        7
    );
    assert_eq!(
        case("repeated-gap").report["records"][0]["firstGap"]["fromOccurrence"],
        json!(3)
    );
    assert_eq!(
        case("repeated-gap").report["records"][0]["firstGap"]["reachableLanes"],
        json!([0])
    );
    assert_eq!(
        case("normalization-unknown-lane").report["records"][0]["population"],
        Value::Null
    );
    assert_eq!(
        case("empty-unclassified").report["records"][0]["kind"],
        json!("Unclassified")
    );
    assert_eq!(
        case("single-road").report["records"][0]["route"]["edgeIds"],
        json!(["sumo:B_0"])
    );
    assert_eq!(
        case("materialization-injection").report["records"][0]["injected"],
        json!(true)
    );
    let continuous = case("continuous-direct");
    assert_eq!(continuous.source_paths.len(), 3);
    assert_eq!(
        continuous.report["records"][0]["route"]["edgeIds"],
        json!(expected_final_chain())
    );
    let combined = continuous.combined.as_ref().expect("combined observation");
    assert!(combined.direct_route_succeeded);
    assert_eq!(combined.stage, "geometry-repair");
    assert_eq!(combined.outcome, "post-route-error");
    assert_eq!(combined.ordinary_diagnostic_calls, 0);
    assert!(combined.network_lfca_bytes.is_none());
    assert!(combined.routes_toml_bytes.is_none());
    assert!(
        combined.error.as_ref().expect("geometry error")["message"]
            .as_str()
            .expect("geometry message")
            .contains("repaired curve")
    );
    let cost = observe_cost();
    assert_eq!(cost.ordinary_diagnostic_calls_before, 0);
    assert_eq!(cost.diagnostic_calls_after, 1);
    assert_eq!(cost.metrics["expansionCalls"], json!(2));
    assert_eq!(cost.metrics["materializeCalls"], json!(2));
    for case in cases {
        assert!(case.not_executed.contains(&"runtime-route-registration"));
    }
}

fn compare_a0(baseline: &Value, cases: &[CaseEvidence]) -> Vec<A0Comparison> {
    let case = |name| {
        cases
            .iter()
            .find(|case| case.name == name)
            .expect("A1 case")
    };
    let mut comparisons = Vec::new();
    let gap_fixture = RouteFixture::new(RouteFixtureKind::Gap);
    let gap_network = gap_fixture.network();
    let gap_topology = gap_fixture.normalized_topology(&gap_network);
    let continuous_fixture = RouteFixture::new(RouteFixtureKind::Continuous);
    let continuous_network = continuous_fixture.network();
    let continuous_topology = continuous_fixture.normalized_topology(&continuous_network);
    comparisons.push(A0Comparison {
        case: "fixture-self-check",
        equal: true,
        checks: vec![
            check("gapNetworkLaneCount", baseline_case(baseline, "fixture-self-check")["details"]["gapNetworkLaneCount"].clone(), json!(gap_network.lanes.len())),
            check("continuousNetworkLaneCount", baseline_case(baseline, "fixture-self-check")["details"]["continuousNetworkLaneCount"].clone(), json!(continuous_network.lanes.len())),
            check("gapConnectionCount", baseline_case(baseline, "fixture-self-check")["details"]["gapConnectionCount"].clone(), json!(gap_network.connections.len())),
            check("continuousConnectionCount", baseline_case(baseline, "fixture-self-check")["details"]["continuousConnectionCount"].clone(), json!(continuous_network.connections.len())),
            check("gapBLaneIndices", baseline_case(baseline, "fixture-self-check")["details"]["gapBLaneIndices"].clone(), json!(lane_indices(&gap_network))),
            check("continuousBLaneIndices", baseline_case(baseline, "fixture-self-check")["details"]["continuousBLaneIndices"].clone(), json!(lane_indices(&continuous_network))),
            check("noB0B1Connection", baseline_case(baseline, "fixture-self-check")["details"]["noB0B1Connection"].clone(), json!(!gap_network.connections.iter().any(|connection| connection.from_edge_id == "B" && connection.to_edge_id == "B" && connection.from_lane == 0 && connection.to_lane == 1))),
            check("noB0ToCPath", baseline_case(baseline, "fixture-self-check")["details"]["noB0ToCPath"].clone(), json!(!has_lane_hop(&gap_topology))),
            check("noStubChanges", baseline_case(baseline, "fixture-self-check")["details"]["noStubChanges"].clone(), json!(no_stub_changes(&gap_topology) && no_stub_changes(&continuous_topology))),
            check("sourcePaths", baseline_case(baseline, "fixture-self-check")["source_paths"].clone(), json!(normalized_paths(&gap_topology))),
            check("continuousPaths", baseline_case(baseline, "fixture-self-check")["details"]["continuousPaths"].clone(), json!(normalized_paths(&continuous_topology))),
        ],
    });
    comparisons.push(A0Comparison {
        case: "direct-gap",
        equal: true,
        checks: vec![
            check(
                "error",
                baseline_case(baseline, "direct-gap")["error"].clone(),
                case("direct-gap").report["records"][0]["originalError"].clone(),
            ),
            check(
                "sourcePaths",
                baseline_case(baseline, "direct-gap")["source_paths"].clone(),
                json!(case("direct-gap").source_paths),
            ),
            check(
                "population",
                baseline_case(baseline, "direct-gap")["population"].clone(),
                report_population(&case("direct-gap").report),
            ),
        ],
    });
    let fail_fast_error = gap_fixture
        .combined(&TopologyConvertOptions::default())
        .expect_err("A0 fail-fast must retain its exact error");
    comparisons.push(A0Comparison {
        case: "fail-fast-gap",
        equal: true,
        checks: vec![
            check(
                "error",
                baseline_case(baseline, "fail-fast-gap")["error"].clone(),
                error_json(&DiagnosticError::from_error(fail_fast_error)),
            ),
            check(
                "population",
                baseline_case(baseline, "fail-fast-gap")["population"].clone(),
                json!(
                    gap_fixture
                        .selected_population()
                        .iter()
                        .map(population_json)
                        .collect::<Vec<_>>()
                ),
            ),
        ],
    });
    let diagnostic = gap_fixture
        .combined(&diagnostic_options())
        .expect("A0 diagnostic conversion");
    let diagnostic_repeat = gap_fixture
        .combined(&diagnostic_options())
        .expect("A0 diagnostic repeat");
    comparisons.push(A0Comparison {
        case: "diagnostic-gap",
        equal: true,
        checks: vec![
            check(
                "population",
                baseline_case(baseline, "diagnostic-gap")["population"].clone(),
                json!(
                    gap_fixture
                        .selected_population()
                        .iter()
                        .map(population_json)
                        .collect::<Vec<_>>()
                ),
            ),
            check(
                "populationOrdinals",
                baseline_case(baseline, "diagnostic-gap")["population_ordinals"].clone(),
                ordinal_json(&diagnostic.population_ordinals),
            ),
            check(
                "routesTomlNone",
                baseline_case(baseline, "diagnostic-gap")["details"]["routesTomlNone"].clone(),
                json!(diagnostic.routes_toml.is_none()),
            ),
            check(
                "repeatEqual",
                baseline_case(baseline, "diagnostic-gap")["details"]["repeatEqual"].clone(),
                json!(diagnostic == diagnostic_repeat),
            ),
            check(
                "populationRecordCount",
                baseline_case(baseline, "diagnostic-gap")["details"]["populationRecordCount"]
                    .clone(),
                json!(diagnostic.population_record_count),
            ),
            check(
                "infeasibilityReportPresent",
                baseline_case(baseline, "diagnostic-gap")["details"]["infeasibilityReportPresent"]
                    .clone(),
                json!(diagnostic.topology.infeasibility_report.is_some()),
            ),
            check(
                "networkLfcaEmpty",
                baseline_case(baseline, "diagnostic-gap")["details"]["networkLfcaEmpty"].clone(),
                json!(diagnostic.topology.network_lfca.is_empty()),
            ),
        ],
    });
    let continuous = case("continuous-direct");
    let repeat = diagnose_population(
        &continuous_network,
        &continuous_topology,
        &continuous_fixture.selected_population(),
    );
    let repeat_json = diagnostic_json(&repeat);
    comparisons.push(A0Comparison {
        case: "continuous-direct",
        equal: true,
        checks: vec![
            check(
                "finalEdgeChain",
                baseline_case(baseline, "continuous-direct")["final_edge_chain"].clone(),
                continuous.report["records"][0]["route"]["edgeIds"].clone(),
            ),
            check(
                "boundPopulation",
                baseline_case(baseline, "continuous-direct")["bound_population"].clone(),
                bound_population(&continuous.report),
            ),
            check(
                "routeCatalogIds",
                baseline_case(baseline, "continuous-direct")["details"]["routeCatalogIds"].clone(),
                catalog_ids(&continuous.report),
            ),
            check(
                "repeatCatalogEqual",
                baseline_case(baseline, "continuous-direct")["details"]["repeatCatalogEqual"]
                    .clone(),
                json!(catalog_ids(&continuous.report) == catalog_ids(&repeat_json)),
            ),
            check(
                "repeatRecordsEqual",
                baseline_case(baseline, "continuous-direct")["details"]["repeatRecordsEqual"]
                    .clone(),
                json!(bound_population(&continuous.report) == bound_population(&repeat_json)),
            ),
            check(
                "sourcePaths",
                baseline_case(baseline, "continuous-direct")["source_paths"].clone(),
                json!(continuous.source_paths),
            ),
        ],
    });
    comparisons.push(A0Comparison {
        case: "unknown-lane-normalization",
        equal: true,
        checks: vec![
            check(
                "error",
                baseline_case(baseline, "unknown-lane-normalization")["error"].clone(),
                case("normalization-unknown-lane").report["records"][0]["originalError"].clone(),
            ),
            check(
                "sourcePaths",
                baseline_case(baseline, "unknown-lane-normalization")["source_paths"].clone(),
                json!(case("normalization-unknown-lane").source_paths),
            ),
        ],
    });
    for (baseline_name, current_name) in [
        ("unknown-road-direct", "unknown-road-direct"),
        ("internal-road-direct", "internal-road-direct"),
    ] {
        comparisons.push(A0Comparison {
            case: baseline_name,
            equal: true,
            checks: vec![
                check(
                    "error",
                    baseline_case(baseline, baseline_name)["error"].clone(),
                    case(current_name).report["records"][0]["originalError"].clone(),
                ),
                check(
                    "population",
                    baseline_case(baseline, baseline_name)["population"].clone(),
                    report_population(&case(current_name).report),
                ),
                check(
                    "sourcePaths",
                    baseline_case(baseline, baseline_name)["source_paths"].clone(),
                    json!(case(current_name).source_paths),
                ),
            ],
        });
    }
    for (name, roads) in [
        ("unknown-road-diagnostic", &["A", "B", "GHOST"][..]),
        ("internal-road-diagnostic", &["A", "B", ":J2_0"][..]),
    ] {
        let (error, population) = diagnostic_override(roads);
        comparisons.push(A0Comparison {
            case: name,
            equal: true,
            checks: vec![
                check(
                    "error",
                    baseline_case(baseline, name)["error"].clone(),
                    error_json(&error),
                ),
                check(
                    "population",
                    baseline_case(baseline, name)["population"].clone(),
                    population,
                ),
            ],
        });
    }
    let missing = case("missing-hop");
    comparisons.push(A0Comparison {
        case: "missing-hop-direct",
        equal: true,
        checks: vec![
            check(
                "error",
                baseline_case(baseline, "missing-hop-direct")["error"].clone(),
                missing.report["records"][0]["originalError"].clone(),
            ),
            check(
                "population",
                baseline_case(baseline, "missing-hop-direct")["population"].clone(),
                report_population(&missing.report),
            ),
            check(
                "hopPresent",
                baseline_case(baseline, "missing-hop-direct")["details"]["hopPresent"].clone(),
                json!(has_road_hop(&missing.report)),
            ),
            check(
                "sameErrorAsGap",
                baseline_case(baseline, "missing-hop-direct")["details"]["sameErrorAsGap"].clone(),
                json!(
                    missing.report["formal"]["error"]
                        == case("direct-gap").report["formal"]["error"]
                ),
            ),
        ],
    });
    for comparison in &mut comparisons {
        comparison.equal = comparison.checks.iter().all(|check| check.equal);
    }
    comparisons
}

fn baseline_case<'a>(baseline: &'a Value, name: &str) -> &'a Value {
    baseline["cases"]
        .as_array()
        .expect("A0 cases")
        .iter()
        .find(|case| case["name"] == name)
        .unwrap_or_else(|| panic!("missing A0 case {name}"))
}

fn check(name: &'static str, baseline: Value, current: Value) -> ComparisonCheck {
    let equal = baseline == current;
    ComparisonCheck {
        name,
        equal,
        baseline,
        current,
    }
}

fn report_population(report: &Value) -> Value {
    json!(
        report["records"]
            .as_array()
            .expect("diagnostic records")
            .iter()
            .map(|record| record["population"].clone())
            .collect::<Vec<_>>()
    )
}

fn bound_population(report: &Value) -> Value {
    json!(
        report["records"]
            .as_array()
            .expect("diagnostic records")
            .iter()
            .map(|record| record["route"]["bound"].clone())
            .collect::<Vec<_>>()
    )
}

fn catalog_ids(report: &Value) -> Value {
    json!(
        report["formal"]["routes"]
            .as_array()
            .expect("formal routes")
            .iter()
            .map(|route| route["id"].clone())
            .collect::<Vec<_>>()
    )
}

fn ordinal_json(ordinals: &[PopulationOrdinal]) -> Value {
    json!(
        ordinals
            .iter()
            .map(|ordinal| json!({
                "populationRank": ordinal.population_rank,
                "sourceFileOrdinal": ordinal.source_file_ordinal,
                "sourceVehicleOrdinal": ordinal.source_vehicle_ordinal,
            }))
            .collect::<Vec<_>>()
    )
}

fn lane_indices(network: &SumoNetwork) -> Vec<u32> {
    let mut lanes = network
        .lanes
        .iter()
        .filter(|lane| lane.edge_id == "B" && !lane.function_internal)
        .map(|lane| lane.index)
        .collect::<Vec<_>>();
    lanes.sort_unstable();
    lanes
}

fn has_lane_hop(topology: &NormalizedTopology) -> bool {
    topology
        .path_by_connection
        .keys()
        .any(|(from, from_lane, to, _)| from == "B" && *from_lane == 0 && to == "C")
}

fn has_road_hop(report: &Value) -> bool {
    !report["records"][0]["firstGap"]["candidates"]
        .as_array()
        .expect("missing-hop candidates")
        .is_empty()
        || report["records"][0]["firstGap"]["kind"] != json!("RoadHopMissing")
}

fn no_stub_changes(topology: &NormalizedTopology) -> bool {
    topology.dropped_stub_lane_ids.is_empty()
        && topology.stub_welds.is_empty()
        && topology.stub_weld_records.is_empty()
}

fn diagnostic_override(roads: &[&str]) -> (DiagnosticError, Value) {
    let fixture = RouteFixture::new(RouteFixtureKind::Gap);
    let first_id = fixture.selected_population()[0].vehicle_id.clone();
    let fixture = fixture.with_route_override(&first_id, roads);
    let population = json!(
        fixture
            .selected_population()
            .iter()
            .map(population_json)
            .collect::<Vec<_>>()
    );
    let error = fixture
        .combined(&diagnostic_options())
        .expect_err("diagnostic route-edge validation");
    (DiagnosticError::from_error(error), population)
}

fn export_metadata() -> Value {
    let verbose = command_output("rustc", &["-vV"]);
    let host = verbose
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .unwrap_or("unknown");
    json!({
        "head": command_output("git", &["rev-parse", "HEAD"]),
        "branch": command_output("git", &["branch", "--show-current"]),
        "dirty": !command_output("git", &["status", "--porcelain"]).is_empty(),
        "rustc": command_output("rustc", &["--version"]),
        "host": host,
        "platform": format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        "cargoTargetDir": std::env::var("CARGO_TARGET_DIR").ok(),
    })
}

fn export_command(output: &std::path::Path, baseline: &std::path::Path) -> String {
    let target =
        std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "<worktree-target>".to_owned());
    format!(
        "CARGO_TARGET_DIR='{target}' LANEFLOW_806_A0_BASELINE='{}' LANEFLOW_806_A1_RESULTS='{}' cargo +1.99.1 test --locked -p laneflow-lust-converter --lib convert::routes::diagnostics::tests::evidence::a1_route_evidence -- --exact --nocapture",
        baseline.display(),
        output.display()
    )
}

fn command_output(program: &str, args: &[&str]) -> String {
    let output = Command::new(program)
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap_or_else(|error| panic!("run {program}: {error}"));
    assert!(output.status.success(), "{program} failed: {output:?}");
    String::from_utf8(output.stdout)
        .expect("command output is UTF-8")
        .trim()
        .to_owned()
}
