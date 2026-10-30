//! #1175 块③：生产资源测量首轮（关键路径耗时/内存）。
//!
//! 三阶段口径（本切片定义，随 G1 章交付）：`parse` = pinned LuST 源校验读入后的
//! 网络/信号灯/车型解析与画像转换；`convert` = 拓扑转换 + 全网曲线发射（观察捕获）；
//! `replay` = **真实回放核验**（第二次发射 + 两次程序输出逐程序位级比对；P2-3 修复）。阶段经
//! `G1_MEASURE_PHASE` 选择、单进程单阶段以便内存归因；双形态（debug/release）与轮次
//! 由本机测量脚本驱动。计时口径对齐 #1113 账目：原始计时入 measurements、条件入
//! environment、无空闲门槛注明、描述性限定；测量仅取证不调优、不改生产。

use super::*;

fn g1_phase() -> String {
    std::env::var("G1_MEASURE_PHASE").expect("G1_MEASURE_PHASE")
}

#[test]
#[ignore = "requires pinned LUST_SOURCE_DIR, G1_MEASURE_PHASE=parse|convert|replay and fresh G1_MEASURE_OUTPUT"]
fn measure_production_key_paths() {
    use crate::convert::profiles::{convert_vehicle_profiles, select_passenger_vtypes};
    use crate::convert::topology::{
        TopologyConvertOptions, convert_network_topology_with_tll_and_profiles,
    };
    use crate::source::verify::prepare_verified_lust_inputs;
    use crate::sumo::{parse_sumo_network_xml, parse_tll_static_xml, parse_vtypes_xml};
    let phase = g1_phase();
    assert!(
        matches!(phase.as_str(), "parse" | "convert" | "replay"),
        "phase vocabulary"
    );
    let root = PathBuf::from(std::env::var("LUST_SOURCE_DIR").expect("LUST_SOURCE_DIR"));
    let out = PathBuf::from(std::env::var("G1_MEASURE_OUTPUT").expect("G1_MEASURE_OUTPUT"));
    assert!(!out.exists(), "fresh measurement output");
    fs::create_dir_all(&out).unwrap();
    // 前置段（源校验/读入/未测阶段）不计入本阶段计时。
    let prepared = prepare_verified_lust_inputs(&root).expect("verified pinned source");
    let mut timing =
        json!({"phase":phase,"accounting":"#1113 原始计时口径；无空闲门槛；描述性限定"});
    let mut detail = json!({});
    if phase == "parse" {
        let started = Instant::now();
        let network = parse_sumo_network_xml(prepared.net_xml()).unwrap();
        let tll = parse_tll_static_xml(prepared.tll_xml()).unwrap();
        let vtypes = parse_vtypes_xml(prepared.vtypes_xml()).unwrap();
        let profiles =
            convert_vehicle_profiles(&select_passenger_vtypes(&vtypes).unwrap()).unwrap();
        timing["phase_seconds"] = json!(started.elapsed().as_secs_f64());
        detail = json!({"parsed":"net+tll+vtypes+profiles"});
        let _ = (network, tll, vtypes, profiles);
    } else {
        let network = parse_sumo_network_xml(prepared.net_xml()).unwrap();
        let tll = parse_tll_static_xml(prepared.tll_xml()).unwrap();
        let vtypes = parse_vtypes_xml(prepared.vtypes_xml()).unwrap();
        let profiles =
            convert_vehicle_profiles(&select_passenger_vtypes(&vtypes).unwrap()).unwrap();
        let options = TopologyConvertOptions {
            require_lust_location_anchors: true,
            emit_infeasibility_report: true,
            ..TopologyConvertOptions::default()
        };
        let started = Instant::now();
        let (original, observation) = observe::capture(|| {
            convert_network_topology_with_tll_and_profiles(
                &network,
                &tll,
                &profiles,
                &options,
                prepared.report_source().clone(),
            )
        });
        let convert_seconds = started.elapsed().as_secs_f64();
        let original = original.unwrap();
        if phase == "convert" {
            timing["phase_seconds"] = json!(convert_seconds);
            detail = json!({"lane_edges":observation.edges.len(),
                "maneuver_paths":observation.traffic.as_ref().unwrap().maneuver_paths.len()});
        } else {
            // replay（P2-3 修复）：**真实回放核验**——第二次发射 + 两次程序输出逐
            // 程序位级比对（重放发射语义，非克隆空转）；断言按 >=2 点位修正
            // （`program_points` 含起点，非空断言恒真）。
            let started = Instant::now();
            let (second, second_observation) = observe::capture(|| {
                convert_network_topology_with_tll_and_profiles(
                    &network,
                    &tll,
                    &profiles,
                    &options,
                    prepared.report_source().clone(),
                )
            });
            let second = second.unwrap();
            let programs = original_programs(&observation);
            let replayed_programs = original_programs(&second_observation);
            assert_eq!(
                programs.len(),
                replayed_programs.len(),
                "replay program count"
            );
            let mut replayed_points = 0_usize;
            for (id, program) in &programs {
                let points = program_points(program);
                assert!(points.len() >= 2, "replay program points for {id}");
                assert_eq!(
                    program_bits(program),
                    program_bits(&replayed_programs[id]),
                    "replay program bits for {id}"
                );
                replayed_points += points.len();
            }
            timing["phase_seconds"] = json!(started.elapsed().as_secs_f64());
            detail = json!({"programs":programs.len(),"replayed_points":replayed_points,
                "replay_semantics":"second emission + per-program bit comparison"});
            let _ = (original, second);
        }
    }
    let record = json!({"measurement":timing,"detail":detail,
        "note":"单进程单阶段；进程峰值内存由本机脚本采样（PeakWorkingSet/PrivateBytes）"});
    recovery::write_json(&out, "measurement.json", &record);
    println!("g1 measure {} saved={}", phase, out.display());
}
