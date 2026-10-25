//! #1053：固定源人口迁移、真实路线诊断与可选局部测量；不构造 DEMAND 位置。

use std::{collections::BTreeMap, fs, hint::black_box, path::PathBuf};

use serde_json::{Value, json};

use super::{
    digest_benchmark::{Case, measure, select_with_key, sha},
    *,
};
use crate::{
    convert::{
        junction::{StubWeldGate, normalize_junctions},
        routes::population_route_failures,
    },
    output::digest::sha256_digest,
    source::{LUST_COMMIT, PINNED_SOURCE_FILES, read_verified, recheck_source_revision},
    sumo::{parse_due_routes_xml_filtered, parse_sumo_network_xml},
};

fn population_bytes(records: &[PopulationRecord]) -> Vec<u8> {
    let rows = records
        .iter()
        .map(|record| {
            json!({
                "populationRank": record.population_rank,
                "vehicleId": record.vehicle_id,
                "typeId": record.type_id,
                "departSeconds": record.depart.to_string(),
                "roadEdgeIds": record.road_edge_ids,
                "sourceFileOrdinal": record.source_file_ordinal,
                "sourceVehicleOrdinal": record.source_vehicle_ordinal,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&json!({"derivationId": POPULATION_DERIVATION_ID, "records": rows})).unwrap()
}

fn write_json(root: &std::path::Path, name: &str, value: &impl serde::Serialize) {
    fs::write(root.join(name), serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

#[test]
#[ignore = "#1053：需要固定 LUST_SOURCE_DIR；可设 LUST_MIGRATION_OUTPUT_DIR 保留独立进程证据"]
fn pinned_lust_population_migration() {
    let baseline: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../research/issue-1053-lust-population-v2/baseline.json"
    )))
    .expect("frozen migration baseline");
    let benchmark = std::env::var_os("LUST_POPULATION_BENCH").is_some();
    assert!(
        !benchmark || !cfg!(debug_assertions),
        "release benchmark required"
    );
    let source_dir = PathBuf::from(std::env::var("LUST_SOURCE_DIR").expect("LUST_SOURCE_DIR"));
    let verified = crate::verify_source(&source_dir).expect("verified source");
    let mut vehicles = Vec::new();
    for (ordinal, file) in PINNED_SOURCE_FILES
        .iter()
        .filter(|file| file.relative_path.contains("DUERoutes/"))
        .enumerate()
    {
        let xml = read_verified(&verified, file.relative_path).expect("verified DUE");
        vehicles.extend(
            parse_due_routes_xml_filtered(
                &xml,
                u8::try_from(ordinal).unwrap(),
                is_population_candidate,
            )
            .expect("DUE parse"),
        );
    }
    recheck_source_revision(verified.source_dir()).expect("source revision after reads");
    let records = select_population(&vehicles, true).expect("production population");
    let legacy =
        select_with_key(&vehicles, |vehicle| sha(vehicle.id.as_bytes())).expect("historical SHA");
    let mut scratch = Vec::new();
    let reference = select_with_key(&vehicles, |vehicle| {
        population_digest(&vehicle.id, &mut scratch)
    })
    .expect("XXH3 reference");
    assert_eq!(records, reference, "all production fields");
    // 改变全部源记录的输入顺序；来源序数只是溯源字段，不能参与唯一 ID 的排序。
    vehicles.reverse();
    assert_eq!(select_population(&vehicles, true).unwrap(), records);
    vehicles.reverse();
    let candidates = vehicles
        .iter()
        .filter(|vehicle| is_population_candidate(&vehicle.type_id, &vehicle.depart))
        .collect::<Vec<_>>();
    assert_eq!(candidates.len(), POPULATION_CANDIDATE_COUNT);

    let root = std::env::var_os("LUST_MIGRATION_OUTPUT_DIR").map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
                "../../target/lust-population-migration-{}",
                std::process::id()
            ))
        },
        PathBuf::from,
    );
    fs::create_dir_all(root.parent().expect("output parent")).unwrap();
    fs::create_dir(&root).expect("fresh evidence directory required");
    let bytes = population_bytes(&records);
    fs::write(root.join("population.json"), &bytes).unwrap();

    let legacy_ranks = legacy
        .iter()
        .map(|record| (record.vehicle_id.as_str(), record.population_rank))
        .collect::<BTreeMap<_, _>>();
    let common = records
        .iter()
        .filter(|record| legacy_ranks.contains_key(record.vehicle_id.as_str()))
        .count();
    let same_rank = records
        .iter()
        .filter(|record| {
            legacy_ranks.get(record.vehicle_id.as_str()) == Some(&record.population_rank)
        })
        .count();
    let mut type_counts = BTreeMap::<&str, usize>::new();
    let mut source_counts = [0_usize; 3];
    let mut departure_bins = [0_usize; 6];
    for record in &records {
        *type_counts.entry(record.type_id.as_str()).or_default() += 1;
        source_counts[usize::from(record.source_file_ordinal)] += 1;
        let bin = (1..6)
            .find(|index| {
                record
                    .depart
                    .is_less_than((28_800 + index * 300).to_string().parse().unwrap())
            })
            .unwrap_or(6)
            - 1;
        departure_bins[bin] += 1;
    }
    let mut scratch = Vec::new();
    let mut key_bins = [0_usize; 16];
    let mut keys = HashSet::new();
    for vehicle in &candidates {
        let key = population_digest(&vehicle.id, &mut scratch);
        keys.insert(key);
        key_bins[(key >> 60) as usize] += 1;
    }
    let net_xml = read_verified(&verified, "scenario/lust.net.xml").expect("verified net");
    let network = parse_sumo_network_xml(&net_xml).expect("net parse");
    let topology = normalize_junctions(
        &network,
        &StubWeldGate::VerifiedDomain {
            net_digest: Some(sha256_digest(net_xml.as_bytes())),
        },
    )
    .expect("verified normalization");
    let failures = population_route_failures(&network, &topology, &records);
    let legacy_failures = population_route_failures(&network, &topology, &legacy);
    // 逐条诊断可以汇总失败；生产批量入口仍须遇错即拒绝，不生成删减过的人口。
    assert!(!failures.is_empty(), "known route gap remains visible");
    assert!(
        crate::convert::routes::build_routes_and_bind_population(&network, &topology, &records)
            .is_err()
    );
    let failures_bytes = serde_json::to_vec(&failures).unwrap();
    fs::write(root.join("route-failures.json"), &failures_bytes).unwrap();
    let summary = json!({
        "sourceCommit": LUST_COMMIT,
        "derivationId": POPULATION_DERIVATION_ID,
        "sourceRecords": vehicles.len(), "candidates": candidates.len(), "selected": records.len(),
        "populationDigest": sha256_digest(&bytes),
        "commonWithHistoricalSha": common, "removed": legacy.len() - common,
        "added": records.len() - common, "commonRankChanged": common - same_rank, "sameRank": same_rank,
        "candidateKeyCollisions": candidates.len() - keys.len(), "candidateKeyBins16": key_bins,
        "selectedTypeCounts": type_counts, "selectedSourceCounts": source_counts,
        "selectedDepartureBins300Seconds": departure_bins,
        "selectedEdgeStrings": records.iter().map(|record| record.road_edge_ids.len()).sum::<usize>(),
        "historicalShaEdgeStrings": legacy.iter().map(|record| record.road_edge_ids.len()).sum::<usize>(),
        "routeFailures": failures.len(), "historicalShaRouteFailures": legacy_failures.len(),
        "routeFailureDigest": sha256_digest(&failures_bytes),
        "firstSelectedId": records[0].vehicle_id,
        "firstRouteFailureId": failures.first().map(|entry| &entry.0),
    });
    write_json(&root, "summary.json", &summary);
    assert_eq!(
        summary, baseline["population"],
        "frozen population and route diagnostics"
    );
    println!("migration_summary,{summary}");
    drop(topology);
    drop(network);
    drop(net_xml);

    if benchmark {
        let mut scratch = Vec::new();
        measure(
            "candidate_hash",
            vec![
                Case::new("historical_sha", || {
                    for vehicle in &candidates {
                        black_box(sha(black_box(vehicle.id.as_bytes())));
                    }
                }),
                Case::new("population_v2", || {
                    for vehicle in &candidates {
                        black_box(population_digest(black_box(&vehicle.id), &mut scratch));
                    }
                }),
            ],
        );
        measure(
            "select_and_drop",
            vec![
                Case::new("historical_sha", || {
                    black_box(
                        select_with_key(black_box(&vehicles), |vehicle| sha(vehicle.id.as_bytes()))
                            .unwrap(),
                    );
                }),
                Case::new("population_v2", || {
                    black_box(select_population(black_box(&vehicles), true).unwrap());
                }),
            ],
        );
    }
    drop(vehicles);

    let config = root.join("config.toml");
    let output_dir = root.join("converted");
    let config_text = format!(
        "source_dir = {}\noutput_dir = {}\n",
        serde_json::to_string(&source_dir.to_string_lossy().replace('\\', "/")).unwrap(),
        serde_json::to_string(&output_dir.to_string_lossy().replace('\\', "/")).unwrap()
    );
    fs::write(&config, config_text).unwrap();
    let outputs = crate::convert(&config).expect("diagnostic conversion");
    assert!(
        outputs.routes.is_none() && outputs.network_lfca.is_none() && outputs.static_tar.is_none()
    );
    let report_bytes = fs::read(&outputs.conversion_report).unwrap();
    let report: Value = serde_json::from_slice(&report_bytes).unwrap();
    assert_eq!(report["formatVersion"], "0.3");
    assert_eq!(
        report["population"]["derivationId"],
        POPULATION_DERIVATION_ID
    );
    assert_eq!(report["population"]["selectedCount"], 10_000);
    let selected = report["population"]["selectedRecords"].as_array().unwrap();
    assert_eq!(selected.len(), records.len());
    for (row, record) in selected.iter().zip(&records) {
        assert_eq!(row["populationRank"], record.population_rank);
        assert_eq!(row["sourceFileOrdinal"], record.source_file_ordinal);
        assert_eq!(row["sourceVehicleOrdinal"], record.source_vehicle_ordinal);
    }
    let semantic: Value =
        serde_json::from_slice(&fs::read(&outputs.semantic_provenance).unwrap()).unwrap();
    let build: Value =
        serde_json::from_slice(&fs::read(&outputs.build_provenance).unwrap()).unwrap();
    let report_digest = sha256_digest(&report_bytes);
    assert_eq!(report_digest, baseline["conversionReportDigest"]);
    assert_eq!(
        sha256_digest(&fs::read(&outputs.infeasibility_survey).unwrap()),
        baseline["geometrySurveyDigest"],
        "population migration must not change geometry diagnostics"
    );
    assert_eq!(
        semantic["semanticOutputs"]["conversionReport"]["digest"],
        report_digest
    );
    assert_eq!(build["rawOutputDigests"]["conversionReport"], report_digest);
    println!("report_digest,{report_digest}");
    println!("evidence_directory,{}", root.display());
}
