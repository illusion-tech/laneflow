use std::fs::File;

use laneflow_synth_generator::{Manifest, handoff};
use serde::Deserialize;
use serde_json::Value;

use super::{EvidenceOutcome, PathContext};
use crate::{Error, invalid};

#[derive(Deserialize)]
struct Run {
    format: String,
    complete: bool,
    scope: String,
    semantic_contract: String,
    implementation_commit: String,
    input_manifest: Manifest,
    warm_up_ticks: u64,
    observation_ticks: u64,
    validated_boundaries: u64,
    checkpoints: Vec<Value>,
}

pub(super) fn resolve(
    path: &str,
    input_path: &str,
    implementation: &str,
    ctx: &PathContext,
) -> Result<EvidenceOutcome, Error> {
    let run: Run = serde_json::from_reader(File::open(ctx.resolve(path)?)?)?;
    let input: Manifest = serde_json::from_reader(File::open(ctx.resolve(input_path)?)?)?;
    input.validate().map_err(|e| invalid(e.to_string()))?;
    if run.input_manifest != input {
        return Err(invalid(format!(
            "L1 回执与声明的输入清单不符：{input_path}"
        )));
    }
    let identity = &input.identity;
    let params = identity
        .lane_change()
        .ok_or_else(|| invalid("清单未声明动态横移竞争"))?;
    let (warm, observation) = identity.lane_change_window();
    if run.format != "lf-synth-semantic-run-1"
        || !run.complete
        || run.scope
            != "L1_lane_change_competition_success_path_not_performance_or_product_certification"
        || run.semantic_contract != "lf-synth-execution-2"
        || implementation.len() != 40
        || !implementation
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        || run.implementation_commit != implementation
        || (run.warm_up_ticks, run.observation_ticks) != (warm, observation)
        || run.validated_boundaries != warm + observation + 1
    {
        return Err(invalid(format!(
            "动态横移回执的提交、合同或完整窗口不符：{path}"
        )));
    }
    handoff::check_history(&run.checkpoints, identity).map_err(invalid)?;
    let label = format!(
        "{}-L1-{}-clusters-{}",
        identity.scale, identity.fixed_delta_ms, params.clusters
    );
    Ok(EvidenceOutcome {
        label: path.to_owned(),
        detail: format!(
            "输入清单 {input_path}，配置摘要 {}，来源 {}，生成器 {}，执行 {}，已核对 {} 辆车的进入、前杠交接和车尾清空",
            input.config_digest,
            input.source_commit,
            input.generator_commit,
            implementation,
            2 * params.clusters
        ),
        identity: Some(label),
    })
}
