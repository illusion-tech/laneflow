//! #1388：逐段负载记录与受扰 round 判定（`docs/design/lf-synth-measurement.md` §7）。
//!
//! 采样只在帧间（W2 为拍间）位置读取 `laneflow-os-probe`，不进入任何计时区间。
//! round 判定只读本模块的负载记录，不读计时结果。

use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use laneflow_os_probe::Probe;
use serde::Serialize;
use serde_json::{Value, json};

use crate::Error;

/// 记录格式名。
pub(super) const FORMAT: &str = "lf-synth-load-1";
/// 目标段长（§7.3）。
pub(super) const SEGMENT: Duration = Duration::from_secs(1);
/// `external_load` 标记阈值初值（§7.4），单位为逻辑 CPU 当量。
pub(super) const EXTERNAL_LOAD_THRESHOLD: f64 = 1.0;
/// `run_queue_wait` 标记阈值初值（§7.4，仅 Linux），单位为线程当量。
pub(super) const RUN_QUEUE_WAIT_THRESHOLD: f64 = 0.05;

/// 段边界处的一次读数；累计量取不到时为 `None`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Reading {
    /// 距采样起点的单调时钟纳秒。
    pub wall_ns: u64,
    /// 整机忙碌纳秒。
    pub system_busy_ns: Option<u64>,
    /// 本进程用户与内核纳秒之和。
    pub process_cpu_ns: Option<u64>,
    /// 本进程全部线程的就绪等待纳秒之和（可减累计，仅 Linux）。
    pub run_queue_wait_ns: Option<u64>,
    /// 本进程缺页总数。
    pub page_faults: Option<u64>,
}

/// 一个段起点：段内首个位置（帧号或拍号）与段首读数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Boundary {
    pub first: u64,
    pub reading: Reading,
}

/// 一个判定段。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct Segment {
    pub index: usize,
    pub first: u64,
    pub last: u64,
    pub observation: bool,
    pub wall_ns: u64,
    pub system_busy_ns: Option<u64>,
    pub process_cpu_ns: Option<u64>,
    /// 差值可能为负（线程退出），原样记录。
    pub run_queue_wait_ns: Option<i64>,
    pub page_faults: Option<u64>,
    /// `L_ext`，逻辑 CPU 当量。
    pub external_load: Option<f64>,
    /// `Q`，线程当量；差值为负时为 `None`。
    pub run_queue_wait: Option<f64>,
    pub flags: Vec<&'static str>,
}

/// round 判定（§7.4）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Judgment {
    Undisturbed,
    Disturbed,
    Undeterminable,
    SamplingDisabled,
}

/// 帧间分段采样器。关闭采样时不调用任何系统接口。
pub(super) struct Sampler {
    probe: Option<Probe>,
    origin: Instant,
    /// W1–W3 在预热与观测分界处强制分段；W4 不强制（§7.3）。
    split_phases: bool,
    unit: &'static str,
    logical_cpus: Option<u32>,
    open: Option<(Instant, bool)>,
    boundaries: Vec<Boundary>,
    observations: Vec<bool>,
    errors: BTreeMap<&'static str, String>,
}

impl Sampler {
    /// `unit` 为位置单位（`frame` 或 `tick`）。
    pub(super) fn new(
        enabled: bool,
        split_phases: bool,
        unit: &'static str,
    ) -> Result<Self, Error> {
        let mut sampler = Self {
            probe: None,
            origin: Instant::now(),
            split_phases,
            unit,
            logical_cpus: None,
            open: None,
            boundaries: Vec::new(),
            observations: Vec::new(),
            errors: BTreeMap::new(),
        };
        if enabled {
            let mut probe = Probe::new()?;
            sampler.logical_cpus = probe.system_cpu().ok().and_then(|cpu| cpu.logical_cpus);
            sampler.probe = Some(probe);
        }
        Ok(sampler)
    }

    pub(super) fn enabled(&self) -> bool {
        self.probe.is_some()
    }

    /// 在位置 `position` 开始前（帧间或拍间）调用：段首达到目标段长，或 W1–W3
    /// 跨越预热与观测分界时，结束当前段并以本位置开始下一段。
    pub(super) fn before(&mut self, position: u64, observation: bool) {
        if self.probe.is_none() {
            return;
        }
        let split = match self.open {
            None => true,
            Some((started, phase)) => {
                started.elapsed() >= SEGMENT || (self.split_phases && phase != observation)
            }
        };
        if split {
            let now = Instant::now();
            self.push(position, now);
            self.open = Some((now, observation));
            self.observations.push(false);
        }
        if let Some(last) = self.observations.last_mut() {
            *last |= observation;
        }
    }

    /// 本进程缺页总数；关闭采样或不可用时为 `None`。在计时区间外调用。
    pub(super) fn page_faults(&mut self) -> Option<u64> {
        let probe = self.probe.as_mut()?;
        match probe.memory() {
            Ok(memory) => memory.page_faults,
            Err(error) => {
                self.errors
                    .entry("page_faults")
                    .or_insert_with(|| error.to_string());
                None
            }
        }
    }

    /// 结束采样：`end` 为最后位置之后的下一个位置。返回 `load.json` 内容与写入
    /// 测量记录的负载汇总。
    pub(super) fn finish(mut self, end: u64) -> (Value, Value) {
        if self.probe.is_some() && self.open.is_some() {
            self.push(end, Instant::now());
        }
        let segments = segments(&self.boundaries, &self.observations, self.split_phases);
        let (judgment, reason) = judge(self.enabled(), &segments);
        let summary = summary(&segments, judgment, reason);
        let unavailable = self.unavailable(&segments);
        let header = json!({
            "platform":std::env::consts::OS,"logical_cpus":self.logical_cpus,
            "sampling":self.enabled(),"segment_target_ns":SEGMENT.as_nanos() as u64,
            "position_unit":self.unit,"split_at_observation_start":self.split_phases,
            "thresholds":thresholds(),"sources":sources(),"unavailable":unavailable,
        });
        let record = json!({"format":FORMAT,"header":header,"segments":segments,"summary":summary});
        let mut report = summary;
        report["sampling"] = json!(self.enabled());
        report["thresholds"] = thresholds();
        (record, report)
    }

    fn push(&mut self, first: u64, now: Instant) {
        let reading = self.read(now);
        self.boundaries.push(Boundary { first, reading });
    }

    fn read(&mut self, now: Instant) -> Reading {
        let Some(probe) = self.probe.as_mut() else {
            return Reading::default();
        };
        let mut errors = Vec::new();
        let system_busy_ns = probe
            .system_cpu()
            .map_err(|e| errors.push(("system_busy", e)))
            .ok()
            .and_then(|cpu| cpu.busy_ns);
        let process_cpu_ns = probe
            .process_cpu()
            .map_err(|e| errors.push(("process_cpu", e)))
            .ok()
            .and_then(|cpu| cpu.user_ns.zip(cpu.kernel_ns))
            .map(|(user, kernel)| user.saturating_add(kernel));
        let run_queue_wait_ns = probe
            .run_queue_wait_ns()
            .map_err(|e| errors.push(("run_queue_wait", e)))
            .ok()
            .flatten();
        let page_faults = probe
            .memory()
            .map_err(|e| errors.push(("page_faults", e)))
            .ok()
            .and_then(|memory| memory.page_faults);
        for (signal, error) in errors {
            self.errors
                .entry(signal)
                .or_insert_with(|| error.to_string());
        }
        Reading {
            wall_ns: now.duration_since(self.origin).as_nanos() as u64,
            system_busy_ns,
            process_cpu_ns,
            run_queue_wait_ns,
            page_faults,
        }
    }

    /// 至少一段为 `null` 的信号及原因（§7.3 文件头）。
    fn unavailable(&self, segments: &[Segment]) -> BTreeMap<&'static str, String> {
        let mut result = BTreeMap::new();
        if !self.enabled() {
            return result;
        }
        type Missing = fn(&Segment) -> bool;
        let checks: [(&'static str, Missing); 4] = [
            ("system_busy", |s| s.system_busy_ns.is_none()),
            ("process_cpu", |s| s.process_cpu_ns.is_none()),
            ("run_queue_wait", |s| s.run_queue_wait.is_none()),
            ("page_faults", |s| s.page_faults.is_none()),
        ];
        for (signal, missing) in checks {
            if segments.iter().any(missing) {
                let reason = self
                    .errors
                    .get(signal)
                    .map(|error| format!("读取失败：{error}"))
                    .unwrap_or_else(|| platform_reason(signal).into());
                result.insert(signal, reason);
            }
        }
        result
    }
}

fn thresholds() -> Value {
    json!({"external_load":EXTERNAL_LOAD_THRESHOLD,"run_queue_wait":RUN_QUEUE_WAIT_THRESHOLD,
        "status":"initial_values_pending_quiet_window_calibration"})
}

fn sources() -> Value {
    if cfg!(windows) {
        json!({"wall":"std::time::Instant","system_busy":"laneflow-os-probe GetSystemTimes",
            "process_cpu":"laneflow-os-probe GetProcessTimes","run_queue_wait":null,
            "page_faults":"laneflow-os-probe GetProcessMemoryInfo PageFaultCount"})
    } else if cfg!(target_os = "linux") {
        json!({"wall":"std::time::Instant","system_busy":"laneflow-os-probe /proc/stat",
            "process_cpu":"laneflow-os-probe /proc/self/stat utime+stime",
            "run_queue_wait":"laneflow-os-probe /proc/self/task/*/schedstat",
            "page_faults":"laneflow-os-probe /proc/self/stat minflt+majflt"})
    } else {
        json!({"wall":"std::time::Instant","system_busy":null,"process_cpu":null,
            "run_queue_wait":null,"page_faults":null})
    }
}

fn platform_reason(signal: &str) -> &'static str {
    match signal {
        "run_queue_wait" if cfg!(target_os = "linux") => {
            "线程退出使就绪等待累计和减少的段记为 null"
        }
        "run_queue_wait" => "仅 Linux 提供本进程线程就绪等待",
        "system_busy" if cfg!(windows) => "活动处理器组多于一个时整机 CPU 时间不可用",
        _ if cfg!(any(windows, target_os = "linux")) => "平台未提供该计数",
        _ => "平台不受支持",
    }
}

/// 由段起点构造判定段：最后一个起点为结束位置。各阶段末尾不足目标段长的段并入
/// 同阶段的前一段；W4（`split_phases` 为假）只合并 round 末段（§7.3）。
pub(super) fn segments(
    boundaries: &[Boundary],
    observations: &[bool],
    split_phases: bool,
) -> Vec<Segment> {
    let count = boundaries.len().saturating_sub(1);
    let target = SEGMENT.as_nanos() as u64;
    let mut kept: Vec<(usize, bool)> = (0..count).map(|i| (i, observations[i])).collect();
    // 段 i 覆盖起点 i 到下一个保留起点（或结束位置）。
    let mut index = 0;
    while index < kept.len() {
        let (start, observation) = kept[index];
        let end = kept.get(index + 1).map_or(count, |&(next, _)| next);
        let run_ends = kept
            .get(index + 1)
            .is_none_or(|&(_, next)| split_phases && next != observation);
        let short = boundaries[end].reading.wall_ns - boundaries[start].reading.wall_ns < target;
        let same_phase_before = index > 0 && (!split_phases || kept[index - 1].1 == observation);
        if run_ends && short && same_phase_before {
            kept[index - 1].1 |= observation;
            kept.remove(index);
        } else {
            index += 1;
        }
    }
    kept.iter()
        .enumerate()
        .map(|(position, &(start, observation))| {
            let end = kept.get(position + 1).map_or(count, |&(next, _)| next);
            segment(position, &boundaries[start], &boundaries[end], observation)
        })
        .collect()
}

fn segment(index: usize, start: &Boundary, end: &Boundary, observation: bool) -> Segment {
    let (a, b) = (start.reading, end.reading);
    let delta = |x: Option<u64>, y: Option<u64>| y.zip(x).and_then(|(y, x)| y.checked_sub(x));
    let wall_ns = b.wall_ns - a.wall_ns;
    let system_busy_ns = delta(a.system_busy_ns, b.system_busy_ns);
    let process_cpu_ns = delta(a.process_cpu_ns, b.process_cpu_ns);
    let run_queue_wait_ns = b
        .run_queue_wait_ns
        .zip(a.run_queue_wait_ns)
        .map(|(y, x)| y as i64 - x as i64);
    let wall = wall_ns as f64;
    let external_load = (wall_ns > 0)
        .then_some(())
        .and(system_busy_ns.zip(process_cpu_ns))
        .map(|(busy, own)| ((busy as f64 - own as f64) / wall).max(0.0));
    let run_queue_wait = run_queue_wait_ns
        .filter(|&q| q >= 0 && wall_ns > 0)
        .map(|q| q as f64 / wall);
    let mut flags = Vec::new();
    if external_load.is_some_and(|l| l >= EXTERNAL_LOAD_THRESHOLD) {
        flags.push("external_load");
    }
    if run_queue_wait.is_some_and(|q| q >= RUN_QUEUE_WAIT_THRESHOLD) {
        flags.push("run_queue_wait");
    }
    Segment {
        index,
        first: start.first,
        last: end.first - 1,
        observation,
        wall_ns,
        system_busy_ns,
        process_cpu_ns,
        run_queue_wait_ns,
        page_faults: delta(a.page_faults, b.page_faults),
        external_load,
        run_queue_wait,
        flags,
    }
}

/// round 判定：只读负载段，不读计时字段（§7.1、§7.4）。
pub(super) fn judge(enabled: bool, segments: &[Segment]) -> (Judgment, Option<&'static str>) {
    if !enabled {
        return (Judgment::SamplingDisabled, None);
    }
    let target = SEGMENT.as_nanos() as u64;
    let observed: Vec<_> = segments.iter().filter(|s| s.observation).collect();
    if observed.is_empty() || observed.iter().any(|s| s.wall_ns < target) {
        return (
            Judgment::Undeterminable,
            Some("observation_segment_shorter_than_target"),
        );
    }
    if observed.iter().any(|s| !s.flags.is_empty()) {
        return (Judgment::Disturbed, None);
    }
    if observed
        .iter()
        .any(|s| s.external_load.is_none() && s.run_queue_wait.is_none())
    {
        return (Judgment::Undeterminable, Some("no_judging_signal"));
    }
    (Judgment::Undisturbed, None)
}

fn summary(segments: &[Segment], judgment: Judgment, reason: Option<&str>) -> Value {
    let observed = || segments.iter().filter(|s| s.observation);
    let max = |values: Vec<f64>| values.into_iter().reduce(f64::max);
    let flagged: Vec<_> = observed()
        .filter(|s| !s.flags.is_empty())
        .map(|s| json!({"index":s.index,"flags":s.flags,"external_load":s.external_load,"run_queue_wait":s.run_queue_wait}))
        .collect();
    json!({
        "segments":segments.len(),"observation_segments":observed().count(),
        "max_observation_external_load":max(observed().filter_map(|s| s.external_load).collect()),
        "max_observation_run_queue_wait":max(observed().filter_map(|s| s.run_queue_wait).collect()),
        "flagged_segments":flagged,"judgment":judgment,"judgment_reason":reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u64 = 1_000_000_000;

    fn at(first: u64, wall_ns: u64, busy: u64, own: u64, wait: Option<u64>) -> Boundary {
        Boundary {
            first,
            reading: Reading {
                wall_ns,
                system_busy_ns: Some(busy),
                process_cpu_ns: Some(own),
                run_queue_wait_ns: wait,
                page_faults: Some(first * 3),
            },
        }
    }

    fn spans(segments: &[Segment]) -> Vec<(u64, u64, bool)> {
        segments
            .iter()
            .map(|s| (s.first, s.last, s.observation))
            .collect()
    }

    /// 段首末位置连续覆盖整个 round；预热与观测分界处恰好分段；各阶段末尾短段并入
    /// 同阶段前一段，参与判定的段不短于 1 s。
    #[test]
    fn phase_tail_segments_merge_and_cover_round_contiguously() {
        let boundaries = [
            at(0, 0, 0, 0, None),
            at(60, S, 0, 0, None),
            at(90, S + S / 2, 0, 0, None), // 预热末尾短段起点
            at(100, 2 * S, 0, 0, None),    // 观测开始（强制分段）
            at(160, 3 * S + 1, 0, 0, None),
            at(220, 4 * S + 2, 0, 0, None),
            at(230, 4 * S + S / 5, 0, 0, None), // 结束位置
        ];
        let observations = [false, false, false, true, true, true];
        let result = segments(&boundaries, &observations, true);
        assert_eq!(
            spans(&result),
            [
                (0, 59, false),
                (60, 99, false),
                (100, 159, true),
                (160, 229, true)
            ]
        );
        assert!(result.windows(2).all(|w| w[0].last + 1 == w[1].first));
        assert!(result.iter().all(|s| s.wall_ns >= S));
        assert_eq!(
            result.iter().map(|s| s.index).collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        assert_eq!(result[3].page_faults, Some(230 * 3 - 160 * 3));
    }

    /// W4 不在分界处强制分段：末段含 B0/B1 与紧邻预热帧，不足 1 s 时并入前一段并
    /// 作为观测段参与判定。
    #[test]
    fn w4_tail_merges_into_previous_and_becomes_observation_segment() {
        let boundaries = [
            at(0, 0, 0, 0, None),
            at(61, S, 0, 0, None),
            at(122, 2 * S, 0, 0, None),
            at(130, 2 * S + S / 10, 0, 0, None),
        ];
        let observations = [false, false, true];
        let result = segments(&boundaries, &observations, false);
        assert_eq!(spans(&result), [(0, 60, false), (61, 129, true)]);
        assert_eq!(judge(true, &result).0, Judgment::Undisturbed);
    }

    /// round 过短、无法构成不短于 1 s 的观测段时记为无法判定。
    #[test]
    fn short_round_is_undeterminable() {
        let boundaries = [at(0, 0, 0, 0, None), at(5, S / 2, 0, 0, None)];
        let result = segments(&boundaries, &[true], false);
        assert_eq!(
            judge(true, &result),
            (
                Judgment::Undeterminable,
                Some("observation_segment_shorter_than_target")
            )
        );
    }

    /// 构造的负载序列：`L_ext` 达到阈值、`Q` 达到阈值都标记；只标记预热段时不判
    /// 为受扰；负的就绪等待差值记为 `null` 而不按零处理；`L_ext` 小于零时记为零。
    #[test]
    fn constructed_loads_flag_segments_and_judge_round() {
        let quiet = [
            at(0, 0, 0, 0, Some(0)),
            at(10, S, S / 2, S / 2, Some(S / 100)),
            at(20, 2 * S, S, S / 2 + S, Some(S / 50)),
        ];
        let result = segments(&quiet, &[true, true], true);
        assert_eq!(result[0].external_load, Some(0.0));
        assert_eq!(result[1].external_load, Some(0.0));
        assert_eq!(result[0].run_queue_wait, Some(0.01));
        assert_eq!(judge(true, &result).0, Judgment::Undisturbed);

        let loaded = [
            at(0, 0, 0, 0, Some(0)),
            at(10, S, 2 * S, S, Some(0)),
            at(20, 2 * S, 2 * S, S, Some(S / 20)),
        ];
        let result = segments(&loaded, &[true, true], true);
        assert_eq!(result[0].flags, ["external_load"]);
        assert_eq!(result[1].flags, ["run_queue_wait"]);
        assert_eq!(judge(true, &result).0, Judgment::Disturbed);

        let warm_loaded = [
            at(0, 0, 0, 0, Some(0)),
            at(10, S, 2 * S, S, Some(0)),
            at(20, 2 * S, 2 * S + S / 2, S + S / 2, Some(0)),
        ];
        let result = segments(&warm_loaded, &[false, true], true);
        assert_eq!(result[0].flags, ["external_load"]);
        assert!(result[1].flags.is_empty());
        assert_eq!(judge(true, &result).0, Judgment::Undisturbed);

        let shrinking = [at(0, 0, 0, 0, Some(S)), at(10, S, 0, 0, Some(0))];
        let result = segments(&shrinking, &[true], true);
        assert_eq!(result[0].run_queue_wait_ns, Some(-(S as i64)));
        assert_eq!(result[0].run_queue_wait, None);
        assert_eq!(result[0].external_load, Some(0.0));
    }

    /// 不可用信号记为 `null`，不填零；观测段没有任何判定信号时无法判定；关闭采样
    /// 时不读取系统接口，判定为 `sampling_disabled`。
    #[test]
    fn unavailable_signals_stay_null() {
        let empty = |first, wall_ns| Boundary {
            first,
            reading: Reading {
                wall_ns,
                ..Reading::default()
            },
        };
        let result = segments(&[empty(0, 0), empty(9, S)], &[true], true);
        let row = serde_json::to_value(&result[0]).unwrap();
        for key in [
            "system_busy_ns",
            "process_cpu_ns",
            "run_queue_wait_ns",
            "page_faults",
            "external_load",
            "run_queue_wait",
        ] {
            assert!(row[key].is_null(), "{key}");
        }
        assert_eq!(
            judge(true, &result),
            (Judgment::Undeterminable, Some("no_judging_signal"))
        );
        let mut disabled = Sampler::new(false, true, "frame").unwrap();
        disabled.before(0, true);
        assert_eq!(disabled.page_faults(), None);
        let (record, report) = disabled.finish(1);
        assert_eq!(record["segments"], json!([]));
        assert_eq!(record["header"]["sampling"], false);
        assert_eq!(report["judgment"], "sampling_disabled");
    }

    /// 实际读取：开启采样时段首末连续，缺页读数可用（支持的平台）。
    #[test]
    fn live_sampler_records_contiguous_segments() {
        let mut sampler = Sampler::new(true, true, "frame").unwrap();
        for position in 0..4 {
            sampler.before(position, position >= 2);
            let _ = sampler.page_faults();
        }
        let (record, report) = sampler.finish(4);
        let segments = record["segments"].as_array().unwrap();
        assert_eq!(segments.first().unwrap()["first"], 0);
        assert_eq!(segments.last().unwrap()["last"], 3);
        assert_eq!(report["sampling"], true);
        // 墙钟远短于 1 s：无法判定，而不是未受扰。
        assert_eq!(report["judgment"], "undeterminable");
        if cfg!(any(windows, target_os = "linux")) {
            assert!(segments.iter().all(|s| s["page_faults"].is_u64()));
        }
    }
}
