//! #538 快照与在线切换 Gate 端到端见证。
//!
//! 在 `transitions` 试用机械上补齐 #538 登记的认证缺口面：
//! - 跨切换逻辑恒等：`identity`（在线切换的事件前缀逐拍摘要与 baseline 逐位相等；
//!   整窗恒等由 identity↔chained 同事件编排组内全等承担——观察摘要含事件游标，
//!   交付事件后全窗对 baseline 相等既不可能也非合同义务）；
//! - 完整工作流串联：`chained`（保存→恢复→重放→在线切换→续跑）；
//! - 可驱动失败面的零发布：`prepare-reject`（策略/来源修订/世界身份/基线游标四类
//!   前置拒绝）与 `overflow`/`lag`（追赶段日志字节与滞后超限），失败后世界继续
//!   推进、无部分发布，且可重试成功；
//! - 多轮切换：`multi`（base→v1 跨修订在线切换 + v1→v1 换新根对象的同修订换根两轮串联）。
//!
//! 执行回执（`gate.json`）绑定 exact head、命令、输入身份与逐场景结果；scope 与
//! timing_basis 字段声明本见证不覆盖的范围与计时口径。

use std::{fs, io::Write, path::Path, time::Instant};

use laneflow_runtime::*;
use laneflow_static_network::SharedNetworkRevision;
use serde_json::{Value, json};

use crate::{
    Artifacts, Harness, ResolvedPlan, Result, UrbanCase, Window, checked, invalid,
    report::{digest_file, write_json},
    transitions::{native, restore, retained_state, selected_counts, target_source, trial},
    variant::{CapacityVariant, load_variant},
    written::DigestWriter,
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
    let world = native(harness)?;
    let started = Instant::now();
    let commit = checked(
        "gate commit",
        transaction
            .take()
            .ok_or_else(|| invalid("missing transaction"))?
            .commit(world),
    )?;
    // 计时只覆盖提交调用本身；后续 O(N) 不变量验证在窗外。
    let commit_ns = started.elapsed().as_nanos();
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
        json!({"round":round,"commit_tick":tick,"commit_ns":commit_ns,
            "events":format!("{:?}", commit.events),"final_command_cursor":commit.final_command_cursor,
            "journal_before_commit":journal_before,"selected_binding_counts":bindings}),
    )
}

/// 逐拍记录骨架：advance、命令/事件与状态摘要落盘，供场景间逐位比较。
fn advance_and_log(
    harness: &mut Harness<'_>,
    log: &mut DigestWriter,
    hashes: &mut Vec<String>,
    commands: &mut usize,
) -> Result<String> {
    let record = harness.advance()?;
    let state_digest = record.state_digest.clone();
    *commands += harness.commands.len();
    hashes.push(crate::digest::Domain::TrialTick.hex(&serde_json::to_vec(&record)?));
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
    save_tick: u64,
    save_digest: &str,
    hashes: &[String],
    command_records: usize,
    transition: &Value,
    output: &Path,
    log: DigestWriter,
) -> Result<Value> {
    // 摘要取自写出时的字节，刷盘后经读回核对（#1042）。
    let ticks_file = log.finish()?;
    let result = json!({"mode":mode,"ticks":END_TICK,"save_tick":save_tick,
        "initial_selected_bindings":initial_bindings,
        "final_selected_bindings":selected_counts(harness, variant)?,
        "save_digest":save_digest,"final_digest":harness.checkpoint()?,
        "final_retained_state":retained_state(harness),
        "tick_hashes":hashes,"transition":transition,
        "command_records":command_records,"tile_evidence":harness.evidence,
        "retry_reasons":harness.error_counts,
        "ticks_file":ticks_file});
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
    let mut log = DigestWriter::create(&output.join("ticks.jsonl"))?;
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
                harness = restore(harness, output, &state_digest, source.clone())?;
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
            let commit = commit_window(&mut harness, transaction.take(), variant, tick, mode)?;
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
        SAVE_TICK,
        &save_digest,
        &hashes,
        commands,
        &transition,
        output,
        log,
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
    let mut log = DigestWriter::create(&output.join("ticks.jsonl"))?;
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
        0,
        &save_digest,
        &hashes,
        commands,
        &transition,
        output,
        log,
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
    let mut log = DigestWriter::create(&output.join("ticks.jsonl"))?;
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
            let CommittedNetworkSource::Published { reference: _ } = target_source(&variant.root)?
            else {
                return Err(invalid("target source 必须是发布来源".to_owned()));
            };
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
        REJECT_TICK,
        &save_digest,
        &hashes,
        commands,
        &transition,
        output,
        log,
    )
}

/// `multi`：两轮切换串联——轮 1 跨修订在线切换（base→v1，日志/追赶/静默提交），
/// 轮 2 同修订换根（v1→v1 新根对象，`cutover_same_revision`）。两轮各推进一次
/// 世代与事件；逐拍 oracle 为事件前缀（至轮 1 提交 tick）对 baseline 相等，轮 2
/// 后无逐拍 oracle，由场景不变量承担。≥2 轮跨修订链与 ABA 往返在单测/集成层
/// 覆盖（runtime 连续 re-cutover 集成测试、bevy ABA），不经本见证认证。
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
    let mut log = DigestWriter::create(&output.join("ticks.jsonl"))?;
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
            let world = native(&mut harness)?;
            let started = Instant::now();
            let events = checked(
                "gate same-revision cutover",
                world.cutover_same_revision(
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
        0,
        &save_digest,
        &hashes,
        commands,
        &transition,
        output,
        log,
    )
}

/// 观察摘要含命令/事件游标：切换按合同交付事件（event_cursor +1），全窗对
/// baseline 的逐拍相等在跨切换场景下既不可能也非合同义务（单测恒等 oracle 比的
/// 是逻辑面）。本见证采用两层逐拍 oracle 加场景级终态不变量：其一，事件前缀
/// （首个事件 tick 之前的逐拍摘要与 baseline 逐位相等——证明恢复、前置拒绝、
/// 失败放弃与日志武装对世界零扰动）；其二，同事件编排组内全等（事件落在同一
/// tick 的场景整窗摘要彼此相等）；终态由场景不变量承担（commit_window 在每次
/// 切换前后比对逐车全量 VehicleState，终态字段逐场景落盘供人工比对）。
/// 「未切换变体基线」（同一需求装在 v1 根上）在当前输入族不可构造：生成器
/// 家族中 virtual-only 设施仅有车库（t{tile}.underground，容量 1000），变体必选
/// 车库；而计划解析按容量 100/1000 字面哨兵锚定车库并派生初始停车人口
/// （plan.rs），容量 +1 后计划在 v1 上不可展开。构造该基线需要生成器提供
/// virtual-only 非车库设施或显式人口规格，属后续输入设计，不在本见证。
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

/// 场景主体（无 provenance 绑定，供测试与公开入口共用）。
fn gate_witness(
    artifacts: &Artifacts,
    reloaded: &Artifacts,
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
    // 未切换变体基线：不可构造（见上方 oracle 注释——变体必选车库，容量哨兵
    // 使计划在 v1 上不可展开），回执登记该约束，终态由场景不变量承担。
    let variant_baseline = json!({"skipped": "unswitched v1 baseline is not constructible in the \
current input family: the generator's only virtual-only facility is the garage (capacity 1000), \
which the variant necessarily selects, and plan resolution anchors garage/parking populations on \
the 100/1000 capacity sentinels, so the plan cannot expand on v1 (see gate.rs oracle notes)"});
    let save_source = crate::save::SaveSource {
        execution_id: "gate-538".into(),
        git_commit: None,
        binary_digest: None,
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
    // 两层逐拍 oracle：事件前缀对 baseline、同事件编排组内全等；终态由场景
    // 不变量承担（commit_window 前后 retained_state 比对，终态字段逐场景落盘）。
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
    Ok(json!({"status":"gate-witness-pass","case":case.as_str(),
        "scale":artifacts.catalog().scale,"end_tick":END_TICK,
        "variant_target_revision":format!("{:x}", v1.root.network_revision().into_digest()),
        "scope":"snapshot/online-cutover correctness witness for the listed scenarios only; \
>=2-round cross-revision chains and ABA round trips are unit/integration covered \
(runtime continuous re-cutover, bevy ABA), not certified here; observation/routing seam \
costs live in the seams witness; timings below are single-shot wall-clock references, \
not product evidence; Editable-source restore is deferred to #529",
        "artifacts_note":"per-trial directories are authoritative except 'chained' (witness-stage saves with unbound provenance; the receipt binds the 'chained-bound' rerun)",
        "timing_basis":"single-shot Instant-based wall clock around the named call only; \
not comparable across machines; percentiles are not additive",
        "transaction_limits":{"journal_bytes":TRANSACTION.max_journal_bytes,
            "lag_ticks":TRANSACTION.max_catch_up_lag_ticks,
            "records_per_pump":TRANSACTION.max_records_per_pump},
        "oracle":{"prefix_ticks":{"identity":COMMIT_TICK,"chained":COMMIT_TICK,
            "prepare-reject":RETRY_COMMIT,"overflow":RETRY_COMMIT,"lag":RETRY_COMMIT,
            "multi":ROUND1_COMMIT},
            "peer_groups":[["identity","chained"],["prepare-reject","overflow","lag"]],
            "final_logical":"scenario invariants (retained_state across each commit window) \
plus per-scenario final fields; unswitched variant baseline not constructible (see \
variant-baseline note)"},
        "scenarios":{"identity":identity,"chained":chained,"prepare-reject":rejects,
            "failures":failures,"multi":multi,"variant-baseline":variant_baseline}}))
}

/// 运行前后输入身份复核（纯比较，供单测覆盖公开入口的绑定链语义）。
#[allow(clippy::too_many_arguments)]
fn verify_gate_inputs(
    source_after: &Value,
    source_before: &Value,
    variant_manifest_after: &crate::digest::FileDigest,
    variant_manifest_before: &crate::digest::FileDigest,
    variant_revision_after: laneflow_static_contract::NetworkRevisionId,
    variant_revision_before: laneflow_static_contract::NetworkRevisionId,
    artifact_files_after: &std::collections::BTreeMap<String, crate::artifacts::ArtifactFileDigest>,
    artifact_files_before: &std::collections::BTreeMap<
        String,
        crate::artifacts::ArtifactFileDigest,
    >,
    variant_files_after: &std::collections::BTreeMap<String, crate::artifacts::ArtifactFileDigest>,
    variant_files_before: &std::collections::BTreeMap<String, crate::artifacts::ArtifactFileDigest>,
    artifact_manifest_after: &str,
    artifact_manifest_before: &str,
    variant_manifest_digest_after: &str,
    variant_manifest_digest_before: &str,
) -> Result<()> {
    if source_after != source_before
        || variant_manifest_after != variant_manifest_before
        || variant_revision_after != variant_revision_before
        || artifact_files_after != artifact_files_before
        || variant_files_after != variant_files_before
        || artifact_manifest_after != artifact_manifest_before
        || variant_manifest_digest_after != variant_manifest_digest_before
    {
        return Err(invalid("gate inputs/source changed"));
    }
    Ok(())
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
    let mut report = gate_witness(&artifacts, &reloaded, &v1, &v1_reloaded, case, output)?;
    // 正式入口下重跑链式场景，使存档绑定 exact head 与二进制摘要。
    let plan = ResolvedPlan::for_case(&artifacts, case, Window::probe(END_TICK)?)?;
    let save_source = crate::save::SaveSource {
        execution_id: "gate-538".into(),
        git_commit: provenance["commit"].as_str().map(String::from),
        binary_digest: provenance["binary"]["digest"].as_str().map(String::from),
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
    verify_gate_inputs(
        &crate::evidence::source()?,
        &provenance,
        &digest_file(&variant_directory.join("variant.json"))?,
        &manifest1,
        load_variant(&artifacts, variant_directory)?
            .root
            .network_revision(),
        v1.root.network_revision(),
        &Artifacts::load_spatial(artifact_directory)?.files,
        &artifacts.files,
        &Artifacts::load_spatial(variant_directory)?.files,
        &variant1_artifacts.files,
        &Artifacts::load_spatial(artifact_directory)?.manifest_digest,
        &artifacts.manifest_digest,
        &Artifacts::load_spatial(variant_directory)?.manifest_digest,
        &variant1_artifacts.manifest_digest,
    )?;
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
        let v1 = load_variant(&artifacts, &v1_dir).unwrap();
        let v1_reloaded = load_variant(&artifacts, &v1_dir).unwrap();
        for case in [UrbanCase::MixedPeak, UrbanCase::GarageEgress] {
            let out = temp.path().join(case.as_str());
            fs::create_dir(&out).unwrap();
            gate_witness(&artifacts, &reloaded, &v1, &v1_reloaded, case, &out).unwrap();
        }
    }

    #[test]
    fn verify_gate_inputs_rejects_each_drift() {
        let files = std::collections::BTreeMap::new();
        let revision = v1_revision_of_fixture();
        let manifest = crate::digest::FileDigest {
            bytes: 1,
            digest: String::new(),
        };
        let check = |source_after: &Value, variant_manifest_after: &crate::digest::FileDigest| {
            verify_gate_inputs(
                source_after,
                &json!({"commit": "c"}),
                variant_manifest_after,
                &manifest,
                revision,
                revision,
                &files,
                &files,
                &files,
                &files,
                "am",
                "am",
                "vm",
                "vm",
            )
            .is_ok()
        };
        let same = json!({"commit": "c"});
        assert!(check(&same, &manifest.clone()));
        assert!(!check(&json!({"commit": "other"}), &manifest.clone()));
        assert!(!check(
            &same,
            &crate::digest::FileDigest {
                bytes: 2,
                digest: String::new()
            }
        ));
    }

    fn v1_revision_of_fixture() -> laneflow_static_contract::NetworkRevisionId {
        // 合成修订标识即可：该单测只比较"同值/异值"，不依赖真实网络。
        laneflow_static_contract::NetworkRevisionId::from_digest(
            laneflow_static_contract::Sha256Digest::from_bytes([7; 32]),
        )
    }
}
