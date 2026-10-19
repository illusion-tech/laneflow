//! #538 三项接缝边界成本测量：observation export、dynamic cost binding 构造与
//! 验证、candidate / admitted 路线注册。
//!
//! 在 10k 城市真实步进后的已提交边界上逐拍采样（每拍一条完整接缝链：delta 导出
//! → 观测集合绑定 → 动态成本绑定构造 → candidate 注册；另测 admitted 注册与
//! 陈旧观测时效拒绝），报告中位数。release 手动运行；正常延迟与插桩归因分开——
//! 本测量不替换全局分配器。成本模型身份与快照摘要字段为计时用合成值（验证与
//! 注册路径不读取其内容真值），在回执中如实登记。

use std::{fs, path::Path, time::Instant};

use laneflow_runtime::*;
use laneflow_static_contract::{Sha256Digest, StableId128};
use serde_json::{Value, json};

use crate::{
    Artifacts, Harness, Result, checked, invalid, report::write_json, transitions::native,
};

const WARM_TICKS: u64 = 32;
const SAMPLES: usize = 31;

fn median_ns(samples: &[u128]) -> u128 {
    if samples.is_empty() {
        return 0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    sorted[sorted.len() / 2]
}

fn route_edge_pool(harness: &Harness<'_>) -> Result<Vec<Vec<StableId128>>> {
    // 从工件目录路表构建（与车辆状态无关，确定性）：取前 16 条目录路线的边组。
    let revision = harness.world.revision().clone();
    let identity = revision.identity();
    let mut pool = Vec::new();
    for route in harness.artifacts.catalog().routes.iter().take(16) {
        let mut ids = Vec::new();
        for key in &route.edge_keys {
            let ordinal = harness
                .artifacts
                .edges
                .get(key)
                .ok_or_else(|| invalid(format!("unknown route edge {key}")))?;
            let id = identity
                .stable_id(*ordinal)
                .ok_or_else(|| invalid("missing lane edge identity"))?;
            ids.push(id.into_untyped());
        }
        if ids.len() >= 2 {
            pool.push(ids);
        }
    }
    if pool.is_empty() {
        return Err(invalid("no catalog route with at least two edges"));
    }
    Ok(pool)
}

fn synthetic_digest(seed: &str) -> Sha256Digest {
    let hex = crate::sha256(seed.as_bytes());
    let mut bytes = [0u8; 32];
    for (slot, byte) in hex.as_bytes().iter().copied().take(32).enumerate() {
        bytes[slot] = byte;
    }
    Sha256Digest::from_bytes(bytes)
}

/// 采样主体（无 provenance 绑定，供测试与公开入口共用）。
fn seam_witness(artifacts: &Artifacts, plan: &crate::ResolvedPlan, output: &Path) -> Result<Value> {
    let started = Instant::now();
    let plan_digest = plan.digest()?;
    fs::create_dir(output)?;
    let mut harness = Harness::install_with_route_slack(
        artifacts,
        plan,
        ExecutionConfig::new(std::num::NonZeroU32::MIN),
        // 每采样注册 candidate+admitted 各一，另留陈旧探针与余量。
        u32::try_from(SAMPLES * 2 + 8).expect("slack fits u32"),
    )?;
    for _ in 0..WARM_TICKS {
        harness.advance()?;
    }

    // —— observation export：会话开启、首满导出，随后逐拍 delta。 ——
    let open_started = Instant::now();
    let mut session = checked(
        "open observation export",
        native(&mut harness)?.open_observation_export(ObservationSelection::AllLaneEdges),
    )?;
    let open_ns = open_started.elapsed().as_nanos();
    let full_started = Instant::now();
    let full = checked(
        "first full export",
        native(&mut harness)?.export_observation(&mut session, ObservationExportMode::Full),
    )?;
    let first_full_ns = full_started.elapsed().as_nanos();
    let full_rows = full.rows().len();

    let pool = route_edge_pool(&harness)?;
    let cost_model = CostModelKey::new(Sha256Digest::ZERO, 1);
    let derivation_version = native(&mut harness)?
        .revision()
        .canonical_origin()
        .static_contract_versions()
        .network_revision_derivation_version();

    // —— 陈旧观测时效拒绝（先行：此时路线表未累积，拒绝不占槽）：用推进前边界
    //    的绑定在推进后注册，验证失败关闭的接缝成本。计时只覆盖注册调用本身。 ——
    let admission = native(&mut harness)?.open_routing_admission(cost_model);
    let stale_batch = checked(
        "pre-advance delta",
        native(&mut harness)?.export_observation(&mut session, ObservationExportMode::Delta),
    )?;
    let stale_binding = checked(
        "pre-advance binding",
        DynamicCostSnapshotBinding::new(
            checked(
                "pre-advance observation set",
                bind_observation_set(&[&stale_batch]),
            )?,
            cost_model,
            stale_batch.tick(),
            0,
            0,
            synthetic_digest("stale"),
        ),
    )?;
    harness.advance()?;
    let stale_edges: Vec<_> = pool[0].clone();
    let stale_started = Instant::now();
    let stale_error = native(&mut harness)?
        .register_candidate_route(
            &admission,
            CandidateRouteInput::new(stale_binding, stale_edges),
        )
        .err()
        .ok_or_else(|| invalid("stale observation unexpectedly accepted"))?;
    let stale_reject_ns = stale_started.elapsed().as_nanos();

    let mut delta_ns = Vec::with_capacity(SAMPLES);
    let mut delta_rows = Vec::with_capacity(SAMPLES);
    let mut bind_ns = Vec::with_capacity(SAMPLES);
    let mut binding_ns = Vec::with_capacity(SAMPLES);
    let mut candidate_ns = Vec::with_capacity(SAMPLES);
    let mut admitted_ns = Vec::with_capacity(SAMPLES);
    let mut registration_samples = 0_u64;
    let mut registration_capacity_note = Value::Null;
    for sample in 0..SAMPLES {
        harness.advance()?;
        let export_started = Instant::now();
        let batch = checked(
            "delta export",
            native(&mut harness)?.export_observation(&mut session, ObservationExportMode::Delta),
        )?;
        delta_ns.push(export_started.elapsed().as_nanos());
        delta_rows.push(batch.rows().len());

        let bind_started = Instant::now();
        let observation_set = checked("bind observation set", bind_observation_set(&[&batch]))?;
        bind_ns.push(bind_started.elapsed().as_nanos());

        let construct_started = Instant::now();
        let rows = u64::try_from(batch.rows().len()).expect("rows fit u64");
        let binding = checked(
            "dynamic cost binding",
            DynamicCostSnapshotBinding::new(
                observation_set,
                cost_model,
                batch.tick(),
                rows,
                rows.saturating_mul(64),
                synthetic_digest(&format!("seam-sample-{sample}")),
            ),
        )?;
        binding_ns.push(construct_started.elapsed().as_nanos());

        let edges: Vec<_> = pool[sample % pool.len()].clone();
        let admission = native(&mut harness)?.open_routing_admission(cost_model);
        let candidate_started = Instant::now();
        match native(&mut harness)?
            .register_candidate_route(&admission, CandidateRouteInput::new(binding, edges.clone()))
        {
            Ok(_) => candidate_ns.push(candidate_started.elapsed().as_nanos()),
            Err(error) => {
                // 路线槽按计划目录尺寸预留，累积注册会到达容量边界；截断并如实登记。
                registration_capacity_note = json!({"stopped_at_sample": sample,
                    "error": format!("{error:?}"),
                    "note": "route capacity sized for catalog routes; registration sampling truncated"});
                break;
            }
        }
        registration_samples += 1;

        let admitted_started = Instant::now();
        let network_revision = native(&mut harness)?.revision().network_revision();
        checked(
            "admitted registration",
            native(&mut harness)?.register_admitted_route(AdmittedRouteRegisterInput::new(
                network_revision,
                derivation_version,
                edges,
            )),
        )?;
        admitted_ns.push(admitted_started.elapsed().as_nanos());
    }

    let logical_bytes = checked("session logical bytes", session.logical_bytes())?;
    let retained_bytes = checked("session retained bytes", session.retained_bytes())?;
    Ok(
        json!({"status":"seam-evidence-pass","scale":artifacts.catalog().scale,
        "warm_ticks":WARM_TICKS,"samples":SAMPLES,
        "observation":{"open_ns":open_ns,"first_full_ns":first_full_ns,"full_rows":full_rows,
            "delta_ns_median":median_ns(&delta_ns),
            "delta_ns_min":delta_ns.iter().copied().min().unwrap_or(0),
            "delta_ns_max":delta_ns.iter().copied().max().unwrap_or(0),
            "delta_rows_last":delta_rows.last().copied().unwrap_or(0),
            "session_logical_bytes":logical_bytes,"session_retained_bytes":retained_bytes},
        "routing":{"bind_observation_set_ns_median":median_ns(&bind_ns),
            "dynamic_cost_binding_ns_median":median_ns(&binding_ns),
            "registration_samples":registration_samples,
            "registration_capacity":registration_capacity_note,
            "candidate_register_ns_median":median_ns(&candidate_ns),
            "candidate_register_ns_max":candidate_ns.iter().copied().max().unwrap_or(0),
            "admitted_register_ns_median":median_ns(&admitted_ns),
            "stale_reject_ns":stale_reject_ns,
            "stale_reject_error":format!("{stale_error:?}"),
            "cost_model_identity":"synthetic (timing only; validation path does not read content)"},
            "inputs":{"artifact_files":serde_json::to_value(&artifacts.files)?,
            "plan_digest":plan_digest},
            "elapsed_ms":started.elapsed().as_millis()}),
    )
}

/// # Errors
///
/// 输入不完整、任一接缝步骤失败、或运行期间源码/输入身份变化时失败。
pub fn run_seam_evidence(
    artifact_directory: &Path,
    plan_path: &Path,
    output: &Path,
) -> Result<Value> {
    let command = std::env::args().collect::<Vec<_>>().join(" ");
    let provenance = crate::evidence::source()?;
    let artifacts = Artifacts::load(artifact_directory)?;
    let plan = crate::ResolvedPlan::read(plan_path)?;
    let mut report = seam_witness(&artifacts, &plan, output)?;
    if crate::evidence::source()? != provenance
        || crate::ResolvedPlan::read(plan_path)?.digest()? != plan.digest()?
        || Artifacts::load(artifact_directory)?.files != artifacts.files
    {
        return Err(invalid("seam inputs/source changed"));
    }
    report["command"] = json!(command);
    report["source"] = provenance;
    write_json(&output.join("seams.json"), &report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use laneflow_urban_generator::{Scale, UrbanConfig, generate};

    #[test]
    fn seam_evidence_passes_at_fixture_scale() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        let config =
            UrbanConfig::parse(include_str!("../../../examples/config/cn-urban.toml")).unwrap();
        generate(&config, Scale::Fixture, &base, None).unwrap();
        let plan_path = temp.path().join("plan.toml");
        let plan = crate::ResolvedPlan::for_case(
            &Artifacts::load(&base).unwrap(),
            crate::UrbanCase::MixedPeak,
            crate::Window::probe(WARM_TICKS + SAMPLES as u64 + 8).unwrap(),
        )
        .unwrap();
        plan.write(&plan_path).unwrap();
        let out = temp.path().join("seams");
        let artifacts = Artifacts::load(&base).unwrap();
        let report = seam_witness(&artifacts, &plan, &out).unwrap();
        assert_eq!(report["status"], "seam-evidence-pass");
    }
}
