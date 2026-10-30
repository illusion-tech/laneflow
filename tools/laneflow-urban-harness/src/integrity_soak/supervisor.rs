use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

use super::{Config, control, directory_bytes, record_failure};
use crate::{Result, invalid};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Spec {
    pub(super) run_id: String,
    pub(super) supervisor_pid: u32,
    pub(super) config: Config,
}

pub(super) fn create_output(output: &Path, config: Config) -> Result<Spec> {
    config.validate()?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| invalid(format!("采集标识时间无效：{error}")))?
        .as_nanos();
    if !output.is_absolute() || output.exists() {
        return Err(invalid("浸泡输出必须为新的绝对目录"));
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    // 只有原子取得新目录的本次采集可以写入；拒绝路径不创建失败记录。
    fs::create_dir(output)?;
    let spec = Spec {
        run_id: format!("{}-{nonce}", std::process::id()),
        supervisor_pid: std::process::id(),
        config,
    };
    let initialize = || -> Result<()> {
        for name in ["segments", "events", "resources", "failures"] {
            fs::create_dir(output.join(name))?;
        }
        control::publish(&output.join("config.json"), &spec)?;
        control::publish(
            &output.join("collection.json"),
            &serde_json::json!({
                "run_id":spec.run_id, "status":"Incomplete", "reason":"collection opened; require final supervisor result",
            }),
        )?;
        control::publish(
            &output.join("control.json"),
            &control::Request {
                run_id: spec.run_id.clone(),
                sequence: 0,
                action: None,
            },
        )
    };
    let result = initialize();
    record_failure(output, &result);
    result?;
    Ok(spec)
}

pub(super) fn command(entry: &str) -> Result<Command> {
    let mut command = Command::new(std::env::current_exe()?);
    command.args(["--ignored", "--exact", entry, "--nocapture"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    Ok(command)
}

pub(super) struct ChildGuard {
    pub(super) child: Child,
    system: System,
    known: BTreeMap<Pid, u64>,
    root: Pid,
    root_start: Option<u64>,
    finished: bool,
}

impl ChildGuard {
    pub(super) fn new(child: Child) -> Self {
        let root = Pid::from_u32(child.id());
        let mut guard = Self {
            child,
            system: System::new(),
            known: BTreeMap::new(),
            root,
            root_start: None,
            finished: false,
        };
        guard.refresh();
        guard
    }

    fn refresh(&mut self) {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_memory().without_tasks(),
        );
        self.known.retain(|pid, start| {
            self.system
                .process(*pid)
                .is_some_and(|process| process.start_time() == *start)
        });
        if let Some(process) = self.system.process(self.root) {
            let expected = *self.root_start.get_or_insert(process.start_time());
            if expected == process.start_time() {
                self.known.entry(self.root).or_insert(expected);
            }
        }
        loop {
            let before = self.known.len();
            for (pid, process) in self.system.processes() {
                if let Some(parent) = process.parent().and_then(|pid| self.known.get(&pid))
                    && process.start_time() >= *parent
                {
                    self.known.entry(*pid).or_insert(process.start_time());
                }
            }
            if self.known.len() == before {
                break;
            }
        }
    }

    fn members(&mut self) -> Vec<serde_json::Value> {
        self.refresh();
        let supervisor = Pid::from_u32(std::process::id());
        self.known
            .keys()
            .chain(std::iter::once(&supervisor))
            .filter_map(|pid| {
                self.system.process(*pid).map(|process| {
                    serde_json::json!({
                        "pid":pid.as_u32(), "start_time":process.start_time(),
                        "resident_bytes":(process.memory() > 0).then_some(process.memory()),
                        "terminated":process.status() == ProcessStatus::Zombie,
                        "parent_pid":process.parent().map(Pid::as_u32),
                    })
                })
            })
            .collect()
    }

    pub(super) fn terminate(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.refresh();
        // 先停父进程，阻止继续派生；已观察后代即使重新挂靠，也按 PID + 启动时间追踪。
        let _ = self.child.kill();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            for pid in self.known.keys() {
                if let Some(process) = self.system.process(*pid) {
                    let _ = process.kill();
                }
            }
            let reaped = self.child.try_wait()?.is_some();
            self.refresh();
            let remaining = self.known.keys().any(|pid| {
                self.system
                    .process(*pid)
                    .is_some_and(|process| process.status() != ProcessStatus::Zombie)
            });
            if reaped && !remaining {
                self.finished = true;
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(invalid("进程清理未确认完成"));
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

/// #1173：soak 摘要浮点判定谓词的可测提取（#1155 审计「待验证」清单收尾）。
///
/// 语义与 supervisor 收尾判定逐条相同（原 :316-321 记账谓词 + :332-335 值谓词），
/// 提取不改行为；调用方保持同一墙钟读取时点。判定内容：`active` 有限且 ≥ 请求秒数、
/// `paused` 有限且非负、记账一致性（虚拟钟 `|active − covered×0.016| ≤ 0.016` 且
/// `paused ≤ wall`；真实钟 `active + paused ≤ wall`）。
fn float_judgment_ok(
    active: f64,
    paused: f64,
    covered: u64,
    requested_seconds: u64,
    virtual_clock: bool,
    wall_seconds: f64,
) -> bool {
    let accounting_ok = if virtual_clock {
        (active - covered as f64 * 0.016).abs() <= 0.016 && paused <= wall_seconds
    } else {
        active + paused <= wall_seconds
    };
    active.is_finite()
        && active >= requested_seconds as f64
        && paused.is_finite()
        && paused >= 0.0
        && accounting_ok
}

pub(super) fn supervise(output: &Path, config: Config, entry: &str) -> Result<()> {
    let spec = create_output(output, config)?;
    let started = Instant::now();
    watch(output, &spec, entry, || started.elapsed())
}

pub(super) fn watch(
    output: &Path,
    spec: &Spec,
    entry: &str,
    elapsed: impl Fn() -> Duration,
) -> Result<()> {
    let run = || -> Result<()> {
        let mut command = command(entry)?;
        command
            .env("LF_SOAK_OUTPUT", output)
            .stdin(Stdio::piped())
            .stdout(Stdio::from(fs::File::create(
                output.join("resident.stdout.log"),
            )?))
            .stderr(Stdio::from(fs::File::create(
                output.join("resident.stderr.log"),
            )?));
        let mut guard = ChildGuard::new(command.spawn()?);
        let mut samples = Vec::new();
        let mut ordinal = 0_u64;
        let mut peak: Option<u64> = None;
        let mut complete_memory_samples = 0_u64;
        let mut observed_resident = false;
        let result = (|| -> Result<()> {
            loop {
                let exit_status = guard.child.try_wait()?;
                let members = guard.members();
                let measured = |pid: u32| {
                    members.iter().any(|member| {
                        member["pid"] == pid && member["resident_bytes"].as_u64().unwrap_or(0) > 0
                    })
                };
                let supervisor_measured = measured(std::process::id());
                let resident_measured = measured(guard.child.id());
                observed_resident |= resident_measured;
                let required_memory_measured =
                    supervisor_measured && (resident_measured || exit_status.is_some());
                let memory_sample_complete = required_memory_measured
                    && members.iter().all(|member| {
                        member["terminated"] == true || member["resident_bytes"].as_u64().is_some()
                    });
                let bytes = members
                    .iter()
                    .map(|member| member["resident_bytes"].as_u64().unwrap_or(0))
                    .try_fold(0_u64, |total, value| total.checked_add(value))
                    .ok_or_else(|| invalid("工作集求和溢出"))?;
                if memory_sample_complete {
                    peak = Some(peak.unwrap_or(0).max(bytes));
                    complete_memory_samples += 1;
                }
                let evidence = directory_bytes(output)?;
                let record = serde_json::json!({
                    "run_id":spec.run_id, "ordinal":ordinal, "wall_seconds":elapsed().as_secs_f64(),
                    "resident_bytes_sum":bytes, "sampled_peak_bytes_sum":peak, "evidence_bytes":evidence,
                    "memory_sample_complete":memory_sample_complete,
                    "method":"sysinfo sampled resident set sum, includes supervisor; not heap peak or OS quota",
                    "members":members,
                });
                samples.push(record.clone());
                control::publish(
                    &output
                        .join("resources")
                        .join(format!("{:06}.json", ordinal / 60)),
                    &samples,
                )?;
                control::publish(&output.join("watch.json"), &record)?;
                ordinal += 1;
                if ordinal.is_multiple_of(60) {
                    samples.clear();
                }
                if !required_memory_measured && elapsed() >= Duration::from_secs(3) {
                    return Err(invalid("无法测得看护或存活驻留进程的工作集（Incomplete）"));
                }
                if bytes > spec.config.max_memory_bytes {
                    return Err(invalid(
                        "进程组工作集采样合计超限（Incomplete; memory threshold）",
                    ));
                }
                if evidence > spec.config.max_disk_bytes {
                    return Err(invalid(
                        "证据容量采样超限（Incomplete; evidence threshold）",
                    ));
                }
                if elapsed() >= Duration::from_secs(spec.config.wall_seconds) {
                    return Err(invalid("含暂停的总墙钟超时（Incomplete; wall timeout）"));
                }
                if let Some(status) = exit_status {
                    if !status.success() {
                        return Err(invalid(format!("驻留进程失败退出（Incomplete）：{status}")));
                    }
                    if !observed_resident || !supervisor_measured {
                        return Err(invalid("没有驻留进程的有效工作集样本（Incomplete）"));
                    }
                    let summary: serde_json::Value =
                        control::read_json(&output.join("summary.json"))?;
                    let active = summary["active_seconds"].as_f64().unwrap_or(f64::NAN);
                    let paused = summary["paused_seconds"].as_f64().unwrap_or(f64::NAN);
                    let covered = summary["covered_ticks"].as_u64().unwrap_or(0);
                    let start = summary["initial_tick"].as_u64();
                    let end = summary["final_tick"].as_u64();
                    // 钟型由看护冻结配置决定（#1110 审阅：摘要自报值不得决定验证
                    // 严格度——防 config.json 在看护启动后、worker 读取前被改的
                    // TOCTOU）；摘要钟型与冻结配置不符即拒绝。
                    let virtual_clock = spec.config.virtual_clock;
                    let clock_ok = summary["clock"]
                        == if virtual_clock {
                            "virtual-ticks"
                        } else {
                            "real"
                        };
                    // 真实时钟下 `paced` 必须成立；虚拟时钟（#1108）不睡眠。
                    let paced_ok = summary["paced"] == !virtual_clock;
                    // 记账一致性：真实模式沿用 `active + paused <= wall`（防虚报
                    // 时间膨胀）；虚拟模式等价替换为 `active ≈ 覆盖 tick × 16ms`
                    // （防记账漂移/超记）且 `paused <= wall`（真实暂停仍受真实
                    // 墙钟约束）。两种模式都拦“虚报时间膨胀”这一失效模式。
                    // #1173：浮点判定谓词提取为 [`float_judgment_ok`]（可测面，语义
                    // 不变）；墙钟在此读取一次，保持与原判定块同一读取时点。
                    let wall_seconds = elapsed().as_secs_f64();
                    if summary["run_id"] != spec.run_id
                        || summary["status"] != "Completed"
                        || summary["scope"] != "bounded-resident-soak"
                        || !clock_ok
                        || !paced_ok
                        || summary["requested_seconds"] != spec.config.seconds
                        || summary["resident_pid"] != guard.child.id()
                        || summary["runtime_workers"] != 1
                        || summary["world_id"].as_u64().is_none()
                        || summary["complete_match_batches"].as_u64().unwrap_or(0) == 0
                        || !float_judgment_ok(
                            active,
                            paused,
                            covered,
                            spec.config.seconds,
                            virtual_clock,
                            wall_seconds,
                        )
                        || covered == 0
                        || !start
                            .zip(end)
                            .is_some_and(|(start, end)| start.checked_add(covered) == Some(end))
                    {
                        return Err(invalid(
                            "驻留进程未完成约定区间或覆盖记录不完整（Incomplete）",
                        ));
                    }
                    return Ok(());
                }
                thread::sleep(Duration::from_secs(1));
            }
        })();
        let cleanup = guard.terminate();
        control::publish(
            &output.join("supervisor.result.json"),
            &serde_json::json!({
                "run_id":spec.run_id, "status":if result.is_ok() && cleanup.is_ok(){"Completed"}else{"Incomplete"},
                "reason":result.as_ref().err().map(ToString::to_string),
                "cleanup":cleanup.as_ref().err().map_or("observed descendants terminated".into(), ToString::to_string),
                "wall_seconds":elapsed().as_secs_f64(), "sample_count":ordinal,
                "sampled_peak_resident_bytes_sum":peak,
                "complete_memory_samples":complete_memory_samples,
            }),
        )?;
        cleanup?;
        result
    };
    let result = run();
    if let Err(error) = &result
        && !output.join("supervisor.result.json").exists()
    {
        let _ = control::publish(
            &output.join("supervisor.result.json"),
            &serde_json::json!({
                "run_id":spec.run_id, "status":"Incomplete", "reason":error.to_string(),
                "cleanup":"no final cleanup confirmation; inspect failure and process state",
                "wall_seconds":elapsed().as_secs_f64(),
            }),
        );
    }
    record_failure(output, &result);
    result
}

#[cfg(test)]
mod float_judgment_tests {
    //! #1173：soak 浮点判定谓词的 ±1 ULP 边界注入翻转测试与输入域零漂移证明
    //!（#1155 审计「待验证」清单收尾：supervisor 谓词 :317/:318/:320/:333 与
    //! tests.rs 三处 `>=` 阈值断言逐条给结论）。

    use std::time::Duration;

    use super::float_judgment_ok;

    /// 正规格数的下一个更大浮点值（测试仅对正规格数使用）。
    fn next_up(value: f64) -> f64 {
        f64::from_bits(value.to_bits() + 1)
    }

    /// 正规格数的下一个更小浮点值（测试仅对正规格数使用）。
    fn next_down(value: f64) -> f64 {
        f64::from_bits(value.to_bits() - 1)
    }

    /// 输入域零漂移证明（判据①）：soak 同款 `Duration::as_secs_f64` → JSON → 解析
    /// 往返逐位恒等（20,000 样本扫掠，含 #1155 探针同款序列）。修复后判定输入不存在
    /// 1 ULP 解析漂移源——任何谓词都不存在由解析漂移触发的翻转路径（「已验证不翻转」
    /// 的成立依据）。
    #[test]
    fn duration_json_round_trip_is_bit_exact() {
        for k in 0..10_000_u64 {
            let ns = (((k * 2_654_435_761) % 10_000_000) * 100) as u32;
            let samples = [
                Duration::new(1, ns).as_secs_f64(),
                Duration::from_millis((((k * 37) % 2_000) + 1) * 16).as_secs_f64(),
            ];
            for value in samples {
                let text = serde_json::to_string(&value).unwrap();
                let parsed: f64 = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed.to_bits(), value.to_bits(), "drift at {text}");
            }
        }
    }

    /// ±1 ULP 边界注入翻转矩阵（判据②）：逐谓词钉住漂移敏感性分类。敏感项在恰界值
    /// 注入 ±1 ULP 会翻转（历史暴露面：修复前默认解析可达）；修复后由判据①零漂移
    /// 保证不可达。tests.rs:223-224,327 三处 `>=` 阈值断言与 :333 同形态，归并同结论。
    #[test]
    fn one_ulp_boundary_injection_flip_matrix() {
        // :333 `active >= requested_seconds`（含 tests.rs 三处 `>=` 形态）：
        // **边界敏感**——恰等阈值时 −1 ULP 翻失败。修复后结论：不翻转（判据①）。
        assert!(
            float_judgment_ok(4.0, 0.0, 1, 4, false, 100.0),
            "threshold boundary passes"
        );
        assert!(
            !float_judgment_ok(next_down(4.0), 0.0, 1, 4, false, 100.0),
            "-1 ULP flips threshold"
        );
        // :318 `paused <= wall`（虚拟钟）：**边界敏感**——恰等墙钟时 +1 ULP 翻失败。
        // 修复后结论：不翻转（判据①）。
        assert!(
            float_judgment_ok(1.6, 5.0, 100, 1, true, 5.0),
            "paused boundary passes"
        );
        assert!(
            !float_judgment_ok(1.6, next_up(5.0), 100, 1, true, 5.0),
            "+1 ULP flips paused<=wall"
        );
        // :320 `active + paused <= wall`（真实钟）：**边界敏感**——和恰等墙钟时任一
        // 侧 +1 ULP 翻失败（取值避开求和 ties-to-even 吸收：4.0 的 +1 ULP 在 5.0
        // 尺度上可表示）。修复后结论：不翻转（判据①）。
        assert!(
            float_judgment_ok(1.0, 4.0, 1, 1, false, 5.0),
            "sum boundary passes"
        );
        assert!(
            !float_judgment_ok(1.0, next_up(4.0), 1, 1, false, 5.0),
            "+1 ULP flips sum<=wall"
        );
        // :317 `|active − covered×0.016| <= 0.016`（虚拟钟容差）：**边界敏感**——
        // 偏差恰等容差时 +1 ULP 翻失败。修复后结论：不翻转（判据①）。
        assert!(
            float_judgment_ok(0.016, 0.0, 0, 0, true, 100.0),
            "tolerance boundary passes"
        );
        assert!(
            !float_judgment_ok(next_up(0.016), 0.0, 0, 0, true, 100.0),
            "+1 ULP flips tolerance"
        );
        // :332/:334 `is_finite`：**漂移不敏感（翻转不可达）**——输入域上界为
        // u64::MAX 秒 ≈ 1.85×10¹⁹ ≪ f64::MAX，±1 ULP 保持有限。
        for value in [5e-324, 1.7743044000000001, 1e19, u64::MAX as f64] {
            assert!(next_up(value).is_finite(), "finite preserved up for {value}");
            assert!(next_down(value).is_finite(), "finite preserved down for {value}");
        }
        assert!(next_up(0.0).is_finite(), "finite preserved up for 0.0");
        // :335 `paused >= 0.0`：**漂移不敏感（翻转不可达）**——token 域为非负
        // Duration 的 zmij 字面量，解析按量级舍入不引入符号，可达结果恒 ≥ +0.0。
        for text in ["0.0", "5e-324", "0.009999999999999787", "1.7743044000000001"] {
            let parsed: f64 = serde_json::from_str(text).unwrap();
            assert!(parsed >= 0.0, "non-negative parse for {text}");
        }
    }
}
