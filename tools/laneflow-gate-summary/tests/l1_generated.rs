//! L1 拓扑核查的进程内全链测试（忽略级：需要编译 LFCA，属重负载）。
//!
//! 经 `cargo test -p laneflow-gate-summary --test l1_generated -- --ignored`
//! 手动运行；正式 L1 输入包（含仓库外缓存）的核查走 CLI `check-transitions`。
//!
//! 动态 L1 不能包含固定过渡。另用最小四边路网保留固定过渡存续期间的 #1271 盲路线反例。

use laneflow_compiler::*;
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
    assert!(report.transitions.is_empty(), "动态换道簇生成了固定过渡");
    assert_eq!(report.verdict, "clean");
}

/// 两条编制车道的起点独立，目标车道在汇入切点之前已有平行边。
fn parallel_edge_fixture() -> BundleInput {
    let limits = CompileProfile::single_network_10k_stable_entities();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "gate/parallel",
            source_document_key: "parallel.document",
            generator_build_id: "parallel-test",
            parameters_and_inputs_digest: [1; 32],
            frontend_options_digest: [2; 32],
            random_seed: None,
            provenance: "test",
        },
        &limits,
    )
    .unwrap();
    let mut builder = SyntheticModuleBuilder::new(header, &limits).unwrap();
    for (name, length, next) in [
        ("source", 30., Some("transition")),
        ("transition", 5., Some("target")),
        ("parallel", 30., Some("target")),
        ("target", 30., None),
    ] {
        let successors: Vec<_> = next.into_iter().map(LaneEdgeReference::local).collect();
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: name,
                length_meters: length,
                speed_limit_meters_per_second: 8.,
                successors: &successors,
            })
            .unwrap();
    }
    builder
        .add_road_section(RoadSectionInput {
            road_section_key: "section",
            kind_id: "motorLane",
            lanes: &[
                AuthoringLaneInput {
                    authoring_lane_key: "left",
                    edge_chain: &[LaneEdgeReference::local("source")],
                    lane_group: None,
                },
                AuthoringLaneInput {
                    authoring_lane_key: "right",
                    edge_chain: &[
                        LaneEdgeReference::local("parallel"),
                        LaneEdgeReference::local("target"),
                    ],
                    lane_group: None,
                },
            ],
        })
        .unwrap();
    builder
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "corridor",
            reference_section: RoadSectionReference::local("section"),
            elements: &[CorridorElementReference::road_section(
                RoadSectionReference::local("section"),
            )],
        })
        .unwrap();
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    unit.add_synthetic_module(builder.finish().unwrap())
        .unwrap();
    let output = Compiler::new().compile(unit.build().unwrap()).unwrap();
    let candidate = emit_portable_candidate(
        &output,
        &PortableEmissionProvenance::try_new("parallel-test").unwrap(),
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    let state = serde_json::json!({"format":"lf-synth-initial-state-1",
        "identity": Identity::new(10_000, Case::W1, 16).unwrap(),
        "policy_id":"","vehicle_profile_id":"","routes":[],"vehicles":[]});
    BundleInput {
        identity: "parallel-edge-fixture".into(),
        manifest: serde_json::json!({}),
        network_lfca: candidate.canonical_artifact().bytes().to_vec(),
        state_json: serde_json::to_vec(&state).unwrap(),
    }
}

#[test]
fn injected_parallel_edge_route_is_flagged_as_blind() {
    let mut input = parallel_edge_fixture();
    assert_eq!(l1check::check(&input).unwrap().verdict, "clean");
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
        handoff: None,
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
