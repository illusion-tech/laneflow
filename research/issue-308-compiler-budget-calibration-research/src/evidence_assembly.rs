//! 正式执行检查点到编译器校准证据 v1 的 Rust 原生投影。
//!
//! 本模块只消费已经固化来源、环境和原始子进程观察的检查点；不能从当前进程补造测量值。

use crate::evidence::{EvidenceError, VerificationContext};
use crate::{
    ATTRIBUTION_BINARY_ID, EVIDENCE_SCHEMA_ID, EVIDENCE_SCHEMA_VERSION,
    FORMAL_PROTOCOL_CHECKPOINT_SCHEMA, FORMAL_PROTOCOL_CHECKPOINT_SCHEMA_VERSION,
    FormalCurrentFixtureProjection, FormalEnvironmentSnapshot, FormalProtocolCheckpoint,
    ORACLE_BINARY_ID, TIMING_BINARY_ID, TrustedContract,
};
use serde_json::{Map, Value, json};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn assemble_evidence_document(
    trusted: &TrustedContract,
    context: &VerificationContext,
    checkpoint: &FormalProtocolCheckpoint,
) -> Result<Value, EvidenceError> {
    validate_checkpoint_source(context, checkpoint)?;
    let candidate_matrix = checkpoint.candidate_matrix.as_ref().ok_or_else(|| {
        EvidenceError::CandidateRegistryRecomputation {
            detail: "正式检查点缺少候选矩阵".to_owned(),
        }
    })?;
    let candidate_bindings = build_candidate_bindings(
        trusted,
        context,
        &checkpoint.environment,
        &candidate_matrix.safety_audit,
    )?;
    let baseline_candidate = candidate_bindings
        .iter()
        .find(|binding| {
            binding["id"] == "baseline-std-randomstate-stable-vec-v1"
                && binding["keyDomain"] == "full-pipeline-baseline"
        })
        .ok_or_else(|| EvidenceError::CandidateRegistryRecomputation {
            detail: "候选注册表缺少完整管线基线绑定".to_owned(),
        })?
        .clone();
    let mut runs = current_fixture_runs(
        trusted,
        &checkpoint.environment,
        &checkpoint.current_fixtures,
        &baseline_candidate,
    )?;
    runs.extend(crate::evidence_assembly::pilot_runs(
        trusted,
        &checkpoint.environment,
        &checkpoint.base_scale_pilot,
        &baseline_candidate,
    )?);
    runs.extend(formal_ladder_runs(
        trusted,
        &checkpoint.environment,
        checkpoint,
        &baseline_candidate,
    )?);
    runs.extend(limit_qualification_runs(
        trusted,
        &checkpoint.environment,
        checkpoint,
        &baseline_candidate,
    )?);
    let base_scales = base_scale_summaries(checkpoint);
    let mut formal_derived = formal_derived_summaries(checkpoint)?;
    let candidate_derived = candidate_evidence(
        trusted,
        &checkpoint.environment,
        candidate_matrix,
        &candidate_bindings,
        &baseline_candidate,
        &formal_derived.reproducibility_envelopes,
    )?;
    runs.extend(candidate_derived.runs);
    formal_derived
        .round_summaries
        .extend(candidate_derived.round_summaries);
    let formal_study_disposition = if base_scales
        .iter()
        .any(|scale| scale["b"]["value"].is_null())
    {
        "no-reliable-base-scale"
    } else if checkpoint.formal_ladders.len() != 9
        || checkpoint.formal_ladders.iter().any(|ladder| {
            ladder.analysis.is_none()
                || ladder.levels.iter().filter(|level| level.complete).count()
                    < crate::FORMAL_LADDER_MINIMUM_LEVEL_COUNT
        })
    {
        "insufficient-formal-ladder"
    } else {
        "formal-analysis-available"
    };
    Ok(json!({
        "schema": EVIDENCE_SCHEMA_ID,
        "schemaVersion": EVIDENCE_SCHEMA_VERSION,
        "source": source_json(trusted, checkpoint),
        "environment": environment_json(&checkpoint.environment),
        "protocol": {
            "id": checkpoint.protocol_id,
            "workloadSeedHexU64": "4c46434f4d500001",
            "clockQuantumNs": checkpoint.base_scale_pilot.clock_quantum_ns,
            "batchCount": 2,
            "candidateOrderDesign": "forward-reverse-cyclic-2c-v1",
            "guardThresholds": crate::GuardThresholds::from_physical_memory_bytes(
                checkpoint.environment.physical_memory_bytes
            ).map_err(|error| EvidenceError::GuardRecomputation { detail: error.to_string() })?
        },
        "binaries": binary_bindings(checkpoint),
        "candidateBindings": candidate_bindings,
        "runs": runs,
        "derived": {
            "formalStudyDisposition": formal_study_disposition,
            "baseScales": base_scales,
            "constantHashQualifications": candidate_derived.constant_hash_qualifications,
            "roundMetricSummaries": formal_derived.round_summaries,
            "ladderBatchSummaries": formal_derived.batch_summaries,
            "adjacentLevelRatios": formal_derived.adjacent_ratios,
            "knees": formal_derived.knees,
            "reproducibilityEnvelopes": formal_derived.reproducibility_envelopes,
            "growthSlopes": formal_derived.growth_slopes,
            "candidateRosters": candidate_derived.rosters,
            "candidateComparisons": candidate_derived.comparisons,
            "recommendations": formal_derived.recommendations
        },
        "artifacts": []
    }))
}

fn validate_checkpoint_source(
    context: &VerificationContext,
    checkpoint: &FormalProtocolCheckpoint,
) -> Result<(), EvidenceError> {
    for (pointer, expected, actual) in [
        (
            "/schema",
            FORMAL_PROTOCOL_CHECKPOINT_SCHEMA.to_owned(),
            checkpoint.schema.clone(),
        ),
        (
            "/schemaVersion",
            FORMAL_PROTOCOL_CHECKPOINT_SCHEMA_VERSION.to_string(),
            checkpoint.schema_version.to_string(),
        ),
        (
            "/source/sourceCommit",
            context.repository_head.clone(),
            checkpoint.source.source_commit.clone(),
        ),
        (
            "/source/harnessCommit",
            context.repository_head.clone(),
            checkpoint.source.harness_commit.clone(),
        ),
        (
            "/source/cargoLockSha256",
            context.cargo_lock_sha256.clone(),
            checkpoint.source.cargo_lock_sha256.clone(),
        ),
    ] {
        if expected != actual {
            return Err(EvidenceError::BindingMismatch {
                pointer: pointer.to_owned(),
                expected,
                actual,
            });
        }
    }
    if checkpoint.source.dirty {
        return Err(EvidenceError::BindingMismatch {
            pointer: "/source/dirty".to_owned(),
            expected: "false".to_owned(),
            actual: "true".to_owned(),
        });
    }
    let actual_binaries = checkpoint
        .source
        .binaries
        .iter()
        .map(|binary| (binary.binary_id.as_str(), binary.sha256.as_str()))
        .collect::<BTreeMap<_, _>>();
    if actual_binaries.len() != 3 {
        return Err(EvidenceError::BinaryBindingRecomputation {
            detail: "正式检查点必须精确绑定三个研究子进程二进制".to_owned(),
        });
    }
    for binary_id in [TIMING_BINARY_ID, ATTRIBUTION_BINARY_ID, ORACLE_BINARY_ID] {
        let expected = context.binary_sha256.get(binary_id).ok_or_else(|| {
            EvidenceError::MissingResearchBinary {
                binary_id: binary_id.to_owned(),
            }
        })?;
        let actual = actual_binaries.get(binary_id).copied().ok_or_else(|| {
            EvidenceError::BinaryBindingRecomputation {
                detail: format!("正式检查点缺少二进制 {binary_id}"),
            }
        })?;
        if actual != expected {
            return Err(EvidenceError::BindingMismatch {
                pointer: format!("/source/binaries/{binary_id}/sha256"),
                expected: expected.clone(),
                actual: actual.to_owned(),
            });
        }
    }
    Ok(())
}

fn source_json(trusted: &TrustedContract, checkpoint: &FormalProtocolCheckpoint) -> Value {
    json!({
        "sourceCommit": checkpoint.source.source_commit,
        "harnessCommit": checkpoint.source.harness_commit,
        "dirty": checkpoint.source.dirty,
        "cargoLockSha256": checkpoint.source.cargo_lock_sha256,
        "contractDescriptorId": trusted.descriptor.schema,
        "contractDescriptorVersion": trusted.descriptor.schema_version,
        "contractDescriptorSha256": trusted.descriptor_sha256,
        "workloadManifestSha256": trusted.descriptor.workload_manifest.sha256,
        "evidenceSchemaSha256": trusted.descriptor.evidence_schema.sha256
    })
}

fn environment_json(environment: &FormalEnvironmentSnapshot) -> Value {
    json!({
        "os": environment.os,
        "osBuild": environment.os_build,
        "cpu": environment.cpu,
        "logicalProcessorCount": environment.logical_processor_count,
        "physicalMemoryBytes": environment.physical_memory_bytes,
        "targetTriple": environment.target_triple,
        "rustc": environment.rustc,
        "llvm": environment.llvm,
        "powerSource": environment.power_source,
        "vendorPerformanceMode": environment.vendor_performance_mode,
        "powerPlan": environment.power_plan,
        "biosFirmware": environment.bios_firmware,
        "monitoringProvider": environment.monitoring_provider,
        "backgroundProcessAudit": environment.background_process_audit
    })
}

fn binary_bindings(checkpoint: &FormalProtocolCheckpoint) -> Vec<Value> {
    [
        (TIMING_BINARY_ID, "timing"),
        (ATTRIBUTION_BINARY_ID, "attribution"),
        (ORACLE_BINARY_ID, "oracle"),
    ]
    .into_iter()
    .map(|(binary_id, mode)| {
        let digest = checkpoint
            .source
            .binaries
            .iter()
            .find(|binary| binary.binary_id == binary_id)
            .expect("checkpoint source validation requires every binary");
        json!({
            "id": binary_id,
            "mode": mode,
            "sha256": digest.sha256,
            "cargoProfile": "release",
            "features": ["research-runner-full"]
        })
    })
    .collect()
}

fn current_fixture_runs(
    trusted: &TrustedContract,
    environment: &FormalEnvironmentSnapshot,
    projection: &FormalCurrentFixtureProjection,
    baseline_candidate: &Value,
) -> Result<Vec<Value>, EvidenceError> {
    let child = projection
        .child
        .as_ref()
        .ok_or_else(|| EvidenceError::FixtureRecomputation {
            detail: "当前固定样例 oracle 子进程未产生结构化报告".to_owned(),
        })?;
    let fixture_workload = trusted.workload_manifest["workloads"]
        .as_array()
        .and_then(|workloads| {
            workloads
                .iter()
                .find(|workload| workload["id"] == "LF-COMP-RESEARCH-CURRENT-FIXTURES-v1")
        })
        .ok_or_else(|| EvidenceError::FixtureRecomputation {
            detail: "工作负载清单缺少当前固定样例投影".to_owned(),
        })?;
    let cases = fixture_workload["cases"].as_array().ok_or_else(|| {
        EvidenceError::FixtureRecomputation {
            detail: "当前固定样例清单缺少 cases".to_owned(),
        }
    })?;
    let external_state =
        external_state_json(environment, projection.external_state.as_ref(), false);
    child
        .cases
        .iter()
        .enumerate()
        .map(|(position, summary)| {
            let case = cases
                .iter()
                .find(|case| case["id"] == summary.case_id)
                .ok_or_else(|| EvidenceError::FixtureRecomputation {
                    detail: format!("清单缺少当前固定样例 {}", summary.case_id),
                })?;
            let mut counts = serde_json::to_value(&summary.counts)
                .expect("fixture counts serialize")
                .as_object()
                .expect("fixture counts object")
                .clone();
            counts.extend(
                summary
                    .entity_counts
                    .iter()
                    .map(|(name, value)| (name.clone(), json!(value))),
            );
            counts.extend(
                summary
                    .relation_record_counts
                    .iter()
                    .map(|(name, value)| (name.clone(), json!(value))),
            );
            let run_id = format!("fixture/{}", summary.case_id);
            Ok(json!({
                "runId": run_id,
                "batch": 0,
                "round": 0,
                "position": position,
                "roundAttempt": {"id": format!("attempt/{run_id}"), "ordinal": 0, "scope": "single-experiment"},
                "compilerInstanceId": null_observation("not-applicable-oracle"),
                "sampleOrdinal": 0,
                "sampleKind": "oracle",
                "status": projection.status,
                "invalidationReasons": projection.invalidation_reasons,
                "workload": {
                    "id": "LF-COMP-RESEARCH-CURRENT-FIXTURES-v1",
                    "revision": 1,
                    "graphProfile": "not-applicable",
                    "stringProfile": "not-applicable",
                    "generatorVersion": 1,
                    "n": 1,
                    "b": null_observation("not-applicable-current-fixture"),
                    "scaleRole": "current-fixture",
                    "caseId": summary.case_id,
                    "manifestDigest": trusted.descriptor.workload_manifest.sha256,
                    "counts": Value::Object(counts),
                    "fixtureInputs": case["files"]
                },
                "candidate": baseline_candidate,
                "process": projection.process,
                "metrics": metrics_json(
                    None,
                    None,
                    Some(&summary.semantic_digest_sha256),
                    Some(crate::diagnostic::empty_diagnostic_digest()),
                    &summary.stages,
                    "not-measured-by-fixture-oracle"
                ),
                "guard": not_applicable_guard(environment, &projection.monitor),
                "cleanup": no_cleanup(),
                "externalState": external_state
            }))
        })
        .collect()
}

fn metrics_json(
    wall_time_ns: Option<u64>,
    private_bytes: Option<u64>,
    semantic_digest: Option<&str>,
    diagnostic_digest: Option<String>,
    stages: &crate::StageBreakdown,
    reason: &str,
) -> Value {
    json!({
        "wallTimeNs": nullable_u64(wall_time_ns, reason),
        "allocationCount": null_observation(reason),
        "reallocationCount": null_observation(reason),
        "allocatedBytes": null_observation(reason),
        "freedBytes": null_observation(reason),
        "liveRequestedBytes": null_observation(reason),
        "peakLiveRequestedBytes": null_observation(reason),
        "retainedCapacityBytes": null_observation(reason),
        "workingSetBytes": null_observation(reason),
        "privateBytes": nullable_u64(private_bytes, reason),
        "commitPeakBytes": null_observation(reason),
        "semanticDigest": nullable_string(semantic_digest, reason),
        "diagnosticDigest": nullable_owned_string(diagnostic_digest, reason),
        "stageBreakdown": stage_breakdown_json(stages, reason)
    })
}

fn stage_breakdown_json(stages: &crate::StageBreakdown, reason: &str) -> Value {
    let serialized = serde_json::to_value(stages).expect("stage breakdown serialize");
    let mut output = Map::new();
    for key in [
        "sourceInput",
        "typedAst",
        "hir",
        "mir",
        "canonicalLir",
        "diagnostics",
        "scratch",
        "outputConstruction",
    ] {
        output.insert(
            key.to_owned(),
            json!({
                "recordCount": serialized[key]["recordCount"],
                "logicalBytes": serialized[key]["logicalBytes"],
                "attributionTimeNs": null_observation(reason),
                "liveRequestedBytes": null_observation(reason),
                "peakLiveRequestedBytes": null_observation(reason)
            }),
        );
    }
    Value::Object(output)
}

fn not_applicable_guard(
    _environment: &FormalEnvironmentSnapshot,
    monitor: &crate::ChildProcessMonitorReport,
) -> Value {
    json!({
        "compilerControlledPredictionBasis": "not-applicable",
        "privateBytesPredictionBasis": "not-applicable",
        "wallTimePredictionBasis": "not-applicable",
        "previousCompletedN": null_observation("not-applicable"),
        "previousPrimaryRecordCount": null_observation("not-applicable"),
        "nextPrimaryRecordCount": 1,
        "previousPeakLiveRequestedBytes": null_observation("not-applicable"),
        "predictedCompilerControlledBytes": null_observation("not-applicable"),
        "previousPrivateBytes": null_observation("not-applicable"),
        "predictedPrivateBytes": null_observation("not-applicable"),
        "previousWallTimeNs": null_observation("not-applicable"),
        "predictedWallTimeNs": null_observation("not-applicable"),
        "logicalBytesLowerBound": 0,
        "reservedBytesBeforeFailure": 0,
        "trigger": monitor_trigger(monitor.trigger),
        "lastAvailablePhysicalMemoryBytes": monitor
            .last_available_physical_memory_bytes
            .value
            .unwrap_or(0),
        "lastPrivateBytes": monitor.last_private_bytes
    })
}

fn monitor_trigger(trigger: Option<crate::ChildMonitorTrigger>) -> &'static str {
    match trigger {
        None => "none",
        Some(crate::ChildMonitorTrigger::PrivateBytes) => "private-bytes-monitor",
        Some(crate::ChildMonitorTrigger::WallTime) => "wall-time-monitor",
        Some(crate::ChildMonitorTrigger::AvailablePhysicalMemory) => "available-physical-memory",
        Some(crate::ChildMonitorTrigger::MonitoringGap) => "monitoring-gap",
    }
}

fn external_state_json(
    environment: &FormalEnvironmentSnapshot,
    observation: Option<&crate::ExternalStateObservation>,
    monitoring_gap_when_missing: bool,
) -> Value {
    observation.map_or_else(
        || {
            json!({
                "powerSource": environment.power_source,
                "vendorPerformanceMode": environment.vendor_performance_mode,
                "powerPlan": environment.power_plan,
                "sleepOrSessionLock": environment.operator_declaration.sleep_or_session_lock_observed,
                "thermalOrPowerThrottling": environment.operator_declaration.thermal_or_power_throttling_observed,
                "backgroundCpuTimeNs": if monitoring_gap_when_missing { null_observation("monitoring-gap") } else { observed(0) },
                "backgroundWriteBytes": if monitoring_gap_when_missing { null_observation("monitoring-gap") } else { observed(0) },
                "monitoringGap": monitoring_gap_when_missing,
                "backgroundProcessDeltas": []
            })
        },
        |observation| serde_json::to_value(observation).expect("external state serialize"),
    )
}

fn no_cleanup() -> Value {
    json!({
        "experimentId": null_observation("not-applicable"),
        "phase": "not-applicable",
        "sequenceIndex": null_observation("not-applicable")
    })
}

fn limit_qualification_runs(
    trusted: &TrustedContract,
    environment: &FormalEnvironmentSnapshot,
    checkpoint: &FormalProtocolCheckpoint,
    baseline_candidate: &Value,
) -> Result<Vec<Value>, EvidenceError> {
    let Some(bundle) = checkpoint.limit_qualification.as_ref() else {
        return Ok(Vec::new());
    };
    let plans =
        crate::ScalableStagePlanFactory::from_trusted_contract(trusted).map_err(|error| {
            EvidenceError::WorkloadRecomputation {
                detail: error.to_string(),
            }
        })?;
    let mut output = Vec::new();

    for run in &bundle.live_byte_baseline_runs {
        let plan = plans
            .plan(run.scale.workload_id, run.scale.graph_profile, run.scale.n)
            .map_err(|error| EvidenceError::WorkloadRecomputation {
                detail: error.to_string(),
            })?;
        output.push(limit_baseline_run(
            trusted,
            environment,
            run,
            &plan,
            baseline_candidate,
        )?);
    }
    for execution in &bundle.limit_pairs {
        let plan = plans
            .plan(
                execution.scale.workload_id,
                execution.scale.graph_profile,
                execution.scale.n,
            )
            .map_err(|error| EvidenceError::WorkloadRecomputation {
                detail: error.to_string(),
            })?;
        output.push(limit_pair_run(
            trusted,
            environment,
            execution,
            crate::LimitPairSide::AtBound,
            &plan,
            baseline_candidate,
        )?);
        output.push(limit_pair_run(
            trusted,
            environment,
            execution,
            crate::LimitPairSide::PlusOne,
            &plan,
            baseline_candidate,
        )?);
    }
    for qualification in &bundle.duplicate_owner_qualifications {
        let plan = plans
            .plan(
                qualification.scale.workload_id,
                qualification.scale.graph_profile,
                qualification.scale.n,
            )
            .map_err(|error| EvidenceError::WorkloadRecomputation {
                detail: error.to_string(),
            })?;
        output.push(duplicate_owner_run(
            trusted,
            environment,
            qualification,
            &plan,
            baseline_candidate,
        )?);
    }
    for qualification in &bundle.cleanup_experiments {
        let plan = plans
            .plan(
                qualification.scale.workload_id,
                qualification.scale.graph_profile,
                qualification.scale.n,
            )
            .map_err(|error| EvidenceError::WorkloadRecomputation {
                detail: error.to_string(),
            })?;
        append_cleanup_runs(
            &mut output,
            trusted,
            environment,
            qualification,
            &plan,
            baseline_candidate,
        )?;
    }
    Ok(output)
}

fn limit_baseline_run(
    trusted: &TrustedContract,
    environment: &FormalEnvironmentSnapshot,
    run: &crate::LimitBaselineProcessRun,
    plan: &crate::ScalableStagePlanSummary,
    baseline_candidate: &Value,
) -> Result<Value, EvidenceError> {
    let child = run.child.as_ref();
    Ok(json!({
        "runId": run.run_id,
        "batch": 0,
        "round": 0,
        "position": run.replica_index,
        "roundAttempt": {"id": run.run_id, "ordinal": 0, "scope": "single-experiment"},
        "compilerInstanceId": child.map_or_else(
            || null_observation("child-report-missing"),
            |child| observed_string_value(&child.compiler_instance_id),
        ),
        "sampleOrdinal": run.replica_index,
        "sampleKind": "limit-baseline",
        "status": run.status,
        "invalidationReasons": run.invalidation_reasons,
        "workload": qualification_workload_json(trusted, &run.scale, plan)?,
        "candidate": baseline_candidate,
        "process": run.process,
        "metrics": attribution_metrics(
            child.and_then(|child| child.allocation.as_ref()),
            child.and_then(|child| child.retained_capacity_bytes.as_ref()),
            monitor_peak_private_bytes(Some(&run.monitor)),
            child.and_then(|child| child.semantic_digest_sha256.as_deref()),
            &plan.stages,
            "limit-baseline-child-incomplete",
        ),
        "guard": not_applicable_guard(environment, &run.monitor),
        "cleanup": no_cleanup(),
        "externalState": external_state_json(environment, run.external_state.as_ref(), true),
        "limitBaseline": {
            "measurementId": "compiler-controlled-live-byte-baseline-v1",
            "dimensionId": "compiler-controlled-live-byte-count",
            "replicaIndex": run.replica_index,
            "privateLimitMode": "operational-hard-ceiling-only"
        }
    }))
}

fn limit_pair_run(
    trusted: &TrustedContract,
    environment: &FormalEnvironmentSnapshot,
    execution: &crate::LimitPairExecution,
    side: crate::LimitPairSide,
    plan: &crate::ScalableStagePlanSummary,
    baseline_candidate: &Value,
) -> Result<Value, EvidenceError> {
    let monitored = match side {
        crate::LimitPairSide::AtBound => &execution.at_bound,
        crate::LimitPairSide::PlusOne => &execution.plus_one,
    };
    let Some(child) = monitored.child.as_ref() else {
        return incomplete_qualification_run(
            trusted,
            environment,
            &format!(
                "{}/incomplete",
                limit_evidence_run_id(&execution.scale, &execution.pair, side)
            ),
            &execution.scale,
            plan,
            monitored,
            baseline_candidate,
        );
    };
    let observation = &child.observation;
    let mut metrics = metrics_json(
        None,
        monitor_peak_private_bytes(Some(&monitored.monitor)),
        observation.semantic_digest_sha256.as_deref(),
        Some(observation.diagnostic_digest_sha256.clone()),
        &plan.stages,
        "limit-qualification-not-measured",
    );
    metrics["liveRequestedBytes"] = observed(observation.live_requested_bytes_after_run);
    metrics["peakLiveRequestedBytes"] = nullable_u64(
        observation.peak_live_requested_bytes,
        "timing-limit-qualification",
    );
    let selected = match side {
        crate::LimitPairSide::AtBound => execution.pair.at_bound_limit_value,
        crate::LimitPairSide::PlusOne => execution.pair.plus_one_limit_value,
    };
    let value_basis = match execution.pair.binding.pair_mode {
        crate::LimitPairMode::SuccessAtBound => "canonical-level-exact-value",
        crate::LimitPairMode::DiagnosticCapOnSemanticFailure => "diagnostic-input-count",
        crate::LimitPairMode::BaselineLiveBytePrescanV1 => "baseline-live-byte-prescan-v1",
    };
    Ok(json!({
        "runId": observation.run_id,
        "batch": 0,
        "round": 0,
        "position": if side == crate::LimitPairSide::AtBound { 0 } else { 1 },
        "roundAttempt": {"id": observation.run_id, "ordinal": 0, "scope": "single-experiment"},
        "compilerInstanceId": observed_string_value(&child.compiler_instance_id),
        "sampleOrdinal": 0,
        "sampleKind": "failure",
        "status": monitored.status,
        "invalidationReasons": monitored.invalidation_reasons,
        "workload": qualification_workload_json(trusted, &execution.scale, plan)?,
        "candidate": baseline_candidate,
        "process": monitored.process,
        "metrics": metrics,
        "guard": not_applicable_guard(environment, &monitored.monitor),
        "cleanup": no_cleanup(),
        "externalState": external_state_json(environment, monitored.external_state.as_ref(), true),
        "failure": {
            "caseId": format!("limit/{}/{}", execution.pair.binding.dimension_id.as_str(), side.as_str()),
            "dimensionId": execution.pair.binding.dimension_id,
            "inputVariantId": execution.pair.binding.input_variant_id,
            "inputDigest": observation.input_digest_sha256,
            "expectedOutcome": observation.expected_outcome,
            "actualOutcome": observation.actual_outcome,
            "stableCompilerErrorCode": nullable_string(
                observation.stable_compiler_error_code.as_deref(),
                "not-applicable-success",
            ),
            "diagnosticCount": observation.diagnostic_count,
            "diagnosticsTruncated": observation.diagnostics_truncated,
            "partialOutputRecordCount": observation.partial_output_record_count,
            "limitSelection": {
                "exactDimensionValue": execution.pair.exact_dimension_value,
                "selectedLimitValue": selected,
                "valueBasis": value_basis,
                "basisRunIds": execution.pair.basis_run_ids
            }
        }
    }))
}

fn duplicate_owner_run(
    trusted: &TrustedContract,
    environment: &FormalEnvironmentSnapshot,
    qualification: &crate::DuplicateOwnerQualification,
    plan: &crate::ScalableStagePlanSummary,
    baseline_candidate: &Value,
) -> Result<Value, EvidenceError> {
    let monitored = &qualification.run;
    let Some(child) = monitored.child.as_ref() else {
        return incomplete_qualification_run(
            trusted,
            environment,
            &format!(
                "failure/semantic-duplicate-owner/{}/{}/n-{}/incomplete",
                qualification.scale.graph_profile.as_str(),
                qualification.scale.scale_role.as_str(),
                qualification.scale.n,
            ),
            &qualification.scale,
            plan,
            monitored,
            baseline_candidate,
        );
    };
    let observation = &child.observation;
    let mut metrics = metrics_json(
        None,
        monitor_peak_private_bytes(Some(&monitored.monitor)),
        observation.semantic_digest_sha256.as_deref(),
        Some(observation.diagnostic_digest_sha256.clone()),
        &plan.stages,
        "semantic-failure-not-measured",
    );
    metrics["liveRequestedBytes"] = observed(observation.live_requested_bytes_after_run);
    Ok(json!({
        "runId": observation.run_id,
        "batch": 0,
        "round": 0,
        "position": 0,
        "roundAttempt": {"id": observation.run_id, "ordinal": 0, "scope": "single-experiment"},
        "compilerInstanceId": observed_string_value(&child.compiler_instance_id),
        "sampleOrdinal": 0,
        "sampleKind": "failure",
        "status": monitored.status,
        "invalidationReasons": monitored.invalidation_reasons,
        "workload": qualification_workload_json(trusted, &qualification.scale, plan)?,
        "candidate": baseline_candidate,
        "process": monitored.process,
        "metrics": metrics,
        "guard": not_applicable_guard(environment, &monitored.monitor),
        "cleanup": no_cleanup(),
        "externalState": external_state_json(environment, monitored.external_state.as_ref(), true),
        "failure": failure_observation_json(
            &qualification.case_id,
            "not-applicable",
            &qualification.input_variant_id,
            &observation.input_digest_sha256,
            observation.expected_outcome,
            observation.actual_outcome,
            observation.stable_compiler_error_code.as_deref(),
            observation.diagnostic_count,
            observation.diagnostics_truncated,
            observation.partial_output_record_count,
        )
    }))
}

fn append_cleanup_runs(
    output: &mut Vec<Value>,
    trusted: &TrustedContract,
    environment: &FormalEnvironmentSnapshot,
    qualification: &crate::CleanupQualification,
    plan: &crate::ScalableStagePlanSummary,
    baseline_candidate: &Value,
) -> Result<(), EvidenceError> {
    let monitored = &qualification.run;
    let Some(child) = monitored.child.as_ref() else {
        output.push(incomplete_qualification_run(
            trusted,
            environment,
            &format!(
                "cleanup/{}/{}/n-{}/incomplete-process",
                qualification.case_id.as_str(),
                qualification.scale.scale_role.as_str(),
                qualification.scale.n,
            ),
            &qualification.scale,
            plan,
            monitored,
            baseline_candidate,
        )?);
        return Ok(());
    };
    for observation in &child.experiment.runs {
        let sample_kind = match observation.phase {
            crate::CleanupPhase::BaselineSuccess | crate::CleanupPhase::FreshInstanceOracle => {
                "cold-instance"
            }
            crate::CleanupPhase::FailureIteration => "failure",
            crate::CleanupPhase::RecoverySuccess => "stable-capacity-reuse",
        };
        let phase = match observation.phase {
            crate::CleanupPhase::BaselineSuccess => "baseline-success",
            crate::CleanupPhase::FailureIteration => "failure-iteration",
            crate::CleanupPhase::RecoverySuccess => "post-recovery-success",
            crate::CleanupPhase::FreshInstanceOracle => "fresh-instance-oracle",
        };
        let mut metrics = metrics_json(
            None,
            monitor_peak_private_bytes(Some(&monitored.monitor)),
            observation.semantic_digest_sha256.as_deref(),
            Some(observation.diagnostic_digest_sha256.clone()),
            &observation.stage_plan.stages,
            "cleanup-attribution-not-collected",
        );
        metrics["liveRequestedBytes"] = observed(observation.live_requested_bytes);
        metrics["retainedCapacityBytes"] = observed(observation.retained_capacity_bytes);
        let mut run = json!({
            "runId": format!("{}/sequence-{}", observation.experiment_id, observation.sequence_index),
            "batch": 0,
            "round": 0,
            "position": observation.sequence_index,
            "roundAttempt": {"id": observation.experiment_id, "ordinal": 0, "scope": "single-experiment"},
            "compilerInstanceId": observed_string_value(&observation.compiler_instance_id),
            "sampleOrdinal": observation.sequence_index,
            "sampleKind": sample_kind,
            "status": monitored.status,
            "invalidationReasons": monitored.invalidation_reasons,
            "workload": qualification_workload_json(trusted, &qualification.scale, &observation.stage_plan)?,
            "candidate": baseline_candidate,
            "process": monitored.process,
            "metrics": metrics,
            "guard": not_applicable_guard(environment, &monitored.monitor),
            "cleanup": {
                "experimentId": observed_string_value(&observation.experiment_id),
                "phase": phase,
                "sequenceIndex": observed(u64::from(observation.sequence_index))
            },
            "externalState": external_state_json(environment, monitored.external_state.as_ref(), true)
        });
        if observation.phase == crate::CleanupPhase::FailureIteration {
            let dimension_id = match observation.case_id {
                crate::CleanupFailureCase::SourceByteLimitPlusOne => "source-byte-count",
                crate::CleanupFailureCase::MissingReferencePerUnit
                | crate::CleanupFailureCase::DiagnosticCapPlusOne => "not-applicable",
            };
            let input_digest = observation.input_digest_sha256.as_deref().ok_or_else(|| {
                EvidenceError::CleanupRecomputation {
                    detail: format!(
                        "清理实验 {} 序号 {} 缺少失败输入摘要",
                        observation.experiment_id, observation.sequence_index
                    ),
                }
            })?;
            run["failure"] = failure_observation_json(
                observation.case_id.as_str(),
                dimension_id,
                &observation.input_variant_id,
                input_digest,
                crate::LimitRunOutcome::CompilerError,
                crate::LimitRunOutcome::CompilerError,
                observation.stable_compiler_error_code.as_deref(),
                observation.diagnostic_count,
                observation.diagnostics_truncated,
                observation.partial_output_record_count,
            );
        }
        output.push(run);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn failure_observation_json(
    case_id: &str,
    dimension_id: &str,
    input_variant_id: &str,
    input_digest: &str,
    expected_outcome: crate::LimitRunOutcome,
    actual_outcome: crate::LimitRunOutcome,
    stable_compiler_error_code: Option<&str>,
    diagnostic_count: u64,
    diagnostics_truncated: bool,
    partial_output_record_count: u64,
) -> Value {
    json!({
        "caseId": case_id,
        "dimensionId": dimension_id,
        "inputVariantId": input_variant_id,
        "inputDigest": input_digest,
        "expectedOutcome": expected_outcome,
        "actualOutcome": actual_outcome,
        "stableCompilerErrorCode": nullable_string(
            stable_compiler_error_code,
            "not-applicable-success",
        ),
        "diagnosticCount": diagnostic_count,
        "diagnosticsTruncated": diagnostics_truncated,
        "partialOutputRecordCount": partial_output_record_count
    })
}

fn incomplete_qualification_run<T>(
    trusted: &TrustedContract,
    environment: &FormalEnvironmentSnapshot,
    run_id: &str,
    scale: &crate::LimitQualificationScale,
    plan: &crate::ScalableStagePlanSummary,
    monitored: &crate::MonitoredQualificationRun<T>,
    baseline_candidate: &Value,
) -> Result<Value, EvidenceError> {
    Ok(json!({
        "runId": run_id,
        "batch": 0,
        "round": 0,
        "position": 0,
        "roundAttempt": {"id": run_id, "ordinal": 0, "scope": "single-experiment"},
        "compilerInstanceId": null_observation("child-report-missing"),
        "sampleOrdinal": 0,
        "sampleKind": "cold-instance",
        "status": monitored.status,
        "invalidationReasons": monitored.invalidation_reasons,
        "workload": qualification_workload_json(trusted, scale, plan)?,
        "candidate": baseline_candidate,
        "process": monitored.process,
        "metrics": metrics_json(
            None,
            monitor_peak_private_bytes(Some(&monitored.monitor)),
            None,
            None,
            &plan.stages,
            "child-report-missing",
        ),
        "guard": not_applicable_guard(environment, &monitored.monitor),
        "cleanup": no_cleanup(),
        "externalState": external_state_json(environment, monitored.external_state.as_ref(), true)
    }))
}

fn qualification_workload_json(
    trusted: &TrustedContract,
    scale: &crate::LimitQualificationScale,
    plan: &crate::ScalableStagePlanSummary,
) -> Result<Value, EvidenceError> {
    let mut counts = serde_json::to_value(&plan.counts)
        .expect("qualification plan counts serialize")
        .as_object()
        .expect("qualification plan counts serialize to object")
        .clone();
    merge_per_unit_counts(
        trusted,
        scale.workload_id.as_str(),
        u64::from(scale.n),
        &mut counts,
    )?;
    Ok(json!({
        "id": scale.workload_id,
        "revision": crate::WORKLOAD_REVISION_V1,
        "graphProfile": scale.graph_profile,
        "stringProfile": "short-unique-v1",
        "generatorVersion": crate::GENERATOR_VERSION_V1,
        "n": scale.n,
        "b": observed(u64::from(scale.b)),
        "scaleRole": scale.scale_role,
        "caseId": "not-applicable",
        "manifestDigest": trusted.descriptor.workload_manifest.sha256,
        "counts": counts,
        "fixtureInputs": []
    }))
}

fn limit_evidence_run_id(
    scale: &crate::LimitQualificationScale,
    pair: &crate::LimitPairPlan,
    side: crate::LimitPairSide,
) -> String {
    format!(
        "limit/{}/{}/{}/n-{}/{}/{}",
        scale.workload_id.as_str(),
        scale.graph_profile.as_str(),
        scale.scale_role.as_str(),
        scale.n,
        pair.binding.dimension_id.as_str(),
        side.as_str(),
    )
}

fn build_candidate_bindings(
    trusted: &TrustedContract,
    context: &VerificationContext,
    environment: &FormalEnvironmentSnapshot,
    safety_audit: &crate::CandidateSafetyAuditSnapshot,
) -> Result<Vec<Value>, EvidenceError> {
    let candidates = trusted.workload_manifest["candidateRegistry"]["candidates"]
        .as_array()
        .ok_or_else(|| EvidenceError::CandidateRegistryRecomputation {
            detail: "候选注册表缺少 candidates".to_owned(),
        })?;
    let mut bindings = Vec::new();
    for candidate in candidates {
        let candidate_id = candidate["id"].as_str().ok_or_else(|| {
            EvidenceError::CandidateRegistryRecomputation {
                detail: "候选缺少 id".to_owned(),
            }
        })?;
        let components = candidate["components"]
            .as_array()
            .ok_or_else(|| EvidenceError::CandidateRegistryRecomputation {
                detail: format!("候选 {candidate_id} 缺少 components"),
            })?
            .iter()
            .map(|component| candidate_component(context, environment, safety_audit, component))
            .collect::<Result<Vec<_>, _>>()?;
        for key_domain in candidate["allowedKeyDomains"].as_array().ok_or_else(|| {
            EvidenceError::CandidateRegistryRecomputation {
                detail: format!("候选 {candidate_id} 缺少 allowedKeyDomains"),
            }
        })? {
            let policy = candidate["hasherSeedPolicy"].as_str().ok_or_else(|| {
                EvidenceError::CandidateRegistryRecomputation {
                    detail: format!("候选 {candidate_id} 缺少 hasherSeedPolicy"),
                }
            })?;
            let hasher_seed = match policy {
                "fixed-u64" => json!({"value": candidate["fixedHasherSeedHexU64"], "reason": null}),
                "random-state-process-random" => null_observation("process-random-not-recorded"),
                "not-applicable" => null_observation("not-applicable"),
                other => {
                    return Err(EvidenceError::CandidateRegistryRecomputation {
                        detail: format!("候选 {candidate_id} 使用未知 seed 策略 {other}"),
                    });
                }
            };
            bindings.push(json!({
                "registryRevision": 1,
                "id": candidate_id,
                "keyDomain": key_domain,
                "components": components,
                "hasherSeedPolicy": policy,
                "hasherSeedHexU64": hasher_seed
            }));
        }
    }
    Ok(bindings)
}

fn candidate_component(
    context: &VerificationContext,
    environment: &FormalEnvironmentSnapshot,
    safety_audit: &crate::CandidateSafetyAuditSnapshot,
    component: &Value,
) -> Result<Value, EvidenceError> {
    let dependency_kind = component["dependencyKind"].as_str().ok_or_else(|| {
        EvidenceError::CandidateRegistryRecomputation {
            detail: "候选组件缺少 dependencyKind".to_owned(),
        }
    })?;
    let (version, features, audit) = match dependency_kind {
        "standard-library" => (
            environment.rustc.clone(),
            Vec::new(),
            not_applicable_dependency_audit(&context.cargo_lock_sha256),
        ),
        "local-workspace" => (
            context.repository_head.clone(),
            Vec::new(),
            not_applicable_dependency_audit(&context.cargo_lock_sha256),
        ),
        "crates-io" | "git" => {
            let source = component["dependencySource"].as_str().ok_or_else(|| {
                EvidenceError::CandidateRegistryRecomputation {
                    detail: "候选组件缺少 dependencySource".to_owned(),
                }
            })?;
            let package_name = source.rsplit('/').next().unwrap_or(source);
            let package = context
                .direct_cargo_packages
                .get(package_name)
                .ok_or_else(|| EvidenceError::CandidateRegistryRecomputation {
                    detail: format!("Cargo 元数据缺少候选依赖 {package_name}"),
                })?;
            let audit = safety_audit
                .package_audits
                .iter()
                .find(|audit| audit.package_name == package_name)
                .ok_or_else(|| EvidenceError::CandidateRegistryRecomputation {
                    detail: format!("候选安全审计缺少依赖 {package_name}"),
                })?;
            if audit.cargo_package_id != package.id
                || audit.version != package.version
                || audit.checksum_sha256 != package.checksum
                || safety_audit.cargo_lock_sha256 != context.cargo_lock_sha256
            {
                return Err(EvidenceError::CandidateRegistryRecomputation {
                    detail: format!("候选安全审计与当前锁文件中的 {package_name} 不匹配"),
                });
            }
            (
                package.version.clone(),
                package.features.iter().cloned().collect(),
                passed_dependency_audit(safety_audit, audit),
            )
        }
        other => {
            return Err(EvidenceError::CandidateRegistryRecomputation {
                detail: format!("未知候选依赖类型 {other}"),
            });
        }
    };
    Ok(json!({
        "role": component["role"],
        "implementationId": component["implementationId"],
        "version": version,
        "features": features,
        "dependencyKind": dependency_kind,
        "dependencySource": component["dependencySource"],
        "dependencyAudit": audit
    }))
}

fn not_applicable_dependency_audit(cargo_lock_sha256: &str) -> Value {
    json!({
        "licenseSpdxExpression": null_observation("not-applicable"),
        "msrvRustVersion": null_observation("not-applicable"),
        "securityAudit": {
            "tool": null_observation("not-applicable"),
            "databaseSnapshot": null_observation("not-applicable"),
            "observedAtUtc": null_observation("not-applicable"),
            "status": "not-applicable",
            "advisoryIds": []
        },
        "cargoPackageId": null_observation("not-applicable"),
        "cargoPackageChecksumSha256": null_observation("not-applicable"),
        "cargoLockSha256": cargo_lock_sha256
    })
}

fn passed_dependency_audit(
    safety_audit: &crate::CandidateSafetyAuditSnapshot,
    package: &crate::CandidatePackageSafetyAudit,
) -> Value {
    json!({
        "licenseSpdxExpression": {"value": package.license_spdx_expression, "reason": null},
        "msrvRustVersion": {"value": package.msrv_rust_version, "reason": null},
        "securityAudit": {
            "tool": {"value": safety_audit.tool, "reason": null},
            "databaseSnapshot": {"value": format!("cargo-deny-output-sha256:{}", safety_audit.output_sha256), "reason": null},
            "observedAtUtc": {"value": safety_audit.observed_at_utc, "reason": null},
            "status": "no-known-advisories",
            "advisoryIds": []
        },
        "cargoPackageId": {"value": package.cargo_package_id, "reason": null},
        "cargoPackageChecksumSha256": {"value": package.checksum_sha256, "reason": null},
        "cargoLockSha256": safety_audit.cargo_lock_sha256
    })
}

fn null_observation(reason: &str) -> Value {
    json!({"value": null, "reason": reason})
}

fn observed(value: u64) -> Value {
    json!({"value": value, "reason": null})
}

fn nullable_u64(value: Option<u64>, reason: &str) -> Value {
    value.map_or_else(|| null_observation(reason), observed)
}

fn nullable_string(value: Option<&str>, reason: &str) -> Value {
    value.map_or_else(
        || null_observation(reason),
        |value| json!({"value": value, "reason": null}),
    )
}

fn nullable_owned_string(value: Option<String>, reason: &str) -> Value {
    value.map_or_else(
        || null_observation(reason),
        |value| json!({"value": value, "reason": null}),
    )
}

fn pilot_runs(
    trusted: &TrustedContract,
    environment: &FormalEnvironmentSnapshot,
    checkpoint: &crate::BaseScalePilotCheckpoint,
    baseline_candidate: &Value,
) -> Result<Vec<Value>, EvidenceError> {
    let plans =
        crate::ScalableStagePlanFactory::from_trusted_contract(trusted).map_err(|error| {
            EvidenceError::WorkloadRecomputation {
                detail: error.to_string(),
            }
        })?;
    let mut output = Vec::with_capacity(checkpoint.runs.len() + checkpoint.oracle_runs.len());
    for run in &checkpoint.runs {
        let selection = pilot_selection(checkpoint, run.workload_id, &run.graph_profile)?;
        let plan = plans
            .plan(
                run.workload_id,
                parse_graph_profile(&run.graph_profile)?,
                run.n,
            )
            .map_err(|error| EvidenceError::WorkloadRecomputation {
                detail: error.to_string(),
            })?;
        let previous = previous_pilot_level(selection, run.n);
        let reason = if run.child.is_some() {
            "not-measured-by-timing-binary"
        } else {
            "child-not-started-or-no-valid-report"
        };
        let child_guard = run
            .child
            .as_ref()
            .and_then(|child| child.controlled_allocation_guard.as_ref());
        let sample_kind = match run.run_kind {
            crate::BaseScalePilotRunKind::ColdInstance => "cold-instance",
            crate::BaseScalePilotRunKind::GuardPreflight => "guard-preflight",
        };
        output.push(json!({
            "runId": run.run_id,
            "batch": 0,
            "round": 0,
            "position": run.pilot_sample_position,
            "roundAttempt": {"id": run.attempt_id, "ordinal": run.retry_ordinal, "scope": "single-experiment"},
            "compilerInstanceId": run.compiler_instance_id,
            "sampleOrdinal": 0,
            "sampleKind": sample_kind,
            "status": run.status,
            "invalidationReasons": run.invalidation_reasons,
            "workload": pilot_workload_json(trusted, selection, &plan)?,
            "candidate": baseline_candidate,
            "process": run.process,
            "metrics": timing_pilot_metrics(run, &plan.stages, reason),
            "guard": pilot_guard_json(
                &run.guard_preflight,
                previous,
                run.monitor.as_ref(),
                child_guard,
                run.process.exit_kind,
            ),
            "cleanup": no_cleanup(),
            "externalState": external_state_json(
                environment,
                run.external_state.as_ref(),
                run.process.child_pid.value.is_some(),
            )
        }));
    }
    for run in &checkpoint.oracle_runs {
        let selection = pilot_selection(checkpoint, run.workload_id, &run.graph_profile)?;
        let plan = plans
            .plan(
                run.workload_id,
                parse_graph_profile(&run.graph_profile)?,
                run.n,
            )
            .map_err(|error| EvidenceError::WorkloadRecomputation {
                detail: error.to_string(),
            })?;
        let previous = previous_pilot_level(selection, run.n);
        let reason = if run.child.is_some() {
            "not-measured-by-oracle"
        } else {
            "child-not-started-or-no-valid-report"
        };
        let child_guard = run
            .child
            .as_ref()
            .and_then(|child| child.controlled_allocation_guard.as_ref());
        output.push(json!({
            "runId": run.run_id,
            "batch": 0,
            "round": 0,
            "position": 7,
            "roundAttempt": {"id": run.run_id, "ordinal": 0, "scope": "single-experiment"},
            "compilerInstanceId": null_observation("not-applicable-oracle"),
            "sampleOrdinal": 0,
            "sampleKind": "oracle",
            "status": run.status,
            "invalidationReasons": run.invalidation_reasons,
            "workload": pilot_workload_json(trusted, selection, &plan)?,
            "candidate": baseline_candidate,
            "process": run.process,
            "metrics": oracle_pilot_metrics(run, &plan.stages, reason),
            "guard": pilot_guard_json(
                &run.guard_preflight,
                previous,
                run.monitor.as_ref(),
                child_guard,
                run.process.exit_kind,
            ),
            "cleanup": no_cleanup(),
            "externalState": external_state_json(
                environment,
                run.external_state.as_ref(),
                run.process.child_pid.value.is_some(),
            )
        }));
    }
    Ok(output)
}

fn pilot_selection<'a>(
    checkpoint: &'a crate::BaseScalePilotCheckpoint,
    workload_id: crate::ScalableWorkloadId,
    graph_profile: &str,
) -> Result<&'a crate::BaseScaleSelection, EvidenceError> {
    checkpoint
        .selections
        .iter()
        .find(|selection| {
            selection.workload_id == workload_id && selection.graph_profile == graph_profile
        })
        .ok_or_else(|| EvidenceError::BaseScaleRecomputation {
            detail: format!(
                "检查点缺少试运行选择 {}/{graph_profile}",
                workload_id.as_str()
            ),
        })
}

fn parse_graph_profile(value: &str) -> Result<crate::GraphProfileId, EvidenceError> {
    match value {
        "wide-star-v1" => Ok(crate::GraphProfileId::WideStar),
        "deep-chain-v1" => Ok(crate::GraphProfileId::DeepChain),
        "shared-fanin-dag-v1" => Ok(crate::GraphProfileId::SharedFaninDag),
        other => Err(EvidenceError::WorkloadRecomputation {
            detail: format!("未知模块图 profile {other}"),
        }),
    }
}

fn previous_pilot_level(
    selection: &crate::BaseScaleSelection,
    n: u32,
) -> Option<&crate::GuardCompletedLevelObservation> {
    selection
        .pilot_levels
        .iter()
        .filter(|level| level.n < n)
        .max_by_key(|level| level.n)
        .map(|level| &level.completed_level_guard_observation)
}

fn pilot_workload_json(
    trusted: &TrustedContract,
    selection: &crate::BaseScaleSelection,
    plan: &crate::ScalableStagePlanSummary,
) -> Result<Value, EvidenceError> {
    let mut counts = serde_json::to_value(&plan.counts)
        .expect("scalable plan counts serialize")
        .as_object()
        .expect("scalable plan counts serialize to object")
        .clone();
    merge_per_unit_counts(
        trusted,
        selection.workload_id.as_str(),
        u64::from(plan.n),
        &mut counts,
    )?;
    Ok(json!({
        "id": selection.workload_id,
        "revision": selection.workload_revision,
        "graphProfile": selection.graph_profile,
        "stringProfile": selection.string_profile,
        "generatorVersion": selection.generator_version,
        "n": plan.n,
        "b": selection.b,
        "scaleRole": "pilot",
        "caseId": "not-applicable",
        "manifestDigest": trusted.descriptor.workload_manifest.sha256,
        "counts": counts,
        "fixtureInputs": []
    }))
}

fn merge_per_unit_counts(
    trusted: &TrustedContract,
    workload_id: &str,
    n: u64,
    counts: &mut Map<String, Value>,
) -> Result<(), EvidenceError> {
    let workload = trusted.workload_manifest["workloads"]
        .as_array()
        .and_then(|workloads| {
            workloads
                .iter()
                .find(|workload| workload["id"] == workload_id)
        })
        .ok_or_else(|| EvidenceError::WorkloadRecomputation {
            detail: format!("工作负载清单缺少 {workload_id}"),
        })?;
    for (field, value) in workload["perUnitCounts"].as_object().ok_or_else(|| {
        EvidenceError::WorkloadRecomputation {
            detail: format!("工作负载 {workload_id} 缺少 perUnitCounts"),
        }
    })? {
        let product = value
            .as_u64()
            .and_then(|value| value.checked_mul(n))
            .ok_or_else(|| EvidenceError::WorkloadRecomputation {
                detail: format!("工作负载 {workload_id} 的 {field} 计数溢出"),
            })?;
        counts.insert(field.clone(), json!(product));
    }
    Ok(())
}

fn timing_pilot_metrics(
    run: &crate::BaseScalePilotRun,
    stages: &crate::StageBreakdown,
    reason: &str,
) -> Value {
    let wall_time = run.child.as_ref().and_then(|child| child.wall_time_ns);
    let semantic = run
        .child
        .as_ref()
        .and_then(|child| child.semantic_digest_sha256.as_deref());
    metrics_json(
        wall_time,
        monitor_peak_private_bytes(run.monitor.as_ref()),
        semantic,
        semantic.map(|_| crate::diagnostic::empty_diagnostic_digest()),
        stages,
        reason,
    )
}

fn oracle_pilot_metrics(
    run: &crate::BaseScaleOracleRun,
    stages: &crate::StageBreakdown,
    reason: &str,
) -> Value {
    let semantic = run
        .child
        .as_ref()
        .and_then(|child| child.semantic_digest_sha256.as_deref());
    metrics_json(
        None,
        monitor_peak_private_bytes(run.monitor.as_ref()),
        semantic,
        semantic.map(|_| crate::diagnostic::empty_diagnostic_digest()),
        stages,
        reason,
    )
}

fn monitor_peak_private_bytes(monitor: Option<&crate::ChildProcessMonitorReport>) -> Option<u64> {
    monitor.and_then(|monitor| monitor.peak_private_bytes.value)
}

fn pilot_guard_json(
    report: &crate::GuardPreflightReport,
    previous: Option<&crate::GuardCompletedLevelObservation>,
    monitor: Option<&crate::ChildProcessMonitorReport>,
    child_guard: Option<&crate::ControlledAllocationGuardReport>,
    exit_kind: crate::ProcessExitKind,
) -> Value {
    let no_previous_reason = "first-level-no-completed-level";
    let previous_n = previous.map_or_else(
        || null_observation(no_previous_reason),
        |previous| observed(u64::from(previous.n)),
    );
    let previous_primary = previous.map_or_else(
        || null_observation(no_previous_reason),
        |previous| observed(previous.primary_record_count),
    );
    let previous_peak = previous.map_or_else(
        || null_observation(no_previous_reason),
        |previous| observed(previous.peak_live_requested_bytes),
    );
    let previous_private = previous.map_or_else(
        || null_observation(no_previous_reason),
        |previous| observed(previous.private_bytes),
    );
    let previous_wall = previous.map_or_else(
        || null_observation(no_previous_reason),
        |previous| observed(previous.wall_time_ns),
    );
    let trigger = report
        .triggers
        .first()
        .map(serialized_token)
        .or_else(|| child_guard.map(|_| "allocation-hard-ceiling".to_owned()))
        .or_else(|| {
            monitor
                .and_then(|monitor| monitor.trigger)
                .map(|trigger| monitor_trigger(Some(trigger)))
                .map(str::to_owned)
        })
        .or_else(|| {
            matches!(
                exit_kind,
                crate::ProcessExitKind::InvalidAbnormalExit
                    | crate::ProcessExitKind::InvalidMonitorTermination
            )
            .then(|| "abnormal-exit".to_owned())
        })
        .unwrap_or_else(|| "none".to_owned());
    let last_private = monitor.map_or_else(
        || null_observation("child-not-started"),
        |monitor| {
            serde_json::to_value(&monitor.last_private_bytes).expect("monitor value serialize")
        },
    );
    json!({
        "compilerControlledPredictionBasis": report.compiler_controlled_prediction_basis,
        "privateBytesPredictionBasis": report.private_bytes_prediction_basis,
        "wallTimePredictionBasis": report.wall_time_prediction_basis,
        "previousCompletedN": previous_n,
        "previousPrimaryRecordCount": previous_primary,
        "nextPrimaryRecordCount": report.primary_record_count,
        "previousPeakLiveRequestedBytes": previous_peak,
        "predictedCompilerControlledBytes": observed(report.predicted_compiler_controlled_bytes),
        "previousPrivateBytes": previous_private,
        "predictedPrivateBytes": nullable_u64(report.predicted_private_bytes, "first-level-monitor-only"),
        "previousWallTimeNs": previous_wall,
        "predictedWallTimeNs": nullable_u64(report.predicted_wall_time_ns, "first-level-monitor-only"),
        "logicalBytesLowerBound": report.logical_bytes_lower_bound,
        "reservedBytesBeforeFailure": child_guard.map_or(0, |guard| guard.live_requested_bytes),
        "trigger": trigger,
        "lastAvailablePhysicalMemoryBytes": report.memory_observation.available_physical_memory_bytes,
        "lastPrivateBytes": last_private
    })
}

fn serialized_token<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .expect("enum token serializes")
        .as_str()
        .expect("enum token serializes to string")
        .to_owned()
}

fn formal_ladder_runs(
    trusted: &TrustedContract,
    environment: &FormalEnvironmentSnapshot,
    checkpoint: &FormalProtocolCheckpoint,
    baseline_candidate: &Value,
) -> Result<Vec<Value>, EvidenceError> {
    let plans =
        crate::ScalableStagePlanFactory::from_trusted_contract(trusted).map_err(|error| {
            EvidenceError::WorkloadRecomputation {
                detail: error.to_string(),
            }
        })?;
    let mut output = Vec::new();
    for ladder in &checkpoint.formal_ladders {
        for (level_index, level) in ladder.levels.iter().enumerate() {
            let plan = plans
                .plan(
                    ladder.workload_id,
                    parse_graph_profile(&ladder.graph_profile)?,
                    level.n,
                )
                .map_err(|error| EvidenceError::WorkloadRecomputation {
                    detail: error.to_string(),
                })?;
            let previous = previous_formal_level(checkpoint, ladder, level_index);
            let scale_role = formal_scale_role(ladder, level.n);
            if let Some(run) = &level.attribution_preflight {
                output.push(attribution_preflight_run(
                    trusted,
                    environment,
                    ladder,
                    level,
                    &plan,
                    previous,
                    scale_role,
                    run,
                    baseline_candidate,
                )?);
            }
            if let Some(run) = &level.timing_guard_run {
                append_ladder_child_runs(
                    &mut output,
                    trusted,
                    environment,
                    ladder,
                    level,
                    &plan,
                    previous,
                    scale_role,
                    &run.run_id,
                    &run.run_id,
                    0,
                    0,
                    0,
                    1,
                    run.status,
                    &run.invalidation_reasons,
                    &run.process,
                    &run.compiler_instance_id,
                    run.child.as_ref(),
                    &run.monitor,
                    run.external_state.as_ref(),
                    baseline_candidate,
                )?;
            }
            if let Some(run) = &level.oracle {
                output.push(formal_oracle_run(
                    trusted,
                    environment,
                    ladder,
                    level,
                    &plan,
                    previous,
                    scale_role,
                    run,
                    baseline_candidate,
                )?);
            }
            for run in &level.formal_runs {
                append_ladder_child_runs(
                    &mut output,
                    trusted,
                    environment,
                    ladder,
                    level,
                    &plan,
                    previous,
                    scale_role,
                    &run.run_id,
                    &run.attempt_id,
                    run.retry_ordinal,
                    run.batch,
                    run.round,
                    run.execution_position,
                    run.status,
                    &run.invalidation_reasons,
                    &run.process,
                    &run.compiler_instance_id,
                    run.child.as_ref(),
                    &run.monitor,
                    run.external_state.as_ref(),
                    baseline_candidate,
                )?;
            }
        }
        if let Some(terminal) = &ladder.terminal_guard_preflight {
            let plan = plans
                .plan(
                    ladder.workload_id,
                    parse_graph_profile(&ladder.graph_profile)?,
                    terminal.guard_preflight.n,
                )
                .map_err(|error| EvidenceError::WorkloadRecomputation {
                    detail: error.to_string(),
                })?;
            let previous = ladder
                .levels
                .last()
                .and_then(|level| level.completed_guard_observation.as_ref());
            output.push(json!({
                "runId": terminal.run_id,
                "batch": 0,
                "round": 0,
                "position": 0,
                "roundAttempt": {"id": terminal.run_id, "ordinal": 0, "scope": "single-experiment"},
                "compilerInstanceId": null_observation("compiler-instance-not-created"),
                "sampleOrdinal": 0,
                "sampleKind": "guard-preflight",
                "status": "guarded",
                "invalidationReasons": [],
                "workload": formal_workload_json(
                    trusted,
                    ladder,
                    &plan,
                    formal_scale_role(ladder, terminal.guard_preflight.n),
                )?,
                "candidate": baseline_candidate,
                "process": terminal.process,
                "metrics": metrics_json(
                    None,
                    None,
                    None,
                    None,
                    &plan.stages,
                    "child-not-started",
                ),
                "guard": pilot_guard_json(
                    &terminal.guard_preflight,
                    previous,
                    None,
                    None,
                    terminal.process.exit_kind,
                ),
                "cleanup": no_cleanup(),
                "externalState": external_state_json(environment, None, false)
            }));
        }
    }
    Ok(output)
}

fn previous_formal_level<'a>(
    checkpoint: &'a FormalProtocolCheckpoint,
    ladder: &'a crate::FormalLadderExecution,
    level_index: usize,
) -> Option<&'a crate::GuardCompletedLevelObservation> {
    if level_index > 0 {
        return ladder.levels[level_index - 1]
            .completed_guard_observation
            .as_ref();
    }
    ladder
        .b
        .checked_div(2)
        .filter(|n| *n > 0)
        .and_then(|n| {
            checkpoint
                .base_scale_pilot
                .selections
                .iter()
                .find(|selection| {
                    selection.workload_id == ladder.workload_id
                        && selection.graph_profile == ladder.graph_profile
                })
                .and_then(|selection| selection.pilot_levels.iter().find(|level| level.n == n))
        })
        .map(|level| &level.completed_level_guard_observation)
}

fn formal_scale_role(ladder: &crate::FormalLadderExecution, n: u32) -> &'static str {
    let selection = ladder
        .analysis
        .as_ref()
        .map(|analysis| &analysis.scale_selection);
    if selection.and_then(|selection| selection.calibration_n) == Some(n) {
        "calibration"
    } else if selection.and_then(|selection| selection.stress_n) == Some(n) {
        "stress"
    } else if ladder.b == n {
        "base"
    } else {
        "ladder"
    }
}

fn formal_workload_json(
    trusted: &TrustedContract,
    ladder: &crate::FormalLadderExecution,
    plan: &crate::ScalableStagePlanSummary,
    scale_role: &str,
) -> Result<Value, EvidenceError> {
    let mut counts = serde_json::to_value(&plan.counts)
        .expect("formal plan counts serialize")
        .as_object()
        .expect("formal plan counts serialize to object")
        .clone();
    merge_per_unit_counts(
        trusted,
        ladder.workload_id.as_str(),
        u64::from(plan.n),
        &mut counts,
    )?;
    Ok(json!({
        "id": ladder.workload_id,
        "revision": ladder.workload_revision,
        "graphProfile": ladder.graph_profile,
        "stringProfile": ladder.string_profile,
        "generatorVersion": ladder.generator_version,
        "n": plan.n,
        "b": observed(u64::from(ladder.b)),
        "scaleRole": scale_role,
        "caseId": "not-applicable",
        "manifestDigest": trusted.descriptor.workload_manifest.sha256,
        "counts": counts,
        "fixtureInputs": []
    }))
}

#[allow(clippy::too_many_arguments)]
fn attribution_preflight_run(
    trusted: &TrustedContract,
    environment: &FormalEnvironmentSnapshot,
    ladder: &crate::FormalLadderExecution,
    level: &crate::FormalLadderLevelExecution,
    plan: &crate::ScalableStagePlanSummary,
    previous: Option<&crate::GuardCompletedLevelObservation>,
    scale_role: &str,
    run: &crate::FormalAttributionPreflightRun,
    baseline_candidate: &Value,
) -> Result<Value, EvidenceError> {
    let child_guard = run
        .child
        .as_ref()
        .and_then(|child| child.controlled_allocation_guard.as_ref());
    Ok(json!({
        "runId": run.run_id,
        "batch": 0,
        "round": 0,
        "position": 0,
        "roundAttempt": {"id": run.run_id, "ordinal": 0, "scope": "formal-ladder-round"},
        "compilerInstanceId": observed_string_value(&run.compiler_instance_id),
        "sampleOrdinal": 0,
        "sampleKind": "cold-instance",
        "status": run.status,
        "invalidationReasons": run.invalidation_reasons,
        "workload": formal_workload_json(trusted, ladder, plan, scale_role)?,
        "candidate": baseline_candidate,
        "process": run.process,
        "metrics": attribution_preflight_metrics(run, &plan.stages),
        "guard": pilot_guard_json(
            &level.guard_preflight,
            previous,
            Some(&run.monitor),
            child_guard,
            run.process.exit_kind,
        ),
        "cleanup": no_cleanup(),
        "externalState": external_state_json(environment, run.external_state.as_ref(), true)
    }))
}

fn attribution_preflight_metrics(
    run: &crate::FormalAttributionPreflightRun,
    stages: &crate::StageBreakdown,
) -> Value {
    let child = run.child.as_ref();
    let allocation = child.and_then(|child| child.allocation.as_ref());
    let semantic = child.and_then(|child| child.semantic_digest_sha256.as_deref());
    attribution_metrics(
        allocation,
        child.and_then(|child| child.retained_capacity_bytes.as_ref()),
        monitor_peak_private_bytes(Some(&run.monitor)),
        semantic,
        stages,
        "attribution-preflight-incomplete",
    )
}

#[allow(clippy::too_many_arguments)]
fn append_ladder_child_runs(
    output: &mut Vec<Value>,
    trusted: &TrustedContract,
    environment: &FormalEnvironmentSnapshot,
    ladder: &crate::FormalLadderExecution,
    level: &crate::FormalLadderLevelExecution,
    plan: &crate::ScalableStagePlanSummary,
    previous: Option<&crate::GuardCompletedLevelObservation>,
    scale_role: &str,
    process_run_id: &str,
    attempt_id: &str,
    retry_ordinal: u32,
    batch: u32,
    round: u32,
    position: u32,
    status: crate::RunStatus,
    invalidation_reasons: &[crate::InvalidationReason],
    process: &crate::ProcessObservation,
    compiler_instance_id: &str,
    child: Option<&crate::ScalableLadderChildReport>,
    monitor: &crate::ChildProcessMonitorReport,
    external_state: Option<&crate::ExternalStateObservation>,
    baseline_candidate: &Value,
) -> Result<(), EvidenceError> {
    let child_guard = child.and_then(|child| child.controlled_allocation_guard.as_ref());
    let mut samples = Vec::new();
    if let Some(child) = child {
        if let Some(cold) = &child.cold_instance {
            samples.push(("cold-instance", 0_u32, cold));
        }
        samples.extend(
            child
                .stable_capacity_reuse
                .iter()
                .map(|sample| ("stable-capacity-reuse", sample.sample_ordinal, sample)),
        );
    }
    if samples.is_empty() {
        output.push(formal_sample_run(
            trusted,
            environment,
            ladder,
            level,
            plan,
            previous,
            scale_role,
            &format!("{process_run_id}/incomplete"),
            attempt_id,
            retry_ordinal,
            batch,
            round,
            position,
            "cold-instance",
            0,
            status,
            invalidation_reasons,
            process,
            compiler_instance_id,
            child.map(|child| child.binary_mode),
            None,
            child.and_then(|child| child.retained_capacity_bytes.as_ref()),
            monitor,
            child_guard,
            external_state,
            baseline_candidate,
        )?);
        return Ok(());
    }
    for (sample_kind, sample_ordinal, sample) in samples {
        output.push(formal_sample_run(
            trusted,
            environment,
            ladder,
            level,
            plan,
            previous,
            scale_role,
            &format!("{process_run_id}/{sample_kind}-{sample_ordinal}"),
            attempt_id,
            retry_ordinal,
            batch,
            round,
            position,
            sample_kind,
            sample_ordinal,
            status,
            invalidation_reasons,
            process,
            compiler_instance_id,
            child.map(|child| child.binary_mode),
            Some(sample),
            child.and_then(|child| child.retained_capacity_bytes.as_ref()),
            monitor,
            child_guard,
            external_state,
            baseline_candidate,
        )?);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn formal_sample_run(
    trusted: &TrustedContract,
    environment: &FormalEnvironmentSnapshot,
    ladder: &crate::FormalLadderExecution,
    level: &crate::FormalLadderLevelExecution,
    plan: &crate::ScalableStagePlanSummary,
    previous: Option<&crate::GuardCompletedLevelObservation>,
    scale_role: &str,
    run_id: &str,
    attempt_id: &str,
    retry_ordinal: u32,
    batch: u32,
    round: u32,
    position: u32,
    sample_kind: &str,
    sample_ordinal: u32,
    status: crate::RunStatus,
    invalidation_reasons: &[crate::InvalidationReason],
    process: &crate::ProcessObservation,
    compiler_instance_id: &str,
    binary_mode: Option<crate::ScalableLadderBinaryMode>,
    sample: Option<&crate::ScalableLadderSample>,
    retained: Option<&crate::StageRetainedCapacityBytes>,
    monitor: &crate::ChildProcessMonitorReport,
    child_guard: Option<&crate::ControlledAllocationGuardReport>,
    external_state: Option<&crate::ExternalStateObservation>,
    baseline_candidate: &Value,
) -> Result<Value, EvidenceError> {
    Ok(json!({
        "runId": run_id,
        "batch": batch,
        "round": round,
        "position": position,
        "roundAttempt": {"id": attempt_id, "ordinal": retry_ordinal, "scope": "formal-ladder-round"},
        "compilerInstanceId": observed_string_value(compiler_instance_id),
        "sampleOrdinal": sample_ordinal,
        "sampleKind": sample_kind,
        "status": status,
        "invalidationReasons": invalidation_reasons,
        "workload": formal_workload_json(trusted, ladder, plan, scale_role)?,
        "candidate": baseline_candidate,
        "process": process,
        "metrics": ladder_sample_metrics(binary_mode, sample, retained, monitor, &plan.stages),
        "guard": pilot_guard_json(
            &level.guard_preflight,
            previous,
            Some(monitor),
            child_guard,
            process.exit_kind,
        ),
        "cleanup": no_cleanup(),
        "externalState": external_state_json(environment, external_state, true)
    }))
}

fn ladder_sample_metrics(
    binary_mode: Option<crate::ScalableLadderBinaryMode>,
    sample: Option<&crate::ScalableLadderSample>,
    retained: Option<&crate::StageRetainedCapacityBytes>,
    monitor: &crate::ChildProcessMonitorReport,
    stages: &crate::StageBreakdown,
) -> Value {
    match binary_mode {
        Some(crate::ScalableLadderBinaryMode::Attribution) => attribution_metrics(
            sample.and_then(|sample| sample.allocation.as_ref()),
            retained,
            monitor_peak_private_bytes(Some(monitor)),
            sample.map(|sample| sample.semantic_digest_sha256.as_str()),
            stages,
            "attribution-child-incomplete",
        ),
        _ => metrics_json(
            sample.and_then(|sample| sample.wall_time_ns),
            monitor_peak_private_bytes(Some(monitor)),
            sample.map(|sample| sample.semantic_digest_sha256.as_str()),
            sample.map(|_| crate::diagnostic::empty_diagnostic_digest()),
            stages,
            "timing-child-incomplete",
        ),
    }
}

fn attribution_metrics(
    allocation: Option<&crate::IdentityAllocationSnapshot>,
    retained: Option<&crate::StageRetainedCapacityBytes>,
    private_bytes: Option<u64>,
    semantic_digest: Option<&str>,
    stages: &crate::StageBreakdown,
    reason: &str,
) -> Value {
    let allocated_bytes = allocation.and_then(|allocation| {
        allocation
            .allocated_bytes
            .checked_add(allocation.reallocated_bytes)
    });
    json!({
        "wallTimeNs": null_observation("timing-binary-only"),
        "allocationCount": nullable_u64(allocation.map(|value| value.allocation_count), reason),
        "reallocationCount": nullable_u64(allocation.map(|value| value.reallocation_count), reason),
        "allocatedBytes": nullable_u64(allocated_bytes, reason),
        "freedBytes": nullable_u64(allocation.map(|value| value.freed_bytes), reason),
        "liveRequestedBytes": nullable_u64(allocation.map(|value| value.live_requested_bytes), reason),
        "peakLiveRequestedBytes": nullable_u64(allocation.map(|value| value.peak_live_requested_bytes), reason),
        "retainedCapacityBytes": nullable_u64(retained.map(|value| value.total), reason),
        "workingSetBytes": null_observation("not-collected-by-rust-research-binary"),
        "privateBytes": nullable_u64(private_bytes, reason),
        "commitPeakBytes": null_observation("not-collected-by-rust-research-binary"),
        "semanticDigest": nullable_string(semantic_digest, reason),
        "diagnosticDigest": nullable_owned_string(
            semantic_digest.map(|_| crate::diagnostic::empty_diagnostic_digest()),
            reason,
        ),
        "stageBreakdown": stage_breakdown_json(stages, "per-stage-attribution-not-collected")
    })
}

#[allow(clippy::too_many_arguments)]
fn formal_oracle_run(
    trusted: &TrustedContract,
    environment: &FormalEnvironmentSnapshot,
    ladder: &crate::FormalLadderExecution,
    level: &crate::FormalLadderLevelExecution,
    plan: &crate::ScalableStagePlanSummary,
    previous: Option<&crate::GuardCompletedLevelObservation>,
    scale_role: &str,
    run: &crate::FormalOracleRun,
    baseline_candidate: &Value,
) -> Result<Value, EvidenceError> {
    let semantic = run
        .child
        .as_ref()
        .and_then(|child| child.semantic_digest_sha256.as_deref());
    let child_guard = run
        .child
        .as_ref()
        .and_then(|child| child.controlled_allocation_guard.as_ref());
    Ok(json!({
        "runId": run.run_id,
        "batch": 0,
        "round": 0,
        "position": 2,
        "roundAttempt": {"id": run.run_id, "ordinal": 0, "scope": "single-experiment"},
        "compilerInstanceId": null_observation("not-applicable-oracle"),
        "sampleOrdinal": 0,
        "sampleKind": "oracle",
        "status": run.status,
        "invalidationReasons": run.invalidation_reasons,
        "workload": formal_workload_json(trusted, ladder, plan, scale_role)?,
        "candidate": baseline_candidate,
        "process": run.process,
        "metrics": metrics_json(
            None,
            monitor_peak_private_bytes(Some(&run.monitor)),
            semantic,
            semantic.map(|_| crate::diagnostic::empty_diagnostic_digest()),
            &plan.stages,
            "oracle-child-incomplete",
        ),
        "guard": pilot_guard_json(
            &level.guard_preflight,
            previous,
            Some(&run.monitor),
            child_guard,
            run.process.exit_kind,
        ),
        "cleanup": no_cleanup(),
        "externalState": external_state_json(environment, run.external_state.as_ref(), true)
    }))
}

fn observed_string_value(value: &str) -> Value {
    json!({"value": value, "reason": null})
}

#[derive(Default)]
struct CandidateDerivedEvidence {
    runs: Vec<Value>,
    constant_hash_qualifications: Vec<Value>,
    round_summaries: Vec<Value>,
    rosters: Vec<Value>,
    comparisons: Vec<Value>,
}

fn candidate_evidence(
    trusted: &TrustedContract,
    environment: &FormalEnvironmentSnapshot,
    bundle: &crate::CandidateMatrixExecutionBundle,
    candidate_bindings: &[Value],
    baseline_candidate: &Value,
    reproducibility_envelopes: &[Value],
) -> Result<CandidateDerivedEvidence, EvidenceError> {
    if bundle.schema != crate::CANDIDATE_MATRIX_CHECKPOINT_SCHEMA
        || bundle.schema_version != crate::CANDIDATE_MATRIX_CHECKPOINT_SCHEMA_VERSION
        || bundle.active_execution.is_some()
        || bundle.scope.comparison_metrics != ["wall-time-ns"]
    {
        return Err(EvidenceError::CandidateRegistryRecomputation {
            detail: "候选矩阵检查点未完成或性能指标范围不匹配".to_owned(),
        });
    }
    let plans =
        crate::ScalableStagePlanFactory::from_trusted_contract(trusted).map_err(|error| {
            EvidenceError::WorkloadRecomputation {
                detail: error.to_string(),
            }
        })?;
    let mut output = CandidateDerivedEvidence::default();
    append_constant_hash_evidence(
        &mut output,
        trusted,
        environment,
        &plans,
        &bundle.constant_hash_qualifications,
        candidate_bindings,
        baseline_candidate,
    )?;
    for execution in &bundle.executions {
        let plan = plans
            .plan(
                execution.stratum.workload_id,
                execution.stratum.graph_profile,
                execution.stratum.n,
            )
            .map_err(|error| EvidenceError::WorkloadRecomputation {
                detail: error.to_string(),
            })?;
        let roster_id = candidate_roster_id(&execution.stratum);
        append_candidate_qualification_runs(
            &mut output.runs,
            trusted,
            environment,
            execution,
            &plan,
            candidate_bindings,
            baseline_candidate,
        )?;
        output
            .rosters
            .push(candidate_roster_json(execution, &roster_id));
        append_candidate_performance_runs(
            &mut output.runs,
            trusted,
            environment,
            execution,
            &plan,
            candidate_bindings,
        )?;
        let summaries = candidate_round_summaries(execution)?;
        output.round_summaries.extend(summaries);
        output.comparisons.extend(candidate_comparisons(
            execution,
            &roster_id,
            reproducibility_envelopes,
        )?);
    }
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
fn append_constant_hash_evidence(
    output: &mut CandidateDerivedEvidence,
    trusted: &TrustedContract,
    environment: &FormalEnvironmentSnapshot,
    plans: &crate::ScalableStagePlanFactory,
    executions: &[crate::ConstantHashQualificationExecution],
    candidate_bindings: &[Value],
    baseline_candidate: &Value,
) -> Result<(), EvidenceError> {
    let plan = plans
        .plan(
            crate::ScalableWorkloadId::Corridor,
            crate::GraphProfileId::WideStar,
            1,
        )
        .map_err(|error| EvidenceError::WorkloadRecomputation {
            detail: error.to_string(),
        })?;
    for execution in executions {
        if execution.runs.len() != 6 {
            return Err(EvidenceError::ConstantHashQualificationRecomputation {
                detail: format!(
                    "{} 必须保留六个新进程运行，实际 {}",
                    execution.qualification.qualification_id,
                    execution.runs.len()
                ),
            });
        }
        let candidate_binding = find_candidate_binding(
            candidate_bindings,
            &execution.qualification.candidate_id,
            crate::CandidateKeyDomain::ValidatedFixedKey,
        )?;
        let mut canonical_candidate_ids = Vec::new();
        let mut canonical_oracle_id = None;
        let mut missing_candidate_ids = Vec::new();
        let mut missing_oracle_id = None;
        let mut observations = Vec::new();
        for (index, run) in execution.runs.iter().enumerate() {
            let (variant, role, repeat) = constant_hash_expected_slot(index)?;
            let run_id = run.run_id.clone();
            match (variant, role) {
                (
                    "constant-hash-canonical-valid-v1",
                    crate::ConstantHashRole::CandidateUnderTest,
                ) => {
                    canonical_candidate_ids.push(run_id.clone());
                }
                (
                    "constant-hash-canonical-valid-v1",
                    crate::ConstantHashRole::ExactResearchOracle,
                ) => {
                    canonical_oracle_id = Some(run_id.clone());
                }
                (
                    "constant-hash-missing-reference-v1",
                    crate::ConstantHashRole::CandidateUnderTest,
                ) => {
                    missing_candidate_ids.push(run_id.clone());
                }
                (
                    "constant-hash-missing-reference-v1",
                    crate::ConstantHashRole::ExactResearchOracle,
                ) => {
                    missing_oracle_id = Some(run_id.clone());
                }
                _ => unreachable!("constant hash variants are closed"),
            }
            if let Some(child) = &run.child {
                if child.candidate_id != execution.qualification.candidate_id
                    || child.observation.input_variant_id != variant
                    || child.observation.role != role
                    || child.observation.repeat != repeat
                {
                    return Err(EvidenceError::ConstantHashQualificationRecomputation {
                        detail: format!("恒定哈希运行 {} 与冻结槽位不匹配", run.run_id),
                    });
                }
                observations.push(child.observation.clone());
            }
            output.runs.push(constant_hash_run_json(
                trusted,
                environment,
                execution,
                run,
                &plan,
                candidate_binding,
                baseline_candidate,
                variant,
                role,
                repeat,
            )?);
        }
        let checks = constant_hash_checks(&observations);
        let all_runs_valid = execution
            .runs
            .iter()
            .all(|run| run.status == crate::RunStatus::Valid);
        let passed = all_runs_valid && checks.all_true();
        if passed != execution.qualification.passed {
            return Err(EvidenceError::ConstantHashQualificationRecomputation {
                detail: format!(
                    "{} 的新进程运行与资格汇总不一致",
                    execution.qualification.qualification_id
                ),
            });
        }
        output.constant_hash_qualifications.push(json!({
            "qualificationId": execution.qualification.qualification_id,
            "candidateId": execution.qualification.candidate_id,
            "protocol": execution.qualification.protocol_id,
            "candidateBuilder": execution.qualification.candidate_builder_id,
            "oracleBuilder": execution.qualification.oracle_builder_id,
            "canonicalValidCandidateRunIds": canonical_candidate_ids,
            "canonicalValidOracleRunId": canonical_oracle_id.ok_or_else(|| EvidenceError::ConstantHashQualificationRecomputation { detail: "恒定哈希规范输入缺少预言机槽位".to_owned() })?,
            "missingReferenceCandidateRunIds": missing_candidate_ids,
            "missingReferenceOracleRunId": missing_oracle_id.ok_or_else(|| EvidenceError::ConstantHashQualificationRecomputation { detail: "恒定哈希缺失引用输入缺少预言机槽位".to_owned() })?,
            "allStageCountsMatchOracle": checks.stage_counts,
            "semanticDigestsMatchOracle": checks.semantic_digests,
            "diagnosticDigestsMatchOracle": checks.diagnostic_digests,
            "candidateRepeatsDeterministic": checks.repeats_deterministic,
            "stableOutcomesMatchOracle": checks.outcomes,
            "partialOutputCountsMatchOracle": checks.partial_outputs,
            "passed": passed
        }));
    }
    Ok(())
}

fn constant_hash_expected_slot(
    index: usize,
) -> Result<(&'static str, crate::ConstantHashRole, u32), EvidenceError> {
    use crate::ConstantHashRole::{CandidateUnderTest, ExactResearchOracle};
    match index {
        0 => Ok(("constant-hash-canonical-valid-v1", CandidateUnderTest, 0)),
        1 => Ok(("constant-hash-canonical-valid-v1", CandidateUnderTest, 1)),
        2 => Ok(("constant-hash-canonical-valid-v1", ExactResearchOracle, 0)),
        3 => Ok(("constant-hash-missing-reference-v1", CandidateUnderTest, 0)),
        4 => Ok(("constant-hash-missing-reference-v1", CandidateUnderTest, 1)),
        5 => Ok(("constant-hash-missing-reference-v1", ExactResearchOracle, 0)),
        _ => Err(EvidenceError::ConstantHashQualificationRecomputation {
            detail: format!("恒定哈希运行槽位 {index} 超出六运行协议"),
        }),
    }
}

#[derive(Clone, Copy, Default)]
struct ConstantHashChecks {
    stage_counts: bool,
    semantic_digests: bool,
    diagnostic_digests: bool,
    repeats_deterministic: bool,
    outcomes: bool,
    partial_outputs: bool,
}

impl ConstantHashChecks {
    fn all_true(self) -> bool {
        self.stage_counts
            && self.semantic_digests
            && self.diagnostic_digests
            && self.repeats_deterministic
            && self.outcomes
            && self.partial_outputs
    }
}

fn constant_hash_checks(observations: &[crate::ConstantHashObservation]) -> ConstantHashChecks {
    let mut checks = ConstantHashChecks {
        stage_counts: true,
        semantic_digests: true,
        diagnostic_digests: true,
        repeats_deterministic: true,
        outcomes: true,
        partial_outputs: true,
    };
    if observations.len() != 6 {
        return ConstantHashChecks::default();
    }
    for variant in [
        "constant-hash-canonical-valid-v1",
        "constant-hash-missing-reference-v1",
    ] {
        let candidates = observations
            .iter()
            .filter(|observation| {
                observation.input_variant_id == variant
                    && observation.role == crate::ConstantHashRole::CandidateUnderTest
            })
            .collect::<Vec<_>>();
        let oracles = observations
            .iter()
            .filter(|observation| {
                observation.input_variant_id == variant
                    && observation.role == crate::ConstantHashRole::ExactResearchOracle
            })
            .collect::<Vec<_>>();
        if candidates.len() != 2 || oracles.len() != 1 {
            return ConstantHashChecks::default();
        }
        let oracle = oracles[0];
        checks.stage_counts &= candidates.iter().all(|candidate| {
            candidate.stage_counts_digest_sha256 == oracle.stage_counts_digest_sha256
        });
        checks.semantic_digests &= candidates
            .iter()
            .all(|candidate| candidate.semantic_digest_sha256 == oracle.semantic_digest_sha256);
        checks.diagnostic_digests &= candidates
            .iter()
            .all(|candidate| candidate.diagnostic_digest_sha256 == oracle.diagnostic_digest_sha256);
        checks.partial_outputs &= candidates.iter().all(|candidate| {
            candidate.partial_output_record_count == oracle.partial_output_record_count
        });
        checks.outcomes &= candidates.iter().all(|candidate| {
            candidate.outcome == oracle.outcome && candidate.error_code == oracle.error_code
        });
        checks.repeats_deterministic &= candidates[0].outcome == candidates[1].outcome
            && candidates[0].error_code == candidates[1].error_code
            && candidates[0].stage_counts_digest_sha256 == candidates[1].stage_counts_digest_sha256
            && candidates[0].semantic_digest_sha256 == candidates[1].semantic_digest_sha256
            && candidates[0].diagnostic_digest_sha256 == candidates[1].diagnostic_digest_sha256
            && candidates[0].partial_output_record_count
                == candidates[1].partial_output_record_count;
    }
    checks
}

#[allow(clippy::too_many_arguments)]
fn constant_hash_run_json(
    trusted: &TrustedContract,
    environment: &FormalEnvironmentSnapshot,
    execution: &crate::ConstantHashQualificationExecution,
    run: &crate::ConstantHashProcessRun,
    plan: &crate::ScalableStagePlanSummary,
    candidate_binding: &Value,
    baseline_candidate: &Value,
    variant: &str,
    role: crate::ConstantHashRole,
    repeat: u32,
) -> Result<Value, EvidenceError> {
    let child = run.child.as_ref();
    let observation = child.map(|child| &child.observation);
    let is_candidate = role == crate::ConstantHashRole::CandidateUnderTest;
    let expected_success = variant == "constant-hash-canonical-valid-v1";
    let expected_outcome = if expected_success {
        "success"
    } else {
        "compiler-error"
    };
    let expected_error = if expected_success {
        null_observation("no-compiler-error-expected")
    } else {
        observed_string_value(crate::UNKNOWN_REFERENCE_ERROR_CODE)
    };
    let (actual_outcome, actual_error, actual_diagnostic_count, actual_partial_output) =
        observation.map_or_else(
            || {
                (
                    "abnormal-termination",
                    null_observation("abnormal-termination"),
                    0,
                    0,
                )
            },
            |observation| match observation.outcome {
                crate::ConstantHashOutcome::Success => (
                    "success",
                    null_observation("no-compiler-error-observed"),
                    0,
                    observation.partial_output_record_count,
                ),
                crate::ConstantHashOutcome::CompilerError => (
                    "compiler-error",
                    nullable_string(
                        observation.error_code.as_deref(),
                        "compiler-error-code-missing",
                    ),
                    1,
                    observation.partial_output_record_count,
                ),
            },
        );
    let semantic = observation.map(|observation| observation.semantic_digest_sha256.as_str());
    let diagnostic = observation.map(|observation| observation.diagnostic_digest_sha256.clone());
    let mut metrics = metrics_json(
        None,
        monitor_peak_private_bytes(Some(&run.monitor)),
        semantic,
        diagnostic,
        &plan.stages,
        "constant-hash-child-incomplete",
    );
    metrics["stageBreakdown"] =
        stage_breakdown_json(&plan.stages, "not-measured-by-correctness-run");
    Ok(json!({
        "runId": run.run_id,
        "batch": 0,
        "round": 0,
        "position": repeat,
        "roundAttempt": {"id": run.run_id, "ordinal": 0, "scope": "single-experiment"},
        "compilerInstanceId": null_observation("not-applicable-correctness-qualification"),
        "sampleOrdinal": 0,
        "sampleKind": "correctness",
        "status": run.status,
        "invalidationReasons": run.invalidation_reasons,
        "workload": constant_hash_workload_json(trusted, plan)?,
        "candidate": if is_candidate { candidate_binding } else { baseline_candidate },
        "process": run.process,
        "metrics": metrics,
        "guard": not_applicable_guard(environment, &run.monitor),
        "correctnessQualification": {
            "qualificationId": execution.qualification.qualification_id,
            "protocol": execution.qualification.protocol_id,
            "role": if is_candidate { "candidate-collision-builder" } else { "exact-oracle" },
            "builder": if is_candidate { execution.qualification.candidate_builder_id.as_str() } else { execution.qualification.oracle_builder_id.as_str() },
            "candidateUnderTestId": execution.qualification.candidate_id,
            "inputVariantId": variant,
            "repeatIndex": repeat,
            "expectedOutcome": expected_outcome,
            "actualOutcome": actual_outcome,
            "expectedStableCompilerErrorCode": expected_error,
            "actualStableCompilerErrorCode": actual_error,
            "expectedDiagnosticCount": if expected_success { 0 } else { 1 },
            "actualDiagnosticCount": actual_diagnostic_count,
            "expectedDiagnosticsTruncated": false,
            "actualDiagnosticsTruncated": false,
            "expectedPartialOutputRecordCount": 0,
            "actualPartialOutputRecordCount": actual_partial_output
        },
        "cleanup": no_cleanup(),
        "externalState": external_state_json(environment, run.external_state.as_ref(), true)
    }))
}

fn constant_hash_workload_json(
    trusted: &TrustedContract,
    plan: &crate::ScalableStagePlanSummary,
) -> Result<Value, EvidenceError> {
    let mut counts = serde_json::to_value(&plan.counts)
        .expect("constant hash plan counts serialize")
        .as_object()
        .expect("constant hash plan counts serialize to object")
        .clone();
    merge_per_unit_counts(
        trusted,
        crate::ScalableWorkloadId::Corridor.as_str(),
        1,
        &mut counts,
    )?;
    Ok(json!({
        "id": crate::ScalableWorkloadId::Corridor,
        "revision": crate::WORKLOAD_REVISION_V1,
        "graphProfile": crate::GraphProfileId::WideStar,
        "stringProfile": crate::BASE_SCALE_STRING_PROFILE,
        "generatorVersion": crate::GENERATOR_VERSION_V1,
        "n": 1,
        "b": null_observation("not-applicable-correctness-qualification"),
        "scaleRole": "known-vector",
        "caseId": "not-applicable",
        "manifestDigest": trusted.descriptor.workload_manifest.sha256,
        "counts": counts,
        "fixtureInputs": []
    }))
}

#[allow(clippy::too_many_arguments)]
fn append_candidate_qualification_runs(
    output: &mut Vec<Value>,
    trusted: &TrustedContract,
    environment: &FormalEnvironmentSnapshot,
    execution: &crate::CandidatePipelineExecution,
    plan: &crate::ScalableStagePlanSummary,
    candidate_bindings: &[Value],
    baseline_candidate: &Value,
) -> Result<(), EvidenceError> {
    let roster = &execution.roster;
    let oracle = &roster.oracle_run;
    let oracle_semantic = oracle
        .child
        .as_ref()
        .and_then(|child| child.semantic_digest_sha256.as_deref());
    output.push(json!({
        "runId": oracle.run_id,
        "batch": 0,
        "round": 0,
        "position": 0,
        "roundAttempt": {"id": oracle.run_id, "ordinal": 0, "scope": "single-experiment"},
        "compilerInstanceId": null_observation("not-applicable-oracle"),
        "sampleOrdinal": 0,
        "sampleKind": "candidate-qualification",
        "status": oracle.status,
        "invalidationReasons": oracle.invalidation_reasons,
        "workload": candidate_workload_json(trusted, &execution.stratum, plan)?,
        "candidate": baseline_candidate,
        "process": oracle.process,
        "metrics": metrics_json(
            None,
            monitor_peak_private_bytes(Some(&oracle.monitor)),
            oracle_semantic,
            oracle_semantic.map(|_| crate::diagnostic::empty_diagnostic_digest()),
            &plan.stages,
            "candidate-qualification-oracle-incomplete"
        ),
        "guard": not_applicable_guard(environment, &oracle.monitor),
        "cleanup": no_cleanup(),
        "externalState": external_state_json(environment, oracle.external_state.as_ref(), true)
    }));
    for (position, run) in roster.candidate_runs.iter().enumerate() {
        let binding = find_candidate_binding(
            candidate_bindings,
            &run.candidate_id,
            execution.stratum.key_domain,
        )?;
        let child = run.child.as_ref();
        let semantic = child.and_then(|child| child.semantic_digest_sha256.as_deref());
        let compiler_instance_id = child
            .map(|child| child.compiler_instance_id.clone())
            .unwrap_or_else(|| format!("{}/compiler", run.run_id));
        output.push(json!({
            "runId": run.run_id,
            "batch": 0,
            "round": 0,
            "position": position + 1,
            "roundAttempt": {"id": run.run_id, "ordinal": 0, "scope": "single-experiment"},
            "compilerInstanceId": observed_string_value(&compiler_instance_id),
            "sampleOrdinal": 0,
            "sampleKind": "candidate-qualification",
            "status": run.status,
            "invalidationReasons": run.invalidation_reasons,
            "workload": candidate_workload_json(trusted, &execution.stratum, plan)?,
            "candidate": binding,
            "process": run.process,
            "metrics": metrics_json(
                child.and_then(|child| child.wall_time_ns),
                monitor_peak_private_bytes(Some(&run.monitor)),
                semantic,
                semantic.map(|_| crate::diagnostic::empty_diagnostic_digest()),
                &plan.stages,
                "candidate-qualification-child-incomplete"
            ),
            "guard": not_applicable_guard(environment, &run.monitor),
            "cleanup": no_cleanup(),
            "externalState": external_state_json(environment, run.external_state.as_ref(), true)
        }));
    }
    Ok(())
}

fn candidate_roster_json(execution: &crate::CandidatePipelineExecution, roster_id: &str) -> Value {
    let entries = execution
        .roster
        .entries
        .iter()
        .map(|entry| {
            let constant_hash = entry.constant_hash_qualification_id.as_ref().map_or_else(
                || {
                    if crate::candidate_matrix::FAST_HASH_CANDIDATES
                        .contains(&entry.candidate_id.as_str())
                    {
                        let reason =
                            if entry.disposition == crate::CandidateDisposition::RejectedSafety {
                                "qualification-not-run-safety-pre-rejection"
                            } else {
                                "qualification-not-run-insufficient-evidence"
                            };
                        null_observation(reason)
                    } else {
                        null_observation("not-applicable-non-fast-hash-candidate")
                    }
                },
                |qualification_id| observed_string_value(qualification_id),
            );
            json!({
                "candidateId": entry.candidate_id,
                "disposition": candidate_disposition(entry.disposition),
                "correctnessEvidenceRunIds": entry.correctness_evidence_run_ids,
                "constantHashQualificationId": constant_hash
            })
        })
        .collect::<Vec<_>>();
    json!({
        "rosterId": roster_id,
        "stratum": candidate_qualification_stratum(&execution.stratum),
        "baselineId": execution.roster.baseline_id,
        "entries": entries
    })
}

fn candidate_disposition(disposition: crate::CandidateDisposition) -> &'static str {
    match disposition {
        crate::CandidateDisposition::BaselineParticipant => "baseline-participant",
        crate::CandidateDisposition::PerformanceParticipant => "performance-participant",
        crate::CandidateDisposition::RejectedSafety => "rejected-safety",
        crate::CandidateDisposition::RejectedCorrectness => "rejected-correctness",
        crate::CandidateDisposition::InsufficientQualificationEvidence => {
            "insufficient-qualification-evidence"
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn append_candidate_performance_runs(
    output: &mut Vec<Value>,
    trusted: &TrustedContract,
    environment: &FormalEnvironmentSnapshot,
    execution: &crate::CandidatePipelineExecution,
    plan: &crate::ScalableStagePlanSummary,
    candidate_bindings: &[Value],
) -> Result<(), EvidenceError> {
    for attempt in &execution.attempts {
        let binding = find_candidate_binding(
            candidate_bindings,
            &attempt.candidate_id,
            execution.stratum.key_domain,
        )?;
        let child = attempt.child.as_ref();
        let semantic = child.and_then(|child| child.semantic_digest_sha256.as_deref());
        let compiler_instance_id = child
            .map(|child| child.compiler_instance_id.clone())
            .unwrap_or_else(|| format!("{}/compiler", attempt.run_id));
        output.push(json!({
            "runId": attempt.run_id,
            "batch": attempt.batch,
            "round": attempt.round,
            "position": attempt.position,
            "roundAttempt": {
                "id": attempt.round_attempt_id,
                "ordinal": attempt.retry_ordinal,
                "scope": "candidate-comparison-round"
            },
            "compilerInstanceId": observed_string_value(&compiler_instance_id),
            "sampleOrdinal": 0,
            "sampleKind": "cold-instance",
            "status": attempt.status,
            "invalidationReasons": attempt.invalidation_reasons,
            "workload": candidate_workload_json(trusted, &execution.stratum, plan)?,
            "candidate": binding,
            "process": attempt.process,
            "metrics": metrics_json(
                child.and_then(|child| child.wall_time_ns),
                monitor_peak_private_bytes(Some(&attempt.monitor)),
                semantic,
                semantic.map(|_| crate::diagnostic::empty_diagnostic_digest()),
                &plan.stages,
                "candidate-pipeline-child-incomplete"
            ),
            "guard": not_applicable_guard(environment, &attempt.monitor),
            "cleanup": no_cleanup(),
            "externalState": external_state_json(environment, attempt.external_state.as_ref(), true)
        }));
    }
    Ok(())
}

fn candidate_workload_json(
    trusted: &TrustedContract,
    stratum: &crate::CandidatePipelineStratum,
    plan: &crate::ScalableStagePlanSummary,
) -> Result<Value, EvidenceError> {
    let mut counts = serde_json::to_value(&plan.counts)
        .expect("candidate plan counts serialize")
        .as_object()
        .expect("candidate plan counts serialize to object")
        .clone();
    merge_per_unit_counts(
        trusted,
        stratum.workload_id.as_str(),
        u64::from(stratum.n),
        &mut counts,
    )?;
    Ok(json!({
        "id": stratum.workload_id,
        "revision": stratum.workload_revision,
        "graphProfile": stratum.graph_profile,
        "stringProfile": stratum.string_profile,
        "generatorVersion": stratum.generator_version,
        "n": stratum.n,
        "b": observed(u64::from(stratum.b)),
        "scaleRole": stratum.scale_role,
        "caseId": stratum.case_id,
        "inputVariantId": stratum.input_variant_id,
        "manifestDigest": trusted.descriptor.workload_manifest.sha256,
        "counts": counts,
        "fixtureInputs": []
    }))
}

fn candidate_qualification_stratum(stratum: &crate::CandidatePipelineStratum) -> Value {
    json!({
        "keyDomain": stratum.key_domain,
        "workloadId": stratum.workload_id,
        "workloadRevision": stratum.workload_revision,
        "graphProfile": stratum.graph_profile,
        "stringProfile": stratum.string_profile,
        "generatorVersion": stratum.generator_version,
        "n": stratum.n,
        "b": observed(u64::from(stratum.b)),
        "scaleRole": stratum.scale_role,
        "caseId": stratum.case_id,
        "inputVariantId": stratum.input_variant_id
    })
}

fn candidate_comparison_stratum(stratum: &crate::CandidatePipelineStratum) -> Value {
    let mut value = candidate_qualification_stratum(stratum);
    value["sampleKind"] = json!(stratum.sample_kind);
    value["binaryMode"] = json!(stratum.binary_mode);
    value
}

fn candidate_roster_id(stratum: &crate::CandidatePipelineStratum) -> String {
    format!(
        "candidate-roster/{}/{}/{}/{}/{}/n-{}",
        stratum.scope_id,
        stratum.scale_role.as_str(),
        stratum.key_domain.as_str(),
        stratum.workload_id.as_str(),
        stratum.graph_profile.as_str(),
        stratum.n
    )
}

fn find_candidate_binding<'a>(
    bindings: &'a [Value],
    candidate_id: &str,
    key_domain: crate::CandidateKeyDomain,
) -> Result<&'a Value, EvidenceError> {
    let mut matching = bindings.iter().filter(|binding| {
        binding["id"].as_str() == Some(candidate_id)
            && binding["keyDomain"].as_str() == Some(key_domain.as_str())
    });
    let binding = matching
        .next()
        .ok_or_else(|| EvidenceError::CandidateRegistryRecomputation {
            detail: format!("候选绑定缺少 {candidate_id}/{}", key_domain.as_str()),
        })?;
    if matching.next().is_some() {
        return Err(EvidenceError::CandidateRegistryRecomputation {
            detail: format!("候选绑定重复 {candidate_id}/{}", key_domain.as_str()),
        });
    }
    Ok(binding)
}

fn candidate_round_summaries(
    execution: &crate::CandidatePipelineExecution,
) -> Result<Vec<Value>, EvidenceError> {
    execution
        .samples
        .iter()
        .map(|sample| {
            let mut attempts = execution
                .attempts
                .iter()
                .filter(|attempt| attempt.run_id == sample.run_id);
            let attempt =
                attempts
                    .next()
                    .ok_or_else(|| EvidenceError::CandidateComparisonRecomputation {
                        detail: format!("候选有效样本 {} 缺少原始尝试", sample.run_id),
                    })?;
            if attempts.next().is_some() || attempt.status != crate::RunStatus::Valid {
                return Err(EvidenceError::CandidateComparisonRecomputation {
                    detail: format!("候选有效样本 {} 的原始尝试不唯一或无效", sample.run_id),
                });
            }
            let wall_time = sample.child.wall_time_ns.ok_or_else(|| {
                EvidenceError::CandidateComparisonRecomputation {
                    detail: format!("候选有效样本 {} 缺少墙钟", sample.run_id),
                }
            })?;
            Ok(json!({
                "summaryId": candidate_summary_id(&sample.run_id),
                "purpose": "candidate-comparison",
                "candidateId": sample.candidate_id,
                "stratum": candidate_comparison_stratum(&execution.stratum),
                "metric": "wall-time-ns",
                "batch": sample.batch,
                "round": sample.round,
                "roundAttemptId": attempt.round_attempt_id,
                "aggregationMethod": crate::FORMAL_LADDER_AGGREGATION_METHOD,
                "contributingRunIds": [sample.run_id.as_str()],
                "median": wall_time,
                "medianAbsoluteDeviation": 0
            }))
        })
        .collect()
}

fn candidate_summary_id(run_id: &str) -> String {
    format!("candidate-summary/{run_id}/wall-time-ns")
}

fn candidate_comparisons(
    execution: &crate::CandidatePipelineExecution,
    roster_id: &str,
    reproducibility_envelopes: &[Value],
) -> Result<Vec<Value>, EvidenceError> {
    let envelope = unique_wall_time_envelope(reproducibility_envelopes)?;
    let participant_count = execution.roster.participant_ids().len();
    execution
        .roster
        .entries
        .iter()
        .filter(|entry| entry.disposition == crate::CandidateDisposition::PerformanceParticipant)
        .map(|entry| {
            let (batch_zero, zero_ratio) =
                candidate_comparison_batch(execution, &entry.candidate_id, 0, participant_count)?;
            let (batch_one, one_ratio) =
                candidate_comparison_batch(execution, &entry.candidate_id, 1, participant_count)?;
            let decision = match (zero_ratio, one_ratio, envelope) {
                (Some(zero), Some(one), Some(envelope)) => {
                    classify_candidate_ratios([zero, one], envelope)?
                }
                _ => "insufficient-evidence",
            };
            Ok(json!({
                "candidateId": entry.candidate_id,
                "baselineId": execution.roster.baseline_id,
                "rosterId": roster_id,
                "stratum": candidate_comparison_stratum(&execution.stratum),
                "metric": "wall-time-ns",
                "batch0": batch_zero,
                "batch1": batch_one,
                "decision": decision
            }))
        })
        .collect()
}

fn candidate_comparison_batch(
    execution: &crate::CandidatePipelineExecution,
    candidate_id: &str,
    batch: u32,
    participant_count: usize,
) -> Result<(Value, Option<AssemblyRatio>), EvidenceError> {
    let mut pairs = Vec::new();
    let mut ratios = Vec::new();
    let mut rounds = BTreeSet::new();
    for scheduled in execution
        .schedule
        .iter()
        .filter(|round| round.batch == batch)
    {
        let baseline = unique_candidate_sample(
            execution,
            batch,
            scheduled.round,
            &execution.roster.baseline_id,
        )?;
        let candidate = unique_candidate_sample(execution, batch, scheduled.round, candidate_id)?;
        let (Some(baseline), Some(candidate)) = (baseline, candidate) else {
            continue;
        };
        let baseline_wall = baseline.child.wall_time_ns.ok_or_else(|| {
            EvidenceError::CandidateComparisonRecomputation {
                detail: format!("候选基线样本 {} 缺少墙钟", baseline.run_id),
            }
        })?;
        let candidate_wall = candidate.child.wall_time_ns.ok_or_else(|| {
            EvidenceError::CandidateComparisonRecomputation {
                detail: format!("候选样本 {} 缺少墙钟", candidate.run_id),
            }
        })?;
        let ratio = AssemblyRatio::new(candidate_wall, baseline_wall)?;
        rounds.insert(scheduled.round);
        ratios.push(ratio);
        pairs.push(json!({
            "round": scheduled.round,
            "baselineRoundSummaryId": candidate_summary_id(&baseline.run_id),
            "candidateRoundSummaryId": candidate_summary_id(&candidate.run_id),
            "ratio": observed_ratio(ratio)
        }));
    }
    let expected_round_count = participant_count.checked_mul(2).ok_or_else(|| {
        EvidenceError::CandidateComparisonRecomputation {
            detail: "候选轮次数溢出".to_owned(),
        }
    })?;
    let expected_rounds = (0..u32::try_from(expected_round_count).map_err(|_| {
        EvidenceError::CandidateComparisonRecomputation {
            detail: "候选轮次数超出 u32".to_owned(),
        }
    })?)
        .collect::<BTreeSet<_>>();
    let median = if ratios.len() == expected_round_count && rounds == expected_rounds {
        Some(exact_even_ratio_median(&ratios)?)
    } else {
        None
    };
    let median_json = median.map_or_else(
        || {
            null_observation(if pairs.is_empty() {
                "null-no-usable-round-pairs-v1"
            } else {
                "null-incomplete-balanced-round-set-v1"
            })
        },
        observed_ratio,
    );
    Ok((
        json!({
            "pairingMethod": "same-batch-same-round-v1",
            "aggregationMethod": "median-of-exact-round-ratios-v1",
            "roundPairs": pairs,
            "medianRatio": median_json
        }),
        median,
    ))
}

fn unique_candidate_sample<'a>(
    execution: &'a crate::CandidatePipelineExecution,
    batch: u32,
    round: u32,
    candidate_id: &str,
) -> Result<Option<&'a crate::CandidatePipelinePerformanceSample>, EvidenceError> {
    let mut matching = execution.samples.iter().filter(|sample| {
        sample.batch == batch && sample.round == round && sample.candidate_id == candidate_id
    });
    let sample = matching.next();
    if matching.next().is_some() {
        return Err(EvidenceError::CandidateComparisonRecomputation {
            detail: format!("候选样本 {batch}/{round}/{candidate_id} 重复"),
        });
    }
    Ok(sample)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AssemblyRatio {
    numerator: u64,
    denominator: u64,
}

impl AssemblyRatio {
    fn new(numerator: u64, denominator: u64) -> Result<Self, EvidenceError> {
        if numerator == 0 || denominator == 0 {
            return Err(candidate_ratio_error("候选比值必须严格大于零"));
        }
        let divisor = gcd_u64(numerator, denominator);
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }
}

fn observed_ratio(ratio: AssemblyRatio) -> Value {
    json!({
        "value": {"numerator": ratio.numerator, "denominator": ratio.denominator},
        "reason": null
    })
}

fn exact_even_ratio_median(ratios: &[AssemblyRatio]) -> Result<AssemblyRatio, EvidenceError> {
    if ratios.is_empty() || !ratios.len().is_multiple_of(2) {
        return Err(candidate_ratio_error("候选精确中位比值需要非空偶数项"));
    }
    let mut sorted = ratios.to_vec();
    sorted.sort_by(compare_assembly_ratios);
    let upper = sorted.len() / 2;
    let left = sorted[upper - 1];
    let right = sorted[upper];
    let numerator = u128::from(left.numerator)
        .checked_mul(u128::from(right.denominator))
        .and_then(|value| {
            u128::from(right.numerator)
                .checked_mul(u128::from(left.denominator))
                .and_then(|right_value| value.checked_add(right_value))
        })
        .ok_or_else(|| candidate_ratio_error("候选精确中位比值分子溢出"))?;
    let denominator = u128::from(left.denominator)
        .checked_mul(u128::from(right.denominator))
        .and_then(|value| value.checked_mul(2))
        .ok_or_else(|| candidate_ratio_error("候选精确中位比值分母溢出"))?;
    let divisor = gcd_u128(numerator, denominator);
    let numerator = u64::try_from(numerator / divisor)
        .map_err(|_| candidate_ratio_error("候选精确中位比值分子超出 u64"))?;
    let denominator = u64::try_from(denominator / divisor)
        .map_err(|_| candidate_ratio_error("候选精确中位比值分母超出 u64"))?;
    Ok(AssemblyRatio {
        numerator,
        denominator,
    })
}

fn compare_assembly_ratios(left: &AssemblyRatio, right: &AssemblyRatio) -> Ordering {
    (u128::from(left.numerator) * u128::from(right.denominator))
        .cmp(&(u128::from(right.numerator) * u128::from(left.denominator)))
}

fn unique_wall_time_envelope(envelopes: &[Value]) -> Result<Option<AssemblyRatio>, EvidenceError> {
    let mut matching = envelopes
        .iter()
        .filter(|envelope| envelope["metric"] == "wall-time-ns");
    let Some(envelope) = matching.next() else {
        return Ok(None);
    };
    if matching.next().is_some() {
        return Err(candidate_ratio_error("墙钟重复性包络不唯一"));
    }
    Ok(Some(AssemblyRatio::new(
        envelope["repeatRatio"]["numerator"]
            .as_u64()
            .ok_or_else(|| candidate_ratio_error("墙钟重复性包络分子缺失"))?,
        envelope["repeatRatio"]["denominator"]
            .as_u64()
            .ok_or_else(|| candidate_ratio_error("墙钟重复性包络分母缺失"))?,
    )?))
}

fn classify_candidate_ratios(
    ratios: [AssemblyRatio; 2],
    envelope: AssemblyRatio,
) -> Result<&'static str, EvidenceError> {
    let mut improvement = true;
    let mut regression = true;
    let mut noise = true;
    for ratio in ratios {
        let improvement_left = checked_ratio_product(ratio.numerator, envelope.numerator)?;
        let improvement_right = checked_ratio_product(ratio.denominator, envelope.denominator)?;
        improvement &= improvement_left < improvement_right;
        let regression_left = checked_ratio_product(ratio.numerator, envelope.denominator)?;
        let regression_right = checked_ratio_product(ratio.denominator, envelope.numerator)?;
        regression &= regression_left > regression_right;
        noise &= improvement_left >= improvement_right && regression_left <= regression_right;
    }
    Ok(if improvement {
        "repeatable-improvement"
    } else if regression {
        "repeatable-regression"
    } else if noise {
        "noise-no-difference"
    } else {
        "insufficient-evidence"
    })
}

fn checked_ratio_product(left: u64, right: u64) -> Result<u128, EvidenceError> {
    u128::from(left)
        .checked_mul(u128::from(right))
        .ok_or_else(|| candidate_ratio_error("候选分类精确乘法溢出"))
}

fn gcd_u128(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn candidate_ratio_error(detail: &str) -> EvidenceError {
    EvidenceError::CandidateComparisonRecomputation {
        detail: detail.to_owned(),
    }
}

#[derive(Default)]
struct FormalDerivedSummaries {
    round_summaries: Vec<Value>,
    batch_summaries: Vec<Value>,
    adjacent_ratios: Vec<Value>,
    knees: Vec<Value>,
    reproducibility_envelopes: Vec<Value>,
    growth_slopes: Vec<Value>,
    recommendations: Vec<Value>,
}

fn formal_derived_summaries(
    checkpoint: &FormalProtocolCheckpoint,
) -> Result<FormalDerivedSummaries, EvidenceError> {
    let mut output = FormalDerivedSummaries::default();
    for ladder in &checkpoint.formal_ladders {
        let Some(analysis) = &ladder.analysis else {
            continue;
        };
        for summary in &analysis.round_summaries {
            let process_run = find_formal_process_run(ladder, summary)?;
            let contributing_run_ids = match summary.sample_kind {
                crate::FormalLadderSampleKind::ColdInstance => {
                    vec![format!("{}/cold-instance-0", process_run.run_id)]
                }
                crate::FormalLadderSampleKind::StableCapacityReuse => process_run
                    .child
                    .as_ref()
                    .ok_or_else(|| EvidenceError::FormalRecomputation {
                        detail: format!("正式汇总 {} 缺少结构化子进程报告", summary.summary_id),
                    })?
                    .stable_capacity_reuse
                    .iter()
                    .map(|sample| {
                        format!(
                            "{}/stable-capacity-reuse-{}",
                            process_run.run_id, sample.sample_ordinal
                        )
                    })
                    .collect(),
            };
            output.round_summaries.push(json!({
                "summaryId": summary.summary_id,
                "purpose": "formal-ladder",
                "candidateId": ladder.candidate_id,
                "stratum": formal_stratum(
                    ladder,
                    summary.n,
                    summary.sample_kind,
                    summary.binary_mode,
                ),
                "metric": formal_metric(summary.metric),
                "batch": summary.batch,
                "round": summary.round,
                "roundAttemptId": process_run.attempt_id,
                "aggregationMethod": crate::FORMAL_LADDER_AGGREGATION_METHOD,
                "contributingRunIds": contributing_run_ids,
                "median": summary.median,
                "medianAbsoluteDeviation": summary.median_absolute_deviation
            }));
        }
        for summary in &analysis.batch_summaries {
            output.batch_summaries.push(json!({
                "summaryId": summary.summary_id,
                "candidateId": ladder.candidate_id,
                "stratum": formal_stratum(
                    ladder,
                    summary.n,
                    summary.sample_kind,
                    summary.binary_mode,
                ),
                "metric": formal_metric(summary.metric),
                "batch": summary.batch,
                "aggregationMethod": summary.aggregation_method,
                "roundSummaryIds": summary.contributing_round_summary_ids,
                "median": summary.median,
                "medianAbsoluteDeviation": summary.median_absolute_deviation
            }));
        }
        for adjacent in &analysis.adjacent_level_ratios {
            let metric = formal_metric(adjacent.metric);
            let binary_mode = binary_mode_for_metric(adjacent.metric);
            let sample_kind = adjacent.sample_kind;
            let round_pairs = adjacent
                .round_ratios
                .iter()
                .enumerate()
                .map(|(round, ratio)| {
                    let lower = find_round_summary_id(
                        analysis,
                        adjacent.lower_n,
                        adjacent.batch,
                        u32::try_from(round).expect("five rounds fit u32"),
                        adjacent.metric,
                        sample_kind,
                    )?;
                    let upper = find_round_summary_id(
                        analysis,
                        adjacent.upper_n,
                        adjacent.batch,
                        u32::try_from(round).expect("five rounds fit u32"),
                        adjacent.metric,
                        sample_kind,
                    )?;
                    Ok(json!({
                        "round": round,
                        "lowerRoundSummaryId": lower,
                        "upperRoundSummaryId": upper,
                        "ratio": ratio_json(ratio)?
                    }))
                })
                .collect::<Result<Vec<_>, EvidenceError>>()?;
            output.adjacent_ratios.push(json!({
                "candidateId": ladder.candidate_id,
                "lowerStratum": formal_stratum(ladder, adjacent.lower_n, sample_kind, binary_mode),
                "upperStratum": formal_stratum(ladder, adjacent.upper_n, sample_kind, binary_mode),
                "metric": metric,
                "batch": adjacent.batch,
                "lowerLadderBatchSummaryId": adjacent.lower_batch_summary_id,
                "upperLadderBatchSummaryId": adjacent.upper_batch_summary_id,
                "pairingMethod": "same-batch-same-round-adjacent-level-v1",
                "normalizationBasis": normalization_basis(adjacent.metric),
                "roundPairs": round_pairs,
                "aggregationMethod": "median-of-five-exact-round-ratios-v1",
                "medianRatio": ratio_json(&adjacent.median_ratio)?,
                "candidateKnee": adjacent.candidate_knee
            }));
        }
        for knee in &analysis.knees {
            let batch_zero = find_adjacent_ratio(analysis, knee, 0)?;
            let batch_one = find_adjacent_ratio(analysis, knee, 1)?;
            let binary_mode = binary_mode_for_metric(knee.metric);
            let profiler_artifact = if knee.batch_zero_candidate {
                null_observation("candidate-knee-unattributed")
            } else {
                null_observation("not-a-candidate-knee")
            };
            output.knees.push(json!({
                "candidateId": ladder.candidate_id,
                "metric": formal_metric(knee.metric),
                "lowerStratum": formal_stratum(ladder, knee.lower_n, knee.sample_kind, binary_mode),
                "upperStratum": formal_stratum(ladder, knee.upper_n, knee.sample_kind, binary_mode),
                "candidateBatch0Ratio": {
                    "batch": 0,
                    "lowerLadderBatchSummaryId": batch_zero.lower_batch_summary_id,
                    "upperLadderBatchSummaryId": batch_zero.upper_batch_summary_id
                },
                "confirmationBatch1Ratio": {
                    "batch": 1,
                    "lowerLadderBatchSummaryId": batch_one.lower_batch_summary_id,
                    "upperLadderBatchSummaryId": batch_one.upper_batch_summary_id
                },
                "candidateKnee": knee.batch_zero_candidate,
                "confirmedKnee": knee.confirmed_knee,
                "profilerArtifactSha256": profiler_artifact
            }));
        }
        output
            .growth_slopes
            .extend(formal_growth_slopes(ladder, analysis)?);
    }
    (output.reproducibility_envelopes, output.recommendations) =
        reproducibility_and_recommendations(
            &output.round_summaries,
            &output.batch_summaries,
            checkpoint.base_scale_pilot.clock_quantum_ns,
        )?;
    Ok(output)
}

fn formal_growth_slopes(
    ladder: &crate::FormalLadderExecution,
    analysis: &crate::FormalLadderAnalysis,
) -> Result<Vec<Value>, EvidenceError> {
    let mut output = Vec::new();
    for metric in [
        crate::FormalLadderMetric::WallTimeNs,
        crate::FormalLadderMetric::PeakLiveRequestedBytes,
    ] {
        for sample_kind in [
            crate::FormalLadderSampleKind::ColdInstance,
            crate::FormalLadderSampleKind::StableCapacityReuse,
        ] {
            let pre_knee_max_n = analysis
                .knees
                .iter()
                .filter(|knee| {
                    knee.metric == metric && knee.sample_kind == sample_kind && knee.confirmed_knee
                })
                .map(|knee| knee.lower_n)
                .min();
            let mut batches = [Vec::new(), Vec::new()];
            for summary in analysis.batch_summaries.iter().filter(|summary| {
                summary.metric == metric
                    && summary.sample_kind == sample_kind
                    && pre_knee_max_n.is_none_or(|maximum| summary.n <= maximum)
            }) {
                let batch = usize::try_from(summary.batch).map_err(|_| {
                    EvidenceError::FormalRecomputation {
                        detail: "增长斜率 batch 超出 usize".to_owned(),
                    }
                })?;
                if batch > 1 {
                    return Err(EvidenceError::FormalRecomputation {
                        detail: format!("增长斜率包含非法 batch {}", summary.batch),
                    });
                }
                batches[batch].push(summary);
            }
            for batch in &mut batches {
                batch.sort_by_key(|summary| summary.normalizer);
            }
            if batches.iter().any(|batch| batch.len() < 3) {
                continue;
            }
            let batch_zero_ns = batches[0]
                .iter()
                .map(|summary| summary.n)
                .collect::<Vec<_>>();
            let batch_one_ns = batches[1]
                .iter()
                .map(|summary| summary.n)
                .collect::<Vec<_>>();
            if batch_zero_ns != batch_one_ns {
                return Err(EvidenceError::FormalRecomputation {
                    detail: format!(
                        "{}/{}/{}/{} 的增长斜率双批次级别集合不一致",
                        ladder.workload_id.as_str(),
                        ladder.graph_profile,
                        formal_metric(metric),
                        formal_sample_kind(sample_kind)
                    ),
                });
            }
            let (batch_zero, slope_zero) = growth_slope_batch_json(&batches[0])?;
            let (batch_one, slope_one) = growth_slope_batch_json(&batches[1])?;
            let mut series = formal_stratum(
                ladder,
                batch_zero_ns[0],
                sample_kind,
                binary_mode_for_metric(metric),
            );
            let series = series.as_object_mut().expect("formal stratum is an object");
            series.remove("n");
            series.remove("scaleRole");
            output.push(json!({
                "candidateId": crate::BASELINE_CANDIDATE_ID,
                "series": series,
                "metric": formal_metric(metric),
                "encoding": "theil-sen-q16.16-nearest-ties-even-v1",
                "batch0": batch_zero,
                "batch1": batch_one,
                "upperBoundFormula": "max-plus-absolute-difference-v1",
                "suggestedUpperSlope": signed_growth_ratio_json(
                    growth_upper_slope_bound(slope_zero, slope_one)?
                )
            }));
        }
    }
    Ok(output)
}

fn growth_slope_batch_json(
    summaries: &[&crate::FormalLadderBatchSummary],
) -> Result<(Value, SignedGrowthRatio), EvidenceError> {
    let mut pairwise = Vec::new();
    let mut slopes = Vec::new();
    for lower_index in 0..summaries.len() {
        for upper_index in (lower_index + 1)..summaries.len() {
            let lower = summaries[lower_index];
            let upper = summaries[upper_index];
            let slope = growth_slope_q16_16(
                lower.normalizer,
                lower.median,
                upper.normalizer,
                upper.median,
            )?;
            pairwise.push(json!({
                "lowerBatchSummaryId": lower.summary_id,
                "upperBatchSummaryId": upper.summary_id,
                "slopeQ16_16": slope
            }));
            slopes.push(slope);
        }
    }
    let theil_sen = growth_median_signed_slopes(&slopes)?;
    Ok((
        json!({
            "levelBatchSummaryIds": summaries
                .iter()
                .map(|summary| summary.summary_id.as_str())
                .collect::<Vec<_>>(),
            "pairwiseSlopes": pairwise,
            "theilSenSlope": signed_growth_ratio_json(theil_sen)
        }),
        theil_sen,
    ))
}

fn growth_slope_q16_16(
    lower_x: u64,
    lower_y: u64,
    upper_x: u64,
    upper_y: u64,
) -> Result<i32, EvidenceError> {
    use num_bigint::BigUint;

    if lower_x == 0 || lower_y == 0 || upper_x <= lower_x || upper_y == 0 {
        return Err(EvidenceError::FormalRecomputation {
            detail: "增长斜率点必须满足 0 < x_l < x_u 且 y_l,y_u > 0".to_owned(),
        });
    }
    if lower_y == upper_y {
        return Ok(0);
    }
    let (a, b, sign) = if upper_y > lower_y {
        (upper_y, lower_y, 1_i64)
    } else {
        (lower_y, upper_y, -1_i64)
    };
    const FRACTIONAL_DENOMINATOR: u32 = 65_536;
    let a_pow = BigUint::from(a).pow(FRACTIONAL_DENOMINATOR);
    let b_pow = BigUint::from(b).pow(FRACTIONAL_DENOMINATOR);
    let lower_x = BigUint::from(lower_x);
    let upper_x = BigUint::from(upper_x);
    let at_least = |k: u32| &a_pow * lower_x.pow(k) >= &b_pow * upper_x.pow(k);
    let mut high = 1_u32;
    while at_least(high) {
        high = high.checked_mul(2).ok_or_else(growth_slope_overflow)?;
        if high > i32::MAX as u32 {
            return Err(growth_slope_overflow());
        }
    }
    let mut low = high / 2;
    while low + 1 < high {
        let middle = low + (high - low) / 2;
        if at_least(middle) {
            low = middle;
        } else {
            high = middle;
        }
    }
    let midpoint_exponent = low
        .checked_mul(2)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(growth_slope_overflow)?;
    let left = BigUint::from(a).pow(FRACTIONAL_DENOMINATOR * 2) * lower_x.pow(midpoint_exponent);
    let right = BigUint::from(b).pow(FRACTIONAL_DENOMINATOR * 2) * upper_x.pow(midpoint_exponent);
    let rounded = match left.cmp(&right) {
        Ordering::Less => low,
        Ordering::Greater => low + 1,
        Ordering::Equal if low.is_multiple_of(2) => low,
        Ordering::Equal => low + 1,
    };
    sign.checked_mul(i64::from(rounded))
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(growth_slope_overflow)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SignedGrowthRatio {
    numerator: i64,
    denominator: i64,
}

fn growth_median_signed_slopes(slopes: &[i32]) -> Result<SignedGrowthRatio, EvidenceError> {
    if slopes.is_empty() {
        return Err(EvidenceError::FormalRecomputation {
            detail: "不能对空斜率集合计算泰尔－森中位数".to_owned(),
        });
    }
    let mut sorted = slopes.to_vec();
    sorted.sort_unstable();
    if sorted.len() % 2 == 1 {
        Ok(SignedGrowthRatio {
            numerator: i64::from(sorted[sorted.len() / 2]),
            denominator: 1,
        })
    } else {
        reduce_growth_signed_ratio(
            i64::from(sorted[sorted.len() / 2 - 1]) + i64::from(sorted[sorted.len() / 2]),
            2,
        )
    }
}

fn growth_upper_slope_bound(
    left: SignedGrowthRatio,
    right: SignedGrowthRatio,
) -> Result<SignedGrowthRatio, EvidenceError> {
    let left_twice = i128::from(left.numerator)
        .checked_mul(2)
        .and_then(|value| value.checked_div(i128::from(left.denominator)))
        .ok_or_else(growth_slope_overflow)?;
    let right_twice = i128::from(right.numerator)
        .checked_mul(2)
        .and_then(|value| value.checked_div(i128::from(right.denominator)))
        .ok_or_else(growth_slope_overflow)?;
    let upper_twice = left_twice
        .max(right_twice)
        .checked_add((left_twice - right_twice).abs())
        .and_then(|value| i64::try_from(value).ok())
        .ok_or_else(growth_slope_overflow)?;
    reduce_growth_signed_ratio(upper_twice, 2)
}

fn reduce_growth_signed_ratio(
    numerator: i64,
    denominator: i64,
) -> Result<SignedGrowthRatio, EvidenceError> {
    if denominator <= 0 {
        return Err(growth_slope_overflow());
    }
    let divisor = growth_gcd(u128::from(numerator.unsigned_abs()), denominator as u128) as i64;
    Ok(SignedGrowthRatio {
        numerator: numerator / divisor,
        denominator: denominator / divisor,
    })
}

fn signed_growth_ratio_json(ratio: SignedGrowthRatio) -> Value {
    json!({
        "numerator": ratio.numerator,
        "denominator": ratio.denominator,
        "fractionalBits": 16
    })
}

fn growth_gcd(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn growth_slope_overflow() -> EvidenceError {
    EvidenceError::FormalRecomputation {
        detail: "增长斜率有理数算术溢出".to_owned(),
    }
}

fn reproducibility_and_recommendations(
    round_summaries: &[Value],
    batch_summaries: &[Value],
    clock_quantum_ns: u64,
) -> Result<(Vec<Value>, Vec<Value>), EvidenceError> {
    let round_by_id = round_summaries
        .iter()
        .map(|summary| {
            (
                summary["summaryId"].as_str().expect("built summary id"),
                summary,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut pairs = BTreeMap::<String, [Option<&Value>; 2]>::new();
    for summary in batch_summaries {
        let key = batch_pair_key(summary);
        let batch = usize::try_from(summary["batch"].as_u64().expect("built batch"))
            .expect("batch fits usize");
        if batch > 1
            || pairs.entry(key.clone()).or_default()[batch]
                .replace(summary)
                .is_some()
        {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("正式批次汇总 {key} 出现重复或非法 batch"),
            });
        }
    }
    let mut maxima = BTreeMap::<String, (&Value, &Value, (u64, u64))>::new();
    let mut recommendations = Vec::new();
    for (key, pair) in pairs {
        let [Some(batch_zero), Some(batch_one)] = pair else {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("正式批次汇总 {key} 缺少双批次配对"),
            });
        };
        let metric = batch_zero["metric"].as_str().expect("built metric");
        let zero_median = batch_zero["median"].as_u64().expect("built median");
        let one_median = batch_one["median"].as_u64().expect("built median");
        let ratio = reduced_ratio(zero_median.max(one_median), zero_median.min(one_median))?;
        if maxima.get(metric).is_none_or(|(_, _, current)| {
            u128::from(ratio.0) * u128::from(current.1)
                > u128::from(current.0) * u128::from(ratio.1)
        }) {
            maxima.insert(metric.to_owned(), (batch_zero, batch_one, ratio));
        }
        let observed_upper = batch_zero["roundSummaryIds"]
            .as_array()
            .expect("built round ids")
            .iter()
            .chain(
                batch_one["roundSummaryIds"]
                    .as_array()
                    .expect("built round ids"),
            )
            .map(|id| {
                let id = id.as_str().expect("built round id");
                round_by_id
                    .get(id)
                    .map(|summary| summary["median"].as_u64().expect("built median"))
                    .ok_or_else(|| EvidenceError::FormalRecomputation {
                        detail: format!("预算建议缺少轮次汇总 {id}"),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .max()
            .ok_or_else(|| EvidenceError::FormalRecomputation {
                detail: format!("预算建议 {key} 没有轮次中位数"),
            })?;
        recommendations.push((
            metric.to_owned(),
            batch_zero.clone(),
            batch_one.clone(),
            observed_upper,
        ));
    }
    let envelopes = maxima
        .iter()
        .map(|(metric, (batch_zero, batch_one, ratio))| {
            json!({
                "candidateId": crate::BASELINE_CANDIDATE_ID,
                "metric": metric,
                "aggregationScope": "all-completed-non-guard-baseline-ladder-strata-v1",
                "maximizingBatch0LadderBatchSummaryId": batch_zero["summaryId"],
                "maximizingBatch1LadderBatchSummaryId": batch_one["summaryId"],
                "repeatRatio": {"numerator": ratio.0, "denominator": ratio.1}
            })
        })
        .collect::<Vec<_>>();
    let recommendations = recommendations
        .into_iter()
        .map(|(metric, batch_zero, batch_one, observed_upper)| {
            let (_, _, ratio) = maxima[&metric];
            let quantum = if metric == "wall-time-ns" {
                clock_quantum_ns
            } else {
                1
            };
            let value = ceil_ratio_to_quantum(observed_upper, ratio, quantum)?;
            Ok(json!({
                "recommendationKind": "r0-budget-v1",
                "candidateId": crate::BASELINE_CANDIDATE_ID,
                "stratum": batch_zero["stratum"],
                "metric": metric,
                "formula": "ceil-div-observed-upper-times-envelope-to-quantum-v1",
                "batch0LadderBatchSummaryId": batch_zero["summaryId"],
                "batch1LadderBatchSummaryId": batch_one["summaryId"],
                "reproducibilityEnvelopeMetric": metric,
                "observedUpper": observed_upper,
                "roundingRule": if metric == "wall-time-ns" { "ceil-to-protocol-clock-quantum-v1" } else { "ceil-to-whole-byte-v1" },
                "roundingQuantum": quantum,
                "value": value,
                "unit": if metric == "wall-time-ns" { "nanosecond" } else { "byte" },
                "scope": "R0 research input for #292; not product SLA"
            }))
        })
        .collect::<Result<Vec<_>, EvidenceError>>()?;
    Ok((envelopes, recommendations))
}

fn batch_pair_key(summary: &Value) -> String {
    let stratum = &summary["stratum"];
    format!(
        "{}/{}/{}/{}/{}/{}/{}/{}/{}",
        summary["candidateId"].as_str().expect("built candidate"),
        summary["metric"].as_str().expect("built metric"),
        stratum["workloadId"].as_str().expect("built workload"),
        stratum["graphProfile"].as_str().expect("built graph"),
        stratum["n"].as_u64().expect("built n"),
        stratum["sampleKind"].as_str().expect("built sample kind"),
        stratum["binaryMode"].as_str().expect("built binary mode"),
        stratum["scaleRole"].as_str().expect("built role"),
        stratum["keyDomain"].as_str().expect("built key domain"),
    )
}

fn reduced_ratio(numerator: u64, denominator: u64) -> Result<(u64, u64), EvidenceError> {
    if numerator == 0 || denominator == 0 {
        return Err(EvidenceError::FormalRecomputation {
            detail: "重复性比值包含零中位数".to_owned(),
        });
    }
    let divisor = gcd_u64(numerator, denominator);
    Ok((numerator / divisor, denominator / divisor))
}

fn gcd_u64(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

fn ceil_ratio_to_quantum(
    observed_upper: u64,
    ratio: (u64, u64),
    quantum: u64,
) -> Result<u64, EvidenceError> {
    let numerator = u128::from(observed_upper)
        .checked_mul(u128::from(ratio.0))
        .ok_or_else(|| EvidenceError::FormalRecomputation {
            detail: "预算取整分子溢出".to_owned(),
        })?;
    let denominator = u128::from(ratio.1)
        .checked_mul(u128::from(quantum))
        .ok_or_else(|| EvidenceError::FormalRecomputation {
            detail: "预算取整分母溢出".to_owned(),
        })?;
    numerator
        .checked_add(denominator - 1)
        .map(|value| value / denominator)
        .and_then(|quanta| quanta.checked_mul(u128::from(quantum)))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| EvidenceError::FormalRecomputation {
            detail: "预算取整结果溢出".to_owned(),
        })
}

fn find_formal_process_run<'a>(
    ladder: &'a crate::FormalLadderExecution,
    summary: &crate::FormalLadderRoundMetricSummary,
) -> Result<&'a crate::FormalLadderProcessRun, EvidenceError> {
    let level = ladder
        .levels
        .iter()
        .find(|level| level.n == summary.n)
        .ok_or_else(|| EvidenceError::FormalRecomputation {
            detail: format!("正式汇总 {} 引用了未知 N={}", summary.summary_id, summary.n),
        })?;
    let mut matching = level.formal_runs.iter().filter(|run| {
        run.status == crate::RunStatus::Valid
            && run.batch == summary.batch
            && run.round == summary.round
            && run.binary_mode == summary.binary_mode
            && run.child.is_some()
    });
    let run = matching
        .next()
        .ok_or_else(|| EvidenceError::FormalRecomputation {
            detail: format!("正式汇总 {} 找不到唯一有效子进程", summary.summary_id),
        })?;
    if matching.next().is_some() {
        return Err(EvidenceError::FormalRecomputation {
            detail: format!("正式汇总 {} 匹配多个有效子进程", summary.summary_id),
        });
    }
    Ok(run)
}

fn formal_stratum(
    ladder: &crate::FormalLadderExecution,
    n: u32,
    sample_kind: crate::FormalLadderSampleKind,
    binary_mode: crate::ScalableLadderBinaryMode,
) -> Value {
    json!({
        "keyDomain": "full-pipeline-baseline",
        "workloadId": ladder.workload_id,
        "workloadRevision": ladder.workload_revision,
        "graphProfile": ladder.graph_profile,
        "stringProfile": ladder.string_profile,
        "generatorVersion": ladder.generator_version,
        "n": n,
        "b": observed(u64::from(ladder.b)),
        "scaleRole": formal_scale_role(ladder, n),
        "caseId": "not-applicable",
        "sampleKind": formal_sample_kind(sample_kind),
        "binaryMode": formal_binary_mode(binary_mode)
    })
}

fn formal_metric(metric: crate::FormalLadderMetric) -> &'static str {
    match metric {
        crate::FormalLadderMetric::WallTimeNs => "wall-time-ns",
        crate::FormalLadderMetric::PeakLiveRequestedBytes => "peak-live-requested-bytes",
    }
}

fn formal_sample_kind(kind: crate::FormalLadderSampleKind) -> &'static str {
    match kind {
        crate::FormalLadderSampleKind::ColdInstance => "cold-instance",
        crate::FormalLadderSampleKind::StableCapacityReuse => "stable-capacity-reuse",
    }
}

fn formal_binary_mode(mode: crate::ScalableLadderBinaryMode) -> &'static str {
    match mode {
        crate::ScalableLadderBinaryMode::Timing => "timing",
        crate::ScalableLadderBinaryMode::Attribution => "memory",
    }
}

fn binary_mode_for_metric(metric: crate::FormalLadderMetric) -> crate::ScalableLadderBinaryMode {
    match metric {
        crate::FormalLadderMetric::WallTimeNs => crate::ScalableLadderBinaryMode::Timing,
        crate::FormalLadderMetric::PeakLiveRequestedBytes => {
            crate::ScalableLadderBinaryMode::Attribution
        }
    }
}

fn normalization_basis(metric: crate::FormalLadderMetric) -> &'static str {
    match metric {
        crate::FormalLadderMetric::WallTimeNs => "primary-record-count",
        crate::FormalLadderMetric::PeakLiveRequestedBytes => {
            "canonical-lir-shape-output-record-count"
        }
    }
}

fn ratio_json(ratio: &crate::ExactPositiveRatio) -> Result<Value, EvidenceError> {
    let numerator =
        ratio
            .numerator
            .parse::<u64>()
            .map_err(|error| EvidenceError::FormalRecomputation {
                detail: format!("比值分子 {} 不是 u64：{error}", ratio.numerator),
            })?;
    let denominator =
        ratio
            .denominator
            .parse::<u64>()
            .map_err(|error| EvidenceError::FormalRecomputation {
                detail: format!("比值分母 {} 不是 u64：{error}", ratio.denominator),
            })?;
    Ok(json!({"numerator": numerator, "denominator": denominator}))
}

fn find_round_summary_id(
    analysis: &crate::FormalLadderAnalysis,
    n: u32,
    batch: u32,
    round: u32,
    metric: crate::FormalLadderMetric,
    sample_kind: crate::FormalLadderSampleKind,
) -> Result<&str, EvidenceError> {
    analysis
        .round_summaries
        .iter()
        .find(|summary| {
            summary.n == n
                && summary.batch == batch
                && summary.round == round
                && summary.metric == metric
                && summary.sample_kind == sample_kind
        })
        .map(|summary| summary.summary_id.as_str())
        .ok_or_else(|| EvidenceError::FormalRecomputation {
            detail: format!("相邻比值缺少 N={n}/batch={batch}/round={round} 轮次汇总"),
        })
}

fn find_adjacent_ratio<'a>(
    analysis: &'a crate::FormalLadderAnalysis,
    knee: &crate::FormalKneeAssessment,
    batch: u32,
) -> Result<&'a crate::FormalAdjacentLevelRatio, EvidenceError> {
    analysis
        .adjacent_level_ratios
        .iter()
        .find(|ratio| {
            ratio.lower_n == knee.lower_n
                && ratio.upper_n == knee.upper_n
                && ratio.batch == batch
                && ratio.metric == knee.metric
                && ratio.sample_kind == knee.sample_kind
        })
        .ok_or_else(|| EvidenceError::FormalRecomputation {
            detail: format!(
                "拐点评估缺少 {}->{} batch {batch} 比值",
                knee.lower_n, knee.upper_n
            ),
        })
}

fn base_scale_summaries(checkpoint: &FormalProtocolCheckpoint) -> Vec<Value> {
    checkpoint
        .base_scale_pilot
        .selections
        .iter()
        .map(|selection| {
            json!({
                "candidateId": selection.candidate_id,
                "workloadId": selection.workload_id,
                "workloadRevision": selection.workload_revision,
                "graphProfile": selection.graph_profile,
                "stringProfile": selection.string_profile,
                "generatorVersion": selection.generator_version,
                "selectionRule": selection.selection_rule,
                "pilotLevels": selection.pilot_levels.iter().map(|level| json!({
                    "n": level.n,
                    "contributingRunIds": level.contributing_run_ids,
                    "aggregationMethod": level.aggregation_method,
                    "wallTimeMedianNs": level.wall_time_median_ns,
                    "wallTimeMedianAbsoluteDeviationNs": level.wall_time_median_absolute_deviation_ns,
                    "minimumReliableWallTimeNs": level.minimum_reliable_wall_time_ns,
                    "semanticDigest": level.semantic_digest,
                    "allSemanticDigestsEqual": level.all_semantic_digests_equal,
                    "allGuardsClear": level.all_guards_clear,
                    "qualifies": level.qualifies
                })).collect::<Vec<_>>(),
                "b": selection.b,
                "terminalGuardRunId": selection.terminal_guard_run_id
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_environment() -> FormalEnvironmentSnapshot {
        FormalEnvironmentSnapshot {
            os: "test-os".to_owned(),
            os_build: "test-build".to_owned(),
            cpu: "test-cpu".to_owned(),
            logical_processor_count: 8,
            physical_memory_bytes: 64 * 1_073_741_824,
            target_triple: "x86_64-pc-windows-msvc".to_owned(),
            rustc: "rustc-test".to_owned(),
            llvm: "llvm-test".to_owned(),
            power_source: "ac".to_owned(),
            vendor_performance_mode: "performance".to_owned(),
            power_plan: "performance".to_owned(),
            bios_firmware: "test-bios".to_owned(),
            monitoring_provider: "test-monitor".to_owned(),
            background_process_audit: Vec::new(),
            operator_declaration: crate::FormalEnvironmentDeclaration {
                vendor_performance_mode: "performance".to_owned(),
                bios_firmware: "test-bios".to_owned(),
                sleep_or_session_lock_observed: false,
                thermal_or_power_throttling_observed: false,
            },
        }
    }

    fn successful_process(binary_id: &str) -> crate::ProcessObservation {
        crate::ProcessObservation {
            coordinator_pid: 1,
            child_pid: crate::NullableObservation::observed(2),
            binary_id: binary_id.to_owned(),
            exit_kind: crate::ProcessExitKind::Success,
            exit_code: crate::NullableObservation::observed(0),
            termination: crate::TerminationObservation {
                kind: crate::TerminationKind::ExitCode,
                signal_number: crate::NullableObservation::unavailable("not-signal-termination"),
                raw_platform_status: crate::NullableObservation::unavailable(
                    "exit-code-is-authoritative",
                ),
            },
        }
    }

    fn test_monitor(
        trigger: Option<crate::ChildMonitorTrigger>,
    ) -> crate::ChildProcessMonitorReport {
        crate::ChildProcessMonitorReport {
            observation_count: 1,
            last_private_bytes: crate::NullableObservation::observed(2_048),
            peak_private_bytes: crate::NullableObservation::observed(4_096),
            last_available_physical_memory_bytes: crate::NullableObservation::observed(
                48 * 1_073_741_824,
            ),
            elapsed_wall_time_ns: 13_000,
            trigger,
        }
    }

    #[test]
    fn monitor_trigger_tokens_match_the_evidence_schema() {
        assert_eq!(monitor_trigger(None), "none");
        assert_eq!(
            monitor_trigger(Some(crate::ChildMonitorTrigger::PrivateBytes)),
            "private-bytes-monitor"
        );
        assert_eq!(
            monitor_trigger(Some(crate::ChildMonitorTrigger::WallTime)),
            "wall-time-monitor"
        );
        assert_eq!(
            monitor_trigger(Some(crate::ChildMonitorTrigger::MonitoringGap)),
            "monitoring-gap"
        );
    }

    #[test]
    fn pilot_projection_uses_only_checkpoint_observations_and_exact_plan_counts() {
        let trusted = crate::load_repository_contract().expect("trusted contract");
        let environment = test_environment();
        let guard = crate::ScalableGuardPlanner::from_trusted_contract(&trusted)
            .expect("guard planner")
            .evaluate_pilot(
                crate::ScalableWorkloadId::Identity,
                crate::GraphProfileId::WideStar,
                1,
                crate::SystemMemoryObservation {
                    physical_memory_bytes: environment.physical_memory_bytes,
                    available_physical_memory_bytes: 48 * 1_073_741_824,
                },
                None,
            )
            .expect("clear pilot guard");
        assert!(guard.allows_child_start);
        let run_id = "pilot/LF-COMP-ID-v1/wide-star-v1/n-1/attempt-0/pilot-sample-0";
        let compiler_instance_id = format!("{run_id}/compiler-instance");
        let digest = "1".repeat(64);
        let external_state = crate::ExternalStateObservation {
            power_source: environment.power_source.clone(),
            vendor_performance_mode: environment.vendor_performance_mode.clone(),
            power_plan: environment.power_plan.clone(),
            sleep_or_session_lock: false,
            thermal_or_power_throttling: false,
            background_cpu_time_ns: crate::NullableObservation::observed(0),
            background_write_bytes: crate::NullableObservation::observed(0),
            monitoring_gap: false,
            background_process_deltas: Vec::new(),
        };
        let checkpoint = crate::BaseScalePilotCheckpoint {
            schema: crate::BASE_SCALE_PILOT_CHECKPOINT_SCHEMA.to_owned(),
            schema_version: crate::BASE_SCALE_PILOT_CHECKPOINT_SCHEMA_VERSION,
            protocol_id: crate::FORMAL_PROTOCOL_ID.to_owned(),
            clock_quantum_ns: 1,
            required_median_wall_time_ns: 10_000,
            selections: vec![crate::BaseScaleSelection {
                candidate_id: crate::BASELINE_CANDIDATE_ID.to_owned(),
                workload_id: crate::ScalableWorkloadId::Identity,
                workload_revision: crate::WORKLOAD_REVISION_V1,
                graph_profile: crate::GraphProfileId::WideStar.as_str().to_owned(),
                string_profile: crate::BASE_SCALE_STRING_PROFILE.to_owned(),
                generator_version: crate::GENERATOR_VERSION_V1,
                selection_rule: crate::BASE_SCALE_SELECTION_RULE.to_owned(),
                pilot_levels: Vec::new(),
                b: crate::NullableObservation::unavailable("base-scale-not-yet-selected"),
                terminal_guard_run_id: crate::NullableObservation::unavailable(
                    "base-scale-not-yet-selected",
                ),
            }],
            active_selection: None,
            runs: vec![crate::BaseScalePilotRun {
                run_id: run_id.to_owned(),
                attempt_id: "pilot/LF-COMP-ID-v1/wide-star-v1/n-1/attempt-0".to_owned(),
                retry_ordinal: 0,
                pilot_sample_position: 0,
                run_kind: crate::BaseScalePilotRunKind::ColdInstance,
                compiler_instance_id: crate::NullableObservation::observed(
                    compiler_instance_id.clone(),
                ),
                workload_id: crate::ScalableWorkloadId::Identity,
                workload_revision: crate::WORKLOAD_REVISION_V1,
                graph_profile: crate::GraphProfileId::WideStar.as_str().to_owned(),
                string_profile: crate::BASE_SCALE_STRING_PROFILE.to_owned(),
                generator_version: crate::GENERATOR_VERSION_V1,
                n: 1,
                status: crate::RunStatus::Valid,
                invalidation_reasons: Vec::new(),
                process: successful_process(TIMING_BINARY_ID),
                guard_preflight: guard.clone(),
                child: Some(crate::ScalableTimingChildReport {
                    schema: crate::SCALABLE_TIMING_CHILD_SCHEMA.to_owned(),
                    schema_version: crate::SCALABLE_TIMING_CHILD_SCHEMA_VERSION,
                    binary_id: TIMING_BINARY_ID.to_owned(),
                    allocation_instrumentation_enabled: false,
                    compiler_instance_id: compiler_instance_id.clone(),
                    child_pid: 2,
                    workload_id: crate::ScalableWorkloadId::Identity,
                    workload_revision: crate::WORKLOAD_REVISION_V1,
                    graph_profile: crate::GraphProfileId::WideStar.as_str().to_owned(),
                    string_profile: crate::BASE_SCALE_STRING_PROFILE.to_owned(),
                    generator_version: crate::GENERATOR_VERSION_V1,
                    n: 1,
                    outcome: crate::ScalableTimingOutcome::Success,
                    controlled_allocation_hard_ceiling_bytes: guard
                        .thresholds
                        .compiler_controlled_bytes,
                    guard_peak_live_requested_bytes: Some(1024),
                    wall_time_ns: Some(12_345),
                    semantic_digest_sha256: Some(digest.clone()),
                    controlled_allocation_guard: None,
                }),
                monitor: Some(crate::ChildProcessMonitorReport {
                    observation_count: 1,
                    last_private_bytes: crate::NullableObservation::observed(2048),
                    peak_private_bytes: crate::NullableObservation::observed(4096),
                    last_available_physical_memory_bytes: crate::NullableObservation::observed(
                        48 * 1_073_741_824,
                    ),
                    elapsed_wall_time_ns: 13_000,
                    trigger: None,
                }),
                external_state: Some(external_state),
                kill_error: None,
                monitor_error: None,
                stderr: String::new(),
            }],
            oracle_runs: Vec::new(),
        };
        let candidate = json!({"id": crate::BASELINE_CANDIDATE_ID});
        let runs = pilot_runs(&trusted, &environment, &checkpoint, &candidate)
            .expect("project pilot checkpoint");
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert_eq!(run["compilerInstanceId"]["value"], compiler_instance_id);
        assert_eq!(run["metrics"]["wallTimeNs"]["value"], 12_345);
        assert_eq!(run["metrics"]["privateBytes"]["value"], 4096);
        assert_eq!(run["metrics"]["semanticDigest"]["value"], digest);
        assert_eq!(run["guard"]["trigger"], "none");
        let plan = crate::ScalableStagePlanFactory::from_trusted_contract(&trusted)
            .expect("plan factory")
            .plan(
                crate::ScalableWorkloadId::Identity,
                crate::GraphProfileId::WideStar,
                1,
            )
            .expect("exact plan");
        assert_eq!(
            run["metrics"]["stageBreakdown"]["canonicalLir"]["recordCount"],
            plan.stages.canonical_lir.record_count
        );
    }

    #[test]
    fn candidate_projection_retains_invalid_attempts_and_binds_the_frozen_input_variant() {
        let trusted = crate::load_repository_contract().expect("trusted contract");
        let environment = test_environment();
        let scope = crate::CandidatePerformanceScopeContract::from_trusted_contract(&trusted)
            .expect("candidate performance scope");
        let scale = crate::CandidatePerformanceScalePlan {
            scale_role: crate::CandidateScaleRole::Base,
            n: 1,
            b: 1,
        };
        let stratum = crate::CandidatePipelineStratum::from_scope(
            &scope,
            crate::CandidateKeyDomain::ExternalString,
            scale,
        )
        .expect("candidate stratum");
        let semantic_digest = "2".repeat(64);
        let baseline_id = "std-hashmap-randomstate-v1";
        let comparison_id = "sorted-vec-binary-search-v1";
        let candidate_child = |run_id: &str, candidate_id: &str, wall_time_ns: u64| {
            crate::CandidatePipelineChildReport {
                schema: crate::CANDIDATE_PIPELINE_CHILD_SCHEMA.to_owned(),
                schema_version: crate::CANDIDATE_PIPELINE_CHILD_SCHEMA_VERSION,
                binary_id: TIMING_BINARY_ID.to_owned(),
                child_pid: 2,
                compiler_instance_id: format!("{run_id}/compiler"),
                candidate_id: candidate_id.to_owned(),
                key_domain: crate::CandidateKeyDomain::ExternalString,
                workload_id: scope.workload_id,
                workload_revision: scope.workload_revision,
                graph_profile: scope.graph_profile.as_str().to_owned(),
                string_profile: scope.string_profile.clone(),
                generator_version: scope.generator_version,
                n: 1,
                controlled_allocation_hard_ceiling_bytes: 1_048_576,
                outcome: crate::CandidatePipelineOutcome::Success,
                wall_time_ns: Some(wall_time_ns),
                semantic_digest_sha256: Some(semantic_digest.clone()),
                candidate_pipeline_checksums: None,
                guard_peak_live_requested_bytes: Some(1_024),
                controlled_allocation_guard: None,
            }
        };
        let oracle_run_id = "candidate-qualification/oracle";
        let baseline_run_id = "candidate-qualification/baseline";
        let comparison_run_id = "candidate-qualification/comparison";
        let roster = crate::CandidatePipelineQualifiedRoster {
            stratum: stratum.clone(),
            baseline_id: baseline_id.to_owned(),
            safety_assessments: Vec::new(),
            oracle_run: crate::CandidatePipelineQualificationOracleRun {
                run_id: oracle_run_id.to_owned(),
                status: crate::RunStatus::Valid,
                invalidation_reasons: Vec::new(),
                process: successful_process(ORACLE_BINARY_ID),
                child: Some(crate::ScalableOracleChildReport {
                    schema: crate::SCALABLE_ORACLE_CHILD_SCHEMA.to_owned(),
                    schema_version: crate::SCALABLE_ORACLE_CHILD_SCHEMA_VERSION,
                    binary_id: ORACLE_BINARY_ID.to_owned(),
                    oracle_run_id: oracle_run_id.to_owned(),
                    child_pid: 2,
                    workload_id: scope.workload_id,
                    workload_revision: scope.workload_revision,
                    graph_profile: scope.graph_profile.as_str().to_owned(),
                    string_profile: scope.string_profile.clone(),
                    generator_version: scope.generator_version,
                    n: 1,
                    outcome: crate::ScalableOracleOutcome::Success,
                    controlled_allocation_hard_ceiling_bytes: 1_048_576,
                    guard_peak_live_requested_bytes: Some(1_024),
                    primary_record_count: Some(1),
                    semantic_digest_sha256: Some(semantic_digest.clone()),
                    complete_counts_equal: true,
                    complete_typed_output_equal: true,
                    controlled_allocation_guard: None,
                }),
                monitor: test_monitor(None),
                external_state: None,
                kill_error: None,
                monitor_error: None,
                stderr: String::new(),
            },
            candidate_runs: vec![
                crate::CandidatePipelineQualificationRun {
                    run_id: baseline_run_id.to_owned(),
                    candidate_id: baseline_id.to_owned(),
                    status: crate::RunStatus::Valid,
                    invalidation_reasons: Vec::new(),
                    process: successful_process(TIMING_BINARY_ID),
                    child: Some(candidate_child(baseline_run_id, baseline_id, 10_000)),
                    monitor: test_monitor(None),
                    external_state: None,
                    kill_error: None,
                    monitor_error: None,
                    stderr: String::new(),
                },
                crate::CandidatePipelineQualificationRun {
                    run_id: comparison_run_id.to_owned(),
                    candidate_id: comparison_id.to_owned(),
                    status: crate::RunStatus::Valid,
                    invalidation_reasons: Vec::new(),
                    process: successful_process(TIMING_BINARY_ID),
                    child: Some(candidate_child(comparison_run_id, comparison_id, 9_000)),
                    monitor: test_monitor(None),
                    external_state: None,
                    kill_error: None,
                    monitor_error: None,
                    stderr: String::new(),
                },
            ],
            entries: vec![
                crate::CandidatePipelineQualifiedRosterEntry {
                    candidate_id: baseline_id.to_owned(),
                    disposition: crate::CandidateDisposition::BaselineParticipant,
                    correctness_evidence_run_ids: vec![
                        baseline_run_id.to_owned(),
                        oracle_run_id.to_owned(),
                    ],
                    constant_hash_qualification_id: None,
                    reason: None,
                },
                crate::CandidatePipelineQualifiedRosterEntry {
                    candidate_id: comparison_id.to_owned(),
                    disposition: crate::CandidateDisposition::PerformanceParticipant,
                    correctness_evidence_run_ids: vec![
                        comparison_run_id.to_owned(),
                        oracle_run_id.to_owned(),
                    ],
                    constant_hash_qualification_id: None,
                    reason: None,
                },
            ],
        };
        let invalid_run_id = "candidate/batch-0/round-0/comparison/attempt-0";
        let execution = crate::CandidatePipelineExecution {
            stratum,
            roster,
            schedule: crate::build_two_batch_balanced_schedule(&[
                baseline_id.to_owned(),
                comparison_id.to_owned(),
            ])
            .expect("balanced schedule"),
            complete: false,
            attempts: vec![crate::CandidatePipelinePerformanceAttempt {
                run_id: invalid_run_id.to_owned(),
                round_attempt_id: "candidate/batch-0/round-0/attempt-0".to_owned(),
                retry_ordinal: 0,
                batch: 0,
                round: 0,
                position: 1,
                candidate_id: comparison_id.to_owned(),
                status: crate::RunStatus::Invalid,
                invalidation_reasons: vec![crate::InvalidationReason::MonitoringGap],
                process: successful_process(TIMING_BINARY_ID),
                child: None,
                monitor: test_monitor(Some(crate::ChildMonitorTrigger::MonitoringGap)),
                external_state: None,
                kill_error: None,
                monitor_error: Some("synthetic monitoring gap".to_owned()),
                stderr: String::new(),
            }],
            samples: Vec::new(),
        };
        let bundle = crate::CandidateMatrixExecutionBundle {
            schema: crate::CANDIDATE_MATRIX_CHECKPOINT_SCHEMA.to_owned(),
            schema_version: crate::CANDIDATE_MATRIX_CHECKPOINT_SCHEMA_VERSION,
            scope,
            scales: vec![scale],
            safety_audit: crate::CandidateSafetyAuditSnapshot {
                tool: "cargo-deny 0.20.2".to_owned(),
                command: "cargo deny check".to_owned(),
                observed_at_utc: "2026-08-01T00:00:00Z".to_owned(),
                output_sha256: "3".repeat(64),
                cargo_lock_sha256: "4".repeat(64),
                advisory_error_count: 0,
                license_error_count: 0,
                source_error_count: 0,
                ban_error_count: 0,
                package_audits: Vec::new(),
                assessments: Vec::new(),
            },
            constant_hash_qualifications: Vec::new(),
            executions: vec![execution],
            active_execution: None,
        };
        let bindings = vec![
            json!({"id": baseline_id, "keyDomain": "external-string"}),
            json!({"id": comparison_id, "keyDomain": "external-string"}),
        ];
        let derived = candidate_evidence(
            &trusted,
            &environment,
            &bundle,
            &bindings,
            &json!({"id": crate::BASELINE_CANDIDATE_ID}),
            &[],
        )
        .expect("candidate evidence projection");

        assert_eq!(derived.runs.len(), 4);
        let invalid = derived
            .runs
            .iter()
            .find(|run| run["runId"] == invalid_run_id)
            .expect("invalid performance attempt retained");
        assert_eq!(invalid["status"], "invalid");
        assert_eq!(invalid["invalidationReasons"], json!(["monitoring-gap"]));
        assert_eq!(
            derived.rosters[0]["stratum"]["inputVariantId"],
            "canonical-valid-v1"
        );
        assert!(derived.round_summaries.is_empty());
        assert_eq!(derived.comparisons.len(), 1);
        assert_eq!(derived.comparisons[0]["decision"], "insufficient-evidence");
    }
}
