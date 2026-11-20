//! 验证层独立计量入口；交通语义、角色见证和规范日志仍由原 runner/report 负责。

use std::{fs, path::Path, time::Instant};

use laneflow_runtime::ExecutionConfig;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{Artifacts, Diagnostics, Harness, ResolvedPlan, Result, RunOptions, invalid, report};

#[cfg(feature = "verification-cost-counts")]
pub(crate) mod counts;

const FILE: &str = "verification-cost.json";
const OBSERVATION_SCOPE: &str = "existing observation_ns: pre_step snapshots, intent and red waiters; post-step signal/arrival processing, events and state checks/encoding; excludes boundary commands, Runtime step, TickRecord event/command digest encoding, logs, checkpoints and cost bookkeeping";
const WALL_SCOPE: &str = "run_with_options: plan validation, output creation, initial incomplete cost marker, Harness installation, complete warm-up and observation, original logs/checkpoints/result finalization; excludes input loading, cost sample preallocation, allocator probe, host environment capture and final cost report serialization";

/// 时间与分配/工作量分别构建、分别运行；计数构建的时间不可用于性能判断。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationCostMode {
    Timing,
    Counts,
}

impl std::str::FromStr for VerificationCostMode {
    type Err = crate::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "timing" => Ok(Self::Timing),
            "counts" => Ok(Self::Counts),
            _ => Err(invalid("verification-cost 模式必须为 timing 或 counts")),
        }
    }
}

impl VerificationCostMode {
    fn label(self) -> &'static str {
        match self {
            Self::Timing => "timing",
            Self::Counts => "counts",
        }
    }

    fn validate_build(self) -> Result<()> {
        if self == Self::Timing
            && cfg!(any(
                feature = "verification-cost-counts",
                feature = "allocation",
                feature = "execution-evidence",
                feature = "motion-kernel-evidence",
                feature = "pose-profiling"
            ))
        {
            return Err(invalid(
                "verification-cost timing 需要未启用计数或性能分析 feature 的独立构建",
            ));
        }
        if self == Self::Counts && !cfg!(feature = "verification-cost-counts") {
            return Err(invalid(
                "verification-cost counts 需要启用 verification-cost-counts feature",
            ));
        }
        if self == Self::Counts
            && cfg!(any(
                feature = "allocation",
                feature = "execution-evidence",
                feature = "motion-kernel-evidence",
                feature = "pose-profiling"
            ))
        {
            return Err(invalid(
                "verification-cost counts 需要独立计数构建，不得混入其他性能分析 feature",
            ));
        }
        #[cfg(feature = "verification-cost-counts")]
        if self == Self::Counts {
            counts::require_allocator()?;
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct TimingSample {
    tick: u64,
    observation_ns: u64,
}

#[cfg(feature = "verification-cost-counts")]
#[derive(Serialize)]
struct CountSample {
    tick: u64,
    workload_counts: counts::WorkloadCounts,
    allocation: counts::ObservationAllocation,
}

pub(crate) struct Recorder {
    mode: VerificationCostMode,
    warm_up_ticks: u64,
    timing: Vec<TimingSample>,
    #[cfg(feature = "verification-cost-counts")]
    counts: Vec<CountSample>,
    #[cfg(feature = "verification-cost-counts")]
    high_water: counts::ScratchHighWater,
    #[cfg(feature = "verification-cost-counts")]
    initial_capacity: counts::ScratchHighWater,
    #[cfg(feature = "verification-cost-counts")]
    first_tick_capacity: Option<counts::ScratchHighWater>,
    #[cfg(feature = "verification-cost-counts")]
    first_tick_increases: counts::ScratchHighWater,
    #[cfg(feature = "verification-cost-counts")]
    later_increases: counts::ScratchHighWater,
    pub(crate) case_validation_started: bool,
    pub(crate) case_validation_passed: bool,
}

impl Recorder {
    fn new(plan: &ResolvedPlan, mode: VerificationCostMode) -> Result<Self> {
        let samples = usize::try_from(plan.window.end())
            .map_err(|_| invalid("verification-cost 窗口样本数超出可寻址范围"))?;
        Ok(Self {
            mode,
            warm_up_ticks: plan.window.warm_up_ticks,
            timing: if mode == VerificationCostMode::Timing {
                Vec::with_capacity(samples)
            } else {
                Vec::new()
            },
            #[cfg(feature = "verification-cost-counts")]
            counts: if mode == VerificationCostMode::Counts {
                Vec::with_capacity(samples)
            } else {
                Vec::new()
            },
            #[cfg(feature = "verification-cost-counts")]
            high_water: Default::default(),
            #[cfg(feature = "verification-cost-counts")]
            initial_capacity: Default::default(),
            #[cfg(feature = "verification-cost-counts")]
            first_tick_capacity: None,
            #[cfg(feature = "verification-cost-counts")]
            first_tick_increases: Default::default(),
            #[cfg(feature = "verification-cost-counts")]
            later_increases: Default::default(),
            case_validation_started: false,
            case_validation_passed: false,
        })
    }

    /// 目录由原 runner 独占创建后立即留不完整标记；后续初始化/写出失败不留空白成功印象。
    pub(crate) fn write_initial(
        &self,
        output: &Path,
        plan: &ResolvedPlan,
        execution: ExecutionConfig,
    ) -> Result<()> {
        crate::written::write_json_verified(
            &output.join(FILE),
            &json!({
                "format":"urban-verification-cost-1","harness":"laneflow-urban-harness",
                "purpose":"verification-cost","mode":self.mode.label(),"complete":false,
                "phase":"initializing","workers":execution.worker_count().get(),
                "window":{"purpose":plan.window.purpose,"warm_up_ticks":plan.window.warm_up_ticks,
                    "observation_ticks":plan.window.observation_ticks,"expected_ticks":plan.window.end()},
                "note":"initial marker; only a finalized complete report witnesses full sampling",
            }),
        )?;
        Ok(())
    }

    pub(crate) fn record(&mut self, harness: &Harness<'_>, tick: u64) {
        if self.mode == VerificationCostMode::Timing {
            self.timing.push(TimingSample {
                tick,
                observation_ns: harness.last_observation_ns,
            });
        }
        #[cfg(feature = "verification-cost-counts")]
        if self.mode == VerificationCostMode::Counts {
            self.counts.push(CountSample {
                tick,
                workload_counts: harness.observation.work,
                allocation: harness.observation.allocation,
            });
            let capacities = Self::capacities(harness);
            if self.first_tick_capacity.is_none() {
                self.first_tick_capacity = Some(capacities);
                self.high_water
                    .update_high_water(capacities, &mut self.first_tick_increases);
            } else {
                self.high_water
                    .update_high_water(capacities, &mut self.later_increases);
            }
        }
    }

    /// 安装完成即记录基线，首次成功拍的增长和后续拍的新高次数分别列出。
    pub(crate) fn installed(&mut self, harness: &Harness<'_>) {
        #[cfg(feature = "verification-cost-counts")]
        if self.mode == VerificationCostMode::Counts {
            self.initial_capacity = Self::capacities(harness);
            self.high_water = self.initial_capacity;
        }
        #[cfg(not(feature = "verification-cost-counts"))]
        let _ = harness;
    }

    #[cfg(feature = "verification-cost-counts")]
    fn capacities(harness: &Harness<'_>) -> counts::ScratchHighWater {
        let mut capacities = harness.observation.capacities();
        capacities.step_before_capacity = harness.step_before.capacity();
        capacities.scratch_red_capacity = harness.scratch_red.capacity();
        capacities
    }

    fn complete_samples(&self, expected: u64) -> bool {
        if self.mode == VerificationCostMode::Timing {
            return self.timing.len() as u64 == expected
                && self
                    .timing
                    .iter()
                    .enumerate()
                    .all(|(i, sample)| sample.tick == i as u64 + 1);
        }
        #[cfg(feature = "verification-cost-counts")]
        {
            self.counts.len() as u64 == expected
                && self
                    .counts
                    .iter()
                    .enumerate()
                    .all(|(i, sample)| sample.tick == i as u64 + 1)
        }
        #[cfg(not(feature = "verification-cost-counts"))]
        false
    }

    fn measurements(&self) -> Result<Value> {
        if self.mode == VerificationCostMode::Timing {
            let summary = |warm: Option<bool>| -> Result<Value> {
                let mut values: Vec<_> = self
                    .timing
                    .iter()
                    .filter(|row| warm.is_none_or(|warm| (row.tick <= self.warm_up_ticks) == warm))
                    .map(|row| row.observation_ns)
                    .collect();
                if values.is_empty() {
                    return Ok(Value::Null);
                }
                let total_ns: u128 = values.iter().map(|&value| u128::from(value)).sum();
                Ok(
                    json!({"total_ns":u64::try_from(total_ns).map_err(|_| invalid("观察时间累计值溢出"))?,
                    "summary":report::sample_summary(&mut values)?}),
                )
            };
            return Ok(json!({
                "observation_ns":{"scope":OBSERVATION_SCOPE,"all":summary(None)?,"warm_up":summary(Some(true))?,"observation":summary(Some(false))?},
                "workload_counts":null,"allocation":null,"scratch_high_water":null,
                "samples":self.timing,
            }));
        }
        #[cfg(feature = "verification-cost-counts")]
        {
            let mut all = counts::WorkloadCounts::default();
            let mut warm = counts::WorkloadCounts::default();
            let mut observation = counts::WorkloadCounts::default();
            let mut allocation = counts::ObservationAllocation::default();
            let mut warm_allocation = counts::ObservationAllocation::default();
            let mut observation_allocation = counts::ObservationAllocation::default();
            for row in &self.counts {
                all.add(row.workload_counts);
                allocation.add(row.allocation);
                if row.tick <= self.warm_up_ticks {
                    warm.add(row.workload_counts);
                    warm_allocation.add(row.allocation);
                } else {
                    observation.add(row.workload_counts);
                    observation_allocation.add(row.allocation);
                }
            }
            let block_ledger_bytes = size_of::<counts::WorkloadCounts>();
            Ok(json!({
                "observation_ns":null,
                "workload_counts":{"scope":"selected existing observer loops; successful ticks only; not all Runtime/API calls or CPU instructions","all":all,"warm_up":warm,"observation":observation},
                "allocation":{"scope":OBSERVATION_SCOPE,"thread_scope":"process-global System allocator; main and observer worker threads; sampling after block joins; unrelated threads in the same process contaminate counts", "all":allocation,"warm_up":warm_allocation,"observation":observation_allocation},
                "scratch_high_water":{"scope":"capacity metadata after installation and each successful tick; field-wise maxima; O(blocks), no extra vehicle scan; not RSS; excludes transient block outcomes and cost sample storage",
                    "initial":self.initial_capacity,"after_first_tick":self.first_tick_capacity,"values":self.high_water,
                    "instrumentation_storage":{
                        "ledger_payload_bytes_per_block":block_ledger_bytes,
                        "count_build_block_element_bytes":crate::observe::ObservationScratch::counted_block_bytes(),
                        "initial_reserved_ledger_payload_bytes":self.initial_capacity.blocks_capacity * block_ledger_bytes,
                        "maximum_reserved_ledger_payload_bytes":self.high_water.blocks_capacity * block_ledger_bytes,
                        "allocation_attribution":"one reusable ledger per BlockBuffers; no ledger payload in per-tick PreBlock/BlockOutcome results; blocks Vec creation/growth within observer regions includes instrumented element bytes and is not subtracted from allocation counts"
                    },
                    "sampled_high_water_increases":{"meaning":"number of sampled new maxima per field, not allocator reallocations","first_tick":self.first_tick_increases,"later_ticks":self.later_increases}},
                "samples":self.counts,
            }))
        }
        #[cfg(not(feature = "verification-cost-counts"))]
        Err(invalid("当前构建不支持 counts 模式"))
    }
}

/// 对完整 correctness 或 probe 计划运行验证成本计量，原结果和规范日志照常保留。
/// `complete` 仅说明输入计划、采样和原校验完整完成；不产生产品性能认证结论。
/// counts 使用进程级 allocator，调用方须让此运行独占进程中的受测工作；并行测试
/// 应启动独立子进程，不能把库测试线程上的分配当作本运行成本。
///
/// # Errors
///
/// 模式与构建不符、输入不是完整 correctness/probe 计划、worker 数不合法、
/// 输出目录已存在、环境采集或原运行初始化/文件写出失败时返回错误。受控运行或
/// 角色见证校验失败时保留原失败报告，并返回 `complete=false` 的成本报告。
pub fn run_verification_cost(
    artifacts: &Artifacts,
    plan: &ResolvedPlan,
    output: &Path,
    execution: ExecutionConfig,
    mode: VerificationCostMode,
) -> Result<Value> {
    mode.validate_build()?;
    if !matches!(plan.window.purpose.as_str(), "correctness" | "probe") {
        return Err(invalid(
            "verification-cost 需要完整的 correctness 或 probe 计划",
        ));
    }
    plan.validate(artifacts)?;
    if !(1..=crate::MAX_WORKERS).contains(&execution.worker_count().get()) {
        return Err(invalid("verification-cost 的 workers 必须在 1..=16 范围内"));
    }
    if output.exists() {
        return Err(invalid("verification-cost 输出目录已存在"));
    }
    let mut recorder = Recorder::new(plan, mode)?;
    // 复用现有 host-observe 输入和记录；入口不制造“已批准 quiet”标记。
    let manual = std::env::var_os(crate::environment::INPUT_VARIABLE)
        .map(|_| crate::environment::manual())
        .transpose()?;
    let before = manual
        .as_ref()
        .map(|manual| {
            crate::environment::collect(laneflow_host_observe::environment::Phase::Before, manual)
        })
        .transpose()?;
    let started = Instant::now();
    let result = report::run_with_options_and_cost(
        artifacts,
        plan,
        output,
        execution,
        Diagnostics::Enabled,
        &RunOptions::default(),
        Some(&mut recorder),
    )?;
    let elapsed = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
    let environment = if let (Some(manual), Some((before, _))) = (manual, before) {
        let (after, _) =
            crate::environment::collect(laneflow_host_observe::environment::Phase::After, &manual)?;
        let before_digest =
            crate::written::write_json_verified(&output.join(crate::environment::BEFORE), &before)?;
        let after_digest =
            crate::written::write_json_verified(&output.join(crate::environment::AFTER), &after)?;
        json!({"before":{"file":crate::environment::BEFORE,"digest":before_digest},
            "after":{"file":crate::environment::AFTER,"digest":after_digest}})
    } else {
        Value::Null
    };
    let expected_status = if plan.window.purpose == "correctness" {
        "case-pass-replay-required"
    } else {
        "probe-complete"
    };
    let complete = result.error.is_none()
        && result.status == expected_status
        && result.completed_ticks == plan.window.end()
        && result
            .endpoint
            .as_ref()
            .is_some_and(|endpoint| endpoint.tick == plan.window.end())
        && recorder.complete_samples(plan.window.end())
        && (plan.window.purpose != "correctness" || recorder.case_validation_passed);
    let diagnostics: Value = serde_json::from_slice(&fs::read(output.join("diagnostics.json"))?)?;
    let mut cost = recorder.measurements()?;
    let object = cost.as_object_mut().expect("measurements is an object");
    let identity = json!({
        "format":"urban-verification-cost-1", "harness":"laneflow-urban-harness",
        "purpose":"verification-cost", "mode":mode.label(), "complete":complete,
        "phase":"finished",
        "input_identity":{"plan_digest":result.plan_digest,"manifest_digest":artifacts.manifest_digest,
            "network_revision":result.network_revision,"policy_id":result.policy_id,"world_id":result.world_id,
            "case":plan.case,"scale":plan.scale,"fixed_step_ms":plan.dt,
            "observation_version":crate::observe::OBSERVATION_VERSION,
            "verification_scope":"existing Urban checks and case witnesses; shared body predicate; no full shared Session migration"},
        "input_manifest":artifacts.files,
        "window":{"purpose":plan.window.purpose,"warm_up_ticks":plan.window.warm_up_ticks,
            "observation_ticks":plan.window.observation_ticks,"expected_ticks":plan.window.end(),"completed_ticks":result.completed_ticks},
        "workers":execution.worker_count().get(),
        "semantic_run":{"result_file":"result.json","result_digest":report::digest_file(&output.join("result.json"))?,
            "status":result.status,"error":result.error,"endpoint":result.endpoint,
            "validate_case":{"requested":plan.window.purpose == "correctness","executed":recorder.case_validation_started,"passed":recorder.case_validation_passed},
            "comparison":"use the original compare command against an independent full run; this cost receipt does not replace replay"},
        "whole_run_wall_ns":if mode == VerificationCostMode::Timing { Some(elapsed) } else { None },
        "whole_run_wall_scope":WALL_SCOPE,
        "source":{"git_commit":diagnostics["git_commit_at_run"],"git_status":diagnostics["git_status_at_run"],
            "binary":diagnostics["binary"],"invocation":diagnostics["invocation"],
            "build_source_commit":option_env!("LANEFLOW_MEASUREMENT_COMMIT"),"debug_assertions":cfg!(debug_assertions),
            "features":{"verification_cost_counts":cfg!(feature="verification-cost-counts"),"allocation":cfg!(feature="allocation"),
                "adapter":cfg!(feature="adapter"),"execution_evidence":cfg!(feature="execution-evidence"),
                "motion_kernel_evidence":cfg!(feature="motion-kernel-evidence"),"pose_profiling":cfg!(feature="pose-profiling")}},
        "environment":environment,
        "existing_timing_artifacts_usable":mode == VerificationCostMode::Timing,
    });
    object.extend(identity.as_object().expect("identity is an object").clone());
    crate::written::write_json_verified(&output.join(FILE), &cost)?;
    Ok(cost)
}
