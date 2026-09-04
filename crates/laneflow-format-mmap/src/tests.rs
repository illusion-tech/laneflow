use super::*;
use std::{
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

fn create_test_directory(label: &str) -> PathBuf {
    for _ in 0..256 {
        let ordinal = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "laneflow-format-mmap-{label}-{}-{ordinal}",
            std::process::id()
        ));
        match std::fs::create_dir(&path) {
            Ok(()) => return path,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("create test directory: {error}"),
        }
    }
    panic!("could not reserve a unique staged-backing test directory");
}

#[test]
fn staged_bytes_roundtrip_through_read_only_map() {
    let directory = create_test_directory("roundtrip");
    let mut staged = PrivateStagedFile::create_in(&directory).expect("create staged backing");
    staged.write_all(&[0xa5, 0x5a]).expect("write staged bytes");
    staged.flush().expect("flush staged bytes");

    let sealed = staged.seal(2).expect("seal with matching length");
    assert_eq!(sealed.exact_byte_length(), 2);
    let first = sealed.map_read_only().expect("map read only");
    let second = sealed.map_read_only().expect("map read only twice");
    assert_eq!(&first[..], &[0xa5, 0x5a]);
    assert_eq!(&second[..], &[0xa5, 0x5a]);

    drop((first, second, sealed));
    std::fs::remove_dir(&directory).expect("remove empty test directory");
}

#[test]
fn patch_exact_at_overwrites_in_place_and_restores_write_position() {
    let directory = create_test_directory("patch");
    let mut staged = PrivateStagedFile::create_in(&directory).expect("create staged backing");
    staged
        .write_all(&[0xaa, 0xbb, 0xcc, 0xdd])
        .expect("write staged bytes");
    staged
        .patch_exact_at(1, &[0x11, 0x22], 4)
        .expect("patch in place");
    // 写位置已恢复到 resume：继续顺序写是追加，不是覆写。
    staged.write_all(&[0xee]).expect("append after patch");
    staged.flush().expect("flush patched bytes");

    let sealed = staged.seal(5).expect("seal with patched length");
    let map = sealed.map_read_only().expect("map read only");
    assert_eq!(&map[..], &[0xaa, 0x11, 0x22, 0xdd, 0xee]);

    drop((map, sealed));
    std::fs::remove_dir(&directory).expect("remove empty test directory");
}

#[test]
fn seal_rejects_length_mismatch_before_mapping() {
    let directory = create_test_directory("seal-mismatch");
    let mut staged = PrivateStagedFile::create_in(&directory).expect("create staged backing");
    staged.write_all(&[0xa5]).expect("write one byte");
    staged.flush().expect("flush one byte");

    assert!(matches!(staged.seal(2), Err(BackingError::BackingChanged)));
    std::fs::remove_dir(&directory).expect("remove empty test directory");
}

#[test]
fn map_rechecks_length_on_every_call() {
    let directory = create_test_directory("map-recheck");
    let mut staged = PrivateStagedFile::create_in(&directory).expect("create staged backing");
    staged.write_all(&[0xa5, 0x5a]).expect("write staged bytes");
    staged.flush().expect("flush staged bytes");
    let sealed = staged.seal(2).expect("seal with matching length");
    sealed.map_read_only().expect("map with matching length");

    // 经平台句柄原地截断模拟 backing 变化（测试在同一进程内持有写能力，
    // 不违反模块文档的外部写者论证）。
    sealed.file.set_len(1).expect("truncate backing in place");
    assert!(matches!(
        sealed.map_read_only(),
        Err(BackingError::BackingChanged)
    ));
    drop(sealed);
    std::fs::remove_dir(&directory).expect("remove empty test directory");
}

#[cfg(unix)]
#[test]
fn unix_private_backing_has_no_directory_entry_while_open() {
    let directory = create_test_directory("unix-unlinked");
    let staged = PrivateStagedFile::create_in(&directory).expect("private staged backing");

    assert_eq!(
        std::fs::read_dir(&directory)
            .expect("read test directory")
            .count(),
        0
    );

    drop(staged);
    std::fs::remove_dir(&directory).expect("remove empty test directory");
}

#[cfg(windows)]
#[test]
fn windows_private_backing_rejects_reopen_until_delete_on_close() {
    let directory = create_test_directory("windows-exclusive");
    let staged = PrivateStagedFile::create_in(&directory).expect("private staged backing");
    let entry = std::fs::read_dir(&directory)
        .expect("read test directory")
        .next()
        .expect("delete-on-close entry")
        .expect("directory entry");

    let reopen = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(entry.path());
    assert!(reopen.is_err(), "exclusive staged backing was reopened");

    drop(staged);
    assert_eq!(
        std::fs::read_dir(&directory)
            .expect("read test directory")
            .count(),
        0
    );
    std::fs::remove_dir(&directory).expect("remove empty test directory");
}
