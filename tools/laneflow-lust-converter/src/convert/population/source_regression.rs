//! 当前人口选择的固定真实源回归：源验证、选取不变性、路线 fail-closed 与
//! 输出/provenance 一致性。旧 #1029 三算法选型与 #1053 迁移 baseline 对照已随
//! #1384 退役；本模块只对照当前生产规则与独立固定预期，不做计时。

use std::{fs, path::PathBuf};

use serde_json::{Value, json};

use super::*;
use crate::{
    convert::{
        junction::{StubWeldGate, normalize_junctions},
        routes::population_route_failures,
    },
    output::digest::sha256_digest,
    source::{LUST_COMMIT, PINNED_SOURCE_FILES, read_verified, recheck_source_revision},
    sumo::{parse_due_routes_xml_filtered, parse_sumo_network_xml},
};

/// 冻结记录的独立固定预期：取自 #1053 迁移证据 baseline（含数据冻结提交
/// `564f9ac8` 的 `research/issue-1053-lust-population-v2/baseline.json`），
/// 期望值不由当次生产结果生成。`FROZEN_ROUTE_FAILURES` 是 #806 路线展开
/// 语义修订前的当前缺口；修复 #806 时须连同其余预期一并重登记。
const FROZEN_FIRST_SELECTED_ID: &str = "carIn47455:1";
const FROZEN_ROUTE_FAILURES: usize = 9_344;
const FROZEN_SELECTED_EDGE_STRINGS: usize = 375_891;
const FROZEN_POPULATION_SHA256: &str =
    "4a8b759fa1bf91526e1e95a2c3efd86597591c05a10ff84e6c2cfef32e746cdd";
const FROZEN_CONVERSION_REPORT_SHA256: &str =
    "2f3c6fb446d4add7bd9e546e695a811b9133019f70cd70a06cf0e8e7918d8f3d";

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

#[test]
#[ignore = "需要固定 LUST_SOURCE_DIR；可设 LUST_POPULATION_REGRESSION_OUTPUT_DIR 保留证据目录"]
fn pinned_lust_population_source_regression() {
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

    // 选取与独立固定预期：require 标志同时钉住候选数 10,592 与入选数 10,000。
    let records = select_population(&vehicles, true).expect("production population");
    assert_eq!(records.len(), POPULATION_SELECTED_COUNT);
    assert_eq!(records[0].vehicle_id, FROZEN_FIRST_SELECTED_ID);
    assert_eq!(
        records
            .iter()
            .map(|record| record.road_edge_ids.len())
            .sum::<usize>(),
        FROZEN_SELECTED_EDGE_STRINGS
    );
    assert_eq!(
        sha256_digest(&population_bytes(&records)),
        format!("sha256:{FROZEN_POPULATION_SHA256}")
    );

    // 改变全部源记录的输入顺序；来源序数只是溯源字段，不能参与唯一 ID 的排序。
    vehicles.reverse();
    assert_eq!(select_population(&vehicles, true).unwrap(), records);
    vehicles.reverse();

    // 路线缺口 fail-closed：逐条诊断可汇总，生产批量入口遇错即拒绝。
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
    assert_eq!(failures.len(), FROZEN_ROUTE_FAILURES);
    assert!(
        crate::convert::routes::build_routes_and_bind_population(&network, &topology, &records)
            .is_err()
    );
    drop(topology);
    drop(network);
    drop(net_xml);

    // 诊断模式转换：报告逐行对照内存选取（rank 与来源序数），provenance 摘要与
    // 报告字节绑定，报告整体摘要对照固定预期。
    let root = std::env::var_os("LUST_POPULATION_REGRESSION_OUTPUT_DIR").map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
                "../../target/lust-population-regression-{}",
                std::process::id()
            ))
        },
        PathBuf::from,
    );
    fs::create_dir_all(root.parent().expect("output parent")).unwrap();
    fs::create_dir(&root).expect("fresh evidence directory required");
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
    let report_digest = sha256_digest(&report_bytes);
    assert_eq!(
        report_digest,
        format!("sha256:{FROZEN_CONVERSION_REPORT_SHA256}"),
        "conversion report diverged from the frozen expectation"
    );
    let semantic: Value =
        serde_json::from_slice(&fs::read(&outputs.semantic_provenance).unwrap()).unwrap();
    let build: Value =
        serde_json::from_slice(&fs::read(&outputs.build_provenance).unwrap()).unwrap();
    assert_eq!(
        semantic["semanticOutputs"]["conversionReport"]["digest"],
        report_digest
    );
    assert_eq!(build["rawOutputDigests"]["conversionReport"], report_digest);
    assert_eq!(report["sourceCommit"], Value::from(LUST_COMMIT));
    println!("report_digest,{report_digest}");
    println!("evidence_directory,{}", root.display());
}
