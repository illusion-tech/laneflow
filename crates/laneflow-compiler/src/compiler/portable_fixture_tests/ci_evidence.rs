use std::{
    collections::hash_map::RandomState,
    fs::{self, OpenOptions},
    hash::{BuildHasher, Hasher},
    io::Write,
    path::Path,
};

use crate::{PortableObjectCandidate, PortablePublicationCandidate};

use super::full_spatial_portable_fixture_candidate;

const EVIDENCE_DIRECTORY_ENV: &str = "LANEFLOW_PORTABLE_EVIDENCE_DIR";
const ALLOCATION_PERTURBATION_ENV: &str = "LANEFLOW_PORTABLE_ALLOCATION_PERTURBATION_BYTES";

/// 只由跨平台 CI 显式调用；普通测试不会写出证据文件。
#[test]
#[ignore = "exports production portable exact-byte evidence for cross-platform CI"]
fn portable_exact_bytes_ci_evidence() {
    let output_directory = std::env::var_os(EVIDENCE_DIRECTORY_ENV)
        .map(std::path::PathBuf::from)
        .expect("cross-platform CI must provide a fresh evidence directory");
    fs::create_dir(&output_directory).expect("evidence directory must not already exist");

    let perturbation_bytes = std::env::var(ALLOCATION_PERTURBATION_ENV)
        .expect("cross-platform CI must select an allocation perturbation")
        .parse::<usize>()
        .expect("allocation perturbation must be a usize");
    assert!((4_096..=1_048_576).contains(&perturbation_bytes));
    let allocation_perturbation = vec![0xa5_u8; perturbation_bytes].into_boxed_slice();

    // 两次完整编译各自创建新的标准库 HashMap random state。相同进程内先证明 exact bytes
    // 不依赖这些随机查找表，再由 workflow 在两个独立进程和两个 OS 之间比较实际文件。
    let first = full_spatial_portable_fixture_candidate();
    let repeated = full_spatial_portable_fixture_candidate();
    assert_same_candidate(&first, &repeated);

    write_new(
        &output_directory.join("actual.lfca"),
        first.canonical_artifact().bytes(),
    );
    write_new(
        &output_directory.join("actual.lfsm"),
        first.source_map().bytes(),
    );
    write_new(
        &output_directory.join("actual.lfsd"),
        first.semantic_diff().bytes(),
    );

    let bindings = format!(
        "supported-worker-count=1\nnetwork-revision={}\nLFCA length={} {}\nLFSM length={} {}\nLFSD length={} {}\n",
        hex_lower(first.network_revision()),
        first.canonical_artifact().byte_length(),
        first.canonical_artifact().object_key(),
        first.source_map().byte_length(),
        first.source_map().object_key(),
        first.semantic_diff().byte_length(),
        first.semantic_diff().object_key(),
    );
    write_new(&output_directory.join("bindings.txt"), bindings.as_bytes());

    let random_state = RandomState::new();
    let mut hash_canary = random_state.build_hasher();
    hash_canary.write(b"laneflow-portable-exact-bytes-ci-v1");
    let process_canary = format!(
        "hash-sample={:016x}\nallocation-perturbation-bytes={perturbation_bytes}\nallocation-canary={:p}\n",
        hash_canary.finish(),
        allocation_perturbation.as_ptr(),
    );
    write_new(
        &output_directory.join("process-canary.txt"),
        process_canary.as_bytes(),
    );
}

fn assert_same_candidate(
    first: &PortablePublicationCandidate,
    repeated: &PortablePublicationCandidate,
) {
    assert_eq!(first.network_revision(), repeated.network_revision());
    for (left, right) in [
        (first.canonical_artifact(), repeated.canonical_artifact()),
        (first.source_map(), repeated.source_map()),
        (first.semantic_diff(), repeated.semantic_diff()),
    ] {
        assert_same_object(left, right);
    }
}

fn assert_same_object(first: &PortableObjectCandidate, repeated: &PortableObjectCandidate) {
    assert_eq!(first.bytes(), repeated.bytes());
    assert_eq!(first.digest(), repeated.digest());
    assert_eq!(first.byte_length(), repeated.byte_length());
    assert_eq!(first.object_key(), repeated.object_key());
}

fn write_new(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", path.display()));
    file.write_all(bytes)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
    file.flush()
        .unwrap_or_else(|error| panic!("failed to flush {}: {error}", path.display()));
}

fn hex_lower(bytes: [u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}
