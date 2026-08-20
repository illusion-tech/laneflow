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
fn recovers_dependabot_target_metadata_from_canonical_g3_assertions() {
    let first = GateEvidenceArgs {
        phase: GateEvidencePhase::G3,
        repo: "illusion-tech/laneflow".to_string(),
        issue: 325,
        delivery_pr: None,
        related_prs: vec![313],
    };
    let second = GateEvidenceArgs {
        issue: 326,
        ..first.clone()
    };
    let body = format!(
        "## G3 合并判断\n- Gate 断言：`{}` 已通过。\n- Gate 断言：`{}` 已通过。",
        expected_gate_command(&first, GateEvidencePhase::G3),
        expected_gate_command(&second, GateEvidencePhase::G3),
    );

    assert_eq!(
        parse_gate_evidence_target_metadata_from_g3_comment("illusion-tech/laneflow", 313, &body,),
        Ok((GateEvidencePrRole::Related, vec![325, 326]))
    );

    let wrong_pr =
        parse_gate_evidence_target_metadata_from_g3_comment("illusion-tech/laneflow", 314, &body)
            .expect_err("a G3 assertion for another PR must not recover metadata");
    assert!(wrong_pr.contains("当前 PR #314"));
}

#[test]
fn dependabot_metadata_recovery_accepts_mixed_current_exception_results() {
    let first = GateEvidenceArgs {
        phase: GateEvidencePhase::G3,
        repo: "illusion-tech/laneflow".to_string(),
        issue: 325,
        delivery_pr: None,
        related_prs: vec![313],
    };
    let second = GateEvidenceArgs {
        issue: 326,
        ..first.clone()
    };
    let body = format!(
        "## G3 合并判断\n- Gate 结果：`G3 Exception`\n- Gate 断言：`{}` 未通过。\n- Gate 断言：`{}` 已通过。",
        expected_gate_command(&first, GateEvidencePhase::G3),
        expected_gate_command(&second, GateEvidencePhase::G3),
    );

    assert_eq!(
        parse_gate_evidence_target_metadata_from_g3_comment("illusion-tech/laneflow", 313, &body,),
        Ok((GateEvidencePrRole::Related, vec![325, 326]))
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
fn all_g3_modes_share_the_same_target_contract() {
    let (args, related_pr) = current_related_g3_target();
    for mode in [
        G3ValidationMode::RelatedOnly,
        G3ValidationMode::DeliveryFullSet,
        G3ValidationMode::ShadowTarget,
    ] {
        assert_eq!(
            validate_g3_target(
                mode,
                "illusion-tech/laneflow",
                62,
                GateEvidencePhase::G3,
                &related_pr,
                std::slice::from_ref(&args),
            ),
            Ok((GateEvidencePrRole::Related, vec![60])),
            "mode={} should use the shared target contract",
            mode.label()
        );
    }
}

#[test]
fn all_g3_modes_reject_missing_shadow_target_evidence() {
    let (args, mut related_pr) = current_related_g3_target();
    related_pr.comments[0].body = related_pr.comments[0]
        .body
        .replace(
            &format!(
                "\n{G3_EVIDENCE_SHADOW_COMMENT_FIELD}R1 non-required：source App 仍为 github-actions，仅作 telemetry"
            ),
            "",
        );

    for mode in [
        G3ValidationMode::RelatedOnly,
        G3ValidationMode::DeliveryFullSet,
        G3ValidationMode::ShadowTarget,
    ] {
        let error = validate_g3_target(
            mode,
            "illusion-tech/laneflow",
            62,
            GateEvidencePhase::G3,
            &related_pr,
            std::slice::from_ref(&args),
        )
        .expect_err("every G3 mode must reject the #351 missing-shadow failure");
        assert!(error.contains(&format!("mode={}", mode.label())));
        assert!(error.contains("G3 Evidence Gate Shadow"));
    }
}

#[test]
fn shared_g3_target_errors_report_expected_and_actual_sets() {
    let (_, related_pr) = current_related_g3_target();
    let actual_args = GateEvidenceArgs {
        issue: 61,
        ..related_only_g3_args()
    };
    let error = validate_g3_target(
        G3ValidationMode::RelatedOnly,
        "illusion-tech/laneflow",
        62,
        GateEvidencePhase::G3,
        &related_pr,
        &[actual_args],
    )
    .expect_err("declared and resolved Issue sets must match");

    assert!(error.contains("预期声明 [#60]"));
    assert!(error.contains("实际解析 [#61]"));
    assert!(error.contains("--issue 61"));
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
    assert!(
        validate_g3_comment_after_external_review_completion(
            "2026-08-06T03:30:01Z",
            "2026-08-06T03:30:00Z",
            "Delivery PR",
        )
        .is_ok()
    );
    let error = validate_g3_comment_after_external_review_completion(
        "2026-08-06T03:30:00Z",
        "2026-08-06T03:30:00Z",
        "Delivery PR",
    )
    .expect_err("a same-second G3 comment cannot prove that disposition completed first");
    assert!(error.contains("必须严格晚于"));
    assert!(parse_utc_timestamp_seconds("2026-08-06T03:30:00.Z").is_none());
}

#[test]
fn validates_rest_backed_effective_time_for_an_edited_g3_comment() {
    let mut comment = g3_comment(DELIVERY_G3_URL, "2026-08-06T03:30:00Z");
    comment.includes_created_edit = true;
    let snapshot = GitHubIssueCommentRest {
        body: Some(comment.body.clone()),
        created_at: comment.created_at.clone(),
        updated_at: "2026-08-06T03:31:00Z".to_string(),
        issue_url: "https://api.github.com/repos/illusion-tech/laneflow/issues/61".to_string(),
    };

    assert_eq!(issue_comment_id_from_permalink(&comment.url), Ok(100));
    assert_eq!(
        validate_edited_g3_comment_snapshot("illusion-tech/laneflow", 61, &comment, &snapshot,),
        Ok("2026-08-06T03:31:00Z".to_string())
    );

    let wrong_pr = GitHubIssueCommentRest {
        issue_url: "https://api.github.com/repos/illusion-tech/laneflow/issues/62".to_string(),
        ..snapshot.clone()
    };
    assert!(
        validate_edited_g3_comment_snapshot("illusion-tech/laneflow", 61, &comment, &wrong_pr,)
            .is_err()
    );
}

#[test]
fn only_dependabot_body_edits_may_reuse_an_older_marker() {
    let timestamps = GitHubEditTimestamps {
        created_at: "2026-08-06T03:00:00Z".to_string(),
        last_edited_at: Some("2026-08-06T03:31:00Z".to_string()),
        updated_at: "2026-08-06T03:31:00Z".to_string(),
    };
    let mut edits = GitHubUserContentEditConnection {
        page_info: GitHubPageInfo {
            has_next_page: false,
        },
        nodes: vec![
            GitHubUserContentEdit {
                edited_at: "2026-08-06T03:20:00Z".to_string(),
                editor: Some(GitHubActor {
                    login: "wangzishi".to_string(),
                }),
                diff: None,
            },
            GitHubUserContentEdit {
                edited_at: "2026-08-06T03:31:00Z".to_string(),
                editor: Some(GitHubActor {
                    login: "dependabot".to_string(),
                }),
                diff: None,
            },
        ],
    };

    assert!(
        validate_dependabot_body_edits_after_marker(
            "2026-08-06T03:30:00Z",
            &timestamps,
            &edits,
            "PR #313",
        )
        .is_ok()
    );
    assert!(
        validate_dependabot_body_edits_after_g3_comment("2026-08-06T03:25:00Z", &edits, "PR #313",)
            .is_ok()
    );
    let error =
        validate_dependabot_body_edits_after_g3_comment("2026-08-06T03:15:00Z", &edits, "PR #313")
            .expect_err(
                "a human edit after the G3 comment cannot be hidden by a later bot refresh",
            );
    assert!(error.contains("G3 comment 后包含非 Dependabot body edit"));
    let error =
        validate_dependabot_body_edits_after_g3_comment("2026-08-06T03:31:00Z", &edits, "PR #313")
            .expect_err("a same-second body edit cannot prove that Dependabot edited after G3");
    assert!(error.contains("与 current G3 comment 同秒"));

    edits.nodes[0].edited_at = "2026-08-06T03:30:00Z".to_string();
    edits.nodes[0].editor = Some(GitHubActor {
        login: "dependabot".to_string(),
    });
    let error = validate_dependabot_body_edits_after_marker(
        "2026-08-06T03:30:00Z",
        &timestamps,
        &edits,
        "PR #313",
    )
    .expect_err("a same-second bot edit is ambiguous and requires a new marker");
    assert!(error.contains("与 G3 evidence marker 同秒"));
    edits.nodes[0].edited_at = "2026-08-06T03:20:00Z".to_string();

    let mut human_edit = edits;
    human_edit.nodes[1].editor = Some(GitHubActor {
        login: "wangzishi".to_string(),
    });
    let error = validate_dependabot_body_edits_after_marker(
        "2026-08-06T03:30:00Z",
        &timestamps,
        &human_edit,
        "PR #313",
    )
    .expect_err("a later human edit still requires a new marker");
    assert!(error.contains("非 Dependabot body edit"));
    assert!(
        validate_dependabot_body_edits_after_g3_comment(
            "2026-08-06T03:25:00Z",
            &human_edit,
            "PR #313",
        )
        .is_err()
    );
}

#[test]
fn recovered_dependabot_metadata_preserves_g4_related_replay_inputs() {
    let g3_args = GateEvidenceArgs {
        phase: GateEvidencePhase::G3,
        repo: "illusion-tech/laneflow".to_string(),
        issue: 325,
        delivery_pr: None,
        related_prs: vec![313],
    };
    let comment = g3_comment_for_args(RELATED_G3_URL, "2026-07-10T05:00:00Z", &g3_args);
    let recovered_body =
        recovered_gate_evidence_target_body(&g3_args.repo, 313, RELATED_G3_URL, &comment.body)
            .expect("validated Dependabot recovery should rebuild target metadata");

    assert_eq!(
        parse_gate_evidence_target_metadata(&recovered_body),
        Ok((GateEvidencePrRole::Related, vec![325]))
    );

    let mut related = related_pr_for_args(false, &g3_args);
    related.body = recovered_body;
    related.comments = vec![comment];
    let issue_body = format!("- [x] G3 合并判断已记录：[Related G3 评论]({RELATED_G3_URL})");
    let g4_args = GateEvidenceArgs {
        phase: GateEvidencePhase::G4,
        ..g3_args
    };

    assert!(
        validate_related_pr_g3(&g4_args, &issue_body, &issue_body, 313, &related, false).is_ok()
    );
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
            "mergeCommit": {"oid": "dddddddddddddddddddddddddddddddddddddddd"},
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
        pr.merge_commit.as_ref().map(|commit| commit.oid.as_str()),
        Some(MAIN_RESULT_OID)
    );
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
fn rejects_merged_related_g3_effective_at_merge_second() {
    let mut related_pr = related_pr(false);
    related_pr.state = "MERGED".to_string();
    related_pr.merged_at = Some("2026-07-10T05:30:00Z".to_string());
    related_pr.comments[0].includes_created_edit = true;
    related_pr.comments[0].updated_at = Some("2026-07-10T05:30:00Z".to_string());

    let error = validate_g3_timing(
        &related_pr,
        RELATED_G3_URL,
        "Related PR #62",
        &related_only_g3_args(),
    )
    .expect_err("the merge second cannot prove that an edit happened before merge");

    assert!(error.contains("生效时间必须严格早于 PR 合并时间"));
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
    assert!(workflow.contains("g3-evidence-gate-${{\n      ("));
    assert!(workflow.contains("github.run_attempt\n    }}-${{\n"));
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
    assert!(workflow.contains("REUSE_MARKER_EVENT"));
    assert!(workflow.contains("REUSE_MARKER"));
    assert!(workflow.contains("github.event.changes.body.from != null"));
    assert!(workflow.contains("github.event.changes.base == null"));
    assert!(!workflow.contains("github.event.changes.title == null"));
    assert!(workflow.contains("repos/${REPOSITORY}/issues/${PR_NUMBER}/comments?per_page=100"));
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
fn selects_delivery_and_related_targets_across_supported_role_phase_matrix() {
    let mut delivery_with_related = gate_args(GateEvidencePhase::G4);
    delivery_with_related.related_prs = vec![62, 63];

    assert_eq!(
        selected_gate_evidence_pr(&gate_args(GateEvidencePhase::G3)),
        Ok((GateEvidencePrRole::Delivery, 61))
    );
    assert_eq!(
        selected_gate_evidence_pr(&delivery_with_related),
        Ok((GateEvidencePrRole::Delivery, 61))
    );
    assert_eq!(
        selected_gate_evidence_pr(&related_only_g3_args()),
        Ok((GateEvidencePrRole::Related, 62))
    );
}

#[test]
fn invalid_role_phase_selection_fails_closed_before_cached_or_remote_access() {
    let cases = [
        (
            GateEvidenceArgs {
                delivery_pr: None,
                related_prs: Vec::new(),
                ..gate_args(GateEvidencePhase::G3)
            },
            "G3 必须指定 Delivery PR",
        ),
        (
            GateEvidenceArgs {
                delivery_pr: None,
                related_prs: vec![62, 63],
                ..gate_args(GateEvidencePhase::G3)
            },
            "只能指定一个 Related PR",
        ),
        (
            GateEvidenceArgs {
                delivery_pr: None,
                related_prs: vec![62],
                ..gate_args(GateEvidencePhase::G4)
            },
            "G4 必须指定",
        ),
    ];
    let issue = issue("OPEN", "In Review");
    let pr = delivery_pr(None);

    for (args, expected_error) in cases {
        let mut issue_fetches = 0;
        let mut pr_fetches = 0;
        let error = check_gate_evidence_with_loaders(
            &args,
            Some(CachedGateEvidence {
                issue_number: 60,
                issue: &issue,
                pr_number: 61,
                pr: &pr,
            }),
            |_, _, _| {
                issue_fetches += 1;
                Err("unexpected Issue fetch".to_string())
            },
            |_, _, _| {
                pr_fetches += 1;
                Err("unexpected PR fetch".to_string())
            },
        )
        .expect_err("invalid role/phase selection must fail closed");

        assert!(error.contains(expected_error), "{error}");
        assert_eq!(issue_fetches, 0);
        assert_eq!(pr_fetches, 0);
        assert!(print_gate_evidence_success(&args).is_err());
    }
}

#[test]
fn missing_internal_pr_snapshots_fail_closed() {
    let delivery_args = gate_args(GateEvidencePhase::G3);
    let delivery_error = validate_current_g3_target(&delivery_args, None, &[])
        .expect_err("missing Delivery snapshot must be rejected");
    assert!(delivery_error.contains("缺少已读取的 Delivery PR snapshot"));

    let related_args = related_only_g3_args();
    let related_error = validate_current_g3_target(&related_args, None, &[])
        .expect_err("missing Related snapshot must be rejected");
    assert!(related_error.contains("snapshot 数量"));

    let issue = issue_with_pending_delivery_and_related_g3();
    let related_pr = related_pr_for_args(false, &related_args);
    let full_set_error = validate_g3_evidence(
        &related_args,
        &issue,
        &related_pr,
        std::slice::from_ref(&related_pr),
    )
    .expect_err("full-set validation without Delivery args must be rejected");
    assert!(full_set_error.contains("缺少 Delivery PR 参数"));
}

#[test]
fn cached_g3_target_avoids_duplicate_issue_and_pr_fetches() {
    let args = related_only_g3_args();
    let issue = issue_with_pending_delivery_and_related_g3();
    let related_pr = related_pr_for_args(false, &args);
    let mut issue_fetches = 0;
    let mut pr_fetches = 0;

    let result = check_gate_evidence_with_loaders(
        &args,
        Some(CachedGateEvidence {
            issue_number: 60,
            issue: &issue,
            pr_number: 62,
            pr: &related_pr,
        }),
        |_, _, _| {
            issue_fetches += 1;
            Err("unexpected Issue fetch".to_string())
        },
        |_, _, _| {
            pr_fetches += 1;
            Err("unexpected PR fetch".to_string())
        },
    );

    assert!(result.is_ok());
    assert_eq!(issue_fetches, 0);
    assert_eq!(pr_fetches, 0);
}

#[test]
fn cached_delivery_target_without_related_prs_avoids_duplicate_fetches_in_g3_and_g4() {
    let cases = [
        (
            gate_args(GateEvidencePhase::G3),
            issue("OPEN", "In Review"),
            delivery_pr(None),
        ),
        (
            gate_args(GateEvidencePhase::G4),
            issue("OPEN", "Done"),
            delivery_pr(Some("2026-07-10T05:30:00Z")),
        ),
    ];

    for (args, issue, delivery_pr) in cases {
        let mut issue_fetches = 0;
        let mut pr_fetches = 0;
        let result = check_gate_evidence_with_loaders(
            &args,
            Some(CachedGateEvidence {
                issue_number: 60,
                issue: &issue,
                pr_number: 61,
                pr: &delivery_pr,
            }),
            |_, _, _| {
                issue_fetches += 1;
                Err("unexpected Issue fetch".to_string())
            },
            |_, _, _| {
                pr_fetches += 1;
                Err("unexpected PR fetch".to_string())
            },
        );

        assert!(result.is_ok(), "{result:?}");
        assert_eq!(issue_fetches, 0);
        assert_eq!(pr_fetches, 0);
    }
}

#[test]
fn cached_delivery_target_with_multiple_related_prs_loads_only_related_members_in_g3_and_g4() {
    for phase in [GateEvidencePhase::G3, GateEvidencePhase::G4] {
        let related_numbers = vec![62, 63];
        let mut args = gate_args(phase);
        args.related_prs = related_numbers.clone();
        let mut g3_args = args.clone();
        g3_args.phase = GateEvidencePhase::G3;

        let project_status = if phase == GateEvidencePhase::G3 {
            "In Review"
        } else {
            "Done"
        };
        let mut issue = issue("OPEN", project_status);
        issue.body = issue
            .body
            .replace("Related PRs：N/A，原因：无部分交付。", "Related PRs：#62、#63")
            .replace(
                &format!(
                    "- [x] G3 合并判断已记录：[Delivery G3 评论]({DELIVERY_G3_URL})"
                ),
                &format!(
                    "- [x] G3 合并判断已记录：[Delivery G3 评论]({DELIVERY_G3_URL})；[Related #62 G3 评论](https://github.com/illusion-tech/laneflow/pull/62#issuecomment-620)；[Related #63 G3 评论](https://github.com/illusion-tech/laneflow/pull/63#issuecomment-630)"
                ),
            );
        if phase == GateEvidencePhase::G4 {
            issue.comments[0] = g4_comment_for_args(ISSUE_G4_URL, "2026-07-10T06:00:00Z", &args);
        }

        let mut delivery_pr =
            delivery_pr((phase == GateEvidencePhase::G4).then_some("2026-07-10T05:30:00Z"));
        delivery_pr.comments[0] =
            g3_comment_for_args(DELIVERY_G3_URL, "2026-07-10T05:00:00Z", &g3_args);

        let mut related_prs = std::collections::BTreeMap::new();
        for number in related_numbers {
            let related_args = GateEvidenceArgs {
                phase: GateEvidencePhase::G3,
                repo: args.repo.clone(),
                issue: args.issue,
                delivery_pr: None,
                related_prs: vec![number],
            };
            let permalink = format!(
                "https://github.com/illusion-tech/laneflow/pull/{number}#issuecomment-{number}0"
            );
            let mut related_pr = related_pr_for_args(false, &related_args);
            related_pr.body = related_pr.body.replace(RELATED_G3_URL, &permalink);
            related_pr.comments[0].url = permalink;
            if phase == GateEvidencePhase::G4 {
                related_pr.state = "MERGED".to_string();
                related_pr.merged_at = Some(if number == 62 {
                    "2026-07-10T05:10:00Z".to_string()
                } else {
                    "2026-07-10T05:20:00Z".to_string()
                });
                related_pr.project_items[0].status = Some(ProjectStatus {
                    name: "Done".to_string(),
                });
            }
            related_prs.insert(number, related_pr);
        }

        let mut issue_fetches = 0;
        let mut pr_fetches = 0;
        let result = check_gate_evidence_with_loaders(
            &args,
            Some(CachedGateEvidence {
                issue_number: 60,
                issue: &issue,
                pr_number: 61,
                pr: &delivery_pr,
            }),
            |_, _, _| {
                issue_fetches += 1;
                Err("unexpected Issue fetch".to_string())
            },
            |_, number, _| {
                pr_fetches += 1;
                related_prs
                    .remove(&number)
                    .ok_or_else(|| format!("unexpected PR fetch #{number}"))
            },
        );

        assert!(result.is_ok(), "phase={phase:?}: {result:?}");
        assert_eq!(issue_fetches, 0);
        assert_eq!(pr_fetches, 2);
        assert!(related_prs.is_empty());
    }
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
fn rejects_current_g3_without_shadow_evidence_field() {
    let args = related_only_g3_args();
    let mut related_pr = related_pr_for_args(false, &args);
    related_pr.comments[0].created_at = G3_EVIDENCE_SHADOW_ACTIVATION.to_string();
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
fn accepts_g3_without_shadow_evidence_before_shadow_activation() {
    let args = related_only_g3_args();
    let mut related_pr = related_pr_for_args(false, &args);
    related_pr.comments[0].created_at = "2026-08-06T10:49:20Z".to_string();
    related_pr.comments[0].body = gate_comment_body(CURRENT_G3_COMMENT_FIELDS, &args);

    assert!(
        validate_gate_evidence_target_pr(
            "illusion-tech/laneflow",
            GateEvidencePhase::G3,
            &related_pr,
            GateEvidencePrRole::Related,
            &[60],
        )
        .is_ok()
    );
}

#[test]
fn accepts_only_unique_populated_shadow_evidence_choices() {
    for value in [
        "Check URL：https://github.com/illusion-tech/laneflow/actions/runs/1",
        "R1 non-required：source App 仍为 github-actions，仅作 telemetry",
        "候选 workflow bootstrap：尚未合入 main，不能用于本 PR 自批",
        "`R1 non-required：source App 仍为 github-actions，仅作 telemetry`",
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
            "{G3_EVIDENCE_SHADOW_COMMENT_FIELD}Check URL：`https://github.com/illusion-tech/laneflow/actions/runs/1`"
        ),
        format!(
            "{G3_EVIDENCE_SHADOW_COMMENT_FIELD}`Check URL：https://github.com/illusion-tech/laneflow/actions/runs/1` trailing"
        ),
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
fn edited_g3_uses_effective_time_for_policy_activation() {
    let args = related_only_g3_args();
    let issue = issue_with_pending_delivery_and_related_g3();
    let mut related_pr = related_pr_for_args(false, &args);
    related_pr.comments[0].created_at = "2026-07-24T15:16:20Z".to_string();
    related_pr.comments[0].updated_at = Some(EXTERNAL_REVIEW_G3_ACTIVATION.to_string());
    related_pr.comments[0].includes_created_edit = true;

    let error = validate_related_g3_evidence(&args, &issue, 62, &related_pr)
        .expect_err("an edit crossing the activation boundary must use current fields");

    assert!(error.contains("- Gate 结果："));
    assert!(error.contains("- Current head："));
}

fn post_merge_shadow_correction_fixture() -> (GateEvidenceArgs, GitHubPullRequest) {
    let args = gate_args(GateEvidencePhase::G4);
    let shadow_value = "R1 non-required：source App 仍为 github-actions，仅作 telemetry";
    let corrected_body = format!(
        "{}\n{G3_EVIDENCE_SHADOW_COMMENT_FIELD}{shadow_value}。",
        gate_comment_body(
            CURRENT_G3_COMMENT_FIELDS,
            &GateEvidenceArgs {
                phase: GateEvidencePhase::G3,
                ..args.clone()
            }
        )
    );
    let original_body = corrected_body.replace(
        &format!("{G3_EVIDENCE_SHADOW_COMMENT_FIELD}{shadow_value}。"),
        &format!("{G3_EVIDENCE_SHADOW_COMMENT_FIELD}`{shadow_value}`。"),
    );
    let edited_at = "2026-07-10T06:00:00Z";
    let mut target_comment = g3_comment(DELIVERY_G3_URL, "2026-07-10T04:50:00Z");
    target_comment.id = "IC_correction_target".to_string();
    target_comment.body = corrected_body.clone();
    target_comment.updated_at = Some(edited_at.to_string());
    target_comment.includes_created_edit = true;
    target_comment.user_content_edits = Some(GitHubUserContentEditConnection {
        page_info: GitHubPageInfo {
            has_next_page: false,
        },
        nodes: vec![
            // GitHub returns userContentEdits newest-first; each `diff` is a full body snapshot.
            GitHubUserContentEdit {
                edited_at: edited_at.to_string(),
                editor: Some(GitHubActor {
                    login: "wangzishi".to_string(),
                }),
                diff: Some(corrected_body.clone()),
            },
            GitHubUserContentEdit {
                edited_at: "2026-07-10T05:00:00Z".to_string(),
                editor: Some(GitHubActor {
                    login: "wangzishi".to_string(),
                }),
                diff: Some(original_body.clone()),
            },
        ],
    });
    let mut correction = g3_comment(
        "https://github.com/illusion-tech/laneflow/pull/61#issuecomment-401",
        "2026-07-10T06:30:00Z",
    );
    correction.body = format!(
        r##"<!-- g3-comment-correction:v1
{{
  "schemaVersion": 1,
  "id": "correction-60-61-1",
  "issue": 60,
  "pullRequest": 61,
  "currentHeadOid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "g3Comment": "{DELIVERY_G3_URL}",
  "originalBodySha256": "{}",
  "newBodySha256": "{}",
  "editedAt": "{edited_at}",
  "editor": "wangzishi",
  "reason": "remove one whole-field backtick wrapper",
  "risk": "post-merge user content edit",
  "acceptanceBoundary": "format-only shadow wrapper correction; never changes Gate result",
  "followUpIssue": "#398",
  "cleanupOwner": "wangzishi",
  "authorizedBy": "wangzishi"
}}
-->"##,
        body_sha256(&original_body),
        body_sha256(&corrected_body),
    );
    let mut pr = delivery_pr(Some("2026-07-10T05:30:00Z"));
    pr.comments = vec![target_comment, correction];
    (args, pr)
}

#[test]
fn accepts_only_signed_post_merge_shadow_wrapper_corrections() {
    let (args, pr) = post_merge_shadow_correction_fixture();
    let result = validate_g3_timing(&pr, DELIVERY_G3_URL, "Delivery G3", &args);
    assert!(result.is_ok(), "{result:?}");

    let (args, mut untrusted_marker) = post_merge_shadow_correction_fixture();
    let mut malformed = untrusted_marker.comments[1].clone();
    malformed.url =
        "https://github.com/illusion-tech/laneflow/pull/61#issuecomment-untrusted-correction"
            .to_string();
    malformed.author = Some(GitHubActor {
        login: "untrusted-user".to_string(),
    });
    malformed.body = "<!-- g3-comment-correction:v1\nnot-json".to_string();
    untrusted_marker.comments.insert(1, malformed);
    let result = validate_g3_timing(&untrusted_marker, DELIVERY_G3_URL, "Delivery G3", &args);
    assert!(result.is_ok(), "{result:?}");

    let (args, mut wrong_hash) = post_merge_shadow_correction_fixture();
    wrong_hash.comments[1].body = wrong_hash.comments[1].body.replace(
        "sha256:",
        "sha256:0000000000000000000000000000000000000000000000000000000000000000-invalid-",
    );
    assert!(validate_g3_timing(&wrong_hash, DELIVERY_G3_URL, "Delivery G3", &args).is_err());

    let (args, mut wrong_editor) = post_merge_shadow_correction_fixture();
    wrong_editor.comments[0]
        .user_content_edits
        .as_mut()
        .unwrap()
        .nodes[0]
        .editor = Some(GitHubActor {
        login: "untrusted-user".to_string(),
    });
    assert!(validate_g3_timing(&wrong_editor, DELIVERY_G3_URL, "Delivery G3", &args).is_err());

    let (args, mut extra_delta) = post_merge_shadow_correction_fixture();
    let edits = extra_delta.comments[0].user_content_edits.as_mut().unwrap();
    let original = edits.nodes[1].diff.clone().unwrap();
    let changed = format!("{original}\n- unrelated semantic change");
    edits.nodes[1].diff = Some(changed.clone());
    extra_delta.comments[1].body = extra_delta.comments[1]
        .body
        .replace(&body_sha256(&original), &body_sha256(&changed));
    let error = validate_g3_timing(&extra_delta, DELIVERY_G3_URL, "Delivery G3", &args)
        .expect_err("correction may not hide any non-shadow delta");
    assert!(error.contains("只允许完整 shadow 字段"));

    let (args, mut punctuation_delta) = post_merge_shadow_correction_fixture();
    let edits = punctuation_delta.comments[0]
        .user_content_edits
        .as_mut()
        .unwrap();
    let original = edits.nodes[1].diff.clone().unwrap();
    let changed = original.replace("`。", "`");
    edits.nodes[1].diff = Some(changed.clone());
    punctuation_delta.comments[1].body = punctuation_delta.comments[1]
        .body
        .replace(&body_sha256(&original), &body_sha256(&changed));
    let error = validate_g3_timing(&punctuation_delta, DELIVERY_G3_URL, "Delivery G3", &args)
        .expect_err("correction may not hide punctuation changes");
    assert!(error.contains("只允许完整 shadow 字段"));
}

#[test]
fn a_historical_exception_applies_only_to_its_exact_full_set_target() {
    let mut args = gate_args(GateEvidencePhase::G4);
    args.related_prs = vec![62];

    let (mut delivery, mut exception_appendix) = g3_exception_fixture(
        "legacy_evidence_reconstruction",
        "G3 Block",
        "2026-07-10T06:00:00Z",
        "2026-07-10T07:00:00Z",
    );
    let original_delivery_body = delivery.comments[0].body.clone();
    let original_command =
        expected_gate_command(&gate_args(GateEvidencePhase::G3), GateEvidencePhase::G3);
    let full_set_command = expected_gate_command(&args, GateEvidencePhase::G3);
    delivery.comments[0].body =
        original_delivery_body.replace(&original_command, &full_set_command);
    exception_appendix.body = exception_appendix.body.replace(
        &body_sha256(&original_delivery_body),
        &body_sha256(&delivery.comments[0].body),
    );
    delivery.comments.truncate(1);
    delivery.state = "MERGED".to_string();
    delivery.merged_at = Some("2026-07-10T05:30:00Z".to_string());
    delivery.merge_commit = Some(GitHubCommit {
        oid: MAIN_RESULT_OID.to_string(),
    });

    let mut related = related_pr(false);
    related.state = "MERGED".to_string();
    related.merged_at = Some("2026-07-10T05:40:00Z".to_string());

    let mut issue = issue("OPEN", "Done");
    issue.body = issue
        .body
        .replace("Related PRs：N/A，原因：无部分交付。", "Related PRs：#62")
        .replace(
            &format!(
                "- [x] G3 合并判断已记录：[Delivery G3 评论]({DELIVERY_G3_URL})"
            ),
            &format!(
                "- [x] G3 合并判断已记录：[Delivery G3 评论]({DELIVERY_G3_URL})；[Related G3 评论]({RELATED_G3_URL})"
            ),
        );
    exception_appendix.url = ISSUE_G4_URL.to_string();
    issue.comments[0] = exception_appendix;

    assert_eq!(
        g3_requires_result_validation(60, 61, &delivery, Some(&issue.comments[0])),
        Ok(true)
    );
    assert_eq!(
        g3_requires_result_validation(60, 62, &related, Some(&issue.comments[0])),
        Ok(false)
    );
    let result = validate_g3_evidence(&args, &issue, &delivery, &[related]);
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn target_assertion_set_honors_a_validated_legacy_exception_result() {
    let args = related_only_g3_args();
    let issue = args.issue;
    let mut related = related_pr(false);
    related.comments[0].body = related.comments[0]
        .body
        .replace("`R0-R1 bootstrap`", "`G3 Pass`")
        .replace("` 已通过。", "` 未通过。");

    assert!(
        validate_gate_evidence_target_assertions(&related, std::slice::from_ref(&args)).is_err()
    );
    assert!(
        validate_gate_evidence_target_assertions_with_legacy_exception(
            &related,
            &[args],
            Some((issue, true)),
        )
        .is_ok()
    );
}

#[test]
fn target_assertion_set_scopes_a_legacy_exception_to_the_matching_issue() {
    let first = related_only_g3_args();
    let second = GateEvidenceArgs {
        phase: GateEvidencePhase::G3,
        repo: first.repo.clone(),
        issue: 61,
        delivery_pr: None,
        related_prs: first.related_prs.clone(),
    };
    let first_command = expected_gate_command(&first, GateEvidencePhase::G3);
    let second_command = expected_gate_command(&second, GateEvidencePhase::G3);
    let mut related = related_pr(false);
    let original_assertion = related.comments[0]
        .body
        .lines()
        .find(|line| line.starts_with(GATE_ASSERTION_PREFIX))
        .unwrap()
        .to_string();
    related.comments[0].body = related.comments[0].body.replace(
        &original_assertion,
        &format!(
            "- Gate 断言：`{first_command}` 未通过。\n- Gate 断言：`{second_command}` 已通过。"
        ),
    );

    assert!(
        validate_gate_evidence_target_assertions_with_legacy_exception(
            &related,
            &[first.clone(), second.clone()],
            Some((first.issue, true)),
        )
        .is_ok()
    );
    assert!(
        validate_gate_evidence_target_assertions(&related, &[first.clone(), second.clone()])
            .is_err()
    );
    assert!(
        validate_gate_evidence_target_assertions_with_legacy_exception(
            &related,
            &[first, second.clone()],
            Some((second.issue, false)),
        )
        .is_ok()
    );
}

#[test]
fn a_lone_backtick_fails_closed_without_panicking() {
    assert!(parse_optional_backtick_value("`", "Gate 结果").is_err());
}

#[test]
fn issue_351_history_fixture_preserves_all_three_pr_targets_and_shadow_forms() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/gate-evidence/issue-351-shadow-edit-v1.json"
    ))
    .expect("#351 regression fixture must remain valid JSON");
    assert_eq!(fixture["issue"], 351);
    assert_eq!(fixture["deliveryPr"], 397);
    assert_eq!(fixture["relatedPrs"], serde_json::json!([395, 396]));
    for record in fixture["g3Comments"].as_array().unwrap() {
        for field in ["historicalShadowLine", "canonicalShadowLine"] {
            assert!(
                validate_g3_evidence_shadow_comment_field(record[field].as_str().unwrap()).is_ok(),
                "fixture PR {} {field} must parse",
                record["pr"]
            );
        }
    }
}

#[test]
fn all_three_governance_sources_preserve_each_rollout_shadow_choice() {
    for (label, source) in [
        (
            "development-gates",
            include_str!("../../../docs/governance/development-gates.md"),
        ),
        (
            "pull-request-template",
            include_str!("../../../.github/pull_request_template.md"),
        ),
        (
            "governance-skill",
            include_str!("../../../.agents/skills/laneflow-governance/SKILL.md"),
        ),
    ] {
        for (documented, canonical) in [
            (
                "候选 workflow bootstrap：<边界>",
                "候选 workflow bootstrap：边界",
            ),
            ("R1 non-required：<原因>", "R1 non-required：原因"),
            (
                "Check URL：https://github.com/...",
                "Check URL：https://github.com/illusion-tech/laneflow/runs/1",
            ),
        ] {
            assert!(source.contains(documented), "{label} 缺少 `{documented}`");
            let line = format!("{G3_EVIDENCE_SHADOW_COMMENT_FIELD}{canonical}");
            assert!(
                validate_g3_evidence_shadow_comment_field(&line).is_ok(),
                "{label} documented an unparsable shadow choice: {line}"
            );
        }
    }
}

#[test]
fn authoritative_g4_template_preserves_historical_non_success() {
    let source = include_str!("../../../docs/governance/development-gates.md");
    assert!(source.contains(
        "正常 G4 写“已通过”；仅精确匹配 `legacy_evidence_reconstruction` 的 historical replay 写“未通过”"
    ));
}

fn g3_exception_fixture(
    exception_type: &str,
    gate_result: &str,
    accepted_at: &str,
    expires_at: &str,
) -> (GitHubPullRequest, GitHubComment) {
    let args = gate_args(GateEvidencePhase::G3);
    let mut g3 = g3_comment(DELIVERY_G3_URL, "2026-07-10T05:00:00Z");
    g3.body = format!(
        "{}\n{G3_EVIDENCE_SHADOW_COMMENT_FIELD}R1 non-required：exception 保持 non-success",
        gate_comment_body(CURRENT_G3_COMMENT_FIELDS, &args)
            .replace("`R0-R1 bootstrap`", &format!("`{gate_result}`"))
            .replace(
                "- Current head：",
                "- Current head：`aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa`",
            )
            .replace("` 已通过。", "` 未通过。")
    );
    let mut appendix = g3_comment(
        "https://github.com/illusion-tech/laneflow/pull/61#issuecomment-402",
        accepted_at,
    );
    appendix.body = format!(
        r##"- 例外：结构化记录，证据见 [gate defect][defect-evidence]。
<!-- g3-exception:v1
{{
  "schemaVersion": 1,
  "id": "exception-60-61-1",
  "exceptionType": "{exception_type}",
  "issue": 60,
  "pullRequest": 61,
  "currentHeadOid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "currentBaseOid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  "g3Comment": "{DELIVERY_G3_URL}",
  "g3CommentBodySha256": "{}",
  "reason": "the validator has a confirmed false negative",
  "evidenceRefs": ["defect-evidence"],
  "risk": "the automated assertion remains failed",
  "acceptanceBoundary": "audit only; never maps to pass",
  "acceptedAt": "{accepted_at}",
  "expiresAt": "{expires_at}",
  "followUpIssue": "#405",
  "cleanupOwner": "wangzishi",
  "authorizedBy": "wangzishi"
}}
-->

[defect-evidence]: https://github.com/illusion-tech/laneflow/issues/405"##,
        body_sha256(&g3.body),
    );
    let mut pr = delivery_pr(None);
    pr.comments = vec![g3, appendix.clone()];
    (pr, appendix)
}

#[test]
fn current_exception_scopes_multi_issue_assertions_without_forcing_other_issues_to_fail() {
    let (mut pr, _) = g3_exception_fixture(
        "confirmed_gate_defect",
        "G3 Exception",
        "2026-07-10T05:10:00Z",
        "2026-07-10T06:00:00Z",
    );
    pr.body = format!(
        "- 关联 Issue：#60、#61\n- PR 角色：Delivery PR\n{}",
        pr.body
    );
    pr.closing_issues_references
        .push(issue_reference("illusion-tech/laneflow", 61));
    let first = gate_args(GateEvidencePhase::G3);
    let second = GateEvidenceArgs {
        issue: 61,
        ..first.clone()
    };
    let first_command = expected_gate_command(&first, GateEvidencePhase::G3);
    let second_command = expected_gate_command(&second, GateEvidencePhase::G3);
    let original_assertion = pr.comments[0]
        .body
        .lines()
        .find(|line| line.starts_with(GATE_ASSERTION_PREFIX))
        .unwrap()
        .to_string();
    pr.comments[0].body = pr.comments[0].body.replace(
        &original_assertion,
        &format!(
            "- Gate 断言：`{first_command}` 未通过。\n- Gate 断言：`{second_command}` 已通过。"
        ),
    );

    assert_eq!(
        validate_g3_target(
            G3ValidationMode::ShadowTarget,
            "illusion-tech/laneflow",
            61,
            GateEvidencePhase::G3,
            &pr,
            &[first.clone(), second.clone()],
        ),
        Ok((GateEvidencePrRole::Delivery, vec![60, 61]))
    );
    assert!(
        validate_comment(
            &pr,
            DELIVERY_G3_URL,
            G3_COMMENT_FIELDS,
            "Issue #60 G3",
            &first,
            false,
        )
        .is_ok()
    );
    assert!(
        validate_comment(
            &pr,
            DELIVERY_G3_URL,
            G3_COMMENT_FIELDS,
            "Issue #61 G3",
            &second,
            false,
        )
        .is_ok()
    );
    pr.comments[0].body = pr.comments[0].body.replace(
        &format!("- Gate 断言：`{second_command}` 已通过。"),
        &format!("- Gate 断言：`{second_command}` 未通过。"),
    );
    let error = validate_g3_target(
        G3ValidationMode::ShadowTarget,
        "illusion-tech/laneflow",
        61,
        GateEvidencePhase::G3,
        &pr,
        &[first, second],
    )
    .expect_err("the non-exception Issue must retain a passing assertion");
    assert!(error.contains("必须在规范命令后明确记录 `已通过`"));
}

#[test]
fn related_full_set_preserves_current_exception_scope() {
    let (mut pr, _) = g3_exception_fixture(
        "confirmed_gate_defect",
        "G3 Exception",
        "2026-07-10T05:10:00Z",
        "2026-07-10T06:00:00Z",
    );
    let args = related_only_g3_args();
    let original_body = pr.comments[0].body.clone();
    let related_body = original_body
        .replace(DELIVERY_G3_URL, RELATED_G3_URL)
        .replace(
            &expected_gate_command(&gate_args(GateEvidencePhase::G3), GateEvidencePhase::G3),
            &expected_gate_command(&args, GateEvidencePhase::G3),
        );
    let original_hash = body_sha256(&original_body);
    let related_hash = body_sha256(&related_body);
    pr.comments[0].url = RELATED_G3_URL.to_string();
    pr.comments[0].body = related_body;
    pr.comments[1].url = pr.comments[1].url.replace("/pull/61", "/pull/62");
    pr.comments[1].body = pr.comments[1]
        .body
        .replace(r#""pullRequest": 61"#, r#""pullRequest": 62"#)
        .replace(DELIVERY_G3_URL, RELATED_G3_URL)
        .replace(&original_hash, &related_hash);
    pr.body = format!(
        "- 关联 Issue：#60\n- PR 角色：Related PR\n- [x] G3 合并判断已记录：[G3 评论]({RELATED_G3_URL})\nRefs: #60"
    );
    pr.closing_issues_references.clear();

    let scope = related_full_set_result_scope(&pr, 62, 60, &BTreeSet::from([60]), false)
        .expect("current exception scope must be derived without a historical G4 record");
    assert_eq!(scope, (60, true));
    assert!(
        validate_gate_evidence_target_assertions_with_legacy_exception(
            &pr,
            std::slice::from_ref(&args),
            Some(scope),
        )
        .is_ok()
    );
}

#[test]
fn current_exception_scope_is_independent_of_g4_appendix_presence() {
    let current_exception_issues = BTreeSet::from([60]);
    assert_eq!(
        scoped_current_g3_result(G3Result::Exception, 60, &current_exception_issues),
        G3Result::Exception
    );
    assert_eq!(
        scoped_current_g3_result(G3Result::Exception, 61, &current_exception_issues),
        G3Result::Pass
    );
}

#[test]
fn current_exception_requires_a_visible_current_head_before_early_acceptance() {
    let (mut pr, _) = g3_exception_fixture(
        "confirmed_gate_defect",
        "G3 Exception",
        "2026-07-10T05:10:00Z",
        "2026-07-10T06:00:00Z",
    );
    pr.body = format!("- 关联 Issue：#60\n- PR 角色：Delivery PR\n{}", pr.body);
    pr.comments[0].body = pr.comments[0].body.replace(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "head-not-recorded",
    );

    let error = validate_external_review_g3(
        "illusion-tech/laneflow",
        60,
        61,
        &pr,
        "Delivery PR",
        None,
        ExternalReviewG3Validation {
            phase: GateEvidencePhase::G3,
            exception_gate_time: None,
            ordinary_waiver_merged_at: None,
        },
    )
    .expect_err("a current exception must not bypass visible current-head validation");
    assert!(error.contains("GitHub PR identity 对应的完整 current head"));

    let g4_appendix = g4_comment(ISSUE_G4_URL, "2026-07-10T05:20:00Z");
    let g4_error = validate_external_review_g3(
        "illusion-tech/laneflow",
        60,
        61,
        &pr,
        "Delivery PR",
        Some(&g4_appendix),
        ExternalReviewG3Validation {
            phase: GateEvidencePhase::G4,
            exception_gate_time: pr.merged_at.as_deref(),
            ordinary_waiver_merged_at: None,
        },
    )
    .expect_err(
        "G4 replay must not let a current exception bypass visible current-head validation",
    );
    assert!(g4_error.contains("GitHub PR identity 对应的完整 current head"));
}

#[test]
fn current_g3_exception_is_auditable_non_pass_and_fails_closed() {
    let (pr, _) = g3_exception_fixture(
        "confirmed_gate_defect",
        "G3 Exception",
        "2026-07-10T05:10:00Z",
        "2026-07-10T06:00:00Z",
    );
    let args = gate_args(GateEvidencePhase::G3);
    assert!(
        validate_gate_assertion(
            &pr.comments[0].body,
            "G3 Exception",
            &args,
            GateEvidencePhase::G3
        )
        .is_ok()
    );
    assert_eq!(
        validate_g3_exception(
            60,
            61,
            &pr,
            &pr.comments[0],
            G3Result::Exception,
            None,
            G3ExceptionValidationTimes {
                gate_time: Some("2026-07-10T05:30:00Z"),
                evaluation_time: None,
            },
        ),
        Ok(true)
    );

    let (mut untrusted_marker, _) = g3_exception_fixture(
        "confirmed_gate_defect",
        "G3 Exception",
        "2026-07-10T05:10:00Z",
        "2026-07-10T06:00:00Z",
    );
    let mut malformed = untrusted_marker.comments[1].clone();
    malformed.url =
        "https://github.com/illusion-tech/laneflow/pull/61#issuecomment-untrusted-exception"
            .to_string();
    malformed.author = Some(GitHubActor {
        login: "untrusted-user".to_string(),
    });
    malformed.body = "<!-- g3-exception:v1\nnot-json".to_string();
    untrusted_marker.comments.insert(1, malformed);
    assert_eq!(
        validate_g3_exception(
            60,
            61,
            &untrusted_marker,
            &untrusted_marker.comments[0],
            G3Result::Exception,
            None,
            G3ExceptionValidationTimes {
                gate_time: Some("2026-07-10T05:30:00Z"),
                evaluation_time: None,
            },
        ),
        Ok(true)
    );

    let (expired, _) = g3_exception_fixture(
        "confirmed_gate_defect",
        "G3 Exception",
        "2026-07-10T05:10:00Z",
        "2026-07-10T05:30:00Z",
    );
    assert!(
        validate_g3_exception(
            60,
            61,
            &expired,
            &expired.comments[0],
            G3Result::Exception,
            None,
            G3ExceptionValidationTimes {
                gate_time: Some("2026-07-10T05:30:00Z"),
                evaluation_time: None,
            },
        )
        .is_err()
    );

    let (mut untrusted, _) = g3_exception_fixture(
        "confirmed_gate_defect",
        "G3 Exception",
        "2026-07-10T05:10:00Z",
        "2026-07-10T06:00:00Z",
    );
    untrusted.comments[1].author = Some(GitHubActor {
        login: "untrusted-user".to_string(),
    });
    assert!(
        validate_g3_exception(
            60,
            61,
            &untrusted,
            &untrusted.comments[0],
            G3Result::Exception,
            None,
            G3ExceptionValidationTimes {
                gate_time: Some("2026-07-10T05:30:00Z"),
                evaluation_time: None,
            },
        )
        .is_err()
    );
}

#[test]
fn current_exception_binding_uses_the_correction_restored_original_body() {
    let (mut pr, _) = g3_exception_fixture(
        "confirmed_gate_defect",
        "G3 Exception",
        "2026-07-10T05:10:00Z",
        "2026-07-10T06:00:00Z",
    );
    let corrected_body = pr.comments[0].body.clone();
    let shadow_value = "R1 non-required：exception 保持 non-success";
    let canonical_shadow = format!("{G3_EVIDENCE_SHADOW_COMMENT_FIELD}{shadow_value}");
    let wrapped_shadow = format!("{G3_EVIDENCE_SHADOW_COMMENT_FIELD}`{shadow_value}`");
    let original_body = corrected_body.replace(&canonical_shadow, &wrapped_shadow);
    assert_ne!(original_body, corrected_body);
    let original_hash = body_sha256(&original_body);
    let corrected_hash = body_sha256(&corrected_body);
    pr.comments[1].body = pr.comments[1].body.replace(&corrected_hash, &original_hash);
    {
        let g3 = &mut pr.comments[0];
        g3.body = corrected_body.clone();
        g3.updated_at = Some("2026-07-10T06:00:00Z".to_string());
        g3.includes_created_edit = true;
        g3.user_content_edits = Some(GitHubUserContentEditConnection {
            page_info: GitHubPageInfo {
                has_next_page: false,
            },
            nodes: vec![
                GitHubUserContentEdit {
                    edited_at: "2026-07-10T06:00:00Z".to_string(),
                    editor: Some(GitHubActor {
                        login: "wangzishi".to_string(),
                    }),
                    diff: Some(corrected_body),
                },
                GitHubUserContentEdit {
                    edited_at: "2026-07-10T05:00:00Z".to_string(),
                    editor: Some(GitHubActor {
                        login: "wangzishi".to_string(),
                    }),
                    diff: Some(original_body),
                },
            ],
        });
    }
    pr.state = "MERGED".to_string();
    pr.merged_at = Some("2026-07-10T05:30:00Z".to_string());

    let (_, correction_source) = post_merge_shadow_correction_fixture();
    let mut correction = correction_source.comments[1].clone();
    let source_record = parse_g3_comment_correction_record(&correction).unwrap();
    correction.body = correction
        .body
        .replace(&source_record.original_body_sha256, &original_hash)
        .replace(&source_record.new_body_sha256, &corrected_hash);
    pr.comments.push(correction);

    assert_eq!(
        validate_g3_exception(
            60,
            61,
            &pr,
            &pr.comments[0],
            G3Result::Exception,
            None,
            G3ExceptionValidationTimes {
                gate_time: pr.merged_at.as_deref(),
                evaluation_time: None,
            },
        ),
        Ok(true)
    );
}

#[test]
fn merged_current_exception_replay_preserves_the_recorded_pre_merge_base() {
    let (mut pr, _) = g3_exception_fixture(
        "confirmed_gate_defect",
        "G3 Exception",
        "2026-07-10T05:10:00Z",
        "2026-07-10T06:00:00Z",
    );
    pr.base_ref_oid = "cccccccccccccccccccccccccccccccccccccccc".to_string();
    assert!(
        validate_g3_exception(
            60,
            61,
            &pr,
            &pr.comments[0],
            G3Result::Exception,
            None,
            G3ExceptionValidationTimes {
                gate_time: Some("2026-07-10T05:30:00Z"),
                evaluation_time: None,
            },
        )
        .is_err(),
        "an open current target must still match the live base"
    );

    pr.state = "MERGED".to_string();
    pr.merged_at = Some("2026-07-10T05:30:00Z".to_string());
    assert_eq!(
        validate_g3_exception(
            60,
            61,
            &pr,
            &pr.comments[0],
            G3Result::Exception,
            None,
            G3ExceptionValidationTimes {
                gate_time: pr.merged_at.as_deref(),
                evaluation_time: None,
            },
        ),
        Ok(true)
    );
}

#[test]
fn historical_g3_block_replay_is_explicitly_non_retroactive() {
    let (mut pr, mut appendix) = g3_exception_fixture(
        "legacy_evidence_reconstruction",
        "G3 Block",
        "2026-07-10T06:00:00Z",
        "2026-07-10T07:00:00Z",
    );
    pr.merged_at = Some("2026-07-10T05:30:00Z".to_string());
    let pr_side_error = validate_g3_exception(
        60,
        61,
        &pr,
        &pr.comments[0],
        G3Result::LegacyBlock,
        None,
        G3ExceptionValidationTimes {
            gate_time: pr.merged_at.as_deref(),
            evaluation_time: None,
        },
    )
    .expect_err("historical reconstruction must be attached to the Issue G4 appendix");
    assert!(pr_side_error.contains("Issue G4 historical appendix"));

    pr.comments.truncate(1);
    appendix.url = ISSUE_G4_URL.to_string();
    assert_eq!(
        validate_g3_exception(
            60,
            61,
            &pr,
            &pr.comments[0],
            G3Result::LegacyBlock,
            Some(&appendix),
            G3ExceptionValidationTimes {
                gate_time: pr.merged_at.as_deref(),
                evaluation_time: parse_utc_timestamp_seconds("2026-07-10T06:30:00Z"),
            },
        ),
        Ok(true)
    );

    let expired_error = validate_g3_exception(
        60,
        61,
        &pr,
        &pr.comments[0],
        G3Result::LegacyBlock,
        Some(&appendix),
        G3ExceptionValidationTimes {
            gate_time: pr.merged_at.as_deref(),
            evaluation_time: parse_utc_timestamp_seconds("2026-07-10T07:00:00Z"),
        },
    )
    .expect_err("historical reconstruction must remain fresh at G4 evaluation time");
    assert!(expired_error.contains("G4 evaluation time 已过期"));
}

#[test]
fn g4_failed_assertion_requires_an_exact_structured_historical_exception_target() {
    let args = gate_args(GateEvidencePhase::G4);
    let (mut delivery, mut exception_appendix) = g3_exception_fixture(
        "legacy_evidence_reconstruction",
        "G3 Block",
        "2026-07-10T06:00:00Z",
        "2026-07-10T07:00:00Z",
    );
    delivery.comments.truncate(1);
    delivery.state = "MERGED".to_string();
    delivery.merged_at = Some("2026-07-10T05:30:00Z".to_string());
    delivery.merge_commit = Some(GitHubCommit {
        oid: MAIN_RESULT_OID.to_string(),
    });
    delivery.project_items[0].status = Some(ProjectStatus {
        name: "Done".to_string(),
    });

    let mut issue = issue("OPEN", "Done");
    let g4_fields = issue.comments[0].body.replace("` 已通过。", "` 未通过。");
    exception_appendix.url = ISSUE_G4_URL.to_string();
    exception_appendix.body = format!("{g4_fields}\n{}", exception_appendix.body);
    issue.comments[0] = exception_appendix.clone();
    let result = validate_g4_evidence(&args, &issue, &delivery, &[]);
    assert!(result.is_ok(), "{result:?}");

    issue.comments[0].body = issue.comments[0]
        .body
        .replace(r#""pullRequest": 61"#, r#""pullRequest": 999"#);
    let error = validate_g4_evidence(&args, &issue, &delivery, &[])
        .expect_err("a failed G4 assertion may not use an unrelated exception record");
    assert!(error.contains("明确记录 `已通过`"));

    issue.comments[0] = exception_appendix;
    issue.comments[0].body = issue.comments[0]
        .body
        .replace("<!-- g3-exception:v1", "<!-- g3-exception:v1\nnot-json");
    assert!(validate_g4_evidence(&args, &issue, &delivery, &[]).is_err());
}

#[test]
fn historical_exception_fixture_accepts_real_block_and_failed_assertion_shapes_only_in_replay() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/gate-evidence/historical-g3-exceptions-v1.json"
    ))
    .expect("historical exception fixture must remain valid JSON");
    let args = gate_args(GateEvidencePhase::G3);

    let block = fixture["records"][0]["gateCommentBody"].as_str().unwrap();
    assert_eq!(parse_g3_result(block), Ok(G3Result::LegacyBlock));
    assert!(
        validate_gate_assertion_with_legacy_exception(
            block,
            "legacy #372 G3",
            &args,
            GateEvidencePhase::G3,
            true,
        )
        .is_ok()
    );
    assert!(
        validate_gate_assertion_with_legacy_exception(
            block,
            "legacy #372 G3",
            &args,
            GateEvidencePhase::G3,
            false,
        )
        .is_err()
    );
    assert!(
        validate_gate_assertion_set_with_legacy_exception(
            block,
            "legacy #372 G3 set",
            std::slice::from_ref(&args),
            GateEvidencePhase::G3,
            Some((args.issue, true)),
        )
        .is_ok()
    );
    assert!(
        validate_gate_assertion_set(
            block,
            "legacy #372 G3 set",
            std::slice::from_ref(&args),
            GateEvidencePhase::G3
        )
        .is_err()
    );

    let failed_pass = fixture["records"][1]["gateCommentBody"].as_str().unwrap();
    assert_eq!(parse_g3_result(failed_pass), Ok(G3Result::Pass));
    assert!(
        validate_gate_assertion_with_legacy_exception(
            failed_pass,
            "legacy #397 G3",
            &args,
            GateEvidencePhase::G3,
            true,
        )
        .is_ok()
    );
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
    assert_eq!(
        parse_g3_result("- Gate 结果：`G3 Exception`").unwrap(),
        G3Result::Exception
    );
    assert_eq!(
        parse_g3_result("- Gate 结果：`G3 Block`").unwrap(),
        G3Result::LegacyBlock
    );
    assert!(parse_g3_result("- Gate 结果：pending").is_err());
    assert!(parse_g3_result("- Gate 结果：`G3 Pass`\n- Gate 结果：`G3 Waived`").is_err());
    assert!(validate_g3_shadow_success_result(G3Result::Pass).is_ok());
    assert!(validate_g3_shadow_success_result(G3Result::Bootstrap).is_ok());
    assert!(validate_g3_shadow_success_result(G3Result::Waived).is_err());
    assert!(validate_g3_shadow_success_result(G3Result::Exception).is_err());
    assert!(validate_g3_shadow_success_result(G3Result::LegacyBlock).is_err());
    assert_eq!(G3Result::Exception.machine_state(), "accepted_exception");
    assert_eq!(G3Result::LegacyBlock.machine_state(), "accepted_exception");
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
        id: String::new(),
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
        updated_at: None,
        user_content_edits: None,
        includes_created_edit: false,
    };
    let now = parse_utc_timestamp_seconds("2026-07-24T16:30:00Z").unwrap();
    let waiver = parse_gate_waiver(&comment, 60, now, false, false).unwrap();

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
    let error = parse_gate_waiver(&comment, 60, now, false, false)
        .expect_err("waiver author must be a trusted G3 Owner");
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
        id: String::new(),
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
        updated_at: None,
        user_content_edits: None,
        includes_created_edit: false,
    };
    let now = parse_utc_timestamp_seconds("2026-07-24T16:30:00Z").unwrap();

    assert_eq!(
        parse_gate_waiver(&comment, 60, now, false, false)
            .unwrap()
            .id,
        "waiver-60-1"
    );
    assert_eq!(
        parse_gate_waiver(&comment, 61, now, false, false)
            .unwrap()
            .id,
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
    let noncanonical_error = parse_gate_waiver(&comment, 60, now, false, false)
        .expect_err("non-canonical per-Issue waiver numbers must fail closed");
    assert!(noncanonical_error.contains("无前导零"));

    comment.body = comment.body.replace(
        r##""followUpIssue": "#060""##,
        r##""followUpIssue": "#60""##,
    );
    let error = parse_gate_waiver(&comment, 60, now, false, false)
        .expect_err("duplicate per-Issue waiver records must fail closed");
    assert!(error.contains("每个 Issue 只能包含一个"));
}

#[test]
fn rejects_expired_structured_gate_waiver() {
    let comment = GitHubComment {
        id: String::new(),
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
        updated_at: None,
        user_content_edits: None,
        includes_created_edit: false,
    };
    let after_expiry = parse_utc_timestamp_seconds("2026-07-24T17:00:01Z").unwrap();
    let current_time = waiver_validation_time(None, after_expiry).unwrap();
    let error = parse_gate_waiver(&comment, 60, current_time, false, false)
        .expect_err("current PR must reject an expired waiver");

    assert!(error.contains("已过期"));

    let merged_at = waiver_validation_time(Some("2026-07-24T16:30:00Z"), after_expiry)
        .expect("historical Related PR must use its merge time");
    assert!(parse_gate_waiver(&comment, 60, merged_at, true, false).is_ok());

    let invalid_merged_at = waiver_validation_time(Some("not-a-timestamp"), after_expiry)
        .expect_err("invalid historical mergedAt must fail closed");
    assert!(invalid_merged_at.contains("mergedAt 不是 UTC RFC3339 时间"));
}

#[test]
fn confirmed_defect_waiver_grandfathering_requires_pre_policy_g4_replay() {
    let before_policy = "2026-08-18T04:20:54Z";
    assert_eq!(
        historical_waiver_replay(GateEvidencePhase::G3, Some(before_policy)),
        Ok(HistoricalWaiverReplay {
            base_identity: false,
            grandfathered_confirmed_gate_defect: false,
        }),
        "a merged Related PR inspected during G3 is not a G4 replay"
    );
    assert_eq!(
        historical_waiver_replay(GateEvidencePhase::G4, Some(before_policy)),
        Ok(HistoricalWaiverReplay {
            base_identity: true,
            grandfathered_confirmed_gate_defect: true,
        })
    );
    assert_eq!(
        historical_waiver_replay(GateEvidencePhase::G4, Some(G3_EXCEPTION_POLICY_ACTIVATION),),
        Ok(HistoricalWaiverReplay {
            base_identity: true,
            grandfathered_confirmed_gate_defect: false,
        }),
        "the retired-type grandfather ends at activation without disabling historical base replay"
    );
    assert_eq!(
        historical_waiver_replay(GateEvidencePhase::G4, Some("2026-08-18T04:20:56Z")),
        Ok(HistoricalWaiverReplay {
            base_identity: true,
            grandfathered_confirmed_gate_defect: false,
        })
    );
    assert!(historical_waiver_replay(GateEvidencePhase::G4, None).is_err());
}

#[test]
fn delivery_ordinary_waiver_uses_g4_evaluation_time() {
    let current_time = parse_utc_timestamp_seconds("2026-08-18T10:00:00Z").unwrap();
    let merged_at = "2026-08-18T04:00:00Z";
    let merged_time = parse_utc_timestamp_seconds(merged_at).unwrap();

    assert_eq!(
        gate_waiver_evaluation_time(
            "provider_platform_outage",
            current_time,
            None,
            false,
            Some(merged_at),
        ),
        Ok(current_time),
        "an ordinary Delivery waiver must remain fresh at the G4 evaluation time"
    );
    assert_eq!(
        gate_waiver_evaluation_time(
            "confirmed_gate_defect",
            current_time,
            None,
            true,
            Some(merged_at),
        ),
        Ok(merged_time),
        "the explicit pre-policy confirmed-defect grandfather replays at merge"
    );
    assert_eq!(
        gate_waiver_evaluation_time(
            "provider_platform_outage",
            current_time,
            Some(merged_at),
            false,
            Some(merged_at),
        ),
        Ok(merged_time),
        "a merged Related member keeps its established merge-time validation"
    );
}

#[test]
fn accepts_edited_current_g3_comment_at_its_effective_time() {
    let args = related_only_g3_args();
    let issue = issue_with_pending_delivery_and_related_g3();
    let mut related_pr = related_pr_for_args(false, &args);
    related_pr.comments[0].created_at = EXTERNAL_REVIEW_G3_ACTIVATION.to_string();
    related_pr.comments[0].updated_at = Some("2026-07-24T15:16:22Z".to_string());
    related_pr.comments[0].body = gate_comment_body(CURRENT_G3_COMMENT_FIELDS, &args);
    related_pr.comments[0].includes_created_edit = true;

    assert!(validate_related_g3_evidence(&args, &issue, 62, &related_pr).is_ok());
}

#[test]
fn edited_current_g3_requires_a_hydrated_updated_at() {
    let args = related_only_g3_args();
    let issue = issue_with_pending_delivery_and_related_g3();
    let mut related_pr = related_pr_for_args(false, &args);
    related_pr.comments[0].created_at = EXTERNAL_REVIEW_G3_ACTIVATION.to_string();
    related_pr.comments[0].body = gate_comment_body(CURRENT_G3_COMMENT_FIELDS, &args);
    related_pr.comments[0].includes_created_edit = true;

    let error = validate_related_g3_evidence(&args, &issue, 62, &related_pr)
        .expect_err("edited G3 must carry the REST-backed effective time");

    assert!(error.contains("缺少 hydrated updatedAt"));
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
fn gate_commands_follow_current_msrv_and_accept_historical_versions_semantically() {
    let args = related_only_g3_args();
    let generated = expected_gate_command_with_rust_version(&args, GateEvidencePhase::G3, "1.97")
        .expect("stable workspace rust-version must generate a command");
    assert!(generated.starts_with("cargo +1.97.0 run "));

    let historical_reordered = "cargo +1.96 run --package xtask --locked -- check-gate-evidence g3 --related-pr 62 --issue 60 --repo illusion-tech/laneflow";
    assert_eq!(
        parse_gate_assertion_command_with_current_version(
            historical_reordered,
            GateEvidencePhase::G3,
            "1.97.0",
        ),
        Ok(args)
    );
}

#[test]
fn gate_commands_fail_closed_outside_the_v1_version_and_argument_boundary() {
    let base = "cargo +1.96.0 run --locked -p xtask -- check-gate-evidence g3 --repo illusion-tech/laneflow --issue 60 --related-pr 62";
    for command in [
        base.replace("+1.96.0", "+1.95.9"),
        base.replace("+1.96.0", "+1.98.0"),
        base.replace("+1.96.0", "+1.96.999"),
        base.replace("+1.96.0", "+1.96.0-beta.1"),
        base.replace("--locked", "--frozen"),
    ] {
        assert!(
            parse_gate_assertion_command_with_current_version(
                &command,
                GateEvidencePhase::G3,
                "1.97.0",
            )
            .is_err(),
            "unexpectedly accepted {command}"
        );
    }
}

#[test]
fn semantic_gate_assertions_accept_reordering_but_reject_semantic_duplicates() {
    let args = related_only_g3_args();
    let canonical = expected_gate_command(&args, GateEvidencePhase::G3);
    let reordered = "cargo +1.96 run --package xtask --locked -- check-gate-evidence g3 --related-pr 62 --issue 60 --repo illusion-tech/laneflow";
    let body = format!("{GATE_ASSERTION_PREFIX}`{reordered}` 已通过。");
    assert!(validate_gate_assertion(&body, "semantic G3", &args, GateEvidencePhase::G3).is_ok());

    let duplicate = format!("{body}\n{GATE_ASSERTION_PREFIX}`{canonical}` 已通过。");
    let error =
        validate_gate_assertion_set(&duplicate, "semantic G3", &[args], GateEvidencePhase::G3)
            .expect_err("equivalent commands with different ordering must still be duplicates");
    assert!(error.contains("同一语义命令"));
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
fn accepts_signed_post_merge_shadow_correction_during_full_set_recovery() {
    let (args, mut issue, _, related_pr) = late_related_recovery_fixture();
    let (_, delivery_pr) = post_merge_shadow_correction_fixture();
    issue.comments[0].created_at = "2026-07-10T07:00:00Z".to_string();

    let result = validate_g4_g3_full_set_recovery(&args, &issue, &delivery_pr, &[related_pr]);
    assert!(result.is_ok(), "{result:?}");
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
fn accepts_edited_delivery_g3_before_merge_during_recovery() {
    let (args, issue, mut delivery_pr, related_pr) = late_related_recovery_fixture();
    delivery_pr.comments[0].includes_created_edit = true;
    delivery_pr.comments[0].updated_at = Some("2026-07-10T05:10:00Z".to_string());

    assert!(validate_g4_g3_full_set_recovery(&args, &issue, &delivery_pr, &[related_pr]).is_ok());
}

#[test]
fn rejects_delivery_g3_edited_after_merge_during_recovery() {
    let (args, issue, mut delivery_pr, related_pr) = late_related_recovery_fixture();
    delivery_pr.comments[0].includes_created_edit = true;
    delivery_pr.comments[0].updated_at = Some("2026-07-10T05:31:00Z".to_string());

    let error = validate_g4_g3_full_set_recovery(&args, &issue, &delivery_pr, &[related_pr])
        .expect_err("an edit after merge is still retroactive evidence");

    assert!(error.contains("生效时间必须严格早于 PR 合并时间"));
}

#[test]
fn rejects_timestamp_equal_delivery_g3_during_recovery() {
    let (args, issue, mut delivery_pr, related_pr) = late_related_recovery_fixture();
    delivery_pr.comments[0].created_at = "2026-07-10T05:30:00Z".to_string();

    let error = validate_g4_g3_full_set_recovery(&args, &issue, &delivery_pr, &[related_pr])
        .expect_err("the original Delivery G3 must strictly predate its merge");

    assert!(error.contains("必须严格早于 PR 合并时间"));
}

#[test]
fn accepts_edited_related_g3_before_its_merge_during_recovery() {
    let (args, issue, delivery_pr, mut related_pr) = late_related_recovery_fixture();
    related_pr.comments[0].includes_created_edit = true;
    related_pr.comments[0].updated_at = Some("2026-07-10T05:41:00Z".to_string());

    assert!(validate_g4_g3_full_set_recovery(&args, &issue, &delivery_pr, &[related_pr]).is_ok());
}

#[test]
fn rejects_original_related_g3_effective_at_delivery_merge_second() {
    assert!(validate_original_related_g3_before_delivery_merge(62, 29, 30).is_ok());
    let error = validate_original_related_g3_before_delivery_merge(62, 30, 30)
        .expect_err("the Delivery merge second cannot prove that an original G3 edit came first");

    assert!(
        error.contains("生效时间必须严格早于 Delivery PR 合并时间"),
        "{error}"
    );
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
    delivery_pr.comments[0].updated_at = Some("2026-07-10T05:10:00Z".to_string());

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
fn rejects_merge_queue_g4_without_identity_chain() {
    let mut issue = issue("OPEN", "Done");
    issue.comments[0].body = issue.comments[0]
        .body
        .lines()
        .filter(|line| !line.starts_with("- H_mg："))
        .collect::<Vec<_>>()
        .join("\n");
    let delivery_pr = delivery_pr(Some("2026-07-10T05:30:00Z"));

    let error = validate_g4_evidence(&gate_args(GateEvidencePhase::G4), &issue, &delivery_pr, &[])
        .expect_err("Merge Queue G4 must preserve all three identities");

    assert!(error.contains("- H_mg："));
}

#[test]
fn rejects_merge_queue_g4_with_wrong_pr_or_main_identity() {
    let mut wrong_pr_issue = issue("OPEN", "Done");
    wrong_pr_issue.comments[0].body = wrong_pr_issue.comments[0].body.replace(
        &format!("- H_pr：`{DELIVERY_HEAD_OID}`"),
        "- H_pr：`eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee`",
    );
    let delivery_pr = delivery_pr(Some("2026-07-10T05:30:00Z"));

    let error = validate_g4_evidence(
        &gate_args(GateEvidencePhase::G4),
        &wrong_pr_issue,
        &delivery_pr,
        &[],
    )
    .expect_err("H_pr must match GitHub headRefOid");
    assert!(error.contains("H_pr 与 Delivery PR headRefOid 不一致"));

    let mut wrong_main_issue = issue("OPEN", "Done");
    wrong_main_issue.comments[0].body = wrong_main_issue.comments[0].body.replace(
        &format!("- H_main：`{MAIN_RESULT_OID}`"),
        "- H_main：`eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee`",
    );
    let error = validate_g4_evidence(
        &gate_args(GateEvidencePhase::G4),
        &wrong_main_issue,
        &delivery_pr,
        &[],
    )
    .expect_err("H_main must match GitHub mergeCommit");
    assert!(error.contains("H_main 与 Delivery PR mergeCommit OID 不一致"));
}

#[test]
fn rejects_merge_queue_g4_without_merge_group_check_evidence() {
    let mut issue = issue("OPEN", "Done");
    issue.comments[0].body = issue.comments[0].body.replace(
        &format!(
            "- H_mg required checks：success；{MERGE_GROUP_OID}；https://github.com/illusion-tech/laneflow/commit/{MERGE_GROUP_OID}/checks"
        ),
        "- H_mg required checks：pending",
    );
    let delivery_pr = delivery_pr(Some("2026-07-10T05:30:00Z"));

    let error = validate_g4_evidence(&gate_args(GateEvidencePhase::G4), &issue, &delivery_pr, &[])
        .expect_err("Merge Queue G4 must bind successful checks to H_mg");

    assert!(error.contains("H_mg required checks 必须同时记录"));
}

#[test]
fn accepts_pre_queue_g4_without_merge_group_fields() {
    let mut issue = issue("OPEN", "Done");
    issue.comments[0].body = issue.comments[0]
        .body
        .replace(
            "- 合并：Merge Queue（最终 Rebase）",
            "- 合并：Rebase and merge；activation 前",
        )
        .lines()
        .filter(|line| {
            !G4_MERGE_QUEUE_COMMENT_FIELDS
                .iter()
                .any(|field| line.starts_with(field))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let delivery_pr = delivery_pr(Some("2026-07-10T05:30:00Z"));

    assert!(
        validate_g4_evidence(&gate_args(GateEvidencePhase::G4), &issue, &delivery_pr, &[]).is_ok()
    );
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
