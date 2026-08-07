#[derive(Clone, Debug)]
pub(crate) struct ChangedFile {
    pub(crate) path: String,
    pub(crate) change_type: String,
}

#[derive(Clone, Debug)]
pub(crate) struct CommitAuthor {
    pub(crate) login: Option<String>,
    pub(crate) name: String,
    pub(crate) email: String,
}

#[derive(Clone, Debug)]
pub(crate) struct CommitSignature {
    pub(crate) kind: String,
    pub(crate) email: String,
    pub(crate) is_valid: bool,
    pub(crate) signer_login: Option<String>,
    pub(crate) state: String,
    pub(crate) was_signed_by_github: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ForcePush {
    pub(crate) actor_login: Option<String>,
    pub(crate) before_oid: String,
    pub(crate) after_oid: String,
    pub(crate) created_at: String,
}

#[derive(Clone, Debug)]
pub(crate) struct PullRequestCommit {
    pub(crate) oid: String,
    pub(crate) committed_at: String,
    pub(crate) url: String,
    pub(crate) message_headline: String,
    pub(crate) authors: Vec<CommitAuthor>,
    pub(crate) signature: Option<CommitSignature>,
}

#[derive(Clone, Debug)]
pub(crate) struct PullRequestMetadata {
    pub(crate) repository: String,
    pub(crate) author_login: String,
    pub(crate) head_oid: String,
    pub(crate) head_ref_name: String,
    pub(crate) head_repository_name_with_owner: String,
    pub(crate) files: Vec<ChangedFile>,
    pub(crate) commits: Vec<PullRequestCommit>,
    pub(crate) force_pushes: Vec<ForcePush>,
    pub(crate) files_complete: bool,
    pub(crate) commits_complete: bool,
    pub(crate) force_pushes_complete: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct VerifiedLockfileOnly {
    pub(crate) head_oid: String,
    pub(crate) committed_at: String,
    pub(crate) commit_url: String,
}

pub(crate) fn verify_dependabot_lockfile_only(
    metadata: &PullRequestMetadata,
) -> Result<VerifiedLockfileOnly, String> {
    if !is_dependabot_pr_author(&metadata.author_login) {
        return Err("PR author 不是 dependabot[bot]".to_string());
    }
    if metadata.head_repository_name_with_owner != metadata.repository {
        return Err("PR head repository 不是目标 repository".to_string());
    }
    if !metadata.head_ref_name.starts_with("dependabot/cargo/") {
        return Err("PR head ref 不是 Dependabot Cargo ref".to_string());
    }
    if !is_full_git_oid(&metadata.head_oid) {
        return Err("PR head 不是 40 位 Git OID".to_string());
    }
    if !metadata.files_complete {
        return Err("changed files connection 不完整".to_string());
    }
    if metadata.files.len() != 1 {
        return Err(format!(
            "changed files 必须恰好为 1，实际为 {}",
            metadata.files.len()
        ));
    }
    let file = &metadata.files[0];
    if file.path != "Cargo.lock" || file.change_type != "MODIFIED" {
        return Err(format!(
            "唯一变更必须是 MODIFIED Cargo.lock，实际为 {} {}",
            file.change_type, file.path
        ));
    }
    if !metadata.commits_complete {
        return Err("commits connection 不完整".to_string());
    }
    if metadata.commits.len() != 1 {
        return Err(format!(
            "PR commit range 必须恰好为 1 个提交，实际为 {}",
            metadata.commits.len()
        ));
    }
    let commit = &metadata.commits[0];
    if commit.oid != metadata.head_oid {
        return Err("唯一 PR commit OID 与 current head 不一致".to_string());
    }
    if !commit.message_headline.starts_with("build(deps): ") {
        return Err("唯一 PR commit 标题不是非 breaking build(deps)".to_string());
    }
    if commit.authors.len() != 1 {
        return Err(format!(
            "唯一 PR commit 必须恰好有 1 个 author，实际为 {}",
            commit.authors.len()
        ));
    }
    let author = &commit.authors[0];
    if author.name != "dependabot[bot]"
        || author.email != "49699333+dependabot[bot]@users.noreply.github.com"
        || author.login.as_deref().map(normalize_actor) != Some("dependabot".to_string())
    {
        return Err("唯一 PR commit author identity 不符合 Dependabot 窄例外".to_string());
    }
    let signature = commit
        .signature
        .as_ref()
        .ok_or_else(|| "唯一 PR commit 缺少 GitHub verified signature".to_string())?;
    if signature.kind != "GpgSignature"
        || signature.email != "noreply@github.com"
        || !signature.is_valid
        || signature.signer_login.as_deref() != Some("web-flow")
        || signature.state != "VALID"
        || !signature.was_signed_by_github
    {
        return Err("唯一 PR commit 不具有 GitHub web-flow verified signature".to_string());
    }
    if !is_utc_rfc3339(&commit.committed_at) {
        return Err("唯一 PR commit committedAt 不是 UTC RFC3339".to_string());
    }
    if !is_github_https_url(&commit.url) {
        return Err("唯一 PR commit URL 不是 GitHub HTTPS URL".to_string());
    }
    if !metadata.force_pushes_complete {
        return Err("force-push provenance connection 不完整".to_string());
    }
    for force_push in &metadata.force_pushes {
        if force_push.actor_login.as_deref().map(normalize_actor) != Some("dependabot".to_string())
            || !is_full_git_oid(&force_push.before_oid)
            || !is_full_git_oid(&force_push.after_oid)
            || !is_utc_rfc3339(&force_push.created_at)
        {
            return Err("force-push provenance 不符合 Dependabot 窄例外".to_string());
        }
    }
    if metadata
        .force_pushes
        .last()
        .is_some_and(|event| event.after_oid != metadata.head_oid)
    {
        return Err("最后一次 force-push after OID 与 current head 不一致".to_string());
    }

    Ok(VerifiedLockfileOnly {
        head_oid: metadata.head_oid.clone(),
        committed_at: commit.committed_at.clone(),
        commit_url: commit.url.clone(),
    })
}

pub(crate) fn oid_matches_any_commit(candidate: &str, commits: &[PullRequestCommit]) -> bool {
    is_git_oid_fragment(candidate)
        && commits.iter().any(|commit| {
            commit
                .oid
                .to_ascii_lowercase()
                .starts_with(&candidate.to_ascii_lowercase())
        })
}

pub(crate) fn normalize_actor(actor: &str) -> String {
    actor.trim().trim_end_matches("[bot]").to_ascii_lowercase()
}

fn is_dependabot_pr_author(actor: &str) -> bool {
    normalize_actor(actor) == "dependabot" || actor.trim().eq_ignore_ascii_case("app/dependabot")
}

fn is_full_git_oid(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn is_git_oid_fragment(value: &str) -> bool {
    (7..=40).contains(&value.len()) && value.chars().all(|character| character.is_ascii_hexdigit())
}

pub(crate) fn is_utc_rfc3339(value: &str) -> bool {
    parse_utc_rfc3339(value).is_some()
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct UtcTimestamp {
    seconds: u64,
    nanos: u32,
}

pub(crate) fn parse_utc_rfc3339(value: &str) -> Option<UtcTimestamp> {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes.last() != Some(&b'Z')
    {
        return None;
    }
    let fixed_digits = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18];
    if fixed_digits
        .iter()
        .any(|index| !bytes[*index].is_ascii_digit())
    {
        return None;
    }
    let year = u16::from(bytes[0] - b'0') * 1_000
        + u16::from(bytes[1] - b'0') * 100
        + u16::from(bytes[2] - b'0') * 10
        + u16::from(bytes[3] - b'0');
    if year == 0 {
        return None;
    }
    let month = two_digits(bytes, 5);
    let day = two_digits(bytes, 8);
    let hour = two_digits(bytes, 11);
    let minute = two_digits(bytes, 14);
    let second = two_digits(bytes, 17);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(400) || (year.is_multiple_of(4) && !year.is_multiple_of(100)) => {
            29
        }
        2 => 28,
        _ => return None,
    };
    if day == 0 || day > days_in_month || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let nanos = if bytes.len() == 20 {
        0
    } else {
        let fraction = &bytes[20..bytes.len() - 1];
        if bytes[19] != b'.'
            || !(1..=9).contains(&fraction.len())
            || !fraction.iter().all(u8::is_ascii_digit)
        {
            return None;
        }
        fraction
            .iter()
            .fold(0_u32, |value, digit| value * 10 + u32::from(*digit - b'0'))
            * 10_u32.pow(9 - fraction.len() as u32)
    };
    let days =
        days_before_year(u64::from(year)) + days_before_month(year, month) + u64::from(day - 1);
    Some(UtcTimestamp {
        seconds: days * 86_400
            + u64::from(hour) * 3_600
            + u64::from(minute) * 60
            + u64::from(second),
        nanos,
    })
}

fn days_before_year(year: u64) -> u64 {
    let years = year.saturating_sub(1);
    years * 365 + years / 4 - years / 100 + years / 400
}

fn days_before_month(year: u16, month: u8) -> u64 {
    const OFFSETS: [u64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let mut days = OFFSETS[usize::from(month - 1)];
    if month > 2
        && (year.is_multiple_of(400) || (year.is_multiple_of(4) && !year.is_multiple_of(100)))
    {
        days += 1;
    }
    days
}

fn two_digits(bytes: &[u8], start: usize) -> u8 {
    (bytes[start] - b'0') * 10 + bytes[start + 1] - b'0'
}

fn is_github_https_url(value: &str) -> bool {
    value.starts_with("https://github.com/") && !value.chars().any(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eligible_metadata() -> PullRequestMetadata {
        PullRequestMetadata {
            repository: "illusion-tech/laneflow".to_string(),
            author_login: "dependabot[bot]".to_string(),
            head_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            head_ref_name: "dependabot/cargo/toml-1.1.4".to_string(),
            head_repository_name_with_owner: "illusion-tech/laneflow".to_string(),
            files: vec![ChangedFile {
                path: "Cargo.lock".to_string(),
                change_type: "MODIFIED".to_string(),
            }],
            commits: vec![PullRequestCommit {
                oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                committed_at: "2026-08-06T02:05:11Z".to_string(),
                url: "https://github.com/illusion-tech/laneflow/commit/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                message_headline: "build(deps): Bump toml".to_string(),
                authors: vec![CommitAuthor {
                    login: Some("dependabot[bot]".to_string()),
                    name: "dependabot[bot]".to_string(),
                    email: "49699333+dependabot[bot]@users.noreply.github.com".to_string(),
                }],
                signature: Some(CommitSignature {
                    kind: "GpgSignature".to_string(),
                    email: "noreply@github.com".to_string(),
                    is_valid: true,
                    signer_login: Some("web-flow".to_string()),
                    state: "VALID".to_string(),
                    was_signed_by_github: true,
                }),
            }],
            force_pushes: vec![ForcePush {
                actor_login: Some("dependabot[bot]".to_string()),
                before_oid: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
                after_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                created_at: "2026-08-06T02:00:00Z".to_string(),
            }],
            files_complete: true,
            commits_complete: true,
            force_pushes_complete: true,
        }
    }

    #[test]
    fn accepts_only_exact_dependabot_lockfile_metadata() {
        let verified = verify_dependabot_lockfile_only(&eligible_metadata()).expect("eligible");
        assert_eq!(
            verified.head_oid,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }

    #[test]
    fn accepts_github_cli_dependabot_app_identity() {
        let mut metadata = eligible_metadata();
        metadata.author_login = "app/dependabot".to_string();
        assert!(verify_dependabot_lockfile_only(&metadata).is_ok());
    }

    #[test]
    fn rejects_source_changes_and_mixed_commit_ranges() {
        let mut metadata = eligible_metadata();
        metadata.files.push(ChangedFile {
            path: "xtask/src/main.rs".to_string(),
            change_type: "MODIFIED".to_string(),
        });
        assert!(verify_dependabot_lockfile_only(&metadata).is_err());

        let mut metadata = eligible_metadata();
        metadata.commits.push(metadata.commits[0].clone());
        assert!(verify_dependabot_lockfile_only(&metadata).is_err());
    }

    #[test]
    fn rejects_non_numeric_or_empty_fractional_commit_timestamps() {
        for committed_at in [
            "aaaa-bb-ccTdd:ee:ffZ",
            "2026-02-29T02:05:11Z",
            "2026-08-06T24:05:11Z",
            "2026-08-06T02:05:11.Z",
            "2026-08-06T02:05:11.xZ",
            "2026-08-06T02:05:11.1234567890Z",
        ] {
            let mut metadata = eligible_metadata();
            metadata.commits[0].committed_at = committed_at.to_string();
            assert!(verify_dependabot_lockfile_only(&metadata).is_err());
        }

        let mut metadata = eligible_metadata();
        metadata.commits[0].committed_at = "2026-08-06T02:05:11.123Z".to_string();
        assert!(verify_dependabot_lockfile_only(&metadata).is_ok());
    }

    #[test]
    fn rejects_spoofed_or_untrusted_dependabot_provenance() {
        let mut metadata = eligible_metadata();
        metadata.commits[0].signature = None;
        assert!(verify_dependabot_lockfile_only(&metadata).is_err());

        let mut metadata = eligible_metadata();
        metadata.force_pushes[0].actor_login = Some("wangzishi".to_string());
        assert!(verify_dependabot_lockfile_only(&metadata).is_err());

        let mut metadata = eligible_metadata();
        metadata.head_repository_name_with_owner = "fork/laneflow".to_string();
        assert!(verify_dependabot_lockfile_only(&metadata).is_err());
    }

    #[test]
    fn parses_fractional_timestamps_for_numeric_ordering() {
        assert!(
            parse_utc_rfc3339("2026-08-06T02:05:11.1Z") > parse_utc_rfc3339("2026-08-06T02:05:11Z")
        );
        assert!(parse_utc_rfc3339("2026-08-06T02:05:11.Z").is_none());
    }

    #[test]
    fn matches_full_or_abbreviated_pr_commit_oids() {
        let metadata = eligible_metadata();
        assert!(oid_matches_any_commit("aaaaaaaa", &metadata.commits));
        assert!(!oid_matches_any_commit("bbbbbbbb", &metadata.commits));
    }
}
