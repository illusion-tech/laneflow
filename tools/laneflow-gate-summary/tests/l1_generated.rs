//! L1 拓扑核查的进程内全链测试（忽略级：需要编译 LFCA，属重负载）。
//!
//! 经 `cargo test -p laneflow-gate-summary --test l1_generated -- --ignored`
//! 手动运行；正式 L1 输入包（含仓库外缓存）的核查走 CLI `check-transitions`。

use laneflow_gate_summary::l1check::{self, BundleInput};
use laneflow_synth_generator::{Case, Identity, LaneChangeParams, Provenance, generate};

#[test]
#[ignore = "进程内编译 LFCA 属重负载，按窗口手动运行"]
fn generated_l1_bundle_has_no_1271_topology() {
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
    let input = BundleInput {
        identity: "10000-L1-16".to_owned(),
        manifest,
        network_lfca: bundle.files()["network.lfca"].clone(),
        state_json: bundle.files()["state.json"].clone(),
    };
    let report = l1check::check(&input).expect("check");
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
