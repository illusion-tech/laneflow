//! Source directory verification against §2.2 pinned digests.

use std::{
    fs::File,
    io::{BufReader, Read},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use crate::{
    Error, Result,
    source::pinned::{PINNED_SOURCE_FILES, PinnedSourceFile},
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
pub fn verify_source_dir(source_dir: &Path) -> Result<VerifiedSourceSet> {
    let mut files = Vec::with_capacity(PINNED_SOURCE_FILES.len());
    for pinned in PINNED_SOURCE_FILES {
        files.push(verify_pinned_file(source_dir, pinned)?);
    }
    Ok(VerifiedSourceSet {
        source_dir: source_dir.to_path_buf(),
        files,
    })
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
    use super::hex_digest;

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
}
