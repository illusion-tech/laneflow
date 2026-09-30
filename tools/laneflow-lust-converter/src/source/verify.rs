//! Source directory verification against §2.2 pinned digests.

use std::{
    fs::File,
    io::{BufReader, Read},
    path::{Path, PathBuf},
    process::Command,
};

use sha2::{Digest, Sha256};

use crate::{
    Error, Result,
    source::pinned::{LUST_COMMIT, PINNED_SOURCE_FILES, PinnedSourceFile},
};

/// Successful verification of one pinned source file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedSourceFile {
    /// Relative path under the LuST checkout.
    pub relative_path: &'static str,
    /// Absolute path that was verified.
    pub absolute_path: PathBuf,
    /// Exact byte length.
    pub bytes: u64,
    /// Lowercase hex SHA-256.
    pub sha256_hex: String,
}

/// Successful verification of the full pinned consumption set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedSourceSet {
    /// LuST checkout root that was verified.
    pub source_dir: PathBuf,
    /// Per-file verification records in pin-table order.
    pub files: Vec<VerifiedSourceFile>,
}

/// Verify every §2.2 pinned file under `source_dir` (fail-closed).
///
/// Does not accept substitute paths from the rest of the upstream tree.
/// After all pinned files verify, the checkout revision is the final
/// authority: `source_dir` must be a git checkout whose HEAD equals
/// [`LUST_COMMIT`] — file bytes alone never prove provenance (§2.2/§9).
pub fn verify_source_dir(source_dir: &Path) -> Result<VerifiedSourceSet> {
    let mut files = Vec::with_capacity(PINNED_SOURCE_FILES.len());
    for pinned in PINNED_SOURCE_FILES {
        files.push(verify_pinned_file(source_dir, pinned)?);
    }
    let actual_revision = checkout_revision(source_dir)?;
    if actual_revision != LUST_COMMIT {
        return Err(Error::SourceRevisionMismatch {
            expected: LUST_COMMIT,
            actual: actual_revision,
        });
    }
    Ok(VerifiedSourceSet {
        source_dir: source_dir.to_path_buf(),
        files,
    })
}

/// Resolve the git HEAD revision of the checkout at `source_dir`.
///
/// Fail-closed: any probe failure (git missing, not a repository, unreadable
/// HEAD) yields [`Error::SourceRevisionUnknown`] rather than a skipped check.
fn checkout_revision(source_dir: &Path) -> Result<String> {
    let unknown = |reason: String| Error::SourceRevisionUnknown {
        source_dir: source_dir.to_path_buf(),
        reason,
    };
    let output = Command::new("git")
        .arg("-C")
        .arg(source_dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|source| unknown(format!("failed to run git rev-parse: {source}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(unknown(format!(
            "git rev-parse HEAD failed ({}): {}",
            output.status,
            stderr.trim()
        )));
    }
    let revision = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_lowercase();
    if revision.len() != 40 || !revision.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(unknown(format!(
            "git rev-parse HEAD returned unexpected output: {revision:?}"
        )));
    }
    Ok(revision)
}

fn verify_pinned_file(source_dir: &Path, pinned: &PinnedSourceFile) -> Result<VerifiedSourceFile> {
    let absolute_path = source_dir.join(pinned.relative_path);
    let file = File::open(&absolute_path).map_err(|source| match source.kind() {
        std::io::ErrorKind::NotFound => Error::MissingSourceFile {
            source_dir: source_dir.to_path_buf(),
            relative_path: pinned.relative_path,
        },
        _ => Error::Io {
            path: absolute_path.clone(),
            source,
        },
    })?;
    let metadata = file.metadata().map_err(|source| Error::Io {
        path: absolute_path.clone(),
        source,
    })?;
    let actual_bytes = metadata.len();
    if actual_bytes != pinned.bytes {
        return Err(Error::SourceSizeMismatch {
            relative_path: pinned.relative_path,
            expected: pinned.bytes,
            actual: actual_bytes,
        });
    }

    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1_024];
    loop {
        let read = reader.read(&mut buffer).map_err(|source| Error::Io {
            path: absolute_path.clone(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest: [u8; 32] = hasher.finalize().into();
    let actual_hex = hex_digest(&digest);
    if actual_hex != pinned.sha256_hex {
        return Err(Error::SourceDigestMismatch {
            relative_path: pinned.relative_path,
            expected: pinned.sha256_hex,
            actual: actual_hex,
        });
    }

    Ok(VerifiedSourceFile {
        relative_path: pinned.relative_path,
        absolute_path,
        bytes: pinned.bytes,
        sha256_hex: actual_hex,
    })
}

fn hex_digest(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::{LUST_COMMIT, checkout_revision, hex_digest, verify_source_dir};
    use crate::{Error, source::PINNED_SOURCE_FILES};

    #[test]
    fn verify_source_dir_rejects_digest_mismatch_before_revision_check() {
        let root = std::env::temp_dir().join(format!(
            "laneflow-lust-verify-digest-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp");
        // 仅摆放第一条 pinned 文件（net.xml，尺寸对齐、内容不符）：文件校验
        // 先于 revision 检查执行，digest 失配必须报 SourceDigestMismatch 而非
        // SourceRevisionUnknown——无需真实 pinned 字节或 git 仓即可锁定顺序。
        let pinned = &PINNED_SOURCE_FILES[0];
        let path = root.join(pinned.relative_path);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        std::fs::write(&path, vec![b'x'; pinned.bytes as usize]).expect("write pinned-shaped file");
        let error = verify_source_dir(&root).expect_err("corrupt pinned file must fail");
        match error {
            Error::SourceDigestMismatch {
                relative_path,
                expected,
                ..
            } => {
                assert_eq!(relative_path, pinned.relative_path);
                assert_eq!(expected, pinned.sha256_hex);
            }
            other => panic!("unexpected error: {other}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn hex_digest_encodes_lowercase() {
        let mut bytes = [0_u8; 32];
        bytes[0] = 0xab;
        bytes[31] = 0xcd;
        let encoded = hex_digest(&bytes);
        assert_eq!(encoded.len(), 64);
        assert!(encoded.starts_with("ab"));
        assert!(encoded.ends_with("cd"));
        assert!(encoded.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
    }

    #[test]
    fn checkout_revision_rejects_non_repository() {
        let root = std::env::temp_dir().join(format!(
            "laneflow-lust-verify-nonrepo-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp");
        let error = checkout_revision(&root).expect_err("non-repo must fail");
        match error {
            Error::SourceRevisionUnknown { source_dir, .. } => {
                assert_eq!(source_dir, root);
            }
            other => panic!("unexpected error: {other}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn checkout_revision_reads_git_head() {
        let root =
            std::env::temp_dir().join(format!("laneflow-lust-verify-repo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp");
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .expect("run git")
        };
        // Skip silently when git is unavailable on this host.
        match Command::new("git")
            .arg("-C")
            .arg(&root)
            .arg("init")
            .output()
        {
            Ok(output) if output.status.success() => {}
            _ => return,
        }
        std::fs::write(root.join("seed.txt"), b"seed").expect("write seed");
        assert!(git(&["add", "seed.txt"]).status.success());
        assert!(
            git(&[
                "-c",
                "user.name=lust-test",
                "-c",
                "user.email=lust-test@example.invalid",
                "commit",
                "-m",
                "seed",
            ])
            .status
            .success()
        );
        let revision = checkout_revision(&root).expect("read HEAD");
        assert_eq!(revision.len(), 40);
        assert!(revision.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
        assert_ne!(revision, LUST_COMMIT);
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// 读入 verify-source 快照文件并在**消费时**重算 SHA-256 与验证记录比对：
/// 验证与消费之间字节被换（TOCTOU）即 fail-closed，消费字节由此与
/// pinned 校验绑定（#253 R2）。pipeline 与 `prepare_verified_lust_inputs`
/// 共用本实现。
pub(crate) fn read_verified(verified: &VerifiedSourceSet, relative_path: &str) -> Result<String> {
    let file = verified
        .files
        .iter()
        .find(|file| file.relative_path == relative_path)
        .ok_or_else(|| Error::SumoModel(format!("verified set missing {relative_path}")))?;
    let bytes = std::fs::read(&file.absolute_path).map_err(|source| Error::Io {
        path: file.absolute_path.clone(),
        source,
    })?;
    let actual = crate::output::digest::hex_sha256(&bytes);
    if actual != file.sha256_hex {
        return Err(Error::SourceChangedAfterVerification {
            relative_path: file.relative_path,
            expected: file.sha256_hex.clone(),
            actual,
        });
    }
    String::from_utf8(bytes)
        .map_err(|_| Error::SumoModel(format!("verified {relative_path} is not UTF-8")))
}

/// 诊断清单模式的已验证输入（#253 R2 第四轮）：`prepare_verified_lust_inputs`
/// 的返回——绑定后的三份转换输入与 verified 来源声明。外部调用方无法自行
/// 构造 `verified = true` 的 [`ReportSource`]（字段私有），已验证声明只能
/// 经本路径或 crate 内 pipeline 获得。字段私有 + 只读 getter（#253 R8
/// 残留缺口）：防止「合法记录 + 改动后字节」错配后经组合入口套取 0.5 m
/// 例外——组合入口在消费时重算摘要，错配即 fail-closed。
pub struct VerifiedLustInputs {
    /// 消费时重哈希绑定后的 `scenario/lust.net.xml` 文本。
    net_xml: String,
    /// 消费时重哈希绑定后的 `scenario/tll.static.xml` 文本。
    tll_xml: String,
    /// 消费时重哈希绑定后的 `scenario/vtypes.add.xml` 文本。
    vtypes_xml: String,
    /// verified = true、摘要取 net 消费字节的来源声明。
    report_source: crate::output::geom::ReportSource,
}

impl VerifiedLustInputs {
    /// 绑定后的 `scenario/lust.net.xml` 文本（只读）。
    pub fn net_xml(&self) -> &str {
        &self.net_xml
    }

    /// 绑定后的 `scenario/tll.static.xml` 文本（只读）。
    pub fn tll_xml(&self) -> &str {
        &self.tll_xml
    }

    /// 绑定后的 `scenario/vtypes.add.xml` 文本（只读）。
    pub fn vtypes_xml(&self) -> &str {
        &self.vtypes_xml
    }

    /// verified 来源声明（只读；`Clone` 出的旧记录错配改动字节会被组合入口
    /// 的消费时重算绑定拒绝）。
    pub fn report_source(&self) -> &crate::output::geom::ReportSource {
        &self.report_source
    }
}

impl std::fmt::Debug for VerifiedLustInputs {
    /// 手动 Debug：字节文本可达 10 MB 级，失败时只报长度与来源声明。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedLustInputs")
            .field("net_xml", &format_args!("{} bytes", self.net_xml.len()))
            .field("tll_xml", &format_args!("{} bytes", self.tll_xml.len()))
            .field(
                "vtypes_xml",
                &format_args!("{} bytes", self.vtypes_xml.len()),
            )
            .field("report_source", &self.report_source)
            .finish()
    }
}

/// 全部 pinned 文件消费完毕后的 revision 重校验（#253 K1）：verify-source
/// 只在入口查一次 HEAD——检查与消费之间 checkout 被切到「保留了 pinned
/// 字节的其他 HEAD」时，文件 digest 全过而 provenance 的 revision 声称失真。
/// 漂移即 fail-closed（SourceRevisionMismatch）。
pub fn recheck_source_revision(source_dir: &Path) -> Result<()> {
    let actual = checkout_revision(source_dir)?;
    if actual != LUST_COMMIT {
        return Err(Error::SourceRevisionMismatch {
            expected: LUST_COMMIT,
            actual,
        });
    }
    Ok(())
}

/// 正式诊断入口的准备函数：先验证、再绑定、后转换（#253 R2 第四轮）。
///
/// 1. `verify_source_dir`：checkout revision 等于 pinned commit + 全部 §2.2
///    pinned digest 校验（文件失配、HEAD 错误、无仓库在此 fail-closed——
///    即任何 normalize/转换之前）。
/// 2. 三份诊断输入逐份 `read_verified`：消费时重哈希与验证记录比对
///    （TOCTOU 闭合），字节在验证后被换即拒绝。
/// 3. 构造 `ReportSource::verified`（crate 内受控构造器，摘要为 net 消费字节）。
pub fn prepare_verified_lust_inputs(source_dir: &Path) -> Result<VerifiedLustInputs> {
    let verified = verify_source_dir(source_dir)?;
    let net_xml = read_verified(&verified, "scenario/lust.net.xml")?;
    let tll_xml = read_verified(&verified, "scenario/tll.static.xml")?;
    let vtypes_xml = read_verified(&verified, "scenario/vtypes.add.xml")?;
    let report_source = crate::output::geom::ReportSource::verified(
        crate::output::digest::sha256_digest(net_xml.as_bytes()),
    );
    Ok(VerifiedLustInputs {
        net_xml,
        tll_xml,
        vtypes_xml,
        report_source,
    })
}

#[cfg(test)]
mod recheck_tests {
    use super::{LUST_COMMIT, recheck_source_revision};
    use crate::Error;

    #[test]
    fn recheck_rejects_wrong_head_after_consumption() {
        // #253 K1：全部文件消费完毕后的 revision 重校验——checkout 被切到
        // 其他 HEAD（即便 pinned 字节保留）即 fail-closed。
        let root =
            std::env::temp_dir().join(format!("laneflow-lust-recheck-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp");
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .expect("run git")
        };
        if !git(&["init"]).status.success() {
            let _ = std::fs::remove_dir_all(&root);
            return;
        }
        std::fs::write(root.join("seed.txt"), b"seed").expect("write seed");
        assert!(git(&["add", "seed.txt"]).status.success());
        assert!(
            git(&[
                "-c",
                "user.name=lust-test",
                "-c",
                "user.email=lust-test@example.invalid",
                "commit",
                "-m",
                "seed",
            ])
            .status
            .success()
        );
        let error = recheck_source_revision(&root).expect_err("wrong HEAD must fail");
        match error {
            Error::SourceRevisionMismatch { expected, actual } => {
                assert_eq!(expected, LUST_COMMIT);
                assert_ne!(actual, LUST_COMMIT);
            }
            other => panic!("unexpected error: {other}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod read_verified_tests {
    use super::{VerifiedSourceFile, VerifiedSourceSet, read_verified};
    use crate::Error;

    fn snapshot_with(
        root: &std::path::Path,
        relative_path: &'static str,
        bytes: &[u8],
    ) -> VerifiedSourceSet {
        let absolute_path = root.join(relative_path);
        std::fs::create_dir_all(absolute_path.parent().expect("parent dir"))
            .expect("create parent");
        std::fs::write(&absolute_path, bytes).expect("write snapshot file");
        VerifiedSourceSet {
            source_dir: root.to_path_buf(),
            files: vec![VerifiedSourceFile {
                relative_path,
                absolute_path,
                bytes: bytes.len() as u64,
                sha256_hex: crate::output::digest::hex_sha256(bytes),
            }],
        }
    }

    #[test]
    fn read_verified_accepts_intact_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "laneflow-lust-verify-intact-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp");
        let snapshot = snapshot_with(&root, "scenario/lust.net.xml", b"<net/>");
        let text =
            read_verified(&snapshot, "scenario/lust.net.xml").expect("intact snapshot reads");
        assert_eq!(text, "<net/>");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_verified_rejects_bytes_changed_after_verification() {
        let root = std::env::temp_dir().join(format!(
            "laneflow-lust-verify-toctou-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp");
        let snapshot = snapshot_with(&root, "scenario/lust.net.xml", b"<net/>");
        // 验证与消费之间文件被换（TOCTOU）：同尺寸换内容必须 fail-closed。
        std::fs::write(&snapshot.files[0].absolute_path, b"<NET/>").expect("swap bytes");
        let error = read_verified(&snapshot, "scenario/lust.net.xml")
            .expect_err("changed bytes must fail closed");
        match error {
            Error::SourceChangedAfterVerification {
                relative_path,
                expected,
                actual,
            } => {
                assert_eq!(relative_path, "scenario/lust.net.xml");
                assert_eq!(expected, crate::output::digest::hex_sha256(b"<net/>"));
                assert_eq!(actual, crate::output::digest::hex_sha256(b"<NET/>"));
            }
            other => panic!("unexpected error: {other}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_verified_reports_snapshot_missing_file() {
        let root = std::env::temp_dir().join(format!(
            "laneflow-lust-verify-missing-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp");
        let snapshot = snapshot_with(&root, "scenario/lust.net.xml", b"<net/>");
        let error = read_verified(&snapshot, "scenario/tll.static.xml")
            .expect_err("unverified file must not be consumed");
        match error {
            Error::SumoModel(message) => {
                assert!(message.contains("scenario/tll.static.xml"), "{message}");
            }
            other => panic!("unexpected error: {other}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
