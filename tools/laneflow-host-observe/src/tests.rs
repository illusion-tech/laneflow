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
const SOURCE_DIRECTORY: &str = "LANEFLOW_HOST_OBSERVE_SOURCE_DIRECTORY";
/// 样例记录与临时仓库钉版共用的工具链 channel。固定虚拟值：升级 Rust 不触及本测试。
const FIXTURE_CHANNEL: &str = "1.0.0";
/// 样例记录对应的构建工具链全名（rustup 代理导出给构建进程的形态）。
const FIXTURE_BUILD: &str = "1.0.0-x86_64-pc-windows-msvc";

/// 在临时仓库根写入钉版文件，内容为给定的 `rust-toolchain.toml` 文本。
fn pin_toolchain(root: &Path, text: &str) {
    std::fs::write(root.join("rust-toolchain.toml"), text).unwrap();
}

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
            // 现行采集不指定选择器：记录里是环境实际解析到的 rustc / cargo。
            selector: None,
            rustc_vv: format!("rustc {FIXTURE_CHANNEL}\nhost: x86_64-pc-windows-msvc"),
            cargo_version: format!("cargo {FIXTURE_CHANNEL}"),
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
    pin_toolchain(
        root.path(),
        &format!("[toolchain]\nchannel = \"{FIXTURE_CHANNEL}\"\n"),
    );
    let ready = |record: &EnvironmentRecord| {
        record.require_measurement_ready(&"c".repeat(40), root.path(), Some(FIXTURE_BUILD))
    };
    let record = sample_record();
    // 构建工具链接受 rustup 代理导出的全名，也接受不带主机后缀的 channel。
    ready(&record).unwrap();
    record
        .require_measurement_ready(&"c".repeat(40), root.path(), Some(FIXTURE_CHANNEL))
        .unwrap();
    assert!(
        record
            .require_measurement_ready(&"a".repeat(40), root.path(), Some(FIXTURE_BUILD))
            .is_err()
    );
    let mut dirty = sample_record();
    dirty.source = Field::Value(Source {
        commit: "c".repeat(40),
        tree: "d".repeat(40),
        clean: false,
    });
    assert!(ready(&dirty).is_err());
    let mut no_toolchain = sample_record();
    no_toolchain.toolchain = Field::Unavailable("x".into());
    let error = ready(&no_toolchain).unwrap_err();
    assert!(error.to_string().contains("工具链不可用"), "{error}");
    pin_toolchain(root.path(), "[toolchain]\n");
    assert!(ready(&record).is_err());
    pin_toolchain(
        root.path(),
        &format!("[toolchain]\nchannel = \"{FIXTURE_CHANNEL}\"\n"),
    );
    let mut p100 = sample_record();
    p100.manual.hardware_role = HardwareRole::P100;
    assert!(ready(&p100).is_err());
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
    ready(&p100).unwrap();
}

/// 工具链两项核对的判定层：测量环境的 rustc / cargo 须是钉版，测量程序自身也须由
/// 钉版工具链编译。记录由采集器按环境解析，不再有「选择等于钉版」这种同义反复。
#[test]
fn measurement_rejects_other_toolchains() {
    let pinned = "1.0.0";
    let sample = sample_record();
    let Field::Value(toolchain) = &sample.toolchain else {
        unreachable!("样例记录带工具链");
    };
    assert_eq!(toolchain.selector, None);
    let check = |toolchain: &Toolchain, built: Option<&str>| {
        environment::require_pinned_toolchain(toolchain, pinned, built)
    };
    check(toolchain, Some(FIXTURE_BUILD)).unwrap();
    check(toolchain, Some(pinned)).unwrap();
    // 测量程序由别的工具链、别的补丁号、别的主机或自定义链接编译：记录本身合格也拒绝。
    for built in [
        "nightly-x86_64-pc-windows-msvc",
        "1.0.1-x86_64-pc-windows-msvc",
        "1.0.0-x86_64-unknown-linux-gnu",
        "1.0.0-custom",
        "",
    ] {
        let error = check(toolchain, Some(built)).unwrap_err();
        assert!(error.to_string().contains(built), "{built}: {error}");
    }
    // 构建进程没有导出工具链名：无从判断，拒绝。
    assert!(check(toolchain, None).is_err());
    // 测量环境的 rustc / cargo 不是钉版（环境变量或目录覆盖造成的偏离）：拒绝。
    // 版本记号须整个相等：别的版本、带预发布后缀或再多一段都算不同。
    for rustc_vv in [
        "rustc 9.9.9 (other)\nhost: x86_64-pc-windows-msvc".to_owned(),
        "rustc 1.0.1-nightly (other)\nhost: x86_64-pc-windows-msvc".to_owned(),
        "rustc 1.0.0.1 (other)\nhost: x86_64-pc-windows-msvc".to_owned(),
        String::new(),
    ] {
        let drifted = Toolchain {
            rustc_vv,
            ..toolchain.clone()
        };
        assert!(check(&drifted, Some(FIXTURE_BUILD)).is_err());
    }
    let drifted = Toolchain {
        cargo_version: "cargo 9.9.9 (other)".to_owned(),
        ..toolchain.clone()
    };
    assert!(check(&drifted, Some(FIXTURE_BUILD)).is_err());
    // 旧记录的 selector 写着钉版、版本却是别的：选择不构成依据，照样拒绝。
    let stale = Toolchain {
        selector: Some(pinned.into()),
        rustc_vv: "rustc 9.9.9 (other)\nhost: x86_64-pc-windows-msvc".to_owned(),
        ..toolchain.clone()
    };
    assert!(check(&stale, Some(FIXTURE_BUILD)).is_err());
}

/// 生产路径：真实采集器在本 checkout 按环境解析出的工具链，与仓库钉版、本测试二进制
/// 编译期的构建工具链三者一致。`cargo test` 经 rustup 选用钉版工具链，三者必然相等；
/// 在被环境变量或目录覆盖带偏的工具链下构建或运行时，本测试失败。
#[test]
fn live_collected_toolchain_is_the_pinned_build_toolchain() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let record = environment::collect(&CollectOptions {
        phase: Phase::Before,
        repository: repository.clone(),
        manual: manual(),
        snapshot_interval: Duration::from_millis(20),
    })
    .unwrap();
    let Field::Value(toolchain) = &record.toolchain else {
        unreachable!("{:?}", record.toolchain);
    };
    assert_eq!(toolchain.selector, None);
    let pinned = environment::pinned_toolchain(&repository).unwrap();
    environment::require_pinned_toolchain(toolchain, &pinned, option_env!("RUSTUP_TOOLCHAIN"))
        .unwrap();
}

#[test]
fn pinned_channel_reads_only_the_toolchain_table() {
    use environment::parse_pinned_channel as parse;
    // 返回值借自入参文本：内嵌的钉版文件可以直接给出 &'static str。
    assert_eq!(parse("[toolchain]\nchannel = \"1.0.0\"\n"), Some("1.0.0"));
    // 注释、行尾注释、缩进与其他表里的同名键都不改变结果；CRLF 不残留进 channel。
    assert_eq!(
        parse(
            "# 钉版\r\n\r\n[toolchain]\r\n  channel = \"1.0.0\" # pinned\r\n[net]\r\nchannel = \"other\"\r\n"
        ),
        Some("1.0.0")
    );
    // 缺表、缺键、重复、空值、非普通字符串、转义、写法不完整：一律不给出 channel。
    for text in [
        "",
        "channel = \"1.0.0\"\n",
        "[toolchain]\n",
        "[toolchain]\nchannel = \"1.0.0\"\nchannel = \"1.0.0\"\n",
        "[toolchain]\nchannel = \"\"\n",
        "[toolchain]\nchannel = '1.0.0'\n",
        "[toolchain]\nchannel = 1.0.0\n",
        "[toolchain]\nchannel = \"1.0\\t.0\"\n",
        "[toolchain]\nchannel = \"1.0.0\n",
        "[ toolchain ]\nchannel = \"1.0.0\"\n",
    ] {
        assert!(parse(text).is_none(), "{text:?}");
    }
}

#[test]
fn pinned_toolchain_reads_the_checkout_pin_file() {
    let root = tempfile::tempdir().unwrap();
    let error = environment::pinned_toolchain(root.path()).unwrap_err();
    assert!(error.to_string().contains("rust-toolchain.toml"), "{error}");
    pin_toolchain(root.path(), "[toolchain]\n");
    assert!(environment::pinned_toolchain(root.path()).is_err());
    pin_toolchain(
        root.path(),
        &format!("[toolchain]\nchannel = \"{FIXTURE_CHANNEL}\"\n"),
    );
    assert_eq!(
        environment::pinned_toolchain(root.path()).unwrap(),
        FIXTURE_CHANNEL
    );
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
        Ok("source") => {
            let directory = std::env::var_os(SOURCE_DIRECTORY).unwrap();
            let source = environment::source(Path::new(&directory)).unwrap();
            println!("source commit={} clean={}", source.commit, source.clean);
        }
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

#[test]
fn git_commands_drop_inherited_git_overrides() {
    let mut command = Command::new("git");
    crate::command::sanitize_git_environment(
        &mut command,
        ["GIT_DIR", "git_work_tree", "XDG_CONFIG_HOME", "PATH"].map(Into::into),
    );
    let envs: Vec<(String, Option<String>)> = command
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().to_ascii_uppercase(),
                value.map(|value| value.to_string_lossy().into_owned()),
            )
        })
        .collect();
    for removed in ["GIT_DIR", "GIT_WORK_TREE", "XDG_CONFIG_HOME"] {
        assert!(
            envs.contains(&(removed.to_owned(), None)),
            "{removed} 未被移除：{envs:?}"
        );
    }
    assert!(envs.contains(&("GIT_NO_REPLACE_OBJECTS".to_owned(), Some("1".to_owned()))));
    assert!(!envs.iter().any(|(key, _)| key == "PATH"));
}

fn git(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(repository)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// 继承的 `GIT_DIR`、`GIT_WORK_TREE` 指向另一个干净克隆时，源码状态仍取自给定
/// 目录所在的 checkout：不干净的工作树不能借此通过核对。
#[test]
fn source_state_ignores_inherited_git_directory_overrides() {
    let root = tempfile::tempdir().unwrap();
    let clean = root.path().join("clean");
    let dirty = root.path().join("dirty");
    for repository in [&clean, &dirty] {
        std::fs::create_dir(repository).unwrap();
        git(repository, &["init", "-q"]);
        git(
            repository,
            &["commit", "-q", "--allow-empty", "-m", "fixture"],
        );
    }
    std::fs::write(dirty.join("untracked.txt"), b"x").unwrap();
    let dirty_commit = git(&dirty, &["rev-parse", "HEAD"]);
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "tests::helper_process",
            "--nocapture",
        ])
        .env(HELPER, "source")
        .env(SOURCE_DIRECTORY, &dirty)
        .env("GIT_DIR", clean.join(".git"))
        .env("GIT_WORK_TREE", &clean)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("source commit={dirty_commit} clean=false")),
        "{stdout}"
    );
}
