use std::{fs, path::PathBuf, process::Command};

use laneflow_lust_converter::{
    Error, ExactDecimal, InfeasibilityMechanism, LUST_COMMIT, LUST_FRAME_ID, PINNED_SOURCE_FILES,
    ReportSource, TopologyConvertOptions, convert_static_from_xml_with_due,
    convert_topology_from_xml_with_tll_and_vtypes,
    convert_topology_from_xml_with_tll_and_vtypes_and_source, hex_sha256, parse_due_routes_xml,
    parse_sumo_network_xml, parse_vtypes_xml, select_passenger_vtypes, verify_source_dir,
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
    let counts = &artifacts.counts;
    assert_eq!(counts.lane_edges, 5);
    assert_eq!(counts.junctions, 1);
    assert_eq!(counts.movements, 2);
    assert_eq!(counts.maneuver_paths, 2);
    assert_eq!(counts.signal_controllers, 1);
    assert_eq!(counts.stop_lines, 1);
    assert_eq!(counts.vehicle_profiles, 6);
    assert!(counts.parking_registry_empty);
    // LFCA 是 FlatBuffers 二进制；声明键以字符串表形式内嵌，可按字节检索。
    let lfca = &artifacts.network_lfca;
    assert!(lfca.windows(4).any(|w| w == b"LFCA"), "missing LFCA magic");
    assert!(contains_bytes(lfca, b"west_0"));
    assert!(contains_bytes(lfca, b"group-0"));
    assert!(contains_bytes(lfca, b"stop:west"));
    assert!(contains_bytes(lfca, b"passenger1"));
    assert!(contains_bytes(lfca, b"passenger5"));
    assert!(contains_bytes(lfca, LUST_FRAME_ID.as_bytes()));
    assert!(!contains_bytes(lfca, b"bus"));
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
    assert_eq!(first.network_lfca, second.network_lfca);
    assert_eq!(first.counts, second.counts);
}

#[test]
fn fixture_due_parse_keeps_source_ordinals() {
    let vehicles = parse_due_routes_xml(&fixture_due0_xml(), 0).expect("parse due0");
    assert_eq!(vehicles.len(), 6);
    assert_eq!(vehicles[0].id, "early");
    assert_eq!(vehicles[0].source_file_ordinal, 0);
    assert_eq!(vehicles[0].source_vehicle_ordinal, 0);
    assert_eq!(vehicles[5].id, "late");
    assert_eq!(vehicles[5].source_vehicle_ordinal, 5);
}

#[test]
fn fixture_due_routes_and_population_round_trip() {
    let artifacts = convert_static_from_xml_with_due(
        &fixture_net_xml(),
        &fixture_tll_xml(),
        &fixture_vtypes_xml(),
        [
            &fixture_due0_xml(),
            &fixture_due1_xml(),
            &fixture_due2_xml(),
        ],
        &TopologyConvertOptions::default(),
    )
    .expect("static+due convert");
    assert_eq!(artifacts.population_record_count, 3);
    assert_eq!(artifacts.route_count, 2);
    let lfca = &artifacts.topology.network_lfca;
    assert!(contains_bytes(lfca, b"west_0"));
    assert!(contains_bytes(lfca, b"int:J_0_0") || contains_bytes(lfca, b"int:J_1_0"));
    let routes = String::from_utf8_lossy(&artifacts.routes_toml);
    assert!(routes.contains("route-0"));
    assert!(routes.contains("population_rank = 0"));
    assert!(routes.contains("west-east-a") || routes.contains("west-south-b"));
    assert!(routes.contains("selected_count = 3"));
    assert!(!routes.contains("bus-in-window"));
}

#[test]
fn fixture_due_population_is_byte_deterministic() {
    let options = TopologyConvertOptions::default();
    let first = convert_static_from_xml_with_due(
        &fixture_net_xml(),
        &fixture_tll_xml(),
        &fixture_vtypes_xml(),
        [
            &fixture_due0_xml(),
            &fixture_due1_xml(),
            &fixture_due2_xml(),
        ],
        &options,
    )
    .expect("first");
    let second = convert_static_from_xml_with_due(
        &fixture_net_xml(),
        &fixture_tll_xml(),
        &fixture_vtypes_xml(),
        [
            &fixture_due0_xml(),
            &fixture_due1_xml(),
            &fixture_due2_xml(),
        ],
        &options,
    )
    .expect("second");
    assert_eq!(first.topology.network_lfca, second.topology.network_lfca);
    assert_eq!(first.routes_toml, second.routes_toml);
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

/// 真实 LuST 全网验收（G1 锚点：2026-09-28 补充记录——验收重划为「converter
/// 交付 + 确定性 fail-closed 诊断清单」；端到端编译依赖未来的路口级 maneuver
/// 几何合成，另立 G1）。本测试锁定：source 锚点、诊断清单两次运行逐字节一致、
/// 总量与机制分布、锚点条目；清单文件落盘 `target/issue253-infeasible-survey.md`
/// 作为验收交付物。
///
/// 当前状态（#253 R1 焊接门控）：`STUB_WELD_MAX_DISPLACEMENT_M = 0.06` 下
/// pinned 基线 84 条 stub 中 48 条焊接位移 6–46 cm 越界、fail-closed，诊断转换
/// 在 normalize 阶段即中止（实测首错 `--30260_0`→`-31938_0` @ 0.1000 m），
/// 下列锁定数字（R1 前基线）与清单落盘均待 G1 修订阈值后重锁；
/// fail-fast 锚点断言（sumo::-1000_2_0）同理漂移。
#[test]
#[ignore = "requires LUST_SOURCE_DIR at c4bd5bd3; 待 G1 修订 STUB_WELD_MAX_DISPLACEMENT_M（48 条 6–46 cm 焊接越界 fail-closed，见 target/issue253-stub-weld-evidence.md）后重锁"]
fn full_lust_net_topology_matches_external_lane_anchor() {
    let source_dir = std::env::var("LUST_SOURCE_DIR").expect("LUST_SOURCE_DIR");
    let root = PathBuf::from(source_dir);
    let net_xml = fs::read_to_string(root.join("scenario/lust.net.xml")).expect("read net");
    let tll_xml = fs::read_to_string(root.join("scenario/tll.static.xml")).expect("read tll");
    let vtypes_xml = fs::read_to_string(root.join("scenario/vtypes.add.xml")).expect("read vtypes");
    // R2：验收入口对全部实际消费字节绑定 pinned 基线——net 经下方 report 头
    // 比对，tll/vtypes 在此直接对消费字节断言 §2.2 pinned digest；注释改动或
    // 任何非 pinned 字节即测试失败。断言位于转换之前，R1 门控下转换在
    // normalize 中止也不影响本断言先行执行。
    for (relative_path, bytes) in [
        ("scenario/tll.static.xml", tll_xml.as_bytes()),
        ("scenario/vtypes.add.xml", vtypes_xml.as_bytes()),
    ] {
        let pinned = PINNED_SOURCE_FILES
            .iter()
            .find(|file| file.relative_path == relative_path)
            .expect("pinned entry present");
        assert_eq!(
            hex_sha256(bytes),
            pinned.sha256_hex,
            "{relative_path} bytes must equal the pinned §2.2 baseline"
        );
    }
    let network = parse_sumo_network_xml(&net_xml).expect("parse lust.net.xml");
    assert!(network.location.matches_lust_anchors());
    assert_eq!(network.external_edge_count(), 5_779);
    assert_eq!(network.external_lane_count(), 8_622);
    assert_eq!(network.connections.len(), 30_051);
    assert_eq!(network.net_tl_logic_ids().len(), 201);

    let options = TopologyConvertOptions {
        require_lust_location_anchors: true,
        emit_infeasibility_report: true,
        ..TopologyConvertOptions::default()
    };
    // R2 第三轮：正式诊断入口——来源验证（前置测试
    // full_lust_source_verification_precedes_diagnostic_convert）之后，以
    // verified = true + 消费字节摘要走显式来源声明入口；通用 XML 入口保持
    // 未验证标注（旁路测试覆盖）。
    let report_source = ReportSource {
        net_digest: Some(format!("sha256:{}", hex_sha256(net_xml.as_bytes()))),
        verified: true,
    };
    let convert = |options: &TopologyConvertOptions| {
        convert_topology_from_xml_with_tll_and_vtypes_and_source(
            &net_xml,
            &tll_xml,
            &vtypes_xml,
            options,
            report_source.clone(),
        )
    };
    let first = convert(&options).expect("first diagnostic-report conversion");
    let second = convert(&options).expect("second diagnostic-report conversion");
    let report = first
        .infeasibility_report
        .as_ref()
        .expect("diagnostic report deliverable");
    assert!(
        first.network_lfca.is_empty() && second.network_lfca.is_empty(),
        "diagnostic mode must not emit network.lfca"
    );
    assert_eq!(
        report.rendered,
        second
            .infeasibility_report
            .as_ref()
            .expect("second report")
            .rendered,
        "diagnostic report must be byte-deterministic across runs"
    );
    // R2：验收输入即 pinned 基线字节，清单头必须如实标注「一致」+「verify-source
    // 已通过」（摘要取实际转换字节；两种声明互不越权）。
    assert!(
        report
            .rendered
            .contains("与 pinned 比对：一致（输入为 pinned 基线字节）"),
        "pinned 基线输入的清单头必须标注一致"
    );
    assert!(
        report
            .rendered
            .contains("来源校验：verify-source 已通过（checkout revision + pinned digest）"),
        "verified 正式入口的清单头必须标注 verify-source 已通过"
    );
    // 点状 stub 内边被移除并焊接（pinned 数据 shape 端点距 < 0.5 m 的共 84 条，
    // 实测锁定），其余 lane 全数保留。
    assert_eq!(first.counts.dropped_point_stub_edges, 84);
    assert_eq!(
        first.counts.lane_edges + first.counts.dropped_point_stub_edges,
        network.lanes.len() as u64
    );

    // 锁定诊断清单（pinned c4bd5bd3 基线；数字随源数据或发射语义变化而更新）。
    assert_eq!(report.total(), 8_984);
    assert_eq!(report.internal_count(), 8_236);
    assert_eq!(report.external_count(), 748);
    assert_eq!(report.junction_count(), 1_854);
    assert_eq!(
        report.internal_mechanism_count(InfeasibilityMechanism::BoundaryClamp),
        5_102
    );
    assert_eq!(
        report.internal_mechanism_count(InfeasibilityMechanism::InteriorCurvature),
        2_695
    );
    assert_eq!(
        report.internal_mechanism_count(InfeasibilityMechanism::HardCornerFillet),
        439
    );
    let anchor_first = &report.entries[0].lane_id;
    let anchor_second = &report.entries[1].lane_id;
    assert_eq!(anchor_first, "sumo::-1000_2_0", "first infeasible lane");
    assert_eq!(anchor_second, "sumo::-1000_7_0", "second infeasible lane");

    // 默认 fail-fast 行为：同一基线下确定性中止于首条不可行 lane，
    // 错误消息逐字节锁定（诊断模式不改变默认路径）。
    let fail_fast = convert_topology_from_xml_with_tll_and_vtypes(
        &net_xml,
        &tll_xml,
        &vtypes_xml,
        &TopologyConvertOptions {
            require_lust_location_anchors: true,
            ..TopologyConvertOptions::default()
        },
    )
    .expect_err("default mode must fail fast at the first infeasible lane");
    let message = fail_fast.to_string();
    assert!(
        message.contains("sumo::-1000_2_0") && message.contains("span 2/2"),
        "fail-fast point drifted: {message}"
    );

    // 验收交付物：确定性清单落盘（与测试断言同源，逐字节稳定）。
    let out_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("issue253-infeasible-survey.md");
    fs::write(&out_path, &report.rendered).expect("write diagnostic survey file");
}

/// R2 第三轮：正式诊断入口的来源验证前置段——verify_source_dir 先验 checkout
/// revision + 全部 §2.2 pinned digest；三份诊断输入字节在转换前完成消费时
/// 绑定；同字节 + 错误 HEAD/无仓库必须拒绝。本段不依赖 G1（R1 门控），与
/// 「转换 + 清单锁定」段解耦，现在就必须绿。
#[test]
#[ignore = "requires LUST_SOURCE_DIR at c4bd5bd3"]
fn full_lust_source_verification_precedes_diagnostic_convert() {
    let source_dir = std::env::var("LUST_SOURCE_DIR").expect("LUST_SOURCE_DIR");
    let root = PathBuf::from(&source_dir);

    // 1) 正式验证通过：revision + pinned digest（含 net/tll/vtypes 三份输入）。
    let verified =
        verify_source_dir(&root).expect("verify_source_dir must accept the pinned checkout");

    // 2) 转换前绑定：三份实际消费字节的消费时重哈希与验证记录一致（TOCTOU 闭合）。
    for relative_path in [
        "scenario/lust.net.xml",
        "scenario/tll.static.xml",
        "scenario/vtypes.add.xml",
    ] {
        let record = verified
            .files
            .iter()
            .find(|file| file.relative_path == relative_path)
            .expect("verification record present");
        let bytes = fs::read(&record.absolute_path).expect("read verified file");
        assert_eq!(
            hex_sha256(&bytes),
            record.sha256_hex,
            "{relative_path} changed after verification"
        );
    }

    // 3) 同字节 + 错误 HEAD：tmp git 仓摆 pinned 原字节，revision ≠ LUST_COMMIT 必拒。
    //    探针放系统临时目录——worktree/target 本身在 git 仓内，git 会向上找到
    //    父仓 HEAD，使「无仓库」用例失效。
    let probe = std::env::temp_dir().join(format!("lust-verify-probe-{}", std::process::id()));
    let _ = fs::remove_dir_all(&probe);
    for pinned in PINNED_SOURCE_FILES {
        let to = probe.join(pinned.relative_path);
        fs::create_dir_all(to.parent().expect("parent")).expect("create probe dir");
        fs::copy(root.join(pinned.relative_path), &to).expect("copy pinned file");
    }
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(&probe)
            .args(args)
            .output()
            .expect("run git")
    };
    match Command::new("git")
        .arg("-C")
        .arg(&probe)
        .arg("init")
        .output()
    {
        Ok(output) if output.status.success() => {}
        _ => {
            // 无 git 主机上跳过拒绝段（机制由 verify.rs 单测覆盖）。
            let _ = fs::remove_dir_all(&probe);
            return;
        }
    }
    assert!(git(&["add", "."]).status.success());
    assert!(
        git(&[
            "-c",
            "user.name=lust-verify-probe",
            "-c",
            "user.email=probe@example.invalid",
            "commit",
            "-m",
            "probe",
        ])
        .status
        .success()
    );
    let wrong_head = verify_source_dir(&probe).expect_err("wrong HEAD must be rejected");
    match wrong_head {
        Error::SourceRevisionMismatch { expected, actual } => {
            assert_eq!(expected, LUST_COMMIT);
            assert_ne!(actual, LUST_COMMIT);
        }
        other => panic!("unexpected error: {other}"),
    }

    // 4) 无仓库：同字节、无 .git，revision 不可知必拒（改名而非删除：.git 内
    //    对象只读，Windows 的 remove_dir_all 会失败）。
    fs::rename(probe.join(".git"), probe.join(".git-disabled")).expect("disable .git");
    let no_repo = verify_source_dir(&probe).expect_err("missing repository must be rejected");
    match no_repo {
        Error::SourceRevisionUnknown { .. } => {}
        other => panic!("unexpected error: {other}"),
    }
    let _ = fs::remove_dir_all(&probe);
}

/// R3 回归：诊断收集为每次转换局部状态——fail-fast 尝试不残留，连续两次
/// 诊断转换条目互不相偷、逐字节一致。
#[test]
fn diagnostics_are_per_conversion_without_cross_run_pollution() {
    let net = fixture_infeasible_net_xml();
    // fail-fast 先行：报错且不留下任何诊断状态。
    let fail_fast = convert_topology_from_xml_with_tll_and_vtypes(
        &net,
        &fixture_tll_xml(),
        &fixture_vtypes_xml(),
        &TopologyConvertOptions::default(),
    );
    assert!(
        fail_fast.is_err(),
        "fixture 内车道不可行，fail-fast 必须报错"
    );

    let options = TopologyConvertOptions {
        emit_infeasibility_report: true,
        ..TopologyConvertOptions::default()
    };
    let first = convert_topology_from_xml_with_tll_and_vtypes(
        &net,
        &fixture_tll_xml(),
        &fixture_vtypes_xml(),
        &options,
    )
    .expect("first diagnostics conversion");
    let second = convert_topology_from_xml_with_tll_and_vtypes(
        &net,
        &fixture_tll_xml(),
        &fixture_vtypes_xml(),
        &options,
    )
    .expect("second diagnostics conversion");
    let first_report = first.infeasibility_report.expect("first report");
    let second_report = second.infeasibility_report.expect("second report");
    assert_eq!(
        first_report, second_report,
        "连续两次诊断转换必须互不相偷、逐字节一致"
    );
    assert_eq!(
        first_report.total(),
        1,
        "fixture 恰一条不可行内车道（硬折角倒圆）"
    );
    assert_eq!(first_report.entries[0].lane_id, "sumo::J_1_0");
}

/// R2 回归：清单头对**实际转换输入的字节**求摘要并如实标注校验状态；
/// 改输入字节（不改语义结构）即摘要变化，且不出现与输入不符的固定声明。
#[test]
fn diagnostic_report_tracks_actual_input_bytes() {
    let net = fixture_infeasible_net_xml();
    let options = TopologyConvertOptions {
        emit_infeasibility_report: true,
        ..TopologyConvertOptions::default()
    };
    let report = |net_xml: &str| {
        convert_topology_from_xml_with_tll_and_vtypes(
            net_xml,
            &fixture_tll_xml(),
            &fixture_vtypes_xml(),
            &options,
        )
        .expect("diagnostics conversion")
        .infeasibility_report
        .expect("report")
    };
    let extract_digest = |rendered: &str| {
        rendered
            .split_once("本次转换输入 net XML 摘要：`")
            .and_then(|(_, tail)| tail.split_once("`"))
            .map(|(digest, _)| digest.to_owned())
            .expect("digest line present")
    };

    let first = report(&net);
    let header = &first.rendered;
    assert!(
        header.contains("不一致（输入不是 pinned 基线字节）"),
        "旁路输入不得声称 pinned 一致: {header}"
    );
    assert!(
        header.contains("未执行独立 verify-source"),
        "旁路输入不得声称已校验: {header}"
    );
    assert!(
        !header.contains("pinned c4bd5bd3 原样"),
        "不得出现与输入不符的固定基线声明: {header}"
    );

    // 改输入字节（speed 13.89 -> 13.90）：摘要随之变化，清单其余结构不变。
    let modified = net.replacen("13.89", "13.90", 1);
    assert_ne!(net, modified);
    let second = report(&modified);
    assert_ne!(
        extract_digest(&first.rendered),
        extract_digest(&second.rendered),
        "输入字节变化必须反映为摘要变化"
    );
    assert_eq!(first.entries, second.entries);
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn fixture_net_xml() -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal/t-junction.net.xml"),
    )
    .expect("read net fixture")
}

fn fixture_tll_xml() -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal/t-junction.tll.xml"),
    )
    .expect("read tll fixture")
}

fn fixture_infeasible_net_xml() -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/minimal/infeasible-kink.net.xml"),
    )
    .expect("read infeasible-kink net fixture")
}

fn fixture_vtypes_xml() -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal/vtypes.add.xml"),
    )
    .expect("read vtypes fixture")
}

fn fixture_due0_xml() -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/minimal/local.static.0.rou.xml"),
    )
    .expect("read due0 fixture")
}

fn fixture_due1_xml() -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/minimal/local.static.1.rou.xml"),
    )
    .expect("read due1 fixture")
}

fn fixture_due2_xml() -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/minimal/local.static.2.rou.xml"),
    )
    .expect("read due2 fixture")
}

trait FromStrChecked {
    fn from_str_checked(input: &str) -> Self;
}

impl FromStrChecked for ExactDecimal {
    fn from_str_checked(input: &str) -> Self {
        input.parse().expect("decimal")
    }
}
