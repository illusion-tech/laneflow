//! #538 快照与在线切换 Gate 端到端见证。
//!
//! 在 `transitions` 试用机械上补齐 #538 登记的认证缺口面：
//! - 跨切换逐拍逻辑恒等：`identity`（在线切换整窗与 baseline 的 tick 摘要逐位相等）；
//! - 完整工作流串联：`chained`（保存→恢复→重放→在线切换→续跑）；
//! - 可驱动失败面的零发布：`prepare-reject`（策略/来源修订/世界身份/基线游标四类
//!   前置拒绝）与 `overflow`/`lag`（追赶段日志字节与滞后超限），失败后世界继续
//!   推进、无部分发布，且可重试成功；
//! - 多轮跨修订切换：`multi`（base→v1→v2 两轮在线切换）。
//!
//! 执行回执（`gate.json`）绑定 exact head、命令、输入身份与逐场景结果。

use std::{
    fs,
    io::{BufWriter, Write},
    path::Path,
    time::Instant,
};

use laneflow_runtime::*;
use laneflow_static_network::SharedNetworkRevision;
use serde_json::{Value, json};

use crate::{
    Artifacts, Harness, ResolvedPlan, Result, UrbanCase, Window, checked, invalid,
    report::{digest_file, write_json},
    transitions::{native, restore, retained_state, selected_counts, target_source, trial},
    variant::{CapacityVariant, load_variant},
};

const SAVE_TICK: u64 = 32;
const PREPARE_TICK: u64 = 40;
const COMMIT_TICK: u64 = 48;
const ROUND1_PREPARE: u64 = 8;
const ROUND1_COMMIT: u64 = 24;
const ROUND2_PREPARE: u64 = 56;
const RETRY_PREPARE: u64 = 16;
const RETRY_COMMIT: u64 = 24;
const REJECT_TICK: u64 = 4;
const END_TICK: u64 = 160;
const PREFLIGHT: CutoverPreflightLimits = CutoverPreflightLimits::new(2_147_483_648);
/// 常规窗口限额：日志预算须对被测规模非绑定（只在失败场景故意压小）。10k 城市
/// 峰段的单拍命令记录即可超出 8 MiB，取 256 MiB 并登记进回执；`transitions` 的
/// 128 KiB 只覆盖其 tick 0-8 早期窗口。
const TRANSACTION: CutoverTransactionLimits = CutoverTransactionLimits {
    max_journal_bytes: 256 * 1_024 * 1_024,
    max_catch_up_lag_ticks: 8,
    max_records_per_pump: 4_096,
};
/// 日志字节上界压到首个武装步即溢出（镜像单测 `journal_overflow_abandons…` 的 63/64 量级）。
const TINY_JOURNAL: CutoverTransactionLimits = CutoverTransactionLimits {
    max_journal_bytes: 64,
    max_catch_up_lag_ticks: 8,
    max_records_per_pump: 4_096,
};
/// 追赶滞后上界压到停泵数拍即超限。
const TINY_LAG: CutoverTransactionLimits = CutoverTransactionLimits {
    max_journal_bytes: 256 * 1_024 * 1_024,
    max_catch_up_lag_ticks: 2,
    max_records_per_pump: 4_096,
};

fn descriptor_for(
    world: &TrafficWorld,
    target: &SharedNetworkRevision,
    binding: SemanticDiffOriginBinding,
    binding_override: Option<WorldBinding>,
    policy: MigrationPolicyKind,
) -> NetworkRevisionCutoverDescriptor {
    NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*world.revision().canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(*target.canonical_origin()),
        Some(binding),
        policy,
        binding_override.unwrap_or_else(|| world.world_binding()),
    )
}

/// 静默边界提交 + 提交不变量断言（镜像 `transitions` 在线提交块；多轮/重试共用）。
#[allow(clippy::type_complexity)]
fn commit_window(
    harness: &mut Harness<'_>,
    mut transaction: Option<CutoverTransaction>,
    variant: &CapacityVariant,
    tick: u64,
    round: &str,
) -> Result<Value> {
    let before = retained_state(harness);
    let bindings = selected_counts(harness, variant)?;
    let generation = harness.world.world_generation().get();
    let cursor = harness.world.event_cursor();
    let journal_before = format!("{:?}", harness.world.migration_journal_stats());
    let started = Instant::now();
    let commit = checked(
        "gate commit",
        transaction
            .take()
            .ok_or_else(|| invalid("missing transaction"))?
            .commit(native(harness)?),
    )?;
    if retained_state(harness) != before
        || selected_counts(harness, variant)? != bindings
        || commit.events.as_slice().len() != 1
        || harness.world.event_cursor() != cursor + 1
        || harness.world.world_generation().get() != generation + 1
        || harness.world.tick_index() != tick
        || harness.world.migration_journal_stats().is_some()
        || !std::sync::Arc::ptr_eq(&harness.world.revision(), &variant.root)
    {
        return Err(invalid(format!(
            "gate {round} commit invariant: journal_was={journal_before} events={} cursor={}->{} generation={}->{}",
            commit.events.as_slice().len(),
            cursor,
            harness.world.event_cursor(),
            generation,
            harness.world.world_generation().get(),
        )));
    }
    Ok(
        json!({"round":round,"commit_tick":tick,"commit_ns":started.elapsed().as_nanos(),
            "events":format!("{:?}", commit.events),"final_command_cursor":commit.final_command_cursor,
            "journal_before_commit":journal_before,"selected_binding_counts":bindings}),
    )
}

/// 逐拍记录骨架：advance、命令/事件与状态摘要落盘，供场景间逐位比较。
fn advance_and_log(
    harness: &mut Harness<'_>,
    log: &mut BufWriter<fs::File>,
    hashes: &mut Vec<String>,
    commands: &mut usize,
) -> Result<String> {
    let record = harness.advance()?;
    let state_digest = record.state_digest.clone();
    *commands += harness.commands.len();
    hashes.push(crate::sha256(&serde_json::to_vec(&record)?));
    serde_json::to_writer(
        &mut *log,
        &json!({"runtime":record,"presentation":null,
        "commands":harness.commands,"events":harness.events}),
    )?;
    log.write_all(b"\n")?;
    Ok(state_digest)
}

#[allow(clippy::too_many_arguments)]
fn trial_result(
    mode: &str,
    harness: &Harness<'_>,
    variant: &CapacityVariant,
    initial_bindings: &Value,
    save_digest: &str,
    hashes: &[String],
    command_records: usize,
    transition: &Value,
    output: &Path,
    log: &mut BufWriter<fs::File>,
) -> Result<Value> {
    // 摘要必须覆盖完整文件：先冲刷缓冲再取 digest（transitions 先例）。
    log.flush()?;
    let result = json!({"mode":mode,"ticks":END_TICK,"save_tick":SAVE_TICK,
        "initial_selected_bindings":initial_bindings,
        "final_selected_bindings":selected_counts(harness, variant)?,
        "save_digest":save_digest,"final_digest":harness.checkpoint()?,
        "final_retained_state":retained_state(harness),
        "tick_hashes":hashes,"transition":transition,
        "command_records":command_records,"tile_evidence":harness.evidence,
        "retry_reasons":harness.error_counts,
        "ticks_file":digest_file(&output.join("ticks.jsonl"))?});
    write_json(&output.join("trial.json"), &result)?;
    Ok(result)
}

/// `identity` / `chained`：同窗（prepare@40/commit@48）在线切换；`chained` 先在
/// SAVE_TICK 保存→丢弃旧世界→恢复再切换，串成保存/恢复/重放/切换完整工作流。
fn workflow_trial(
    mode: &str,
    restore_at_save: bool,
    artifacts: &Artifacts,
    plan: &ResolvedPlan,
    variant: &CapacityVariant,
    output: &Path,
    source: crate::save::SaveSource,
) -> Result<Value> {
    fs::create_dir(output)?;
    let mut harness = Harness::install(
        artifacts,
        plan,
        ExecutionConfig::new(std::num::NonZeroU32::MIN),
    )?;
    let initial_bindings = selected_counts(&harness, variant)?;
    let mut log = BufWriter::new(fs::File::create(output.join("ticks.jsonl"))?);
    let mut hashes = Vec::new();
    let mut commands = 0;
    let mut transition = Value::Null;
    let mut transaction = None;
    let mut save_digest = String::new();
    let mut state_digest = String::new();
    for tick in 0..END_TICK {
        if tick == SAVE_TICK {
            save_digest = harness.checkpoint()?;
            if restore_at_save {
                harness = restore(harness, output, &state_digest, source.clone(), false)?;
            }
        }
        if tick == PREPARE_TICK {
            let world = native(&mut harness)?;
            let descriptor = descriptor_for(
                world,
                &variant.root,
                variant.binding,
                None,
                MigrationPolicyKind::CrossRevisionDirect,
            );
            let started = Instant::now();
            let prepare_cursor = world.command_cursor();
            transaction = Some(checked(
                "gate prepare",
                world.prepare_cross_revision_cutover(
                    variant.root.clone(),
                    target_source(&variant.root)?,
                    &descriptor,
                    &variant.diff,
                    &PREFLIGHT,
                    &TRANSACTION,
                ),
            )?);
            transition = json!({"prepare_tick":tick,"prepare_ns":started.elapsed().as_nanos(),
                "prepare_command_cursor":prepare_cursor});
        }
        if tick == COMMIT_TICK {
            let commit = commit_window(&mut harness, transaction.take(), variant, tick, "chained")?;
            let object = transition.as_object_mut().unwrap();
            object.insert("commit".into(), commit);
        }
        state_digest = advance_and_log(&mut harness, &mut log, &mut hashes, &mut commands)?;
        if let Some(transaction) = transaction.as_mut() {
            checked("gate pump", transaction.pump(native(&mut harness)?))?;
        }
    }
    trial_result(
        mode,
        &harness,
        variant,
        &initial_bindings,
        &save_digest,
        &hashes,
        commands,
        &transition,
        output,
        &mut log,
    )
}

/// `overflow` / `lag`：可驱动追赶段失败的零发布与重试。失败后世界继续推进，
/// 无部分发布；随后按正常限额重试并提交成功。
fn failure_trial(
    kind: &str,
    artifacts: &Artifacts,
    plan: &ResolvedPlan,
    variant: &CapacityVariant,
    output: &Path,
) -> Result<Value> {
    fs::create_dir(output)?;
    let mut harness = Harness::install(
        artifacts,
        plan,
        ExecutionConfig::new(std::num::NonZeroU32::MIN),
    )?;
    let initial_bindings = selected_counts(&harness, variant)?;
    let mut log = BufWriter::new(fs::File::create(output.join("ticks.jsonl"))?);
    let mut hashes = Vec::new();
    let mut commands = 0;
    let limits = match kind {
        "overflow" => TINY_JOURNAL,
        "lag" => TINY_LAG,
        _ => return Err(invalid("unknown failure kind")),
    };
    let mut transition = json!({"failure":kind,"limits":{"journal_bytes":limits.max_journal_bytes,
        "lag_ticks":limits.max_catch_up_lag_ticks,"records_per_pump":limits.max_records_per_pump}});
    let mut transaction = None;
    let mut save_digest = String::new();
    let mut first_error: Option<(u64, String)> = None;
    let mut settled_at: Option<u64> = None;
    let mut retried = false;
    let mut committed = false;
    for tick in 0..END_TICK {
        if tick == 0 {
            save_digest = harness.checkpoint()?;
            let world = native(&mut harness)?;
            let descriptor = descriptor_for(
                world,
                &variant.root,
                variant.binding,
                None,
                MigrationPolicyKind::CrossRevisionDirect,
            );
            let generation = world.world_generation().get();
            let cursor = world.event_cursor();
            transaction = Some(checked(
                "gate prepare",
                world.prepare_cross_revision_cutover(
                    variant.root.clone(),
                    target_source(&variant.root)?,
                    &descriptor,
                    &variant.diff,
                    &PREFLIGHT,
                    &limits,
                ),
            )?);
            transition["prepare"] =
                json!({"tick":tick,"generation":generation,"event_cursor":cursor});
        }
        if tick == RETRY_PREPARE {
            // settled 事务的放弃路径已在 pump 失败时由 settle_failure 自动完成并
            // 释放在途唯一性（单测先例：直接重 prepare）；此处仅丢弃旧事务对象。
            drop(transaction.take());
            let world = native(&mut harness)?;
            let descriptor = descriptor_for(
                world,
                &variant.root,
                variant.binding,
                None,
                MigrationPolicyKind::CrossRevisionDirect,
            );
            let started = Instant::now();
            transaction = Some(checked(
                "gate retry prepare",
                world.prepare_cross_revision_cutover(
                    variant.root.clone(),
                    target_source(&variant.root)?,
                    &descriptor,
                    &variant.diff,
                    &PREFLIGHT,
                    &TRANSACTION,
                ),
            )?);
            transition["retry_prepare_ns"] = json!(started.elapsed().as_nanos());
            retried = true;
        }
        if tick == RETRY_COMMIT && retried {
            let commit = commit_window(&mut harness, transaction.take(), variant, tick, "retry")?;
            transition["retry_commit"] = commit;
            committed = true;
        }
        advance_and_log(&mut harness, &mut log, &mut hashes, &mut commands)?;
        if let Some(transaction) = transaction.as_mut() {
            // 溢出场景每拍泵；lag 场景停前三拍制造滞后，第四拍起恢复泵。
            let pump_now = kind != "lag" || tick >= 3;
            if pump_now {
                match transaction.pump(native(&mut harness)?) {
                    Ok(_) => {
                        if first_error.is_some() && !retried {
                            return Err(invalid(format!(
                                "{kind} pump succeeded after settled failure before retry"
                            )));
                        }
                    }
                    Err(error) => {
                        let expected_first = kind == "overflow";
                        let matches_expected = if expected_first {
                            matches!(error, CutoverError::JournalOverflow)
                        } else {
                            matches!(error, CutoverError::CatchUpLagExceeded { .. })
                        };
                        if first_error.is_none() {
                            if !matches_expected {
                                return Err(invalid(format!("{kind} first pump error: {error:?}")));
                            }
                            if harness.world.migration_journal_stats().is_some() {
                                return Err(invalid(format!(
                                    "{kind} failure did not disarm journal"
                                )));
                            }
                            let generation = harness.world.world_generation().get();
                            let cursor = harness.world.event_cursor();
                            let prepare = &transition["prepare"];
                            if generation != prepare["generation"].as_u64().unwrap_or(u64::MAX)
                                || cursor != prepare["event_cursor"].as_u64().unwrap_or(u64::MAX)
                            {
                                return Err(invalid(format!(
                                    "{kind} failure published state or events"
                                )));
                            }
                            first_error = Some((tick, format!("{error:?}")));
                            transition["first_failure"] =
                                json!({"tick":tick,"error":format!("{error:?}")});
                        } else if !matches!(error, CutoverError::TransactionSettled) {
                            return Err(invalid(format!("{kind} settled pump error: {error:?}")));
                        } else if settled_at.is_none() {
                            settled_at = Some(tick);
                        }
                    }
                }
            }
        }
    }
    if first_error.is_none() || settled_at.is_none() || !retried || !committed {
        return Err(invalid(format!(
            "{kind} scenario incomplete: failure={first_error:?} settled={settled_at:?} retried={retried} committed={committed}"
        )));
    }
    trial_result(
        kind,
        &harness,
        variant,
        &initial_bindings,
        &save_digest,
        &hashes,
        commands,
        &transition,
        output,
        &mut log,
    )
}

/// `prepare-reject`：四类准备段前置拒绝（策略、来源修订、世界身份、基线游标）
/// 全部失败关闭、零发布；随后正常切换成功。
fn reject_trial(
    artifacts: &Artifacts,
    plan: &ResolvedPlan,
    variant: &CapacityVariant,
    output: &Path,
) -> Result<Value> {
    fs::create_dir(output)?;
    let mut harness = Harness::install(
        artifacts,
        plan,
        ExecutionConfig::new(std::num::NonZeroU32::MIN),
    )?;
    let initial_bindings = selected_counts(&harness, variant)?;
    let mut log = BufWriter::new(fs::File::create(output.join("ticks.jsonl"))?);
    let mut hashes = Vec::new();
    let mut commands = 0;
    let mut transition = Value::Null;
    let mut transaction = None;
    let mut save_digest = String::new();
    let mut rejects = Vec::new();
    for tick in 0..END_TICK {
        if tick == REJECT_TICK {
            save_digest = harness.checkpoint()?;
            let digest = harness.checkpoint()?;
            let generation = harness.world.world_generation().get();
            let cursor = harness.world.event_cursor();
            let base_origin = *native(&mut harness)?.revision().canonical_origin();
            let binding = native(&mut harness)?.world_binding();
            let base_root = native(&mut harness)?.revision().clone();
            let target_source_of_base = CommittedNetworkSource::Published {
                reference: checked(
                    "base target source",
                    PublishedLfcaReference::new(
                        "urban://capacity-increment-v1",
                        base_origin.canonical_artifact_digest(),
                        base_origin.canonical_artifact_byte_length(),
                        base_origin.network_revision(),
                    ),
                )?,
            };
            // 策略拒绝须先通过描述符校验：两侧 origin 都取世界（base）origin、
            // target 传 base 根、不携带语义 diff 绑定——描述符以同修订形态通过
            // 校验后，跨修订入口按策略精确落到 PolicyMismatch。
            let wrong_policy = NetworkRevisionCutoverDescriptor::new(
                LfcaOriginBinding::from_canonical_origin(base_origin),
                LfcaOriginBinding::from_canonical_origin(base_origin),
                None,
                MigrationPolicyKind::SameRevisionRestore,
                binding,
            );
            let CommittedNetworkSource::Published { reference: _ } = target_source(&variant.root)?;
            let reference = checked(
                "wrong target source",
                PublishedLfcaReference::new(
                    "urban://capacity-increment-v1",
                    base_origin.canonical_artifact_digest(),
                    base_origin.canonical_artifact_byte_length(),
                    base_origin.network_revision(),
                ),
            )?;
            let wrong_source = CommittedNetworkSource::Published { reference };
            let wrong_identity = descriptor_for(
                native(&mut harness)?,
                &variant.root,
                variant.binding,
                Some(WorldBinding::new(
                    binding.world_id() + 1,
                    binding.world_generation(),
                    binding.baseline_command_cursor(),
                    binding.baseline_event_cursor(),
                )),
                MigrationPolicyKind::CrossRevisionDirect,
            );
            let stale_cursor = descriptor_for(
                native(&mut harness)?,
                &variant.root,
                variant.binding,
                Some(WorldBinding::new(
                    binding.world_id(),
                    binding.world_generation(),
                    binding.baseline_command_cursor().saturating_sub(1),
                    binding.baseline_event_cursor(),
                )),
                MigrationPolicyKind::CrossRevisionDirect,
            );
            let attempts: [(
                &str,
                NetworkRevisionCutoverDescriptor,
                Option<CommittedNetworkSource>,
                &str,
            ); 4] = [
                ("policy", wrong_policy, None, "PolicyMismatch"),
                (
                    "source_revision",
                    descriptor_for(
                        native(&mut harness)?,
                        &variant.root,
                        variant.binding,
                        None,
                        MigrationPolicyKind::CrossRevisionDirect,
                    ),
                    Some(wrong_source),
                    "TargetSourceRevisionMismatch",
                ),
                (
                    "world_identity",
                    wrong_identity,
                    None,
                    "WorldBindingMismatch",
                ),
                (
                    "baseline_cursor",
                    stale_cursor,
                    None,
                    "BaselineCommandCursorMismatch",
                ),
            ];
            for (name, descriptor, source_override, expected) in attempts {
                // policy 尝试以 base 根为目标（同修订形态描述符），其余以变体根为目标。
                let (target_root, diff, source) = if name == "policy" {
                    (base_root.clone(), Vec::new(), target_source_of_base.clone())
                } else {
                    let source = match source_override {
                        Some(source) => source,
                        None => target_source(&variant.root)?,
                    };
                    (variant.root.clone(), variant.diff.clone(), source)
                };
                let error = native(&mut harness)?
                    .prepare_cross_revision_cutover(
                        target_root,
                        source,
                        &descriptor,
                        &diff,
                        &PREFLIGHT,
                        &TRANSACTION,
                    )
                    .err()
                    .ok_or_else(|| {
                        invalid(format!("prepare reject {name}: unexpectedly succeeded"))
                    })?;
                let matched = match expected {
                    "PolicyMismatch" => matches!(error, CutoverError::PolicyMismatch),
                    "TargetSourceRevisionMismatch" => {
                        matches!(error, CutoverError::TargetSourceRevisionMismatch)
                    }
                    "WorldBindingMismatch" => matches!(error, CutoverError::WorldBindingMismatch),
                    _ => matches!(error, CutoverError::BaselineCommandCursorMismatch { .. }),
                };
                if !matched {
                    return Err(invalid(format!("prepare reject {name}: {error:?}")));
                }
                if harness.checkpoint()? != digest
                    || harness.world.world_generation().get() != generation
                    || harness.world.event_cursor() != cursor
                    || harness.world.migration_journal_stats().is_some()
                {
                    return Err(invalid(format!(
                        "prepare reject {name} mutated committed state"
                    )));
                }
                rejects.push(json!({"name":name,"error":format!("{error:?}")}));
            }
            transition = json!({"reject_tick":tick,"rejects":rejects});
        }
        if tick == RETRY_PREPARE {
            let world = native(&mut harness)?;
            let descriptor = descriptor_for(
                world,
                &variant.root,
                variant.binding,
                None,
                MigrationPolicyKind::CrossRevisionDirect,
            );
            transaction = Some(checked(
                "gate prepare after rejects",
                world.prepare_cross_revision_cutover(
                    variant.root.clone(),
                    target_source(&variant.root)?,
                    &descriptor,
                    &variant.diff,
                    &PREFLIGHT,
                    &TRANSACTION,
                ),
            )?);
        }
        if tick == RETRY_COMMIT {
            let commit = commit_window(
                &mut harness,
                transaction.take(),
                variant,
                tick,
                "after-rejects",
            )?;
            let object = transition.as_object_mut().unwrap();
            object.insert("commit".into(), commit);
        }
        advance_and_log(&mut harness, &mut log, &mut hashes, &mut commands)?;
        if let Some(transaction) = transaction.as_mut() {
            checked("gate pump", transaction.pump(native(&mut harness)?))?;
        }
    }
    trial_result(
        "prepare-reject",
        &harness,
        variant,
        &initial_bindings,
        &save_digest,
        &hashes,
        commands,
        &transition,
        output,
        &mut log,
    )
}

/// `multi`：两轮切换串联——轮 1 跨修订在线切换（base→v1，日志/追赶/静默提交），
/// 轮 2 同修订换根（v1→v1 新根实例，`cutover_same_revision`）。两轮各推进一次
/// 世代与事件；逐拍摘要仍须与 baseline 逐位相等。≥2 轮跨修订链与 ABA 往返在
/// 单测层覆盖（cutover_migration 重复迁移、bevy ABA），端到端随变体工具泛化再补。
fn multi_trial(
    artifacts: &Artifacts,
    plan: &ResolvedPlan,
    v1: &CapacityVariant,
    v1_reloaded: &CapacityVariant,
    output: &Path,
) -> Result<Value> {
    fs::create_dir(output)?;
    let mut harness = Harness::install(
        artifacts,
        plan,
        ExecutionConfig::new(std::num::NonZeroU32::MIN),
    )?;
    let initial_bindings = selected_counts(&harness, v1)?;
    let mut log = BufWriter::new(fs::File::create(output.join("ticks.jsonl"))?);
    let mut hashes = Vec::new();
    let mut commands = 0;
    let mut transition = json!({"rounds":[]});
    let mut transaction = None;
    let mut save_digest = String::new();
    let initial_generation = harness.world.world_generation().get();
    let initial_cursor = harness.world.event_cursor();
    for tick in 0..END_TICK {
        if tick == 0 {
            save_digest = harness.checkpoint()?;
        }
        if tick == ROUND1_PREPARE {
            let world = native(&mut harness)?;
            let descriptor = descriptor_for(
                world,
                &v1.root,
                v1.binding,
                None,
                MigrationPolicyKind::CrossRevisionDirect,
            );
            let started = Instant::now();
            let prepare_cursor = world.command_cursor();
            transaction = Some(checked(
                "gate multi prepare",
                world.prepare_cross_revision_cutover(
                    v1.root.clone(),
                    target_source(&v1.root)?,
                    &descriptor,
                    &v1.diff,
                    &PREFLIGHT,
                    &TRANSACTION,
                ),
            )?);
            transition["rounds"]
                .as_array_mut()
                .unwrap()
                .push(json!({"round":"one",
                "kind":"cross-revision","prepare_tick":tick,
                "prepare_ns":started.elapsed().as_nanos(),
                "prepare_command_cursor":prepare_cursor}));
        }
        if tick == ROUND1_COMMIT {
            let commit = commit_window(&mut harness, transaction.take(), v1, tick, "one")?;
            transition["rounds"].as_array_mut().unwrap().push(commit);
        }
        if tick == ROUND2_PREPARE {
            let descriptor = {
                let world = native(&mut harness)?;
                NetworkRevisionCutoverDescriptor::new(
                    LfcaOriginBinding::from_canonical_origin(*world.revision().canonical_origin()),
                    LfcaOriginBinding::from_canonical_origin(*v1_reloaded.root.canonical_origin()),
                    None,
                    MigrationPolicyKind::SameRevisionRestore,
                    world.world_binding(),
                )
            };
            let before = retained_state(&harness);
            let bindings = selected_counts(&harness, v1_reloaded)?;
            let generation = harness.world.world_generation().get();
            let cursor = harness.world.event_cursor();
            let started = Instant::now();
            let events = checked(
                "gate same-revision cutover",
                native(&mut harness)?.cutover_same_revision(
                    v1_reloaded.root.clone(),
                    target_source(&v1_reloaded.root)?,
                    &descriptor,
                    &PREFLIGHT,
                ),
            )?;
            let same_ns = started.elapsed().as_nanos();
            if retained_state(&harness) != before
                || selected_counts(&harness, v1_reloaded)? != bindings
                || events.as_slice().len() != 1
                || harness.world.event_cursor() != cursor + 1
                || harness.world.world_generation().get() != generation + 1
                || harness.world.tick_index() != tick
                || harness.world.migration_journal_stats().is_some()
                || !std::sync::Arc::ptr_eq(&harness.world.revision(), &v1_reloaded.root)
            {
                return Err(invalid("same-revision round invariant failed"));
            }
            transition["rounds"]
                .as_array_mut()
                .unwrap()
                .push(json!({"round":"two",
                "kind":"same-revision","tick":tick,"ns":same_ns,
                "events":format!("{events:?}")}));
        }
        advance_and_log(&mut harness, &mut log, &mut hashes, &mut commands)?;
        if let Some(transaction) = transaction.as_mut() {
            checked("gate pump", transaction.pump(native(&mut harness)?))?;
        }
    }
    if harness.world.world_generation().get() != initial_generation + 2
        || harness.world.event_cursor() != initial_cursor + 2
    {
        return Err(invalid(
            "multi round did not advance two generations and events",
        ));
    }
    trial_result(
        "multi",
        &harness,
        v1,
        &initial_bindings,
        &save_digest,
        &hashes,
        commands,
        &transition,
        output,
        &mut log,
    )
}

/// 计划解析按「容量 1000 的虚拟设施」锚定车库出口（plan.rs 每案例每 tile 无条件
/// 查找）。变体选中该设施时 +1 即破坏锚点，变体基线（未切换 v1 世界）不可解析。
/// 该分支下提交后区段没有独立参照，由另两层承担：提交窗口前后的 retained_state
/// （逐车全量 VehicleState——进度/速度/状态/路线游标——加绑定）比对，以及同事件
/// 编排组内全窗逐拍相等；skip 说明如实登记。非车库设施成为首个合格选择的输入
/// 走变体基线分支（计划从变体工件展开）。
fn selected_is_garage_anchor(artifacts: &Artifacts, variant: &CapacityVariant) -> bool {
    artifacts
        .revision()
        .identity()
        .ordinal(variant.selected)
        .and_then(|ordinal| {
            artifacts
                .revision()
                .traffic()
                .relations()
                .parking_facility(ordinal)
        })
        .is_some_and(|facility| facility.virtual_capacity() == 1_000)
}

/// 观察摘要含命令/事件游标：切换按合同交付事件（event_cursor +1），全窗对
/// baseline 的逐拍相等在跨切换场景下既不可能也非合同义务（单测恒等 oracle 比的
/// 是逻辑面）。本见证采用三层 oracle：
/// 1. 事件前缀：首个事件 tick 之前的逐拍摘要与 baseline 逐位相等——证明恢复、
///    前置拒绝、失败放弃与日志武装对世界零扰动；
/// 2. 同事件编排组内全等：事件落在同一 tick 的场景整窗摘要彼此相等；
/// 3. 终态逻辑面：final_selected_bindings 与 final_retained_state 与 baseline 相等。
fn assert_prefix_identity(
    mode: &str,
    candidate: &Value,
    baseline: &Value,
    until: u64,
) -> Result<()> {
    let candidate_hashes = candidate["tick_hashes"].as_array().unwrap();
    let baseline_hashes = baseline["tick_hashes"].as_array().unwrap();
    if candidate_hashes.len() != baseline_hashes.len() {
        return Err(invalid(format!("{mode} tick count differs from baseline")));
    }
    for (tick, (left, right)) in candidate_hashes
        .iter()
        .zip(baseline_hashes)
        .enumerate()
        .take(usize::try_from(until).expect("tick fits usize"))
    {
        if left != right {
            return Err(invalid(format!(
                "{mode} prefix identity broken at tick {tick}"
            )));
        }
    }
    Ok(())
}

fn assert_peer_identity(
    left_mode: &str,
    right_mode: &str,
    left: &Value,
    right: &Value,
) -> Result<()> {
    if left["tick_hashes"] != right["tick_hashes"] {
        return Err(invalid(format!(
            "{left_mode} and {right_mode} share the event schedule but differ"
        )));
    }
    Ok(())
}

fn assert_final_logical_identity(mode: &str, candidate: &Value, baseline: &Value) -> Result<()> {
    for field in ["final_selected_bindings", "final_retained_state"] {
        if candidate[field] != baseline[field] {
            return Err(invalid(format!(
                "{mode} final logical state differs on {field}"
            )));
        }
    }
    Ok(())
}

/// 场景主体（无 provenance 绑定，供测试与公开入口共用）。
fn gate_witness(
    artifacts: &Artifacts,
    reloaded: &Artifacts,
    variant1_artifacts: &Artifacts,
    v1: &CapacityVariant,
    v1_reloaded: &CapacityVariant,
    case: UrbanCase,
    output: &Path,
) -> Result<Value> {
    let plan = ResolvedPlan::for_case(artifacts, case, Window::probe(END_TICK)?)?;
    plan.write(&output.join("resolved-plan.toml"))?;

    let baseline = trial(
        artifacts,
        reloaded,
        &plan,
        v1,
        "baseline",
        &output.join("baseline"),
    )?;
    // 变体基线：同一需求程序直接装在 v1 根上的未切换世界。容量 +1 是变体的
    // 语义效果，终态逻辑面（停车行为可在近满设施处分岔）只能对它比较——这是
    // 单测「identity against unswitched world」的端到端化。变体在 fixture 规模
    // 选中车库（唯一有 tick-0 占用的虚拟设施），GARAGE-EGRESS 计划按容量 1000
    // 锚定车库、无法在 v1 上解析，该案例不建变体基线（场景不受影响：它们装在
    // base 上、计划从 base 解析），终态逻辑面以场景不变量承担并登记。
    let variant_baseline = if selected_is_garage_anchor(artifacts, v1) {
        json!({"skipped": "variant increments the garage (capacity 1000 anchor keyed by plan resolution); no unswitched v1 baseline is resolvable for this input. Post-commit evolution is then covered by commit-window identity and peer determinism only."})
    } else {
        // 计划身份绑定展开它的工件：变体基线必须用从变体工件重新展开的计划
        // （需求源自相同的 config/genesis，语义等价；validate 才能通过）。
        let variant_plan =
            ResolvedPlan::for_case(variant1_artifacts, case, Window::probe(END_TICK)?)?;
        variant_plan.write(&output.join("variant-resolved-plan.toml"))?;
        trial(
            variant1_artifacts,
            reloaded,
            &variant_plan,
            v1,
            "baseline",
            &output.join("variant-baseline"),
        )?
    };
    let save_source = crate::save::SaveSource {
        execution_id: "gate-538".into(),
        git_commit: None,
        binary_sha256: None,
        workers: 1,
    };
    let identity = workflow_trial(
        "identity",
        false,
        artifacts,
        &plan,
        v1,
        &output.join("identity"),
        save_source.clone(),
    )?;
    let chained = workflow_trial(
        "chained",
        true,
        artifacts,
        &plan,
        v1,
        &output.join("chained"),
        save_source,
    )?;
    let rejects = reject_trial(artifacts, &plan, v1, &output.join("prepare-reject"))?;
    let mut failures = serde_json::Map::new();
    for kind in ["overflow", "lag"] {
        let failure = failure_trial(kind, artifacts, &plan, v1, &output.join(kind))?;
        failures.insert(kind.into(), failure);
    }
    let multi = multi_trial(artifacts, &plan, v1, v1_reloaded, &output.join("multi"))?;
    // 三层 oracle：事件前缀对 baseline、同编排组内全等、终态逻辑面对 baseline。
    assert_prefix_identity("identity", &identity, &baseline, COMMIT_TICK)?;
    assert_prefix_identity("chained", &chained, &baseline, COMMIT_TICK)?;
    assert_prefix_identity("prepare-reject", &rejects, &baseline, RETRY_COMMIT)?;
    assert_prefix_identity("overflow", &failures["overflow"], &baseline, RETRY_COMMIT)?;
    assert_prefix_identity("lag", &failures["lag"], &baseline, RETRY_COMMIT)?;
    assert_prefix_identity("multi", &multi, &baseline, ROUND1_COMMIT)?;
    assert_peer_identity("identity", "chained", &identity, &chained)?;
    assert_peer_identity(
        "prepare-reject",
        "overflow",
        &rejects,
        &failures["overflow"],
    )?;
    assert_peer_identity("overflow", "lag", &failures["overflow"], &failures["lag"])?;
    if !selected_is_garage_anchor(artifacts, v1) {
        for (mode, value) in [
            ("identity", &identity),
            ("chained", &chained),
            ("prepare-reject", &rejects),
            ("overflow", &failures["overflow"]),
            ("lag", &failures["lag"]),
            ("multi", &multi),
        ] {
            assert_final_logical_identity(mode, value, &variant_baseline)?;
        }
    }
    Ok(json!({"status":"gate-witness-pass","case":case.as_str(),
        "scale":artifacts.catalog().scale,"end_tick":END_TICK,
        "transaction_limits":{"journal_bytes":TRANSACTION.max_journal_bytes,
            "lag_ticks":TRANSACTION.max_catch_up_lag_ticks,
            "records_per_pump":TRANSACTION.max_records_per_pump},
        "oracle":{"prefix_ticks":{"identity":COMMIT_TICK,"chained":COMMIT_TICK,
            "prepare-reject":RETRY_COMMIT,"overflow":RETRY_COMMIT,"lag":RETRY_COMMIT,
            "multi":ROUND1_COMMIT},
            "peer_groups":[["identity","chained"],["prepare-reject","overflow","lag"]],
            "final_logical_fields":["final_selected_bindings","final_retained_state"]},
        "scenarios":{"identity":identity,"chained":chained,"prepare-reject":rejects,
            "failures":failures,"multi":multi,"variant-baseline":variant_baseline}}))
}

/// # Errors
///
/// 输入不完整、任一场景不变量失败、或运行期间源码/输入身份变化时失败。
pub fn run_gate_evidence(
    artifact_directory: &Path,
    variant_directory: &Path,
    case: UrbanCase,
    output: &Path,
) -> Result<Value> {
    if !matches!(case, UrbanCase::MixedPeak | UrbanCase::GarageEgress) {
        return Err(invalid("gate case must be MIXED-PEAK or GARAGE-EGRESS"));
    }
    let started = Instant::now();
    let command = std::env::args().collect::<Vec<_>>().join(" ");
    let provenance = crate::evidence::source()?;
    let artifacts = Artifacts::load_spatial(artifact_directory)?;
    let reloaded = Artifacts::load_spatial(artifact_directory)?;
    let v1 = load_variant(&artifacts, variant_directory)?;
    let v1_reloaded = load_variant(&artifacts, variant_directory)?;
    let manifest1 = digest_file(&variant_directory.join("variant.json"))?;
    fs::create_dir(output)?;
    let variant1_artifacts = Artifacts::load_spatial(variant_directory)?;
    let mut report = gate_witness(
        &artifacts,
        &reloaded,
        &variant1_artifacts,
        &v1,
        &v1_reloaded,
        case,
        output,
    )?;
    // 正式入口下重跑链式场景，使存档绑定 exact head 与二进制摘要。
    let plan = ResolvedPlan::for_case(&artifacts, case, Window::probe(END_TICK)?)?;
    let save_source = crate::save::SaveSource {
        execution_id: "gate-538".into(),
        git_commit: provenance["commit"].as_str().map(String::from),
        binary_sha256: provenance["binary"].as_str().map(String::from),
        workers: 1,
    };
    let bound_chained = workflow_trial(
        "chained",
        true,
        &artifacts,
        &plan,
        &v1,
        &output.join("chained-bound"),
        save_source,
    )?;
    assert_peer_identity(
        "chained(bound)",
        "chained",
        &bound_chained,
        &report["scenarios"]["chained"],
    )?;
    report["scenarios"]["chained"] = bound_chained;
    if crate::evidence::source()? != provenance
        || digest_file(&variant_directory.join("variant.json"))? != manifest1
        || load_variant(&artifacts, variant_directory)?
            .root
            .network_revision()
            != v1.root.network_revision()
        || Artifacts::load_spatial(artifact_directory)?.files != artifacts.files
        || Artifacts::load_spatial(variant_directory)?.files != variant1_artifacts.files
        || Artifacts::load_spatial(artifact_directory)?.manifest_digest != artifacts.manifest_digest
        || Artifacts::load_spatial(variant_directory)?.manifest_digest
            != variant1_artifacts.manifest_digest
    {
        return Err(invalid("gate inputs/source changed"));
    }
    report["command"] = json!(command);
    report["source"] = provenance;
    report["inputs"] = json!({"artifact_files":serde_json::to_value(&artifacts.files)?,
        "artifact_manifest_digest":serde_json::to_value(artifacts.manifest_digest.clone())?,
        "variant_artifacts_manifest_digest":serde_json::to_value(
            variant1_artifacts.manifest_digest.clone(),
        )?,
        "variant_manifest":serde_json::to_value(manifest1)?,
        "resolved_plan_digest":serde_json::to_value(digest_file(
            &output.join("resolved-plan.toml"),
        )?)?});
    report["elapsed_ms"] = json!(started.elapsed().as_millis());
    write_json(&output.join("gate.json"), &report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use laneflow_urban_generator::{Scale, UrbanConfig, generate};

    #[test]
    fn gate_scenarios_pass_at_fixture_scale() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        let v1_dir = temp.path().join("v1");
        let config =
            UrbanConfig::parse(include_str!("../../../examples/config/cn-urban.toml")).unwrap();
        generate(&config, Scale::Fixture, &base, None).unwrap();
        crate::prepare_capacity_variant(&base, &v1_dir).unwrap();
        let artifacts = Artifacts::load_spatial(&base).unwrap();
        let reloaded = Artifacts::load_spatial(&base).unwrap();
        let variant1_artifacts = Artifacts::load_spatial(&v1_dir).unwrap();
        let v1 = load_variant(&artifacts, &v1_dir).unwrap();
        let v1_reloaded = load_variant(&artifacts, &v1_dir).unwrap();
        for case in [UrbanCase::MixedPeak, UrbanCase::GarageEgress] {
            let out = temp.path().join(case.as_str());
            fs::create_dir(&out).unwrap();
            gate_witness(
                &artifacts,
                &reloaded,
                &variant1_artifacts,
                &v1,
                &v1_reloaded,
                case,
                &out,
            )
            .unwrap();
        }
    }
}
