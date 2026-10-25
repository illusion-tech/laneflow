use std::{fs, path::Path, process::Command};

use laneflow_urban_generator::verified_write::{
    SourceRegistration, WritePolicy, test_support::with_after_write, write_verified,
};

use crate::child_diagnostics::{DiagnosticDirectory, RecoveryCounts, output, output_with};

fn command(mode: &str, target: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            "child_diagnostics_tests::fault_worker",
            "--nocapture",
        ])
        .env("LF_CHILD_DIAGNOSTIC_TEST_MODE", mode)
        .env("LF_CHILD_DIAGNOSTIC_TEST_PATH", target);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

fn receipts(root: &Path) -> Vec<serde_json::Value> {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            serde_json::from_slice(&fs::read(entry.unwrap().path().join("capture.json")).unwrap())
                .unwrap()
        })
        .collect()
}

#[test]
fn nested_child_logs_retain_recovery_and_exhaustion_once() {
    let archive = tempfile::tempdir().unwrap();
    let mut directory = DiagnosticDirectory::new().unwrap();
    let root = directory.path().to_owned();
    fs::create_dir(root.join("nested")).unwrap();
    let mut expected = Vec::new();
    for (mode, file) in [
        ("once", "resident.stderr.log"),
        ("always", "nested/replay.stderr.log"),
    ] {
        let path = root.join(file);
        let status = command(mode, &root.join(format!("{mode}.bin")))
            .stdout(std::process::Stdio::null())
            .stderr(fs::File::create(&path).unwrap())
            .status()
            .unwrap();
        assert_eq!(status.success(), mode == "once");
        expected.push(fs::read(path).unwrap());
    }
    let mut forwarded = Vec::new();
    let counts = directory
        .finish_with(Some(archive.path()), &mut forwarded)
        .unwrap();
    assert_eq!(counts.records, 5);
    assert_eq!(counts.write_readback_retries, 4);
    assert_eq!(counts.fail_closed, 1);
    assert!(!root.exists(), "temporary logs collected before cleanup");
    let before = forwarded.len();
    assert_eq!(
        directory
            .finish_with(Some(archive.path()), &mut forwarded)
            .unwrap(),
        RecoveryCounts::default()
    );
    assert_eq!(forwarded.len(), before, "do not forward twice");
    let saved = receipts(archive.path());
    assert_eq!(saved.len(), 2);
    for receipt in saved {
        let raw = Path::new(receipt["archive"].as_str().unwrap()).join("stderr.log");
        assert!(
            expected.contains(&fs::read(raw).unwrap()),
            "raw stderr preserved"
        );
    }
    assert_eq!(
        String::from_utf8(forwarded)
            .unwrap()
            .matches("[digest-probe]")
            .count(),
        5
    );
}

#[test]
fn captured_output_preserves_status_and_streams() {
    let archive = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let mut forwarded = Vec::new();
    let captured = output_with(
        &mut command("always", &target.path().join("target.bin")),
        Some(archive.path()),
        &mut forwarded,
    )
    .unwrap();
    assert!(!captured.status.success());
    assert!(String::from_utf8_lossy(&captured.stderr).contains("intentional exhaustion"));
    assert!(String::from_utf8_lossy(&captured.stdout).contains("worker stdout"));
    let receipt = &receipts(archive.path())[0];
    assert_eq!(receipt["counts"]["write_readback_retries"], 3);
    assert_eq!(receipt["counts"]["fail_closed"], 1);
    assert_eq!(
        fs::read(Path::new(receipt["archive"].as_str().unwrap()).join("stderr.log")).unwrap(),
        captured.stderr
    );
    let clean = output(&mut command("clean", &target.path().join("clean.bin"))).unwrap();
    assert!(clean.status.success());
    assert_eq!(clean.stderr, b"worker stderr\n");
}

#[test]
fn parallel_collectors_keep_distinct_archives() {
    let archive = tempfile::tempdir().unwrap();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    let target = tempfile::tempdir().unwrap();
                    let mut forwarded = Vec::new();
                    let captured = output_with(
                        &mut command("once", &target.path().join("target.bin")),
                        Some(archive.path()),
                        &mut forwarded,
                    )
                    .unwrap();
                    assert!(captured.status.success());
                    assert_eq!(
                        String::from_utf8(forwarded)
                            .unwrap()
                            .matches("[digest-probe]")
                            .count(),
                        1
                    );
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    });
    let receipts = receipts(archive.path());
    assert_eq!(receipts.len(), 2);
    assert_ne!(receipts[0]["archive"], receipts[1]["archive"]);
    for receipt in receipts {
        assert_eq!(receipt["counts"]["write_readback_retries"], 1);
    }
}

#[test]
fn archive_failure_and_unclassified_records_preserve_source() {
    let archive = tempfile::tempdir().unwrap();
    let blocked = archive.path().join("blocked");
    fs::write(&blocked, b"not a directory").unwrap();
    for (bytes, destination) in [
        (b"plain stderr\n".as_slice(), Some(blocked.as_path())),
        (
            b"[digest-probe] site=unknown action=retry\n".as_slice(),
            Some(archive.path()),
        ),
        (
            b"[digest-probe] site=write-readback attempt=1".as_slice(),
            None,
        ),
        (b"\xff\n".as_slice(), None),
    ] {
        let mut directory = DiagnosticDirectory::new().unwrap();
        let source = directory.path().to_owned();
        fs::write(source.join("child.stderr.log"), bytes).unwrap();
        assert!(directory.finish_with(destination, &mut Vec::new()).is_err());
        assert_eq!(fs::read(source.join("child.stderr.log")).unwrap(), bytes);
        fs::remove_dir_all(source).unwrap();
    }
}

#[test]
fn counts_actions_without_recounting_cleanup_or_prior() {
    let mut directory = DiagnosticDirectory::new().unwrap();
    fs::write(directory.path().join("child.stderr.log"), concat!(
        "[digest-probe] site=fixture-copy action=fail-closed prior=[digest-probe] site=fixture-copy action=regenerate\n",
        "[digest-probe] site=write-readback reason=cleanup-failed action=retry\n",
        "[digest-probe] site=emission-retry action=retry\n",
        "[digest-probe] site=stream-readback action=retry\n",
        "[digest-probe] site=fixture-copy phase=source action=regenerate\n",
    )).unwrap();
    let counts = directory.finish_with(None, &mut Vec::new()).unwrap();
    assert_eq!(counts.records, 5);
    assert_eq!(counts.fail_closed, 1);
    assert_eq!(counts.write_readback_retries, 0);
    assert_eq!(counts.emission_retries, 1);
    assert_eq!(counts.stream_retries, 1);
    assert_eq!(counts.source_regenerations, 1);
}

#[test]
fn panic_keeps_temporary_evidence() {
    let directory = DiagnosticDirectory::new().unwrap();
    let path = directory.path().to_owned();
    fs::write(path.join("child.stderr.log"), b"panic context\n").unwrap();
    let result = std::panic::catch_unwind(move || {
        let _directory = directory;
        panic!("intentional evidence retention");
    });
    assert!(result.is_err());
    assert_eq!(
        fs::read(path.join("child.stderr.log")).unwrap(),
        b"panic context\n"
    );
    fs::remove_dir_all(path).unwrap();
}

#[test]
#[ignore = "子进程诊断留存的受控写后故障，仅由回归测试调用"]
fn fault_worker() {
    println!("worker stdout");
    eprintln!("worker stderr");
    let mode = std::env::var("LF_CHILD_DIAGNOSTIC_TEST_MODE").unwrap();
    if mode == "clean" {
        return;
    }
    let path = std::path::PathBuf::from(std::env::var_os("LF_CHILD_DIAGNOSTIC_TEST_PATH").unwrap());
    let persistent = mode == "always";
    let source = vec![0x5a; 8_192];
    let mut first = true;
    let result = with_after_write(
        move |path| {
            if first || persistent {
                first = false;
                let mut bytes = fs::read(path)?;
                bytes[11] ^= 1;
                fs::write(path, bytes)?;
            }
            Ok(())
        },
        || {
            write_verified(
                &path,
                &source,
                &SourceRegistration::of(&source),
                WritePolicy::DEFAULT,
            )
        },
    );
    if persistent {
        assert!(!path.exists(), "exhaustion removes rejected artifact");
        panic!("intentional exhaustion: {}", result.unwrap_err());
    }
    assert_eq!(result.unwrap().rewrites, 1);
    assert_eq!(fs::read(path).unwrap(), source);
}
