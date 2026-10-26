//! #806-A0 的真实双车道夹具与生产入口观察辅助。

use serde::Serialize;

use crate::{
    Error,
    convert::{
        junction::{NormalizedTopology, StubWeldGate, normalize_junctions},
        population::{PopulationOrdinal, PopulationRecord, select_population},
        profiles::{convert_vehicle_profiles, select_passenger_vtypes},
        topology::{StaticConversionArtifacts, TopologyConvertOptions, convert_static_with_due},
    },
    output::{digest::sha256_digest, geom::ReportSource},
    sumo::{
        DueVehicle, SumoNetwork, parse_due_routes_xml_filtered, parse_sumo_network_xml,
        parse_vtypes_xml,
    },
};

pub(crate) const NO_COMPLETE_ROUTE_ERROR: &str =
    "no complete lane-level path for road route [\"A\", \"B\", \"C\"]";
const VTYPES: &str = include_str!("../../../tests/fixtures/minimal/vtypes.add.xml");

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ErrorEvidence {
    pub variant: &'static str,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct PathEvidence {
    pub connection: String,
    pub path_id: String,
    pub entry: String,
    pub internals: Vec<String>,
    pub exit: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RouteFixtureKind {
    Gap,
    Continuous,
    MissingHop,
    UnknownLane,
    TwoLaneOrigin,
    LoopBack,
}

#[derive(Clone, Debug)]
pub(crate) struct RouteFixture {
    pub kind: RouteFixtureKind,
    vehicles: Vec<VehicleSpec>,
}

#[derive(Clone, Debug)]
struct VehicleSpec {
    id: &'static str,
    type_id: &'static str,
    depart: &'static str,
    source_file: u8,
    road_edge_ids: Vec<String>,
}

impl RouteFixture {
    pub(crate) fn new(kind: RouteFixtureKind) -> Self {
        let roads = || vec!["A".to_owned(), "B".to_owned(), "C".to_owned()];
        let vehicles = vec![
            VehicleSpec {
                id: "outside-window",
                type_id: "passenger1",
                depart: "28799",
                source_file: 0,
                road_edge_ids: roads(),
            },
            VehicleSpec {
                id: "selected-z",
                type_id: "passenger1",
                depart: "28800",
                source_file: 0,
                road_edge_ids: roads(),
            },
            VehicleSpec {
                id: "selected-a",
                type_id: "passenger1",
                depart: "28801",
                source_file: 1,
                road_edge_ids: roads(),
            },
            VehicleSpec {
                id: "after-window",
                type_id: "passenger1",
                depart: "30600",
                source_file: 2,
                road_edge_ids: roads(),
            },
        ];
        Self { kind, vehicles }
    }

    pub(crate) fn with_route_override(mut self, vehicle_id: &str, roads: &[&str]) -> Self {
        let vehicle = self
            .vehicles
            .iter_mut()
            .find(|vehicle| vehicle.id == vehicle_id)
            .expect("route override target exists");
        vehicle.road_edge_ids = roads.iter().map(|road| (*road).to_owned()).collect();
        self
    }

    pub(crate) fn with_road_sequence(mut self, roads: &[&str]) -> Self {
        let roads = roads
            .iter()
            .map(|road| (*road).to_owned())
            .collect::<Vec<_>>();
        for vehicle in &mut self.vehicles {
            vehicle.road_edge_ids.clone_from(&roads);
        }
        self
    }

    pub(crate) fn network(&self) -> SumoNetwork {
        parse_sumo_network_xml(&self.net_xml()).expect("parse A0 network")
    }

    pub(crate) fn normalized_topology(&self, network: &SumoNetwork) -> NormalizedTopology {
        normalize_junctions(network, &StubWeldGate::Unrestricted).expect("normalize A0 network")
    }

    pub(crate) fn selected_population(&self) -> Vec<PopulationRecord> {
        select_population(&self.due_vehicles(false), false).expect("select A0 population")
    }

    pub(crate) fn combined(
        &self,
        options: &TopologyConvertOptions,
    ) -> crate::Result<StaticConversionArtifacts> {
        let network = self.network();
        let vtypes = parse_vtypes_xml(VTYPES).expect("parse A0 vtypes");
        let passengers = select_passenger_vtypes(&vtypes).expect("select A0 profiles");
        let profiles = convert_vehicle_profiles(&passengers).expect("convert A0 profiles");
        let vehicles = self.due_vehicles(true);
        convert_static_with_due(
            &network,
            &[],
            &profiles,
            &vehicles,
            options,
            ReportSource::xml_unverified(sha256_digest(self.net_xml().as_bytes())),
        )
    }

    fn due_vehicles(&self, candidates_only: bool) -> Vec<DueVehicle> {
        let mut vehicles = Vec::new();
        for (index, document) in self.due_documents().iter().enumerate() {
            let source_file = u8::try_from(index).expect("DUE file ordinal fits u8");
            if candidates_only {
                vehicles.extend(
                    parse_due_routes_xml_filtered(
                        document,
                        source_file,
                        crate::convert::population::is_population_candidate,
                    )
                    .expect("parse A0 DUE candidates"),
                );
            } else {
                vehicles.extend(
                    parse_due_routes_xml_filtered(document, source_file, |_, _| true)
                        .expect("parse A0 DUE"),
                );
            }
        }
        vehicles
    }

    fn net_xml(&self) -> String {
        match self.kind {
            RouteFixtureKind::TwoLaneOrigin => return two_lane_origin_net_xml(),
            RouteFixtureKind::LoopBack => return loop_back_net_xml(),
            _ => {}
        }
        let include_p2 = !matches!(self.kind, RouteFixtureKind::MissingHop);
        let include_p0 = matches!(self.kind, RouteFixtureKind::Continuous);
        let p2_from_lane = if matches!(self.kind, RouteFixtureKind::UnknownLane) {
            9
        } else {
            1
        };
        let i2_edge = if include_p2 {
            r#"  <edge id=":J2_0" function="internal"><lane id=":J2_0_0" index="0" speed="13.89" length="5.00" shape="6836.88,5730.52 6840.88,5727.52"/></edge>"#
        } else {
            ""
        };
        let i0_edge = if include_p0 {
            r#"  <edge id=":J2_1" function="internal"><lane id=":J2_1_0" index="0" speed="13.89" length="4.00" shape="6836.88,5727.52 6840.88,5727.52"/></edge>"#
        } else {
            ""
        };
        let j2_int_lanes = match (include_p2, include_p0) {
            (true, true) => ":J2_0_0 :J2_1_0",
            (true, false) => ":J2_0_0",
            (false, false) => "",
            (false, true) => unreachable!("continuous fixture always retains P2"),
        };
        let p2_connections = if include_p2 {
            format!(
                r#"  <connection from="B" to="C" fromLane="{p2_from_lane}" toLane="0" via=":J2_0_0"/>
  <connection from=":J2_0" to="C" fromLane="0" toLane="0"/>"#
            )
        } else {
            String::new()
        };
        let p0_connections = if include_p0 {
            r#"  <connection from="B" to="C" fromLane="0" toLane="0" via=":J2_1_0"/>
  <connection from=":J2_1" to="C" fromLane="0" toLane="0"/>"#
                .to_owned()
        } else {
            String::new()
        };
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<net version="1.16">
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="A" from="X" to="J1"><lane id="A_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="B" from="J1" to="J2"><lane id="B_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/><lane id="B_1" index="1" speed="13.89" length="20.00" shape="6816.88,5730.52 6836.88,5730.52"/></edge>
  <edge id="C" from="J2" to="Z"><lane id="C_0" index="0" speed="13.89" length="20.00" shape="6840.88,5727.52 6860.88,5727.52"/></edge>
  <edge id=":J1_0" function="internal"><lane id=":J1_0_0" index="0" speed="13.89" length="10.00" shape="6806.88,5727.52 6816.88,5727.52"/></edge>
{i2_edge}
{i0_edge}
  <junction id="X" type="dead_end" incLanes="" intLanes=""/>
  <junction id="Z" type="dead_end" incLanes="C_0" intLanes=""/>
  <junction id="J1" type="priority" incLanes="A_0" intLanes=":J1_0_0"/>
  <junction id="J2" type="priority" incLanes="B_0 B_1" intLanes="{j2_int_lanes}"/>
  <connection from="A" to="B" fromLane="0" toLane="0" via=":J1_0_0"/>
  <connection from=":J1_0" to="B" fromLane="0" toLane="0"/>
{p2_connections}
{p0_connections}
</net>
"#
        )
    }

    fn due_documents(&self) -> [String; 3] {
        let mut documents = [
            "<routes>\n".to_owned(),
            "<routes>\n".to_owned(),
            "<routes>\n".to_owned(),
        ];
        for vehicle in &self.vehicles {
            let document = &mut documents[usize::from(vehicle.source_file)];
            document.push_str(&format!(
                "  <vehicle id=\"{}\" type=\"{}\" depart=\"{}\" departPos=\"random\">\n    <route edges=\"{}\"/>\n  </vehicle>\n",
                vehicle.id,
                vehicle.type_id,
                vehicle.depart,
                vehicle.road_edge_ids.join(" ")
            ));
        }
        for document in &mut documents {
            document.push_str("</routes>\n");
        }
        documents
    }
}

fn two_lane_origin_net_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<net version="1.16">
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="A" from="X" to="J1"><lane id="A_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/><lane id="A_1" index="1" speed="13.89" length="20.00" shape="6786.88,5730.52 6806.88,5730.52"/></edge>
  <edge id="B" from="J1" to="J2"><lane id="B_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/><lane id="B_1" index="1" speed="13.89" length="20.00" shape="6816.88,5730.52 6836.88,5730.52"/></edge>
  <edge id="C" from="J2" to="Z"><lane id="C_0" index="0" speed="13.89" length="20.00" shape="6840.88,5727.52 6860.88,5727.52"/></edge>
  <edge id=":J1_0" function="internal"><lane id=":J1_0_0" index="0" speed="13.89" length="10.00" shape="6806.88,5727.52 6816.88,5727.52"/></edge>
  <edge id=":J1_1" function="internal"><lane id=":J1_1_0" index="0" speed="13.89" length="10.00" shape="6806.88,5730.52 6816.88,5730.52"/></edge>
  <edge id=":J2_0" function="internal"><lane id=":J2_0_0" index="0" speed="13.89" length="5.00" shape="6836.88,5730.52 6840.88,5727.52"/></edge>
  <junction id="X" type="dead_end" incLanes="" intLanes=""/>
  <junction id="Z" type="dead_end" incLanes="C_0" intLanes=""/>
  <junction id="J1" type="priority" incLanes="A_0 A_1" intLanes=":J1_0_0 :J1_1_0"/>
  <junction id="J2" type="priority" incLanes="B_0 B_1" intLanes=":J2_0_0"/>
  <connection from="A" to="B" fromLane="0" toLane="0" via=":J1_0_0"/>
  <connection from=":J1_0" to="B" fromLane="0" toLane="0"/>
  <connection from="A" to="B" fromLane="1" toLane="1" via=":J1_1_0"/>
  <connection from=":J1_1" to="B" fromLane="0" toLane="1"/>
  <connection from="B" to="C" fromLane="1" toLane="0" via=":J2_0_0"/>
  <connection from=":J2_0" to="C" fromLane="0" toLane="0"/>
</net>
"#
    .to_owned()
}

fn loop_back_net_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<net version="1.16">
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="A" from="X" to="J1"><lane id="A_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="B" from="J1" to="J2"><lane id="B_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/><lane id="B_1" index="1" speed="13.89" length="20.00" shape="6816.88,5730.52 6836.88,5730.52"/></edge>
  <edge id="C" from="J2" to="J1"><lane id="C_0" index="0" speed="13.89" length="40.00" shape="6846.88,5720.52 6806.88,5720.52"/></edge>
  <edge id=":J1_0" function="internal"><lane id=":J1_0_0" index="0" speed="13.89" length="13.00" shape="6806.88,5727.52 6806.88,5730.52 6816.88,5730.52"/></edge>
  <edge id=":J2_0" function="internal"><lane id=":J2_0_0" index="0" speed="13.89" length="20.00" shape="6836.88,5730.52 6846.88,5730.52 6846.88,5720.52"/></edge>
  <edge id=":J1_1" function="internal"><lane id=":J1_1_0" index="0" speed="13.89" length="17.00" shape="6806.88,5720.52 6816.88,5720.52 6816.88,5727.52"/></edge>
  <junction id="X" type="dead_end" incLanes="" intLanes=""/>
  <junction id="J1" type="priority" incLanes="A_0 C_0" intLanes=":J1_0_0 :J1_1_0"/>
  <junction id="J2" type="priority" incLanes="B_0 B_1" intLanes=":J2_0_0"/>
  <connection from="A" to="B" fromLane="0" toLane="1" via=":J1_0_0"/>
  <connection from=":J1_0" to="B" fromLane="0" toLane="1"/>
  <connection from="B" to="C" fromLane="1" toLane="0" via=":J2_0_0"/>
  <connection from=":J2_0" to="C" fromLane="0" toLane="0"/>
  <connection from="C" to="B" fromLane="0" toLane="0" via=":J1_1_0"/>
  <connection from=":J1_1" to="B" fromLane="0" toLane="0"/>
</net>
"#
    .to_owned()
}

pub(crate) fn diagnostic_options() -> TopologyConvertOptions {
    TopologyConvertOptions {
        emit_infeasibility_report: true,
        ..TopologyConvertOptions::default()
    }
}

pub(crate) fn population_record(vehicle_id: &str, roads: &[&str]) -> PopulationRecord {
    PopulationRecord {
        population_rank: 0,
        vehicle_id: vehicle_id.to_owned(),
        type_id: "passenger1".to_owned(),
        depart: "28800".parse().expect("valid A0 departure"),
        road_edge_ids: roads.iter().map(|road| (*road).to_owned()).collect(),
        source_file_ordinal: 0,
        source_vehicle_ordinal: 0,
    }
}

pub(crate) fn expected_ordinals(population: &[PopulationRecord]) -> Vec<PopulationOrdinal> {
    population
        .iter()
        .map(|record| PopulationOrdinal {
            population_rank: record.population_rank,
            source_file_ordinal: record.source_file_ordinal,
            source_vehicle_ordinal: record.source_vehicle_ordinal,
        })
        .collect()
}

pub(crate) fn expected_final_chain() -> Vec<String> {
    [
        "sumo:A_0",
        "sumo::J1_0_0",
        "sumo:B_0",
        "sumo::J2_1_0",
        "sumo:C_0",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

pub(crate) fn error_evidence(error: Error) -> ErrorEvidence {
    match error {
        Error::SumoModel(message) => ErrorEvidence {
            variant: "SumoModel",
            message,
        },
        other => ErrorEvidence {
            variant: "other",
            message: other.to_string(),
        },
    }
}

pub(crate) fn path_evidence(
    topology: &NormalizedTopology,
    from: &str,
    from_lane: u32,
    to: &str,
    to_lane: u32,
) -> PathEvidence {
    let connection = format!("{from}/{from_lane}->{to}/{to_lane}");
    let path_id = topology
        .path_by_connection
        .get(&(from.to_owned(), from_lane, to.to_owned(), to_lane))
        .unwrap_or_else(|| panic!("missing ManeuverPath for {connection}"))
        .clone();
    let path = topology
        .maneuver_paths
        .iter()
        .find(|path| path.id == path_id)
        .expect("path_by_connection resolves an emitted path");
    PathEvidence {
        connection,
        path_id,
        entry: path.entry_edge_id.clone(),
        internals: path.internal_edge_ids.clone(),
        exit: path.exit_edge_id.clone(),
    }
}

#[cfg(test)]
mod evidence {
    use std::{collections::BTreeMap, fs, path::PathBuf, process::Command};

    use serde::Serialize;
    use serde_json::{Value, json};

    use super::*;
    use crate::convert::routes::{
        BoundPopulationRecord, RoutePopulationBundle, build_routes_and_bind_population,
    };

    const ROUTE_TESTS: &str = "convert::routes::tests";
    const ACCEPTANCE_TEST: &str =
        "convert::acceptance::diagnostic_mode_skips_route_expansion_but_keeps_population";
    const RUNTIME_STAGES: [&str; 3] = [
        "runtime-route-registration",
        "vehicle-placement-and-admission",
        "fixed-tick-execution",
    ];

    #[derive(Clone, Debug, Serialize)]
    struct CaseEvidence {
        name: &'static str,
        test: String,
        input: String,
        stage: &'static str,
        outcome: &'static str,
        source_paths: Vec<PathEvidence>,
        final_edge_chain: Vec<String>,
        population: Vec<Value>,
        bound_population: Vec<Value>,
        population_ordinals: Vec<Value>,
        error: Option<ErrorEvidence>,
        details: BTreeMap<&'static str, Value>,
        not_executed: Vec<&'static str>,
    }

    #[derive(Serialize)]
    struct EvidenceReport {
        code: Value,
        export_command: String,
        cases: Vec<CaseEvidence>,
        unexecuted: Vec<&'static str>,
    }

    #[test]
    fn a0_route_evidence() {
        let cases = observed_cases();
        validate_cases(&cases);
        let Some(output) = std::env::var_os("LANEFLOW_806_A0_RESULTS").map(PathBuf::from) else {
            return;
        };
        let code = export_metadata();
        let report = EvidenceReport {
            export_command: export_command(&output),
            code,
            cases,
            unexecuted: vec![
                "A1 internal diagnostic classification",
                "continuous combined static conversion",
                "Runtime route registration and vehicle admission",
                "fixed-tick and multi-vehicle safety",
                "ignored pinned full-LuST, migration, TOPO/DEMAND and simulation",
            ],
        };
        if let Some(parent) = output.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).expect("create A0 evidence directory");
        }
        let bytes = crate::output::json_bytes("806-A0 route evidence", &report)
            .expect("serialize A0 evidence");
        fs::write(&output, bytes).expect("write A0 results.json");
        println!(
            "A0_RESULTS={} CASES={} HEAD={}",
            output.display(),
            report.cases.len(),
            report.code["head"].as_str().expect("metadata head")
        );
    }

    fn observed_cases() -> Vec<CaseEvidence> {
        let gap = RouteFixture::new(RouteFixtureKind::Gap);
        let gap_network = gap.network();
        let gap_topology = gap.normalized_topology(&gap_network);
        let gap_population = gap.selected_population();
        let gap_error = error_evidence(
            build_routes_and_bind_population(&gap_network, &gap_topology, &gap_population)
                .expect_err("gap route expansion fails"),
        );

        let continuous = RouteFixture::new(RouteFixtureKind::Continuous);
        let continuous_network = continuous.network();
        let continuous_topology = continuous.normalized_topology(&continuous_network);
        let continuous_population = continuous.selected_population();
        let build_continuous = || {
            build_routes_and_bind_population(
                &continuous_network,
                &continuous_topology,
                &continuous_population,
            )
        };
        let continuous_first = build_continuous().expect("continuous route materializes");
        let continuous_second = build_continuous().expect("continuous route repeats");
        let continuous_catalog_equal = catalog(&continuous_first) == catalog(&continuous_second);
        let continuous_records_equal = continuous_first.records == continuous_second.records;

        let mut continuous_case = new_case(
            "continuous-direct",
            &format!(
                "{ROUTE_TESTS}::a0_continuous_path_materializes_and_binds_population_deterministically"
            ),
            "Continuous; two selected records; no signal controllers",
            "direct-route-materialization",
            &continuous_topology,
            &continuous_population,
            "combined static conversion (not required for the direct-route acceptance)",
        );
        continuous_case.final_edge_chain = continuous_first.routes[0].edge_ids.clone();
        continuous_case.bound_population = bound_json(&continuous_first.records);
        continuous_case.details.insert(
            "routeCatalogIds",
            json!(
                continuous_first
                    .routes
                    .iter()
                    .map(|route| route.id.clone())
                    .collect::<Vec<_>>()
            ),
        );
        continuous_case
            .details
            .insert("repeatCatalogEqual", json!(continuous_catalog_equal));
        continuous_case
            .details
            .insert("repeatRecordsEqual", json!(continuous_records_equal));

        let mut fixture_case = new_case(
            "fixture-self-check",
            &format!("{ROUTE_TESTS}::a0_fixture_proves_real_lane_gap_and_continuous_control"),
            "Gap and Continuous fixtures",
            "source-parse-and-normalization",
            &gap_topology,
            &gap_population,
            "combined static conversion (fixture self-check only)",
        );
        fixture_case
            .details
            .insert("gapNetworkLaneCount", json!(gap_network.lanes.len()));
        fixture_case.details.insert(
            "continuousNetworkLaneCount",
            json!(continuous_network.lanes.len()),
        );
        fixture_case
            .details
            .insert("gapConnectionCount", json!(gap_network.connections.len()));
        fixture_case.details.insert(
            "continuousConnectionCount",
            json!(continuous_network.connections.len()),
        );
        fixture_case
            .details
            .insert("gapBLaneIndices", json!(lane_indices(&gap_network)));
        fixture_case.details.insert(
            "continuousBLaneIndices",
            json!(lane_indices(&continuous_network)),
        );
        fixture_case.details.insert(
            "noB0B1Connection",
            json!(!gap_network.connections.iter().any(|connection| {
                connection.from_edge_id == "B"
                    && connection.to_edge_id == "B"
                    && connection.from_lane == 0
                    && connection.to_lane == 1
            })),
        );
        fixture_case
            .details
            .insert("noB0ToCPath", json!(!has_lane_hop(&gap_topology, 0)));
        fixture_case.details.insert(
            "noStubChanges",
            json!(
                gap_topology.dropped_stub_lane_ids.is_empty()
                    && gap_topology.stub_welds.is_empty()
                    && gap_topology.stub_weld_records.is_empty()
                    && continuous_topology.dropped_stub_lane_ids.is_empty()
                    && continuous_topology.stub_welds.is_empty()
                    && continuous_topology.stub_weld_records.is_empty()
            ),
        );
        fixture_case.details.insert(
            "continuousPaths",
            json!(continuous_case.source_paths.clone()),
        );

        let direct_case = with_evidence(
            new_case(
                "direct-gap",
                &format!("{ROUTE_TESTS}::a0_lane_gap_fails_at_route_expansion_with_exact_error"),
                "Gap; roads=[A,B,C]",
                "direct-route-expansion",
                &gap_topology,
                &gap_population,
                "combined static conversion after rejected route expansion",
            ),
            gap_error.clone(),
        );

        let fail_fast_error = gap
            .combined(&TopologyConvertOptions::default())
            .expect_err("combined fail-fast route error");
        let fail_fast_case = with_error(
            new_case(
                "fail-fast-gap",
                ACCEPTANCE_TEST,
                "Gap; combined fail-fast entry",
                "combined-fail-fast",
                &gap_topology,
                &gap_population,
                "combined static conversion after rejected route expansion",
            ),
            fail_fast_error,
        );

        let diagnostic = gap
            .combined(&diagnostic_options())
            .expect("diagnostic conversion succeeds");
        let diagnostic_repeat = gap
            .combined(&diagnostic_options())
            .expect("diagnostic conversion repeats");
        let diagnostic_repeat_equal = diagnostic == diagnostic_repeat;
        let mut diagnostic_case = new_case(
            "diagnostic-gap",
            ACCEPTANCE_TEST,
            "Gap; combined diagnostic entry",
            "combined-diagnostic",
            &gap_topology,
            &gap_population,
            "network.lfca build (diagnostic mode)",
        );
        diagnostic_case.population_ordinals = ordinal_json(&diagnostic.population_ordinals);
        diagnostic_case.details.insert(
            "populationRecordCount",
            json!(diagnostic.population_record_count),
        );
        diagnostic_case
            .details
            .insert("routesTomlNone", json!(diagnostic.routes_toml.is_none()));
        diagnostic_case.details.insert(
            "infeasibilityReportPresent",
            json!(diagnostic.topology.infeasibility_report.is_some()),
        );
        diagnostic_case.details.insert(
            "networkLfcaEmpty",
            json!(diagnostic.topology.network_lfca.is_empty()),
        );
        diagnostic_case
            .details
            .insert("repeatEqual", json!(diagnostic_repeat_equal));

        let unknown_lane = RouteFixture::new(RouteFixtureKind::UnknownLane);
        let unknown_lane_network = unknown_lane.network();
        let unknown_lane_population = unknown_lane.selected_population();
        let unknown_lane_error = normalize_junctions(
            &unknown_lane_network,
            &crate::convert::junction::StubWeldGate::Unrestricted,
        )
        .expect_err("unknown lane fails normalization");
        let unknown_lane_case = with_error(
            empty_case(
                "unknown-lane-normalization",
                &format!("{ROUTE_TESTS}::a0_source_reference_failures_are_stage_specific"),
                "UnknownLane; B connection fromLane=9",
                "normalization",
                &unknown_lane_population,
                "route expansion and combined conversion after rejected normalization",
            ),
            unknown_lane_error,
        );

        let unknown_road = population_record("unknown-road", &["A", "B", "GHOST"]);
        let unknown_road_error = build_routes_and_bind_population(
            &gap_network,
            &gap_topology,
            std::slice::from_ref(&unknown_road),
        )
        .expect_err("unknown road fails route-edge validation");
        let unknown_road_case = with_error(
            new_case(
                "unknown-road-direct",
                &format!("{ROUTE_TESTS}::a0_source_reference_failures_are_stage_specific"),
                "direct population road=[A,B,GHOST]",
                "route-edge-validation",
                &gap_topology,
                std::slice::from_ref(&unknown_road),
                "route expansion and combined conversion after rejected route-edge validation",
            ),
            unknown_road_error,
        );

        let internal_road = population_record("internal-road", &["A", "B", ":J2_0"]);
        let internal_road_error = build_routes_and_bind_population(
            &gap_network,
            &gap_topology,
            std::slice::from_ref(&internal_road),
        )
        .expect_err("internal road fails route-edge validation");
        let internal_road_case = with_error(
            new_case(
                "internal-road-direct",
                &format!("{ROUTE_TESTS}::a0_source_reference_failures_are_stage_specific"),
                "direct population road=[A,B,:J2_0]",
                "route-edge-validation",
                &gap_topology,
                std::slice::from_ref(&internal_road),
                "route expansion and combined conversion after rejected route-edge validation",
            ),
            internal_road_error,
        );

        let reject_in_diagnostic = |roads: &[&str], name: &'static str| {
            let fixture = RouteFixture::new(RouteFixtureKind::Gap);
            let first_id = fixture.selected_population()[0].vehicle_id.clone();
            let fixture = fixture.with_route_override(&first_id, roads);
            let network = fixture.network();
            let topology = fixture.normalized_topology(&network);
            let population = fixture.selected_population();
            let error = fixture
                .combined(&diagnostic_options())
                .expect_err("diagnostic route-edge validation fails");
            with_error(
                new_case(
                    name,
                    &format!("{ROUTE_TESTS}::a0_source_reference_failures_are_stage_specific"),
                    format!("diagnostic first selected road=[{}]", roads.join(",")),
                    "diagnostic-route-edge-validation",
                    &topology,
                    &population,
                    "route expansion and static conversion after rejected route-edge validation",
                ),
                error,
            )
        };
        let unknown_road_diagnostic_case =
            reject_in_diagnostic(&["A", "B", "GHOST"], "unknown-road-diagnostic");
        let internal_road_diagnostic_case =
            reject_in_diagnostic(&["A", "B", ":J2_0"], "internal-road-diagnostic");

        let missing_hop = RouteFixture::new(RouteFixtureKind::MissingHop);
        let missing_hop_network = missing_hop.network();
        let missing_hop_topology = missing_hop.normalized_topology(&missing_hop_network);
        let missing_hop_population = missing_hop.selected_population();
        let missing_hop_error = error_evidence(
            build_routes_and_bind_population(
                &missing_hop_network,
                &missing_hop_topology,
                &missing_hop_population,
            )
            .expect_err("missing road hop fails route expansion"),
        );
        let mut missing_hop_case = with_evidence(
            new_case(
                "missing-hop-direct",
                &format!("{ROUTE_TESTS}::a0_missing_road_hop_shares_current_lane_gap_error"),
                "MissingHop; no B to C source connection",
                "direct-route-expansion",
                &missing_hop_topology,
                &missing_hop_population,
                "combined static conversion after rejected route expansion",
            ),
            missing_hop_error.clone(),
        );
        missing_hop_case
            .details
            .insert("hopPresent", json!(has_road_hop(&missing_hop_topology)));
        missing_hop_case
            .details
            .insert("sameErrorAsGap", json!(missing_hop_error == gap_error));

        vec![
            fixture_case,
            direct_case,
            fail_fast_case,
            diagnostic_case,
            continuous_case,
            unknown_lane_case,
            unknown_road_case,
            unknown_road_diagnostic_case,
            internal_road_case,
            internal_road_diagnostic_case,
            missing_hop_case,
        ]
    }

    fn empty_case(
        name: &'static str,
        test: &str,
        input: impl Into<String>,
        stage: &'static str,
        population: &[PopulationRecord],
        static_not_executed: &'static str,
    ) -> CaseEvidence {
        CaseEvidence {
            name,
            test: test.to_owned(),
            input: input.into(),
            stage,
            outcome: "returned",
            source_paths: Vec::new(),
            final_edge_chain: Vec::new(),
            population: population_json(population),
            bound_population: Vec::new(),
            population_ordinals: Vec::new(),
            error: None,
            details: BTreeMap::new(),
            not_executed: [static_not_executed]
                .into_iter()
                .chain(RUNTIME_STAGES)
                .collect(),
        }
    }

    fn new_case(
        name: &'static str,
        test: &str,
        input: impl Into<String>,
        stage: &'static str,
        topology: &NormalizedTopology,
        population: &[PopulationRecord],
        static_not_executed: &'static str,
    ) -> CaseEvidence {
        let mut case = empty_case(name, test, input, stage, population, static_not_executed);
        let mut connections = topology.path_by_connection.keys().collect::<Vec<_>>();
        connections.sort();
        case.source_paths = connections
            .into_iter()
            .map(|(from, from_lane, to, to_lane)| {
                path_evidence(topology, from, *from_lane, to, *to_lane)
            })
            .collect();
        case
    }

    fn with_error(mut case: CaseEvidence, error: Error) -> CaseEvidence {
        case.outcome = "rejected";
        case.error = Some(error_evidence(error));
        case
    }

    fn with_evidence(mut case: CaseEvidence, error: ErrorEvidence) -> CaseEvidence {
        case.outcome = "rejected";
        case.error = Some(error);
        case
    }

    fn population_json(population: &[PopulationRecord]) -> Vec<Value> {
        population
            .iter()
            .map(|record| {
                json!({
                    "populationRank": record.population_rank,
                    "vehicleId": record.vehicle_id,
                    "vehicleTypeId": record.type_id,
                    "depart": record.depart.to_string(),
                    "roadEdgeIds": record.road_edge_ids,
                    "sourceFileOrdinal": record.source_file_ordinal,
                    "sourceVehicleOrdinal": record.source_vehicle_ordinal,
                })
            })
            .collect()
    }

    fn bound_json(records: &[BoundPopulationRecord]) -> Vec<Value> {
        records
            .iter()
            .map(|record| {
                json!({
                    "populationRank": record.population_rank,
                    "vehicleId": record.vehicle_id,
                    "vehicleProfileId": record.vehicle_profile_id,
                    "depart": record.depart.to_string(),
                    "routeId": record.route_id,
                    "roadEdgeIds": record.road_edge_ids,
                    "sourceFileOrdinal": record.source_file_ordinal,
                    "sourceVehicleOrdinal": record.source_vehicle_ordinal,
                })
            })
            .collect()
    }

    fn ordinal_json(ordinals: &[PopulationOrdinal]) -> Vec<Value> {
        ordinals
            .iter()
            .map(|ordinal| {
                json!({
                    "populationRank": ordinal.population_rank,
                    "sourceFileOrdinal": ordinal.source_file_ordinal,
                    "sourceVehicleOrdinal": ordinal.source_vehicle_ordinal,
                })
            })
            .collect()
    }

    fn catalog(bundle: &RoutePopulationBundle) -> Vec<(String, Vec<String>)> {
        bundle
            .routes
            .iter()
            .map(|route| (route.id.clone(), route.edge_ids.clone()))
            .collect()
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

    fn has_lane_hop(topology: &NormalizedTopology, from_lane: u32) -> bool {
        topology
            .path_by_connection
            .keys()
            .any(|(from, candidate_lane, to, _)| {
                from == "B" && *candidate_lane == from_lane && to == "C"
            })
    }

    fn has_road_hop(topology: &NormalizedTopology) -> bool {
        topology
            .path_by_connection
            .keys()
            .any(|(from, _, to, _)| from == "B" && to == "C")
    }

    fn validate_cases(cases: &[CaseEvidence]) {
        assert_eq!(cases.len(), 11);
        let case = |name| cases.iter().find(|case| case.name == name).expect("case");
        for case in cases {
            assert!(case.not_executed.contains(&"runtime-route-registration"));
        }

        let fixture = case("fixture-self-check");
        assert_eq!(fixture.source_paths.len(), 2);
        assert_eq!(fixture.details["gapNetworkLaneCount"], json!(6));
        assert_eq!(fixture.details["continuousNetworkLaneCount"], json!(7));
        assert_eq!(fixture.details["gapBLaneIndices"], json!([0, 1]));
        assert_eq!(fixture.details["noB0B1Connection"], json!(true));
        assert_eq!(fixture.details["noB0ToCPath"], json!(true));
        assert_eq!(fixture.details["noStubChanges"], json!(true));
        assert_eq!(
            fixture.details["continuousPaths"]
                .as_array()
                .expect("continuous paths")
                .len(),
            3
        );

        assert_error(case("direct-gap"), NO_COMPLETE_ROUTE_ERROR);
        assert_eq!(case("direct-gap").stage, "direct-route-expansion");
        assert_error(case("fail-fast-gap"), NO_COMPLETE_ROUTE_ERROR);
        assert_eq!(case("fail-fast-gap").error, case("direct-gap").error);

        let diagnostic = case("diagnostic-gap");
        assert_eq!(diagnostic.population.len(), 2);
        assert_eq!(diagnostic.population_ordinals.len(), 2);
        assert_eq!(diagnostic.details["populationRecordCount"], json!(2));
        assert_eq!(diagnostic.details["routesTomlNone"], json!(true));
        assert_eq!(
            diagnostic.details["infeasibilityReportPresent"],
            json!(true)
        );
        assert_eq!(diagnostic.details["repeatEqual"], json!(true));

        let continuous = case("continuous-direct");
        assert_eq!(continuous.final_edge_chain, expected_final_chain());
        assert_eq!(continuous.population.len(), 2);
        assert_eq!(continuous.bound_population.len(), 2);
        assert_eq!(continuous.details["repeatCatalogEqual"], json!(true));
        assert_eq!(continuous.details["repeatRecordsEqual"], json!(true));
        for (selected, bound) in continuous
            .population
            .iter()
            .zip(&continuous.bound_population)
        {
            assert_eq!(selected["vehicleId"], bound["vehicleId"]);
            assert_eq!(selected["populationRank"], bound["populationRank"]);
            assert_eq!(bound["routeId"], json!("sumo:route-0"));
            assert_eq!(bound["vehicleProfileId"], json!("sumo:passenger1"));
        }

        assert_error(
            case("unknown-lane-normalization"),
            "connection references unknown lane edge=\"B\" index=9",
        );
        assert_eq!(case("unknown-lane-normalization").stage, "normalization");
        assert_error(
            case("unknown-road-direct"),
            "DUE vehicle \"unknown-road\" references unknown road edge \"GHOST\"",
        );
        assert_error(
            case("internal-road-direct"),
            "DUE vehicle \"internal-road\" route references internal edge \":J2_0\"",
        );
        for (name, expected) in [
            (
                "unknown-road-diagnostic",
                "references unknown road edge \"GHOST\"",
            ),
            (
                "internal-road-diagnostic",
                "route references internal edge \":J2_0\"",
            ),
        ] {
            let case = case(name);
            let id = case.population[0]["vehicleId"]
                .as_str()
                .expect("vehicle ID");
            assert_error(case, &format!("DUE vehicle {id:?} {expected}"));
        }

        let missing_hop = case("missing-hop-direct");
        assert_error(missing_hop, NO_COMPLETE_ROUTE_ERROR);
        assert_eq!(missing_hop.details["hopPresent"], json!(false));
        assert_eq!(missing_hop.details["sameErrorAsGap"], json!(true));
    }

    fn assert_error(case: &CaseEvidence, expected_message: &str) {
        let error = case.error.as_ref().expect("expected rejected case");
        assert_eq!(case.outcome, "rejected");
        assert_eq!(error.variant, "SumoModel");
        assert_eq!(error.message, expected_message);
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

    fn export_command(output: &std::path::Path) -> String {
        let target =
            std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "<worktree-target>".to_owned());
        format!(
            "CARGO_TARGET_DIR='{target}' LANEFLOW_806_A0_RESULTS='{}' cargo +1.99.1 test --locked -p laneflow-lust-converter --lib convert::routes::test_support::evidence::a0_route_evidence -- --exact --nocapture",
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
}
