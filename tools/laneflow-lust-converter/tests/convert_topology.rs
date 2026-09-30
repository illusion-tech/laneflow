use std::{fs, path::PathBuf, process::Command};

use laneflow_lust_converter::{
    BudgetOutcome, Error, ExactDecimal, InfeasibilityMechanism, LUST_COMMIT, LUST_FRAME_ID,
    PINNED_SOURCE_FILES, TopologyConvertOptions, convert_static_from_xml_with_due,
    convert_topology_from_verified_lust_inputs, convert_topology_from_xml_with_tll_and_vtypes,
    convert_topology_from_xml_with_tll_and_vtypes_and_source, parse_due_routes_xml,
    parse_sumo_network_xml, parse_vtypes_xml, prepare_verified_lust_inputs,
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
    let routes = String::from_utf8_lossy(
        artifacts
            .routes_toml
            .as_ref()
            .expect("fail-fast mode delivers routes.toml"),
    );
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
/// 当前状态（G1 已确认，2026-09-30，issuecomment-5901846033 §5）：位移阈值
/// 0.5 m 启用 + 拒绝处置改为「不焊接、保留原始连接、记录诊断」——80 条
/// 焊接、4 条共享入口破坏（join 间隙 0.20–0.42 m > 5 mm）拒绝保留，诊断
/// 转换全量跑通；下列数字为 @2 规则重锁基线（清单两次运行逐字节一致）。
#[test]
#[ignore = "requires LUST_SOURCE_DIR at c4bd5bd3; G1 0.5m 重锁基线（rule stub-weld/g1-six-cond@2）"]
fn full_lust_net_topology_matches_external_lane_anchor() {
    let source_dir = std::env::var("LUST_SOURCE_DIR").expect("LUST_SOURCE_DIR");
    let root = PathBuf::from(source_dir);
    // R2 第四轮：正式诊断入口的准备函数——verify_source_dir（revision + 全部
    // pinned digest）→ 三份输入消费时重哈希绑定（TOCTOU 闭合）→ verified
    // 来源声明（crate 内受控构造器，调用方无法自行声明已验证）。本测试单跑
    // 时此准备段照样执行：验证前置内建于主生成路径，不依赖任何其他测试。
    let prepared = prepare_verified_lust_inputs(&root).expect("prepare verified Lust inputs");
    let net_xml = prepared.net_xml().to_owned();
    let tll_xml = prepared.tll_xml().to_owned();
    let vtypes_xml = prepared.vtypes_xml().to_owned();
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
    // 同源入口（#253 R8 残留缺口）：字节与验证记录内聚不可错配，内部委托
    // 组合入口且消费时重算绑定仍会执行（双保险）；通用 XML 入口保持未验证
    // 标注（旁路与 tamper 回归覆盖）。
    let convert = |options: &TopologyConvertOptions| {
        convert_topology_from_verified_lust_inputs(&prepared, options)
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
    // 验收证据逐字节比对（evidence/README.md 的锁定方式）：随仓库提交的清单
    // 必须与 pinned 基线重扫结果一致（归一化换行，防 CRLF checkout 干扰）。
    let evidence = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("evidence/lust-infeasible-survey.md"),
    )
    .expect("read committed evidence survey");
    assert_eq!(
        report.rendered,
        evidence.replace("\r\n", "\n"),
        "committed evidence survey drifted from the pinned baseline rescan"
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
    // 点状 stub 处置（G1 @2 实测锁定）：84 条候选中 80 条焊接移除；4 条共享
    // 入口破坏拒绝保留，stub 作为正常 lane 保留（入 24,495 条 lane_edges），
    // 80 + 24,495 = 24,575 = SUMO lane 总数。
    assert_eq!(first.counts.dropped_point_stub_edges, 80);
    assert_eq!(
        first.counts.lane_edges + first.counts.dropped_point_stub_edges,
        network.lanes.len() as u64
    );

    // 锁定诊断清单（pinned c4bd5bd3 基线，G1 @2 重锁；数字随源数据或发射
    // 语义变化而更新；清单两次运行逐字节一致）。
    assert_eq!(report.total(), 8_230);
    assert_eq!(report.internal_count(), 7_533);
    assert_eq!(report.external_count(), 697);
    assert_eq!(report.junction_count(), 1_854);
    assert_eq!(
        report.internal_mechanism_count(InfeasibilityMechanism::BoundaryClamp),
        4_253
    );
    assert_eq!(
        report.internal_mechanism_count(InfeasibilityMechanism::InteriorCurvature),
        2_777
    );
    assert_eq!(
        report.internal_mechanism_count(InfeasibilityMechanism::HardCornerFillet),
        503
    );
    // 发射预算裁决分布（G1 重新验收口径 + R9 审计：curvature is infeasible
    // 均为 0.105 m 质量目标预检 → 预检拒绝；已证预算冲突审计后无发射点为 0；
    // 4 条保留 stub 亦为预检拒绝）。
    assert_eq!(report.outcome_count(BudgetOutcome::SamplerExhausted), 5_613);
    assert_eq!(report.outcome_count(BudgetOutcome::PreCheckRejected), 2_599);
    assert_eq!(report.outcome_count(BudgetOutcome::ProvenBudgetConflict), 0);
    assert_eq!(report.outcome_count(BudgetOutcome::NotBudget), 18);
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

    // G3 验收证据：与提交仓库的 evidence/lust-infeasible-survey.md 逐字节
    // 一致（发射语义变化时按 evidence/README.md 流程双跑重锁并同步更新）。
    let evidence_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("evidence/lust-infeasible-survey.md");
    let evidence = fs::read_to_string(&evidence_path).expect("read committed survey evidence");
    assert_eq!(
        evidence.replace("\r\n", "\n").trim_end(),
        report.rendered.trim_end(),
        "survey drifted from committed evidence: regenerate per evidence/README.md and update both copies"
    );
}

/// L1 回归：诊断模式绕过未完成的 lane-level route 展开——population 选取
/// 保留（计数健康事实），routes.toml 不产出；fail-fast 路径仍严格展开。
#[test]
fn diagnostic_mode_skips_route_expansion_but_keeps_population() {
    // x(2 车道) → y 仅 (0,0) → z 仅 (1,1)：车道连续走法不存在（边内换道
    // 语义缺口的最小复现），fail-fast 展开必败。
    let net = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="x" from="X" to="J"><lane id="x_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/><lane id="x_1" index="1" speed="13.89" length="20.00" shape="6786.88,5730.52 6806.88,5730.52"/></edge>
  <edge id="y" from="J" to="K"><lane id="y_0" index="0" speed="13.89" length="20.00" shape="6816.88,5727.52 6836.88,5727.52"/></edge>
  <edge id="z" from="K" to="Z"><lane id="z_0" index="0" speed="13.89" length="20.00" shape="6846.88,5727.52 6866.88,5727.52"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="10.00" shape="6806.88,5727.52 6816.88,5727.52"/></edge>
  <edge id=":K_0" function="internal"><lane id=":K_0_0" index="0" speed="13.89" length="10.00" shape="6836.88,5727.52 6846.88,5727.52"/></edge>
  <junction id="J" type="priority" intLanes=":J_0_0"/>
  <junction id="K" type="priority" intLanes=":K_0_0"/>
  <connection from="x" to="y" fromLane="0" toLane="0" via=":J_0_0" tl="J" linkIndex="0"/>
  <connection from=":J_0" to="y" fromLane="0" toLane="0"/>
  <connection from="y" to="z" fromLane="0" toLane="0" via=":K_0_0"/>
  <connection from=":K_0" to="z" fromLane="0" toLane="0"/>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="Gr"/>
  </tlLogic>
</net>"#;
    let due = |id: &str| {
        format!(
            r#"<routes>
  <vehicle id="{id}" type="passenger1" depart="28800">
    <route edges="x y z"/>
  </vehicle>
</routes>"#
        )
    };
    // fail-fast：展开失败（routes 构建先于信号闭包）。
    let fail_fast = convert_static_from_xml_with_due(
        net,
        &fixture_tll_xml(),
        &fixture_vtypes_xml(),
        [&due("v0"), &fixture_due1_xml(), &fixture_due2_xml()],
        &TopologyConvertOptions::default(),
    );
    assert!(
        fail_fast.is_err(),
        "fail-fast 路径仍须严格展开（边内换道语义缺口下必败）"
    );
    // 诊断模式：绕过展开，population 计数保留，不产 routes.toml。
    let artifacts = convert_static_from_xml_with_due(
        net,
        &fixture_tll_xml(),
        &fixture_vtypes_xml(),
        [&due("v0"), &fixture_due1_xml(), &fixture_due2_xml()],
        &TopologyConvertOptions {
            emit_infeasibility_report: true,
            ..TopologyConvertOptions::default()
        },
    )
    .expect("diagnostic mode bypasses route expansion");
    assert_eq!(artifacts.population_record_count, 1);
    assert!(artifacts.routes_toml.is_none(), "诊断模式不产 routes.toml");
    assert!(artifacts.topology.infeasibility_report.is_some());
}

/// R8 回归：通用 XML 入口（未验证来源）+ 几何可焊形态 stub → 0.5 m 例外
/// 不生效（BlockedOutOfDomain），stub 保留走普通穿越，处置记录入附录。
#[test]
fn generic_xml_entry_stub_weld_blocked_outside_approved_domain() {
    let net = r#"<?xml version="1.0" encoding="UTF-8"?>
<net>
  <location netOffset="-285448.66,-5492398.13" convBoundary="0.00,0.00,13613.76,11455.04"/>
  <edge id="west" from="W" to="J"><lane id="west_0" index="0" speed="13.89" length="20.00" shape="6786.88,5727.52 6806.88,5727.52"/></edge>
  <edge id="east" from="J" to="E"><lane id="east_0" index="0" speed="13.89" length="20.00" shape="6806.90,5727.53 6826.90,5727.53"/></edge>
  <edge id=":J_0" function="internal"><lane id=":J_0_0" index="0" speed="13.89" length="0.40" shape="6806.88,5727.52 6807.18,5727.60"/></edge>
  <junction id="J" type="priority" intLanes=":J_0_0"/>
  <connection from="west" to="east" fromLane="0" toLane="0" via=":J_0_0" tl="J" linkIndex="0"/>
  <connection from=":J_0" to="east" fromLane="0" toLane="0"/>
  <tlLogic id="J" type="static" programID="1" offset="0">
    <phase duration="31" state="Gr"/>
    <phase duration="4" state="yr"/>
    <phase duration="31" state="rG"/>
  </tlLogic>
</net>"#;
    let artifacts = convert_topology_from_xml_with_tll_and_vtypes(
        net,
        &fixture_tll_xml(),
        &fixture_vtypes_xml(),
        &TopologyConvertOptions {
            require_lust_location_anchors: true,
            emit_infeasibility_report: true,
            ..TopologyConvertOptions::default()
        },
    )
    .expect("generic entry normalizes with the stub retained");
    let report = artifacts.infeasibility_report.expect("report");
    // 附录记录 BlockedOutOfDomain；stub 保留（未进 dropped 计数）。
    assert!(
        report
            .rendered
            .contains("## 点状 stub 删焊处置（已归一化 / 拒绝保留）"),
        "weld disposition appendix missing"
    );
    assert!(
        report.rendered.contains("blocked-out-of-domain"),
        "blocked disposition row missing: {}",
        &report.rendered[report.rendered.len().saturating_sub(2500)..]
    );
    assert_eq!(artifacts.counts.dropped_point_stub_edges, 0);
}

/// L1 端到端验收（真实 pinned）：`convert` CLI 在 pinned 输入上首次完整
/// 跑通——诊断交付物 survey（字节 = evidence）、manifest 单配对 survey、
/// report 含 K6/K7/K4a 健康事实（parking 175 / phase 1,298 / 声明臂
/// -13968 缺 index 9）、population 计数保留、不产 network.lfca /
/// lust-static.tar / routes.toml。
#[test]
#[ignore = "requires LUST_SOURCE_DIR at c4bd5bd3"]
fn full_lust_convert_cli_end_to_end() {
    let source_dir = std::env::var("LUST_SOURCE_DIR").expect("LUST_SOURCE_DIR");
    let root = std::env::temp_dir().join(format!("lust-cli-e2e-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let config_path = root.join("config.toml");
    fs::create_dir_all(&root).expect("create temp");
    fs::write(
        &config_path,
        format!(
            "source_dir = {source_dir:?}
output_dir = {:?}
converter_commit = \"e7004fe7000000000000000000000000000000000\"
",
            root.join("out").to_string_lossy().replace('\\', "/"),
        ),
    )
    .expect("write config");
    let paths = laneflow_lust_converter::convert(&config_path).expect("CLI convert");
    let read =
        |path: &std::path::Path| -> String { fs::read_to_string(path).expect("read output") };
    // 诊断交付物：survey 字节 = evidence。
    let survey = fs::read(&paths.infeasibility_survey).expect("read survey");
    let evidence = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("evidence/lust-infeasible-survey.md"),
    )
    .expect("read evidence");
    assert_eq!(survey, evidence, "survey 必须与 evidence 逐字节一致");
    // 不产出排除产物。
    assert!(paths.network_lfca.is_none());
    assert!(paths.static_tar.is_none());
    assert!(paths.routes.is_none());
    // manifest 单配对 survey。
    let manifest = read(&paths.manifest);
    assert!(
        manifest.contains("[files.\"issue253-infeasible-survey.md\"]"),
        "{manifest}"
    );
    assert!(!manifest.contains("network.lfca"), "{manifest}");
    assert!(!manifest.contains("routes.toml"), "{manifest}");
    assert!(
        manifest.contains("population_records = 10000"),
        "{manifest}"
    );
    // report 健康事实（K6/K7/K4a pinned 断言）。
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&paths.conversion_report).expect("read report"))
            .expect("report json");
    assert_eq!(report["health"]["parkingPolygonCount"], 175);
    assert_eq!(report["normalization"]["signalPhaseCount"], 1298);
    let arms = report["health"]["unclaimedSignalArms"]
        .as_array()
        .expect("arms");
    assert_eq!(arms.len(), 1);
    assert_eq!(arms[0]["controllerId"], "-13968");
    assert_eq!(arms[0]["missingLinkIndices"], serde_json::json!([9]));
    assert!(report["digests"].get("networkLfca").is_none());
    assert!(report["digests"].get("routesToml").is_none());
    let _ = fs::remove_dir_all(&root);
}

/// R8 残留缺口回归（真实 pinned）：prepare 后改动 net/tll 字节、沿用旧
/// verified 记录走组合入口——消费时重算绑定必须 fail-closed（解析/转换
/// 之前，不降级、不静默继续）。
#[test]
#[ignore = "requires LUST_SOURCE_DIR at c4bd5bd3"]
fn verified_record_with_tampered_bytes_fails_closed() {
    let source_dir = std::env::var("LUST_SOURCE_DIR").expect("LUST_SOURCE_DIR");
    let root = PathBuf::from(source_dir);
    let prepared = prepare_verified_lust_inputs(&root).expect("prepare");
    let options = TopologyConvertOptions {
        require_lust_location_anchors: true,
        ..TopologyConvertOptions::default()
    };
    // net 追加换行：记录摘要与本次消费字节失配（① 记录一致性检查）。
    let mut net_xml = prepared.net_xml().to_owned();
    net_xml.push('\n');
    let error = convert_topology_from_xml_with_tll_and_vtypes_and_source(
        &net_xml,
        prepared.tll_xml(),
        prepared.vtypes_xml(),
        &options,
        prepared.report_source().clone(),
    )
    .expect_err("stale record with tampered net bytes must fail closed");
    match error {
        Error::SourceChangedAfterVerification { relative_path, .. } => {
            assert_eq!(relative_path, "scenario/lust.net.xml");
        }
        other => panic!("unexpected error: {other}"),
    }
    // tll 追加换行（net 与记录均未动）：pinned 条目命中检查拒绝（②）。
    let mut tll_xml = prepared.tll_xml().to_owned();
    tll_xml.push('\n');
    let error = convert_topology_from_xml_with_tll_and_vtypes_and_source(
        prepared.net_xml(),
        &tll_xml,
        prepared.vtypes_xml(),
        &options,
        prepared.report_source().clone(),
    )
    .expect_err("tampered tll bytes must fail closed");
    match error {
        Error::SourceChangedAfterVerification { relative_path, .. } => {
            assert_eq!(relative_path, "scenario/tll.static.xml");
        }
        other => panic!("unexpected error: {other}"),
    }
    // vtypes 追加换行：对称分支（第七轮非阻断建议）。
    let mut vtypes_xml = prepared.vtypes_xml().to_owned();
    vtypes_xml.push('\n');
    let error = convert_topology_from_xml_with_tll_and_vtypes_and_source(
        prepared.net_xml(),
        prepared.tll_xml(),
        &vtypes_xml,
        &options,
        prepared.report_source().clone(),
    )
    .expect_err("tampered vtypes bytes must fail closed");
    match error {
        Error::SourceChangedAfterVerification { relative_path, .. } => {
            assert_eq!(relative_path, "scenario/vtypes.add.xml");
        }
        other => panic!("unexpected error: {other}"),
    }
}

/// R2 第四轮：正式诊断入口的来源验证前置段——verify_source_dir 先验 checkout
/// revision + 全部 §2.2 pinned digest；三份诊断输入字节在转换前完成消费时
/// 绑定；同字节 + 错误 HEAD/无仓库必须拒绝。本段不依赖 G1（R1 门控），与
/// 「转换 + 清单锁定」段解耦，现在就必须绿。
#[test]
#[ignore = "requires LUST_SOURCE_DIR at c4bd5bd3"]
fn full_lust_source_verification_precedes_diagnostic_convert() {
    // R2 第四轮：本测试断言的是主测试调用的**同一个准备函数**
    // （prepare_verified_lust_inputs）——验证（revision + 全部 pinned digest）、
    // 消费时绑定（TOCTOU 闭合）与 verified 来源声明都在准备函数内部完成，
    // 任何拒绝天然发生在 normalize/转换之前。
    let source_dir = std::env::var("LUST_SOURCE_DIR").expect("LUST_SOURCE_DIR");
    let root = PathBuf::from(&source_dir);

    // 1) pinned checkout：准备函数成功；verified 声明 + net 摘要绑定 pinned。
    let prepared =
        prepare_verified_lust_inputs(&root).expect("prepare must accept the pinned checkout");
    assert!(
        prepared.report_source().is_verified(),
        "prepared source must be verified"
    );
    let pinned_net = PINNED_SOURCE_FILES
        .iter()
        .find(|file| file.relative_path == "scenario/lust.net.xml")
        .expect("pinned net entry");
    assert_eq!(
        prepared.report_source().net_digest(),
        Some(format!("sha256:{}", pinned_net.sha256_hex).as_str()),
        "net digest must bind the consumed pinned bytes"
    );
    assert!(!prepared.net_xml().is_empty() && !prepared.tll_xml().is_empty());

    // 2) 同字节 + 错误 HEAD：tmp git 仓摆 pinned 原字节，revision ≠ LUST_COMMIT
    //    在准备函数阶段必拒（早于任何 normalize/转换）。
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
    let wrong_head = prepare_verified_lust_inputs(&probe).expect_err("wrong HEAD must be rejected");
    match wrong_head {
        Error::SourceRevisionMismatch { expected, actual } => {
            assert_eq!(expected, LUST_COMMIT);
            assert_ne!(actual, LUST_COMMIT);
        }
        other => panic!("unexpected error: {other}"),
    }

    // 3) 无仓库：同字节、无 .git，revision 不可知在准备函数阶段必拒（改名而非
    //    删除：.git 内对象只读，Windows 的 remove_dir_all 会失败）。
    fs::rename(probe.join(".git"), probe.join(".git-disabled")).expect("disable .git");
    let no_repo = prepare_verified_lust_inputs(&probe).expect_err("missing repository must fail");
    match no_repo {
        Error::SourceRevisionUnknown { .. } => {}
        other => panic!("unexpected error: {other}"),
    }

    // 4) 字节改动：同尺寸改写一份文件（digest 关先于 revision 关），准备函数
    //    以 SourceDigestMismatch 拒绝——早于任何 normalize/转换。
    let tll_path = probe.join("scenario/tll.static.xml");
    let mut bytes = fs::read(&tll_path).expect("read probe tll");
    let midpoint = bytes.len() / 2;
    bytes[midpoint] = bytes[midpoint].wrapping_add(1);
    fs::write(&tll_path, &bytes).expect("write modified probe tll");
    let digest_mismatch =
        prepare_verified_lust_inputs(&probe).expect_err("modified bytes must fail");
    match digest_mismatch {
        Error::SourceDigestMismatch { relative_path, .. } => {
            assert_eq!(relative_path, "scenario/tll.static.xml");
        }
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
