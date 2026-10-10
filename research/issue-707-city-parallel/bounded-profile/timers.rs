//! #826 独立研究插桩；仅补丁构建载入，不进入正式 Runtime API。
#![allow(missing_docs)]
use std::{
    fmt::Write,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};
#[derive(Clone, Copy)]
pub enum Stage {
    Replace,
    LiveRank,
    Overlap,
    FreshAdmission,
    EnsureOccupancy,
    EnsureDownstream,
    EnsureContenders,
    RebuildContenders,
    RefreshContenders,
    InsertedVehicle,
    RestrictiveStop,
    DownstreamSpeed,
    OwnMotion,
    NearestLeader,
    DirectFollowers,
    UpstreamFollowers,
    ReserveSpawn,
    ApplySpawn,
    Preflight,
    Occupancy,
    WaitingPrepare,
    ConflictPrepare,
    MotionLoop,
    WaitingFinalize,
    Signals,
    ConflictFinalize,
    WaitingOutputs,
    Commit,
    Frontier,
    ValidateUpdates,
    ColumnSetup,
    ColumnDispatch,
    ColumnConsume,
    BeforeState,
    RedWaiters,
    ObserveEvents,
    ObserveState,
    ObserveCounts,
    ParkingInvariants,
    StateVehicles,
    StateBodies,
    PublicReplace,
    CallerCommands,
}
const N: usize = 43;
const NAMES: [&str; N] = [
    "Replace",
    "LiveRank",
    "Overlap",
    "FreshAdmission",
    "EnsureOccupancy",
    "EnsureDownstream",
    "EnsureContenders",
    "RebuildContenders",
    "RefreshContenders",
    "InsertedVehicle",
    "RestrictiveStop",
    "DownstreamSpeed",
    "OwnMotion",
    "NearestLeader",
    "DirectFollowers",
    "UpstreamFollowers",
    "ReserveSpawn",
    "ApplySpawn",
    "Preflight",
    "Occupancy",
    "WaitingPrepare",
    "ConflictPrepare",
    "MotionLoop",
    "WaitingFinalize",
    "Signals",
    "ConflictFinalize",
    "WaitingOutputs",
    "Commit",
    "Frontier",
    "ValidateUpdates",
    "ColumnSetup",
    "ColumnDispatch",
    "ColumnConsume",
    "BeforeState",
    "RedWaiters",
    "ObserveEvents",
    "ObserveState",
    "ObserveCounts",
    "ParkingInvariants",
    "StateVehicles",
    "StateBodies",
    "PublicReplace",
    "CallerCommands",
];
#[derive(Clone, Copy)]
pub struct Sample {
    ns: [u64; N],
    calls: [u64; N],
}
struct Counters {
    ns: [AtomicU64; N],
    calls: [AtomicU64; N],
}
impl Counters {
    const ZERO: Self = Self {
        ns: [const { AtomicU64::new(0) }; N],
        calls: [const { AtomicU64::new(0) }; N],
    };
    fn read(&self) -> Sample {
        Sample {
            ns: std::array::from_fn(|i| self.ns[i].load(Ordering::Relaxed)),
            calls: std::array::from_fn(|i| self.calls[i].load(Ordering::Relaxed)),
        }
    }
    fn clear(&self) {
        for i in 0..N {
            self.ns[i].store(0, Ordering::Relaxed);
            self.calls[i].store(0, Ordering::Relaxed);
        }
    }
}
struct Data {
    all: Counters,
    outcomes: [Counters; 4],
}
static DATA: Data = Data {
    all: Counters::ZERO,
    outcomes: [const { Counters::ZERO }; 4],
};
pub struct Timer(Stage, Instant);
pub fn begin(stage: Stage) -> Timer {
    Timer(stage, Instant::now())
}
impl Drop for Timer {
    fn drop(&mut self) {
        let ns = self.1.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        add(self.0, ns);
    }
}
pub fn add(stage: Stage, ns: u64) {
    let i = stage as usize;
    DATA.all.ns[i].fetch_add(ns, Ordering::Relaxed);
    DATA.all.calls[i].fetch_add(1, Ordering::Relaxed);
}
// 只在同步公共调用返回、全部 worker 已 join 的边界读取或清零。
pub fn snapshot() -> Sample {
    DATA.all.read()
}
pub fn outcome(before: Sample, outcome: usize) {
    let after = snapshot();
    for i in 0..N {
        DATA.outcomes[outcome].ns[i].fetch_add(after.ns[i] - before.ns[i], Ordering::Relaxed);
        DATA.outcomes[outcome].calls[i]
            .fetch_add(after.calls[i] - before.calls[i], Ordering::Relaxed);
    }
}
pub fn reset() {
    DATA.all.clear();
    for outcome in &DATA.outcomes {
        outcome.clear();
    }
}
// 包含式墙钟区间；嵌套项不可相加。分发计时包含 join 等待，不是 worker CPU 累计。
pub fn flush(tick: u64, step_ns: u64, command_ns: u64, observation_ns: u64) {
    {
        let all = DATA.all.read();
        let outcomes: [Sample; 4] = std::array::from_fn(|i| DATA.outcomes[i].read());
        let mut line = format!(
            "{{\"tick\":{tick},\"step_ns\":{step_ns},\"command_ns\":{command_ns},\"observation_ns\":{observation_ns},\"stages\":{{"
        );
        for (i, name) in NAMES.iter().enumerate() {
            if i > 0 {
                line.push(',');
            }
            write!(&mut line, "\"{name}\":[{},{}]", all.calls[i], all.ns[i]).unwrap();
        }
        line.push_str("},\"outcomes\":{");
        for (j, name) in ["success", "entry-blocked", "unsafe-follower", "other"]
            .iter()
            .enumerate()
        {
            if j > 0 {
                line.push(',');
            }
            write!(&mut line, "\"{name}\":{{").unwrap();
            for (i, stage) in NAMES.iter().enumerate() {
                if i > 0 {
                    line.push(',');
                }
                write!(
                    &mut line,
                    "\"{stage}\":[{},{}]",
                    outcomes[j].calls[i], outcomes[j].ns[i]
                )
                .unwrap();
            }
            line.push('}');
        }
        line.push_str("}}");
        eprintln!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_exclude_prior_calls_and_reset_discards_previous_tick() {
        reset();
        add(Stage::PublicReplace, 7);
        let before = snapshot();
        add(Stage::PublicReplace, 100);
        add(Stage::FreshAdmission, 80);
        add(Stage::DirectFollowers, 60);
        outcome(before, 2);
        assert_eq!(snapshot().calls[Stage::PublicReplace as usize], 2);
        let rejected = DATA.outcomes[2].read();
        assert_eq!(rejected.ns[Stage::PublicReplace as usize], 100);
        assert_eq!(rejected.calls[Stage::PublicReplace as usize], 1);
        assert_eq!(rejected.ns[Stage::DirectFollowers as usize], 60);
        assert_eq!(DATA.outcomes[0].read().ns, [0; N]);
        reset();
        assert_eq!(snapshot().ns, [0; N]);
        assert_eq!(DATA.outcomes[2].read().calls, [0; N]);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..1_000 {
                        add(Stage::MotionLoop, 1);
                    }
                });
            }
        });
        assert_eq!(snapshot().calls[Stage::MotionLoop as usize], 8_000);
        assert_eq!(snapshot().ns[Stage::MotionLoop as usize], 8_000);
        reset();
        assert_eq!(snapshot().ns, [0; N]);
    }
}
