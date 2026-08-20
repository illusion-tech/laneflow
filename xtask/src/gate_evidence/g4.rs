//! G4 完成证据与 late Related PR recovery 验证。

use std::collections::{BTreeMap, BTreeSet};

use super::{args::*, document::*, g3::*, github::*, model::*};

pub(super) const MERGE_QUEUE_REQUIRED_CHECKS: [&str; 5] = [
    "Governance checks",
    "Rust checks",
    "Dependency policy",
    "Analyze (actions)",
    "Analyze (rust)",
];
pub(super) const GITHUB_ACTIONS_INTEGRATION_ID: u64 = 15368;
pub(super) const CODEQL_ANALYSIS_KEY: &str = ".github/workflows/codeql.yml:analyze";

fn full_commit_oid(value: &str, label: &str) -> Result<String, String> {
    if value.len() != 40 || !value.chars().all(|character| character.is_ascii_hexdigit()) {
        return Err(format!("G4 `{label}` 不是 40 位十六进制 commit OID"));
    }
    Ok(value.to_ascii_lowercase())
}

fn merge_queue_g4_record(body: &str) -> Result<Option<MergeQueueG4Record>, String> {
    let count = body.matches(MERGE_QUEUE_G4_RECORD_START).count();
    if count == 0 {
        return Ok(None);
    }
    if count != 1 {
        return Err("Issue G4 必须包含且只包含一个 merge-queue-g4-evidence:v1 记录".to_string());
    }
    let (_, after_start) = body
        .split_once(MERGE_QUEUE_G4_RECORD_START)
        .expect("marker count guarantees a start marker");
    let (json, _) = after_start
        .split_once(MERGE_QUEUE_G4_RECORD_END)
        .ok_or("merge-queue-g4-evidence:v1 缺少结束 marker")?;
    serde_json::from_str(json.trim())
        .map(Some)
        .map_err(|error| format!("merge-queue-g4-evidence:v1 JSON 无效：{error}"))
}

fn merged_after_queue_activation(pr: &GitHubPullRequest, label: &str) -> Result<bool, String> {
    let merged_at = pr
        .merged_at
        .as_deref()
        .ok_or_else(|| format!("{label} 尚未合并，不能判断 Merge Queue G4 边界"))?;
    let merged_at = parse_utc_timestamp_seconds(merged_at)
        .ok_or_else(|| format!("{label} mergedAt 不是 UTC RFC3339 秒级时间"))?;
    let activation = parse_utc_timestamp_seconds(MERGE_QUEUE_G4_ACTIVATION)
        .expect("MERGE_QUEUE_G4_ACTIVATION must be a valid UTC timestamp");
    Ok(merged_at >= activation)
}

pub(super) fn validate_g4_pr_record(
    repo: &str,
    issue_number: u64,
    issue: &GitHubIssue,
    number: u64,
    role: &str,
    pr: &GitHubPullRequest,
    record: &MergeQueueG4PullRequestRecord,
) -> Result<bool, String> {
    if record.number != number || record.role != role {
        return Err(format!(
            "Merge Queue G4 PR record identity/order 不一致：期望 {role} PR #{number}"
        ));
    }
    let h_pr = full_commit_oid(&record.h_pr, &format!("PR #{number} H_pr"))?;
    let h_main = full_commit_oid(&record.h_main, &format!("PR #{number} H_main"))?;
    if h_pr != pr.head_ref_oid.to_ascii_lowercase() {
        return Err(format!("G4 PR #{number} H_pr 与 GitHub headRefOid 不一致"));
    }
    let merge_commit = pr
        .merge_commit
        .as_ref()
        .ok_or_else(|| format!("PR #{number} 缺少 GitHub mergeCommit，无法验证 H_main"))?;
    if h_main != merge_commit.oid.to_ascii_lowercase() {
        return Err(format!(
            "G4 PR #{number} H_main 与 GitHub mergeCommit OID 不一致"
        ));
    }

    let uses_queue = merged_after_queue_activation(pr, &format!("PR #{number}"))?;
    if !uses_queue {
        if record.mode != "pre_activation"
            || record.reason.as_deref().is_none_or(str::is_empty)
            || record.h_mg.is_some()
            || record.checks_conclusion.is_some()
            || record.checks_url.is_some()
            || record.chain.is_some()
            || record.inclusion_method.is_some()
            || record.inclusion_evidence_url.is_some()
            || record.bootstrap_evidence_url.is_some()
        {
            return Err(format!(
                "G4 PR #{number} 在 activation 前合并，必须只记录 pre_activation identity 与非空 reason"
            ));
        }
        return Ok(false);
    }

    if record.mode == CODEQL_QUEUE_BOOTSTRAP_MODE {
        let is_exact_bootstrap = repo == "illusion-tech/laneflow"
            && issue_number == 451
            && number == 452
            && role == "related"
            && pr.merged_at.as_deref() == Some("2026-08-20T07:27:35Z")
            && record.reason.as_deref() == Some(CODEQL_QUEUE_BOOTSTRAP_REASON)
            && record.bootstrap_evidence_url.as_deref()
                == Some(CODEQL_QUEUE_BOOTSTRAP_EVIDENCE_URL)
            && record.h_mg.is_none()
            && record.checks_conclusion.is_none()
            && record.checks_url.is_none()
            && record.chain.is_none()
            && record.inclusion_method.is_none()
            && record.inclusion_evidence_url.is_none();
        let evidence = issue
            .comments
            .iter()
            .find(|comment| comment.url == CODEQL_QUEUE_BOOTSTRAP_EVIDENCE_URL);
        let has_trusted_authorization = evidence.is_some_and(|comment| {
            comment.author.as_ref().map(|author| author.login.as_str()) == Some("wangzishi")
                && !comment.includes_created_edit
                && comment.created_at == "2026-08-20T06:46:31Z"
                && comment.body.contains("当前只授权 Related PR 1 / bootstrap")
                && comment
                    .body
                    .contains("不修改 live Ruleset、Merge Queue 或 `allow_auto_merge`")
        });
        if !is_exact_bootstrap || !has_trusted_authorization {
            return Err(
                "activation_bootstrap 只允许 #451 Related PR #452 的已冻结 CodeQL queue bootstrap 记录"
                    .to_string(),
            );
        }
        return Ok(false);
    }

    if record.mode != "merge_queue"
        || record.reason.is_some()
        || record.bootstrap_evidence_url.is_some()
    {
        return Err(format!(
            "G4 PR #{number} 在 activation 边界后合并，必须使用 merge_queue record；非队列例外需先扩展结构化治理契约"
        ));
    }
    let h_mg = full_commit_oid(
        record
            .h_mg
            .as_deref()
            .ok_or_else(|| format!("G4 PR #{number} merge_queue record 缺少 H_mg"))?,
        &format!("PR #{number} H_mg"),
    )?;
    if h_mg == h_pr || h_mg == h_main {
        return Err(format!("G4 PR #{number} H_mg 必须独立于 H_pr 与 H_main"));
    }
    if record.checks_conclusion.as_deref() != Some("success") {
        return Err(format!(
            "G4 PR #{number} H_mg required checks conclusion 必须为 success"
        ));
    }
    let expected_checks_url = format!("https://github.com/{repo}/commit/{h_mg}/checks");
    if record.checks_url.as_deref() != Some(expected_checks_url.as_str()) {
        return Err(format!(
            "G4 PR #{number} checksUrl 必须精确绑定 H_mg 的 commit checks 页面"
        ));
    }
    let expected_chain = format!("{h_pr} -> {h_mg} -> {h_main}");
    if record.chain.as_deref() != Some(expected_chain.as_str()) {
        return Err(format!(
            "G4 PR #{number} chain 必须按 H_pr -> H_mg -> H_main 规范顺序记录"
        ));
    }
    if record.inclusion_method.as_deref() != Some(MERGE_QUEUE_G4_INCLUSION_METHOD) {
        return Err(format!(
            "G4 PR #{number} inclusionMethod 必须使用已冻结的 trusted merge_group + compare 方法"
        ));
    }
    let expected_inclusion_url = format!("https://github.com/{repo}/compare/{h_pr}...{h_mg}");
    if record.inclusion_evidence_url.as_deref() != Some(expected_inclusion_url.as_str()) {
        return Err(format!(
            "G4 PR #{number} inclusionEvidenceUrl 必须精确绑定 H_pr...H_mg compare"
        ));
    }
    Ok(true)
}

fn validate_merge_queue_g4_evidence(
    args: &GateEvidenceArgs,
    issue: &GitHubIssue,
    body: &str,
    delivery_pr: &GitHubPullRequest,
    related_prs: &[GitHubPullRequest],
) -> Result<(), String> {
    let delivery_number = args
        .delivery_pr
        .ok_or("G4 validation 缺少 Delivery PR 参数")?;
    let any_post_activation =
        merged_after_queue_activation(delivery_pr, &format!("Delivery PR #{delivery_number}"))?
            || args
                .related_prs
                .iter()
                .zip(related_prs)
                .map(|(number, pr)| {
                    merged_after_queue_activation(pr, &format!("Related PR #{number}"))
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .any(|value| value);

    let Some(record) = merge_queue_g4_record(body)? else {
        return if any_post_activation {
            Err("post-activation G4 缺少 merge-queue-g4-evidence:v1 记录".to_string())
        } else {
            Ok(())
        };
    };
    unique_metadata_line(body, "Merge Queue evidence")?;
    if record.schema_version != 1 || record.activation_boundary != MERGE_QUEUE_G4_ACTIVATION {
        return Err(
            "merge-queue-g4-evidence:v1 schemaVersion / activationBoundary 不匹配".to_string(),
        );
    }
    if record.pull_requests.len() != 1 + related_prs.len() {
        return Err("Merge Queue G4 PR record 集合必须等于 Delivery + 全部 Related PR".to_string());
    }

    let mut queue_records = 0usize;
    queue_records += validate_g4_pr_record(
        &args.repo,
        args.issue,
        issue,
        delivery_number,
        "delivery",
        delivery_pr,
        &record.pull_requests[0],
    )? as usize;
    for ((number, pr), entry) in args
        .related_prs
        .iter()
        .zip(related_prs)
        .zip(record.pull_requests.iter().skip(1))
    {
        queue_records +=
            validate_g4_pr_record(&args.repo, args.issue, issue, *number, "related", pr, entry)?
                as usize;
    }
    if queue_records > 0 && !unique_metadata_line(body, "合并")?.contains("Merge Queue") {
        return Err("post-activation G4 `- 合并：` 必须明确记录 Merge Queue".to_string());
    }
    Ok(())
}

pub(super) fn validate_trusted_merge_group_evidence(
    repo: &str,
    number: u64,
    base_ref_name: &str,
    h_mg: &str,
    check_runs: &[GitHubCheckRun],
    workflow_runs: &[GitHubWorkflowRun],
    codeql_analyses: &[GitHubCodeScanningAnalysis],
    branch_rules: &[GitHubBranchRule],
    timeline: &[GitHubTimelineItem],
) -> Result<(), String> {
    if base_ref_name.is_empty() {
        return Err(format!("PR #{number} 缺少 trusted baseRefName"));
    }
    let merged_positions = timeline
        .iter()
        .enumerate()
        .filter(|(_, item)| item.event == "merged")
        .collect::<Vec<_>>();
    let [(merged_position, merged_event)] = merged_positions.as_slice() else {
        return Err(format!(
            "PR #{number} timeline 必须包含且只包含一个 merged event"
        ));
    };
    let (queue_position, queue_event) = timeline[..*merged_position]
        .iter()
        .enumerate()
        .rev()
        .find(|(_, item)| {
            matches!(
                item.event.as_str(),
                "added_to_merge_queue" | "removed_from_merge_queue"
            )
        })
        .ok_or_else(|| format!("PR #{number} merged 前缺少 Merge Queue timeline event"))?;
    if queue_event.event != "added_to_merge_queue" {
        return Err(format!(
            "PR #{number} 最后一个 merge 前 queue event 是 `{}`（position={queue_position}），不能证明保持入队到合并",
            queue_event.event
        ));
    }
    let queued_at = parse_utc_timestamp_seconds(
        queue_event
            .created_at
            .as_deref()
            .ok_or_else(|| format!("PR #{number} added_to_merge_queue 缺少 created_at"))?,
    )
    .ok_or_else(|| format!("PR #{number} added_to_merge_queue created_at 无效"))?;
    let merged_at = parse_utc_timestamp_seconds(
        merged_event
            .created_at
            .as_deref()
            .ok_or_else(|| format!("PR #{number} merged event 缺少 created_at"))?,
    )
    .ok_or_else(|| format!("PR #{number} merged event created_at 无效"))?;

    let expected_branch_prefix = format!("gh-readonly-queue/{base_ref_name}/pr-{number}-");
    let mut pr_bound_runs = Vec::new();
    for run in workflow_runs.iter().filter(|run| {
        run.event == "merge_group"
            && run
                .head_branch
                .as_deref()
                .is_some_and(|branch| branch.starts_with(&expected_branch_prefix))
    }) {
        let created_at = parse_utc_timestamp_seconds(&run.created_at)
            .ok_or_else(|| format!("PR #{number} merge_group run created_at 无效"))?;
        if created_at >= queued_at && created_at <= merged_at {
            pr_bound_runs.push((created_at, run.id, run));
        }
    }
    let latest_generation = pr_bound_runs
        .iter()
        .max_by_key(|(created_at, id, _)| (*created_at, *id))
        .ok_or_else(|| format!("PR #{number} 入队到合并之间缺少 PR-bound merge_group run"))?;
    if !latest_generation.2.head_sha.eq_ignore_ascii_case(h_mg) {
        return Err(format!(
            "PR #{number} record H_mg 不是合并前最后一代 merge_group head：record={h_mg} latest={}",
            latest_generation.2.head_sha
        ));
    }
    let queue_branch = latest_generation
        .2
        .head_branch
        .as_deref()
        .expect("PR-bound merge_group run filter guarantees head_branch");
    let expected_analysis_ref = format!("refs/heads/{queue_branch}");
    if !pr_bound_runs.iter().any(|(_, _, run)| {
        run.head_sha.eq_ignore_ascii_case(h_mg)
            && run.status == "completed"
            && run.conclusion.as_deref() == Some("success")
            && run
                .html_url
                .starts_with(&format!("https://github.com/{repo}/actions/runs/"))
    }) {
        return Err(format!(
            "PR #{number} H_mg 未绑定 trusted GitHub merge_group success workflow run"
        ));
    }

    let mut required_checks = BTreeMap::<String, BTreeSet<Option<u64>>>::new();
    let mut ruleset_required_count = 0usize;
    let mut has_live_merge_queue_rule = false;
    for rule in branch_rules {
        if rule.rule_type == "merge_queue" {
            has_live_merge_queue_rule = true;
            continue;
        }
        if rule.rule_type != "required_status_checks" {
            continue;
        }
        let parameters = rule
            .parameters
            .as_ref()
            .ok_or("required_status_checks rule 缺少 parameters")?;
        for required in &parameters.required_status_checks {
            ruleset_required_count += 1;
            required_checks
                .entry(required.context.clone())
                .or_default()
                .insert(required.integration_id);
        }
    }
    if !has_live_merge_queue_rule {
        return Err(format!(
            "PR #{number} base `{base_ref_name}` 缺少 live merge_queue rule；G4 失败关闭"
        ));
    }
    if ruleset_required_count == 0 {
        return Err(format!(
            "PR #{number} base `{base_ref_name}` 缺少 live required status checks；G4 失败关闭"
        ));
    }
    let missing_required = MERGE_QUEUE_REQUIRED_CHECKS
        .iter()
        .copied()
        .filter(|name| !required_checks.contains_key(*name))
        .collect::<Vec<_>>();
    if !missing_required.is_empty() {
        return Err(format!(
            "PR #{number} base `{base_ref_name}` live required status checks 缺少固定 context：{}",
            missing_required.join(", ")
        ));
    }
    for name in MERGE_QUEUE_REQUIRED_CHECKS {
        if !required_checks.get(name).is_some_and(|ids| {
            !ids.is_empty()
                && ids
                    .iter()
                    .all(|id| *id == Some(GITHUB_ACTIONS_INTEGRATION_ID))
        }) {
            return Err(format!(
                "PR #{number} live required check `{name}` 存在未绑定 integration_id={GITHUB_ACTIONS_INTEGRATION_ID} 的 source"
            ));
        }
    }

    for name in required_checks.keys() {
        let latest = check_runs
            .iter()
            .filter(|run| run.name == *name && run.head_sha.eq_ignore_ascii_case(h_mg))
            .filter_map(|run| {
                let completed_at = run
                    .completed_at
                    .as_deref()
                    .and_then(parse_utc_timestamp_seconds)?;
                (completed_at <= merged_at).then_some((completed_at, run.id, run))
            })
            .max_by_key(|(completed_at, id, _)| (*completed_at, *id))
            .ok_or_else(|| {
                format!("PR #{number} H_mg 缺少 merge 前完成的 trusted GitHub check `{name}`")
            })?
            .2;
        if latest.status != "completed" || latest.conclusion.as_deref() != Some("success") {
            return Err(format!(
                "PR #{number} H_mg 最新 check `{name}` 不是 completed/success：status={} conclusion={:?}",
                latest.status, latest.conclusion
            ));
        }
        if !latest
            .html_url
            .starts_with(&format!("https://github.com/{repo}/"))
        {
            return Err(format!(
                "PR #{number} H_mg check `{name}` URL 不属于当前 repository"
            ));
        }
        if MERGE_QUEUE_REQUIRED_CHECKS.contains(&name.as_str())
            && latest.app.as_ref().map(|app| app.id) != Some(GITHUB_ACTIONS_INTEGRATION_ID)
        {
            return Err(format!(
                "PR #{number} H_mg check `{name}` source 不是 integration_id={GITHUB_ACTIONS_INTEGRATION_ID}"
            ));
        }
    }

    for language in ["actions", "rust"] {
        let category = format!("/language:{language}");
        let latest = codeql_analyses
            .iter()
            .filter(|analysis| {
                analysis.commit_sha.eq_ignore_ascii_case(h_mg)
                    && analysis.git_ref == expected_analysis_ref
                    && analysis.analysis_key == CODEQL_ANALYSIS_KEY
                    && analysis.category == category
                    && analysis.tool.name == "CodeQL"
            })
            .filter_map(|analysis| {
                let created_at = parse_utc_timestamp_seconds(&analysis.created_at)?;
                (created_at >= queued_at && created_at <= merged_at).then_some((
                    created_at,
                    analysis.id,
                    analysis,
                ))
            })
            .max_by_key(|(created_at, id, _)| (*created_at, *id))
            .ok_or_else(|| {
                format!(
                    "PR #{number} H_mg 缺少合并前 trusted advanced CodeQL `{language}` analysis"
                )
            })?
            .2;
        if !latest.error.is_empty() {
            return Err(format!(
                "PR #{number} H_mg advanced CodeQL `{language}` analysis 包含错误：{}",
                latest.error
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_live_merge_queue_g4_evidence(args: &GateEvidenceArgs) -> Result<(), String> {
    let issue = gh_issue_view_for_phase(&args.repo, args.issue, GateEvidencePhase::G4)?;
    let issue_g4_permalink = completed_gate_permalink(&issue.body, "G4")?;
    let comment = comment_for_permalink(&issue, &issue_g4_permalink, "Issue G4")?;
    let Some(record) = merge_queue_g4_record(&comment.body)? else {
        return Ok(());
    };
    for entry in record
        .pull_requests
        .iter()
        .filter(|entry| entry.mode == "merge_queue")
    {
        let h_mg = full_commit_oid(
            entry
                .h_mg
                .as_deref()
                .ok_or_else(|| format!("PR #{} merge_queue record 缺少 H_mg", entry.number))?,
            &format!("PR #{} H_mg", entry.number),
        )?;
        let pr = gh_pr_view_for_phase(&args.repo, entry.number, GateEvidencePhase::G4)?;
        let check_runs = gh_commit_check_runs(&args.repo, &h_mg)?;
        let workflow_runs = gh_merge_group_workflow_runs(&args.repo)?;
        let expected_branch_prefix = format!(
            "gh-readonly-queue/{}/pr-{}-",
            pr.base_ref_name, entry.number
        );
        let codeql_analyses = workflow_runs
            .iter()
            .filter(|run| {
                run.event == "merge_group"
                    && run.head_sha.eq_ignore_ascii_case(&h_mg)
                    && run
                        .head_branch
                        .as_deref()
                        .is_some_and(|branch| branch.starts_with(&expected_branch_prefix))
            })
            .max_by_key(|run| (&run.created_at, run.id))
            .and_then(|run| run.head_branch.as_deref())
            .map(|branch| gh_code_scanning_analyses(&args.repo, &format!("refs/heads/{branch}")))
            .transpose()?
            .unwrap_or_default();
        let branch_rules = gh_branch_rules(&args.repo, &pr.base_ref_name)?;
        let timeline = gh_issue_timeline(&args.repo, entry.number)?;
        validate_trusted_merge_group_evidence(
            &args.repo,
            entry.number,
            &pr.base_ref_name,
            &h_mg,
            &check_runs,
            &workflow_runs,
            &codeql_analyses,
            &branch_rules,
            &timeline,
        )?;
    }
    Ok(())
}

pub(super) fn reject_inapplicable_g4_recovery_marker(issue: &GitHubIssue) -> Result<(), String> {
    let issue_g4_permalink = completed_gate_permalink(&issue.body, "G4")?;
    let g4_comment = comment_for_permalink(issue, &issue_g4_permalink, "Issue G4")?;
    if g4_comment.body.contains(G3_FULL_SET_RECOVERY_START) {
        return Err(
            "不存在 late Related PR 时，Issue G4 comment 不得包含 G3 full-set recovery 记录"
                .to_string(),
        );
    }
    Ok(())
}

pub(super) fn has_late_related_pr(
    args: &GateEvidenceArgs,
    delivery_pr: &GitHubPullRequest,
    related_prs: &[GitHubPullRequest],
) -> Result<bool, String> {
    if args.related_prs.len() != related_prs.len() {
        return Err("Related PR 参数与已读取 PR 数量不一致".to_string());
    }
    let delivery_merged_at = delivery_pr
        .merged_at
        .as_deref()
        .ok_or("Delivery PR 尚未合并，不能判断 late Related PR")?;
    let delivery_merged_at_seconds = parse_utc_timestamp_seconds(delivery_merged_at)
        .ok_or("Delivery PR mergedAt 不是 UTC RFC3339 秒级时间")?;
    for (number, related_pr) in args.related_prs.iter().zip(related_prs) {
        let related_created_at = parse_utc_timestamp_seconds(&related_pr.created_at)
            .ok_or_else(|| format!("Related PR #{number} createdAt 不是 UTC RFC3339 秒级时间"))?;
        if related_created_at == delivery_merged_at_seconds {
            return Err(format!(
                "Related PR #{number} createdAt 与 Delivery mergedAt 同秒，无法安全判断是否为 late Related PR"
            ));
        }
        if related_created_at > delivery_merged_at_seconds {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn validate_original_related_g3_before_delivery_merge(
    number: u64,
    related_g3_effective_at: u64,
    delivery_merged_at: u64,
) -> Result<(), String> {
    if related_g3_effective_at >= delivery_merged_at {
        return Err(format!(
            "original Related PR #{number} G3 comment 生效时间必须严格早于 Delivery PR 合并时间"
        ));
    }
    Ok(())
}

pub(super) fn validate_g4_g3_full_set_recovery(
    args: &GateEvidenceArgs,
    issue: &GitHubIssue,
    delivery_pr: &GitHubPullRequest,
    related_prs: &[GitHubPullRequest],
) -> Result<(), String> {
    if args.phase != GateEvidencePhase::G4 {
        return Err("G3 full-set recovery 只允许用于 G4".to_string());
    }
    let delivery_number = args
        .delivery_pr
        .ok_or("G3 full-set recovery 缺少 Delivery PR")?;
    let delivery_merged_at = delivery_pr
        .merged_at
        .as_deref()
        .ok_or("Delivery PR 尚未合并，不能使用 G3 full-set recovery")?;
    let delivery_merged_at_seconds = parse_utc_timestamp_seconds(delivery_merged_at)
        .ok_or("Delivery PR mergedAt 不是 UTC RFC3339 秒级时间")?;
    let issue_g3_line = completed_gate_line(&issue.body, "G3")?;
    let delivery_pr_line = metadata_line(&issue.body, "Delivery PR")?;
    if !delivery_pr_line.contains(&format!("#{delivery_number}")) {
        return Err(format!(
            "Issue 的 `Delivery PR` 字段未记录 Delivery PR #{}",
            delivery_number
        ));
    }
    let related_prs_line = metadata_line(&issue.body, "Related PRs")?;
    validate_recovery_related_pr_order(related_prs_line, &args.related_prs)?;
    let requested_related_prs = args.related_prs.iter().copied().collect::<BTreeSet<_>>();

    let issue_g4_permalink = completed_gate_permalink(&issue.body, "G4")?;
    let g4_comment = comment_for_permalink(issue, &issue_g4_permalink, "Issue G4")?;
    let (record, evidence_urls) = parse_g3_full_set_recovery(g4_comment, args, delivery_merged_at)?;
    let mut reconstructed_related_prs = record.original_related_prs.clone();
    reconstructed_related_prs.extend(record.late_related_prs.iter().copied());
    if reconstructed_related_prs != args.related_prs {
        return Err(format!(
            "G3 full-set recovery 的 originalRelatedPrs + lateRelatedPrs 必须按顺序等于最终 Related PR 参数 [{}]",
            format_issue_numbers(&requested_related_prs)
        ));
    }

    let delivery_permalink = completed_gate_permalink(&delivery_pr.body, "G3")?;
    if !line_links_to_comment_permalink(&issue.body, issue_g3_line, &delivery_permalink) {
        return Err("Issue 的 G3 checkbox 未回链 Delivery PR 的 G3 comment permalink".to_string());
    }
    if !evidence_urls.contains(&delivery_permalink) {
        return Err(
            "G3 full-set recovery evidenceRefs 未覆盖 Delivery PR G3 permalink".to_string(),
        );
    }
    let original_args = GateEvidenceArgs {
        phase: GateEvidencePhase::G3,
        repo: args.repo.clone(),
        issue: args.issue,
        delivery_pr: Some(delivery_number),
        related_prs: record.original_related_prs.clone(),
    };
    let delivery_g3_effective_at =
        parse_utc_timestamp_seconds(&g3_effective_at_for_merge_validation(
            delivery_pr,
            &delivery_permalink,
            "Delivery PR original G3 comment",
            &original_args,
        )?)
        .ok_or("Delivery PR original G3 comment effectiveAt 不是 UTC RFC3339 秒级时间")?;
    if delivery_g3_effective_at >= delivery_merged_at_seconds {
        return Err(
            "G3 full-set recovery 的 Delivery PR original G3 comment 生效时间必须严格早于 Delivery merge"
                .to_string(),
        );
    }
    let allow_delivery_legacy_exception = historical_exception_applies_to_target(
        g4_comment,
        args.issue,
        delivery_number,
        &delivery_permalink,
    )?;
    validate_comment(
        delivery_pr,
        &delivery_permalink,
        G3_COMMENT_FIELDS,
        "Delivery PR original G3",
        &original_args,
        allow_delivery_legacy_exception,
    )?;
    if !delivery_pr
        .closing_issues_references
        .iter()
        .any(|reference| issue_reference_matches(reference, &args.repo, args.issue))
    {
        return Err(format!(
            "Delivery PR #{} 的 closingIssuesReferences 未覆盖 Issue #{}",
            delivery_number, args.issue
        ));
    }

    let original_related_prs = record
        .original_related_prs
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let late_related_prs = record
        .late_related_prs
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if original_related_prs.len() != record.original_related_prs.len()
        || late_related_prs.len() != record.late_related_prs.len()
        || !original_related_prs.is_disjoint(&late_related_prs)
    {
        return Err(
            "G3 full-set recovery 的 originalRelatedPrs / lateRelatedPrs 不得重复或重叠"
                .to_string(),
        );
    }
    if late_related_prs.is_empty() {
        return Err("G3 full-set recovery 至少需要一个 late Related PR".to_string());
    }

    for (number, related_pr) in args.related_prs.iter().zip(related_prs) {
        let related_args = GateEvidenceArgs {
            phase: GateEvidencePhase::G3,
            repo: args.repo.clone(),
            issue: args.issue,
            delivery_pr: None,
            related_prs: vec![*number],
        };
        let related_permalink = completed_gate_permalink(&related_pr.body, "G3")?;
        let allow_related_legacy_exception = historical_exception_applies_to_target(
            g4_comment,
            args.issue,
            *number,
            &related_permalink,
        )?;
        validate_related_pr_g3(
            &related_args,
            &issue.body,
            issue_g3_line,
            *number,
            related_pr,
            allow_related_legacy_exception,
        )?;
        if !evidence_urls.contains(&related_permalink) {
            return Err(format!(
                "G3 full-set recovery evidenceRefs 未覆盖 Related PR #{number} G3 permalink"
            ));
        }
        let related_created_at = parse_utc_timestamp_seconds(&related_pr.created_at)
            .ok_or_else(|| format!("Related PR #{number} createdAt 不是 UTC RFC3339 秒级时间"))?;
        if late_related_prs.contains(number) {
            if related_created_at <= delivery_merged_at_seconds {
                return Err(format!(
                    "late Related PR #{number} 必须在 Delivery PR 合并后创建"
                ));
            }
        } else if original_related_prs.contains(number) {
            if related_created_at > delivery_merged_at_seconds {
                return Err(format!(
                    "original Related PR #{number} 不得在 Delivery PR 合并后创建"
                ));
            }
            let related_g3_effective_at =
                parse_utc_timestamp_seconds(&g3_effective_at_for_merge_validation(
                    related_pr,
                    &related_permalink,
                    &format!("Related PR #{number} G3 comment"),
                    &related_args,
                )?)
                .ok_or_else(|| {
                    format!("Related PR #{number} G3 comment effectiveAt 不是 UTC RFC3339 秒级时间")
                })?;
            validate_original_related_g3_before_delivery_merge(
                *number,
                related_g3_effective_at,
                delivery_merged_at_seconds,
            )?;
        } else {
            return Err(format!(
                "Related PR #{number} 未归入 originalRelatedPrs 或 lateRelatedPrs"
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_g4_evidence(
    args: &GateEvidenceArgs,
    issue: &GitHubIssue,
    delivery_pr: &GitHubPullRequest,
    related_prs: &[GitHubPullRequest],
) -> Result<(), String> {
    validate_related_pr_snapshot_count(args, related_prs)?;
    if issue.state != "OPEN" {
        return Err("G4 断言必须在手动关闭 Issue 前运行".to_string());
    }
    let merged_at = delivery_pr
        .merged_at
        .as_deref()
        .ok_or("Delivery PR 尚未合并，不能通过 G4")?;
    if delivery_pr.state != "MERGED" {
        return Err("Delivery PR 状态不是 MERGED，不能通过 G4".to_string());
    }
    let mut latest_merge = merged_at;
    for (number, related_pr) in args.related_prs.iter().zip(related_prs) {
        let related_merged_at = related_pr
            .merged_at
            .as_deref()
            .ok_or_else(|| format!("Related PR #{number} 尚未合并，不能通过 G4"))?;
        if related_pr.state != "MERGED" {
            return Err(format!("Related PR #{number} 状态不是 MERGED，不能通过 G4"));
        }
        if related_merged_at > latest_merge {
            latest_merge = related_merged_at;
        }
    }

    let issue_g4_permalink = completed_gate_permalink(&issue.body, "G4")?;
    let g4_comment = comment_for_permalink(issue, &issue_g4_permalink, "Issue G4")?;
    validate_comment_body(&g4_comment.body, G4_COMMENT_FIELDS, "Issue G4")?;
    validate_merge_queue_g4_evidence(args, issue, &g4_comment.body, delivery_pr, related_prs)?;
    let delivery_number = args
        .delivery_pr
        .ok_or("G4 validation 缺少 Delivery PR 参数")?;
    let delivery_permalink = completed_gate_permalink(&delivery_pr.body, "G3")?;
    let mut allow_g4_legacy_exception = historical_exception_applies_to_target(
        g4_comment,
        args.issue,
        delivery_number,
        &delivery_permalink,
    )?;
    for (number, related_pr) in args.related_prs.iter().zip(related_prs) {
        let related_permalink = completed_gate_permalink(&related_pr.body, "G3")?;
        allow_g4_legacy_exception |= historical_exception_applies_to_target(
            g4_comment,
            args.issue,
            *number,
            &related_permalink,
        )?;
    }
    validate_gate_assertion_with_legacy_exception(
        &g4_comment.body,
        "Issue G4",
        args,
        GateEvidencePhase::G4,
        allow_g4_legacy_exception,
    )?;
    if g4_comment.created_at.as_str() < latest_merge {
        return Err("Issue G4 comment 早于最后一个关联 PR 的合并时间".to_string());
    }
    if !delivery_pr.body.contains("G4 回写") || !delivery_pr.body.contains(&issue_g4_permalink) {
        return Err(
            "Delivery PR body 缺少指向 Issue G4 comment 的 `G4 回写` permalink".to_string(),
        );
    }
    for gate in ["G0", "G1", "G2", "G3", "G4"] {
        completed_gate_line(&issue.body, gate)?;
    }
    if !is_laneflow_project_done(&issue.project_items) {
        return Err("Issue 尚未处于 LaneFlow Project 的 Done 状态".to_string());
    }
    if !is_laneflow_project_done(&delivery_pr.project_items) {
        return Err("Delivery PR 尚未处于 LaneFlow Project 的 Done 状态".to_string());
    }
    for (number, related_pr) in args.related_prs.iter().zip(related_prs) {
        if !is_laneflow_project_done(&related_pr.project_items) {
            return Err(format!(
                "Related PR #{number} 尚未处于 LaneFlow Project 的 Done 状态"
            ));
        }
    }
    Ok(())
}

pub(super) fn is_laneflow_project_done(project_items: &[ProjectItem]) -> bool {
    project_items.iter().any(|item| {
        item.title == "LaneFlow"
            && item
                .status
                .as_ref()
                .is_some_and(|status| status.name == "Done")
    })
}
