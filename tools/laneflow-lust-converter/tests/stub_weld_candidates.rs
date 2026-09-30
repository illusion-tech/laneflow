//! 点状 stub 删焊候选 manifest（G1 六条件之限定适用域）：生成与防漂移比对。
//! manifest 固定 pinned c4bd5bd3 的候选身份（含 rejected 与处置），
//! `scan_stub_weld_candidates` 为唯一数据源，重扫即比对。

use std::{fs, path::PathBuf};

use laneflow_lust_converter::{
    StubWeldDisposition, hex_sha256, parse_sumo_network_xml, scan_stub_weld_candidates,
    stub_weld_manifest_json,
};

fn manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("evidence/lust-stub-weld-candidates.json")
}

fn pinned_network() -> (String, laneflow_lust_converter::SumoNetwork) {
    let source_dir = std::env::var("LUST_SOURCE_DIR").expect("LUST_SOURCE_DIR");
    let net_xml = fs::read_to_string(PathBuf::from(source_dir).join("scenario/lust.net.xml"))
        .expect("read pinned net");
    let network = parse_sumo_network_xml(&net_xml).expect("parse pinned net");
    (net_xml, network)
}

/// 重新生成 manifest（候选身份、度量或门控阈值变化后手动跑：
/// `LUST_SOURCE_DIR=... cargo test -- --ignored regenerate`）。
#[test]
#[ignore = "requires LUST_SOURCE_DIR at c4bd5bd3; manual regeneration of the manifest"]
fn regenerate_lust_stub_weld_candidates_manifest() {
    let (net_xml, network) = pinned_network();
    let digest = format!("sha256:{}", hex_sha256(net_xml.as_bytes()));
    let json = stub_weld_manifest_json(&network, &digest).expect("manifest json");
    fs::write(manifest_path(), format!("{json}\n")).expect("write manifest");
}

/// 防漂移：对 pinned 源重扫，候选身份与处置同 manifest 一致（归一化换行后
/// 逐字节相等）；并锁定阈值 0.06 m 下的处置分布（36 welded / 48 rejected）。
#[test]
#[ignore = "requires LUST_SOURCE_DIR at c4bd5bd3"]
fn lust_stub_weld_candidates_match_manifest() {
    let (net_xml, network) = pinned_network();
    let candidates = scan_stub_weld_candidates(&network).expect("rescan");
    assert_eq!(
        candidates.len(),
        84,
        "pinned baseline has 84 stub candidates"
    );
    let count = |disposition: StubWeldDisposition| {
        candidates
            .iter()
            .filter(|candidate| candidate.disposition == disposition)
            .count()
    };
    // G1 确认后阈值 0.5 m：80 welded；4 条共享入口破坏（join 间隙 0.20–0.42 m
    // > 5 mm）拒绝保留，原始连接走正常穿越。
    assert_eq!(count(StubWeldDisposition::Welded), 80);
    assert_eq!(count(StubWeldDisposition::RejectedDisplacement), 0);
    assert_eq!(count(StubWeldDisposition::RejectedLocality), 0);
    assert_eq!(count(StubWeldDisposition::RejectedTopology), 0);
    assert_eq!(count(StubWeldDisposition::RejectedSharedEntry), 4);
    let refused_shared = candidates
        .iter()
        .filter(|candidate| candidate.disposition == StubWeldDisposition::RejectedSharedEntry)
        .collect::<Vec<_>>();
    for candidate in &refused_shared {
        assert!(
            !candidate.shared_traversals.is_empty(),
            "refused-shared-entry {} must carry traversal details",
            candidate.stub_lane_id
        );
        assert!(candidate.detail.contains("breaking shared traversal"));
    }

    let digest = format!("sha256:{}", hex_sha256(net_xml.as_bytes()));
    let generated = stub_weld_manifest_json(&network, &digest).expect("manifest json");
    let on_disk = fs::read_to_string(manifest_path()).expect("read manifest");
    assert_eq!(
        on_disk.replace("\r\n", "\n").trim_end(),
        generated.trim_end(),
        "manifest drifted: rescan the pinned source and regenerate"
    );
}
