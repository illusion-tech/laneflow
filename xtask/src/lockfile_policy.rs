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
pub(crate) struct PullRequestCommit {
    pub(crate) oid: String,
    pub(crate) committed_at: String,
    pub(crate) url: String,
    pub(crate) message_headline: String,
    pub(crate) authors: Vec<CommitAuthor>,
}

#[derive(Clone, Debug)]
pub(crate) struct PullRequestMetadata {
    pub(crate) author_login: String,
    pub(crate) head_oid: String,
    pub(crate) files: Vec<ChangedFile>,
    pub(crate) commits: Vec<PullRequestCommit>,
    pub(crate) files_complete: bool,
    pub(crate) commits_complete: bool,
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
    if !is_utc_rfc3339(&commit.committed_at) {
        return Err("唯一 PR commit committedAt 不是 UTC RFC3339".to_string());
    }
    if !is_github_https_url(&commit.url) {
        return Err("唯一 PR commit URL 不是 GitHub HTTPS URL".to_string());
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

fn is_utc_rfc3339(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes.last() != Some(&b'Z')
    {
        return false;
    }
    let fixed_digits = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18];
    if fixed_digits
        .iter()
        .any(|index| !bytes[*index].is_ascii_digit())
    {
        return false;
    }
    let year = u16::from(bytes[0] - b'0') * 1_000
        + u16::from(bytes[1] - b'0') * 100
        + u16::from(bytes[2] - b'0') * 10
        + u16::from(bytes[3] - b'0');
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
        _ => return false,
    };
    if day == 0 || day > days_in_month || hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    if bytes.len() == 20 {
        return true;
    }
    bytes.len() > 21
        && bytes[19] == b'.'
        && bytes[20..bytes.len() - 1].iter().all(u8::is_ascii_digit)
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
            author_login: "dependabot[bot]".to_string(),
            head_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
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
            }],
            files_complete: true,
            commits_complete: true,
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
    fn matches_full_or_abbreviated_pr_commit_oids() {
        let metadata = eligible_metadata();
        assert!(oid_matches_any_commit("aaaaaaaa", &metadata.commits));
        assert!(!oid_matches_any_commit("bbbbbbbb", &metadata.commits));
    }
}
