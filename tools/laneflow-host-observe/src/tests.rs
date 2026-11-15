use std::{
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::{
    Field,
    child::{ChildObserver, Scope},
    environment::{
        self, CollectOptions, Collection, Cpu, EnvironmentRecord, Fingerprint, Firmware,
        HardwareRole, ManualFields, Memory, Os, Phase, Power, PowerPlan, Source, Toolchain,
    },
    snapshot::{self, ProcessSnapshot, Reading},
    time::format_unix_millis,
};

const HELPER: &str = "LANEFLOW_HOST_OBSERVE_HELPER";

fn manual() -> ManualFields {
    ManualFields {
        vendor_performance_mode: "无厂商性能模式".into(),
        hardware_role: HardwareRole::Uncertified,
    }
}

fn sample_record() -> EnvironmentRecord {
    EnvironmentRecord {
        format: environment::FORMAT.into(),
        version: environment::VERSION,
        collection: Collection {
            recorded_at: "2026-11-16T00:00:00.000Z".into(),
            phase: Phase::Before,
            collector_version: "0.0.0".into(),
        },
        os: Os {
            name: Field::Value("Windows".into()),
            version: Field::Value("11".into()),
            long_version: Field::Value("Windows 11 Pro".into()),
            kernel: Field::Value("29704".into()),
            architecture: "x86_64".into(),
        },
        cpu: Cpu {
            model: Field::Value("CPU".into()),
            physical_cores: Field::Value(16),
            logical_processors: Field::Value(32),
        },
        memory: Memory {
            physical_bytes: Field::Value(64 << 30),
        },
        power: Power {
            ac_online: Field::Value(true),
            battery_present: Field::Value(true),
            battery_percent: Field::Value(90),
            windows_power_plan: Field::Value(PowerPlan {
                guid: "381b4222-f694-41f0-9685-ff5bb260df2e".into(),
                name: Field::Value("Balanced".into()),
            }),
            linux_cpufreq_governors: Field::Unavailable("仅 Linux 提供".into()),
            linux_platform_profile: Field::Unavailable("仅 Linux 提供".into()),
        },
        firmware: Firmware {
            bios_vendor: Field::Value("vendor".into()),
            bios_version: Field::Value("1.0".into()),
            bios_date: Field::Value("01/01/2025".into()),
        },
        toolchain: Field::Value(Toolchain {
            selector: Some("1.99.1".into()),
            rustc_vv: "rustc 1.99.1\nhost: x86_64-pc-windows-msvc".into(),
            cargo_version: "cargo 1.99.1".into(),
            host_target: "x86_64-pc-windows-msvc".into(),
        }),
        source: Field::Value(Source {
            commit: "c".repeat(40),
            tree: "d".repeat(40),
            clean: true,
        }),
        fingerprint: Field::Value(Fingerprint {
            scheme: environment::FINGERPRINT_SCHEME.into(),
            sha256: "e".repeat(64),
        }),
        manual: manual(),
        processes: ProcessSnapshot {
            captured_at: "2026-11-16T00:00:01.000Z".into(),
            interval_ms: 1000,
            cpu_time_unit: "ms".into(),
            processes: Vec::new(),
        },
    }
}

#[test]
fn utc_formatting_covers_epoch_and_leap_day() {
    assert_eq!(format_unix_millis(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(
        format_unix_millis(1_763_262_245_678),
        "2025-11-16T03:04:05.678Z"
    );
    assert_eq!(
        format_unix_millis(1_709_168_523_004),
        "2024-02-29T01:02:03.004Z"
    );
}

#[test]
fn unavailable_fields_carry_reasons_in_json() {
    let value: serde_json::Value =
        serde_json::to_value(Field::<u32>::Unavailable("原因".into())).unwrap();
    assert_eq!(value, serde_json::json!({"unavailable": "原因"}));
    assert_eq!(
        serde_json::to_value(Field::Value(3_u32)).unwrap(),
        serde_json::json!({"value": 3})
    );
}

#[test]
fn manual_fields_reject_blank_placeholder_and_unknown_fields() {
    let parsed =
        ManualFields::from_json(br#"{"vendor_performance_mode":"Turbo","hardware_role":"P100"}"#)
            .unwrap();
    assert_eq!(parsed.hardware_role, HardwareRole::P100);
    for mode in ["", "   ", "unknown", "N/A", "Unavailable", "未知"] {
        let text = serde_json::json!({"vendor_performance_mode": mode, "hardware_role": "R0"});
        assert!(
            ManualFields::from_json(text.to_string().as_bytes()).is_err(),
            "{mode:?}"
        );
    }
    for text in [
        r#"{"hardware_role":"R0"}"#,
        r#"{"vendor_performance_mode":"Turbo"}"#,
        r#"{"vendor_performance_mode":"Turbo","hardware_role":"R9"}"#,
        r#"{"vendor_performance_mode":"Turbo","hardware_role":"R0","rustc_vv":"x"}"#,
    ] {
        assert!(ManualFields::from_json(text.as_bytes()).is_err(), "{text}");
    }
}

#[test]
fn fingerprint_follows_v2_normalization() {
    let expected = "ddde3e28090c564bf4b3dcf91eb7b6edd0a463924c60a0853c933063d58149a8";
    assert_eq!(
        environment::fingerprint_digest(
            "4C4C4544-0000-1111-2222-333344445555",
            "ABC123XYZ",
            "BOARD-9"
        )
        .as_deref(),
        Some(expected)
    );
    assert_eq!(
        environment::fingerprint_digest(
            " 4c4c4544-0000-1111-2222-333344445555\t",
            "abc 123 xyz",
            "board-9\n"
        )
        .as_deref(),
        Some(expected)
    );
    assert_eq!(environment::fingerprint_digest("u", "  ", "b"), None);
}

#[test]
fn p100_requires_matching_reference_declaration() {
    let reference = serde_json::json!({
        "hardwareIdentityScheme": environment::FINGERPRINT_SCHEME,
        "hardwareIdentitySha256": "e".repeat(64),
    })
    .to_string();
    let record = sample_record();
    environment::require_reference_machine(&record.fingerprint, reference.as_bytes()).unwrap();
    let other = Field::Value(Fingerprint {
        scheme: environment::FINGERPRINT_SCHEME.into(),
        sha256: "f".repeat(64),
    });
    assert!(environment::require_reference_machine(&other, reference.as_bytes()).is_err());
    let wrong_scheme = Field::Value(Fingerprint {
        scheme: "laneflow-p100-hardware-identity-v1".into(),
        sha256: "e".repeat(64),
    });
    assert!(environment::require_reference_machine(&wrong_scheme, reference.as_bytes()).is_err());
    assert!(
        environment::require_reference_machine(
            &Field::Unavailable("x".into()),
            reference.as_bytes()
        )
        .is_err()
    );
}

#[test]
fn measurement_readiness_checks_commit_cleanliness_toolchain_and_p100() {
    let root = tempfile::tempdir().unwrap();
    let record = sample_record();
    record
        .require_measurement_ready(&"c".repeat(40), root.path())
        .unwrap();
    assert!(
        record
            .require_measurement_ready(&"a".repeat(40), root.path())
            .is_err()
    );
    let mut dirty = sample_record();
    dirty.source = Field::Value(Source {
        commit: "c".repeat(40),
        tree: "d".repeat(40),
        clean: false,
    });
    assert!(
        dirty
            .require_measurement_ready(&"c".repeat(40), root.path())
            .is_err()
    );
    let mut no_toolchain = sample_record();
    no_toolchain.toolchain = Field::Unavailable("x".into());
    assert!(
        no_toolchain
            .require_measurement_ready(&"c".repeat(40), root.path())
            .is_err()
    );
    let mut p100 = sample_record();
    p100.manual.hardware_role = HardwareRole::P100;
    assert!(
        p100.require_measurement_ready(&"c".repeat(40), root.path())
            .is_err()
    );
    let declaration = root.path().join(environment::REFERENCE_MACHINE_PATH);
    std::fs::create_dir_all(declaration.parent().unwrap()).unwrap();
    std::fs::write(
        &declaration,
        serde_json::json!({
            "hardwareIdentityScheme": environment::FINGERPRINT_SCHEME,
            "hardwareIdentitySha256": "e".repeat(64),
        })
        .to_string(),
    )
    .unwrap();
    p100.require_measurement_ready(&"c".repeat(40), root.path())
        .unwrap();
}

#[test]
fn record_round_trips_and_rejects_unknown_versions_and_fields() {
    let record = sample_record();
    let bytes = record.to_json();
    assert_eq!(EnvironmentRecord::from_json(&bytes).unwrap(), record);
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["version"] = 2.into();
    let error = EnvironmentRecord::from_json(value.to_string().as_bytes()).unwrap_err();
    assert!(error.to_string().contains("版本"), "{error}");
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["format"] = "other".into();
    assert!(EnvironmentRecord::from_json(value.to_string().as_bytes()).is_err());
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["os"]["extra"] = 1.into();
    assert!(EnvironmentRecord::from_json(value.to_string().as_bytes()).is_err());
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["manual"]["vendor_performance_mode"] = " ".into();
    assert!(EnvironmentRecord::from_json(value.to_string().as_bytes()).is_err());
}

#[test]
fn before_after_comparison_detects_each_invariant_group() {
    let before = sample_record();
    let mut after = sample_record();
    after.collection.phase = Phase::After;
    after.collection.recorded_at = "later".into();
    after.power.battery_percent = Field::Value(10);
    after.processes.interval_ms = 5;
    assert!(before.differences(&after).is_empty());
    assert_eq!(before.differences(&before), vec!["collection.phase"]);
    type Mutation = fn(&mut EnvironmentRecord);
    let cases: [(&str, Mutation); 10] = [
        ("os", |r| r.os.kernel = Field::Value("1".into())),
        ("firmware", |r| {
            r.firmware.bios_version = Field::Value("2".into())
        }),
        ("power.windows_power_plan", |r| {
            r.power.windows_power_plan = Field::Unavailable("x".into());
        }),
        ("power.linux_cpufreq_governors", |r| {
            r.power.linux_cpufreq_governors = Field::Value(vec!["performance".into()]);
        }),
        ("power.linux_platform_profile", |r| {
            r.power.linux_platform_profile = Field::Value("performance".into());
        }),
        ("power.ac_online", |r| {
            r.power.ac_online = Field::Value(false)
        }),
        ("toolchain", |r| {
            r.toolchain = Field::Unavailable("x".into())
        }),
        ("source", |r| r.source = Field::Unavailable("x".into())),
        ("fingerprint", |r| {
            r.fingerprint = Field::Unavailable("x".into())
        }),
        ("manual", |r| r.manual.hardware_role = HardwareRole::R0),
    ];
    for (name, mutate) in cases {
        let mut changed = after.clone();
        mutate(&mut changed);
        assert_eq!(before.differences(&changed), vec![name]);
    }
}

#[test]
fn power_plan_parsing_maps_known_guids_and_tolerates_non_utf8() {
    let mut gbk =
        b"\xb5\xe7\xd4\xb4\xb7\xbd\xb0\xb8 GUID: 381B4222-F694-41F0-9685-FF5BB260DF2E  (".to_vec();
    gbk.extend_from_slice(b"\xc6\xbd\xba\xe2)\r\n");
    let plan = environment::parse_power_plan(&gbk).unwrap();
    assert_eq!(plan.guid, "381b4222-f694-41f0-9685-ff5bb260df2e");
    assert_eq!(plan.name, Field::Value("Balanced".into()));
    let custom = environment::parse_power_plan(
        "Power Scheme GUID: 01234567-89ab-cdef-0123-456789abcdef  (Quiet Lab)".as_bytes(),
    )
    .unwrap();
    assert_eq!(custom.name, Field::Value("Quiet Lab".into()));
    let mut non_utf8 = b"GUID: 01234567-89ab-cdef-0123-456789abcdef  (".to_vec();
    non_utf8.extend_from_slice(b"\xff\xfe)");
    assert!(matches!(
        environment::parse_power_plan(&non_utf8).unwrap().name,
        Field::Unavailable(_)
    ));
    assert_eq!(environment::parse_power_plan(b"no scheme here"), None);
}

#[test]
fn registry_and_index_parsers() {
    let output = "\r\nHKEY_LOCAL_MACHINE\\HARDWARE\\DESCRIPTION\\System\\BIOS\r\n    BIOSVendor    REG_SZ    American Megatrends International, LLC.\r\n    BIOSVersion    REG_SZ    N.1.09\r\n    BIOSMajorRelease    REG_DWORD    0x5\r\n";
    assert_eq!(
        environment::parse_registry_value(output, "BIOSVendor").as_deref(),
        Some("American Megatrends International, LLC.")
    );
    assert_eq!(
        environment::parse_registry_value(output, "BIOSVersion").as_deref(),
        Some("N.1.09")
    );
    assert_eq!(
        environment::parse_registry_value(output, "BIOSMajorRelease"),
        None
    );
    assert_eq!(
        environment::parse_registry_value(output, "BIOSReleaseDate"),
        None
    );
    assert!(!environment::has_hidden_index_entries(
        "H src/a.rs\nH src/b.rs"
    ));
    assert!(environment::has_hidden_index_entries(
        "H src/a.rs\nS src/b.rs"
    ));
    assert!(environment::has_hidden_index_entries("h src/a.rs"));
}

fn reading(pid: u32, start: u64, cpu: u64, memory: u64) -> Reading {
    Reading {
        pid,
        start_time_s: start,
        parent_pid: Some(1),
        name: format!("p{pid}"),
        status: "Run".into(),
        exited_unreaped: false,
        cpu_ms: cpu,
        resident_bytes: memory,
    }
}

#[test]
fn snapshot_deltas_need_same_identity_in_both_readings() {
    let first = [
        reading(10, 100, 50, 1),
        reading(11, 100, 70, 1),
        reading(12, 100, 9, 1),
    ];
    let second = vec![
        reading(12, 100, 5, 0),
        reading(10, 100, 80, 4096),
        reading(11, 200, 3, 1),
        reading(13, 300, 1, 1),
    ];
    let snapshot = snapshot::build(&first, second, "t".into(), 1000);
    let pids: Vec<u32> = snapshot.processes.iter().map(|entry| entry.pid).collect();
    assert_eq!(pids, [10, 11, 12, 13]);
    let entry = |pid: u32| snapshot.processes.iter().find(|e| e.pid == pid).unwrap();
    assert_eq!(entry(10).cpu_delta_ms, Some(30));
    assert_eq!(entry(10).resident_bytes, Some(4096));
    assert_eq!(entry(11).cpu_ms_first, None);
    assert_eq!(entry(11).cpu_delta_ms, None);
    assert_eq!(entry(12).cpu_delta_ms, None, "计数减少不得记为增量");
    assert_eq!(entry(12).resident_bytes, None);
    assert_eq!(entry(13).cpu_delta_ms, None);
}

#[test]
fn live_snapshot_lists_this_process() {
    let snapshot = snapshot::capture(Duration::from_millis(20)).unwrap();
    assert_eq!(snapshot.interval_ms, 20);
    assert!(
        snapshot
            .processes
            .iter()
            .any(|entry| entry.pid == std::process::id())
    );
}

#[test]
fn live_record_reports_platform_fields_and_keeps_smbios_values_out() {
    let record = environment::collect(&CollectOptions {
        phase: Phase::Before,
        repository: Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf(),
        toolchain: None,
        manual: manual(),
        snapshot_interval: Duration::from_millis(20),
    })
    .unwrap();
    assert!(record.toolchain.value().is_some(), "{:?}", record.toolchain);
    assert!(record.source.value().is_some(), "{:?}", record.source);
    assert!(record.cpu.logical_processors.value().is_some());
    assert!(record.memory.physical_bytes.value().is_some());
    if cfg!(windows) {
        assert!(record.power.windows_power_plan.value().is_some());
        assert!(record.power.linux_cpufreq_governors.value().is_none());
    } else {
        assert!(record.power.windows_power_plan.value().is_none());
        assert!(record.fingerprint.value().is_none());
    }
    let text = String::from_utf8(record.to_json()).unwrap();
    let raw = [
        sysinfo::Product::uuid().ok(),
        sysinfo::Product::serial_number().ok(),
        sysinfo::Motherboard::new()
            .ok()
            .and_then(|board| board.serial_number()),
    ];
    for value in raw.into_iter().flatten() {
        let value = value.trim();
        if value.len() >= 4 {
            assert!(!text.contains(value), "环境记录含原始 SMBIOS 值");
            assert!(!text.to_uppercase().contains(&value.to_uppercase()));
        }
    }
    assert_eq!(
        EnvironmentRecord::from_json(text.as_bytes()).unwrap(),
        record
    );
}

/// P100 参考机上的人工验证：在参考机上运行
/// `cargo test -p laneflow-host-observe -- --ignored reference_machine`。
#[test]
#[ignore = "只在 P100 参考机上人工运行"]
fn reference_machine_fingerprint_matches_published_declaration() {
    let record = environment::collect(&CollectOptions {
        phase: Phase::Before,
        repository: Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf(),
        toolchain: None,
        manual: ManualFields {
            vendor_performance_mode: "未记录".into(),
            hardware_role: HardwareRole::P100,
        },
        snapshot_interval: Duration::from_millis(20),
    })
    .unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let declaration = std::fs::read(root.join(environment::REFERENCE_MACHINE_PATH)).unwrap();
    environment::require_reference_machine(&record.fingerprint, &declaration).unwrap();
}

fn helper(mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            "tests::helper_process",
            "--nocapture",
        ])
        .env(HELPER, mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// 子进程观测测试的辅助进程：按环境变量退出、空转或派生孙进程。
#[test]
#[ignore = "仅作为子进程观测测试的辅助进程"]
fn helper_process() {
    match std::env::var(HELPER).as_deref() {
        Ok("exit3") => std::process::exit(3),
        Ok("sleep") => thread::sleep(Duration::from_secs(60)),
        Ok("spawn") => {
            let mut grandchild = helper("sleep").spawn().unwrap();
            thread::sleep(Duration::from_secs(60));
            let _ = grandchild.kill();
            let _ = grandchild.wait();
        }
        _ => {}
    }
}

fn wait_exit(observer: &mut ChildObserver) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while observer.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "辅助进程未退出");
        observer.sample();
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn child_exit_code_and_sampled_summary() {
    let mut observer =
        ChildObserver::new(helper("exit3").spawn().unwrap(), Scope::DirectChild).unwrap();
    assert!(observer.summary().is_none());
    wait_exit(&mut observer);
    let summary = observer.summary().unwrap();
    assert_eq!(summary.scope, Scope::DirectChild);
    assert_eq!(summary.exit.code, Some(3));
    assert!(!summary.exit.success);
    assert_eq!(summary.exit.signal, None);
    assert_eq!(summary.bounds, crate::child::SAMPLED_BOUNDS);
}

#[test]
fn direct_child_sampling_records_memory_and_observer() {
    let mut observer =
        ChildObserver::new(helper("sleep").spawn().unwrap(), Scope::DirectChild).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let sample = loop {
        let sample = observer.sample();
        if sample
            .members
            .iter()
            .any(|member| member.pid == observer.id() && member.resident_bytes.is_some())
        {
            break sample;
        }
        assert!(Instant::now() < deadline, "未测得辅助进程驻留字节");
        thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(sample.members.len(), 1);
    assert_eq!(
        sample.observer.as_ref().map(|member| member.pid),
        Some(std::process::id())
    );
    observer.terminate(Duration::from_secs(10)).unwrap();
    let summary = observer.summary().unwrap();
    assert!(!summary.exit.success);
    assert!(summary.sampled_peak_resident_bytes.is_some());
    assert!(summary.sample_count >= 1);
    #[cfg(unix)]
    assert_eq!(summary.exit.signal, Some(9));
}

#[test]
fn process_group_tracks_descendants_and_terminates_by_identity() {
    let mut observer =
        ChildObserver::new(helper("spawn").spawn().unwrap(), Scope::ProcessGroup).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let grandchild = loop {
        let sample = observer.sample();
        if let Some(member) = sample
            .members
            .iter()
            .find(|member| member.parent_pid == Some(observer.id()))
        {
            break member.clone();
        }
        assert!(Instant::now() < deadline, "未观察到孙进程");
        thread::sleep(Duration::from_millis(50));
    };
    observer.terminate(Duration::from_secs(10)).unwrap();
    let mut system = sysinfo::System::new().unwrap();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let alive = system
        .process(sysinfo::Pid::from_u32(grandchild.pid))
        .is_some_and(|process| {
            process.start_time() == grandchild.start_time_s
                && process.status() != sysinfo::ProcessStatus::Zombie
        });
    assert!(!alive, "孙进程未被终止");
}
