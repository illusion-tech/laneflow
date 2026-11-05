//! L1 拓扑核查的进程内全链测试（忽略级：需要编译 LFCA，属重负载）。
//!
//! 经 `cargo test -p laneflow-gate-summary --test l1_generated -- --ignored`
//! 手动运行；正式 L1 输入包（含仓库外缓存）的核查走 CLI `check-transitions`。
//!
//! 覆盖两条：诚实包必须 clean；向 state 注入一条「只走过渡段目标车道链前段、
//! 避开可见邻域」的路线后必须翻成 violations-found（#1285 评审 P1 回归——
//! 车道链原点不共享时，锚定窗口须在候选车道自身坐标系内对齐）。

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_gate_summary::l1check::{self, BundleInput};
use laneflow_static_contract::LaneEdgeOrdinal;
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};
use laneflow_synth_generator::{
    Case, Identity, InitialState, LaneChangeParams, Provenance, RouteRecord, generate,
};

fn l1_bundle_input(identity_label: &str) -> BundleInput {
    let commit = "0".repeat(40);
    let identity = Identity::new(
        10_000,
        Case::LaneChange(LaneChangeParams {
            clusters: 1,
            changers_per_cluster: 2,
        }),
        16,
    )
    .expect("identity");
    let bundle = generate(
        identity,
        Provenance {
            source_commit: commit.clone(),
            generator_commit: commit,
        },
    )
    .expect("generate");
    let manifest: serde_json::Value =
        serde_json::from_slice(&bundle.files()["manifest.json"]).expect("manifest");
    BundleInput {
        identity: identity_label.to_owned(),
        manifest,
        network_lfca: bundle.files()["network.lfca"].clone(),
        state_json: bundle.files()["state.json"].clone(),
    }
}

#[test]
#[ignore = "进程内编译 LFCA 属重负载，按窗口手动运行"]
fn generated_l1_bundle_has_no_1271_topology() {
    let input = l1_bundle_input("10000-L1-16");
    let report = l1check::check(&input).expect("check");
    assert_eq!(report.bundle.identity, "10000-L1-16");
    assert!(
        report
            .transitions
            .iter()
            .all(|transition| transition.blind_routes.is_empty()),
        "盲路线应为空：{:?}",
        report.transitions
    );
    assert_eq!(report.verdict, "clean");
}

#[test]
#[ignore = "进程内编译 LFCA 属重负载，按窗口手动运行"]
fn injected_parallel_edge_route_is_flagged_as_blind() {
    let mut input = l1_bundle_input("10000-L1-16");

    // 找过渡段 t、其后渡边 e2 与目标车道链上 e2 的车道前驱 e4
    //（e4 物理上与 t 平行：链偏移 [0,30000) 物理跨度 5–35 m）。
    let checked = check_canonical_network_input(input.network_lfca.as_slice(), FormatLimits::HARD)
        .expect("lfca");
    let revision = build_shared_network_revision(
        checked,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(2 * 1_024 * 1_024 * 1_024, 2 * 1_024 * 1_024 * 1_024),
        ),
    )
    .expect("revision");
    let traffic = revision.traffic();
    let relations = traffic.relations();
    let identity = revision.identity();
    let mut transition = None;
    for raw in 0..traffic.lane_edge_count() {
        let edge = LaneEdgeOrdinal::from_raw(raw);
        if traffic.is_lateral_transition_segment(edge) {
            transition = Some(edge);
            break;
        }
    }
    let transition = transition.expect("transition edge");
    let successor = traffic.successors(transition).expect("successors")[0];
    let parallel = traffic
        .predecessors(successor)
        .expect("predecessors")
        .iter()
        .copied()
        .find(|edge| *edge != transition && relations.lane_edge_authoring_lane(*edge).is_some())
        .expect("parallel edge on target lane");
    let parallel_lfid = identity.stable_id(parallel).expect("stable id").to_string();

    // 注入一条只经过平行边的路线（避开邻域 {前驱, t, 后继}）。
    let mut state: InitialState = serde_json::from_slice(&input.state_json).expect("state");
    let next_rank = state.routes.iter().map(|r| r.route_rank).max().unwrap_or(0) + 1;
    state.routes.push(RouteRecord {
        route_rank: next_rank,
        cell: 0,
        route: 0,
        edge_ids: vec![parallel_lfid.clone()],
        signal_group_id: None,
    });
    input.state_json = serde_json::to_vec(&state).expect("state re-encode");

    let report = l1check::check(&input).expect("check");
    assert_eq!(
        report.verdict, "violations-found",
        "注入平行边盲路线后必须翻成违规：{:?}",
        report.transitions
    );
    let transition_report = &report.transitions[0];
    assert!(
        transition_report
            .parallel_edges
            .iter()
            .any(|candidate| candidate.edge == parallel_lfid),
        "平行边候选应包含注入目标：{:?}",
        transition_report.parallel_edges
    );
    assert_eq!(transition_report.blind_routes.len(), 1);
    assert_eq!(transition_report.blind_routes[0].route_rank, next_rank);
}
