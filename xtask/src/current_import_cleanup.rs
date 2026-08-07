//! current 迁移导入清理责任记录（cleanup authority）的独立验证边界。
//!
//! 该记录是 #297 G2 append-only 授权记录的物化结果，固定位于
//! `docs/reference/current-import-cleanup-authority-v1.json`。本模块只从指定提交的
//! tree 读取记录 bytes（不得使用工作树或调用方提供的替代值），验证其 exact bytes、
//! 闭合结构与冻结字段规则，并提供与资产审计报告 `cleanup` 重复字段的逐项精确比较，
//! 供 G2 切片 1 的负向矩阵与后续切片 5 的完整 validator 复用同一契约。

use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const AUTHORITY_PATH: &str = "docs/reference/current-import-cleanup-authority-v1.json";
// 报告侧 profile 与绑定比较在切片 1 由负向测试冻结契约，切片 5 完整 validator 直接消费。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const REPORT_AUTHORITY_PROFILE: &str = "LF-CURRENT-CLEANUP-AUTHORITY-v1";

const AUTHORITY_SCHEMA: &str = "laneflow.current-import-cleanup-authority";
const AUTHORITY_SCHEMA_VERSION: u64 = 1;
const RETIREMENT_PROFILE: &str = "LF-CURRENT-OFFLINE-MIGRATION-RETIREMENT-v1";
const SOURCE_ISSUE_URL: &str = "https://github.com/illusion-tech/laneflow/issues/297";
const G2_EVIDENCE_PREFIX: &str =
    "https://github.com/illusion-tech/laneflow/issues/297#issuecomment-";
const ISSUE_URL_PREFIX: &str = "https://github.com/illusion-tech/laneflow/issues/";
const FORBIDDEN_CLEANUP_ISSUES: [&str; 2] = [
    "https://github.com/illusion-tech/laneflow/issues/294",
    "https://github.com/illusion-tech/laneflow/issues/297",
];

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CleanupAuthority {
    schema: String,
    schema_version: u64,
    retirement_profile: String,
    source_issue: String,
    g2_evidence: String,
    cleanup_issue: String,
    cleanup_issue_node_id: String,
    cleanup_owner: String,
}

impl CleanupAuthority {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn g2_evidence(&self) -> &str {
        &self.g2_evidence
    }

    pub(crate) fn cleanup_issue(&self) -> &str {
        &self.cleanup_issue
    }

    pub(crate) fn cleanup_issue_node_id(&self) -> &str {
        &self.cleanup_issue_node_id
    }

    pub(crate) fn cleanup_owner(&self) -> &str {
        &self.cleanup_owner
    }
}

/// 资产审计报告 `cleanup` 重复记录的清理责任字段。
///
/// `authority.{profile,path,sha256}` 加报告级 `issue`/`issueNodeId`/`owner`；
/// 全部取自报告值，预期值只能来自 A-tree 记录，不能自证。
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReportCleanupFields<'a> {
    pub(crate) authority_profile: &'a str,
    pub(crate) authority_path: &'a str,
    pub(crate) authority_sha256: &'a str,
    pub(crate) issue: &'a str,
    pub(crate) issue_node_id: &'a str,
    pub(crate) owner: &'a str,
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// 从 `commit` 的 tree 固定路径读取清理责任记录 exact bytes。
///
/// 只接受完整 40 位十六进制提交；依次验证提交存在、固定路径存在于该 tree、
/// 再按 blob id 读取原始 bytes，全程不经过工作树或普通 index。
pub(crate) fn read_authority_bytes_from_tree(
    repo_root: &Path,
    commit: &str,
) -> Result<Vec<u8>, String> {
    if commit.len() != 40
        || !commit
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(format!(
            "清理责任验证只接受完整 40 位十六进制提交：{commit}"
        ));
    }
    git_text(
        repo_root,
        &["rev-parse", "--verify", &format!("{commit}^{{commit}}")],
    )
    .map_err(|error| format!("清理责任验证的提交不存在或不是 commit：{commit}（{error}）"))?;
    let locator = format!("{commit}:{AUTHORITY_PATH}");
    let blob_id = git_text(repo_root, &["rev-parse", &locator]).map_err(|_| {
        format!("提交 {commit} 的 tree 缺少固定路径 {AUTHORITY_PATH} 的清理责任记录")
    })?;
    let blob_id = blob_id.trim();
    if blob_id.len() != 40 || !blob_id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "固定路径 {AUTHORITY_PATH} 在提交 {commit} 中未解析为 Git blob：{blob_id}"
        ));
    }
    git_bytes(repo_root, &["cat-file", "blob", blob_id])
}

/// 验证清理责任记录的 exact bytes、闭合结构与冻结字段规则。
pub(crate) fn validate_authority_bytes(bytes: &[u8]) -> Result<CleanupAuthority, String> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Err("清理责任记录不得携带 UTF-8 BOM".to_string());
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("清理责任记录不是合法 UTF-8：{error}"))?;
    if text.contains('\r') {
        return Err("清理责任记录必须使用 LF 换行，发现 CR".to_string());
    }
    if !text.ends_with('\n') || text.ends_with("\n\n") {
        return Err("清理责任记录必须以单个末尾 LF 结束".to_string());
    }
    let authority: CleanupAuthority = serde_json::from_str(text)
        .map_err(|error| format!("清理责任记录不满足 cleanupAuthorityRecord 闭合结构：{error}"))?;
    let canonical = format!(
        "{}\n",
        serde_json::to_string_pretty(&authority)
            .map_err(|error| format!("清理责任记录无法规范重序列化：{error}"))?
    );
    if canonical.as_bytes() != bytes {
        return Err(
            "清理责任记录 exact bytes 与规范序列化（两空格缩进、schema 字段顺序、单个末尾 LF）不一致"
                .to_string(),
        );
    }
    validate_authority_fields(&authority)?;
    Ok(authority)
}

/// 把 A-tree 记录作为唯一预期值，与报告 `cleanup` 重复字段逐项精确比较。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn validate_report_cleanup_binding(
    authority: &CleanupAuthority,
    authority_bytes: &[u8],
    report: &ReportCleanupFields,
) -> Result<(), String> {
    if report.authority_profile != REPORT_AUTHORITY_PROFILE {
        return Err(format!(
            "报告 cleanup.authority.profile 必须固定为 `{REPORT_AUTHORITY_PROFILE}`：{}",
            report.authority_profile
        ));
    }
    if report.authority_path != AUTHORITY_PATH {
        return Err(format!(
            "报告 cleanup.authority.path 必须固定为 `{AUTHORITY_PATH}`：{}",
            report.authority_path
        ));
    }
    let expected_sha256 = sha256_hex(authority_bytes);
    if report.authority_sha256 != expected_sha256 {
        return Err(format!(
            "报告 cleanup.authority.sha256 与 A-tree 记录 exact bytes 摘要不一致：报告 {}；预期 {expected_sha256}",
            report.authority_sha256
        ));
    }
    let field_mismatches = [
        ("issue", report.issue, authority.cleanup_issue()),
        (
            "issueNodeId",
            report.issue_node_id,
            authority.cleanup_issue_node_id(),
        ),
        ("owner", report.owner, authority.cleanup_owner()),
    ]
    .into_iter()
    .filter(|(_, actual, expected)| actual != expected)
    .map(|(field, actual, expected)| format!("{field}：报告 {actual}；预期 {expected}"))
    .collect::<Vec<_>>();
    if !field_mismatches.is_empty() {
        return Err(format!(
            "报告 cleanup 重复字段与 A-tree 清理责任记录不相等：{}",
            field_mismatches.join("；")
        ));
    }
    Ok(())
}

/// `check-current-import-cleanup-authority --commit <40-hex>`：
/// 在当前工作目录所在仓库内，从给定提交的 tree 读取并验证清理责任记录。
pub(crate) fn check_current_import_cleanup_authority(args: &[String]) -> Result<(), String> {
    let commit = match args {
        [flag, value] if flag == "--commit" => value.clone(),
        _ => {
            return Err(
                "用法：check-current-import-cleanup-authority --commit <40-hex>".to_string(),
            );
        }
    };
    let repo_root =
        std::env::current_dir().map_err(|error| format!("无法取得当前目录：{error}"))?;
    let bytes = read_authority_bytes_from_tree(&repo_root, &commit)?;
    let authority = validate_authority_bytes(&bytes)?;
    println!(
        "已校验 current 清理责任记录：commit {commit}，path {AUTHORITY_PATH}，sha256 {}，cleanupIssue {}（{}），owner {}",
        sha256_hex(&bytes),
        authority.cleanup_issue(),
        authority.cleanup_issue_node_id(),
        authority.cleanup_owner()
    );
    Ok(())
}

fn validate_authority_fields(authority: &CleanupAuthority) -> Result<(), String> {
    if authority.schema != AUTHORITY_SCHEMA {
        return Err(format!(
            "清理责任记录 schema 必须固定为 `{AUTHORITY_SCHEMA}`：{}",
            authority.schema
        ));
    }
    if authority.schema_version != AUTHORITY_SCHEMA_VERSION {
        return Err(format!(
            "清理责任记录 schemaVersion 必须固定为 {AUTHORITY_SCHEMA_VERSION}：{}",
            authority.schema_version
        ));
    }
    if authority.retirement_profile != RETIREMENT_PROFILE {
        return Err(format!(
            "清理责任记录 retirementProfile 必须固定为 `{RETIREMENT_PROFILE}`：{}",
            authority.retirement_profile
        ));
    }
    if authority.source_issue != SOURCE_ISSUE_URL {
        return Err(format!(
            "清理责任记录 sourceIssue 必须固定为 `{SOURCE_ISSUE_URL}`：{}",
            authority.source_issue
        ));
    }
    validate_g2_evidence(&authority.g2_evidence)?;
    validate_cleanup_issue(&authority.cleanup_issue)?;
    validate_cleanup_issue_node_id(&authority.cleanup_issue_node_id)?;
    validate_cleanup_owner(&authority.cleanup_owner)?;
    Ok(())
}

fn validate_g2_evidence(value: &str) -> Result<(), String> {
    let Some(suffix) = value.strip_prefix(G2_EVIDENCE_PREFIX) else {
        return Err(format!(
            "清理责任记录 g2Evidence 必须是 #297 的规范 issue-comment permalink：{value}"
        ));
    };
    if !valid_positive_decimal(suffix) {
        return Err(format!(
            "清理责任记录 g2Evidence 的 comment id 必须是无前导零正整数：{value}"
        ));
    }
    Ok(())
}

fn validate_cleanup_issue(value: &str) -> Result<(), String> {
    let Some(suffix) = value.strip_prefix(ISSUE_URL_PREFIX) else {
        return Err(format!(
            "清理责任记录 cleanupIssue 必须是本仓库 Issue 规范 URL：{value}"
        ));
    };
    if !valid_positive_decimal(suffix) {
        return Err(format!(
            "清理责任记录 cleanupIssue 的 Issue 编号必须是无前导零正整数：{value}"
        ));
    }
    if FORBIDDEN_CLEANUP_ISSUES.contains(&value) {
        return Err(format!(
            "清理责任记录 cleanupIssue 不得复用 #294 或 #297：{value}"
        ));
    }
    Ok(())
}

fn validate_cleanup_issue_node_id(value: &str) -> Result<(), String> {
    let valid = value.len() > 2
        && value.starts_with("I_")
        && value[2..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !valid {
        return Err(format!(
            "清理责任记录 cleanupIssueNodeId 必须是规范 GitHub Issue Node ID：{value}"
        ));
    }
    Ok(())
}

fn validate_cleanup_owner(value: &str) -> Result<(), String> {
    let characters = value.chars().collect::<Vec<_>>();
    let valid = !characters.is_empty()
        && characters.len() <= 39
        && characters
            .first()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && characters.last().is_some_and(|c| c.is_ascii_alphanumeric())
        && characters
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == '-');
    if !valid {
        return Err(format!(
            "清理责任记录 cleanupOwner 必须是不带 `@` 的规范 GitHub 登录名：{value}"
        ));
    }
    Ok(())
}

fn valid_positive_decimal(value: &str) -> bool {
    !value.is_empty() && !value.starts_with('0') && value.chars().all(|c| c.is_ascii_digit())
}

fn git_text(repo_root: &Path, args: &[&str]) -> Result<String, String> {
    let bytes = git_bytes(repo_root, args)?;
    String::from_utf8(bytes).map_err(|error| format!("git {} 输出不是 UTF-8：{error}", args[0]))
}

fn git_bytes(repo_root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .map_err(|error| format!("无法运行 git：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} 失败：{}",
            args[0],
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

    const VALID_RECORD: &str = "{\n  \"schema\": \"laneflow.current-import-cleanup-authority\",\n  \"schemaVersion\": 1,\n  \"retirementProfile\": \"LF-CURRENT-OFFLINE-MIGRATION-RETIREMENT-v1\",\n  \"sourceIssue\": \"https://github.com/illusion-tech/laneflow/issues/297\",\n  \"g2Evidence\": \"https://github.com/illusion-tech/laneflow/issues/297#issuecomment-5223228274\",\n  \"cleanupIssue\": \"https://github.com/illusion-tech/laneflow/issues/333\",\n  \"cleanupIssueNodeId\": \"I_kwDOS9Hn_c8AAAABL6tLzA\",\n  \"cleanupOwner\": \"wangzishi\"\n}\n";

    struct TempRepo(PathBuf);

    impl TempRepo {
        fn new() -> Self {
            let nonce = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "laneflow-xtask-cleanup-authority-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("创建临时仓库目录失败");
            let repo = Self(path);
            repo.git(&["init", "-b", "main"]);
            repo.git(&["config", "user.name", "LaneFlow Test"]);
            repo.git(&["config", "user.email", "laneflow-test@example.invalid"]);
            repo
        }

        fn git(&self, args: &[&str]) -> Vec<u8> {
            git_bytes(&self.0, args).unwrap_or_else(|error| panic!("git {:?} 失败：{error}", args))
        }

        fn write(&self, relative_path: &str, bytes: &[u8]) {
            let path = self.0.join(relative_path);
            fs::create_dir_all(path.parent().expect("测试路径必须有父目录"))
                .expect("创建测试目录失败");
            fs::write(path, bytes).expect("写入测试文件失败");
        }

        fn commit_all(&self, message: &str) -> String {
            self.git(&["add", "-A"]);
            self.git(&["commit", "-m", message]);
            String::from_utf8(self.git(&["rev-parse", "HEAD"]))
                .expect("HEAD 输出必须是 UTF-8")
                .trim()
                .to_string()
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn tampered_record(old: &str, new: &str) -> Vec<u8> {
        VALID_RECORD.replacen(old, new, 1).into_bytes()
    }

    fn valid_report_fields(authority_sha256: &str) -> ReportCleanupFields<'_> {
        ReportCleanupFields {
            authority_profile: REPORT_AUTHORITY_PROFILE,
            authority_path: AUTHORITY_PATH,
            authority_sha256,
            issue: "https://github.com/illusion-tech/laneflow/issues/333",
            issue_node_id: "I_kwDOS9Hn_c8AAAABL6tLzA",
            owner: "wangzishi",
        }
    }

    #[test]
    fn accepts_repository_committed_record() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(AUTHORITY_PATH);
        let bytes = fs::read(&path)
            .unwrap_or_else(|error| panic!("无法读取仓库清理责任记录 {}：{error}", path.display()));
        let authority = validate_authority_bytes(&bytes).expect("仓库清理责任记录必须通过验证");
        assert_eq!(
            authority.g2_evidence(),
            "https://github.com/illusion-tech/laneflow/issues/297#issuecomment-5223228274"
        );
        assert_eq!(
            authority.cleanup_issue(),
            "https://github.com/illusion-tech/laneflow/issues/333"
        );
        assert_eq!(authority.cleanup_owner(), "wangzishi");
    }

    #[test]
    fn accepts_valid_record_from_commit_tree() {
        let repo = TempRepo::new();
        repo.write(AUTHORITY_PATH, VALID_RECORD.as_bytes());
        let commit = repo.commit_all("add authority");
        let bytes = read_authority_bytes_from_tree(&repo.0, &commit).expect("A-tree 读取必须成功");
        assert_eq!(bytes, VALID_RECORD.as_bytes());
        validate_authority_bytes(&bytes).expect("A-tree 记录必须通过验证");
    }

    #[test]
    fn rejects_missing_fixed_path_and_unknown_commit() {
        let repo = TempRepo::new();
        repo.write("README.md", b"test\n");
        let commit = repo.commit_all("without authority");
        let missing =
            read_authority_bytes_from_tree(&repo.0, &commit).expect_err("固定路径缺失必须失败关闭");
        assert!(missing.contains("缺少固定路径"), "错误信息：{missing}");
        let unknown = read_authority_bytes_from_tree(&repo.0, &"0".repeat(40))
            .expect_err("未知提交必须失败关闭");
        assert!(unknown.contains("不存在"), "错误信息：{unknown}");
        let malformed = read_authority_bytes_from_tree(&repo.0, "HEAD")
            .expect_err("非完整十六进制提交必须失败关闭");
        assert!(malformed.contains("40 位十六进制"), "错误信息：{malformed}");
    }

    #[test]
    fn reads_commit_tree_bytes_ignoring_worktree_and_later_commits() {
        let repo = TempRepo::new();
        repo.write(AUTHORITY_PATH, VALID_RECORD.as_bytes());
        let commit_a = repo.commit_all("add authority");

        let tampered = tampered_record(
            "LF-CURRENT-OFFLINE-MIGRATION-RETIREMENT-v1",
            "LF-CURRENT-CLEANUP-AUTHORITY-v1",
        );
        repo.write(AUTHORITY_PATH, &tampered);
        let bytes = read_authority_bytes_from_tree(&repo.0, &commit_a)
            .expect("工作树篡改不得影响 A-tree 读取");
        assert_eq!(bytes, VALID_RECORD.as_bytes(), "读取必须来自 A 的 tree");

        let commit_e = repo.commit_all("tamper authority");
        assert_ne!(commit_e, commit_a);
        let error = validate_authority_bytes(
            &read_authority_bytes_from_tree(&repo.0, &commit_e).expect("E-tree 读取必须成功"),
        )
        .expect_err("E 上的篡改记录必须失败关闭");
        assert!(error.contains("retirementProfile"), "错误信息：{error}");
    }

    #[test]
    fn rejects_exact_bytes_tampering() {
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("UTF-8 BOM", {
                let mut bytes = vec![0xEF, 0xBB, 0xBF];
                bytes.extend_from_slice(VALID_RECORD.as_bytes());
                bytes
            }),
            ("CRLF 换行", VALID_RECORD.replace('\n', "\r\n").into_bytes()),
            (
                "缺少末尾 LF",
                VALID_RECORD.trim_end_matches('\n').as_bytes().to_vec(),
            ),
            ("双重末尾 LF", format!("{VALID_RECORD}\n").into_bytes()),
            (
                "四空格缩进",
                VALID_RECORD.replace("\n  \"", "\n    \"").into_bytes(),
            ),
            (
                "字段乱序",
                VALID_RECORD
                    .replacen(
                        "  \"schemaVersion\": 1,\n  \"retirementProfile\": \"LF-CURRENT-OFFLINE-MIGRATION-RETIREMENT-v1\",",
                        "  \"retirementProfile\": \"LF-CURRENT-OFFLINE-MIGRATION-RETIREMENT-v1\",\n  \"schemaVersion\": 1,",
                        1,
                    )
                    .into_bytes(),
            ),
            (
                "额外字段",
                VALID_RECORD
                    .replacen(
                        "\n  \"cleanupOwner\": \"wangzishi\"",
                        "\n  \"cleanupOwner\": \"wangzishi\",\n  \"extra\": true",
                        1,
                    )
                    .into_bytes(),
            ),
            (
                "缺失字段",
                VALID_RECORD
                    .replacen("  \"schemaVersion\": 1,\n", "", 1)
                    .into_bytes(),
            ),
            (
                "schemaVersion 类型错误",
                tampered_record("\"schemaVersion\": 1", "\"schemaVersion\": \"1\""),
            ),
        ];
        for (label, bytes) in cases {
            assert!(
                validate_authority_bytes(&bytes).is_err(),
                "{label} 必须失败关闭"
            );
        }
    }

    #[test]
    fn rejects_field_value_tampering() {
        let cases: Vec<(&str, Vec<u8>)> = vec![
            (
                "schema 篡改",
                tampered_record(
                    "laneflow.current-import-cleanup-authority",
                    "laneflow.current-asset-audit",
                ),
            ),
            (
                "schemaVersion 篡改",
                tampered_record("\"schemaVersion\": 1", "\"schemaVersion\": 2"),
            ),
            (
                "retirementProfile 篡改",
                tampered_record(
                    "LF-CURRENT-OFFLINE-MIGRATION-RETIREMENT-v1",
                    "LF-CURRENT-CLEANUP-AUTHORITY-v1",
                ),
            ),
            (
                "sourceIssue 篡改",
                tampered_record(
                    "\"sourceIssue\": \"https://github.com/illusion-tech/laneflow/issues/297\"",
                    "\"sourceIssue\": \"https://github.com/illusion-tech/laneflow/issues/298\"",
                ),
            ),
            (
                "g2Evidence 非 #297",
                tampered_record(
                    "issues/297#issuecomment-5223228274",
                    "issues/298#issuecomment-5223228274",
                ),
            ),
            (
                "g2Evidence 非 comment permalink",
                tampered_record(
                    "issues/297#issuecomment-5223228274",
                    "pull/330#issuecomment-5223228274",
                ),
            ),
            (
                "g2Evidence 前导零",
                tampered_record("issuecomment-5223228274", "issuecomment-05223228274"),
            ),
            (
                "cleanupIssue 冒充 #294",
                tampered_record("issues/333", "issues/294"),
            ),
            (
                "cleanupIssue 冒充 #297",
                tampered_record("issues/333", "issues/297"),
            ),
            (
                "cleanupIssue 非本仓库",
                tampered_record(
                    "https://github.com/illusion-tech/laneflow/issues/333",
                    "https://example.com/illusion-tech/laneflow/issues/333",
                ),
            ),
            (
                "cleanupIssueNodeId 缺少 I_ 前缀",
                tampered_record("\"I_kwDOS9Hn_c8AAAABL6tLzA\"", "\"kwDOS9Hn_c8AAAABL6tLzA\""),
            ),
            (
                "cleanupIssueNodeId 非法字符",
                tampered_record("I_kwDOS9Hn_c8AAAABL6tLzA", "I_kwDOS9Hn_c8AAAABL6tLz!"),
            ),
            (
                "cleanupOwner 非法首字符",
                tampered_record("wangzishi", "-wangzishi"),
            ),
            (
                "cleanupOwner 超长",
                tampered_record("wangzishi", "a234567890123456789012345678901234567890"),
            ),
        ];
        for (label, bytes) in cases {
            assert!(
                validate_authority_bytes(&bytes).is_err(),
                "{label} 必须失败关闭"
            );
        }
    }

    #[test]
    fn validates_report_cleanup_binding_field_by_field() {
        let authority_bytes = VALID_RECORD.as_bytes();
        let authority = validate_authority_bytes(authority_bytes).expect("有效记录必须通过验证");
        let digest = sha256_hex(authority_bytes);
        let valid = valid_report_fields(&digest);
        validate_report_cleanup_binding(&authority, authority_bytes, &valid)
            .expect("一致报告字段必须通过验证");

        let cases: Vec<(&str, ReportCleanupFields<'_>)> = vec![
            (
                "authority profile 篡改",
                ReportCleanupFields {
                    authority_profile: "LF-CURRENT-OFFLINE-MIGRATION-RETIREMENT-v1",
                    ..valid
                },
            ),
            (
                "authority path 篡改",
                ReportCleanupFields {
                    authority_path: "docs/reference/current-asset-audit-v1.schema.json",
                    ..valid
                },
            ),
            (
                "authority digest 篡改",
                ReportCleanupFields {
                    authority_sha256: "0000000000000000000000000000000000000000000000000000000000000000",
                    ..valid
                },
            ),
            (
                "issue 不相等",
                ReportCleanupFields {
                    issue: "https://github.com/illusion-tech/laneflow/issues/334",
                    ..valid
                },
            ),
            (
                "issueNodeId 不相等",
                ReportCleanupFields {
                    issue_node_id: "I_kwDOS9Hn_c8AAAABL6tLzB",
                    ..valid
                },
            ),
            (
                "owner 不相等",
                ReportCleanupFields {
                    owner: "mallory",
                    ..valid
                },
            ),
        ];
        for (label, report) in cases {
            assert!(
                validate_report_cleanup_binding(&authority, authority_bytes, &report).is_err(),
                "{label} 必须失败关闭"
            );
        }
    }

    #[test]
    fn rejects_malformed_numeric_and_identifier_shapes() {
        assert!(!valid_positive_decimal(""));
        assert!(!valid_positive_decimal("0"));
        assert!(!valid_positive_decimal("0333"));
        assert!(valid_positive_decimal("333"));
        assert!(validate_cleanup_issue_node_id("I_").is_err());
        assert!(validate_cleanup_issue_node_id("X_kwDOS9Hn").is_err());
        assert!(validate_cleanup_owner("").is_err());
        assert!(validate_cleanup_owner("wangzishi-").is_err());
        assert!(validate_cleanup_owner("wang zishi").is_err());
        assert!(validate_cleanup_owner("a").is_ok());
        assert!(validate_cleanup_owner("wang-zishi").is_ok());
    }
}
