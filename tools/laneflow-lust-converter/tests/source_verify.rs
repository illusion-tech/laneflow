use std::{
    fs,
    path::{Path, PathBuf},
};

use laneflow_lust_converter::{
    Error, LustConverterConfig, PINNED_SOURCE_FILES, load_config, verify_source,
};
use sha2::{Digest, Sha256};

#[test]
fn pinned_table_matches_contract_shape() {
    assert_eq!(PINNED_SOURCE_FILES.len(), 8);
    for file in PINNED_SOURCE_FILES {
        assert!(!file.relative_path.is_empty());
        assert!(file.bytes > 0);
        assert_eq!(file.sha256_hex.len(), 64);
        assert!(
            file.sha256_hex
                .chars()
                .all(|c| matches!(c, '0'..='9' | 'a'..='f'))
        );
    }
}

#[test]
fn verify_source_rejects_missing_file() {
    let root = temp_dir("missing");
    let error = verify_source(&root).expect_err("missing file must fail");
    match error {
        Error::MissingSourceFile { relative_path, .. } => {
            assert_eq!(relative_path, "scenario/lust.net.xml");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn verify_source_rejects_size_mismatch() {
    let root = temp_dir("size");
    write_stub_tree(&root, b"too-small");
    let error = verify_source(&root).expect_err("size mismatch must fail");
    match error {
        Error::SourceSizeMismatch {
            relative_path,
            expected,
            actual,
        } => {
            assert_eq!(relative_path, "scenario/lust.net.xml");
            assert_eq!(expected, 10_940_662);
            assert_eq!(actual, 9);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn verify_source_rejects_digest_mismatch_when_size_matches() {
    let root = temp_dir("digest");
    let first = &PINNED_SOURCE_FILES[0];
    let mut bytes = vec![0_u8; first.bytes as usize];
    bytes[0] = 1;
    write_sized_first_file(&root, &bytes);
    // Later pinned files intentionally omitted: verification fails on the first digest.
    let error = verify_source(&root).expect_err("digest mismatch must fail");
    match error {
        Error::SourceDigestMismatch {
            relative_path,
            expected,
            actual,
        } => {
            assert_eq!(relative_path, first.relative_path);
            assert_eq!(expected, first.sha256_hex);
            assert_ne!(actual, expected);
            assert_eq!(actual.len(), 64);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn load_config_reads_toml() {
    let root = temp_dir("config");
    let path = root.join("lust.toml");
    fs::write(
        &path,
        "source_dir = \"C:/tmp/lust\"\noutput_dir = \"C:/tmp/out\"\n",
    )
    .expect("write config");
    let config = load_config(&path).expect("load config");
    assert_eq!(
        config,
        LustConverterConfig {
            source_dir: PathBuf::from("C:/tmp/lust"),
            output_dir: PathBuf::from("C:/tmp/out"),
        }
    );
}

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "laneflow-lust-converter-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create temp");
    path
}

fn write_stub_tree(root: &Path, first_contents: &[u8]) {
    write_bytes(
        &root.join(PINNED_SOURCE_FILES[0].relative_path),
        first_contents,
    );
}

fn write_sized_first_file(root: &Path, contents: &[u8]) {
    write_bytes(&root.join(PINNED_SOURCE_FILES[0].relative_path), contents);
}

fn write_bytes(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, contents).expect("write file");
}

#[test]
fn sha256_of_known_vector() {
    let digest: [u8; 32] = Sha256::digest(b"abc").into();
    let mut encoded = String::new();
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    assert_eq!(
        encoded,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
