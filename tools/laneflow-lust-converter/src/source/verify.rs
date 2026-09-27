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
    let revision = String::from_utf8_lossy(&output.stdout).trim().to_lowercase();
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

    use super::{LUST_COMMIT, checkout_revision, hex_digest};
    use crate::Error;

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
        let root = std::env::temp_dir().join(format!(
            "laneflow-lust-verify-repo-{}",
            std::process::id()
        ));
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
