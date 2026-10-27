use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::{Sample, SourceIdentity, attest, sample, supervisor::Spec, write};
use crate::{Artifacts, Harness, Result, invalid};

pub(super) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(256 * 1_024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 256 * 1_024 {
        return Err(invalid("浸泡控制记录超限"));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

pub(super) fn publish<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let temporary = path.with_extension("tmp");
    write(&temporary, &serde_json::to_vec_pretty(value)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

pub(super) struct Clock {
    started: Instant,
    paused: Duration,
    pause_start: Option<Instant>,
    /// `Some` 时为无节拍虚拟时钟（#1108）：`active` 只由推进 tick 按 16ms/拍
    /// 记账，不等待真实时间；暂停期间无 tick 推进即无记账，`active` 天然不含
    /// 暂停。仅测试仪器使用；长时取证仍用真实时钟。
    credited: Option<Duration>,
}

impl Clock {
    pub(super) fn new() -> Self {
        Self {
            started: Instant::now(),
            paused: Duration::ZERO,
            pause_start: None,
            credited: None,
        }
    }

    /// 无节拍虚拟时钟（#1108）：短回归只验证正确性与记账一致性，不等待驻留节奏。
    pub(super) fn virtual_ticks() -> Self {
        Self {
            credited: Some(Duration::ZERO),
            ..Self::new()
        }
    }

    /// 虚拟时钟下按推进的 tick 数记账（16ms/拍）；真实时钟为空操作。
    pub(super) fn credit(&mut self, ticks: u64) {
        if let Some(credited) = &mut self.credited {
            *credited += Duration::from_millis(16 * ticks);
        }
    }

    pub(super) fn is_virtual(&self) -> bool {
        self.credited.is_some()
    }

    pub(super) fn wall(&self) -> Duration {
        self.started.elapsed()
    }

    pub(super) fn paused(&self) -> Duration {
        self.paused
            + self
                .pause_start
                .map_or(Duration::ZERO, |start| start.elapsed())
    }

    pub(super) fn active(&self) -> Duration {
        if let Some(credited) = self.credited {
            return credited;
        }
        self.pause_start
            .unwrap_or_else(Instant::now)
            .duration_since(self.started)
            .saturating_sub(self.paused)
    }

    fn pause(&mut self) {
        self.pause_start = Some(Instant::now());
    }

    fn resume(&mut self) {
        self.paused += self.pause_start.take().expect("paused clock").elapsed();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Action {
    Pause,
    Resume,
    Stop,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) run_id: String,
    pub(super) sequence: u64,
    pub(super) action: Option<Action>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Status {
    pub(super) run_id: String,
    pub(super) state: String,
    pub(super) last_control: u64,
    pub(super) resident_pid: u32,
    pub(super) world_id: u64,
    pub(super) tick: u64,
    pub(super) command_cursor: u64,
    pub(super) active_seconds: f64,
    pub(super) paused_seconds: f64,
    pub(super) wall_seconds: f64,
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    run_id: String,
    resident_pid: u32,
    sequence: u64,
    sample: Sample,
}

struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}

pub(super) fn issue(output: &Path, command: &str) -> Result<()> {
    if command == "status" {
        for name in [
            "config.json",
            "status.json",
            "control.json",
            "watch.json",
            "supervisor.result.json",
            "failure.json",
        ] {
            let path = output.join(name);
            if path.exists() {
                println!("{name}: {}", read_json::<serde_json::Value>(&path)?);
            }
        }
        return Ok(());
    }
    let action = match command {
        "pause" => Action::Pause,
        "resume" => Action::Resume,
        "stop" => Action::Stop,
        _ => return Err(invalid("命令必须为 status / pause / resume / stop")),
    };
    let lock = output.join("control.lock");
    fs::create_dir(&lock).map_err(|error| invalid(format!("控制写入锁不可用：{error}")))?;
    let _lock = Lock(lock);
    if output.join("supervisor.result.json").exists() || output.join("failure.json").exists() {
        return Err(invalid("采集已结束，不能继续控制"));
    }
    let spec: Spec = read_json(&output.join("config.json"))?;
    let status: Status = read_json(&output.join("status.json"))?;
    let previous: Request = read_json(&output.join("control.json"))?;
    if status.run_id != spec.run_id
        || previous.run_id != spec.run_id
        || previous.sequence != status.last_control
    {
        return Err(invalid("控制身份不匹配或上一请求尚未确认"));
    }
    let allowed = match action {
        Action::Pause => status.state == "Running",
        Action::Resume => status.state == "Paused",
        Action::Stop => matches!(status.state.as_str(), "Running" | "Paused"),
    };
    if !allowed {
        return Err(invalid("当前状态不接受该命令"));
    }
    let sequence = previous
        .sequence
        .checked_add(1)
        .ok_or_else(|| invalid("控制序号耗尽"))?;
    publish(
        &output.join("control.json"),
        &Request {
            run_id: spec.run_id,
            sequence,
            action: Some(action),
        },
    )?;
    println!("请求 {sequence} {action:?} 已写入；以 status.json 的确认状态为准");
    Ok(())
}

pub(super) struct Controller {
    output: PathBuf,
    run_id: String,
    seen: u64,
    supervisor_closed: Arc<AtomicBool>,
}

impl Controller {
    pub(super) fn new(output: &Path, spec: &Spec, supervised: bool) -> Result<Self> {
        let supervisor_closed = Arc::new(AtomicBool::new(false));
        if supervised {
            let closed = Arc::clone(&supervisor_closed);
            thread::Builder::new()
                .name("soak-supervisor-pipe".into())
                .spawn(move || {
                    // 看护独占管道写端；关闭或异常输入均终止驻留采集，不靠 PID 存活猜测。
                    let _ = std::io::stdin().read(&mut [0]);
                    closed.store(true, Ordering::Relaxed);
                })?;
        }
        Ok(Self {
            output: output.into(),
            run_id: spec.run_id.clone(),
            seen: 0,
            supervisor_closed,
        })
    }

    pub(super) fn check_supervisor(&self) -> Result<()> {
        if self.supervisor_closed.load(Ordering::Relaxed) {
            return Err(invalid(
                "资源看护管道已断开（Incomplete; supervisor disconnected）",
            ));
        }
        Ok(())
    }

    pub(super) fn pending(&self) -> Result<Option<Request>> {
        let request: Request = read_json(&self.output.join("control.json"))?;
        if request.run_id != self.run_id || request.sequence < self.seen {
            return Err(invalid("控制身份或序号不匹配"));
        }
        if request.sequence == self.seen {
            return Ok(None);
        }
        if request.sequence != self.seen + 1 || request.action.is_none() {
            return Err(invalid("控制序号不连续或动作缺失"));
        }
        Ok(Some(request))
    }

    pub(super) fn status(&self, state: &str, harness: &Harness<'_>, clock: &Clock) -> Result<()> {
        publish(
            &self.output.join("status.json"),
            &Status {
                run_id: self.run_id.clone(),
                state: state.into(),
                last_control: self.seen,
                resident_pid: std::process::id(),
                world_id: harness.world().world_id(),
                tick: harness.world().tick_index(),
                command_cursor: harness.world().command_cursor(),
                active_seconds: clock.active().as_secs_f64(),
                paused_seconds: clock.paused().as_secs_f64(),
                wall_seconds: clock.wall().as_secs_f64(),
            },
        )
    }

    fn event(&self, name: &str, value: &serde_json::Value) -> Result<()> {
        let path = self
            .output
            .join("events")
            .join(format!("{:08}-{name}.json", self.seen));
        if path.exists() {
            return Err(invalid("控制事件已存在"));
        }
        publish(&path, value)
    }

    pub(super) fn boundary(
        &mut self,
        harness: &Harness<'_>,
        artifacts: &Artifacts,
        source: &Path,
        identity: &SourceIdentity,
        clock: &mut Clock,
    ) -> Result<bool> {
        let Some(request) = self.pending()? else {
            return Ok(false);
        };
        self.seen = request.sequence;
        match request.action.expect("validated action") {
            Action::Stop => {
                self.event(
                    "stopped",
                    &serde_json::json!({"tick":harness.world().tick_index()}),
                )?;
                return Ok(true);
            }
            Action::Resume => return Err(invalid("没有暂停检查点，拒绝恢复")),
            Action::Pause => {}
        }
        let checkpoint = Checkpoint {
            run_id: self.run_id.clone(),
            resident_pid: std::process::id(),
            sequence: self.seen,
            sample: sample(harness, 0)?,
        };
        clock.pause();
        let path = self
            .output
            .join("events")
            .join(format!("{:08}-checkpoint.json", self.seen));
        publish(&path, &checkpoint)?;
        self.event(
            "paused",
            &serde_json::json!({
                "tick":checkpoint.sample.tick, "active_seconds":clock.active().as_secs_f64(),
                "wall_seconds":clock.wall().as_secs_f64(), "checkpoint":path,
            }),
        )?;
        self.status("Paused", harness, clock)?;
        let pause_started = Instant::now();
        loop {
            self.check_supervisor()?;
            if let Some(request) = self.pending()? {
                match request.action.expect("validated action") {
                    Action::Stop => {
                        self.seen = request.sequence;
                        self.event(
                            "stopped",
                            &serde_json::json!({"tick":checkpoint.sample.tick}),
                        )?;
                        return Ok(true);
                    }
                    Action::Pause => return Err(invalid("已经暂停，拒绝重复暂停请求")),
                    Action::Resume => {}
                }
                let stored: Checkpoint = read_json(&path)?;
                if stored != checkpoint {
                    return Err(invalid("暂停检查点文件失配"));
                }
                let attestation = attest(artifacts, source, identity)?;
                if sample(harness, 0)? != checkpoint.sample {
                    return Err(invalid("暂停期间驻留世界检查点失配，拒绝继续"));
                }
                self.seen = request.sequence;
                clock.resume();
                self.event(
                    "resumed",
                    &serde_json::json!({
                        "tick":checkpoint.sample.tick, "world_id":checkpoint.sample.world_id,
                        "resident_pid":checkpoint.resident_pid, "root_attestation":attestation,
                        "paused_interval_seconds":pause_started.elapsed().as_secs_f64(),
                        "sampling_gap":"pause interval; no ticks or periodic attestations",
                    }),
                )?;
                self.status("Running", harness, clock)?;
                return Ok(false);
            }
            thread::sleep(Duration::from_millis(200));
        }
    }
}
