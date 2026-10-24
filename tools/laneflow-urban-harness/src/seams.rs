//! #538 三项接缝边界成本测量：observation export、dynamic cost binding 构造与
//! 验证、candidate / admitted 路线注册。
//!
//! 在 10k 城市真实步进后的已提交边界上逐拍采样（每拍一条完整接缝链：delta 导出
//! → 观测集合绑定 → 动态成本绑定构造 → candidate 注册；另测 admitted 注册与
//! 陈旧观测时效拒绝），报告中位数并落盘全部原始样本供复核。计时窗只覆盖被测
//! 调用本身——输入构造、克隆与摘要准备一律在计时窗之外。每条注册的路线在计时
//! 后于测量区内移除（沿 §7「成功注册后在测量区外移除」先例），路表保持平稳。
//! 本测量不替换全局分配器：按
//! `docs/design/traffic-observation-and-routing-integration.md` §7 的完整维度
//! （累计分配、峰值 retained、tick 间隔干扰），本回执是**描述性部分证据**，
//! 不是该 G2 线的完整通过结论（完整维度见同文档 §7.1 首轮证据与后续补测）。
//! 成本模型身份与快照摘要/条目数/字节长度字段为计时用合成值（验证与注册路径
//! 不读取其内容真值），在回执中逐项登记。测量世界经路线容量余量装机，与标准
//! 装机不同身份（容量进入快照与逻辑摘要），数值不外推到标准装机。

use std::{fs, path::Path, time::Instant};

use laneflow_runtime::*;
use laneflow_static_contract::{Sha256Digest, StableId128};
use serde_json::{Value, json};

use crate::{
    Artifacts, Harness, Result, checked, invalid, report::write_json, transitions::native,
};

const WARM_TICKS: u64 = 32;
const SAMPLES: usize = 31;
/// 路线容量余量（条）；配合注册后移除协议，任何时刻占用不超过 2 条，
/// 余量仅需覆盖瞬时占用与陈旧探针，取值保守。
const ROUTE_SLACK: u32 = 8;

fn median_ns(samples: &[u128]) -> Value {
    if samples.is_empty() {
        return Value::Null;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    Value::from(u64::try_from(sorted[sorted.len() / 2]).expect("ns fits u64"))
}

fn spread_ns(samples: &[u128]) -> Value {
    if samples.is_empty() {
        return json!({"samples": 0, "min": null, "median": null, "max": null});
    }
    json!({"samples": samples.len(),
        "min": samples.iter().copied().min().unwrap_or(0),
        "median": median_ns(samples),
        "max": samples.iter().copied().max().unwrap_or(0)})
}

/// 容量族错误才允许截断注册采样（见下方调用处）。
fn is_capacity_truncation(error: &CandidateRouteError) -> bool {
    matches!(
        error,
        CandidateRouteError::Route(RouteError::CapacityExceeded)
            | CandidateRouteError::Route(RouteError::EdgeOccurrenceCapacityExceeded)
            | CandidateRouteError::Route(RouteError::ConflictOccurrenceCapacityExceeded { .. })
    )
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
        ROUTE_SLACK,
    )?;
    for _ in 0..WARM_TICKS {
        harness.advance()?;
    }
    // 采样边界状态绑定：warm-up 末的确定性快照摘要（外部可复核的边界锚点）。
    let boundary_digest = harness.checkpoint()?;

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
    let cost_model = CostModelKey::new(CostModelId::from_bytes([0; 32]), 1);
    let network_revision = native(&mut harness)?.revision().network_revision();
    let derivation_version = native(&mut harness)?
        .revision()
        .canonical_origin()
        .static_contract_versions()
        .network_revision_derivation_version();

    // —— 陈旧观测时效拒绝（先行：路表未累积、拒绝不占槽；计时只覆盖注册调用）。 ——
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
    let stale_input = CandidateRouteInput::new(stale_binding, pool[0].clone());
    harness.advance()?;
    let stale_started = Instant::now();
    let stale_error = native(&mut harness)?
        .register_candidate_route(&admission, stale_input)
        .err()
        .ok_or_else(|| invalid("stale observation unexpectedly accepted"))?;
    let stale_reject_ns = stale_started.elapsed().as_nanos();
    if !matches!(stale_error, CandidateRouteError::StaleDynamicCost) {
        return Err(invalid(format!(
            "stale probe did not hit StaleDynamicCost: {stale_error:?}"
        )));
    }

    let mut delta_ns = Vec::with_capacity(SAMPLES);
    let mut delta_rows = Vec::with_capacity(SAMPLES);
    let mut bind_ns = Vec::with_capacity(SAMPLES);
    let mut binding_ns = Vec::with_capacity(SAMPLES);
    let mut candidate_ns = Vec::with_capacity(SAMPLES);
    let mut admitted_ns = Vec::with_capacity(SAMPLES);
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

        // 计时窗外完成全部准备：行数换算、合成摘要、绑定构造。
        let rows = u64::try_from(batch.rows().len()).expect("rows fit u64");
        let digest = synthetic_digest(&format!("seam-sample-{sample}"));
        let construct_started = Instant::now();
        let binding = checked(
            "dynamic cost binding",
            DynamicCostSnapshotBinding::new(
                observation_set,
                cost_model,
                batch.tick(),
                rows,
                rows.saturating_mul(64),
                digest,
            ),
        )?;
        binding_ns.push(construct_started.elapsed().as_nanos());

        // 计时窗外完成输入构造（克隆边序列），窗内只有注册调用本身。
        let edges: Vec<_> = pool[sample % pool.len()].clone();
        let candidate_input = CandidateRouteInput::new(binding, edges.clone());
        let admission = native(&mut harness)?.open_routing_admission(cost_model);
        let candidate_started = Instant::now();
        let candidate = native(&mut harness)?.register_candidate_route(&admission, candidate_input);
        let candidate_elapsed = candidate_started.elapsed().as_nanos();
        match candidate {
            Ok(handle) => {
                candidate_ns.push(candidate_elapsed);
                checked(
                    "remove candidate route",
                    native(&mut harness)?.remove_route(handle),
                )?;
            }
            Err(error) => {
                // 仅容量族错误允许截断采样（余量配合注册后移除，正常不触发）；
                // 其余任何错误（稳定 ID 解析、绑定失配、分配失败等）都是测量
                // 缺陷，直接失败而不是以零/部分样本冒充通过。
                if !is_capacity_truncation(&error) {
                    return Err(invalid(format!(
                        "candidate registration failed at sample {sample}: {error:?}"
                    )));
                }
                registration_capacity_note = json!({"stopped_at_sample": sample,
                    "error": format!("{error:?}"),
                    "note": "capacity family only; registration sampling truncated"});
                break;
            }
        }

        // admitted：输入构造在窗外，注册与移除各计时一次。
        let admitted_input =
            AdmittedRouteRegisterInput::new(network_revision, derivation_version, edges);
        let admitted_started = Instant::now();
        let admitted = native(&mut harness)?.register_admitted_route(admitted_input);
        let admitted_elapsed = admitted_started.elapsed().as_nanos();
        match admitted {
            Ok(handle) => {
                admitted_ns.push(admitted_elapsed);
                checked(
                    "remove admitted route",
                    native(&mut harness)?.remove_route(handle),
                )?;
            }
            Err(error) => {
                let is_capacity = matches!(
                    error,
                    AdmittedRouteRegisterError::Route(RouteError::CapacityExceeded)
                        | AdmittedRouteRegisterError::Route(
                            RouteError::EdgeOccurrenceCapacityExceeded,
                        )
                        | AdmittedRouteRegisterError::Route(
                            RouteError::ConflictOccurrenceCapacityExceeded { .. },
                        )
                );
                if !is_capacity {
                    return Err(invalid(format!(
                        "admitted registration failed at sample {sample}: {error:?}"
                    )));
                }
                registration_capacity_note = json!({"stopped_at_sample": sample,
                    "error": format!("{error:?}"),
                    "note": "capacity family only; registration sampling truncated"});
                break;
            }
        }
    }

    let logical_bytes = checked("session logical bytes", session.logical_bytes())?;
    let retained_bytes = checked("session retained bytes", session.retained_bytes())?;
    Ok(
        json!({"status":"seam-evidence-descriptive","scale":artifacts.catalog().scale,
        "warm_ticks":WARM_TICKS,"samples":SAMPLES,
        "classification":"descriptive partial evidence; NOT the full section-7 G2 pass \
(allocation totals, peak retained, tick-interval interference are not measured here — \
see traffic-observation-and-routing-integration.md section 7/7.1)",
        "timing_basis":"single-shot-per-tick Instant wall clock around the named call only; \
input construction, clones and digest preparation are outside the timed window; medians \
over consecutive committed boundaries in one process; not comparable across machines",
        "route_slack":{"extra_routes":ROUTE_SLACK,
            "note":"measurement world installs with added route capacity; capacities enter \
snapshot and logical digests, so this world's identity differs from a standard install; \
values do not extrapolate to standard installs"},
        "boundary_checkpoint_digest":boundary_digest,
        "observation":{"open_ns":open_ns,"first_full_ns":first_full_ns,"full_rows":full_rows,
            "delta_ns":spread_ns(&delta_ns),
            "delta_rows":delta_rows,
            "session_logical_bytes":logical_bytes,"session_retained_bytes":retained_bytes},
        "routing":{"bind_observation_set_ns":spread_ns(&bind_ns),
            "dynamic_cost_binding_ns":spread_ns(&binding_ns),
            "candidate_register_ns":spread_ns(&candidate_ns),
            "admitted_register_ns":spread_ns(&admitted_ns),
            "stale_reject_ns":stale_reject_ns,
            "stale_reject_error":format!("{stale_error:?}"),
            "registration_capacity":registration_capacity_note,
            "synthetic_fields":"cost model identity, snapshot digest, entry count and exact \
byte length are timing-only synthetic values; validation/registration paths do not read \
their content truth"},
        "inputs":{"artifact_files":serde_json::to_value(&artifacts.files)?,
            "artifact_manifest_digest":artifacts.manifest_digest.clone(),
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
        || Artifacts::load(artifact_directory)?.manifest_digest != artifacts.manifest_digest
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
    fn is_capacity_truncation_matches_only_capacity_family() {
        assert!(is_capacity_truncation(&CandidateRouteError::Route(
            RouteError::CapacityExceeded
        )));
        assert!(is_capacity_truncation(&CandidateRouteError::Route(
            RouteError::EdgeOccurrenceCapacityExceeded
        )));
        assert!(is_capacity_truncation(&CandidateRouteError::Route(
            RouteError::ConflictOccurrenceCapacityExceeded {
                current: 1,
                added: 1,
                capacity: 1,
            }
        )));
        assert!(!is_capacity_truncation(
            &CandidateRouteError::UnknownLaneEdge {
                stable_id: StableId128::from_bytes([0; 16]),
            }
        ));
    }

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
        assert_eq!(report["status"], "seam-evidence-descriptive");
        assert_eq!(report["routing"]["stale_reject_error"], "StaleDynamicCost");
        assert_eq!(
            report["routing"]["candidate_register_ns"]["samples"],
            SAMPLES
        );
        assert_eq!(
            report["routing"]["admitted_register_ns"]["samples"],
            SAMPLES
        );
        assert_eq!(report["observation"]["delta_ns"]["samples"], SAMPLES);
    }
}
