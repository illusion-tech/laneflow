use std::{fs, path::PathBuf};

use laneflow_lust_converter::{
    ExactDecimal, LUST_FRAME_ID, TopologyConvertOptions, convert_topology_from_xml,
    parse_sumo_network_xml,
};

#[test]
fn fixture_parses_lane_and_connection_counts() {
    let network = parse_sumo_network_xml(&fixture_xml()).expect("fixture parses");
    assert!(network.location.matches_lust_anchors());
    assert_eq!(network.lanes.len(), 5);
    assert_eq!(network.external_lane_count(), 3);
    assert_eq!(network.external_edge_count(), 3);
    assert_eq!(network.connections.len(), 4);
}

#[test]
fn fixture_topology_round_trips_loader_and_spatial() {
    let artifacts = convert_topology_from_xml(&fixture_xml(), &TopologyConvertOptions::default())
        .expect("topology convert");
    assert_eq!(artifacts.edge_count, 5);
    assert!(
        String::from_utf8_lossy(&artifacts.spatial).contains(LUST_FRAME_ID),
        "spatial package must use fixed LuST frameId"
    );
    assert!(
        String::from_utf8_lossy(&artifacts.traffic).contains("sumo:west_0"),
        "lane IDs must use sumo: namespace"
    );
    let traffic = String::from_utf8_lossy(&artifacts.traffic);
    assert!(
        traffic.contains("\"id\": \"sumo:J\""),
        "fixture must emit road junction sumo:J"
    );
    assert!(
        traffic.contains("sumo:J:west-to-east"),
        "fixture must emit west-to-east Movement"
    );
    assert!(
        traffic.contains("sumo:J:west-to-south"),
        "fixture must emit west-to-south Movement"
    );
    assert!(
        traffic.contains("\"internalEdgeIds\": [\n        \"sumo::J_0_0\"\n      ]")
            || traffic.contains("\"internalEdgeIds\": [\"sumo::J_0_0\"]"),
        "fixture must emit ManeuverPath via :J_0_0"
    );
}

#[test]
fn fixture_topology_is_byte_deterministic() {
    let options = TopologyConvertOptions::default();
    let first = convert_topology_from_xml(&fixture_xml(), &options).expect("first");
    let second = convert_topology_from_xml(&fixture_xml(), &options).expect("second");
    assert_eq!(first.traffic, second.traffic);
    assert_eq!(first.spatial, second.spatial);
    assert_eq!(first.manifest, second.manifest);
}

#[test]
fn simplified_origin_matches_three_step_formula_on_lust_location() {
    let network = parse_sumo_network_xml(&fixture_xml()).expect("fixture parses");
    let origin = network.location.canonical_origin().expect("origin");
    let expected_x = ExactDecimal::from_str_checked("292255.54");
    let expected_z = ExactDecimal::from_str_checked("5498125.65");
    assert_eq!(origin.0.to_f64().unwrap(), expected_x.to_f64().unwrap());
    assert_eq!(origin.1.to_f64().unwrap(), expected_z.to_f64().unwrap());

    let sx = ExactDecimal::from_str_checked("6806.88");
    let sy = ExactDecimal::from_str_checked("5727.52");
    let projected_x = sx
        .checked_sub(network.location.net_offset.0)
        .unwrap();
    let projected_y = sy
        .checked_sub(network.location.net_offset.1)
        .unwrap();
    let x = projected_x.checked_sub(origin.0).unwrap().to_f64().unwrap();
    let z = projected_y.checked_sub(origin.1).unwrap().to_f64().unwrap();
    assert_eq!(x, 0.0);
    assert_eq!(z, 0.0);
}

#[test]
#[ignore = "requires LUST_SOURCE_DIR pointing at commit c4bd5bd3751d426d42a9a1749c815e47ea188549"]
fn full_lust_net_topology_matches_external_lane_anchor() {
    let source_dir = std::env::var("LUST_SOURCE_DIR").expect("LUST_SOURCE_DIR");
    let net_path = PathBuf::from(source_dir).join("scenario/lust.net.xml");
    let xml = fs::read_to_string(&net_path).expect("read lust.net.xml");
    let network = parse_sumo_network_xml(&xml).expect("parse lust.net.xml");
    assert!(network.location.matches_lust_anchors());
    assert_eq!(network.external_edge_count(), 5_779);
    assert_eq!(network.external_lane_count(), 8_622);
    assert_eq!(network.connections.len(), 30_051);

    let artifacts = convert_topology_from_xml(
        &xml,
        &TopologyConvertOptions {
            require_lust_location_anchors: true,
            ..TopologyConvertOptions::default()
        },
    )
    .expect("full topology convert");
    assert_eq!(artifacts.edge_count, network.lanes.len());
}

fn fixture_xml() -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/minimal/t-junction.net.xml"),
    )
    .expect("read fixture")
}

trait FromStrChecked {
    fn from_str_checked(input: &str) -> Self;
}

impl FromStrChecked for ExactDecimal {
    fn from_str_checked(input: &str) -> Self {
        input.parse().expect("decimal")
    }
}
