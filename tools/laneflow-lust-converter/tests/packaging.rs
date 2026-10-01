use laneflow_lust_converter::{
    ConversionReportInput, build_conversion_report, embedded_notice_bytes, embedded_odbl_bytes,
};

#[test]
fn licenses_are_non_empty_and_contain_required_attribution() {
    let notice = std::str::from_utf8(embedded_notice_bytes()).expect("utf8");
    assert!(notice.contains("Road network data © OpenStreetMap contributors"));
    assert!(notice.contains("opendatacommons.org/licenses/odbl/1-0"));
    assert!(notice.contains("Codeca"));
    assert!(!embedded_odbl_bytes().is_empty());
}

#[test]
fn conversion_report_records_payload_digests_not_self() {
    let lfca = b"LFCA-fake\n".to_vec();
    let routes = b"format_version = \"0.1\"\n".to_vec();
    let manifest = b"manifest_version = 1\n".to_vec();
    let report = build_conversion_report(&ConversionReportInput {
        external_edge_count: 3,
        external_lane_count: 3,
        connection_count: 4,
        junction_count: 1,
        movement_count: 2,
        maneuver_path_count: 2,
        route_catalog_count: 2,
        vehicle_profile_count: 6,
        signal_controller_count: 1,
        signal_group_count: 2,
        stop_line_count: 1,
        maneuver_gate_count: 2,
        population_record_count: 3,
        require_lust_population_count: false,
        parking_registry_empty: true,
        parking_polygon_count: 0,
        signal_phase_count: 0,
        unclaimed_signal_arms: Vec::new(),
        major_minor_green_collapsed: true,
        network_lfca_bytes: Some(lfca.clone()),
        infeasibility_survey_bytes: None,
        routes_toml_bytes: Some(routes.clone()),
        manifest_bytes: manifest.clone(),
    })
    .expect("report");
    let text = String::from_utf8(report.clone()).expect("utf8");
    assert!(text.contains("sha256:"));
    assert!(text.contains("majorMinorGreenCollapsedToGreen"));
    assert!(!text.contains("conversionReport"));
    let again = build_conversion_report(&ConversionReportInput {
        external_edge_count: 3,
        external_lane_count: 3,
        connection_count: 4,
        junction_count: 1,
        movement_count: 2,
        maneuver_path_count: 2,
        route_catalog_count: 2,
        vehicle_profile_count: 6,
        signal_controller_count: 1,
        signal_group_count: 2,
        stop_line_count: 1,
        maneuver_gate_count: 2,
        population_record_count: 3,
        require_lust_population_count: false,
        parking_registry_empty: true,
        parking_polygon_count: 0,
        signal_phase_count: 0,
        unclaimed_signal_arms: Vec::new(),
        major_minor_green_collapsed: true,
        network_lfca_bytes: Some(lfca),
        infeasibility_survey_bytes: None,
        routes_toml_bytes: Some(routes),
        manifest_bytes: manifest,
    })
    .expect("report again");
    assert_eq!(report, again);
}
