//! Gate evidence 单元测试夹具。

use super::*;

pub(super) const DELIVERY_G3_URL: &str =
    "https://github.com/illusion-tech/laneflow/pull/61#issuecomment-100";
pub(super) const ISSUE_G4_URL: &str =
    "https://github.com/illusion-tech/laneflow/issues/60#issuecomment-200";
pub(super) const RELATED_G3_URL: &str =
    "https://github.com/illusion-tech/laneflow/pull/62#issuecomment-300";

pub(super) fn gate_comment_body(required_fields: &[&str], args: &GateEvidenceArgs) -> String {
    required_fields
        .iter()
        .map(|field| {
            if *field == GATE_ASSERTION_PREFIX {
                format!(
                    "{GATE_ASSERTION_PREFIX}`{}` 已通过。",
                    expected_gate_command(args, args.phase)
                )
            } else if *field == "- Gate 结果：" {
                format!("{field}`R0-R1 bootstrap`")
            } else {
                (*field).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn g3_comment_for_args(
    url: &str,
    created_at: &str,
    args: &GateEvidenceArgs,
) -> GitHubComment {
    GitHubComment {
        url: url.to_string(),
        body: gate_comment_body(G3_COMMENT_FIELDS, args),
        author: Some(GitHubActor {
            login: "wangzishi".to_string(),
        }),
        created_at: created_at.to_string(),
        includes_created_edit: false,
    }
}

pub(super) fn g3_comment(url: &str, created_at: &str) -> GitHubComment {
    g3_comment_for_args(url, created_at, &gate_args(GateEvidencePhase::G3))
}

pub(super) fn g4_comment_for_args(
    url: &str,
    created_at: &str,
    args: &GateEvidenceArgs,
) -> GitHubComment {
    GitHubComment {
        url: url.to_string(),
        body: gate_comment_body(G4_COMMENT_FIELDS, args),
        author: Some(GitHubActor {
            login: "wangzishi".to_string(),
        }),
        created_at: created_at.to_string(),
        includes_created_edit: false,
    }
}

pub(super) fn g4_comment(url: &str, created_at: &str) -> GitHubComment {
    g4_comment_for_args(url, created_at, &gate_args(GateEvidencePhase::G4))
}

pub(super) fn g4_recovery_comment(args: &GateEvidenceArgs) -> GitHubComment {
    let mut comment = g4_comment_for_args(ISSUE_G4_URL, "2026-07-10T06:00:00Z", args);
    comment.body = comment.body.replace(
        "- 关系：",
        "- 关系：[Delivery G3][delivery-g3]、[Related G3][related-g3]。",
    );
    comment.body.push_str(
        r##"
<!-- g3-full-set-recovery:v1
{
  "schemaVersion": 1,
  "exceptionType": "late_related_after_delivery_merge",
  "issue": 60,
  "deliveryPr": 61,
  "deliveryMergedAt": "2026-07-10T05:30:00Z",
  "originalRelatedPrs": [],
  "lateRelatedPrs": [62],
  "reason": "publication gap was discovered after Delivery merge",
  "evidenceRefs": ["delivery-g3", "related-g3"],
  "risk": "historical Delivery G3 cannot name a future Related PR",
  "acceptanceBoundary": "G4 recovery only; normal G3 remains strict",
  "followUpIssue": "#246",
  "cleanupOwner": "wangzishi",
  "authorizedBy": "wangzishi"
}
-->

[delivery-g3]: https://github.com/illusion-tech/laneflow/pull/61#issuecomment-100
[related-g3]: https://github.com/illusion-tech/laneflow/pull/62#issuecomment-300
"##,
    );
    comment
}

pub(super) fn issue_reference(repo: &str, number: u64) -> IssueReference {
    let (owner, name) = repo.split_once('/').expect("test repository must be valid");
    IssueReference {
        number,
        repository: IssueReferenceRepository {
            name: name.to_string(),
            owner: GitHubActor {
                login: owner.to_string(),
            },
        },
    }
}

pub(super) fn delivery_pr(merged_at: Option<&str>) -> GitHubPullRequest {
    GitHubPullRequest {
        body: format!(
            "- [x] G3 合并判断已记录：[G3 评论]({DELIVERY_G3_URL})\n- G4 回写：[Issue G4 评论]({ISSUE_G4_URL})"
        ),
        state: if merged_at.is_some() {
            "MERGED".to_string()
        } else {
            "OPEN".to_string()
        },
        is_draft: false,
        created_at: "2026-07-10T04:00:00Z".to_string(),
        merged_at: merged_at.map(ToOwned::to_owned),
        closing_issues_references: vec![issue_reference("illusion-tech/laneflow", 60)],
        project_items: vec![ProjectItem {
            title: "LaneFlow".to_string(),
            status: Some(ProjectStatus {
                name: if merged_at.is_some() {
                    "Done".to_string()
                } else {
                    "In Review".to_string()
                },
            }),
        }],
        comments: vec![g3_comment(DELIVERY_G3_URL, "2026-07-10T05:00:00Z")],
    }
}

pub(super) fn issue(state: &str, project_status: &str) -> GitHubIssue {
    GitHubIssue {
        body: format!(
            "- Delivery PR：#61\n- Related PRs：N/A，原因：无部分交付。\n- [x] G0 立项已记录：\n- [x] G1 设计判断已记录：\n- [x] G2 开工判断已记录：\n- [x] G3 合并判断已记录：[Delivery G3 评论]({DELIVERY_G3_URL})\n- [x] G4 完成判断已记录：[G4 评论]({ISSUE_G4_URL})"
        ),
        state: state.to_string(),
        project_items: vec![ProjectItem {
            title: "LaneFlow".to_string(),
            status: Some(ProjectStatus {
                name: project_status.to_string(),
            }),
        }],
        comments: vec![g4_comment(ISSUE_G4_URL, "2026-07-10T06:00:00Z")],
    }
}

pub(super) fn related_pr(closes_issue: bool) -> GitHubPullRequest {
    let args = related_only_g3_args();
    related_pr_for_args(closes_issue, &args)
}

pub(super) fn related_pr_for_args(
    closes_issue: bool,
    args: &GateEvidenceArgs,
) -> GitHubPullRequest {
    GitHubPullRequest {
        body: format!(
            "- 关联 Issue：#{}\n- PR 角色：Related PR\n- [x] G3 合并判断已记录：[G3 评论]({RELATED_G3_URL})\nRefs: #{}",
            args.issue, args.issue
        ),
        state: "OPEN".to_string(),
        is_draft: false,
        created_at: "2026-07-10T04:30:00Z".to_string(),
        merged_at: None,
        closing_issues_references: closes_issue
            .then_some(vec![issue_reference("illusion-tech/laneflow", 60)])
            .unwrap_or_default(),
        project_items: vec![ProjectItem {
            title: "LaneFlow".to_string(),
            status: Some(ProjectStatus {
                name: "In Review".to_string(),
            }),
        }],
        comments: vec![g3_comment_for_args(
            RELATED_G3_URL,
            "2026-07-10T05:00:00Z",
            args,
        )],
    }
}

pub(super) fn gate_args(phase: GateEvidencePhase) -> GateEvidenceArgs {
    GateEvidenceArgs {
        phase,
        repo: "illusion-tech/laneflow".to_string(),
        issue: 60,
        delivery_pr: Some(61),
        related_prs: Vec::new(),
    }
}

pub(super) fn related_only_g3_args() -> GateEvidenceArgs {
    GateEvidenceArgs {
        phase: GateEvidencePhase::G3,
        repo: "illusion-tech/laneflow".to_string(),
        issue: 60,
        delivery_pr: None,
        related_prs: vec![62],
    }
}

pub(super) fn issue_with_pending_delivery_and_related_g3() -> GitHubIssue {
    let mut issue = issue("OPEN", "In Review");
    issue.body = issue
        .body
        .replace("- Delivery PR：#61", "- Delivery PR：pending")
        .replace("Related PRs：N/A，原因：无部分交付。", "Related PRs：#62")
        .replace(
            &format!("- [x] G3 合并判断已记录：[Delivery G3 评论]({DELIVERY_G3_URL})"),
            &format!("- [ ] G3 合并判断已记录：[Related G3 评论]({RELATED_G3_URL})"),
        );
    issue
}

pub(super) fn late_related_recovery_fixture() -> (
    GateEvidenceArgs,
    GitHubIssue,
    GitHubPullRequest,
    GitHubPullRequest,
) {
    let mut args = gate_args(GateEvidencePhase::G4);
    args.related_prs = vec![62];
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
    issue.comments[0] = g4_recovery_comment(&args);
    let delivery_pr = delivery_pr(Some("2026-07-10T05:30:00Z"));
    let mut related_pr = related_pr(false);
    related_pr.state = "MERGED".to_string();
    related_pr.created_at = "2026-07-10T05:31:00Z".to_string();
    related_pr.merged_at = Some("2026-07-10T05:50:00Z".to_string());
    related_pr.project_items[0].status = Some(ProjectStatus {
        name: "Done".to_string(),
    });
    related_pr.comments[0].created_at = "2026-07-10T05:40:00Z".to_string();
    (args, issue, delivery_pr, related_pr)
}
