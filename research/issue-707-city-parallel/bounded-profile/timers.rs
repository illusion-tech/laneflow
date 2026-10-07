//! #826 独立研究插桩；仅补丁构建载入，不进入正式 Runtime API。
#![allow(missing_docs)]
use std::{cell::RefCell, fmt::Write, time::Instant};
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
impl Sample {
    const ZERO: Self = Self {
        ns: [0; N],
        calls: [0; N],
    };
}
struct Data {
    all: Sample,
    outcomes: [Sample; 4],
}
thread_local! { static DATA: RefCell<Data> = const { RefCell::new(Data { all: Sample::ZERO, outcomes: [Sample::ZERO; 4] }) }; }
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
    DATA.with(|d| {
        let mut d = d.borrow_mut();
        let i = stage as usize;
        d.all.ns[i] += ns;
        d.all.calls[i] += 1;
    });
}
pub fn snapshot() -> Sample {
    DATA.with(|d| d.borrow().all)
}
pub fn outcome(before: Sample, outcome: usize) {
    DATA.with(|d| {
        let mut d = d.borrow_mut();
        for i in 0..N {
            d.outcomes[outcome].ns[i] += d.all.ns[i] - before.ns[i];
            d.outcomes[outcome].calls[i] += d.all.calls[i] - before.calls[i];
        }
    });
}
pub fn reset() {
    DATA.with(|d| {
        *d.borrow_mut() = Data {
            all: Sample::ZERO,
            outcomes: [Sample::ZERO; 4],
        }
    });
}
// 主线程的包含式墙钟区间；嵌套项不可相加。分发计时包含 worker 完成等待，不是 CPU 时间。
pub fn flush(tick: u64, step_ns: u64, command_ns: u64, observation_ns: u64) {
    DATA.with(|d| {
        let d = d.borrow();
        let mut line = format!("{{\"tick\":{tick},\"step_ns\":{step_ns},\"command_ns\":{command_ns},\"observation_ns\":{observation_ns},\"stages\":{{");
        for (i, name) in NAMES.iter().enumerate() {
            if i > 0 {
                line.push(',');
            }
            write!(&mut line, "\"{name}\":[{},{}]", d.all.calls[i], d.all.ns[i]).unwrap();
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
                    d.outcomes[j].calls[i], d.outcomes[j].ns[i]
                )
                .unwrap();
            }
            line.push('}');
        }
        line.push_str("}}");
        eprintln!("{line}");
    });
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
        DATA.with(|d| {
            let d = d.borrow();
            assert_eq!(d.all.calls[Stage::PublicReplace as usize], 2);
            assert_eq!(d.outcomes[2].ns[Stage::PublicReplace as usize], 100);
            assert_eq!(d.outcomes[2].calls[Stage::PublicReplace as usize], 1);
            assert_eq!(d.outcomes[2].ns[Stage::DirectFollowers as usize], 60);
            assert_eq!(d.outcomes[0].ns, [0; N]);
        });
        reset();
        assert_eq!(snapshot().ns, [0; N]);
        DATA.with(|d| assert_eq!(d.borrow().outcomes[2].calls, [0; N]));
    }
}
