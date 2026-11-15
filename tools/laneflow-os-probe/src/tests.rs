use std::time::{Duration, Instant};

use crate::proc_text::{
    PidStat, StatCpu, is_thread_gone, parse_cpu_list_count, parse_pid_stat, parse_schedstat_wait,
    parse_stat_cpu, parse_status_bytes, ticks_to_ns,
};
use crate::{
    PageFaultCounter, PowerStatus, Probe, decode_windows_power, hundred_ns_to_ns,
    system_times_cover_machine,
};

#[test]
fn stat_cpu_line_parses_fields_and_excludes_guest_from_busy() {
    let text = "cpu  10 20 30 400 50 6 7 8 9 10\ncpu0 1 2 3 4 5 6 7 8 9 10\n";
    let stat = parse_stat_cpu(text.as_bytes()).unwrap();
    assert_eq!(
        stat,
        StatCpu {
            user: 10,
            nice: 20,
            system: 30,
            idle: 400,
            iowait: 50,
            irq: 6,
            softirq: 7,
            steal: 8,
        }
    );
    // guest（第 9、10 项）已计入 user，不重复累加；iowait 不计入忙碌。
    assert_eq!(stat.busy(), 10 + 20 + 30 + 6 + 7 + 8);
}

#[test]
fn stat_cpu_line_rejects_missing_or_malformed_fields() {
    assert!(parse_stat_cpu(b"").is_err());
    assert!(parse_stat_cpu(b"cpu0 1 2 3 4 5 6 7 8\n").is_err());
    assert!(parse_stat_cpu(b"cpu  1 2 3 4 5 6 7\n").is_err());
    assert!(parse_stat_cpu(b"cpu  1 2 x 4 5 6 7 8\n").is_err());
}

#[test]
fn pid_stat_parses_after_last_parenthesis_even_with_spaces_in_name() {
    // 字段：pid (comm) state ppid pgrp session tty tpgid flags minflt cminflt
    // majflt cmajflt utime stime …
    let text = "1234 (a (b) c) S 1 2 3 4 5 6 111 7 222 8 333 444 9 10\n";
    assert_eq!(
        parse_pid_stat(text.as_bytes()).unwrap(),
        PidStat {
            minflt: 111,
            majflt: 222,
            utime: 333,
            stime: 444,
        }
    );
    assert!(parse_pid_stat(b"1234 comm S 1 2").is_err());
    assert!(parse_pid_stat(b"1234 (comm) S 1 2 3").is_err());
}

#[test]
fn status_bytes_reads_kib_fields_and_reports_absent_as_none() {
    let text = "Name:\tx\nVmHWM:\t    2048 kB\nVmRSS:\t    1024 kB\nRssAnon:\t512 kB\n";
    assert_eq!(
        parse_status_bytes(text.as_bytes(), "VmRSS").unwrap(),
        Some(1024 * 1024)
    );
    assert_eq!(
        parse_status_bytes(text.as_bytes(), "VmHWM").unwrap(),
        Some(2048 * 1024)
    );
    assert_eq!(
        parse_status_bytes(text.as_bytes(), "RssAnon").unwrap(),
        Some(512 * 1024)
    );
    assert_eq!(parse_status_bytes(text.as_bytes(), "VmSwap").unwrap(), None);
    // 前缀相同的其他键不误匹配。
    assert_eq!(
        parse_status_bytes(b"VmRSSx:\t1 kB\n", "VmRSS").unwrap(),
        None
    );
    assert!(parse_status_bytes(b"VmRSS:\tabc kB\n", "VmRSS").is_err());
    assert!(parse_status_bytes(b"VmRSS:\t1 MB\n", "VmRSS").is_err());
}

#[test]
fn proc_parsers_accept_non_utf8_and_truncated_process_names() {
    // comm 是任意字节序列；内核 15 字节截断还可能切开多字节 UTF-8 字符。
    let truncated = "x交通运行时线程".as_bytes()[..15].to_vec();
    assert!(std::str::from_utf8(&truncated).is_err());
    for name in [vec![0xff, 0xfe, b')', b' '], truncated] {
        let mut stat = b"1234 (".to_vec();
        stat.extend_from_slice(&name);
        stat.extend_from_slice(
            b") S 1 2 3 4 5 6 111 7 222 8 333 444 9 10
",
        );
        assert_eq!(parse_pid_stat(&stat).unwrap().utime, 333);
        let mut status = b"Name:	".to_vec();
        status.extend_from_slice(&name);
        status.extend_from_slice(
            b"
VmRSS:	   8 kB
",
        );
        assert_eq!(
            parse_status_bytes(&status, "VmRSS").unwrap(),
            Some(8 * 1024)
        );
    }
}

#[test]
fn thread_disappearance_covers_enoent_and_esrch_only() {
    assert!(is_thread_gone(&std::io::Error::from(
        std::io::ErrorKind::NotFound
    )));
    assert!(is_thread_gone(&std::io::Error::from_raw_os_error(3)));
    assert!(!is_thread_gone(&std::io::Error::from(
        std::io::ErrorKind::PermissionDenied
    )));
    assert!(!is_thread_gone(&std::io::Error::from_raw_os_error(13)));
}

#[test]
fn schedstat_reads_second_column() {
    assert_eq!(parse_schedstat_wait(b"100 200 3\n").unwrap(), 200);
    assert!(parse_schedstat_wait(b"").is_err());
    assert!(parse_schedstat_wait(b"100\n").is_err());
}

#[test]
fn cpu_list_counts_ranges_and_singletons() {
    assert_eq!(parse_cpu_list_count(b"0-31\n").unwrap(), 32);
    assert_eq!(parse_cpu_list_count(b"0-3,8,10-11").unwrap(), 7);
    assert_eq!(parse_cpu_list_count(b"0").unwrap(), 1);
    assert!(parse_cpu_list_count(b"").is_err());
    assert!(parse_cpu_list_count(b"3-1").is_err());
    assert!(parse_cpu_list_count(b"a-b").is_err());
}

#[test]
fn tick_and_filetime_conversions_are_exact() {
    assert_eq!(ticks_to_ns(3, 100), 30_000_000);
    assert_eq!(ticks_to_ns(1, 250), 4_000_000);
    assert_eq!(hundred_ns_to_ns(1, 0), 100);
    assert_eq!(hundred_ns_to_ns(0, 1), (1_u64 << 32) * 100);
}

#[test]
fn system_times_only_cover_single_processor_group() {
    assert!(system_times_cover_machine(1));
    assert!(!system_times_cover_machine(2));
    assert!(!system_times_cover_machine(0));
}

#[test]
fn windows_power_treats_unknown_battery_flag_as_unavailable() {
    // 255 同时含“无电池”位，必须先判为未知。
    assert_eq!(
        decode_windows_power(1, 255, 80),
        PowerStatus {
            ac_online: Some(true),
            battery_present: None,
            battery_percent: None,
        }
    );
    assert_eq!(
        decode_windows_power(0, 128, 255),
        PowerStatus {
            ac_online: Some(false),
            battery_present: Some(false),
            battery_percent: None,
        }
    );
    assert_eq!(
        decode_windows_power(255, 8, 95),
        PowerStatus {
            ac_online: None,
            battery_present: Some(true),
            battery_percent: Some(95),
        }
    );
    assert_eq!(decode_windows_power(1, 1, 255).battery_percent, None);
}

#[test]
fn page_fault_counter_unwraps_one_wraparound_between_reads() {
    let mut counter = PageFaultCounter::default();
    assert_eq!(counter.extend(u32::MAX - 1), u64::from(u32::MAX - 1));
    assert_eq!(counter.extend(3), u64::from(u32::MAX - 1) + 5);
    assert_eq!(counter.extend(3), u64::from(u32::MAX - 1) + 5);
}

/// 可减累计字段的负差值由调用方记为不可用：示例规则与本测试共同固定口径。
#[test]
fn decreasable_counter_difference_is_unavailable_when_negative() {
    fn difference(before: u64, after: u64) -> Option<u64> {
        after.checked_sub(before)
    }
    assert_eq!(difference(10, 15), Some(5));
    assert_eq!(difference(15, 10), None);
}

/// 空转直到调用线程自身的 CPU 时间增加 `target`，最多等 `timeout` 墙钟。
/// 以线程自己的计数为终止条件：被抢占时只是多等，不把调度争用误判为计数缺陷。
#[cfg(any(windows, target_os = "linux"))]
fn spin_until_thread_cpu_advances(probe: &mut Probe, target: Duration, timeout: Duration) {
    let total = |cpu: crate::CpuTime| cpu.user_ns.unwrap() + cpu.kernel_ns.unwrap();
    let start_cpu = total(probe.thread_cpu().unwrap());
    let start = Instant::now();
    let mut value = 0_u64;
    loop {
        for _ in 0..10_000 {
            value = std::hint::black_box(value.wrapping_mul(6364136223846793005).wrapping_add(1));
        }
        let advanced = total(probe.thread_cpu().unwrap()) - start_cpu;
        if advanced >= target.as_nanos() as u64 {
            return;
        }
        assert!(
            start.elapsed() < timeout,
            "{timeout:?} 内线程 CPU 只增加 {advanced} ns，计数器未随空转推进"
        );
    }
}

/// 单调累计字段两次读取不减；线程 CPU 增量不超过墙钟增量，进程 CPU 增量
/// 不小于线程 CPU 增量（各留两档粒度）。不要求线程获得最低 CPU 份额。
#[cfg(any(windows, target_os = "linux"))]
#[test]
fn live_monotonic_counters_do_not_decrease_and_track_busy_spin() {
    let mut probe = Probe::new().unwrap();
    let system_before = probe.system_cpu().unwrap();
    let process_before = probe.process_cpu().unwrap();
    let thread_before = probe.thread_cpu().unwrap();
    let memory_before = probe.memory().unwrap();
    let start = Instant::now();
    spin_until_thread_cpu_advances(
        &mut probe,
        Duration::from_millis(50),
        Duration::from_secs(10),
    );
    let wall = start.elapsed();
    let system_after = probe.system_cpu().unwrap();
    let process_after = probe.process_cpu().unwrap();
    let thread_after = probe.thread_cpu().unwrap();
    let memory_after = probe.memory().unwrap();

    let total = |cpu: crate::CpuTime| cpu.user_ns.unwrap() + cpu.kernel_ns.unwrap();
    assert!(total(process_after) >= total(process_before));
    assert!(total(thread_after) >= total(thread_before));
    assert!(memory_after.page_faults.unwrap() >= memory_before.page_faults.unwrap());
    assert!(
        memory_after.peak_resident_bytes.unwrap() >= memory_before.peak_resident_bytes.unwrap()
    );
    assert!(memory_after.resident_bytes.unwrap() > 0);
    assert!(system_after.logical_cpus.unwrap() > 0);
    if let (Some(before), Some(after)) = (system_before.busy_ns, system_after.busy_ns) {
        assert!(after >= before);
        assert!(system_after.idle_ns.unwrap() >= system_before.idle_ns.unwrap());
    }

    // 两档粒度：Windows 约 15.6 ms，Linux 常见 10 ms；取 2 × 16 ms。
    let slack = Duration::from_millis(32).as_nanos() as u64;
    let thread_delta = total(thread_after) - total(thread_before);
    let process_delta = total(process_after) - total(process_before);
    let wall_ns = wall.as_nanos() as u64;
    assert!(
        thread_delta <= wall_ns + slack,
        "线程 CPU 增量 {thread_delta} ns 超过墙钟 {wall_ns} ns"
    );
    assert!(
        process_delta + slack >= thread_delta,
        "进程 CPU 增量 {process_delta} ns 小于线程 CPU 增量 {thread_delta} ns"
    );
}

#[cfg(windows)]
#[test]
fn windows_reports_linux_only_fields_as_unavailable() {
    let mut probe = Probe::new().unwrap();
    assert_eq!(probe.system_cpu().unwrap().linux_iowait_ns, None);
    let memory = probe.memory().unwrap();
    assert_eq!(memory.linux_anonymous_resident_bytes, None);
    assert_eq!(memory.linux_major_page_faults, None);
    assert!(memory.windows_private_bytes.is_some());
    assert_eq!(probe.run_queue_wait_ns().unwrap(), None);
    probe.power().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn linux_reports_windows_only_fields_as_unavailable() {
    let mut probe = Probe::new().unwrap();
    assert!(probe.system_cpu().unwrap().linux_iowait_ns.is_some());
    let memory = probe.memory().unwrap();
    assert_eq!(memory.windows_private_bytes, None);
    assert_eq!(memory.windows_commit_bytes, None);
    assert_eq!(memory.windows_peak_commit_bytes, None);
    assert!(memory.linux_major_page_faults.is_some());
    assert!(probe.run_queue_wait_ns().unwrap().is_some());
    probe.power().unwrap();
}
