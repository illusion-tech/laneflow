use laneflow_lust_converter::{
    BuildInvocation, BuildProvenanceInput, ConversionReportInput, LicenseArtifacts,
    RawOutputDigests, ReleaseAssetUrls, SemanticProvenanceInput, TarMember,
    build_build_provenance, build_conversion_report, build_semantic_provenance,
    embedded_notice_bytes, embedded_odbl_bytes, write_deterministic_ustar,
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
    let traffic = b"{\"traffic\":true}\n".to_vec();
    let spatial = b"{\"spatial\":true}\n".to_vec();
    let manifest = b"{\"manifest\":true}\n".to_vec();
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
        major_minor_green_collapsed: true,
        traffic_bytes: traffic.clone(),
        spatial_bytes: spatial.clone(),
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
        major_minor_green_collapsed: true,
        traffic_bytes: traffic,
        spatial_bytes: spatial,
        manifest_bytes: manifest,
    })
    .expect("report again");
    assert_eq!(report, again);
}

#[test]
fn semantic_and_build_provenance_are_byte_deterministic() {
    let licenses = LicenseArtifacts {
        license_md: b"MIT\n".to_vec(),
        odbl: embedded_odbl_bytes().to_vec(),
        notice: embedded_notice_bytes().to_vec(),
    };
    let source_tar = write_deterministic_ustar(&[TarMember {
        path: "LICENSE.md".to_owned(),
        contents: licenses.license_md.clone(),
    }])
    .expect("source tar");
    let static_tar = write_deterministic_ustar(&[TarMember {
        path: "lust-topology.traffic.json".to_owned(),
        contents: b"{}\n".to_vec(),
    }])
    .expect("static tar");
    let semantic_input = SemanticProvenanceInput {
        config_toml_bytes: b"source_dir=\"x\"\noutput_dir=\"y\"\n".to_vec(),
        licenses,
        release_urls: ReleaseAssetUrls::default(),
        source_tar: source_tar.clone(),
        static_tar: static_tar.clone(),
        traffic_bytes: b"{}\n".to_vec(),
        spatial_bytes: b"{}\n".to_vec(),
        manifest_bytes: b"{}\n".to_vec(),
        conversion_report_bytes: b"{}\n".to_vec(),
        population_bytes: b"{}\n".to_vec(),
    };
    let first = build_semantic_provenance(&semantic_input).expect("semantic");
    let second = build_semantic_provenance(&semantic_input).expect("semantic again");
    assert_eq!(first, second);
    assert!(String::from_utf8_lossy(&first).contains("lust-source.tar"));
    assert!(!String::from_utf8_lossy(&first).contains("converterCommit"));

    let build_input = BuildProvenanceInput {
        converter_commit: "abc123".to_owned(),
        rust_version: "1.96.0",
        cargo_lock_sha256: "deadbeef".to_owned(),
        config_digest: "sha256:00".to_owned(),
        semantic_provenance_digest: "sha256:11".to_owned(),
        invocation: BuildInvocation {
            command: "convert",
            require_lust_location_anchors: true,
            require_lust_population_count: true,
            traffic_artifact_ref: "lust-topology.traffic.json".to_owned(),
            spatial_artifact_ref: "lust-topology.spatial.json".to_owned(),
        },
        raw_output_digests: RawOutputDigests {
            traffic: "sha256:a".to_owned(),
            spatial: "sha256:b".to_owned(),
            scenario_manifest: "sha256:c".to_owned(),
            conversion_report: "sha256:d".to_owned(),
            population_table: "sha256:e".to_owned(),
            source_tar: "sha256:f".to_owned(),
            static_tar: "sha256:0".to_owned(),
        },
    };
    let build_a = build_build_provenance(&build_input).expect("build");
    let build_b = build_build_provenance(&build_input).expect("build again");
    assert_eq!(build_a, build_b);
    assert!(String::from_utf8_lossy(&build_a).contains("converterCommit"));
}
