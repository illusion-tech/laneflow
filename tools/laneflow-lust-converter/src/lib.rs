//! LuST Scenario v2.0 source/static converter (#253).
//!
//! Delivers pinned source verification, static Traffic/Spatial conversion
//! (Junction/Movement/ManeuverPath/Signals/profiles/DUE routes/population),
//! conversion report, deterministic tar bundles, and semantic/build provenance.

mod config;
mod convert;
mod error;
mod output;
mod source;
mod sumo;

pub use config::{LustConverterConfig, load_config, load_config_with_bytes};
pub use convert::{
    LUST_PASSENGER_VTYPE_IDS, POPULATION_CANDIDATE_COUNT, POPULATION_DEPART_END_SECONDS,
    POPULATION_DEPART_START_SECONDS, POPULATION_SELECTED_COUNT, PopulationRecord,
    StaticConversionArtifacts, StubWeldDisposition, StubWeldPolicy, StubWeldRecord,
    TopologyConvertOptions, UnclaimedSignalArm, convert_network_topology,
    convert_network_topology_with_tll, scan_stub_weld_candidates, select_passenger_vtypes,
    select_population, stub_weld_manifest_json,
};
pub use error::{Error, Result};
pub use output::{
    BudgetOutcome, BuildInvocation, BuildProvenanceInput, ConversionReportInput,
    ConvertOutputPaths, InfeasibilityDiagnosis, InfeasibilityMechanism, InfeasibilityReport,
    LicenseArtifacts, RawOutputDigests, ReleaseAssetUrls, ReportSource, SemanticConfig,
    SemanticProvenanceInput, TarMember, TopologyArtifacts, TopologyCounts, VerifiedSourceTar,
    build_build_provenance, build_conversion_report, build_semantic_provenance,
    convert_with_config, embedded_notice_bytes, embedded_odbl_bytes, hex_sha256,
    write_deterministic_ustar,
};
pub use source::{
    LUST_COMMIT, LUST_REPOSITORY, LUST_TAG, PINNED_SOURCE_FILES, PinnedSourceFile,
    VerifiedLustInputs, VerifiedSourceFile, VerifiedSourceSet, prepare_verified_lust_inputs,
    recheck_source_revision, verify_source_dir,
};
pub use sumo::{
    DueVehicle, ExactDecimal, LUST_CONV_BOUNDARY, LUST_FRAME_ID, LUST_NET_OFFSET, SUMO_ID_PREFIX,
    SumoNetwork, SumoVType, parse_due_routes_xml, parse_parking_polygon_count,
    parse_sumo_network_xml, parse_tll_static_xml, parse_vtypes_xml,
};

use std::path::Path;

use crate::convert::{
    convert_network_topology_with_tll_and_profiles, convert_static_with_due,
    convert_vehicle_profiles,
};
use crate::output::convert_with_config as run_convert;

/// Verify the pinned LuST source set under `source_dir`.
pub fn verify_source(source_dir: &Path) -> Result<VerifiedSourceSet> {
    verify_source_dir(source_dir)
}

/// Convert topology packages from an already-parsed SUMO network (no tll/profiles).
pub fn convert_topology_from_network(
    network: &SumoNetwork,
    options: &TopologyConvertOptions,
) -> Result<TopologyArtifacts> {
    convert_network_topology(network, options)
}

/// Convert topology packages from SUMO network XML text (no tll/profiles).
pub fn convert_topology_from_xml(
    xml: &str,
    options: &TopologyConvertOptions,
) -> Result<TopologyArtifacts> {
    let network = parse_sumo_network_xml(xml)?;
    convert_network_topology(&network, options)
}

/// Convert topology + static signals from network XML and `tll.static.xml` text.
pub fn convert_topology_from_xml_with_tll(
    net_xml: &str,
    tll_xml: &str,
    options: &TopologyConvertOptions,
) -> Result<TopologyArtifacts> {
    let network = parse_sumo_network_xml(net_xml)?;
    let tll = parse_tll_static_xml(tll_xml)?;
    convert_network_topology_with_tll(&network, &tll, options)
}

/// Convert topology + signals + passenger profiles from net/tll/vtypes XML.
///
/// 诊断清单模式的来源声明：摘要对 `net_xml` 实际字节求值；本入口不执行
/// verify-source，报告按未验证输入如实标注（verify-source 见 [`verify_source`]）。
pub fn convert_topology_from_xml_with_tll_and_vtypes(
    net_xml: &str,
    tll_xml: &str,
    vtypes_xml: &str,
    options: &TopologyConvertOptions,
) -> Result<TopologyArtifacts> {
    convert_topology_from_xml_with_tll_and_vtypes_and_source(
        net_xml,
        tll_xml,
        vtypes_xml,
        options,
        xml_report_source(net_xml),
    )
}

/// [`convert_topology_from_xml_with_tll_and_vtypes`] 的显式来源声明变体：
/// 诊断清单模式的正式验收入口——来源已经 verify-source 走过（checkout
/// revision + pinned digest）的调用方传入 `verified = true` 与 pinned 校验
/// 得到的摘要（#253 R2 第三轮：先验证、再绑定、后转换）。
///
/// R8 残留缺口闭合：本入口分别接收 XML 与 ReportSource，调用方可能错配
/// 「旧记录 + 改动后字节」套取 0.5 m 删焊例外——verified 时对**本次实际
/// 消费字节**重算 SHA-256（解析/转换之前）：net 摘要须等于记录摘要，三份
/// 均须命中 pinned 条目；任一失配 fail-closed（不降级、不静默继续）。
pub fn convert_topology_from_xml_with_tll_and_vtypes_and_source(
    net_xml: &str,
    tll_xml: &str,
    vtypes_xml: &str,
    options: &TopologyConvertOptions,
    source: ReportSource,
) -> Result<TopologyArtifacts> {
    rebind_verified_source(net_xml, tll_xml, vtypes_xml, &source)?;
    let network = parse_sumo_network_xml(net_xml)?;
    let tll = parse_tll_static_xml(tll_xml)?;
    let vtypes = parse_vtypes_xml(vtypes_xml)?;
    let passengers = select_passenger_vtypes(&vtypes)?;
    let profiles = convert_vehicle_profiles(&passengers)?;
    convert_network_topology_with_tll_and_profiles(&network, &tll, &profiles, options, source)
}

/// [`prepare_verified_lust_inputs`] 的配套转换入口：字节与验证记录同源
/// （结构内聚，无法错配），内部委托组合入口——消费时重算绑定仍会执行，
/// 双保险。
pub fn convert_topology_from_verified_lust_inputs(
    prepared: &VerifiedLustInputs,
    options: &TopologyConvertOptions,
) -> Result<TopologyArtifacts> {
    convert_topology_from_xml_with_tll_and_vtypes_and_source(
        prepared.net_xml(),
        prepared.tll_xml(),
        prepared.vtypes_xml(),
        options,
        prepared.report_source().clone(),
    )
}

/// verified 来源的消费时重算绑定（#253 R8 残留缺口）：在解析/转换之前对
/// 本次实际消费字节重算 SHA-256。未 verified 的通用输入不受影响（其
/// BlockedDomain 门控在 normalize 阶段拒绝焊接）。
fn rebind_verified_source(
    net_xml: &str,
    tll_xml: &str,
    vtypes_xml: &str,
    source: &ReportSource,
) -> Result<()> {
    if !source.is_verified() {
        return Ok(());
    }
    let pinned_sha256 = |relative_path: &'static str| -> Result<&'static str> {
        source::PINNED_SOURCE_FILES
            .iter()
            .find(|pinned| pinned.relative_path == relative_path)
            .map(|pinned| pinned.sha256_hex)
            .ok_or_else(|| Error::SumoModel(format!("pinned entry missing {relative_path}")))
    };
    // ① net 摘要与记录一致性（None 视为失配）：记录必须描述本次消费字节。
    let net_hex = output::digest::hex_sha256(net_xml.as_bytes());
    let record_hex = source.net_digest().map(str::to_owned).unwrap_or_default();
    let record_hex = record_hex.strip_prefix("sha256:").unwrap_or(&record_hex);
    if record_hex != net_hex {
        return Err(Error::SourceChangedAfterVerification {
            relative_path: "scenario/lust.net.xml",
            expected: format!("sha256:{record_hex}"),
            actual: format!("sha256:{net_hex}"),
        });
    }
    // ② 三份均须命中 pinned 条目（唯一 verified 域即 pinned 基线）。
    for (relative_path, bytes) in [
        ("scenario/lust.net.xml", net_xml.as_bytes()),
        ("scenario/tll.static.xml", tll_xml.as_bytes()),
        ("scenario/vtypes.add.xml", vtypes_xml.as_bytes()),
    ] {
        let actual = output::digest::hex_sha256(bytes);
        let expected = pinned_sha256(relative_path)?;
        if actual != expected {
            return Err(Error::SourceChangedAfterVerification {
                relative_path,
                expected: format!("sha256:{expected}"),
                actual: format!("sha256:{actual}"),
            });
        }
    }
    Ok(())
}

/// xml 入口的诊断来源声明：实际输入字节摘要 + 未执行独立校验。
fn xml_report_source(net_xml: &str) -> ReportSource {
    ReportSource::xml_unverified(output::digest::sha256_digest(net_xml.as_bytes()))
}

/// Convert topology + DUE routes + population table from net/tll/vtypes/DUE XML.
///
/// `due_xmls` must be the three `local.static.{0,1,2}.rou.xml` texts in order.
pub fn convert_static_from_xml_with_due(
    net_xml: &str,
    tll_xml: &str,
    vtypes_xml: &str,
    due_xmls: [&str; 3],
    options: &TopologyConvertOptions,
) -> Result<StaticConversionArtifacts> {
    convert_static_from_xml_with_due_and_source(
        net_xml,
        tll_xml,
        vtypes_xml,
        due_xmls,
        options,
        xml_report_source(net_xml),
    )
}

/// [`convert_static_from_xml_with_due`] 的显式来源声明变体：verify-source
/// 走过的调用方传入 `verified = true` 与 pinned 校验得到的摘要。
pub(crate) fn convert_static_from_xml_with_due_and_source(
    net_xml: &str,
    tll_xml: &str,
    vtypes_xml: &str,
    due_xmls: [&str; 3],
    options: &TopologyConvertOptions,
    report_source: ReportSource,
) -> Result<StaticConversionArtifacts> {
    let network = parse_sumo_network_xml(net_xml)?;
    let tll = parse_tll_static_xml(tll_xml)?;
    let vtypes = parse_vtypes_xml(vtypes_xml)?;
    let passengers = select_passenger_vtypes(&vtypes)?;
    let profiles = convert_vehicle_profiles(&passengers)?;
    let mut due_vehicles = Vec::new();
    for (ordinal, xml) in due_xmls.into_iter().enumerate() {
        let file_ordinal = u8::try_from(ordinal).expect("0..2 fits u8");
        due_vehicles.extend(parse_due_routes_xml(xml, file_ordinal)?);
    }
    convert_static_with_due(
        &network,
        &tll,
        &profiles,
        &due_vehicles,
        options,
        report_source,
    )
}

/// Verify pinned source and write static/source bundles plus provenance.
pub fn convert(config_path: &Path) -> Result<ConvertOutputPaths> {
    let (config, config_bytes) = load_config_with_bytes(config_path)?;
    run_convert(&config, &config_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_record_with_mismatched_net_bytes_fails_closed() {
        // R8 残留缺口回归：组合入口分别接收 XML 与 ReportSource——「旧
        // verified 记录 + 改动后 net 字节」必须 fail-closed（消费时重算绑定，
        // 解析/转换之前）。
        let source = ReportSource::verified(format!(
            "sha256:{}",
            output::digest::hex_sha256(b"<pinned-net/>")
        ));
        let error = convert_topology_from_xml_with_tll_and_vtypes_and_source(
            "<tampered-net/>",
            "<tll/>",
            "<vtypes/>",
            &TopologyConvertOptions::default(),
            source,
        )
        .expect_err("stale verified record with mismatched bytes must fail closed");
        match error {
            Error::SourceChangedAfterVerification { relative_path, .. } => {
                assert_eq!(relative_path, "scenario/lust.net.xml");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn verified_record_matching_nonpinned_bytes_fails_closed() {
        // 记录与消费字节一致、但字节非 pinned 基线：pinned 命中检查必须拒绝
        // （唯一 verified 域即 pinned 基线；不得用任意字节伪造摘要）。
        let bytes = b"<arbitrary-but-self-consistent/>";
        let source =
            ReportSource::verified(format!("sha256:{}", output::digest::hex_sha256(bytes)));
        let error = convert_topology_from_xml_with_tll_and_vtypes_and_source(
            std::str::from_utf8(bytes).expect("utf8"),
            "<tll/>",
            "<vtypes/>",
            &TopologyConvertOptions::default(),
            source,
        )
        .expect_err("non-pinned bytes must not enter the verified domain");
        match error {
            Error::SourceChangedAfterVerification { relative_path, .. } => {
                assert_eq!(relative_path, "scenario/lust.net.xml");
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
