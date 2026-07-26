use std::{fs, path::PathBuf};

use laneflow_lust_converter::{
    ExactDecimal, LUST_FRAME_ID, TopologyConvertOptions,
    convert_topology_from_xml_with_tll_and_vtypes, parse_sumo_network_xml, parse_vtypes_xml,
    select_passenger_vtypes,
};

#[test]
fn fixture_parses_lane_and_connection_counts() {
    let network = parse_sumo_network_xml(&fixture_net_xml()).expect("fixture parses");
    assert!(network.location.matches_lust_anchors());
    assert_eq!(network.lanes.len(), 5);
    assert_eq!(network.external_lane_count(), 3);
    assert_eq!(network.external_edge_count(), 3);
    assert_eq!(network.connections.len(), 4);
    assert_eq!(network.net_tl_logic_ids(), vec!["J".to_owned()]);
}

#[test]
fn fixture_vtypes_contain_bus_and_six_passengers() {
    let vtypes = parse_vtypes_xml(&fixture_vtypes_xml()).expect("parse vtypes");
    assert_eq!(vtypes.len(), 7);
    let passengers = select_passenger_vtypes(&vtypes).expect("select passengers");
    assert_eq!(passengers.len(), 6);
}

#[test]
fn fixture_topology_with_signals_and_profiles_round_trips() {
    let artifacts = convert_topology_from_xml_with_tll_and_vtypes(
        &fixture_net_xml(),
        &fixture_tll_xml(),
        &fixture_vtypes_xml(),
        &TopologyConvertOptions::default(),
    )
    .expect("topology+signals+profiles convert");
    assert_eq!(artifacts.edge_count, 5);
    let traffic = String::from_utf8_lossy(&artifacts.traffic);
    assert!(String::from_utf8_lossy(&artifacts.spatial).contains(LUST_FRAME_ID));
    assert!(traffic.contains("sumo:west_0"));
    assert!(traffic.contains("\"id\": \"sumo:J\""));
    assert!(traffic.contains("sumo:J:group-0"));
    assert!(traffic.contains("sumo:stop:west"));
    assert!(traffic.contains("\"durationMs\": 31000"));
    assert!(traffic.contains("sumo:passenger1"));
    assert!(traffic.contains("sumo:passenger5"));
    assert!(traffic.contains("\"emergencyDeceleration\": 8.0"));
    assert!(traffic.contains("\"timeHeadway\": 1.0"));
    assert!(!traffic.contains("sumo:bus"));
}

#[test]
fn fixture_topology_is_byte_deterministic() {
    let options = TopologyConvertOptions::default();
    let first = convert_topology_from_xml_with_tll_and_vtypes(
        &fixture_net_xml(),
        &fixture_tll_xml(),
        &fixture_vtypes_xml(),
        &options,
    )
    .expect("first");
    let second = convert_topology_from_xml_with_tll_and_vtypes(
        &fixture_net_xml(),
        &fixture_tll_xml(),
        &fixture_vtypes_xml(),
        &options,
    )
    .expect("second");
    assert_eq!(first.traffic, second.traffic);
    assert_eq!(first.spatial, second.spatial);
    assert_eq!(first.manifest, second.manifest);
}

#[test]
fn simplified_origin_matches_three_step_formula_on_lust_location() {
    let network = parse_sumo_network_xml(&fixture_net_xml()).expect("fixture parses");
    let origin = network.location.canonical_origin().expect("origin");
    let expected_x = ExactDecimal::from_str_checked("292255.54");
    let expected_z = ExactDecimal::from_str_checked("5498125.65");
    assert_eq!(origin.0.to_f64().unwrap(), expected_x.to_f64().unwrap());
    assert_eq!(origin.1.to_f64().unwrap(), expected_z.to_f64().unwrap());

    let sx = ExactDecimal::from_str_checked("6806.88");
    let sy = ExactDecimal::from_str_checked("5727.52");
    let projected_x = sx.checked_sub(network.location.net_offset.0).unwrap();
    let projected_y = sy.checked_sub(network.location.net_offset.1).unwrap();
    let x = projected_x.checked_sub(origin.0).unwrap().to_f64().unwrap();
    let z = projected_y.checked_sub(origin.1).unwrap().to_f64().unwrap();
    assert_eq!(x, 0.0);
    assert_eq!(z, 0.0);
}

#[test]
#[ignore = "requires LUST_SOURCE_DIR pointing at commit c4bd5bd3751d426d42a9a1749c815e47ea188549"]
fn full_lust_net_topology_matches_external_lane_anchor() {
    let source_dir = std::env::var("LUST_SOURCE_DIR").expect("LUST_SOURCE_DIR");
    let root = PathBuf::from(source_dir);
    let net_xml = fs::read_to_string(root.join("scenario/lust.net.xml")).expect("read net");
    let tll_xml = fs::read_to_string(root.join("scenario/tll.static.xml")).expect("read tll");
    let vtypes_xml = fs::read_to_string(root.join("scenario/vtypes.add.xml")).expect("read vtypes");
    let network = parse_sumo_network_xml(&net_xml).expect("parse lust.net.xml");
    assert!(network.location.matches_lust_anchors());
    assert_eq!(network.external_edge_count(), 5_779);
    assert_eq!(network.external_lane_count(), 8_622);
    assert_eq!(network.connections.len(), 30_051);
    assert_eq!(network.net_tl_logic_ids().len(), 201);

    let artifacts = convert_topology_from_xml_with_tll_and_vtypes(
        &net_xml,
        &tll_xml,
        &vtypes_xml,
        &TopologyConvertOptions {
            require_lust_location_anchors: true,
            ..TopologyConvertOptions::default()
        },
    )
    .expect("full topology+signals+profiles convert");
    assert_eq!(artifacts.edge_count, network.lanes.len());
}

fn fixture_net_xml() -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/minimal/t-junction.net.xml"),
    )
    .expect("read net fixture")
}

fn fixture_tll_xml() -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/minimal/t-junction.tll.xml"),
    )
    .expect("read tll fixture")
}

fn fixture_vtypes_xml() -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal/vtypes.add.xml"),
    )
    .expect("read vtypes fixture")
}

trait FromStrChecked {
    fn from_str_checked(input: &str) -> Self;
}

impl FromStrChecked for ExactDecimal {
    fn from_str_checked(input: &str) -> Self {
        input.parse().expect("decimal")
    }
}
