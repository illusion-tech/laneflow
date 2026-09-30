//! Conversion report for the shared static bundle (§3.6).

use serde::Serialize;

use crate::{
    Result,
    convert::population::{
        POPULATION_CANDIDATE_COUNT, POPULATION_DEPART_END_SECONDS, POPULATION_DEPART_START_SECONDS,
        POPULATION_SELECTED_COUNT,
    },
    output::{digest::sha256_digest, json_bytes},
    source::{LUST_COMMIT, LUST_REPOSITORY, LUST_TAG},
};

/// 「声明臂无受控 link」report 序列化视图（#253 K4a）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportUnclaimedArm {
    controller_id: String,
    missing_link_indices: Vec<u32>,
}

/// Inputs used to build a conversion report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversionReportInput {
    pub external_edge_count: u64,
    pub external_lane_count: u64,
    pub connection_count: u64,
    pub junction_count: u64,
    pub movement_count: u64,
    pub maneuver_path_count: u64,
    pub route_catalog_count: u64,
    pub vehicle_profile_count: u64,
    pub signal_controller_count: u64,
    pub signal_group_count: u64,
    pub stop_line_count: u64,
    pub maneuver_gate_count: u64,
    pub population_record_count: u64,
    pub require_lust_population_count: bool,
    pub parking_registry_empty: bool,
    /// §3.5 parking polygon 健康事实（pinned 基线 175，#253 K6）。
    pub parking_polygon_count: u64,
    /// §3.3 信号相位计数（与其他信号对象计数并列；pinned 基线 1,298，#253 K7）。
    pub signal_phase_count: u64,
    /// 「声明臂无受控 link」health 事实（#253 K4a；通常为 empty）。
    pub unclaimed_signal_arms: Vec<crate::UnclaimedSignalArm>,
    pub major_minor_green_collapsed: bool,
    /// 诊断模式无 network.lfca（`None` → digest 字段序列化不出现，由
    /// infeasibility_survey_bytes 接任交付物认证，#253 N1）。
    pub network_lfca_bytes: Option<Vec<u8>>,
    /// 诊断交付物 `issue253-infeasible-survey.md` 字节（fail-fast 路径 None）。
    pub infeasibility_survey_bytes: Option<Vec<u8>>,
    pub routes_toml_bytes: Vec<u8>,
    pub manifest_bytes: Vec<u8>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConversionReport {
    format_version: &'static str,
    source: ReportSource,
    health: ReportHealth,
    normalization: ReportNormalization,
    population: ReportPopulation,
    warning_boundaries: ReportWarnings,
    digests: ReportDigests,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportSource {
    repository: &'static str,
    tag: &'static str,
    commit: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportHealth {
    external_edge_count: u64,
    external_lane_count: u64,
    connection_count: u64,
    parking_registry_empty: bool,
    /// parking polygon 健康事实（§3.5；pinned 基线 175，#253 K6）。
    parking_polygon_count: u64,
    /// 「声明臂无受控 link」health 事实（K4a；pinned：controller -13968 缺 index 9）。
    unclaimed_signal_arms: Vec<ReportUnclaimedArm>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportNormalization {
    junction_count: u64,
    movement_count: u64,
    maneuver_path_count: u64,
    route_catalog_count: u64,
    vehicle_profile_count: u64,
    signal_controller_count: u64,
    signal_group_count: u64,
    /// 信号相位计数（§3.3；pinned 基线 1,298，#253 K7）。
    signal_phase_count: u64,
    stop_line_count: u64,
    maneuver_gate_count: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportPopulation {
    depart_start_seconds: &'static str,
    depart_end_seconds_exclusive: &'static str,
    require_lust_candidate_count: bool,
    candidate_count_expected: Option<u64>,
    selected_count_expected: Option<u64>,
    selected_count: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportWarnings {
    /// SUMO major/minor green collapse into a single Green aspect (§3.3).
    major_minor_green_collapsed_to_green: bool,
    /// LuST polygons are health facts only; Traffic Parking stays empty (§3.5).
    parking_polygons_not_synthesized: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportDigests {
    /// 诊断模式无 network.lfca：字段不出现，认证对象替换为诊断清单（#253 N1）。
    #[serde(skip_serializing_if = "Option::is_none")]
    network_lfca: Option<String>,
    routes_toml: String,
    manifest_toml: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    infeasibility_survey: Option<String>,
}

/// Serialize the conversion report JSON (pretty + trailing newline).
pub fn build_conversion_report(input: &ConversionReportInput) -> Result<Vec<u8>> {
    let report = ConversionReport {
        format_version: "0.2",
        source: ReportSource {
            repository: LUST_REPOSITORY,
            tag: LUST_TAG,
            commit: LUST_COMMIT,
        },
        health: ReportHealth {
            external_edge_count: input.external_edge_count,
            external_lane_count: input.external_lane_count,
            connection_count: input.connection_count,
            parking_registry_empty: input.parking_registry_empty,
            parking_polygon_count: input.parking_polygon_count,
            unclaimed_signal_arms: input
                .unclaimed_signal_arms
                .iter()
                .map(|arm| ReportUnclaimedArm {
                    controller_id: arm.controller_id.clone(),
                    missing_link_indices: arm.missing_link_indices.clone(),
                })
                .collect(),
        },
        normalization: ReportNormalization {
            junction_count: input.junction_count,
            movement_count: input.movement_count,
            maneuver_path_count: input.maneuver_path_count,
            route_catalog_count: input.route_catalog_count,
            vehicle_profile_count: input.vehicle_profile_count,
            signal_controller_count: input.signal_controller_count,
            signal_group_count: input.signal_group_count,
            signal_phase_count: input.signal_phase_count,
            stop_line_count: input.stop_line_count,
            maneuver_gate_count: input.maneuver_gate_count,
        },
        population: ReportPopulation {
            depart_start_seconds: POPULATION_DEPART_START_SECONDS,
            depart_end_seconds_exclusive: POPULATION_DEPART_END_SECONDS,
            require_lust_candidate_count: input.require_lust_population_count,
            candidate_count_expected: input
                .require_lust_population_count
                .then_some(POPULATION_CANDIDATE_COUNT as u64),
            selected_count_expected: input
                .require_lust_population_count
                .then_some(POPULATION_SELECTED_COUNT as u64),
            selected_count: input.population_record_count,
        },
        warning_boundaries: ReportWarnings {
            major_minor_green_collapsed_to_green: input.major_minor_green_collapsed,
            parking_polygons_not_synthesized: true,
        },
        digests: ReportDigests {
            network_lfca: input
                .network_lfca_bytes
                .as_ref()
                .map(|bytes| sha256_digest(bytes)),
            routes_toml: sha256_digest(&input.routes_toml_bytes),
            manifest_toml: sha256_digest(&input.manifest_bytes),
            infeasibility_survey: input
                .infeasibility_survey_bytes
                .as_ref()
                .map(|bytes| sha256_digest(bytes)),
        },
    };
    json_bytes("ConversionReport", &report)
}
