//! Gate evidence 模块的行为等价回归测试。

use super::fixtures::*;
use super::*;

#[test]
fn parses_gate_evidence_arguments() {
    let args = vec![
        "g4".to_string(),
        "--repo".to_string(),
        "illusion-tech/laneflow".to_string(),
        "--issue".to_string(),
        "#60".to_string(),
        "--delivery-pr".to_string(),
        "61".to_string(),
        "--related-pr".to_string(),
        "62".to_string(),
    ];

    assert_eq!(
        parse_gate_evidence_args(&args),
        Ok(GateEvidenceArgs {
            phase: GateEvidencePhase::G4,
            repo: "illusion-tech/laneflow".to_string(),
            issue: 60,
            delivery_pr: Some(61),
            related_prs: vec![62],
        })
    );
}

#[test]
fn recovery_preserves_issue_related_pr_order_and_rejects_duplicates() {
    assert!(validate_recovery_related_pr_order("- Related PRs：#62、#63", &[62, 63]).is_ok());

    let order_error = validate_recovery_related_pr_order("- Related PRs：#63、#62", &[62, 63])
        .expect_err("recovery must preserve Issue metadata order");
    assert!(order_error.contains("顺序"));

    let duplicate_error = validate_recovery_related_pr_order("- Related PRs：#62、#62", &[62])
        .expect_err("recovery must reject duplicate Related PR metadata");
    assert!(duplicate_error.contains("不得包含重复 PR"));
}

#[test]
fn parses_related_only_g3_arguments() {
    let args = vec![
        "g3".to_string(),
        "--repo".to_string(),
        "illusion-tech/laneflow".to_string(),
        "--issue".to_string(),
        "60".to_string(),
        "--related-pr".to_string(),
        "62".to_string(),
    ];

    assert_eq!(parse_gate_evidence_args(&args), Ok(related_only_g3_args()));
}

#[test]
fn parses_gate_evidence_target_arguments() {
    let args = vec![
        "--repo".to_string(),
        "illusion-tech/laneflow".to_string(),
        "--pr".to_string(),
        "#62".to_string(),
    ];

    assert_eq!(
        parse_gate_evidence_target_args(&args),
        Ok(("illusion-tech/laneflow".to_string(), 62))
    );
}

#[test]
fn target_requires_exact_closing_issue_associations() {
    let mut delivery_target = delivery_pr(None);
    delivery_target
        .closing_issues_references
        .push(issue_reference("illusion-tech/laneflow", 61));
    let delivery_error = validate_gate_evidence_target_pr(
        "illusion-tech/laneflow",
        GateEvidencePhase::G3,
        &delivery_target,
        GateEvidencePrRole::Delivery,
        &[60],
    )
    .expect_err("Delivery closing references must exactly match declared Issues");
    assert!(delivery_error.contains("声明 [#60]；closing [#60, #61]"));

    let mut related_pr = related_pr(false);
    related_pr.closing_issues_references = vec![issue_reference("illusion-tech/laneflow", 61)];
    let related_error = validate_gate_evidence_target_pr(
        "illusion-tech/laneflow",
        GateEvidencePhase::G3,
        &related_pr,
        GateEvidencePrRole::Related,
        &[60],
    )
    .expect_err("Related targets must not close any Issue");
    assert!(related_error.contains("Related PR 不得关闭任何 Issue"));

    let mut cross_repo_delivery = delivery_pr(None);
    cross_repo_delivery.closing_issues_references =
        vec![issue_reference("illusion-tech/other", 60)];
    let cross_repo_error = validate_gate_evidence_target_pr(
        "illusion-tech/laneflow",
        GateEvidencePhase::G3,
        &cross_repo_delivery,
        GateEvidencePrRole::Delivery,
        &[60],
    )
    .expect_err("Delivery closing references must preserve repository identity");
    assert!(cross_repo_error.contains("illusion-tech/other#60"));
}

#[test]
fn delivery_full_set_validates_each_related_target_metadata() {
    let related = related_pr(false);
    assert_eq!(
        validate_related_full_set_member_metadata(
            "illusion-tech/laneflow",
            GateEvidencePhase::G4,
            60,
            &related,
        )
        .unwrap(),
        vec![60]
    );

    let mut wrong_role = related_pr(false);
    wrong_role.body = wrong_role
        .body
        .replace("PR 角色：Related PR", "PR 角色：Delivery PR");
    let role_error = validate_related_full_set_member_metadata(
        "illusion-tech/laneflow",
        GateEvidencePhase::G4,
        60,
        &wrong_role,
    )
    .expect_err("full-set member must declare the Related role");
    assert!(role_error.contains("必须精确为 `Related PR`"));

    let closing_error = validate_related_full_set_member_metadata(
        "illusion-tech/laneflow",
        GateEvidencePhase::G4,
        60,
        &related_pr(true),
    )
    .expect_err("full-set Related member must keep an empty closing set");
    assert!(closing_error.contains("Related PR 不得关闭任何 Issue"));

    let mut wrong_issue = related_pr(false);
    wrong_issue.body = wrong_issue
        .body
        .replace("关联 Issue：#60", "关联 Issue：#61");
    let issue_error = validate_related_full_set_member_metadata(
        "illusion-tech/laneflow",
        GateEvidencePhase::G4,
        60,
        &wrong_issue,
    )
    .expect_err("full-set member must declare the current Issue");
    assert!(issue_error.contains("未包含当前 Issue #60"));
}

#[test]
fn g3_remote_queries_omit_g4_only_project_items() {
    assert!(!gh_issue_fields(GateEvidencePhase::G3).contains("projectItems"));
    assert!(!gh_pr_fields(GateEvidencePhase::G3).contains("projectItems"));
    assert!(gh_issue_fields(GateEvidencePhase::G4).contains("projectItems"));
    assert!(gh_pr_fields(GateEvidencePhase::G4).contains("projectItems"));
}

#[test]
fn resolves_delivery_target_with_complete_related_set() {
    let (role, issue_numbers) =
        parse_gate_evidence_target_metadata("- 关联 Issue：#60\n- PR 角色：`Delivery PR`").unwrap();
    let args = resolve_gate_evidence_target_args(
        "illusion-tech/laneflow".to_string(),
        61,
        role,
        issue_numbers[0],
        "- Delivery PR：#61\n- Related PRs：#62、#63",
    )
    .unwrap();

    assert_eq!(
        args,
        GateEvidenceArgs {
            phase: GateEvidencePhase::G3,
            repo: "illusion-tech/laneflow".to_string(),
            issue: 60,
            delivery_pr: Some(61),
            related_prs: vec![62, 63],
        }
    );
}

#[test]
fn resolves_related_target_as_related_only_g3() {
    let (role, issue_numbers) =
        parse_gate_evidence_target_metadata("- 关联 Issue：#60\n- PR 角色：Related PR").unwrap();
    let args = resolve_gate_evidence_target_args(
        "illusion-tech/laneflow".to_string(),
        62,
        role,
        issue_numbers[0],
        "- Delivery PR：pending\n- Related PRs：#62、#63",
    )
    .unwrap();

    assert_eq!(args, related_only_g3_args());
}

#[test]
fn parses_multiple_targets_and_rejects_role_placeholders() {
    let (role, issue_numbers) =
        parse_gate_evidence_target_metadata("- 关联 Issue：#60、#61\n- PR 角色：Related PR")
            .unwrap();
    assert_eq!(role, GateEvidencePrRole::Related);
    assert_eq!(issue_numbers, vec![60, 61]);

    let duplicate_issue_error =
        parse_gate_evidence_target_metadata("- 关联 Issue：#60、#60\n- PR 角色：Related PR")
            .expect_err("associated Issues must not repeat");
    assert!(duplicate_issue_error.contains("不得包含重复 PR"));

    let role_error = parse_gate_evidence_target_metadata(
        "- 关联 Issue：#60\n- PR 角色：`Delivery PR` / `Related PR`",
    )
    .expect_err("template role placeholder must fail closed");
    assert!(role_error.contains("必须精确为"));

    let duplicate_error = parse_gate_evidence_target_metadata(
        "- 关联 Issue：#60\n- PR 角色：Related PR\n- PR 角色：Delivery PR",
    )
    .expect_err("target role metadata must be unique");
    assert!(duplicate_error.contains("只能包含一个 `PR 角色`"));
}

#[test]
fn discovers_delivery_targets_recorded_for_a_related_pr() {
    let issue_bodies = [
        "- Delivery PR：#61\n- Related PRs：#62、#63",
        "- Delivery PR：pending\n- Related PRs：#62",
        "- Delivery PR：#81\n- Related PRs：#82",
    ];

    assert_eq!(
        discover_g3_evidence_shadow_targets(62, &issue_bodies).unwrap(),
        vec![61, 62]
    );
}

#[test]
fn dependent_target_discovery_fails_closed_on_relevant_template_residue() {
    let issue_bodies = [
        "- Delivery PR：pending / #71 / N/A，原因：模板未清理\n- Related PRs：#62",
        "- Delivery PR：#81\n- Related PRs：pending / #62 / N/A，原因：模板未清理",
    ];

    let error = discover_g3_evidence_shadow_targets(62, &issue_bodies)
        .expect_err("relevant Issue metadata residue must trigger the all-open fallback");

    assert!(error.contains("未清理的互斥模板选项"));
}

#[test]
fn issue_event_target_discovery_is_bounded_to_recorded_prs() {
    let governed_issue = "- Delivery PR：#61\n- Related PRs：#62、#63\n- [x] G0 立项已记录：";
    assert_eq!(
        discover_g3_evidence_shadow_issue_targets(governed_issue).unwrap(),
        vec![61, 62, 63]
    );
    assert!(
        discover_g3_evidence_shadow_issue_targets("ordinary issue without PR metadata")
            .unwrap()
            .is_empty()
    );

    let partial_error = discover_g3_evidence_shadow_issue_targets("- Delivery PR：#61")
        .expect_err("partial governed metadata must trigger the conservative fallback");
    assert!(partial_error.contains("各包含一个"));

    let edited_event = GitHubIssuesEvent {
        issue: GitHubIssuesEventIssue {
            number: 60,
            body: Some("- Delivery PR：#71\n- Related PRs：#72".to_string()),
            pull_request: None,
        },
        changes: GitHubIssuesEventChanges {
            body: Some(GitHubIssuesEventBodyChange {
                from: Some("- Delivery PR：#61\n- Related PRs：#62".to_string()),
            }),
        },
        repository: GitHubIssuesEventRepository {
            full_name: "illusion-tech/laneflow".to_string(),
        },
    };
    assert_eq!(
        discover_g3_evidence_shadow_issue_event_targets(&edited_event).unwrap(),
        vec![61, 62, 71, 72]
    );
}

#[test]
fn delivery_marker_freshness_covers_the_full_associated_pr_set() {
    let args = GateEvidenceArgs {
        phase: GateEvidencePhase::G3,
        repo: "illusion-tech/laneflow".to_string(),
        issue: 60,
        delivery_pr: Some(61),
        related_prs: vec![62, 63],
    };

    assert_eq!(
        relevant_g3_marker_prs(61, &[args]),
        BTreeSet::from([61, 62, 63])
    );
}

#[test]
fn parses_g3_evidence_marker_arguments() {
    assert_eq!(
        parse_g3_evidence_marker_args(&[
            "--repo".to_string(),
            "illusion-tech/laneflow".to_string(),
            "--pr".to_string(),
            "324".to_string(),
            "--comment-id".to_string(),
            "5190249680".to_string(),
        ])
        .unwrap(),
        ("illusion-tech/laneflow".to_string(), 324, 5_190_249_680)
    );
}

#[test]
fn validates_unedited_marker_identity_and_strict_ordering() {
    let marker = GitHubIssueCommentRest {
        body: Some("g3-evidence: changed".to_string()),
        created_at: "2026-08-06T03:30:01Z".to_string(),
        updated_at: "2026-08-06T03:30:01Z".to_string(),
        issue_url: "https://api.github.com/repos/illusion-tech/laneflow/issues/324".to_string(),
    };

    assert!(validate_g3_evidence_marker_comment(&marker, "illusion-tech/laneflow", 324).is_ok());
    assert!(
        validate_marker_is_strictly_later(
            &marker.created_at,
            "2026-08-06T03:30:00Z",
            "current G3 comment"
        )
        .is_ok()
    );
    assert!(
        validate_marker_is_strictly_later(&marker.created_at, "2026-08-06T03:30:01Z", "Issue body")
            .is_err()
    );
    assert!(
        validate_marker_is_strictly_later(
            &marker.created_at,
            "2026-08-06T03:30:00.999999Z",
            "PR body"
        )
        .is_ok()
    );
    assert!(
        validate_marker_is_strictly_later(
            &marker.created_at,
            "2026-08-06T03:30:01.000001Z",
            "PR body"
        )
        .is_err()
    );
    assert!(
        validate_marker_after_activity_timestamp(
            &marker.created_at,
            &marker.created_at,
            true,
            "marker target PR activity",
        )
        .is_ok()
    );
    assert!(
        validate_marker_after_activity_timestamp(
            &marker.created_at,
            &marker.created_at,
            false,
            "Related PR activity",
        )
        .is_err()
    );
    assert!(parse_utc_timestamp_seconds("2026-08-06T03:30:00.Z").is_none());
}

#[test]
fn marker_freshness_covers_related_lifecycle_and_comment_activity() {
    let marker_id = 5_190_249_680;
    let marker_created_at = "2026-08-06T03:30:01Z";
    let mut timeline = vec![
        GitHubTimelineItem {
            id: Some(1),
            event: "closed".to_string(),
            created_at: Some("2026-08-06T03:30:00Z".to_string()),
            updated_at: None,
            submitted_at: None,
            committer: None,
        },
        GitHubTimelineItem {
            id: Some(marker_id),
            event: "commented".to_string(),
            created_at: Some(marker_created_at.to_string()),
            updated_at: Some("2026-08-06T03:30:05Z".to_string()),
            submitted_at: None,
            committer: None,
        },
    ];
    assert!(
        validate_marker_after_timeline(
            marker_id,
            marker_created_at,
            &timeline,
            GitHubTimelineTarget::PullRequest,
            true,
            "Related PR #62",
        )
        .is_ok()
    );

    timeline.push(GitHubTimelineItem {
        id: Some(2),
        event: "reopened".to_string(),
        created_at: Some("2026-08-06T03:30:02Z".to_string()),
        updated_at: None,
        submitted_at: None,
        committer: None,
    });
    let lifecycle_error = validate_marker_after_timeline(
        marker_id,
        marker_created_at,
        &timeline,
        GitHubTimelineTarget::PullRequest,
        true,
        "Related PR #62",
    )
    .expect_err("a later reopen must invalidate an older marker");
    assert!(lifecycle_error.contains("`reopened` timeline event"));

    timeline.pop();
    timeline.push(GitHubTimelineItem {
        id: Some(3),
        event: "commented".to_string(),
        created_at: Some("2026-08-06T03:29:00Z".to_string()),
        updated_at: Some("2026-08-06T03:30:02Z".to_string()),
        submitted_at: None,
        committer: None,
    });
    let comment_error = validate_marker_after_timeline(
        marker_id,
        marker_created_at,
        &timeline,
        GitHubTimelineTarget::PullRequest,
        true,
        "Related PR #62",
    )
    .expect_err("a later comment edit must invalidate an older marker");
    assert!(comment_error.contains("`commented` timeline event"));
}

#[test]
fn rejects_edited_or_wrong_pr_g3_evidence_marker() {
    let edited = GitHubIssueCommentRest {
        body: Some("g3-evidence: changed".to_string()),
        created_at: "2026-08-06T03:30:01Z".to_string(),
        updated_at: "2026-08-06T03:30:02Z".to_string(),
        issue_url: "https://api.github.com/repos/illusion-tech/laneflow/issues/324".to_string(),
    };
    let wrong_pr = GitHubIssueCommentRest {
        updated_at: edited.created_at.clone(),
        issue_url: "https://api.github.com/repos/illusion-tech/laneflow/issues/325".to_string(),
        ..edited.clone()
    };

    assert!(validate_g3_evidence_marker_comment(&edited, "illusion-tech/laneflow", 324).is_err());
    assert!(validate_g3_evidence_marker_comment(&wrong_pr, "illusion-tech/laneflow", 324).is_err());
}

#[test]
fn rejects_unresolved_issue_metadata_placeholders() {
    let delivery_error = resolve_gate_evidence_target_args(
        "illusion-tech/laneflow".to_string(),
        61,
        GateEvidencePrRole::Delivery,
        60,
        "- Delivery PR：pending / #61 / N/A，原因：\n- Related PRs：N/A，原因：无部分交付。",
    )
    .expect_err("mixed Delivery metadata choices must fail closed");
    assert!(delivery_error.contains("具体值必须使用") || delivery_error.contains("模板"));

    let related_error = resolve_gate_evidence_target_args(
        "illusion-tech/laneflow".to_string(),
        62,
        GateEvidencePrRole::Related,
        60,
        "- Delivery PR：pending\n- Related PRs：#62、N/A，原因：无其他 PR",
    )
    .expect_err("mixed Related metadata choices must fail closed");
    assert!(related_error.contains("互斥模板选项"));
}

#[test]
fn rejects_target_role_that_disagrees_with_issue_metadata() {
    let delivery_error = resolve_gate_evidence_target_args(
        "illusion-tech/laneflow".to_string(),
        61,
        GateEvidencePrRole::Delivery,
        60,
        "- Delivery PR：#64\n- Related PRs：#62",
    )
    .expect_err("Delivery target must match Issue metadata");
    assert!(delivery_error.contains("当前 Delivery PR #61"));

    let related_error = resolve_gate_evidence_target_args(
        "illusion-tech/laneflow".to_string(),
        62,
        GateEvidencePrRole::Related,
        60,
        "- Delivery PR：#61\n- Related PRs：#63",
    )
    .expect_err("Related target must be recorded by the Issue");
    assert!(related_error.contains("未记录当前 Related PR #62"));
}

#[test]
fn rejects_g4_without_delivery_pr() {
    let args = vec![
        "g4".to_string(),
        "--repo".to_string(),
        "illusion-tech/laneflow".to_string(),
        "--issue".to_string(),
        "60".to_string(),
        "--related-pr".to_string(),
        "62".to_string(),
    ];

    let error = parse_gate_evidence_args(&args).expect_err("G4 requires a Delivery PR");

    assert!(error.contains("G4 必须指定"));
}

#[test]
fn rejects_related_only_g3_with_multiple_prs() {
    let args = vec![
        "g3".to_string(),
        "--repo".to_string(),
        "illusion-tech/laneflow".to_string(),
        "--issue".to_string(),
        "60".to_string(),
        "--related-pr".to_string(),
        "62".to_string(),
        "--related-pr".to_string(),
        "63".to_string(),
    ];

    let error =
        parse_gate_evidence_args(&args).expect_err("standalone G3 validates one Related PR");

    assert!(error.contains("只能指定一个 Related PR"));
}

#[test]
fn deserializes_gh_project_items_with_top_level_title() {
    let pr: GitHubPullRequest = serde_json::from_str(
        r#"{
            "body": "body",
            "state": "MERGED",
            "isDraft": false,
            "createdAt": "2026-07-10T04:00:00Z",
            "mergedAt": "2026-07-10T05:30:00Z",
            "closingIssuesReferences": [],
            "projectItems": [{
                "status": {"optionId": "6114ac6a", "name": "Done"},
                "title": "LaneFlow"
            }],
            "comments": []
        }"#,
    )
    .expect("current gh pr view projectItems shape should deserialize");

    assert_eq!(pr.project_items[0].title, "LaneFlow");
    assert_eq!(
        pr.project_items[0]
            .status
            .as_ref()
            .map(|status| status.name.as_str()),
        Some("Done")
    );
}

#[test]
fn deserializes_closing_issue_repository_identity() {
    let pr: GitHubPullRequest = serde_json::from_str(
        r#"{
            "body": "body",
            "state": "OPEN",
            "isDraft": false,
            "createdAt": "2026-08-06T08:00:00Z",
            "mergedAt": null,
            "closingIssuesReferences": [{
                "number": 60,
                "repository": {
                    "name": "laneflow",
                    "owner": {"login": "illusion-tech"}
                }
            }],
            "comments": []
        }"#,
    )
    .expect("current gh closingIssuesReferences repository shape should deserialize");

    assert!(issue_reference_matches(
        &pr.closing_issues_references[0],
        "ILLUSION-TECH/LaneFlow",
        60,
    ));
    assert!(!issue_reference_matches(
        &pr.closing_issues_references[0],
        "illusion-tech/other",
        60,
    ));
}

#[test]
fn rejects_duplicate_delivery_and_related_pr() {
    let args = vec![
        "g3".to_string(),
        "--repo".to_string(),
        "illusion-tech/laneflow".to_string(),
        "--issue".to_string(),
        "60".to_string(),
        "--delivery-pr".to_string(),
        "61".to_string(),
        "--related-pr".to_string(),
        "61".to_string(),
    ];

    let error =
        parse_gate_evidence_args(&args).expect_err("delivery PR cannot also be a related PR");

    assert!(error.contains("不能重复"));
}

#[test]
fn accepts_complete_g3_evidence() {
    let issue = issue("OPEN", "In Review");
    let delivery_pr = delivery_pr(None);

    assert!(
        validate_g3_evidence(&gate_args(GateEvidencePhase::G3), &issue, &delivery_pr, &[]).is_ok()
    );
}

#[test]
fn rejects_merged_delivery_as_current_g3_target() {
    let args = gate_args(GateEvidencePhase::G3);
    let delivery_pr = delivery_pr(Some("2026-07-10T05:30:00Z"));

    let error = validate_current_g3_target(&args, Some(&delivery_pr), &[])
        .expect_err("standard G3 must be pre-merge");

    assert!(error.contains("合并前仍为 OPEN 且非 Draft"));
}

#[test]
fn rejects_draft_delivery_as_current_g3_target() {
    let args = gate_args(GateEvidencePhase::G3);
    let mut delivery_pr = delivery_pr(None);
    delivery_pr.is_draft = true;

    let error = validate_current_g3_target(&args, Some(&delivery_pr), &[])
        .expect_err("standard G3 must reject draft PRs");

    assert!(error.contains("非 Draft"));
}

#[test]
fn rejects_closed_related_pr_in_delivery_full_set_g3() {
    let mut args = gate_args(GateEvidencePhase::G3);
    args.related_prs = vec![62];
    let delivery_pr = delivery_pr(None);
    let mut related_pr = related_pr(false);
    related_pr.state = "CLOSED".to_string();

    let error = validate_current_g3_target(&args, Some(&delivery_pr), &[related_pr])
        .expect_err("abandoned Related PR must fail a Delivery full-set G3");

    assert!(error.contains("Related PR #62"));
    assert!(error.contains("CLOSED"));
}

#[test]
fn accepts_merged_related_pr_history_in_delivery_full_set_g3() {
    let mut args = gate_args(GateEvidencePhase::G3);
    args.related_prs = vec![62];
    let delivery_pr = delivery_pr(None);
    let mut related_pr = related_pr(false);
    related_pr.state = "MERGED".to_string();
    related_pr.merged_at = Some("2026-07-10T05:30:00Z".to_string());

    assert!(validate_current_g3_target(&args, Some(&delivery_pr), &[related_pr]).is_ok());
}

#[test]
fn merged_waiver_replay_uses_merge_time_without_reauthorizing_current_targets() {
    let mut historical = related_pr(false);
    historical.state = "MERGED".to_string();
    historical.merged_at = Some("2026-07-10T05:30:00Z".to_string());
    assert_eq!(
        gate_waiver_reference_time(&historical, 2_000_000_000).unwrap(),
        1_783_661_400
    );

    let current = related_pr(false);
    assert_eq!(
        gate_waiver_reference_time(&current, 2_000_000_000).unwrap(),
        2_000_000_000
    );
}

#[test]
fn rejects_merged_related_as_current_related_only_g3_target() {
    let args = related_only_g3_args();
    let mut related_pr = related_pr(false);
    related_pr.state = "MERGED".to_string();
    related_pr.merged_at = Some("2026-07-10T05:30:00Z".to_string());

    let error = validate_current_g3_target(&args, None, &[related_pr])
        .expect_err("Related-only G3 must be pre-merge");

    assert!(error.contains("历史合并证据只能由 G4 复核"));
}

#[test]
fn g4_still_accepts_merged_delivery_as_historical_target() {
    let args = gate_args(GateEvidencePhase::G4);
    let delivery_pr = delivery_pr(Some("2026-07-10T05:30:00Z"));

    assert!(validate_current_g3_target(&args, Some(&delivery_pr), &[]).is_ok());
}

#[test]
fn rejects_closed_issue_as_current_g3_target() {
    let closed_issue = issue("CLOSED", "Done");

    let error = validate_current_g3_issue(GateEvidencePhase::G3, &closed_issue)
        .expect_err("standard G3 must reject a closed Issue");

    assert!(error.contains("仍为 OPEN 的关联 Issue"));
    assert!(validate_current_g3_issue(GateEvidencePhase::G4, &closed_issue).is_ok());
}

#[test]
fn g3_evidence_shadow_workflow_preserves_trusted_ref_boundary() {
    let workflow = include_str!("../../../.github/workflows/g3-evidence-gate.yml");

    assert!(workflow.contains("name: G3 Evidence Gate Shadow"));
    assert!(workflow.contains("pull_request_target:"));
    assert!(!workflow.contains("pull_request_target:\n    branches:"));
    assert!(workflow.contains("issue_comment:\n    types:\n      - created"));
    assert!(workflow.contains("      - edited\n      - deleted\n  issues:"));
    assert!(
        workflow.contains("issues:\n    types:\n      - edited\n      - closed\n      - reopened")
    );
    assert!(workflow.contains("workflow_run:\n    workflows:\n      - External Review Signal"));
    assert!(workflow.contains("workflow_dispatch:"));
    assert!(workflow.contains("g3-evidence-gate-event-${{"));
    assert!(workflow.contains("github.event.workflow_run.pull_requests[0].number"));
    assert!(workflow.contains("github.event.comment.body == 'g3-evidence: changed'"));
    assert!(workflow.contains("github.run_attempt == 1"));
    assert!(workflow.contains("github.event.issue.pull_request != null"));
    assert!(workflow.contains(
        "permissions:\n  contents: read\n  pull-requests: read\n  issues: read\n  checks: write"
    ));
    assert!(workflow.contains("ref: refs/heads/main"));
    assert!(workflow.contains("persist-credentials: false"));
    assert!(workflow.contains("actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"));
    assert!(workflow.contains("check-gate-evidence-target"));
    assert!(workflow.contains("check-g3-shadow-success-eligibility"));
    assert!(workflow.contains("check-g3-evidence-marker"));
    assert!(workflow.contains("resolve-g3-evidence-shadow-targets"));
    assert!(workflow.contains("resolve-g3-evidence-shadow-issue-event-targets"));
    assert!(workflow.contains("--event-path \"$GITHUB_EVENT_PATH\""));
    assert!(workflow.contains("issues)\n              jq -e"));
    assert!(!workflow.contains("issues)\n              pr_numbers=\"$(all_open_prs)\""));
    assert!(workflow.contains("2>\"$resolver_stderr\""));
    assert!(workflow.contains("cancel-in-progress: false"));
    assert!(workflow.contains("Final trusted revalidation:"));
    assert!(workflow.contains("publishing failure on the originally evaluated head"));
    assert!(workflow.contains("Fresh G3 evidence marker required"));
    assert!(workflow.contains("ALLOW_SUCCESS"));
    assert!(workflow.contains("MARKER_COMMENT_ID"));
    assert!(workflow.contains("- closed"));
    assert!(workflow.contains("initial_head"));
    assert!(workflow.contains("final_head"));
    assert!(workflow.contains("final_eligibility"));
    assert!(workflow.contains(".app.slug == \"github-actions\""));
    assert!(!workflow.contains("refs/pull/"));
    assert!(!workflow.contains("secrets."));
    assert!(!workflow.contains("schedule:"));
    assert!(
        !workflow
            .lines()
            .any(|line| line.trim_start() == "pull_request:")
    );
}

#[test]
fn accepts_full_set_g3_with_related_only_assertion() {
    let mut args = gate_args(GateEvidencePhase::G3);
    args.related_prs = vec![62];
    let mut issue = issue("OPEN", "In Review");
    issue.body = issue
        .body
        .replace("Related PRs：N/A，原因：无部分交付。", "Related PRs：#62")
        .replace(
            DELIVERY_G3_URL,
            &format!("{DELIVERY_G3_URL})，[Related G3 评论]({RELATED_G3_URL}"),
        );
    let mut delivery_pr = delivery_pr(None);
    delivery_pr.comments[0] = g3_comment_for_args(DELIVERY_G3_URL, "2026-07-10T05:00:00Z", &args);
    let related_pr = related_pr(false);

    assert!(validate_g3_evidence(&args, &issue, &delivery_pr, &[related_pr]).is_ok());
}

#[test]
fn accepts_related_g3_before_delivery_pr_exists() {
    let args = related_only_g3_args();
    let issue = issue_with_pending_delivery_and_related_g3();
    let related_pr = related_pr_for_args(false, &args);

    assert!(validate_related_g3_evidence(&args, &issue, 62, &related_pr).is_ok());
}

#[test]
fn accepts_current_g3_fields_at_external_review_activation() {
    let args = related_only_g3_args();
    let issue = issue_with_pending_delivery_and_related_g3();
    let mut related_pr = related_pr_for_args(false, &args);
    related_pr.comments[0].created_at = EXTERNAL_REVIEW_G3_ACTIVATION.to_string();
    related_pr.comments[0].body = gate_comment_body(CURRENT_G3_COMMENT_FIELDS, &args);

    assert!(validate_related_g3_evidence(&args, &issue, 62, &related_pr).is_ok());
    assert!(g3_requires_external_review(&related_pr).unwrap());
}

#[test]
fn requires_explicit_codeql_state_after_codeql_activation() {
    let args = related_only_g3_args();
    let issue = issue_with_pending_delivery_and_related_g3();
    let mut related_pr = related_pr_for_args(false, &args);
    related_pr.comments[0].created_at = CODEQL_G3_ACTIVATION.to_string();
    related_pr.comments[0].body = gate_comment_body(CURRENT_G3_COMMENT_FIELDS, &args);

    let error = validate_related_g3_evidence(&args, &issue, 62, &related_pr)
        .expect_err("CodeQL activation must require an explicit field");
    assert!(error.contains("- CodeQL："));

    related_pr.comments[0]
        .body
        .push_str("\n- CodeQL：`pass`，https://github.com/illusion-tech/laneflow/runs/1\n");
    assert!(validate_related_g3_evidence(&args, &issue, 62, &related_pr).is_ok());
}

#[test]
fn extracts_one_recorded_codeql_evidence_url() {
    assert_eq!(
        codeql_evidence_url(
            "- CodeQL：`pass`，https://github.com/illusion-tech/laneflow/runs/92519398933。"
        ),
        Ok("https://github.com/illusion-tech/laneflow/runs/92519398933")
    );
    assert!(codeql_evidence_url("- CodeQL：`pass`").is_err());
    assert!(
        codeql_evidence_url(
            "- CodeQL：https://github.com/a/b/runs/1 https://github.com/a/b/runs/2"
        )
        .is_err()
    );
    assert_eq!(
        codeql_evidence_url(
            "- CodeQL：`pass`，https://github.com/illusion-tech/laneflow/runs/925193989330"
        ),
        Ok("https://github.com/illusion-tech/laneflow/runs/925193989330")
    );
    assert!(
        codeql_evidence_matches(
            "- CodeQL：`pass`，https://github.com/illusion-tech/laneflow/runs/92519398933",
            "https://github.com/illusion-tech/laneflow/runs/92519398933"
        )
        .unwrap()
    );
    assert!(
        !codeql_evidence_matches(
            "- CodeQL：`pass`，https://github.com/illusion-tech/laneflow/runs/925193989330",
            "https://github.com/illusion-tech/laneflow/runs/92519398933"
        )
        .unwrap()
    );
    assert!(!codeql_evidence_matches(
        "- CodeQL：`pass`，https://github.com/illusion-tech/laneflow/runs/92519398933?attempt=2",
        "https://github.com/illusion-tech/laneflow/runs/92519398933"
    )
    .unwrap());
}

#[test]
fn parses_one_exact_codeql_state_and_rejects_contradictions() {
    assert_eq!(
        codeql_state(
            "- CodeQL：`pass`，https://github.com/illusion-tech/laneflow/runs/92519398933"
        ),
        Ok(crate::codeql::CodeQlState::Pass)
    );
    assert!(
        codeql_state(
            "- CodeQL：`pass`，但另记 `failed`，https://github.com/illusion-tech/laneflow/runs/1"
        )
        .is_err()
    );
    assert!(
        codeql_state("- CodeQL：状态 pass，https://github.com/illusion-tech/laneflow/runs/1")
            .is_err()
    );
}

#[test]
fn codeql_completion_must_not_follow_the_append_only_g3_comment() {
    assert!(
        validate_codeql_completion_order(
            "Delivery PR",
            "2026-08-07T10:00:00Z",
            Some("2026-08-07T10:00:01Z")
        )
        .is_err()
    );
    assert!(
        validate_codeql_completion_order(
            "Delivery PR",
            "2026-08-07T10:00:01Z",
            Some("2026-08-07T10:00:01Z")
        )
        .is_ok()
    );
}

#[test]
fn codeql_activation_uses_numeric_timestamp_ordering() {
    assert!(!codeql_g3_active("2026-08-07T23:59:59.999Z").unwrap());
    assert!(codeql_g3_active("2026-08-08T00:00:00Z").unwrap());
    assert!(codeql_g3_active("2026-08-08T00:00:00.123Z").unwrap());
    assert!(codeql_g3_active("invalid").is_err());
}

#[test]
fn rejects_current_g3_without_shadow_evidence_field() {
    let args = related_only_g3_args();
    let mut related_pr = related_pr_for_args(false, &args);
    related_pr.comments[0].created_at = EXTERNAL_REVIEW_G3_ACTIVATION.to_string();
    related_pr.comments[0].body = gate_comment_body(CURRENT_G3_COMMENT_FIELDS, &args);

    assert!(
        validate_gate_evidence_target_pr(
            "illusion-tech/laneflow",
            GateEvidencePhase::G4,
            &related_pr,
            GateEvidencePrRole::Related,
            &[60],
        )
        .is_ok()
    );
    let error = validate_gate_evidence_target_pr(
        "illusion-tech/laneflow",
        GateEvidencePhase::G3,
        &related_pr,
        GateEvidencePrRole::Related,
        &[60],
    )
    .expect_err("target/shadow G3 must record the shadow evidence boundary");

    assert!(error.contains("G3 Evidence Gate Shadow"));
}

#[test]
fn accepts_only_unique_populated_shadow_evidence_choices() {
    for value in [
        "Check URL：https://github.com/illusion-tech/laneflow/actions/runs/1",
        "R1 non-required：source App 仍为 github-actions，仅作 telemetry",
        "候选 workflow bootstrap：尚未合入 main，不能用于本 PR 自批",
    ] {
        assert!(
            validate_g3_evidence_shadow_comment_field(&format!(
                "{G3_EVIDENCE_SHADOW_COMMENT_FIELD}{value}"
            ))
            .is_ok()
        );
    }

    for body in [
        G3_EVIDENCE_SHADOW_COMMENT_FIELD.to_string(),
        format!(
            "说明中嵌入 {G3_EVIDENCE_SHADOW_COMMENT_FIELD}Check URL：https://github.com/example"
        ),
        format!("{G3_EVIDENCE_SHADOW_COMMENT_FIELD}Check URL：https://github.com/example"),
        format!("{G3_EVIDENCE_SHADOW_COMMENT_FIELD}稍后补充"),
        format!(
            "{G3_EVIDENCE_SHADOW_COMMENT_FIELD}R1 non-required：原因一\n{G3_EVIDENCE_SHADOW_COMMENT_FIELD}R1 non-required：原因二"
        ),
    ] {
        assert!(validate_g3_evidence_shadow_comment_field(&body).is_err());
    }
}

#[test]
fn rejects_legacy_g3_fields_after_external_review_activation() {
    let args = related_only_g3_args();
    let issue = issue_with_pending_delivery_and_related_g3();
    let mut related_pr = related_pr_for_args(false, &args);
    related_pr.comments[0].created_at = EXTERNAL_REVIEW_G3_ACTIVATION.to_string();

    let error = validate_related_g3_evidence(&args, &issue, 62, &related_pr)
        .expect_err("current G3 must include external-review fields");

    assert!(error.contains("- Gate 结果："));
    assert!(error.contains("- Rollout phase："));
    assert!(error.contains("- Current head："));
}

#[test]
fn parses_explicit_current_g3_results() {
    assert_eq!(
        parse_g3_result("- Gate 结果：`G3 Pass`").unwrap(),
        G3Result::Pass
    );
    assert_eq!(
        parse_g3_result("- Gate 结果：`G3 Waived`").unwrap(),
        G3Result::Waived
    );
    assert_eq!(
        parse_g3_result("- Gate 结果：`R0-R1 bootstrap`").unwrap(),
        G3Result::Bootstrap
    );
    assert!(parse_g3_result("- Gate 结果：pending").is_err());
    assert!(parse_g3_result("- Gate 结果：`G3 Pass`\n- Gate 结果：`G3 Waived`").is_err());
    assert!(validate_g3_shadow_success_result(G3Result::Pass).is_ok());
    assert!(validate_g3_shadow_success_result(G3Result::Bootstrap).is_ok());
    assert!(validate_g3_shadow_success_result(G3Result::Waived).is_err());
}

#[test]
fn waived_full_set_member_cannot_receive_shadow_success() {
    let mut delivery = delivery_pr(None);
    delivery.comments[0].body = "- Gate 结果：`G3 Pass`".to_string();
    let mut related = related_pr(false);
    related.comments[0].body = "- Gate 结果：`G3 Waived`".to_string();

    assert!(validate_g3_shadow_success_pr(&delivery, "Delivery PR #61").is_ok());
    let error = validate_g3_shadow_success_pr(&related, "Related PR #62")
        .expect_err("a waived Related member must keep Delivery shadow non-success");
    assert!(error.contains("Related PR #62"));
    assert!(error.contains("不得发布 success"));
}

#[test]
fn parses_reference_style_structured_gate_waiver() {
    let mut comment = GitHubComment {
        url: RELATED_G3_URL.to_string(),
        body: r##"- Gate 结果：`G3 Waived`
- 例外：`G3 Waived`；结构化 waiver `waiver-60-1`；证据：[批准记录][waiver-evidence]。
<!-- external-review-waiver:v1
{
  "schemaVersion": 1,
  "id": "waiver-60-1",
  "exceptionType": "provider_platform_outage",
  "currentHeadOid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "currentBaseOid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  "reason": "all configured providers unavailable",
  "evidenceRefs": ["waiver-evidence"],
  "risk": "review coverage unavailable",
  "acceptanceBoundary": "metadata-only governance change",
  "expiresAt": "2026-07-24T17:00:00Z",
  "followUpIssue": "#60",
  "cleanupOwner": "wangzishi",
  "authorizedBy": "wangzishi"
}
-->

[waiver-evidence]: https://github.com/illusion-tech/laneflow/issues/60#issuecomment-1"##
            .to_string(),
        author: Some(GitHubActor {
            login: "wangzishi".to_string(),
        }),
        created_at: "2026-07-24T16:00:00Z".to_string(),
        includes_created_edit: false,
    };
    let now = parse_utc_timestamp_seconds("2026-07-24T16:30:00Z").unwrap();
    let waiver = parse_gate_waiver(&comment, 60, now).unwrap();

    assert_eq!(waiver.id, "waiver-60-1");
    assert_eq!(waiver.exception_type, "provider_platform_outage");
    assert_eq!(
        waiver.evidence_urls,
        vec!["https://github.com/illusion-tech/laneflow/issues/60#issuecomment-1".to_string()]
    );
    comment.author = Some(GitHubActor {
        login: "untrusted-contributor".to_string(),
    });
    comment.body = comment.body.replace(
        r#""authorizedBy": "wangzishi""#,
        r#""authorizedBy": "untrusted-contributor""#,
    );
    let error =
        parse_gate_waiver(&comment, 60, now).expect_err("waiver author must be a trusted G3 Owner");
    assert!(error.contains("不在 trusted G3 Owner allowlist"));
}

#[test]
fn parses_one_structured_gate_waiver_per_associated_issue() {
    let waiver_60 = r##"<!-- external-review-waiver:v1
{
  "schemaVersion": 1,
  "id": "waiver-60-1",
  "exceptionType": "provider_platform_outage",
  "currentHeadOid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "currentBaseOid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  "reason": "all configured providers unavailable",
  "evidenceRefs": ["waiver-evidence-60"],
  "risk": "review coverage unavailable",
  "acceptanceBoundary": "metadata-only governance change",
  "expiresAt": "2026-07-24T17:00:00Z",
  "followUpIssue": "#60",
  "cleanupOwner": "wangzishi",
  "authorizedBy": "wangzishi"
}
-->"##;
    let waiver_61 = r##"<!-- external-review-waiver:v1
{
  "schemaVersion": 1,
  "id": "waiver-61-1",
  "exceptionType": "provider_platform_outage",
  "currentHeadOid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "currentBaseOid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  "reason": "all configured providers unavailable",
  "evidenceRefs": ["waiver-evidence-61"],
  "risk": "review coverage unavailable",
  "acceptanceBoundary": "metadata-only governance change",
  "expiresAt": "2026-07-24T17:00:00Z",
  "followUpIssue": "#61",
  "cleanupOwner": "wangzishi",
  "authorizedBy": "wangzishi"
}
-->"##;
    let mut comment = GitHubComment {
        url: RELATED_G3_URL.to_string(),
        body: format!(
            r##"- Gate 结果：`G3 Waived`
- 例外：`G3 Waived`；Issue #60 证据：[批准记录 60][waiver-evidence-60]；Issue #61 证据：[批准记录 61][waiver-evidence-61]。
{waiver_60}
{waiver_61}

[waiver-evidence-60]: https://github.com/illusion-tech/laneflow/issues/60#issuecomment-1
[waiver-evidence-61]: https://github.com/illusion-tech/laneflow/issues/61#issuecomment-2"##
        ),
        author: Some(GitHubActor {
            login: "wangzishi".to_string(),
        }),
        created_at: "2026-07-24T16:00:00Z".to_string(),
        includes_created_edit: false,
    };
    let now = parse_utc_timestamp_seconds("2026-07-24T16:30:00Z").unwrap();

    assert_eq!(
        parse_gate_waiver(&comment, 60, now).unwrap().id,
        "waiver-60-1"
    );
    assert_eq!(
        parse_gate_waiver(&comment, 61, now).unwrap().id,
        "waiver-61-1"
    );
    let declared_issues = [60, 61].into_iter().collect::<BTreeSet<_>>();
    assert!(validate_gate_waiver_record_set(&comment, &declared_issues).is_ok());
    let incomplete_associations = [60].into_iter().collect::<BTreeSet<_>>();
    let extra_error = validate_gate_waiver_record_set(&comment, &incomplete_associations)
        .expect_err("waiver records for undeclared Issues must fail closed");
    assert!(extra_error.contains("必须与 `关联 Issue` 精确一致"));

    comment.body = comment.body.replace(
        r##""followUpIssue": "#61""##,
        r##""followUpIssue": "#060""##,
    );
    let noncanonical_error = parse_gate_waiver(&comment, 60, now)
        .expect_err("non-canonical per-Issue waiver numbers must fail closed");
    assert!(noncanonical_error.contains("无前导零"));

    comment.body = comment.body.replace(
        r##""followUpIssue": "#060""##,
        r##""followUpIssue": "#60""##,
    );
    let error = parse_gate_waiver(&comment, 60, now)
        .expect_err("duplicate per-Issue waiver records must fail closed");
    assert!(error.contains("每个 Issue 只能包含一个"));
}

#[test]
fn rejects_expired_structured_gate_waiver() {
    let comment = GitHubComment {
        url: RELATED_G3_URL.to_string(),
        body: r##"- Gate 结果：`G3 Waived`
- 例外：`G3 Waived`；证据：[批准记录][waiver-evidence]。
<!-- external-review-waiver:v1
{
  "schemaVersion": 1,
  "id": "waiver-60-1",
  "exceptionType": "provider_platform_outage",
  "currentHeadOid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "currentBaseOid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  "reason": "all configured providers unavailable",
  "evidenceRefs": ["waiver-evidence"],
  "risk": "review coverage unavailable",
  "acceptanceBoundary": "metadata-only governance change",
  "expiresAt": "2026-07-24T17:00:00Z",
  "followUpIssue": "#60",
  "cleanupOwner": "wangzishi",
  "authorizedBy": "wangzishi"
}
-->

[waiver-evidence]: https://github.com/illusion-tech/laneflow/issues/60#issuecomment-1"##
            .to_string(),
        author: Some(GitHubActor {
            login: "wangzishi".to_string(),
        }),
        created_at: "2026-07-24T16:00:00Z".to_string(),
        includes_created_edit: false,
    };
    let after_expiry = parse_utc_timestamp_seconds("2026-07-24T17:00:01Z").unwrap();
    let error =
        parse_gate_waiver(&comment, 60, after_expiry).expect_err("expired waiver must fail closed");

    assert!(error.contains("已过期"));
}

#[test]
fn rejects_edited_current_g3_comment() {
    let args = related_only_g3_args();
    let issue = issue_with_pending_delivery_and_related_g3();
    let mut related_pr = related_pr_for_args(false, &args);
    related_pr.comments[0].created_at = EXTERNAL_REVIEW_G3_ACTIVATION.to_string();
    related_pr.comments[0].body = gate_comment_body(CURRENT_G3_COMMENT_FIELDS, &args);
    related_pr.comments[0].includes_created_edit = true;

    let error = validate_related_g3_evidence(&args, &issue, 62, &related_pr)
        .expect_err("current G3 must be append-only");

    assert!(error.contains("创建后被编辑"));
}

#[test]
fn accepts_related_g3_with_reference_style_permalinks() {
    let args = related_only_g3_args();
    let mut issue = issue_with_pending_delivery_and_related_g3();
    issue.body = issue.body.replace(
        &format!("[Related G3 评论]({RELATED_G3_URL})"),
        "[Related G3 评论][related-g3]",
    );
    issue
        .body
        .push_str(&format!("\n\n[related-g3]: {RELATED_G3_URL}\n"));
    let mut related_pr = related_pr_for_args(false, &args);
    related_pr.body = related_pr
        .body
        .replace(&format!("[G3 评论]({RELATED_G3_URL})"), "[G3 评论][g3]");
    related_pr
        .body
        .push_str(&format!("\n\n[g3]: {RELATED_G3_URL}\n"));

    assert!(validate_related_g3_evidence(&args, &issue, 62, &related_pr).is_ok());
}

#[test]
fn rejects_related_g3_when_issue_g3_is_already_completed() {
    let args = related_only_g3_args();
    let mut issue = issue_with_pending_delivery_and_related_g3();
    issue.body = issue
        .body
        .replace("- [ ] G3 合并判断已记录：", "- [x] G3 合并判断已记录：");
    let related_pr = related_pr_for_args(false, &args);

    let error = validate_related_g3_evidence(&args, &issue, 62, &related_pr)
        .expect_err("Related-only G3 requires the Issue ledger to remain pending");

    assert!(error.contains("缺少未勾选的 `G3` Gate Ledger 项"));
}

#[test]
fn rejects_related_g3_missing_from_issue_metadata() {
    let args = related_only_g3_args();
    let mut issue = issue_with_pending_delivery_and_related_g3();
    issue.body = issue.body.replace("Related PRs：#62", "Related PRs：#63");
    let related_pr = related_pr_for_args(false, &args);

    let error = validate_related_g3_evidence(&args, &issue, 62, &related_pr)
        .expect_err("Related PR must be recorded in Issue metadata");

    assert!(error.contains("未记录 Related PR #62"));
}

#[test]
fn rejects_related_g3_without_issue_permalink() {
    let args = related_only_g3_args();
    let mut issue = issue_with_pending_delivery_and_related_g3();
    issue.body = issue.body.replace(RELATED_G3_URL, DELIVERY_G3_URL);
    let related_pr = related_pr_for_args(false, &args);

    let error = validate_related_g3_evidence(&args, &issue, 62, &related_pr)
        .expect_err("Issue G3 ledger must link the Related PR comment");

    assert!(error.contains("未回链 Related PR #62"));
}

#[test]
fn g4_invocation_still_validates_pr_comment_as_g3() {
    let issue = issue("OPEN", "In Review");
    let delivery_pr = delivery_pr(None);

    assert!(
        validate_g3_evidence(&gate_args(GateEvidencePhase::G4), &issue, &delivery_pr, &[]).is_ok()
    );
}

#[test]
fn rejects_g3_assertion_that_is_still_pending() {
    let issue = issue("OPEN", "In Review");
    let mut delivery_pr = delivery_pr(None);
    delivery_pr.comments[0].body = delivery_pr.comments[0]
        .body
        .replace("` 已通过。", "` 待运行。");

    let error = validate_g3_evidence(&gate_args(GateEvidencePhase::G3), &issue, &delivery_pr, &[])
        .expect_err("pending G3 assertion must not pass");

    assert!(error.contains("明确记录 `已通过`"));
}

#[test]
fn rejects_g3_assertion_with_mismatched_command_arguments() {
    let issue = issue("OPEN", "In Review");
    let mut delivery_pr = delivery_pr(None);
    delivery_pr.comments[0].body = delivery_pr.comments[0]
        .body
        .replace("--delivery-pr 61`", "--delivery-pr 99`");

    let error = validate_g3_evidence(&gate_args(GateEvidencePhase::G3), &issue, &delivery_pr, &[])
        .expect_err("G3 assertion arguments must match the current invocation");

    assert!(error.contains("命令与当前参数不一致"));
}

#[test]
fn accepts_one_g3_assertion_per_associated_issue() {
    let first_args = related_only_g3_args();
    let second_args = GateEvidenceArgs {
        phase: GateEvidencePhase::G3,
        repo: "illusion-tech/laneflow".to_string(),
        issue: 61,
        delivery_pr: None,
        related_prs: vec![62],
    };
    let body = format!(
        "{GATE_ASSERTION_PREFIX}`{}` 已通过。\n{GATE_ASSERTION_PREFIX}`{}` 已通过。",
        expected_gate_command(&first_args, GateEvidencePhase::G3),
        expected_gate_command(&second_args, GateEvidencePhase::G3)
    );

    assert!(
        validate_gate_assertion(&body, "multi-Issue G3", &first_args, GateEvidencePhase::G3)
            .is_ok()
    );
    assert!(
        validate_gate_assertion(&body, "multi-Issue G3", &second_args, GateEvidencePhase::G3)
            .is_ok()
    );
    assert!(
        validate_gate_assertion_set(
            &body,
            "multi-Issue G3",
            &[first_args.clone(), second_args.clone()],
            GateEvidencePhase::G3
        )
        .is_ok()
    );

    let unrelated_args = GateEvidenceArgs {
        issue: 999,
        ..second_args.clone()
    };
    let body_with_extra = format!(
        "{body}\n{GATE_ASSERTION_PREFIX}`{}` 已通过。",
        expected_gate_command(&unrelated_args, GateEvidencePhase::G3)
    );
    let error = validate_gate_assertion_set(
        &body_with_extra,
        "multi-Issue G3",
        &[first_args, second_args],
        GateEvidencePhase::G3,
    )
    .expect_err("undeclared or mismatched assertion commands must fail closed");
    assert!(error.contains("完整 `Gate 断言` 命令集合"));
}

#[test]
fn rejects_multiple_g4_assertions() {
    let args = gate_args(GateEvidencePhase::G4);
    let command = expected_gate_command(&args, GateEvidencePhase::G4);
    let body = format!(
        "{GATE_ASSERTION_PREFIX}`{command}` 已通过。\n{GATE_ASSERTION_PREFIX}`{command} --related-pr 62` 已通过。"
    );

    let error = validate_gate_assertion(&body, "Issue G4", &args, GateEvidencePhase::G4)
        .expect_err("G4 must keep one full-set assertion");

    assert!(error.contains("G4 只能包含一条"));
}

#[test]
fn rejects_g3_when_issue_does_not_link_delivery_comment() {
    let mut issue = issue("OPEN", "In Review");
    issue.body = issue.body.replace(DELIVERY_G3_URL, ISSUE_G4_URL);
    let delivery_pr = delivery_pr(None);

    let error = validate_g3_evidence(&gate_args(GateEvidencePhase::G3), &issue, &delivery_pr, &[])
        .expect_err("Issue G3 must link the delivery PR G3 comment");

    assert!(error.contains("未回链"));
}

#[test]
fn ignores_acceptance_items_that_start_with_gate_names() {
    let body = format!(
        "- [x] G3/G4 收口流程具有可执行的远端状态断言。\n- [x] G3 合并判断已记录：[Delivery G3 评论]({DELIVERY_G3_URL})"
    );

    assert_eq!(
        completed_gate_permalink(&body, "G3"),
        Ok(DELIVERY_G3_URL.to_string())
    );
}

#[test]
fn resolves_reference_style_gate_permalink_from_the_completed_line() {
    let body = format!(
        "- [x] G3 合并判断已记录：[Delivery G3 评论][delivery-g3]\n\n[unrelated]: {ISSUE_G4_URL}\n[delivery-g3]: {DELIVERY_G3_URL}\n"
    );

    assert_eq!(
        completed_gate_permalink(&body, "G3"),
        Ok(DELIVERY_G3_URL.to_string())
    );
}

#[test]
fn resolves_angle_bracketed_reference_style_gate_permalink() {
    let body = format!(
        "- [x] G3 合并判断已记录：[Delivery G3 评论][delivery-g3]\n\n[delivery-g3]: <{DELIVERY_G3_URL}>\n"
    );

    assert_eq!(
        completed_gate_permalink(&body, "G3"),
        Ok(DELIVERY_G3_URL.to_string())
    );
}

#[test]
fn rejects_unreferenced_permalink_definition() {
    let body = format!("- [x] G3 合并判断已记录：待补链接\n\n[delivery-g3]: {DELIVERY_G3_URL}\n");

    let error = completed_gate_permalink(&body, "G3")
        .expect_err("an unreferenced definition is not Gate evidence");

    assert!(error.contains("inline 或 reference-style"));
}

#[test]
fn rejects_related_pr_that_closes_the_delivery_issue() {
    let mut issue = issue("OPEN", "In Review");
    issue.body = issue
        .body
        .replace(
            DELIVERY_G3_URL,
            &format!("{DELIVERY_G3_URL})，[Related G3 评论]({RELATED_G3_URL}"),
        )
        .replace("Related PRs：N/A，原因：无部分交付。", "Related PRs：#62");
    let mut delivery_pr = delivery_pr(None);
    let related_pr = related_pr(true);
    let mut args = gate_args(GateEvidencePhase::G3);
    args.related_prs = vec![62];
    delivery_pr.comments[0] = g3_comment_for_args(DELIVERY_G3_URL, "2026-07-10T05:00:00Z", &args);

    let error = validate_g3_evidence(&args, &issue, &delivery_pr, &[related_pr])
        .expect_err("Related PR cannot close the delivery Issue");

    assert!(error.contains("不得以 closing keyword"));
}

#[test]
fn rejects_related_pr_arguments_that_do_not_match_issue_metadata() {
    let issue = issue("OPEN", "In Review");
    let mut delivery_pr = delivery_pr(None);
    let related_pr = related_pr(false);
    let mut args = gate_args(GateEvidencePhase::G3);
    args.related_prs = vec![62];
    delivery_pr.comments[0] = g3_comment_for_args(DELIVERY_G3_URL, "2026-07-10T05:00:00Z", &args);

    let error = validate_g3_evidence(&args, &issue, &delivery_pr, &[related_pr])
        .expect_err("Related PR arguments must match Issue metadata");

    assert!(error.contains("字段与命令参数不一致"));
}

#[test]
fn accepts_complete_g4_evidence() {
    let issue = issue("OPEN", "Done");
    let delivery_pr = delivery_pr(Some("2026-07-10T05:30:00Z"));

    assert!(
        validate_g4_evidence(&gate_args(GateEvidencePhase::G4), &issue, &delivery_pr, &[]).is_ok()
    );
}

#[test]
fn accepts_structured_g4_recovery_for_late_related_pr() {
    let (args, issue, delivery_pr, related_pr) = late_related_recovery_fixture();

    assert!(validate_g4_g3_full_set_recovery(&args, &issue, &delivery_pr, &[related_pr]).is_ok());
}

#[test]
fn rejects_g4_recovery_when_related_pr_predates_delivery_merge() {
    let (args, issue, delivery_pr, mut related_pr) = late_related_recovery_fixture();
    related_pr.created_at = "2026-07-10T05:29:59Z".to_string();

    let error = validate_g4_g3_full_set_recovery(&args, &issue, &delivery_pr, &[related_pr])
        .expect_err("late Related PR must be created after Delivery merge");

    assert!(error.contains("必须在 Delivery PR 合并后创建"));
}

#[test]
fn rejects_edited_g4_recovery_comment() {
    let (args, mut issue, delivery_pr, related_pr) = late_related_recovery_fixture();
    issue.comments[0].includes_created_edit = true;

    let error = validate_g4_g3_full_set_recovery(&args, &issue, &delivery_pr, &[related_pr])
        .expect_err("recovery evidence must remain append-only");

    assert!(error.contains("创建后被编辑"));
}

#[test]
fn rejects_edited_legacy_delivery_g3_during_recovery() {
    let (args, issue, mut delivery_pr, related_pr) = late_related_recovery_fixture();
    delivery_pr.comments[0].includes_created_edit = true;

    let error = validate_g4_g3_full_set_recovery(&args, &issue, &delivery_pr, &[related_pr])
        .expect_err("recovery must reject edits even for a legacy Delivery G3");

    assert!(error.contains("Delivery PR original G3 comment 在创建后被编辑"));
}

#[test]
fn rejects_timestamp_equal_delivery_g3_during_recovery() {
    let (args, issue, mut delivery_pr, related_pr) = late_related_recovery_fixture();
    delivery_pr.comments[0].created_at = "2026-07-10T05:30:00Z".to_string();

    let error = validate_g4_g3_full_set_recovery(&args, &issue, &delivery_pr, &[related_pr])
        .expect_err("the original Delivery G3 must strictly predate its merge");

    assert!(error.contains("必须严格早于 Delivery merge"));
}

#[test]
fn rejects_edited_related_g3_during_recovery() {
    let (args, issue, delivery_pr, mut related_pr) = late_related_recovery_fixture();
    related_pr.comments[0].includes_created_edit = true;

    let error = validate_g4_g3_full_set_recovery(&args, &issue, &delivery_pr, &[related_pr])
        .expect_err("recovery must reject edits to every Related G3");

    assert!(error.contains("Related PR #62 G3 comment 在创建后被编辑"));
}

#[test]
fn rejects_g4_recovery_without_structured_record() {
    let (args, mut issue, delivery_pr, related_pr) = late_related_recovery_fixture();
    issue.comments[0] = g4_comment_for_args(ISSUE_G4_URL, "2026-07-10T06:00:00Z", &args);

    let error = validate_g4_g3_full_set_recovery(&args, &issue, &delivery_pr, &[related_pr])
        .expect_err("recovery requires a structured record");

    assert!(error.contains("必须包含且只包含一个"));
}

#[test]
fn late_related_pr_cannot_take_edited_legacy_strict_shortcut() {
    let (args, mut issue, mut delivery_pr, related_pr) = late_related_recovery_fixture();
    issue.comments[0] = g4_comment_for_args(ISSUE_G4_URL, "2026-07-10T06:00:00Z", &args);
    let mut strict_g3_args = gate_args(GateEvidencePhase::G3);
    strict_g3_args.related_prs = vec![62];
    delivery_pr.comments[0] =
        g3_comment_for_args(DELIVERY_G3_URL, "2026-07-10T05:00:00Z", &strict_g3_args);
    delivery_pr.comments[0].includes_created_edit = true;

    assert!(
        validate_g3_evidence(
            &args,
            &issue,
            &delivery_pr,
            std::slice::from_ref(&related_pr),
        )
        .is_ok(),
        "legacy strict validation alone demonstrates the bypass precondition"
    );
    let error = validate_gate_g3_evidence(
        &args,
        &issue,
        &delivery_pr,
        std::slice::from_ref(&related_pr),
    )
    .expect_err("a late Related PR must force structured recovery");

    assert!(error.contains("必须包含且只包含一个"));
}

#[test]
fn rejects_timestamp_equal_related_pr_boundary() {
    let (args, issue, delivery_pr, mut related_pr) = late_related_recovery_fixture();
    related_pr.created_at = "2026-07-10T05:30:00Z".to_string();

    let error = validate_gate_g3_evidence(
        &args,
        &issue,
        &delivery_pr,
        std::slice::from_ref(&related_pr),
    )
    .expect_err("timestamp equality is ambiguous at GitHub's reported precision");

    assert!(error.contains("同秒"));
}

#[test]
fn strict_g4_rejects_inapplicable_recovery_marker() {
    let args = gate_args(GateEvidencePhase::G4);
    let mut issue = issue("OPEN", "Done");
    issue.comments[0]
        .body
        .push_str("\n<!-- g3-full-set-recovery:v1\n{}\n-->");
    let delivery_pr = delivery_pr(Some("2026-07-10T05:30:00Z"));

    let error = validate_gate_g3_evidence(&args, &issue, &delivery_pr, &[])
        .expect_err("strict G4 must reject an inapplicable recovery record");

    assert!(error.contains("不存在 late Related PR"));
}

#[test]
fn rejects_g4_recovery_when_final_related_set_mismatches_record() {
    let (mut args, mut issue, delivery_pr, related_pr) = late_related_recovery_fixture();
    args.related_prs = vec![62, 63];
    issue.body = issue
        .body
        .replace("Related PRs：#62", "Related PRs：#62、#63");

    let error = validate_g4_g3_full_set_recovery(&args, &issue, &delivery_pr, &[related_pr])
        .expect_err("recovery record must match the final Related PR set");

    assert!(error.contains("必须按顺序等于最终 Related PR 参数"));
}

#[test]
fn rejects_g4_recovery_with_untrusted_author() {
    let (args, mut issue, delivery_pr, related_pr) = late_related_recovery_fixture();
    issue.comments[0].author = Some(GitHubActor {
        login: "untrusted-contributor".to_string(),
    });
    issue.comments[0].body = issue.comments[0].body.replace(
        r#""authorizedBy": "wangzishi""#,
        r#""authorizedBy": "untrusted-contributor""#,
    );

    let error = validate_g4_g3_full_set_recovery(&args, &issue, &delivery_pr, &[related_pr])
        .expect_err("recovery author must be a trusted G3 Owner");

    assert!(error.contains("不在 trusted G3 Owner allowlist"));
}

#[test]
fn rejects_g4_recovery_outside_g4_phase() {
    let (mut args, issue, delivery_pr, related_pr) = late_related_recovery_fixture();
    args.phase = GateEvidencePhase::G3;

    let error = validate_g4_g3_full_set_recovery(&args, &issue, &delivery_pr, &[related_pr])
        .expect_err("recovery is a G4-only path");

    assert!(error.contains("只允许用于 G4"));
}

#[test]
fn rejects_g4_assertion_that_is_still_pending() {
    let mut issue = issue("OPEN", "Done");
    issue.comments[0].body = issue.comments[0]
        .body
        .replace("` 已通过。", "` 待 body 回链后运行。");
    let delivery_pr = delivery_pr(Some("2026-07-10T05:30:00Z"));

    let error = validate_g4_evidence(&gate_args(GateEvidencePhase::G4), &issue, &delivery_pr, &[])
        .expect_err("pending G4 assertion must not pass");

    assert!(error.contains("明确记录 `已通过`"));
}

#[test]
fn rejects_g4_assertion_with_mismatched_command_arguments() {
    let mut issue = issue("OPEN", "Done");
    issue.comments[0].body = issue.comments[0]
        .body
        .replace("--delivery-pr 61`", "--delivery-pr 99`");
    let delivery_pr = delivery_pr(Some("2026-07-10T05:30:00Z"));

    let error = validate_g4_evidence(&gate_args(GateEvidencePhase::G4), &issue, &delivery_pr, &[])
        .expect_err("G4 assertion arguments must match the current invocation");

    assert!(error.contains("命令与当前参数不一致"));
}

#[test]
fn rejects_g4_comment_created_before_merge() {
    let mut issue = issue("OPEN", "Done");
    issue.comments[0].created_at = "2026-07-10T05:00:00Z".to_string();
    let delivery_pr = delivery_pr(Some("2026-07-10T05:30:00Z"));

    let error = validate_g4_evidence(&gate_args(GateEvidencePhase::G4), &issue, &delivery_pr, &[])
        .expect_err("G4 comment must be created after merge");

    assert!(error.contains("早于最后一个关联 PR"));
}

#[test]
fn rejects_g4_when_delivery_pr_is_not_project_done() {
    let issue = issue("OPEN", "Done");
    let mut delivery_pr = delivery_pr(Some("2026-07-10T05:30:00Z"));
    delivery_pr.project_items[0].status = Some(ProjectStatus {
        name: "In Review".to_string(),
    });

    let error = validate_g4_evidence(&gate_args(GateEvidencePhase::G4), &issue, &delivery_pr, &[])
        .expect_err("delivery PR must be Project Done before G4");

    assert!(error.contains("Delivery PR 尚未处于 LaneFlow Project 的 Done"));
}
